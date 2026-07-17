//! Daemon orchestration for the `service` subcommand: wires the fan
//! control loop and power-button monitor together, notifies systemd once
//! both are up (`A§4.2` minus the web-specific bits), and shuts down
//! cleanly on SIGTERM/SIGINT.

use crate::config::{ConfigPaths, FanCurve, RtcSchedule, RtcScheduleEntry, TempUnit};
use crate::fan::FanController;
use crate::hardware::{self, ButtonEvent};
use crate::oled::{OledData, Rotation, Screen};
use crate::sysinfo;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// How often the OLED render loop ticks — fast enough for the clock screen
/// to visibly update, independent of `switchduration`/`screensaver`, which
/// control how often the *screen itself* advances or blanks (W§1.2).
const OLED_TICK_INTERVAL: Duration = Duration::from_secs(1);

struct SystemCpuTemp;

impl crate::fan::TempSource for SystemCpuTemp {
    fn read_cpu_temp_c(&mut self) -> Option<f32> {
        sysinfo::read_cpu_temp_c()
    }
}

/// The HDD curve's demand as a floor on the CPU curve's own target — "if
/// the HDD curve requests a higher speed than the CPU curve, the higher
/// value wins" (`04-fan-curve-editor.html`). No disk temp reading
/// available (no disks, or `smartctl` unavailable) contributes a 0%
/// floor — i.e. no additional demand, the CPU curve's own target still
/// applies via `tick_with_floor`'s `max`.
fn hdd_floor(hdd_curve: &FanCurve, disk_temp_c: Option<f32>) -> u8 {
    disk_temp_c.map(|t| hdd_curve.speed_for(t)).unwrap_or(0)
}

/// `/var/lib/argonone-rs/argonone.db` under systemd's `StateDirectory=`
/// (A§3.2) by default; overridable for local dev/testing where that path
/// isn't creatable (or shouldn't be shared with a real install).
pub(crate) fn db_path() -> std::path::PathBuf {
    std::env::var("ARGONONE_DB_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(crate::db::DEFAULT_DB_PATH))
}

/// `0.0.0.0:8080` by default — no HTTPS until v0.6.0 (A§4.4), so this is
/// plain HTTP, meant for a trusted LAN.
fn bind_addr() -> String {
    std::env::var("ARGONONE_BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string())
}

pub async fn run() {
    let mut hw = hardware::detect();
    tracing::info!(board = ?hw.board, "argonone-rs daemon starting");

    let db_path = db_path();
    let pool = match crate::db::connect(&db_path).await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::error!(error = %e, path = %db_path.display(), "failed to open database, exiting");
            std::process::exit(1);
        }
    };

    // Fan curves and units are DB-backed as of v0.4.0 (A§3.4: "the
    // text-config-file source of truth is gone by design" once the
    // covering table exists) — the legacy config files stay readable
    // only for a one-time import, deferred to v0.7.0.
    let cpu_curve = crate::fan::curve_store::load(&pool, "cpu")
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to load CPU fan curve from DB, using default");
            FanCurve::default_curve()
        });
    let hdd_curve = crate::fan::curve_store::load(&pool, "hdd")
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to load HDD fan curve from DB, using default");
            FanCurve::default_curve()
        });
    let unit = crate::db::settings::load_units(&pool).await;
    // DB-backed as of v0.5.0 (mirrors fan curves/units); the config file
    // stays the fallback default until something's actually been saved.
    let mut oled_cfg = crate::db::settings::load_oled_config(&pool).await;
    let mut rtc_schedule = crate::db::settings::load_rtc_schedule(&pool).await;

    apply_rtc_wake_alarm(hw.rtc.as_mut(), &rtc_schedule);
    let mut rtc_last_sleep_trigger: Option<(u16, u8, u8, u8, u8)> = None;

    let (fan_speed_tx, fan_speed_rx) = tokio::sync::watch::channel(0u8);
    // W§2.7: the REST write handlers push edited curves on these
    // channels after their DB write commits; the control loop below
    // wakes on either its poll-interval timer or a channel update,
    // applying the new curve without restarting (and so without losing
    // the hysteresis state a curve edit shouldn't reset).
    let (cpu_curve_tx, mut cpu_curve_rx) = tokio::sync::watch::channel(cpu_curve.clone());
    let (hdd_curve_tx, mut hdd_curve_rx) = tokio::sync::watch::channel(hdd_curve.clone());
    let (units_tx, mut units_rx) = tokio::sync::watch::channel(unit);
    let (rtc_schedule_tx, mut rtc_schedule_rx) = tokio::sync::watch::channel(rtc_schedule.clone());
    let (oled_config_tx, mut oled_config_rx) = tokio::sync::watch::channel(oled_cfg.clone());
    let (oled_screen_tx, oled_screen_rx) = tokio::sync::watch::channel(None::<Screen>);
    let bind_addr = bind_addr();
    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, addr = %bind_addr, "failed to bind web server, exiting");
            std::process::exit(1);
        }
    };
    let router = crate::web::build_router(
        pool,
        hw.board,
        fan_speed_rx,
        cpu_curve_tx,
        hdd_curve_tx,
        units_tx,
        rtc_schedule_tx,
        oled_config_tx,
        oled_screen_rx,
    )
    .await;
    tracing::info!(addr = %bind_addr, "web server listening");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!(error = %e, "web server task ended unexpectedly");
        }
    });

    let mut button_rx = hw.button.spawn();
    let mut fan_controller = FanController::new(cpu_curve, SystemCpuTemp);
    let mut hdd_curve = hdd_curve;
    let mut last_disk_temp_c: Option<f32> = None;
    let mut oled_cpu_usage = sysinfo::CpuUsage::new();
    let mut oled_unit = unit;
    let mut rotation = Rotation::new(
        Screen::parse_list(&oled_cfg.screenlist),
        Duration::from_secs(oled_cfg.switch_duration_secs.max(1) as u64),
        oled_cfg.screensaver_duration(),
        Instant::now(),
    );
    let mut oled_shown_screen: Option<Screen> = None;

    notify_ready();

    let mut poll = tokio::time::interval(crate::fan::POLL_INTERVAL);
    let mut oled_tick = tokio::time::interval(OLED_TICK_INTERVAL);
    let mut shutdown = Box::pin(shutdown_signal());

    loop {
        tokio::select! {
            _ = poll.tick() => {
                let snapshot = sysinfo::read_storage_snapshot().await;
                last_disk_temp_c = snapshot
                    .iter()
                    .filter_map(|d| d.temp_c)
                    .fold(None, |acc: Option<f32>, t| Some(acc.map_or(t, |a| a.max(t))));
                let floor = hdd_floor(&hdd_curve, last_disk_temp_c);
                let speed = fan_controller.tick_with_floor(hw.fan.as_ref(), Instant::now(), floor);
                fan_speed_tx.send_replace(speed);
                if rtc_schedule.enabled {
                    check_rtc_sleep_schedule(
                        hw.rtc.as_mut(),
                        hw.fan.as_ref(),
                        &rtc_schedule.entries,
                        &mut rtc_last_sleep_trigger,
                    );
                }
            }
            Ok(()) = cpu_curve_rx.changed() => {
                fan_controller.set_curve(cpu_curve_rx.borrow_and_update().clone());
                let floor = hdd_floor(&hdd_curve, last_disk_temp_c);
                let speed = fan_controller.tick_with_floor(hw.fan.as_ref(), Instant::now(), floor);
                fan_speed_tx.send_replace(speed);
            }
            Ok(()) = hdd_curve_rx.changed() => {
                hdd_curve = hdd_curve_rx.borrow_and_update().clone();
                let floor = hdd_floor(&hdd_curve, last_disk_temp_c);
                let speed = fan_controller.tick_with_floor(hw.fan.as_ref(), Instant::now(), floor);
                fan_speed_tx.send_replace(speed);
            }
            Ok(()) = units_rx.changed() => {
                oled_unit = *units_rx.borrow();
            }
            Ok(()) = rtc_schedule_rx.changed() => {
                rtc_schedule = rtc_schedule_rx.borrow_and_update().clone();
                apply_rtc_wake_alarm(hw.rtc.as_mut(), &rtc_schedule);
            }
            Ok(()) = oled_config_rx.changed() => {
                oled_cfg = oled_config_rx.borrow_and_update().clone();
                rotation = Rotation::new(
                    Screen::parse_list(&oled_cfg.screenlist),
                    Duration::from_secs(oled_cfg.switch_duration_secs.max(1) as u64),
                    oled_cfg.screensaver_duration(),
                    Instant::now(),
                );
                oled_shown_screen = None;
                if !oled_cfg.enabled {
                    oled_screen_tx.send_replace(None);
                    if let Err(e) = hw.oled.clear() {
                        tracing::warn!(error = %e, "failed to clear OLED after disabling the panel");
                    }
                }
            }
            _ = oled_tick.tick() => {
                if oled_cfg.enabled {
                    render_oled_tick(
                        hw.oled.as_mut(),
                        &mut rotation,
                        &mut oled_shown_screen,
                        &mut oled_cpu_usage,
                        oled_unit,
                        &oled_screen_tx,
                    );
                }
            }
            Some(event) = button_rx.recv() => {
                handle_button_event(event, hw.fan.as_ref(), &mut rotation);
            }
            _ = &mut shutdown => {
                tracing::info!("received shutdown signal, exiting");
                break;
            }
        }
    }
}

/// Advance the rotation state machine and (re-)render only when the
/// screen to display actually changed, so a static screen isn't
/// needlessly re-flushed over I2C every second. Also publishes the
/// currently-selected screen on `screen_tx` (v0.5.0) whenever the
/// *selection* changes (not every clock-tick redraw) — the web layer's
/// live preview watches this to know when to re-fetch a fresh render,
/// and the `/api/ws` connection forwards it as an `oled_screen` message.
fn render_oled_tick(
    oled: &mut dyn hardware::OledBackend,
    rotation: &mut Rotation,
    shown: &mut Option<Screen>,
    cpu_usage: &mut sysinfo::CpuUsage,
    unit: TempUnit,
    screen_tx: &tokio::sync::watch::Sender<Option<Screen>>,
) {
    rotation.tick(Instant::now());
    let current = rotation.current();

    let selection_changed = current != *shown;
    // The clock screen needs to redraw every tick even though the
    // *selected* screen hasn't changed, so its displayed time doesn't go
    // stale — everything else only redraws when the rotation advances.
    let needs_redraw = selection_changed || current == Some(Screen::Clock);
    *shown = current;
    if selection_changed {
        screen_tx.send_replace(current);
    }
    if !needs_redraw {
        return;
    }

    match current {
        Some(screen) => {
            let data = build_oled_data(cpu_usage, unit);
            if let Err(e) = oled.render(screen, &data) {
                tracing::warn!(error = %e, ?screen, "failed to render OLED screen");
            }
        }
        None => {
            if let Err(e) = oled.clear() {
                tracing::warn!(error = %e, "failed to clear OLED for screensaver blank");
            }
        }
    }
}

/// `pub(crate)` so the web layer's live-preview endpoint (`src/web/oled.rs`,
/// v0.5.0) can gather the same data a real render tick would, for a second
/// render pass into an in-memory framebuffer instead of the physical panel.
pub(crate) fn build_oled_data(cpu_usage: &mut sysinfo::CpuUsage, unit: TempUnit) -> OledData {
    let disks = sysinfo::read_disk_usage()
        .into_iter()
        .map(|d| (d.mount, d.used_pct))
        .collect();
    let raid = sysinfo::read_raid_status()
        .into_iter()
        .map(|r| (r.name, r.state))
        .collect();
    OledData {
        now_utc: time::OffsetDateTime::now_utc(),
        ip: sysinfo::read_local_ip(),
        cpu_pct: cpu_usage.sample_percent(),
        cpu_temp_c: sysinfo::read_cpu_temp_c(),
        ram_pct: sysinfo::read_mem_info().map(|m| m.used_percent()),
        disks,
        raid,
        unit,
        pi_model: sysinfo::read_pi_model(),
    }
}

/// Resolves the schedule's full entry table down to the single next `Wake`
/// occurrence and arms the PCF8563's one alarm slot with it — called once
/// at startup and again on every schedule edit (`rtc_schedule_rx`) and
/// every self-triggered sleep, so the alarm is always armed for whichever
/// wake comes next, not stuck on whatever was configured at boot.
fn apply_rtc_wake_alarm(rtc: &mut dyn hardware::RtcBackend, schedule: &RtcSchedule) {
    if !schedule.enabled {
        return;
    }
    let now = match rtc.read_time() {
        Ok(dt) => dt,
        Err(e) => {
            tracing::warn!(error = %e, "RTC unavailable, cannot program wake alarm");
            return;
        }
    };
    let Some((weekday, hour, minute)) = crate::rtc_schedule::next_wake(&schedule.entries, now)
    else {
        tracing::debug!("no wake entries configured, nothing to arm");
        return;
    };
    // Clear any stale alarm flag left over from a previous fire before
    // programming the next one — a set-but-unacknowledged flag can
    // otherwise make the alarm output look "stuck" to the case MCU.
    if let Err(e) = rtc.clear_alarm() {
        tracing::debug!(error = %e, "no prior RTC alarm to clear (or RTC unavailable)");
    }
    match rtc.set_wake_alarm(hour, minute, Some(weekday)) {
        Ok(()) => tracing::info!(weekday, hour, minute, "RTC wake alarm programmed"),
        Err(e) => tracing::warn!(error = %e, "failed to program RTC wake alarm"),
    }
}

/// Checks the RTC's own clock (not the system clock — the RTC is the
/// source of truth for scheduling since it keeps time across power loss,
/// W§1.1) against the configured `Sleep` entries, and triggers the same
/// shutdown sequence the power button's `Shutdown` pulse uses.
/// `last_trigger` de-dupes so one matching minute doesn't poweroff-loop if
/// something aborts the shutdown.
fn check_rtc_sleep_schedule(
    rtc: &mut dyn hardware::RtcBackend,
    fan: &dyn hardware::FanBackend,
    entries: &[RtcScheduleEntry],
    last_trigger: &mut Option<(u16, u8, u8, u8, u8)>,
) {
    let now = match rtc.read_time() {
        Ok(dt) => dt,
        Err(e) => {
            tracing::debug!(error = %e, "RTC read failed — skipping sleep-schedule check");
            return;
        }
    };
    if crate::rtc_schedule::matching_sleep_entry(entries, now).is_none() {
        return;
    }
    let stamp = (now.year, now.month, now.day, now.hour, now.minute);
    if *last_trigger == Some(stamp) {
        return;
    }
    *last_trigger = Some(stamp);

    tracing::info!(
        hour = now.hour,
        minute = now.minute,
        "RTC sleep schedule matched, shutting down"
    );
    // Re-arm the next wake before going dark — nothing will be running to
    // reprogram the PCF8563's one alarm slot once the Pi actually powers
    // off, so this has to happen now, not on next boot.
    if let Some((weekday, hour, minute)) = crate::rtc_schedule::next_wake(entries, now)
        && let Err(e) = rtc.set_wake_alarm(hour, minute, Some(weekday))
    {
        tracing::warn!(error = %e, "failed to re-arm RTC wake alarm before scheduled sleep");
    }
    if let Err(e) = fan.signal_poweroff() {
        tracing::warn!(error = %e, "failed to signal poweroff to case MCU before scheduled shutdown");
    }
    spawn_system_command("systemctl", &["poweroff"]);
}

fn handle_button_event(
    event: ButtonEvent,
    fan: &dyn hardware::FanBackend,
    rotation: &mut Rotation,
) {
    tracing::info!(?event, "power button event");
    match event {
        ButtonEvent::Reboot => {
            spawn_system_command("systemctl", &["reboot"]);
        }
        ButtonEvent::Shutdown => {
            if let Err(e) = fan.signal_poweroff() {
                tracing::warn!(error = %e, "failed to signal poweroff to case MCU before shutdown");
            }
            spawn_system_command("systemctl", &["poweroff"]);
        }
        ButtonEvent::OledSwitch => {
            rotation.on_button_switch(Instant::now());
        }
    }
}

fn spawn_system_command(program: &str, args: &[&str]) {
    match Command::new(program).args(args).status() {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::error!(program, ?args, ?status, "system command exited non-zero"),
        Err(e) => tracing::error!(program, ?args, error = %e, "failed to run system command"),
    }
}

/// One-shot `SHUTDOWN` compat command: signal the case MCU, then hand off
/// to the system shutdown.
pub fn shutdown_once() {
    let hw = hardware::detect();
    if let Err(e) = hw.fan.signal_poweroff() {
        tracing::warn!(error = %e, "failed to signal poweroff to case MCU");
    }
    spawn_system_command("systemctl", &["poweroff"]);
}

/// One-shot `FANOFF` compat command.
pub fn fanoff_once() {
    let hw = hardware::detect();
    if let Err(e) = hw.fan.set_speed(0) {
        tracing::error!(error = %e, "failed to turn fan off");
    }
}

/// One-shot `status` command (`argonstatus.py` pretty-printer parity):
/// CPU%, RAM, CPU temp, disk usage, RAID status, local IP. Fan curves and
/// units are read from the database (the daemon's actual live source of
/// truth as of v0.4.0), not the legacy config files, so this reflects
/// what the running service is actually applying — falls back to the
/// config-file defaults if the database can't be opened (e.g. run
/// without ever having started the service) rather than failing outright.
pub async fn print_status() {
    let mut hw = hardware::detect();
    println!("board:      {:?}", hw.board);
    println!("fan:        {:?}", hw.fan.capability());

    let pool = crate::db::connect(&db_path()).await.ok();

    let now = hw.rtc.read_time();
    match now {
        Ok(t) => println!(
            "rtc:        {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        ),
        Err(_) => println!("rtc:        unavailable"),
    }
    let schedule = match &pool {
        Some(pool) => crate::db::settings::load_rtc_schedule(pool).await,
        None => RtcSchedule::load_or_default(Path::new(ConfigPaths::RTC_SCHEDULE)),
    };
    if schedule.enabled {
        let wake_count = schedule
            .entries
            .iter()
            .filter(|e| e.kind == crate::config::RtcEventKind::Wake)
            .count();
        let sleep_count = schedule.entries.len() - wake_count;
        print!("rtc wake:   {wake_count} wake entries, {sleep_count} sleep entries configured");
        match now
            .ok()
            .and_then(|dt| crate::rtc_schedule::next_wake(&schedule.entries, dt))
        {
            Some((_, h, m)) => println!(", next wake: {h:02}:{m:02}"),
            None => println!(),
        }
    } else {
        println!("rtc wake:   disabled");
    }

    // CPU% needs two /proc/stat samples; take them 200ms apart.
    let mut cpu = sysinfo::CpuUsage::new();
    cpu.sample_percent();
    std::thread::sleep(std::time::Duration::from_millis(200));
    match cpu.sample_percent() {
        Some(pct) => println!("cpu:        {pct:.1}%"),
        None => println!("cpu:        unavailable"),
    }

    let unit = match &pool {
        Some(pool) => crate::db::settings::load_units(pool).await,
        None => crate::config::TempUnit::load_or_default(Path::new(ConfigPaths::UNITS)),
    };
    match sysinfo::read_cpu_temp_c() {
        Some(t) => println!(
            "temp:       {:.1}\u{b0}{}",
            unit.convert_c(t),
            unit.suffix()
        ),
        None => println!("temp:       unavailable"),
    }

    match sysinfo::read_mem_info() {
        Some(mem) => println!(
            "ram:        {:.1}% used ({} / {} kB)",
            mem.used_percent(),
            mem.total_kb.saturating_sub(mem.available_kb),
            mem.total_kb
        ),
        None => println!("ram:        unavailable"),
    }

    let disks = sysinfo::read_disk_usage();
    if disks.is_empty() {
        println!("disks:      none reported");
    } else {
        for d in disks {
            println!("disk:       {} {}% used", d.mount, d.used_pct);
        }
    }

    let raid = sysinfo::read_raid_status();
    for array in raid {
        println!("raid:       {} {}", array.name, array.state);
    }

    match sysinfo::read_local_ip() {
        Some(ip) => println!("ip:         {ip}"),
        None => println!("ip:         unavailable"),
    }

    match &pool {
        Some(pool) => {
            let cpu = crate::fan::curve_store::load(pool, "cpu")
                .await
                .unwrap_or_else(|_| FanCurve::default_curve());
            let hdd = crate::fan::curve_store::load(pool, "hdd")
                .await
                .unwrap_or_else(|_| FanCurve::default_curve());
            println!("cpu curve:  {} point(s) configured", cpu.0.len());
            println!("hdd curve:  {} point(s) configured", hdd.0.len());
        }
        None => {
            // Database unreachable (e.g. the service has never started) —
            // fall back to what the legacy config files say, same as
            // pre-v0.4.0 behavior.
            match FanCurve::load_or_default(Path::new(ConfigPaths::CPU_CURVE)) {
                Ok(curve) => println!("cpu curve:  {} point(s) configured", curve.0.len()),
                Err(e) => println!("cpu curve:  {e}"),
            }
            match FanCurve::load_or_default(Path::new(ConfigPaths::HDD_CURVE)) {
                Ok(curve) => println!("hdd curve:  {} point(s) configured", curve.0.len()),
                Err(e) => println!("hdd curve:  {e}"),
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn notify_ready() {
    if let Err(e) = sd_notify::notify(&[sd_notify::NotifyState::Ready]) {
        tracing::debug!(error = %e, "sd_notify unavailable (not running under systemd?)");
    }
}

#[cfg(not(target_os = "linux"))]
fn notify_ready() {}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = sigint.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{FanCapability, HwResult};

    struct RecordingFan(std::sync::Mutex<Vec<&'static str>>);
    impl hardware::FanBackend for RecordingFan {
        fn capability(&self) -> FanCapability {
            FanCapability::None
        }
        fn set_speed(&self, _percent: u8) -> HwResult<()> {
            Ok(())
        }
        fn signal_poweroff(&self) -> HwResult<()> {
            self.0.lock().unwrap().push("signal_poweroff");
            Ok(())
        }
    }

    #[test]
    fn oled_switch_event_advances_rotation_without_touching_fan_or_mcu() {
        let fan = RecordingFan(std::sync::Mutex::new(vec![]));
        let mut rotation = Rotation::new(
            vec![Screen::Clock, Screen::Ip],
            Duration::from_secs(1000),
            None,
            Instant::now(),
        );
        assert_eq!(rotation.current(), Some(Screen::Clock));
        handle_button_event(ButtonEvent::OledSwitch, &fan, &mut rotation);
        assert!(fan.0.lock().unwrap().is_empty());
        assert_eq!(rotation.current(), Some(Screen::Ip));
    }

    #[test]
    fn render_oled_tick_skips_redraw_when_screen_unchanged() {
        struct CountingOled(std::sync::Mutex<u32>);
        impl hardware::OledBackend for CountingOled {
            fn render(&mut self, _screen: Screen, _data: &OledData) -> HwResult<()> {
                *self.0.lock().unwrap() += 1;
                Ok(())
            }
            fn clear(&mut self) -> HwResult<()> {
                Ok(())
            }
        }

        let mut oled = CountingOled(std::sync::Mutex::new(0));
        let mut rotation = Rotation::new(
            vec![Screen::Ip],
            Duration::from_secs(1000),
            None,
            Instant::now(),
        );
        let mut shown = None;
        let mut cpu = sysinfo::CpuUsage::new();
        let (screen_tx, mut screen_rx) = tokio::sync::watch::channel(None::<Screen>);

        render_oled_tick(
            &mut oled,
            &mut rotation,
            &mut shown,
            &mut cpu,
            TempUnit::Celsius,
            &screen_tx,
        );
        render_oled_tick(
            &mut oled,
            &mut rotation,
            &mut shown,
            &mut cpu,
            TempUnit::Celsius,
            &screen_tx,
        );

        assert_eq!(*oled.0.lock().unwrap(), 1);
        // Published once, on the first tick's selection (None -> Some(Ip)),
        // not again on the second, unchanged-selection tick.
        assert!(screen_rx.has_changed().unwrap());
        assert_eq!(*screen_rx.borrow_and_update(), Some(Screen::Ip));
        assert!(!screen_rx.has_changed().unwrap());
    }

    // Reboot/Shutdown aren't exercised here: handle_button_event hardcodes
    // `systemctl reboot`/`poweroff`, which would be unsafe to actually
    // invoke from a test (including on a real systemd CI runner). Those
    // paths would need spawn_system_command's program to be injectable
    // before they can be tested safely.

    #[test]
    fn spawn_system_command_success_does_not_panic() {
        spawn_system_command("true", &[]);
    }

    #[test]
    fn spawn_system_command_nonzero_exit_does_not_panic() {
        spawn_system_command("false", &[]);
    }

    #[test]
    fn spawn_system_command_missing_binary_does_not_panic() {
        spawn_system_command("argonone-rs-definitely-not-a-real-binary", &[]);
    }
}

//! Daemon orchestration for the `service` subcommand: wires the fan
//! control loop and power-button monitor together, notifies systemd once
//! both are up (`A§4.2` minus the web-specific bits), and shuts down
//! cleanly on SIGTERM/SIGINT.

use crate::config::{ConfigPaths, FanCurve, OledConfig, RtcSchedule, TempUnit};
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

pub async fn run() {
    let mut hw = hardware::detect();
    tracing::info!(board = ?hw.board, "argonone-rs daemon starting");

    let curve = match FanCurve::load_or_default(Path::new(ConfigPaths::CPU_CURVE)) {
        Ok(curve) => curve,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse fan curve config, using default");
            FanCurve::default_curve()
        }
    };
    let oled_cfg = OledConfig::load_or_default(Path::new(ConfigPaths::OLED));
    let unit = TempUnit::load_or_default(Path::new(ConfigPaths::UNITS));
    let rtc_schedule = RtcSchedule::load_or_default(Path::new(ConfigPaths::RTC_SCHEDULE));

    apply_rtc_wake_alarm(hw.rtc.as_mut(), rtc_schedule);
    let mut rtc_last_sleep_trigger: Option<(u16, u8, u8, u8, u8)> = None;

    let mut button_rx = hw.button.spawn();
    let mut fan_controller = FanController::new(curve, SystemCpuTemp);
    let mut oled_cpu_usage = sysinfo::CpuUsage::new();
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
                fan_controller.tick(hw.fan.as_ref(), Instant::now());
                if let Some(sleep_at) = rtc_schedule.sleep {
                    check_rtc_sleep_schedule(
                        hw.rtc.as_mut(),
                        hw.fan.as_ref(),
                        sleep_at,
                        &mut rtc_last_sleep_trigger,
                    );
                }
            }
            _ = oled_tick.tick() => {
                if oled_cfg.enabled {
                    render_oled_tick(
                        hw.oled.as_mut(),
                        &mut rotation,
                        &mut oled_shown_screen,
                        &mut oled_cpu_usage,
                        unit,
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
/// needlessly re-flushed over I2C every second.
fn render_oled_tick(
    oled: &mut dyn hardware::OledBackend,
    rotation: &mut Rotation,
    shown: &mut Option<Screen>,
    cpu_usage: &mut sysinfo::CpuUsage,
    unit: TempUnit,
) {
    rotation.tick(Instant::now());
    let current = rotation.current();

    // The clock screen needs to redraw every tick even though the
    // *selected* screen hasn't changed, so its displayed time doesn't go
    // stale — everything else only redraws when the rotation advances.
    let needs_redraw = current != *shown || current == Some(Screen::Clock);
    *shown = current;
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

fn build_oled_data(cpu_usage: &mut sysinfo::CpuUsage, unit: TempUnit) -> OledData {
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

fn apply_rtc_wake_alarm(rtc: &mut dyn hardware::RtcBackend, schedule: RtcSchedule) {
    if !schedule.enabled {
        return;
    }
    // Clear any stale alarm flag left over from a previous fire before
    // programming today's — a set-but-unacknowledged flag can otherwise
    // make the alarm output look "stuck" to the case MCU.
    if let Err(e) = rtc.clear_alarm() {
        tracing::debug!(error = %e, "no prior RTC alarm to clear (or RTC unavailable)");
    }
    match rtc.set_wake_alarm(schedule.wake_hour, schedule.wake_minute) {
        Ok(()) => tracing::info!(
            hour = schedule.wake_hour,
            minute = schedule.wake_minute,
            "RTC wake alarm programmed"
        ),
        Err(e) => tracing::warn!(error = %e, "failed to program RTC wake alarm"),
    }
}

/// Checks the RTC's own clock (not the system clock — the RTC is the
/// source of truth for scheduling since it keeps time across power loss,
/// W§1.1) against the configured daily poweroff time, and triggers the
/// same shutdown sequence the power button's `Shutdown` pulse uses.
/// `last_trigger` de-dupes so one matching minute doesn't poweroff-loop if
/// something aborts the shutdown.
fn check_rtc_sleep_schedule(
    rtc: &mut dyn hardware::RtcBackend,
    fan: &dyn hardware::FanBackend,
    sleep_at: (u8, u8),
    last_trigger: &mut Option<(u16, u8, u8, u8, u8)>,
) {
    let now = match rtc.read_time() {
        Ok(dt) => dt,
        Err(e) => {
            tracing::debug!(error = %e, "RTC read failed — skipping sleep-schedule check");
            return;
        }
    };
    if (now.hour, now.minute) != sleep_at {
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
/// CPU%, RAM, CPU temp, disk usage, RAID status, local IP.
pub fn print_status() {
    let mut hw = hardware::detect();
    println!("board:      {:?}", hw.board);
    println!("fan:        {:?}", hw.fan.capability());

    match hw.rtc.read_time() {
        Ok(t) => println!(
            "rtc:        {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        ),
        Err(_) => println!("rtc:        unavailable"),
    }
    let schedule = RtcSchedule::load_or_default(Path::new(ConfigPaths::RTC_SCHEDULE));
    if schedule.enabled {
        print!(
            "rtc wake:   {:02}:{:02}",
            schedule.wake_hour, schedule.wake_minute
        );
        match schedule.sleep {
            Some((h, m)) => println!(", sleep: {h:02}:{m:02}"),
            None => println!(", sleep: not configured"),
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

    let unit = crate::config::TempUnit::load_or_default(Path::new(ConfigPaths::UNITS));
    match sysinfo::read_cpu_temp_c() {
        Some(t) => {
            let (value, suffix) = match unit {
                crate::config::TempUnit::Celsius => (t, "C"),
                crate::config::TempUnit::Fahrenheit => (t * 9.0 / 5.0 + 32.0, "F"),
            };
            println!("temp:       {value:.1}\u{b0}{suffix}");
        }
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

    match FanCurve::load_or_default(Path::new(ConfigPaths::HDD_CURVE)) {
        Ok(curve) => println!("hdd curve:  {} point(s) configured", curve.0.len()),
        Err(e) => println!("hdd curve:  {e}"),
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

        render_oled_tick(
            &mut oled,
            &mut rotation,
            &mut shown,
            &mut cpu,
            TempUnit::Celsius,
        );
        render_oled_tick(
            &mut oled,
            &mut rotation,
            &mut shown,
            &mut cpu,
            TempUnit::Celsius,
        );

        assert_eq!(*oled.0.lock().unwrap(), 1);
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

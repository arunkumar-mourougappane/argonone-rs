//! Daemon orchestration for the `service` subcommand: wires the fan
//! control loop and power-button monitor together, notifies systemd once
//! both are up (`A§4.2` minus the web-specific bits), and shuts down
//! cleanly on SIGTERM/SIGINT.

use crate::config::{ConfigPaths, FanCurve};
use crate::fan::FanController;
use crate::hardware::{self, ButtonEvent};
use crate::sysinfo;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

struct SystemCpuTemp;

impl crate::fan::TempSource for SystemCpuTemp {
    fn read_cpu_temp_c(&mut self) -> Option<f32> {
        sysinfo::read_cpu_temp_c()
    }
}

pub async fn run() {
    let hw = hardware::detect();
    tracing::info!(board = ?hw.board, "argonone-rs daemon starting");

    let curve = match FanCurve::load_or_default(Path::new(ConfigPaths::CPU_CURVE)) {
        Ok(curve) => curve,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse fan curve config, using default");
            FanCurve::default_curve()
        }
    };

    let mut button_rx = hw.button.spawn();
    let mut fan_controller = FanController::new(curve, SystemCpuTemp);

    notify_ready();

    let mut poll = tokio::time::interval(crate::fan::POLL_INTERVAL);
    let mut shutdown = Box::pin(shutdown_signal());

    loop {
        tokio::select! {
            _ = poll.tick() => {
                fan_controller.tick(hw.fan.as_ref(), Instant::now());
            }
            Some(event) = button_rx.recv() => {
                handle_button_event(event, hw.fan.as_ref());
            }
            _ = &mut shutdown => {
                tracing::info!("received shutdown signal, exiting");
                break;
            }
        }
    }
}

fn handle_button_event(event: ButtonEvent, fan: &dyn hardware::FanBackend) {
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
            // No OLED yet — that's v0.2.0. Nothing to do until then.
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
    let hw = hardware::detect();
    println!("board:      {:?}", hw.board);
    println!("fan:        {:?}", hw.fan.capability());

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

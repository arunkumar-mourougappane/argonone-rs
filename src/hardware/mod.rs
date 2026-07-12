//! Hardware access seam: everything that touches I2C/GPIO goes through the
//! traits here, so the daemon runs identically (and without crashing) on a
//! Pi that has no Argon case attached. Backends are chosen once at startup
//! by probing, not decided per-call.

pub mod board;
#[cfg(target_os = "linux")]
pub mod gpio;
#[cfg(target_os = "linux")]
pub mod i2c;
pub mod noop;

use std::fmt;

pub type HwResult<T> = Result<T, HwError>;

#[derive(Debug)]
pub enum HwError {
    Bus(String),
}

impl fmt::Display for HwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HwError::Bus(msg) => write!(f, "bus error: {msg}"),
        }
    }
}

impl std::error::Error for HwError {}

/// What generation of fan register interface the attached board speaks.
/// Detected once via `argonregister_checksupport`-style probe (write a
/// sentinel to reg 0x80, read it back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanCapability {
    /// No case detected on the I2C bus at all.
    None,
    /// Older firmware: single `write_byte(addr, speed)`, no registers.
    LegacyByteWrite,
    /// Newer firmware: register `0x80` = duty cycle, `0x86` = poweroff signal.
    Registers,
}

/// Fan + poweroff-signal control. One instance per running daemon, chosen
/// once at startup by [`FanCapability`] detection.
pub trait FanBackend: Send + Sync {
    fn capability(&self) -> FanCapability;

    /// Set fan duty cycle, 0-100.
    fn set_speed(&self, percent: u8) -> HwResult<()>;

    /// Tell the case's MCU that the Pi is powering off (so it can cut power
    /// once the OS has finished shutting down).
    fn signal_poweroff(&self) -> HwResult<()>;
}

/// A power-button pulse-width event, as decoded from the GPIO monitor
/// thread. Durations are approximate (10ms polling ticks), matching the
/// Python daemon's bucketing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonEvent {
    Reboot,
    Shutdown,
    OledSwitch,
}

/// Power-button monitor. Runs its own blocking poll loop; events are
/// delivered over the returned channel.
pub trait PowerButtonBackend: Send + Sync {
    /// Spawn the monitor, returning a receiver for button events. The
    /// no-op backend returns a channel that never yields anything.
    fn spawn(self: Box<Self>) -> tokio::sync::mpsc::Receiver<ButtonEvent>;
}

/// Bundles the two hardware seams the daemon needs. Built once at startup
/// via [`detect`].
pub struct HardwareBackend {
    pub fan: Box<dyn FanBackend>,
    pub button: Box<dyn PowerButtonBackend>,
    pub board: board::Board,
}

/// Probe for real hardware; fall back to no-op backends for anything not
/// found (or not on Linux at all) so the daemon never crashes for lack of
/// a case.
pub fn detect() -> HardwareBackend {
    #[cfg(target_os = "linux")]
    {
        let mut fan_present = false;
        let fan: Box<dyn FanBackend> = match i2c::I2cFan::detect() {
            Ok(Some(fan)) => {
                tracing::info!(capability = ?fan.capability(), "fan controller detected on I2C bus");
                fan_present = true;
                Box::new(fan)
            }
            Ok(None) => {
                tracing::warn!(
                    "no Argon fan controller found on I2C bus 1 (addr 0x1a) — running with no-op fan backend"
                );
                Box::new(noop::NoopFan)
            }
            Err(e) => {
                tracing::warn!(error = %e, "I2C bus unavailable — running with no-op fan backend");
                Box::new(noop::NoopFan)
            }
        };

        let button: Box<dyn PowerButtonBackend> = match gpio::GpiodPowerButton::open() {
            Ok(b) => {
                tracing::info!("power button GPIO monitor attached");
                Box::new(b)
            }
            Err(e) => {
                tracing::warn!(error = %e, "power button GPIO unavailable — running with no-op button backend");
                Box::new(noop::NoopPowerButton)
            }
        };

        let board = board::detect(fan_present);
        tracing::info!(?board, "board auto-detection complete");

        HardwareBackend { fan, button, board }
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("non-Linux platform — hardware backends are no-op");
        HardwareBackend {
            fan: Box::new(noop::NoopFan),
            button: Box::new(noop::NoopPowerButton),
            board: board::Board::NoCase,
        }
    }
}

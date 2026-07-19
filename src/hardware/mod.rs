//! Hardware access seam: everything that touches I2C/GPIO goes through the
//! traits here, so the daemon runs identically (and without crashing) on a
//! Pi that has no Argon case attached. Backends are chosen once at startup
//! by probing, not decided per-call.

pub mod board;
#[cfg(target_os = "linux")]
pub mod gpio;
#[cfg(target_os = "linux")]
pub mod i2c;
pub mod lockfile;
pub mod noop;
#[cfg(target_os = "linux")]
pub mod oled;
#[cfg(target_os = "linux")]
pub mod rtc;

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
///
/// `LegacyByteWrite`/`Registers` are only ever constructed by the
/// Linux-only I2C probe (`i2c::I2cFan::detect`) — on a non-Linux dev
/// build there's no code path that produces them, which is a real (if
/// platform-specific) dead-code fact, not a bug; scope the lint to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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

    /// Puts the case MCU's own IR receiver into a listen window and
    /// returns whatever code it captured (`None` if nothing was captured
    /// in time), so the case responds to a remote's power button
    /// (v0.6.0, W§3.2's IR remote config page).
    ///
    /// **Unverified against real hardware.** The only documentation this
    /// crate has for I2C register `0x82` is one line — "IR code (block
    /// write)" — with no confirmed trigger sequence or code width. This
    /// implementation is a best-effort reconstruction (a zeroed block
    /// write to start the window, then a block read of the same register
    /// for the result) pending confirmation on a real Argon ONE/EON
    /// board, the same bar every other hardware claim in this codebase is
    /// held to before being called settled.
    fn learn_ir_code(&self) -> HwResult<Option<u32>>;

    /// Re-writes a previously-learned code (e.g. restoring one read back
    /// from the DB after a restart, in case the MCU doesn't retain it
    /// itself across a power cycle). Same unverified-protocol caveat as
    /// [`FanBackend::learn_ir_code`].
    fn program_ir_code(&self, code: u32) -> HwResult<()>;
}

/// A power-button pulse-width event, as decoded from the GPIO monitor
/// thread. Durations are approximate (10ms polling ticks), matching the
/// Python daemon's bucketing.
///
/// Only the Linux-only GPIO monitor (`gpio::classify`) ever constructs
/// these — see the `FanCapability` note above for why the lint is scoped
/// rather than silenced outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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

/// EON OLED panel (W§1.2, §1.7). No-op on Argon ONE/no-case builds — the
/// screen-rotation loop still runs, it just renders into nothing.
pub trait OledBackend: Send + Sync {
    fn render(&mut self, screen: crate::oled::Screen, data: &crate::oled::OledData)
    -> HwResult<()>;
    fn clear(&mut self) -> HwResult<()>;
}

/// A point in time as read from / written to the PCF8563 RTC (W§1.1) — no
/// timezone, matches the chip's own local wall-clock register semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// 0 = Sunday .. 6 = Saturday, matching the PCF8563's own weekday register.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

/// EON RTC (PCF8563) access: read the battery-backed wall-clock time
/// (source of truth for the sleep schedule below, since it keeps ticking
/// across power loss even before NTP is reachable) and program the daily
/// wake alarm (W§1.1). No-op on Argon ONE/no-case builds. There's
/// deliberately no `set_time`: nothing in this daemon needs to program the
/// RTC's own clock — it's the chip's job to free-run, not the daemon's job
/// to drive it.
pub trait RtcBackend: Send + Sync {
    fn read_time(&mut self) -> HwResult<RtcDateTime>;
    /// Program the wake alarm at `hour:minute`. `weekday` (0 = Sunday .. 6
    /// = Saturday, matching [`RtcDateTime::weekday`]) restricts the match
    /// to that one day when `Some`; `None` disables day-of-month/weekday
    /// matching entirely, so it fires every day. The PCF8563 has exactly
    /// one alarm slot — it can hold "every day" or "one specific weekday",
    /// never an arbitrary multi-day set — so a multi-entry schedule
    /// (v0.5.0, `src/rtc_schedule.rs`) always resolves to a single
    /// `Some(weekday)` call for whichever entry is soonest.
    fn set_wake_alarm(&mut self, hour: u8, minute: u8, weekday: Option<u8>) -> HwResult<()>;
    fn clear_alarm(&mut self) -> HwResult<()>;
}

/// Bundles the hardware seams the daemon needs. Built once at startup via
/// [`detect`].
pub struct HardwareBackend {
    /// `Arc`, not `Box` — the fan control loop and the web layer's IR
    /// remote endpoints (v0.6.0) both need a handle to the same backend,
    /// and every `FanBackend` method takes `&self` so sharing it needs no
    /// extra synchronization beyond what `I2cFan` already does internally.
    pub fan: std::sync::Arc<dyn FanBackend>,
    pub button: Box<dyn PowerButtonBackend>,
    pub oled: Box<dyn OledBackend>,
    pub rtc: Box<dyn RtcBackend>,
    pub board: board::Board,
}

/// Probe for real hardware; fall back to no-op backends for anything not
/// found (or not on Linux at all) so the daemon never crashes for lack of
/// a case.
pub fn detect() -> HardwareBackend {
    #[cfg(target_os = "linux")]
    {
        let mut fan_present = false;
        let fan: std::sync::Arc<dyn FanBackend> = match i2c::I2cFan::detect() {
            Ok(Some(fan)) => {
                tracing::info!(capability = ?fan.capability(), "fan controller detected on I2C bus");
                fan_present = true;
                std::sync::Arc::new(fan)
            }
            Ok(None) => {
                tracing::warn!(
                    "no Argon fan controller found on I2C bus 1 (addr 0x1a) — running with no-op fan backend"
                );
                std::sync::Arc::new(noop::NoopFan)
            }
            Err(e) => {
                tracing::warn!(error = %e, "I2C bus unavailable — running with no-op fan backend");
                std::sync::Arc::new(noop::NoopFan)
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

        let (oled, rtc): (Box<dyn OledBackend>, Box<dyn RtcBackend>) = if board == board::Board::Eon
        {
            let oled: Box<dyn OledBackend> = match oled::I2cOled::open() {
                Ok(o) => Box::new(o),
                Err(e) => {
                    tracing::warn!(error = %e, "EON OLED unavailable — running with no-op OLED backend");
                    Box::new(noop::NoopOled)
                }
            };
            let rtc: Box<dyn RtcBackend> = match rtc::Pcf8563Rtc::open() {
                Ok(r) => Box::new(r),
                Err(e) => {
                    tracing::warn!(error = %e, "EON RTC unavailable — running with no-op RTC backend");
                    Box::new(noop::NoopRtc)
                }
            };
            (oled, rtc)
        } else {
            (Box::new(noop::NoopOled), Box::new(noop::NoopRtc))
        };

        HardwareBackend {
            fan,
            button,
            oled,
            rtc,
            board,
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("non-Linux platform — hardware backends are no-op");
        HardwareBackend {
            fan: std::sync::Arc::new(noop::NoopFan),
            button: Box::new(noop::NoopPowerButton),
            oled: Box::new(noop::NoopOled),
            rtc: Box::new(noop::NoopRtc),
            board: board::Board::NoCase,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_falls_back_to_no_op_without_real_hardware() {
        // Dev machines and CI runners have neither an Argon case nor its
        // I2C/GPIO devices attached, so this exercises the same no-op
        // fallback path a bare Raspberry Pi hits (W§1.4).
        let hw = detect();
        assert_eq!(hw.fan.capability(), FanCapability::None);
        assert_eq!(hw.board, board::Board::NoCase);
    }
}

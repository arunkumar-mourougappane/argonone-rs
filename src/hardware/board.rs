//! Board auto-detection: Argon ONE vs Argon EON, decided at runtime by
//! probing the I2C bus rather than an install-time file/flag (W§2.6). EON
//! adds an OLED (`0x3c`) and RTC (`0x51`) on the same bus as the shared
//! fan controller (`0x1a`) — presence of those two addresses is what
//! distinguishes it. EON-specific behavior stays inert on a v0.1.0 build
//! regardless (OLED/RTC land in v0.2.0); this just records the signal.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    /// No Argon fan controller answered on the bus at all.
    NoCase,
    /// Fan controller present, no OLED/RTC — Argon ONE.
    One,
    /// Fan controller + OLED + RTC all present — Argon EON.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Eon,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const OLED_ADDR: u16 = 0x3c;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const RTC_ADDR: u16 = 0x51;

/// Probes bus 1 for the OLED and RTC addresses. Takes whether the fan
/// controller was already found (from [`super::i2c::I2cFan::detect`]) so
/// a bus-open failure there doesn't need to be repeated here.
///
/// `hardware::detect`'s non-Linux branch hardcodes `Board::NoCase`
/// directly rather than calling this (there's no bus to probe at all off
/// Linux), so this function itself goes unreachable in a non-Linux
/// release build specifically — real, not a bug; the `#[cfg(test)]`
/// module below still exercises it directly.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn detect(fan_present: bool) -> Board {
    if !fan_present {
        return Board::NoCase;
    }

    #[cfg(target_os = "linux")]
    {
        let oled = probe_addr(OLED_ADDR);
        let rtc = probe_addr(RTC_ADDR);
        if oled && rtc { Board::Eon } else { Board::One }
    }

    #[cfg(not(target_os = "linux"))]
    {
        Board::One
    }
}

/// How many times to retry a failed probe before believing the address
/// really is absent. A single failed read at boot (bus still settling,
/// a momentary NACK) is common enough on real hardware that treating it
/// as gospel would misdetect a real EON as a plain ONE for the rest of
/// that boot — `detect` only runs once at startup, so there's no later
/// chance to correct it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const PROBE_ATTEMPTS: u32 = 3;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const PROBE_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

/// Retries `probe` up to `attempts` times (with a short delay between
/// tries), succeeding on the first `true`. Platform-agnostic and free of
/// any actual I2C call so the retry behavior itself is testable without
/// real hardware or `cfg(target_os = "linux")` — only `probe_addr` (the
/// real caller) is Linux-only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn probe_with_retries(attempts: u32, mut probe: impl FnMut() -> bool) -> bool {
    for attempt in 0..attempts {
        if probe() {
            return true;
        }
        if attempt + 1 < attempts {
            std::thread::sleep(PROBE_RETRY_DELAY);
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn probe_addr(addr: u16) -> bool {
    use i2cdev::core::I2CDevice;
    use i2cdev::linux::LinuxI2CDevice;
    probe_with_retries(PROBE_ATTEMPTS, || {
        match LinuxI2CDevice::new("/dev/i2c-1", addr) {
            Ok(mut dev) => dev.smbus_read_byte().is_ok(),
            Err(_) => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_fan_controller_means_no_case() {
        assert_eq!(detect(false), Board::NoCase);
    }

    #[test]
    fn fan_present_without_real_i2c_bus_falls_back_to_one() {
        // Dev machines and CI runners have no /dev/i2c-1, so the OLED/RTC
        // probe (Linux) fails closed the same way the non-Linux stub does.
        assert_eq!(detect(true), Board::One);
    }

    #[test]
    fn probe_with_retries_recovers_from_a_transient_failure() {
        let mut calls = 0;
        let found = probe_with_retries(PROBE_ATTEMPTS, || {
            calls += 1;
            // Fails the first attempt (a momentary bus glitch), succeeds
            // on the second — a real EON must still be detected as one,
            // not permanently downgraded to a plain ONE for this boot.
            calls > 1
        });
        assert!(found);
        assert_eq!(calls, 2);
    }

    #[test]
    fn probe_with_retries_gives_up_after_exhausting_all_attempts() {
        let mut calls = 0;
        let found = probe_with_retries(PROBE_ATTEMPTS, || {
            calls += 1;
            false
        });
        assert!(!found);
        assert_eq!(calls, PROBE_ATTEMPTS);
    }
}

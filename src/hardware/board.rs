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

#[cfg(target_os = "linux")]
fn probe_addr(addr: u16) -> bool {
    use i2cdev::core::I2CDevice;
    use i2cdev::linux::LinuxI2CDevice;
    match LinuxI2CDevice::new("/dev/i2c-1", addr) {
        Ok(mut dev) => dev.smbus_read_byte().is_ok(),
        Err(_) => false,
    }
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
}

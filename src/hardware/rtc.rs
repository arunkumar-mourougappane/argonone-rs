//! EON RTC backend (`argonrtc.py` parity): PCF8563 over I2C, address
//! `0x51` (W§1.1). Registers are BCD-encoded; this module owns the BCD
//! math so it's testable without a real chip attached.

use super::{HwError, HwResult, RtcBackend, RtcDateTime};
use i2cdev::core::I2CDevice;
use i2cdev::linux::LinuxI2CDevice;
use std::sync::Mutex;

const RTC_ADDR: u16 = 0x51;

// PCF8563 register map (datasheet section 8.3).
const REG_SECONDS: u8 = 0x02;
const REG_MINUTES: u8 = 0x03;
const REG_HOURS: u8 = 0x04;
const REG_DAYS: u8 = 0x05;
const REG_WEEKDAYS: u8 = 0x06;
const REG_MONTHS: u8 = 0x07;
const REG_YEARS: u8 = 0x08;
const REG_MINUTE_ALARM: u8 = 0x09;
const REG_HOUR_ALARM: u8 = 0x0a;
const REG_DAY_ALARM: u8 = 0x0b;
const REG_WEEKDAY_ALARM: u8 = 0x0c;
const REG_CONTROL_STATUS_2: u8 = 0x01;

/// Alarm register bit that disables matching on that field (datasheet:
/// "AE_x = 1 -> alarm disabled").
const ALARM_DISABLE_BIT: u8 = 0x80;
/// Century bit packed into the months register alongside the BCD month.
const CENTURY_BIT: u8 = 0x80;
/// `CONTROL_STATUS_2` bit that enables the alarm interrupt flag/output.
const CTRL2_AIE: u8 = 0x02;
const CTRL2_AF_CLEAR_MASK: u8 = !0x08;

pub struct Pcf8563Rtc {
    dev: Mutex<LinuxI2CDevice>,
}

impl Pcf8563Rtc {
    pub fn open() -> HwResult<Self> {
        let dev = LinuxI2CDevice::new("/dev/i2c-1", RTC_ADDR)
            .map_err(|e| HwError::Bus(format!("opening /dev/i2c-1 at {RTC_ADDR:#x}: {e}")))?;
        Ok(Pcf8563Rtc {
            dev: Mutex::new(dev),
        })
    }
}

fn bcd_encode(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

fn bcd_decode(value: u8) -> u8 {
    (value >> 4) * 10 + (value & 0x0f)
}

impl RtcBackend for Pcf8563Rtc {
    fn read_time(&mut self) -> HwResult<RtcDateTime> {
        let mut dev = self.dev.lock().expect("RTC mutex poisoned");
        let read = |dev: &mut LinuxI2CDevice, reg: u8| {
            dev.smbus_read_byte_data(reg)
                .map_err(|e| HwError::Bus(format!("reading register {reg:#04x}: {e}")))
        };

        let second = bcd_decode(read(&mut dev, REG_SECONDS)? & 0x7f);
        let minute = bcd_decode(read(&mut dev, REG_MINUTES)? & 0x7f);
        let hour = bcd_decode(read(&mut dev, REG_HOURS)? & 0x3f);
        let day = bcd_decode(read(&mut dev, REG_DAYS)? & 0x3f);
        let weekday = read(&mut dev, REG_WEEKDAYS)? & 0x07;
        let months_reg = read(&mut dev, REG_MONTHS)?;
        let month = bcd_decode(months_reg & 0x1f);
        let century_offset = if months_reg & CENTURY_BIT != 0 {
            1900
        } else {
            2000
        };
        let year = century_offset + bcd_decode(read(&mut dev, REG_YEARS)?) as u16;

        Ok(RtcDateTime {
            year,
            month,
            day,
            weekday,
            hour,
            minute,
            second,
        })
    }

    fn set_wake_alarm(&mut self, hour: u8, minute: u8, weekday: Option<u8>) -> HwResult<()> {
        let mut dev = self.dev.lock().expect("RTC mutex poisoned");
        let write = |dev: &mut LinuxI2CDevice, reg: u8, value: u8| {
            dev.smbus_write_byte_data(reg, value)
                .map_err(|e| HwError::Bus(format!("writing register {reg:#04x}: {e}")))
        };

        // Day-of-month matching is never used (no such concept in the
        // schedule model); weekday matching is enabled only when a
        // specific `weekday` was requested — otherwise disabled so the
        // alarm fires every day, same as before v0.5.0's multi-entry
        // schedule.
        write(&mut dev, REG_MINUTE_ALARM, bcd_encode(minute))?;
        write(&mut dev, REG_HOUR_ALARM, bcd_encode(hour))?;
        write(&mut dev, REG_DAY_ALARM, ALARM_DISABLE_BIT)?;
        let weekday_reg = match weekday {
            Some(wd) => wd & 0x07,
            None => ALARM_DISABLE_BIT,
        };
        write(&mut dev, REG_WEEKDAY_ALARM, weekday_reg)?;

        let status = dev
            .smbus_read_byte_data(REG_CONTROL_STATUS_2)
            .map_err(|e| HwError::Bus(format!("reading control/status 2: {e}")))?;
        write(&mut dev, REG_CONTROL_STATUS_2, status | CTRL2_AIE)?;
        Ok(())
    }

    fn clear_alarm(&mut self) -> HwResult<()> {
        let mut dev = self.dev.lock().expect("RTC mutex poisoned");
        let status = dev
            .smbus_read_byte_data(REG_CONTROL_STATUS_2)
            .map_err(|e| HwError::Bus(format!("reading control/status 2: {e}")))?;
        dev.smbus_write_byte_data(
            REG_CONTROL_STATUS_2,
            (status & CTRL2_AF_CLEAR_MASK) & !CTRL2_AIE,
        )
        .map_err(|e| HwError::Bus(format!("clearing alarm flag: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd_round_trips_valid_range() {
        for v in 0..60u8 {
            assert_eq!(bcd_decode(bcd_encode(v)), v);
        }
    }

    #[test]
    fn bcd_encode_matches_datasheet_examples() {
        assert_eq!(bcd_encode(59), 0x59);
        assert_eq!(bcd_encode(0), 0x00);
        assert_eq!(bcd_decode(0x23), 23);
    }
}

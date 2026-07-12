//! I2C fan controller backend (`argonregister.py` parity). Case MCU sits at
//! address `0x1a` on bus 1. Newer firmware exposes registers; older
//! firmware only understands a raw `write_byte`. Capability is detected
//! once at startup by probing, matching `argonregister_checksupport`.

use super::{FanBackend, FanCapability, HwError, HwResult};
use i2cdev::core::I2CDevice;
use i2cdev::linux::LinuxI2CDevice;
use std::sync::Mutex;

const FAN_ADDR: u16 = 0x1a;
const REG_DUTY_CYCLE: u8 = 0x80;
const REG_CTRL: u8 = 0x86;
const CTRL_POWEROFF: u8 = 0x01;

/// Sentinel written to `REG_DUTY_CYCLE` and read back to confirm the
/// firmware actually implements registers rather than treating every
/// `write_byte` as a raw speed value.
const PROBE_VALUE: u8 = 0;

pub struct I2cFan {
    dev: Mutex<LinuxI2CDevice>,
    capability: FanCapability,
}

impl I2cFan {
    /// Probe bus 1 for the Argon fan controller. Returns `Ok(None)` if
    /// nothing answers at `0x1a` (no case attached) rather than treating
    /// that as an error — only a bus-open failure is an error.
    pub fn detect() -> HwResult<Option<Self>> {
        let bus_path = "/dev/i2c-1";
        let mut dev = LinuxI2CDevice::new(bus_path, FAN_ADDR)
            .map_err(|e| HwError::Bus(format!("opening {bus_path}: {e}")))?;

        let capability = probe_capability(&mut dev);
        if capability == FanCapability::None {
            return Ok(None);
        }

        Ok(Some(I2cFan {
            dev: Mutex::new(dev),
            capability,
        }))
    }
}

fn probe_capability(dev: &mut LinuxI2CDevice) -> FanCapability {
    // Try the register interface first: write a sentinel to the duty-cycle
    // register, then read it back. Legacy firmware ignores register reads
    // (or errors on the smbus read op entirely), so a mismatch or error
    // means "no registers" rather than "no fan".
    if dev
        .smbus_write_byte_data(REG_DUTY_CYCLE, PROBE_VALUE)
        .is_ok()
        && let Ok(readback) = dev.smbus_read_byte_data(REG_DUTY_CYCLE)
        && readback == PROBE_VALUE
    {
        return FanCapability::Registers;
    }

    // Fall back: does anything answer a raw byte write at all?
    if dev.smbus_write_byte(PROBE_VALUE).is_ok() {
        return FanCapability::LegacyByteWrite;
    }

    FanCapability::None
}

impl FanBackend for I2cFan {
    fn capability(&self) -> FanCapability {
        self.capability
    }

    fn set_speed(&self, percent: u8) -> HwResult<()> {
        let percent = percent.min(100);
        let mut dev = self.dev.lock().expect("I2C fan mutex poisoned");
        match self.capability {
            FanCapability::Registers => dev
                .smbus_write_byte_data(REG_DUTY_CYCLE, percent)
                .map_err(|e| HwError::Bus(format!("writing duty cycle: {e}"))),
            FanCapability::LegacyByteWrite => dev
                .smbus_write_byte(percent)
                .map_err(|e| HwError::Bus(format!("writing legacy speed byte: {e}"))),
            FanCapability::None => Ok(()),
        }
    }

    fn signal_poweroff(&self) -> HwResult<()> {
        let mut dev = self.dev.lock().expect("I2C fan mutex poisoned");
        match self.capability {
            FanCapability::Registers => dev
                .smbus_write_byte_data(REG_CTRL, CTRL_POWEROFF)
                .map_err(|e| HwError::Bus(format!("writing poweroff signal: {e}"))),
            // Legacy firmware overloads the raw speed byte: 0xFF means
            // "powering off" instead of "full speed".
            FanCapability::LegacyByteWrite => dev
                .smbus_write_byte(0xFF)
                .map_err(|e| HwError::Bus(format!("writing legacy poweroff byte: {e}"))),
            FanCapability::None => Ok(()),
        }
    }
}

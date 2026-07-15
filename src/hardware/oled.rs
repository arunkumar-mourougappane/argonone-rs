//! EON OLED backend: SSD1306 over I2C via the `ssd1306` + `embedded-graphics`
//! crates (W§1.7 — the panel itself is a stock SSD1306, nothing custom
//! about the silicon; only Argon40's asset *format* was bespoke, and this
//! project doesn't use those assets — see `crate::oled` module docs).

use super::{HwError, HwResult, OledBackend};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use linux_embedded_hal::I2cdev;
use ssd1306::mode::BufferedGraphicsMode;
use ssd1306::prelude::*;
use ssd1306::{I2CDisplayInterface, Ssd1306};

type Display = Ssd1306<
    display_interface_i2c::I2CInterface<I2cdev>,
    DisplaySize128x64,
    BufferedGraphicsMode<DisplaySize128x64>,
>;

pub struct I2cOled {
    display: Display,
}

impl I2cOled {
    /// Open `/dev/i2c-1` and initialize the panel at its default address
    /// (`0x3C`, matching `board::detect`'s probe address).
    pub fn open() -> HwResult<Self> {
        let i2c = I2cdev::new("/dev/i2c-1")
            .map_err(|e| HwError::Bus(format!("opening /dev/i2c-1: {e}")))?;
        let interface = I2CDisplayInterface::new(i2c);
        let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        display
            .init()
            .map_err(|e| HwError::Bus(format!("initializing SSD1306: {e:?}")))?;
        Ok(I2cOled { display })
    }
}

impl OledBackend for I2cOled {
    fn render(
        &mut self,
        screen: crate::oled::Screen,
        data: &crate::oled::OledData,
    ) -> HwResult<()> {
        crate::oled::render::draw_screen(&mut self.display, screen, data)
            .map_err(|e| HwError::Bus(format!("drawing OLED screen: {e:?}")))?;
        self.display
            .flush()
            .map_err(|e| HwError::Bus(format!("flushing OLED: {e:?}")))
    }

    fn clear(&mut self) -> HwResult<()> {
        self.display
            .clear(BinaryColor::Off)
            .map_err(|e| HwError::Bus(format!("clearing OLED: {e:?}")))?;
        self.display
            .flush()
            .map_err(|e| HwError::Bus(format!("flushing OLED: {e:?}")))
    }
}

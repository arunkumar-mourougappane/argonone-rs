//! An in-memory `DrawTarget` for the OLED's live web preview (v0.5.0):
//! `render::draw_screen` is already generic over any
//! `embedded-graphics::DrawTarget<Color = BinaryColor>`, so rendering a
//! preview is just a second render pass into this instead of the real
//! `ssd1306` driver — no framebuffer-readback API needed, and no new
//! dependency (`embedded-graphics` is already used for the real panel).

use embedded_graphics::Pixel;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;

/// Matches the real panel's `DisplaySize128x64` (`src/hardware/oled.rs`).
pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 64;

pub struct Framebuffer {
    pixels: [bool; WIDTH * HEIGHT],
}

impl Framebuffer {
    pub fn new() -> Self {
        Framebuffer {
            pixels: [false; WIDTH * HEIGHT],
        }
    }

    /// Row-major, 8 pixels per byte, MSB first — the wire format the web
    /// preview endpoint returns (as a plain JSON byte array) for the
    /// client to unpack straight onto a `<canvas>` via `putImageData`, no
    /// image-encoding crate needed for a 1-bit 128×64 image this small.
    pub fn packed_bits(&self) -> Vec<u8> {
        self.pixels
            .chunks(8)
            .map(|chunk| {
                chunk.iter().enumerate().fold(
                    0u8,
                    |byte, (i, &on)| {
                        if on { byte | (0x80 >> i) } else { byte }
                    },
                )
            })
            .collect()
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginDimensions for Framebuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for Framebuffer {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x >= 0
                && (point.x as usize) < WIDTH
                && point.y >= 0
                && (point.y as usize) < HEIGHT
            {
                self.pixels[point.y as usize * WIDTH + point.x as usize] = color == BinaryColor::On;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oled::{OledData, Screen, render::draw_screen};

    fn sample_data() -> OledData {
        OledData {
            now_utc: time::OffsetDateTime::UNIX_EPOCH,
            ip: None,
            cpu_pct: Some(42.0),
            cpu_temp_c: Some(51.2),
            ram_pct: Some(60.0),
            disks: vec![],
            raid: vec![],
            unit: crate::config::TempUnit::Celsius,
            pi_model: None,
        }
    }

    #[test]
    fn every_screen_lights_at_least_one_pixel() {
        let data = sample_data();
        for screen in [
            Screen::Clock,
            Screen::Ip,
            Screen::Cpu,
            Screen::Ram,
            Screen::Storage,
            Screen::Temp,
            Screen::Raid,
            Screen::Splash,
        ] {
            let mut fb = Framebuffer::new();
            draw_screen(&mut fb, screen, &data).unwrap();
            assert!(
                fb.pixels.iter().any(|&p| p),
                "{screen:?} lit no pixels in the framebuffer"
            );
        }
    }

    #[test]
    fn packed_bits_round_trips_a_known_pattern() {
        let mut fb = Framebuffer::new();
        // Light every pixel in the first byte-row's first 8 columns.
        for x in 0..8 {
            fb.pixels[x] = true;
        }
        let bits = fb.packed_bits();
        assert_eq!(bits[0], 0xff);
        assert_eq!(bits.len(), (WIDTH * HEIGHT) / 8);
    }

    #[test]
    fn packed_bits_length_matches_128x64_1bit() {
        let fb = Framebuffer::new();
        assert_eq!(fb.packed_bits().len(), 1024);
    }
}

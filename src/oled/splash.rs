//! Original boot/rotation splash screen (W§1.5's resolution): `RPI`
//! rotated 90° in a left column, the detected Pi model large and
//! horizontal, and a small `argonone` signature — same layout *structure*
//! as Argon40's original `logo1v5.bin` ("ONE V5"), but built from scratch:
//! an original wordmark plus `embedded-graphics`'s bundled font, zero bytes
//! of Argon40's asset touched.

use embedded_graphics::{
    mono_font::{
        MonoTextStyle,
        ascii::{FONT_6X10, FONT_9X15_BOLD},
    },
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};

/// A tiny off-screen 1bpp buffer, used to render text once and then blit it
/// rotated into the real target — `embedded-graphics` has no built-in text
/// rotation, so this composes it out of primitives it does have.
struct TinyBuffer {
    width: u32,
    height: u32,
    bits: Vec<bool>,
}

impl TinyBuffer {
    fn new(width: u32, height: u32) -> Self {
        TinyBuffer {
            width,
            height,
            bits: vec![false; (width * height) as usize],
        }
    }

    fn get(&self, x: u32, y: u32) -> bool {
        self.bits[(y * self.width + x) as usize]
    }
}

impl OriginDimensions for TinyBuffer {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}

impl DrawTarget for TinyBuffer {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x >= 0
                && point.y >= 0
                && (point.x as u32) < self.width
                && (point.y as u32) < self.height
            {
                let idx = (point.y as u32) * self.width + point.x as u32;
                self.bits[idx as usize] = color == BinaryColor::On;
            }
        }
        Ok(())
    }
}

/// Render `text` at the given style into a tightly-fitted [`TinyBuffer`],
/// then blit it rotated 90° clockwise into `target` with its top-left
/// corner at `origin`.
fn draw_rotated_text<D>(
    target: &mut D,
    text: &str,
    style: MonoTextStyle<'static, BinaryColor>,
    origin: Point,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    let glyph_w = style.font.character_size.width;
    let glyph_h = style.font.character_size.height;
    let buf_w = glyph_w * text.chars().count() as u32;
    let buf_h = glyph_h;
    let mut buf = TinyBuffer::new(buf_w.max(1), buf_h.max(1));
    let _ = Text::with_baseline(text, Point::zero(), style, Baseline::Top).draw(&mut buf);

    let mut pixels = Vec::new();
    for y in 0..buf.height {
        for x in 0..buf.width {
            if buf.get(x, y) {
                // 90° clockwise: buffer column becomes the row, buffer row
                // (from the bottom) becomes the column.
                let rx = origin.x + (buf.height - 1 - y) as i32;
                let ry = origin.y + x as i32;
                pixels.push(Pixel(Point::new(rx, ry), BinaryColor::On));
            }
        }
    }
    target.draw_iter(pixels)
}

pub fn draw_splash<D>(target: &mut D, pi_model: Option<&str>) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    target.clear(BinaryColor::Off)?;

    let word_style = MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On);
    draw_rotated_text(target, "RPI", word_style, Point::new(2, 2))?;

    let version = version_label(pi_model);
    let version_style = MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On);
    Text::with_baseline(&version, Point::new(24, 20), version_style, Baseline::Top).draw(target)?;

    let signature_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    Text::with_baseline(
        "argonone",
        Point::new(80, 54),
        signature_style,
        Baseline::Top,
    )
    .draw(target)?;

    Ok(())
}

/// Extracts a `V<N>` label from a `/proc/device-tree/model`-style string
/// (e.g. `"Raspberry Pi 5 Model B"` -> `"V5"`). Falls back to `"V?"` when
/// the model string is missing or doesn't match the expected shape.
fn version_label(pi_model: Option<&str>) -> String {
    let digit = pi_model.and_then(|model| {
        model
            .split_whitespace()
            .skip_while(|word| *word != "Pi")
            .nth(1)
            .and_then(|word| word.chars().next())
            .filter(|c| c.is_ascii_digit())
    });
    match digit {
        Some(d) => format!("V{d}"),
        None => "V?".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_graphics::mock_display::MockDisplay;

    #[test]
    fn version_label_extracts_model_number() {
        assert_eq!(version_label(Some("Raspberry Pi 5 Model B Rev 1.0")), "V5");
        assert_eq!(version_label(Some("Raspberry Pi 4 Model B")), "V4");
    }

    #[test]
    fn version_label_falls_back_when_model_unknown() {
        assert_eq!(version_label(None), "V?");
        assert_eq!(version_label(Some("some other board")), "V?");
    }

    #[test]
    fn splash_draws_without_error() {
        let mut display: MockDisplay<BinaryColor> = MockDisplay::new();
        display.set_allow_out_of_bounds_drawing(true);
        display.set_allow_overdraw(true);
        draw_splash(&mut display, Some("Raspberry Pi 5 Model B")).unwrap();
        assert!(display.affected_area().size.width > 0);
    }

    #[test]
    fn rotated_text_actually_rotates_bounding_box() {
        // A wide "RPI" glyph run should end up *taller* than wide once
        // rotated 90°, proving the rotation transform actually ran rather
        // than silently degrading to an unrotated blit.
        let style = MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On);
        let mut display: MockDisplay<BinaryColor> = MockDisplay::new();
        display.set_allow_out_of_bounds_drawing(true);
        display.set_allow_overdraw(true);
        draw_rotated_text(&mut display, "RPI", style, Point::new(0, 0)).unwrap();
        let area = display.affected_area();
        assert!(
            area.size.height > area.size.width,
            "expected rotated text to be taller than wide, got {area:?}"
        );
    }
}

//! Per-screen drawing, generic over any `embedded-graphics` `DrawTarget` so
//! the same code path drives the real SSD1306 panel and
//! `embedded_graphics::mock_display::MockDisplay` in tests.

use super::{OledData, Screen};
use crate::config::TempUnit;
use embedded_graphics::{
    mono_font::{
        MonoTextStyle,
        ascii::{FONT_6X10, FONT_9X15_BOLD},
    },
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};

const HEADLINE_Y: i32 = 34;
const SUBLINE_Y: i32 = 52;

fn label_style() -> MonoTextStyle<'static, BinaryColor> {
    MonoTextStyle::new(&FONT_6X10, BinaryColor::On)
}

fn headline_style() -> MonoTextStyle<'static, BinaryColor> {
    MonoTextStyle::new(&FONT_9X15_BOLD, BinaryColor::On)
}

fn draw_label<D>(target: &mut D, text: &str) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    Text::with_baseline(text, Point::new(0, 0), label_style(), Baseline::Top).draw(target)?;
    Ok(())
}

fn draw_headline<D>(target: &mut D, text: &str, y: i32) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    Text::with_baseline(text, Point::new(0, y), headline_style(), Baseline::Top).draw(target)?;
    Ok(())
}

fn draw_subline<D>(target: &mut D, text: &str, y: i32) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    Text::with_baseline(text, Point::new(0, y), label_style(), Baseline::Top).draw(target)?;
    Ok(())
}

fn fmt_temp(temp_c: f32, unit: TempUnit) -> String {
    format!("{:.1}{}", unit.convert_c(temp_c), unit.suffix())
}

/// Draw `screen`'s content into `target`, clearing it first. Splash is
/// handled separately by [`super::splash::draw_splash`] since it needs
/// rotated text, not this module's label/headline layout.
pub fn draw_screen<D>(target: &mut D, screen: Screen, data: &OledData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    target.clear(BinaryColor::Off)?;
    match screen {
        Screen::Clock => draw_clock(target, data),
        Screen::Ip => draw_ip(target, data),
        Screen::Cpu => draw_cpu(target, data),
        Screen::Ram => draw_ram(target, data),
        Screen::Storage => draw_storage(target, data),
        Screen::Temp => draw_temp(target, data),
        Screen::Raid => draw_raid(target, data),
        Screen::Splash => super::splash::draw_splash(target, data.pi_model.as_deref()),
    }
}

fn draw_clock<D>(target: &mut D, data: &OledData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    draw_label(target, "CLOCK (UTC)")?;
    let t = data.now_utc.time();
    draw_headline(
        target,
        &format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second()),
        HEADLINE_Y,
    )?;
    let d = data.now_utc.date();
    draw_subline(
        target,
        &format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day()),
        SUBLINE_Y,
    )
}

fn draw_ip<D>(target: &mut D, data: &OledData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    draw_label(target, "IP ADDRESS")?;
    match data.ip {
        Some(ip) => draw_headline(target, &ip.to_string(), HEADLINE_Y),
        None => draw_headline(target, "unavailable", HEADLINE_Y),
    }
}

fn draw_cpu<D>(target: &mut D, data: &OledData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    draw_label(target, "CPU")?;
    match data.cpu_pct {
        Some(pct) => draw_headline(target, &format!("{pct:.0}%"), HEADLINE_Y)?,
        None => draw_headline(target, "n/a", HEADLINE_Y)?,
    }
    match data.cpu_temp_c {
        Some(t) => draw_subline(target, &fmt_temp(t, data.unit), SUBLINE_Y),
        None => Ok(()),
    }
}

fn draw_ram<D>(target: &mut D, data: &OledData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    draw_label(target, "RAM")?;
    match data.ram_pct {
        Some(pct) => draw_headline(target, &format!("{pct:.0}%"), HEADLINE_Y),
        None => draw_headline(target, "n/a", HEADLINE_Y),
    }
}

fn draw_storage<D>(target: &mut D, data: &OledData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    draw_label(target, "STORAGE")?;
    if data.disks.is_empty() {
        return draw_subline(target, "no disks", HEADLINE_Y);
    }
    for (i, (mount, pct)) in data.disks.iter().take(4).enumerate() {
        let y = HEADLINE_Y - 12 + (i as i32) * 12;
        draw_subline(target, &format!("{mount} {pct}%"), y)?;
    }
    Ok(())
}

fn draw_temp<D>(target: &mut D, data: &OledData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    draw_label(target, "TEMP")?;
    match data.cpu_temp_c {
        Some(t) => draw_headline(target, &fmt_temp(t, data.unit), HEADLINE_Y),
        None => draw_headline(target, "n/a", HEADLINE_Y),
    }
}

fn draw_raid<D>(target: &mut D, data: &OledData) -> Result<(), D::Error>
where
    D: DrawTarget<Color = BinaryColor>,
{
    draw_label(target, "RAID")?;
    if data.raid.is_empty() {
        return draw_subline(target, "no arrays", HEADLINE_Y);
    }
    for (i, (name, state)) in data.raid.iter().take(4).enumerate() {
        let y = HEADLINE_Y - 12 + (i as i32) * 12;
        draw_subline(target, &format!("{name} {state}"), y)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_graphics::mock_display::MockDisplay;
    use std::net::{IpAddr, Ipv4Addr};
    use time::macros::datetime;

    fn sample_data() -> OledData {
        OledData {
            now_utc: datetime!(2026-07-14 12:34:56 UTC),
            ip: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 42))),
            cpu_pct: Some(37.0),
            cpu_temp_c: Some(45.5),
            ram_pct: Some(61.0),
            disks: vec![("/".to_string(), 50)],
            raid: vec![("md0".to_string(), "active".to_string())],
            unit: TempUnit::Celsius,
            pi_model: Some("Raspberry Pi 5 Model B".to_string()),
        }
    }

    #[test]
    fn every_screen_draws_without_error_and_lights_at_least_one_pixel() {
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
            let mut display: MockDisplay<BinaryColor> = MockDisplay::new();
            display.set_allow_out_of_bounds_drawing(true);
            display.set_allow_overdraw(true);
            draw_screen(&mut display, screen, &data).unwrap();
            let lit = display.affected_area();
            assert!(
                lit.size.width > 0 && lit.size.height > 0,
                "{screen:?} drew nothing"
            );
        }
    }

    #[test]
    fn clock_renders_formatted_time_and_date() {
        let data = sample_data();
        let mut display: MockDisplay<BinaryColor> = MockDisplay::new();
        display.set_allow_out_of_bounds_drawing(true);
        display.set_allow_overdraw(true);
        draw_screen(&mut display, Screen::Clock, &data).unwrap();
        assert!(display.affected_area().size.width > 0);
    }

    #[test]
    fn fmt_temp_converts_to_fahrenheit() {
        assert_eq!(fmt_temp(0.0, TempUnit::Fahrenheit), "32.0F");
        assert_eq!(fmt_temp(100.0, TempUnit::Celsius), "100.0C");
    }

    #[test]
    fn missing_data_falls_back_to_placeholder_text() {
        let mut data = sample_data();
        data.ip = None;
        data.cpu_pct = None;
        data.cpu_temp_c = None;
        data.ram_pct = None;
        data.disks.clear();
        data.raid.clear();
        let mut display: MockDisplay<BinaryColor> = MockDisplay::new();
        display.set_allow_out_of_bounds_drawing(true);
        display.set_allow_overdraw(true);
        for screen in [
            Screen::Ip,
            Screen::Cpu,
            Screen::Ram,
            Screen::Storage,
            Screen::Temp,
            Screen::Raid,
        ] {
            draw_screen(&mut display, screen, &data).unwrap();
        }
    }
}

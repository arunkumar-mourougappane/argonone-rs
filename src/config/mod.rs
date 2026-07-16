//! Compat parsers for the existing Python daemon's plain-text config files
//! (W§1.3). Kept byte-for-byte format compatible so an upgrade from the
//! Python daemon (or a fresh install following its docs) doesn't require
//! reformatting anything — this stays the only source of truth until the
//! v0.3.0 SQLite migration replaces it.

use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub struct ConfigError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for ConfigError {}

/// One `temp°C=fan%` point from `/etc/argononed.conf` or
/// `/etc/argononed-hdd.conf`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub temp_c: i32,
    pub speed_pct: u8,
}

/// A fan curve, sorted descending by temperature (highest threshold wins,
/// matching the Python daemon's evaluation order).
#[derive(Debug, Clone, PartialEq)]
pub struct FanCurve(pub Vec<CurvePoint>);

impl FanCurve {
    /// Default curve if no config file exists yet — conservative, audible
    /// only under real load. Fan curve *editing* is v0.4.0; this is just
    /// the hardcoded fallback for v0.1.0.
    pub fn default_curve() -> Self {
        FanCurve(vec![
            CurvePoint {
                temp_c: 65,
                speed_pct: 100,
            },
            CurvePoint {
                temp_c: 60,
                speed_pct: 55,
            },
            CurvePoint {
                temp_c: 55,
                speed_pct: 30,
            },
        ])
    }

    /// Highest-threshold-first speed lookup: the first point whose
    /// threshold the current temperature meets or exceeds wins; below all
    /// thresholds, the fan is off.
    pub fn speed_for(&self, temp_c: f32) -> u8 {
        self.0
            .iter()
            .find(|p| temp_c >= p.temp_c as f32)
            .map(|p| p.speed_pct)
            .unwrap_or(0)
    }

    /// Server-enforced safety floor (W§2.8): rejects a curve that would
    /// give less than [`MIN_SAFE_PCT`] fan speed at or above
    /// [`SAFETY_CEILING_C`], independent of what points an operator
    /// configured — a client-side-only check can't stop an API caller
    /// from bypassing it. Checks every configured breakpoint at or above
    /// the ceiling plus the ceiling itself: `speed_for` is a step
    /// function that only changes value at breakpoints, so that set
    /// fully characterizes its behavior across `[SAFETY_CEILING_C, ∞)`.
    pub fn violates_safety_floor(&self) -> bool {
        const SAFETY_CEILING_C: f32 = 75.0;
        const MIN_SAFE_PCT: u8 = 25;
        let mut checkpoints: Vec<f32> = self
            .0
            .iter()
            .map(|p| p.temp_c as f32)
            .filter(|&t| t >= SAFETY_CEILING_C)
            .collect();
        checkpoints.push(SAFETY_CEILING_C);
        checkpoints
            .iter()
            .any(|&t| self.speed_for(t) < MIN_SAFE_PCT)
    }

    pub fn parse(contents: &str) -> Result<Self, String> {
        let mut points = Vec::new();
        for (lineno, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (temp, speed) = line.split_once('=').ok_or_else(|| {
                format!("line {}: expected `temp=speed`, got {line:?}", lineno + 1)
            })?;
            let temp_c: i32 = temp
                .trim()
                .parse()
                .map_err(|_| format!("line {}: invalid temperature {temp:?}", lineno + 1))?;
            let speed_pct: u8 = speed
                .trim()
                .parse()
                .map_err(|_| format!("line {}: invalid speed {speed:?}", lineno + 1))?;
            points.push(CurvePoint {
                temp_c,
                speed_pct: speed_pct.min(100),
            });
        }
        points.sort_by_key(|p| std::cmp::Reverse(p.temp_c));
        Ok(FanCurve(points))
    }

    /// Load from disk, falling back to [`default_curve`] if the file is
    /// absent (fresh install, no config written yet).
    pub fn load_or_default(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Self::parse(&contents).map_err(|message| ConfigError {
                path: path.display().to_string(),
                message,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default_curve()),
            Err(e) => Err(ConfigError {
                path: path.display().to_string(),
                message: e.to_string(),
            }),
        }
    }
}

/// `/etc/argonunits.conf`: `temperature=C` or `temperature=F`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempUnit {
    Celsius,
    Fahrenheit,
}

impl TempUnit {
    pub fn load_or_default(path: &Path) -> Self {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return TempUnit::Celsius,
        };
        for line in contents.lines() {
            if let Some((key, value)) = line.trim().split_once('=')
                && key.trim() == "temperature"
            {
                return match value.trim() {
                    "F" | "f" => TempUnit::Fahrenheit,
                    _ => TempUnit::Celsius,
                };
            }
        }
        TempUnit::Celsius
    }

    /// Converts a Celsius reading to this unit, in degrees — the plain
    /// numeric value, so callers can format it however their surface
    /// needs (a `%.1f°C` string here, an OLED-safe ASCII `C` suffix
    /// there, a bare number over JSON elsewhere). Sysinfo/hardware
    /// always reads Celsius; this is the one conversion formula every
    /// display surface should share, rather than each re-deriving it
    /// (and risking exactly the kind of "some pages forgot to convert"
    /// bug this was extracted to close off).
    pub fn convert_c(self, celsius: f32) -> f32 {
        match self {
            TempUnit::Celsius => celsius,
            TempUnit::Fahrenheit => celsius * 9.0 / 5.0 + 32.0,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            TempUnit::Celsius => "C",
            TempUnit::Fahrenheit => "F",
        }
    }
}

/// `/etc/argoneonoled.conf`: EON screen-rotation settings (W§1.3, §1.2).
/// `screenlist` values in the wild are double-quoted
/// (`screenlist="clock ip cpu storage temp"`); quotes are stripped on read.
#[derive(Debug, Clone, PartialEq)]
pub struct OledConfig {
    pub switch_duration_secs: u32,
    /// Screensaver blank-after-idle, in seconds. `0` means disabled,
    /// matching the shell config tool's convention for "off".
    pub screensaver_secs: u32,
    pub screenlist: String,
    pub enabled: bool,
}

impl OledConfig {
    pub fn default_config() -> Self {
        OledConfig {
            switch_duration_secs: 10,
            screensaver_secs: 120,
            screenlist: "clock ip cpu storage temp".to_string(),
            enabled: true,
        }
    }

    pub fn parse(contents: &str) -> Self {
        let mut cfg = Self::default_config();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            match key.trim() {
                "switchduration" => {
                    if let Ok(v) = value.parse() {
                        cfg.switch_duration_secs = v;
                    }
                }
                "screensaver" => {
                    if let Ok(v) = value.parse() {
                        cfg.screensaver_secs = v;
                    }
                }
                "screenlist" => cfg.screenlist = value.to_string(),
                "enabled" => cfg.enabled = matches!(value, "Y" | "y" | "1" | "true"),
                _ => {}
            }
        }
        cfg
    }

    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => Self::parse(&contents),
            Err(_) => Self::default_config(),
        }
    }

    /// `None` means the screensaver is disabled (`screensaver=0`).
    pub fn screensaver_duration(&self) -> Option<std::time::Duration> {
        if self.screensaver_secs == 0 {
            None
        } else {
            Some(std::time::Duration::from_secs(self.screensaver_secs as u64))
        }
    }
}

/// `/etc/argonrtc.conf`: EON RTC daily wake/sleep schedule. Not a
/// Python-daemon legacy format (undocumented upstream, W§1.1 only
/// describes the register map) — new to `argonone-rs`, kept in the same
/// `key=value` style as the other config files for consistency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcSchedule {
    pub enabled: bool,
    pub wake_hour: u8,
    pub wake_minute: u8,
    /// Daily poweroff time, checked against the RTC's own clock (W§1.1) —
    /// `None` if `sleep=` wasn't set, meaning no scheduled poweroff.
    pub sleep: Option<(u8, u8)>,
}

impl RtcSchedule {
    pub fn disabled() -> Self {
        RtcSchedule {
            enabled: false,
            wake_hour: 0,
            wake_minute: 0,
            sleep: None,
        }
    }

    fn parse_hh_mm(value: &str) -> Option<(u8, u8)> {
        let (h, m) = value.split_once(':')?;
        let (h, m) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
        (h < 24 && m < 60).then_some((h, m))
    }

    pub fn parse(contents: &str) -> Self {
        let mut schedule = Self::disabled();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "enabled" => schedule.enabled = matches!(value, "Y" | "y" | "1" | "true"),
                "wake" => {
                    if let Some((h, m)) = Self::parse_hh_mm(value) {
                        schedule.wake_hour = h;
                        schedule.wake_minute = m;
                    }
                }
                "sleep" => schedule.sleep = Self::parse_hh_mm(value),
                _ => {}
            }
        }
        schedule
    }

    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => Self::parse(&contents),
            Err(_) => Self::disabled(),
        }
    }
}

/// Well-known paths, matching the Python daemon's layout exactly so an
/// existing install's config carries over unmodified.
pub struct ConfigPaths;

impl ConfigPaths {
    pub const CPU_CURVE: &'static str = "/etc/argononed.conf";
    pub const HDD_CURVE: &'static str = "/etc/argononed-hdd.conf";
    pub const UNITS: &'static str = "/etc/argonunits.conf";
    pub const OLED: &'static str = "/etc/argoneonoled.conf";
    pub const RTC_SCHEDULE: &'static str = "/etc/argonrtc.conf";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_descending_curve_regardless_of_file_order() {
        let curve = FanCurve::parse("55=30\n65=100\n60=55\n").unwrap();
        assert_eq!(
            curve.0,
            vec![
                CurvePoint {
                    temp_c: 65,
                    speed_pct: 100
                },
                CurvePoint {
                    temp_c: 60,
                    speed_pct: 55
                },
                CurvePoint {
                    temp_c: 55,
                    speed_pct: 30
                },
            ]
        );
    }

    #[test]
    fn skips_blank_lines_and_comments() {
        let curve = FanCurve::parse("# comment\n\n65=100\n").unwrap();
        assert_eq!(curve.0.len(), 1);
    }

    #[test]
    fn speed_for_picks_highest_met_threshold() {
        let curve = FanCurve::default_curve();
        assert_eq!(curve.speed_for(70.0), 100);
        assert_eq!(curve.speed_for(62.0), 55);
        assert_eq!(curve.speed_for(56.0), 30);
        assert_eq!(curve.speed_for(40.0), 0);
    }

    #[test]
    fn default_curve_does_not_violate_safety_floor() {
        assert!(!FanCurve::default_curve().violates_safety_floor());
    }

    #[test]
    fn empty_curve_violates_safety_floor() {
        // No points at all -> speed_for always returns 0, well under the
        // 25% floor at the 75C checkpoint.
        assert!(FanCurve(vec![]).violates_safety_floor());
    }

    #[test]
    fn zero_percent_at_high_temp_violates_safety_floor() {
        let curve = FanCurve(vec![CurvePoint {
            temp_c: 90,
            speed_pct: 0,
        }]);
        assert!(curve.violates_safety_floor());
    }

    #[test]
    fn gap_between_safe_points_above_ceiling_is_caught() {
        // 75C has only 10% (unsafe); the higher 95C point can't paper
        // over the unsafe gap in between, since real temps sit at 75-94C
        // too before ever reaching 95C.
        let curve = FanCurve(vec![
            CurvePoint {
                temp_c: 95,
                speed_pct: 100,
            },
            CurvePoint {
                temp_c: 75,
                speed_pct: 10,
            },
        ]);
        assert!(curve.violates_safety_floor());
    }

    #[test]
    fn adequate_speed_at_and_above_ceiling_passes() {
        let curve = FanCurve(vec![CurvePoint {
            temp_c: 75,
            speed_pct: 25,
        }]);
        assert!(!curve.violates_safety_floor());
    }

    #[test]
    fn rejects_malformed_line() {
        assert!(FanCurve::parse("not-a-line").is_err());
    }

    #[test]
    fn clamps_speed_over_100() {
        let curve = FanCurve::parse("50=150").unwrap();
        assert_eq!(curve.0[0].speed_pct, 100);
    }

    #[test]
    fn load_or_default_falls_back_when_file_missing() {
        let curve = FanCurve::load_or_default(Path::new("/nonexistent/argononed.conf")).unwrap();
        assert_eq!(curve, FanCurve::default_curve());
    }

    #[test]
    fn load_or_default_parses_existing_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "65=100\n55=30\n").unwrap();
        let curve = FanCurve::load_or_default(file.path()).unwrap();
        assert_eq!(curve.0.len(), 2);
    }

    #[test]
    fn load_or_default_reports_parse_errors() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "not-a-line\n").unwrap();
        let err = FanCurve::load_or_default(file.path()).unwrap_err();
        assert!(err.message.contains("expected `temp=speed`"));
        assert_eq!(err.path, file.path().display().to_string());
    }

    #[test]
    fn config_error_display_includes_path_and_message() {
        let err = ConfigError {
            path: "/etc/argononed.conf".to_string(),
            message: "boom".to_string(),
        };
        assert_eq!(err.to_string(), "/etc/argononed.conf: boom");
    }

    #[test]
    fn temp_unit_defaults_to_celsius_when_file_missing() {
        let unit = TempUnit::load_or_default(Path::new("/nonexistent/argonunits.conf"));
        assert_eq!(unit, TempUnit::Celsius);
    }

    #[test]
    fn temp_unit_reads_fahrenheit_from_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "temperature=F\n").unwrap();
        assert_eq!(TempUnit::load_or_default(file.path()), TempUnit::Fahrenheit);
    }

    #[test]
    fn convert_c_is_identity_for_celsius() {
        assert_eq!(TempUnit::Celsius.convert_c(0.0), 0.0);
        assert_eq!(TempUnit::Celsius.convert_c(100.0), 100.0);
    }

    #[test]
    fn convert_c_converts_to_fahrenheit() {
        assert_eq!(TempUnit::Fahrenheit.convert_c(0.0), 32.0);
        assert_eq!(TempUnit::Fahrenheit.convert_c(100.0), 212.0);
    }

    #[test]
    fn suffix_matches_unit() {
        assert_eq!(TempUnit::Celsius.suffix(), "C");
        assert_eq!(TempUnit::Fahrenheit.suffix(), "F");
    }

    #[test]
    fn temp_unit_defaults_to_celsius_for_unknown_value() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "temperature=K\n").unwrap();
        assert_eq!(TempUnit::load_or_default(file.path()), TempUnit::Celsius);
    }

    #[test]
    fn oled_config_parses_quoted_screenlist() {
        let cfg = OledConfig::parse(
            "switchduration=15\nscreensaver=60\nscreenlist=\"clock ip cpu\"\nenabled=Y\n",
        );
        assert_eq!(cfg.switch_duration_secs, 15);
        assert_eq!(cfg.screensaver_secs, 60);
        assert_eq!(cfg.screenlist, "clock ip cpu");
        assert!(cfg.enabled);
    }

    #[test]
    fn oled_config_missing_file_falls_back_to_default() {
        let cfg = OledConfig::load_or_default(Path::new("/nonexistent/argoneonoled.conf"));
        assert_eq!(cfg, OledConfig::default_config());
    }

    #[test]
    fn oled_config_screensaver_zero_means_disabled() {
        let cfg = OledConfig::parse("screensaver=0\n");
        assert_eq!(cfg.screensaver_duration(), None);
    }

    #[test]
    fn oled_config_screensaver_nonzero_converts_to_duration() {
        let cfg = OledConfig::parse("screensaver=45\n");
        assert_eq!(
            cfg.screensaver_duration(),
            Some(std::time::Duration::from_secs(45))
        );
    }

    #[test]
    fn oled_config_disabled_flag_parses_n() {
        let cfg = OledConfig::parse("enabled=N\n");
        assert!(!cfg.enabled);
    }

    #[test]
    fn rtc_schedule_parses_wake_and_sleep_time() {
        let schedule = RtcSchedule::parse("enabled=Y\nwake=07:30\nsleep=23:00\n");
        assert!(schedule.enabled);
        assert_eq!(schedule.wake_hour, 7);
        assert_eq!(schedule.wake_minute, 30);
        assert_eq!(schedule.sleep, Some((23, 0)));
    }

    #[test]
    fn rtc_schedule_rejects_out_of_range_time() {
        let schedule = RtcSchedule::parse("wake=25:99\n");
        assert_eq!(schedule.wake_hour, 0);
        assert_eq!(schedule.wake_minute, 0);
    }

    #[test]
    fn rtc_schedule_sleep_defaults_to_none() {
        let schedule = RtcSchedule::parse("wake=07:30\n");
        assert_eq!(schedule.sleep, None);
    }

    #[test]
    fn rtc_schedule_missing_file_is_disabled() {
        let schedule = RtcSchedule::load_or_default(Path::new("/nonexistent/argonrtc.conf"));
        assert_eq!(schedule, RtcSchedule::disabled());
    }
}

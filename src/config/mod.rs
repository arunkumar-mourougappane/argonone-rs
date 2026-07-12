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
}

/// Well-known paths, matching the Python daemon's layout exactly so an
/// existing install's config carries over unmodified.
pub struct ConfigPaths;

impl ConfigPaths {
    pub const CPU_CURVE: &'static str = "/etc/argononed.conf";
    pub const HDD_CURVE: &'static str = "/etc/argononed-hdd.conf";
    pub const UNITS: &'static str = "/etc/argonunits.conf";
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
    fn rejects_malformed_line() {
        assert!(FanCurve::parse("not-a-line").is_err());
    }

    #[test]
    fn clamps_speed_over_100() {
        let curve = FanCurve::parse("50=150").unwrap();
        assert_eq!(curve.0[0].speed_pct, 100);
    }
}

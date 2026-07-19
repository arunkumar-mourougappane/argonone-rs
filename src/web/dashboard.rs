//! Authenticated landing page — the app shell (sidebar/status-strip) is
//! shared with the fan/storage/system pages via `app_shell.html`. Full
//! card-grid dashboard (Fan control, Power & RTC, Network, Storage,
//! Display, System, Signed-in-as), mirroring `03-dashboard.html` — each
//! card reuses the same data source its own dedicated page already uses,
//! rather than inventing a second copy of that logic.

use super::AppState;
use super::templates::render;
use crate::auth::AuthSession;
use crate::hardware::RtcDateTime;
use crate::hardware::board::Board;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::Serialize;

/// Local weekday/hour/minute via the `date` command — matches this
/// codebase's existing "shell out to a system tool" convention
/// (`df`/`smartctl`/`mdadm`/`tailscale` are all called the same way)
/// rather than pulling in `time`'s `local-offset` feature, which is
/// gated off by default for cross-thread soundness reasons. Only used
/// for this card's display-only "next wake/sleep" computation — the
/// real RTC wake-alarm programming (`service::run`) reads the actual
/// hardware RTC, not this.
fn local_weekday_hour_minute() -> Option<(u8, u8, u8)> {
    let output = std::process::Command::new("date")
        .arg("+%u %H %M")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut parts = text.split_whitespace();
    let iso_weekday: u8 = parts.next()?.parse().ok()?; // 1=Monday..7=Sunday
    let hour: u8 = parts.next()?.parse().ok()?;
    let minute: u8 = parts.next()?.parse().ok()?;
    // RtcDateTime/schedule convention: 0=Sunday..6=Saturday.
    let weekday = if iso_weekday == 7 { 0 } else { iso_weekday };
    Some((weekday, hour, minute))
}

const DAY_ABBREV: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn format_occurrence(weekday: u8, hour: u8, minute: u8) -> String {
    format!(
        "{} {hour:02}:{minute:02}",
        DAY_ABBREV.get(weekday as usize).copied().unwrap_or("?"),
    )
}

fn usage_severity(pct: u8) -> &'static str {
    if pct >= 90 {
        "crit"
    } else if pct >= 70 {
        "warn"
    } else {
        "good"
    }
}

/// The RAID level to show next to a disk's name, if any of its members
/// belong to an array. `member_level` is ordered the same as
/// `sysinfo::read_raid_status()`'s array list, and this returns the
/// *first* match deterministically — a plain `Vec` scan, not a
/// `HashMap` lookup, so a disk with members in more than one array
/// always reports the same array's level rather than one that depends
/// on that process's randomized hash-iteration order.
fn disk_raid_level(member_level: &[(String, String)], disk_name: &str) -> Option<String> {
    member_level
        .iter()
        .find(|(dev, _)| crate::sysinfo::filesystem_belongs_to_device(dev, disk_name))
        .map(|(_, lvl)| lvl.clone())
}

#[derive(Debug, Serialize)]
struct DashDiskRow {
    /// e.g. `"sda (RAID1)"` for a RAID member, plain `"mmcblk0"` otherwise.
    label: String,
    used_pct: Option<u8>,
    severity: &'static str,
    temp_display: Option<String>,
}

pub async fn show(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        // require_login should already have caught this, but a handler
        // shouldn't assume its middleware always ran.
        return Redirect::to("/login").into_response();
    };
    let is_eon = state.board == Board::Eon;
    let has_case = state.board != Board::NoCase;
    let board_label = match state.board {
        Board::NoCase => "No case detected",
        Board::One => "Argon ONE",
        Board::Eon => "Argon EON",
    };

    // Fan control card: current applied speed vs. what the active CPU
    // curve would target at the current temperature — "ramping" is
    // exactly that mismatch, without needing the control loop's own
    // private hysteresis state (which the web layer never sees).
    let cpu_curve = state.cpu_curve_tx.borrow().clone();
    let cpu_temp_c = crate::sysinfo::read_cpu_temp_c();
    let current_fan_pct = *state.fan_speed.borrow();
    let target_fan_pct = cpu_temp_c.map(|t| cpu_curve.speed_for(t));

    // Power & RTC card (EON only) — "next wake"/"next sleep" resolved
    // against local wall-clock time, same schedule-matching logic
    // (`rtc_schedule::next_wake`/`next_sleep`) the real RTC-alarm
    // programming uses.
    let (next_wake, next_sleep) = if is_eon {
        let schedule = crate::db::settings::load_rtc_schedule(&state.pool).await;
        match local_weekday_hour_minute() {
            Some((weekday, hour, minute)) => {
                let now = RtcDateTime {
                    year: 0,
                    month: 0,
                    day: 0,
                    weekday,
                    hour,
                    minute,
                    second: 0,
                };
                (
                    crate::rtc_schedule::next_wake(&schedule.entries, now)
                        .map(|(w, h, m)| format_occurrence(w, h, m)),
                    crate::rtc_schedule::next_sleep(&schedule.entries, now)
                        .map(|(w, h, m)| format_occurrence(w, h, m)),
                )
            }
            None => (None, None),
        }
    } else {
        (None, None)
    };

    // Storage card: same snapshot/usage/RAID-membership sources the
    // Storage & RAID page (v0.4.0, `src/web/storage.rs`) reads, just
    // summarized to one line per disk instead of that page's fuller table.
    let unit = crate::db::settings::load_units(&state.pool).await;
    let snapshot = crate::sysinfo::read_storage_snapshot().await;
    let usage = crate::sysinfo::read_disk_usage();
    let raid = crate::sysinfo::read_raid_status();
    // A `Vec`, not a `HashMap`: a whole-disk device can, in principle,
    // have members in more than one array (e.g. separate partitions each
    // in their own RAID set), and `filesystem_belongs_to_device`'s prefix
    // match means more than one entry here can match the same disk. A
    // `HashMap`'s iteration order is randomized per-process, so picking
    // "the" match out of one would show a different array's level on
    // every restart — this preserves `raid`'s own array order instead,
    // so the same disk always reports the same (first) array's level.
    let mut member_level: Vec<(String, String)> = Vec::new();
    for array in &raid {
        for device in &array.devices {
            member_level.push((device.name.clone(), array.level.clone()));
        }
    }
    let disk_rows: Vec<DashDiskRow> = snapshot
        .into_iter()
        .map(|d| {
            let used_pct = usage
                .iter()
                .find(|u| {
                    crate::sysinfo::filesystem_belongs_to_device(&u.filesystem, &d.device.name)
                })
                .map(|u| u.used_pct);
            let level = disk_raid_level(&member_level, &d.device.name);
            let label = match level {
                Some(lvl) => format!("{} ({})", d.device.name, lvl.to_uppercase()),
                None => d.device.name.clone(),
            };
            DashDiskRow {
                label,
                used_pct,
                severity: used_pct.map(usage_severity).unwrap_or("good"),
                temp_display: d
                    .temp_c
                    .map(|c| format!("{:.0}\u{b0}{}", unit.convert_c(c), unit.suffix())),
            }
        })
        .collect();

    // Signed in as card — `auth::User` (the session-hot-path struct)
    // deliberately skips first/last name and last-login, so this reuses
    // `db::users::UserRow`'s fuller column set instead.
    let full_user = crate::db::users::get_user(&state.pool, user.id)
        .await
        .ok()
        .flatten();

    let html: Html<String> = render(
        &state.env,
        "dashboard.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "dashboard",
            is_eon => is_eon,
            has_case => has_case,
            board_label => board_label,
            os => crate::sysinfo::os_release_pretty_name(),
            version => env!("CARGO_PKG_VERSION"),
            unit => match unit { crate::config::TempUnit::Celsius => "Celsius", crate::config::TempUnit::Fahrenheit => "Fahrenheit" },
            current_fan_pct => current_fan_pct,
            target_fan_pct => target_fan_pct,
            cpu_temp_c => cpu_temp_c,
            next_wake => next_wake,
            next_sleep => next_sleep,
            disk_rows => disk_rows,
            full_user => full_user,
        },
    );
    html.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_occurrence_pads_and_labels_the_day() {
        assert_eq!(format_occurrence(1, 6, 5), "Mon 06:05");
        assert_eq!(format_occurrence(0, 23, 0), "Sun 23:00");
    }

    #[test]
    fn local_weekday_hour_minute_returns_values_in_range() {
        // Time-dependent (shells out to `date`), so this only checks the
        // parse produced a plausible result, not a specific value.
        let (weekday, hour, minute) = local_weekday_hour_minute().expect("`date` should succeed");
        assert!(weekday <= 6);
        assert!(hour <= 23);
        assert!(minute <= 59);
    }

    #[test]
    fn usage_severity_bands_match_storage_page_thresholds() {
        assert_eq!(usage_severity(50), "good");
        assert_eq!(usage_severity(70), "warn");
        assert_eq!(usage_severity(90), "crit");
    }

    #[test]
    fn disk_raid_level_is_deterministic_when_a_disk_has_members_in_two_arrays() {
        // sda1 in md0 (raid1), sda2 in md1 (raid0) — both belong to the
        // whole disk "sda" under filesystem_belongs_to_device's prefix
        // match. Run the lookup many times: with the old HashMap-backed
        // version this would occasionally surface "raid0" instead of
        // "raid1" depending on that process's randomized hash seed: here
        // it must always return the first array's level, every time.
        let member_level = vec![
            ("sda1".to_string(), "raid1".to_string()),
            ("sda2".to_string(), "raid0".to_string()),
        ];
        for _ in 0..50 {
            assert_eq!(
                disk_raid_level(&member_level, "sda"),
                Some("raid1".to_string())
            );
        }
    }

    #[test]
    fn disk_raid_level_is_none_for_a_disk_with_no_raid_members() {
        let member_level = vec![("sda1".to_string(), "raid1".to_string())];
        assert_eq!(disk_raid_level(&member_level, "sdb"), None);
    }
}

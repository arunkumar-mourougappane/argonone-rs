//! Storage & RAID page (v0.4.0, mirrors `05-storage-raid.html`) —
//! server-rendered only, no REST endpoint of its own: nothing else needs
//! this data live over the API contract this milestone (W§2.5's table
//! doesn't list one), so a page refresh is enough to see current state.

use super::AppState;
use super::templates::render;
use crate::auth::AuthSession;
use axum::extract::State;
use axum::response::{Html, IntoResponse, Redirect, Response};
use minijinja::context;
use serde::Serialize;

/// Usage-percent severity banding for the progress bar/role badge —
/// matches the green/yellow/red convention `argondashboard.py` already
/// used for temperature and RAID health (W§3.4).
fn usage_severity(pct: u8) -> &'static str {
    if pct >= 90 {
        "crit"
    } else if pct >= 70 {
        "warn"
    } else {
        "good"
    }
}

/// Disk temperature severity banding, in Celsius regardless of display
/// unit — thresholds are a hardware property, not something that should
/// shift with the operator's C/F preference.
fn temp_severity(celsius: f32) -> &'static str {
    if celsius >= 55.0 {
        "crit"
    } else if celsius >= 45.0 {
        "warn"
    } else {
        "good"
    }
}

#[derive(Debug, Serialize)]
struct DiskRow {
    name: String,
    size_gb: f64,
    model: Option<String>,
    /// Pre-formatted in the operator's unit preference (System page,
    /// v0.4.0) — e.g. "34.0°C"/"93.2°F" — so the template doesn't need
    /// its own conversion (and can't independently forget to convert).
    temp_display: Option<String>,
    temp_severity: &'static str,
    used_pct: Option<u8>,
    usage_severity: &'static str,
    /// "RAID member" if this device backs any array below, else
    /// "NN% full" once usage is known, else empty (no badge).
    role_label: String,
    role_severity: &'static str,
}

#[derive(Debug, Serialize)]
struct RaidRow {
    name: String,
    level: String,
    state: String,
    working_disks: u8,
    raid_disks: u8,
    failed_disks: u8,
    spare_disks: usize,
    size_gb: Option<f64>,
    devices: Vec<String>,
}

pub async fn page(auth_session: AuthSession, State(state): State<AppState>) -> Response {
    let Some(user) = auth_session.user else {
        return Redirect::to("/login").into_response();
    };

    let unit = crate::db::settings::load_units(&state.pool).await;
    let snapshot = crate::sysinfo::read_storage_snapshot().await;
    let usage = crate::sysinfo::read_disk_usage();
    let raid = crate::sysinfo::read_raid_status();

    // Membership check reuses filesystem_belongs_to_device: a RAID
    // member is recorded as a partition name (e.g. "sda1"), and "does
    // this partition name belong to this whole-disk device" is exactly
    // that same prefix relationship.
    let raid_member_names: Vec<String> = raid
        .iter()
        .flat_map(|a| a.devices.iter().map(|d| d.name.clone()))
        .collect();

    let disks: Vec<DiskRow> = snapshot
        .into_iter()
        .map(|d| {
            // Match against `df`'s Filesystem column (e.g. `/dev/sda1`),
            // not its mount path — a whole-disk device name like "sda"
            // essentially never appears in a mount path, but always
            // prefixes its own partitions' device names.
            let used_pct = usage
                .iter()
                .find(|u| {
                    crate::sysinfo::filesystem_belongs_to_device(&u.filesystem, &d.device.name)
                })
                .map(|u| u.used_pct);

            let is_raid_member = raid_member_names
                .iter()
                .any(|m| crate::sysinfo::filesystem_belongs_to_device(m, &d.device.name));
            let (role_label, role_severity) = if is_raid_member {
                ("RAID member".to_string(), "good")
            } else if let Some(pct) = used_pct {
                (format!("{pct}% full"), usage_severity(pct))
            } else {
                (String::new(), "good")
            };

            DiskRow {
                name: d.device.name,
                size_gb: d.device.size_bytes as f64 / 1_000_000_000.0,
                model: d.device.model,
                temp_display: d
                    .temp_c
                    .map(|c| format!("{:.1}\u{b0}{}", unit.convert_c(c), unit.suffix())),
                temp_severity: d.temp_c.map(temp_severity).unwrap_or("good"),
                used_pct,
                usage_severity: used_pct.map(usage_severity).unwrap_or("good"),
                role_label,
                role_severity,
            }
        })
        .collect();

    let raid_rows: Vec<RaidRow> = raid
        .into_iter()
        .map(|a| RaidRow {
            name: a.name.clone(),
            level: a.level.clone(),
            state: a.state.clone(),
            working_disks: a.working_disks,
            raid_disks: a.raid_disks,
            failed_disks: a.failed_disks(),
            spare_disks: a.spare_disks(),
            size_gb: a.size_kb.map(|kb| kb as f64 / 1_000_000.0),
            devices: a.devices.iter().map(|d| d.name.clone()).collect(),
        })
        .collect();

    let html: Html<String> = render(
        &state.env,
        "storage.html",
        context! {
            username => user.username,
            role => user.role().as_str(),
            active_page => "storage",
            disks => disks,
            raid_arrays => raid_rows,
        },
    );
    html.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_severity_bands_at_70_and_90() {
        assert_eq!(usage_severity(0), "good");
        assert_eq!(usage_severity(69), "good");
        assert_eq!(usage_severity(70), "warn");
        assert_eq!(usage_severity(89), "warn");
        assert_eq!(usage_severity(90), "crit");
        assert_eq!(usage_severity(100), "crit");
    }

    #[test]
    fn temp_severity_bands_at_45_and_55() {
        assert_eq!(temp_severity(0.0), "good");
        assert_eq!(temp_severity(44.9), "good");
        assert_eq!(temp_severity(45.0), "warn");
        assert_eq!(temp_severity(54.9), "warn");
        assert_eq!(temp_severity(55.0), "crit");
        assert_eq!(temp_severity(80.0), "crit");
    }
}

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

#[derive(Debug, Serialize)]
struct DiskRow {
    name: String,
    size_gb: f64,
    model: Option<String>,
    temp_c: Option<f32>,
    used_pct: Option<u8>,
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

    let snapshot = crate::sysinfo::read_storage_snapshot().await;
    let usage = crate::sysinfo::read_disk_usage();
    let raid = crate::sysinfo::read_raid_status();

    let disks: Vec<DiskRow> = snapshot
        .into_iter()
        .map(|d| {
            // Best-effort match against `df`'s mount-keyed usage: a whole
            // disk device (e.g. "sda") won't itself have a mount, but its
            // partitions might — this is a coarse "does anything on this
            // device look full" signal, not a precise per-partition table.
            let used_pct = usage
                .iter()
                .find(|u| u.mount.contains(&d.device.name))
                .map(|u| u.used_pct);
            DiskRow {
                name: d.device.name,
                size_gb: d.device.size_bytes as f64 / 1_000_000_000.0,
                model: d.device.model,
                temp_c: d.temp_c,
                used_pct,
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

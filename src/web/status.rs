//! `GET /api/status` (W§2.5): one-shot snapshot doubling as a health
//! check — `hardware` reflects whether a case was actually detected, not
//! just "the process is running."

use super::AppState;
use crate::hardware::board::Board;
use crate::sysinfo;
use axum::Json;
use axum::extract::State;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    status: &'static str,
    hardware: &'static str,
    board: &'static str,
    cpu_pct: Option<f32>,
    cpu_temp_c: Option<f32>,
    ram_used_pct: Option<f32>,
    fan_pct: u8,
}

pub async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let hardware = match state.board {
        Board::NoCase => "absent",
        Board::One | Board::Eon => "ok",
    };
    let board = match state.board {
        Board::NoCase => "none",
        Board::One => "one",
        Board::Eon => "eon",
    };

    let mut cpu = sysinfo::CpuUsage::new();
    cpu.sample_percent();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let cpu_pct = cpu.sample_percent();

    Json(StatusResponse {
        status: "ok",
        hardware,
        board,
        cpu_pct,
        cpu_temp_c: sysinfo::read_cpu_temp_c(),
        ram_used_pct: sysinfo::read_mem_info().map(|m| m.used_percent()),
        fan_pct: *state.fan_speed.borrow(),
    })
}

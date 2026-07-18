//! `GET /api/ws` (W§2.5): one shared connection pushing tagged-JSON
//! `stats`/`fan_state` messages — server-push only, no client-to-server
//! messages in this contract (writes go through REST once they exist).

use super::AppState;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use serde_json::json;

pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream(socket, state))
}

async fn stream(mut socket: WebSocket, state: AppState) {
    let mut cpu = crate::sysinfo::CpuUsage::new();
    let mut interval = tokio::time::interval(super::WS_TICK_INTERVAL);
    let mut oled_screen_rx = state.oled_screen.clone();

    // Send the currently-selected screen right away (v0.5.0's live OLED
    // preview, W§2.5) rather than making a freshly-connected client wait
    // for the next rotation to learn what's already showing.
    {
        let screen = *oled_screen_rx.borrow_and_update();
        let oled_screen = json!({ "type": "oled_screen", "name": screen.map(|s| s.name()) });
        if socket
            .send(Message::Text(oled_screen.to_string().into()))
            .await
            .is_err()
        {
            return;
        }
    }

    loop {
        tokio::select! {
            Ok(()) = oled_screen_rx.changed() => {
                let screen = *oled_screen_rx.borrow_and_update();
                let oled_screen = json!({ "type": "oled_screen", "name": screen.map(|s| s.name()) });
                if socket.send(Message::Text(oled_screen.to_string().into())).await.is_err() {
                    return;
                }
            }
            _ = interval.tick() => {
                let unit = match *state.units_tx.borrow() {
                    crate::config::TempUnit::Celsius => "C",
                    crate::config::TempUnit::Fahrenheit => "F",
                };
                let mem = crate::sysinfo::read_mem_info();
                let stats = json!({
                    "type": "stats",
                    "cpu_pct": cpu.sample_percent(),
                    // Always Celsius — pair with `unit` (the operator's
                    // display preference) to convert client-side, same
                    // contract as GET /api/status.
                    "cpu_temp_c": crate::sysinfo::read_cpu_temp_c(),
                    "unit": unit,
                    "ram_used_pct": mem.map(|m| m.used_percent()),
                    "ram_used_kb": mem.map(|m| m.total_kb.saturating_sub(m.available_kb)),
                    "ram_total_kb": mem.map(|m| m.total_kb),
                    // Rarely change tick-to-tick, but re-read each time
                    // rather than caching — a DHCP renewal or long-running
                    // process shouldn't require a page reload to notice.
                    "ip_address": crate::sysinfo::read_local_ip().map(|ip| ip.to_string()),
                    "uptime_secs": crate::sysinfo::read_uptime_secs(),
                });
                if socket.send(Message::Text(stats.to_string().into())).await.is_err() {
                    return;
                }

                let fan_state = json!({
                    "type": "fan_state",
                    "curve": "cpu",
                    "current_pct": *state.fan_speed.borrow(),
                });
                if socket.send(Message::Text(fan_state.to_string().into())).await.is_err() {
                    return;
                }
            }
            msg = socket.recv() => {
                // No client -> server messages in this contract (W§2.5);
                // anything else (close frame, error, stream end) means
                // the connection is done.
                if !matches!(msg, Some(Ok(_))) {
                    return;
                }
            }
        }
    }
}

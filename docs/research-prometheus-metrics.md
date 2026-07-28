# Research: Prometheus Metrics Endpoint

Speculative — not on [docs/ROADMAP.md](ROADMAP.md) yet. Written after v0.7.0 shipped,
in response to "what should we build next": this app's target audience (README's
"homelab/NAS self-hosters") very plausibly already runs Prometheus + Grafana, and
right now the only way to see trends over time is the dashboard's live, in-browser
sparklines — nothing survives a page reload or is visible from outside a browser tab.

## 1. What this app already collects (no new sampling code needed)

Every candidate metric already has a live reader, used today by
[`src/web/status.rs`](../src/web/status.rs) (`GET /api/status`) and
[`src/web/ws.rs`](../src/web/ws.rs) (the dashboard's live WebSocket feed):

| Data | Source |
|---|---|
| CPU %, CPU temp (°C) | `sysinfo::CpuUsage`, `sysinfo::read_cpu_temp_c` |
| RAM used/total, swap used % | `sysinfo::read_mem_info` |
| Load average (1/5/15m) | `sysinfo::read_load_avg` |
| Network rx/tx rates | `sysinfo::NetUsage::sample_rates` |
| Fan target/current % | `fan::FanState` (via `fan_state` watch channel) |
| Per-disk usage/temp, RAID state | `src/web/storage.rs`'s existing collectors |
| HTTPS cert expiry | `src/https.rs`'s cert-status parsing (W§4.4 follow-up) |

So this is purely a **new export surface**, not new instrumentation — every number
a `/metrics` endpoint would report is already computed somewhere for the dashboard
or `/api/status`.

## 2. Crate choice

Checked current versions on crates.io (2026-07-27):

| Crate | Version | What it is |
|---|---|---|
| `metrics` | 0.24.6 | Facade crate — `counter!`/`gauge!`/`histogram!` macros, backend-agnostic |
| `metrics-exporter-prometheus` | 0.18.3 | A `metrics`-compatible recorder that renders Prometheus text format |
| `axum-prometheus` | 0.10.0 | Tower middleware that auto-instruments HTTP request count/latency/status for Axum |
| `prometheus-client` | 0.25.0 | Official OpenMetrics client — typed metric structs registered into a `Registry`, no macros |

`axum-prometheus` is the wrong shape for this: it instruments HTTP traffic
(requests/latency/status codes), which is not the interesting data here — nobody
wants a Grafana panel for `/api/ws` request latency, they want CPU temp and fan
speed over time. It could be added *later*, additively, if HTTP-level observability
becomes wanted, but it isn't the v1 ask.

Between `metrics`+`metrics-exporter-prometheus` and `prometheus-client`: the
`metrics` facade wins here because instrumentation call sites end up as
one-line macro calls dropped directly into the existing `sysinfo`/`fan`/`storage`
read functions, rather than threading typed `Gauge`/`Family` handles through
`AppState` and updating them by hand in three different places (status.rs,
ws.rs, and now metrics.rs) — less duplication for the same set of numbers three
call sites already read independently.

`metrics-exporter-prometheus`'s `PrometheusBuilder` does not require running its
own bundled HTTP server: `PrometheusBuilder::new().install_recorder()` returns a
`PrometheusHandle` whose `.render()` produces the scrape text on demand. That
handle can be stored in `AppState` and called from an ordinary Axum route —
consistent with this app owning exactly one HTTP listener (`src/https.rs`'s mode
dispatch), never a second port for metrics. The one operational cost: bypassing
the crate's own server means this app becomes responsible for periodically
calling `handle.run_upkeep()` (histogram/quantile bookkeeping) — a small
`tokio::spawn`'d interval loop, the same shape as the webhook notifier's
transition-tracking background task already in `src/notify/mod.rs`.

**Recommendation:** `metrics` 0.24 + `metrics-exporter-prometheus` 0.18, mounted
as a route in the existing Axum router, not a second listener.

## 3. Auth — the actual hard part

Every route in this app requires an interactive login session
(`middleware::from_fn(login::require_login)`, applied to the whole `protected`
router in [`src/web/mod.rs`](../src/web/mod.rs)). A Prometheus scrape target
can't do cookie-based login — this is the one real design decision here, not
the metrics plumbing itself.

Options considered:

- **Put it in `protected`** — rejected outright, a scrape target has no session cookie.
- **Put it in `public`, fully unauthenticated** — rejected. Even though Prometheus
  metrics are conventionally exposed without auth (network-boundary trust is the
  usual model), this app already treats comparable exposure as sensitive: the
  webhook URL is admin-only "since a webhook URL can itself be a bearer secret"
  ([`src/web/notifications.rs`](../src/web/notifications.rs)). Hostname, uptime,
  IP, per-disk serials-in-labels, and cert-expiry timing are a meaningful
  information-disclosure surface for an app that defaults to binding `0.0.0.0`.
- **Admin-configured static bearer token, checked in front of `/metrics` only**
  (recommended) — same shape as the webhook's own secret-as-config pattern:
  a token stored via `db/settings.rs`, checked by a small middleware layer that
  sits *outside* the session-based `require_login` layer (a scrape client sends
  `Authorization: Bearer <token>` per-request, never a cookie), rotatable/
  visible only to an admin session in the web UI. `/metrics` becomes its own
  third router alongside today's `public`/`protected` split — public in the
  sense of "no cookie required," but gated by its own bearer check, not open.

## 4. Candidate metric names

Prometheus convention: `snake_case`, unit-suffixed, one `argonone_` prefix.

```
argonone_cpu_temp_celsius
argonone_cpu_usage_percent
argonone_ram_used_bytes / argonone_ram_total_bytes
argonone_swap_used_percent
argonone_load_average{period="1m|5m|15m"}
argonone_fan_percent{curve="cpu|hdd"}
argonone_disk_temp_celsius{device="sda"}
argonone_disk_usage_percent{device="sda"}
argonone_raid_state{array="md0"}          # gauge, enum-style (0=ok,1=degraded,2=failed)
argonone_network_rx_bytes_total / _tx_bytes_total
argonone_https_cert_expiry_seconds
argonone_webhook_notifications_total      # counter, matches src/notify/mod.rs's AlertTracker transitions
```

Per-disk/per-array labels mean cardinality stays tied to physical hardware count
(typically 1-4 disks on this class of device) — not a real cardinality risk.

## 5. Do we need a mockup?

**No new mockup file for the scrape endpoint itself** — `GET /metrics` is a
machine-readable text response, not a page a human browses; nothing in
`docs/mockups/` documents a non-visual API response and this shouldn't be the
first.

**A small settings surface is still needed** for the enable/token-rotate control
(mirrors why Notifications got its own admin-only config, not just a hardcoded
always-on webhook). Two shapes, in ascending order of how deserving of a
dedicated mockup this is:

1. **A new card on the existing `/system` page** (same pattern as its HTTPS/IR
   cards) — enable toggle + token display/rotate + a `curl` example. No new
   mockup needed; follows `04-fan-curve-editor.html`-adjacent styling already
   established on that page. **This is what I'd actually build** — the config
   surface here (one toggle, one token) is smaller than Notifications'
   (webhook URL + severity thresholds + test-send), which is the bar that
   justified Notifications getting its own sidebar page + `13-notifications.html`.
2. If this grows into its own sidebar page later (e.g. multiple scrape tokens,
   per-metric enable/disable), *then* it crosses the same threshold Logs and
   Notifications did, and should get a `docs/mockups/14-metrics.html` at that
   point — per the standing "mockups are source of truth" convention, that
   mockup should exist and be matched *before* building the page, not
   retrofitted after.

## 6. Open questions (deliberately unresolved — not scoped yet)

- Does `run_upkeep()`'s cadence matter for gauges-only usage (no histograms
  planned in v1), or can it be skipped entirely until a histogram metric is added?
- Should `argonone_raid_state` be a numeric enum gauge (Prometheus convention)
  or should degraded/failed instead just tie into the existing webhook
  notification path rather than needing a Grafana alert rule at all — two
  different tools solving the same "tell me when this goes bad" problem.
- Packaging: no new `.deb` dependency (no new listener, no new port to open
  in any firewall/udev config) — confirmed by design in §2, not something
  that needs re-checking once implemented.

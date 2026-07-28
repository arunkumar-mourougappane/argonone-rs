# Research: Notification Target Formats (Discord / ntfy)

Speculative — not on [docs/ROADMAP.md](ROADMAP.md) yet. Follow-up prompted by
checking whether v0.7.0's webhook notifier ([`src/notify/mod.rs`](../src/notify/mod.rs))
actually delivers usable notifications to the integrations its own UI advertises,
not just a generic claim taken at face value.

## 1. What's claimed today vs. what actually happens

[`templates/notifications.html`](../templates/notifications.html) (and
[`docs/mockups/13-notifications.html`](../mockups/13-notifications.html), which
even uses `https://ntfy.sh/your-topic` as the field's placeholder) both label the
webhook URL field: *"Discord, ntfy, Home Assistant, or anything accepting a plain
JSON POST."* [`notify::send`](../src/notify/mod.rs) backs that claim with exactly
one fixed body shape, for every target:

```json
{"source": "cpu_temp", "severity": "crit", "message": "CPU temperature reached 80.1°C..."}
```

Checked each named integration's actual expected request format:

| Target | What it actually needs | What happens with today's generic body |
|---|---|---|
| **Home Assistant** (webhook automation trigger) | Any JSON — the automation reads whatever keys you send | Works as claimed. No gap here. |
| **ntfy** simple publish (`POST https://ntfy.sh/{topic}`, the placeholder's own URL shape) | Raw request body *is* the message text, sent as-is, no JSON parsing | "Works" in the sense that a notification arrives, but its body is the literal JSON string `{"source":"cpu_temp","severity":"crit","message":"..."}` — not a clean message. Functional, but not the experience the placeholder implies. |
| **ntfy** JSON publish (`POST https://ntfy.sh/` root, not a per-topic URL) | JSON body **requires** a `topic` field, plus optional `message`/`title`/`priority` | Not used at all today — the placeholder points at the per-topic simple-POST URL, not the JSON API's root URL, so this path isn't actually reachable with the current UI copy. |
| **Discord** (Execute Webhook endpoint) | JSON body must include at least one of `content`/`embeds`/`components`/`file`/`poll` | **Fails outright.** Discord rejects the request (400) — none of those keys exist in the generic body. This claim is currently just wrong. |

So of the three named integrations, one works as documented (Home Assistant),
one technically delivers but degrades to an ugly raw-JSON message body (ntfy),
and one doesn't work at all (Discord).

## 2. What would actually fix it

Small, additive change — doesn't touch the transition-tracking logic in
`AlertTracker` at all, only how `send()` builds the outgoing body:

- Add a `format` field to `WebhookConfig` (`src/notify/mod.rs`): an enum,
  `Generic | Discord | Ntfy`, defaulting to `Generic` (today's behavior,
  unchanged for anyone not touching the new setting — covers Home Assistant
  and "anything accepting a plain JSON POST" as before).
- `Discord`: body becomes `{"content": alert.message}` (using `source`/`severity`
  to prefix the message text, e.g. `[crit] cpu_temp: CPU temperature reached
  80.1°C...`, since Discord has no structured-field concept for those outside
  `embeds`).
- `Ntfy`: send the *plain-text* `alert.message` as the raw request body (not
  JSON at all) — this is simpler than adopting the JSON API, and it's the
  shape the existing placeholder URL (`https://ntfy.sh/your-topic`) already
  expects. Optionally set `X-Title`/`X-Priority` headers (ntfy reads priority/
  title from headers on the simple endpoint too) using `source`/`severity`.
- UI: one new `<select>` next to the existing URL field in
  `templates/notifications.html` — no new page, no new mockup file needed;
  this is a same-scale addition to notifications' existing single-page config,
  not a new feature surface the way Logs/Notifications themselves were.

## 3. Open questions

- Is `Generic` (today's raw JSON) worth keeping as a fourth explicit option
  once `Discord`/`Ntfy` exist, or does everyone who wanted it just pick `Ntfy`
  or `Discord` and nobody actually depends on the raw shape? Leaning toward
  keeping it — Home Assistant and "anything else" still want the structured
  JSON, and defaulting new installs to `Generic` avoids a breaking change for
  anyone who already configured a URL under v0.7.0's one-shape behavior.
- Should `test_webhook` (the existing "Send test notification" button in
  `src/web/notifications.rs`) send through the selected format, or always send
  the generic shape? Almost certainly the former — testing the wrong body
  shape would validate nothing useful once formats diverge.
- Not investigated: Slack's incoming-webhook format (`{"text": "..."}`) is
  similarly simple to Discord's and would be a near-zero-cost fourth option if
  this gets built — worth a one-line mention when implementing, not its own
  research pass.

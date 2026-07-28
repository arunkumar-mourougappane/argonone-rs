# Research: Scheduled / Quiet-Hours Fan Profile

Speculative — not on [docs/ROADMAP.md](ROADMAP.md) yet. Combines two
subsystems that already exist independently — the fan curve editor and the
RTC day-of-week schedule table — into "run quieter at night," without
needing new architecture for either half.

## 1. This does not need EON/RTC hardware at all

Easy trap to fall into: the existing day-of-week schedule
(`RtcScheduleEntry`, `src/rtc_schedule.rs`) is EON-only, because it's
ultimately about arming the PCF8563 RTC chip's physical wake-alarm — real
hardware only EON has. Quiet hours is a different kind of "scheduled," with
no hardware dependency at all: it's purely "is the current wall-clock time
inside a configured window," checked from software on every fan-loop poll
— the same **already-established pattern**
[`src/web/dashboard.rs::local_weekday_hour_minute`](../src/web/dashboard.rs)
uses (`std::process::Command::new("date").arg("+%u %H %M")`), just called
from the 30s control-loop tick instead of a page render. This means quiet
hours should work identically on an **Argon ONE or a bare Pi with no case**
— it must not be gated behind `is_eon`, unlike the Power & RTC schedule it
otherwise superficially resembles.

## 2. Reusing the day-bitmask convention, not the RTC entry type itself

`RtcScheduleEntry{kind, days (bitmask), hour, minute}` and its matching
logic (`rtc_schedule::matching_sleep_entry`, `next_occurrence`) are good
precedent for the *bitmask + hour/minute* shape, but quiet hours is a
window (start → end), not an instant trigger like Wake/Sleep — so it needs
its own small type, not a third `RtcEventKind` variant:

```rust
pub struct QuietHoursConfig {
    pub enabled: bool,
    pub days: u8,          // same bitmask convention: bit 0 = Sunday .. bit 6 = Saturday
    pub start_hour: u8, pub start_minute: u8,
    pub end_hour: u8, pub end_minute: u8,
    pub max_pct: u8,       // fan speed cap while the window is active
}
```

Matching needs to handle an overnight window (e.g. 22:00 → 06:00, crossing
midnight) — the same minute-of-week wraparound arithmetic
`rtc_schedule::next_occurrence` already implements for finding the next
Wake/Sleep entry, just checking "is `now` between start and end" instead of
"what's the next occurrence."

## 3. The safety-critical part: never let quiet hours suppress the safety floor

This is the one finding worth being careful about. `FanCurve::speed_for`
([`src/config/mod.rs`](../src/config/mod.rs)) has **no runtime floor
enforcement** — `violates_safety_floor()` is checked only at *save time*
(rejecting a curve whose points would dip below 25% at ≥75°C), so a curve
already accepted is trusted to behave at runtime. If a quiet-hours cap were
applied as a blind `.min(max_pct)` on top of `speed_for`'s result, a
low-enough `max_pct` (or a legitimate one during a genuine thermal event)
could suppress the fan below the 25%-at-75°C guarantee the whole safety
floor mechanism exists for — silently defeating it during exactly the hours
someone's asleep and least likely to notice a hot Pi.

**The fix is a hard rule, not a configurable one**: the cap only ever
applies below `config::SAFETY_CEILING_C` (75°C), and only to the CPU
curve's *own* contribution — **never** to `floor_pct` (the HDD curve's
folded-in demand). This second part matters just as much as the first: an
earlier draft of this doc capped the two demands' combined `.max()` result,
which meant a hot disk during quiet hours could get capped down along with
everything else — silently defeating the *other* existing "higher demand
wins" guarantee (`04-fan-curve-editor.html`'s documented CPU/HDD behavior),
not just the CPU safety floor. Both floors — the 75°C CPU one and the
HDD-demand one — must survive the cap independently, and neither is
something an operator's quiet-hours setting should be able to override,
configured value or not.

Concretely, inside [`FanController::tick_with_floor`](../src/fan/mod.rs)
(or a new sibling `tick_with_floor_and_ceiling`, folding the cap in at
this same point so the controller's own hysteresis state —
`current_speed`, `pending_decrease` — stays consistent with what's
actually applied, rather than being clamped externally after the fact and
drifting out of sync with what the controller thinks it last set):

```rust
let curve_speed = self.curve.speed_for(temp);
let capped_curve_speed = if temp >= SAFETY_CEILING_C {
    curve_speed // never cap in the CPU safety-critical range
} else {
    curve_speed.min(quiet_hours_cap.unwrap_or(100))
};
let target = capped_curve_speed.max(floor_pct); // HDD demand always wins over the cap too — same position it already occupies today
```

Cap the CPU curve's own output first, *then* fold in the HDD floor via the
same `.max()` this app's control loop already uses — never the other way
around. This is a stronger and simpler guarantee than "validate the
configured cap is ≥25%" would be — it doesn't matter what cap an operator
picks, because the cap structurally cannot reach the temperature range the
CPU floor protects, and structurally cannot outrank whatever the HDD curve
is separately demanding.

## 4. Where it plugs into the existing control loop

`service::run`'s poll tick ([`src/service.rs`](../src/service.rs), around
the `fan_controller.tick_with_floor(...)` call) already computes `floor`
from the HDD curve each cycle. Quiet-hours' active/inactive state and cap
value would be resolved the same tick, from a `watch::channel` following
this app's own established live-apply pattern (same as `cpu_curve_rx`/
`hdd_curve_rx`/`webhook_config_rx` already do) — a PUT to
`/api/settings/quiet-hours` pushes a new `QuietHoursConfig` in without a
restart, same idiom as every other setting.

## 5. Mockup

One new supplementary mockup:
[`docs/mockups/19-quiet-hours.html`](mockups/19-quiet-hours.html) — a new
card on the existing Fan control page (`04-fan-curve-editor.html`'s shell),
below the curve editor: enable toggle, day-of-week chip selector (reusing
the same Mon–Sun chip convention, visually, as the RTC schedule table's
"Days" column), start/end time inputs, and a max-speed field — plus an
inline note citing §3's safety-floor guarantee, since that's the one thing
an operator setting this up actually needs reassurance about ("will this
let my Pi overheat quietly at 3am").

## 6. Open questions

- Should quiet hours be a single global window (simplest, proposed above)
  or support multiple windows like the RTC schedule's multi-entry table
  does? Leaning toward single-window for a first version — "quieter at
  night" is the actual use case motivating this; multiple disjoint windows
  is speculative-on-top-of-speculative until someone asks for it.
- **Resolved during review, not left as an open question**: an earlier
  draft of §3 applied the cap to the *combined* `curve.max(floor_pct)`
  result, which would have let quiet hours suppress a legitimate
  HDD-driven demand along with everything else — the same class of bug
  the CPU safety floor guarantee was written to prevent, just for the
  disk-temp case instead. Fixed by capping the CPU curve's own output
  *before* folding in the HDD floor via `.max()`, so the HDD floor — like
  the CPU safety ceiling — always wins over the cap regardless of its
  configured value. Same "higher demand wins" principle
  `docs/ROADMAP.md`'s v0.6.0 entry already documents for the CPU/HDD
  relationship, correctly extended this time to include the cap as a third
  input that only ever *lowers* the CPU curve's own baseline, never
  outranks either floor.
- One consequence worth stating plainly for the mockup: because the cap
  applies to the overall physical fan output (never scoped to "just the
  CPU curve" or "just the HDD curve" from the operator's point of view —
  it's one fan, one speed), the Quiet Hours card correctly stays visible
  and unchanged regardless of which curve tab (CPU/HDD) is selected on
  the Fan control page. It isn't hidden or altered when viewing the HDD
  tab — nothing about it is CPU-specific; it's a constraint on the
  control loop's *final* output, downstream of whichever curve is
  currently driving the demand.

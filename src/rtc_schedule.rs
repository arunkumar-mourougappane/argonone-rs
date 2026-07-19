//! Pure RTC schedule-matching logic (v0.5.0) — kept free of I2C access so
//! it's testable without hardware, mirroring `src/fan/mod.rs`'s split
//! between pure curve logic and the I/O that applies it.
//!
//! The PCF8563 has exactly one alarm slot (`src/hardware/rtc.rs`): it can
//! hold "fire every day" or "fire on one specific weekday", never an
//! arbitrary multi-day set at once. [`next_wake`] is what resolves a full
//! multi-entry, day-of-week schedule down to the single next occurrence to
//! arm into that one slot.

use crate::config::{RtcEventKind, RtcScheduleEntry};
use crate::hardware::RtcDateTime;

const MINUTES_PER_WEEK: i32 = 7 * 24 * 60;

fn week_minutes(weekday: u8, hour: u8, minute: u8) -> i32 {
    weekday as i32 * 24 * 60 + hour as i32 * 60 + minute as i32
}

/// The soonest entry of `kind` strictly after `after`, wrapping at the
/// week boundary if nothing matches later this week. Returns `(weekday,
/// hour, minute)`. `None` if there are no entries of that kind (or none
/// with any day bit set).
pub fn next_occurrence(
    entries: &[RtcScheduleEntry],
    after: RtcDateTime,
    kind: RtcEventKind,
) -> Option<(u8, u8, u8)> {
    let after_minutes = week_minutes(after.weekday, after.hour, after.minute);
    let mut best: Option<(i32, u8, u8, u8)> = None;

    for entry in entries.iter().filter(|e| e.kind == kind) {
        for weekday in 0..7u8 {
            if entry.days & (1 << weekday) == 0 {
                continue;
            }
            let entry_minutes = week_minutes(weekday, entry.hour, entry.minute);
            let mut delta = entry_minutes - after_minutes;
            if delta <= 0 {
                delta += MINUTES_PER_WEEK;
            }
            let is_better = match best {
                Some((best_delta, ..)) => delta < best_delta,
                None => true,
            };
            if is_better {
                best = Some((delta, weekday, entry.hour, entry.minute));
            }
        }
    }

    best.map(|(_, weekday, hour, minute)| (weekday, hour, minute))
}

/// The soonest `Wake` entry strictly after `after` — exactly the
/// arguments [`crate::hardware::RtcBackend::set_wake_alarm`] needs.
pub fn next_wake(entries: &[RtcScheduleEntry], after: RtcDateTime) -> Option<(u8, u8, u8)> {
    next_occurrence(entries, after, RtcEventKind::Wake)
}

/// The soonest `Sleep` entry strictly after `after` — the dashboard's
/// "Next sleep" row (v0.6.0). Unlike [`matching_sleep_entry`] (an exact
/// "does `now` match" check the shutdown-trigger poll uses), this is a
/// forward-looking "when next" query, the same shape [`next_wake`]
/// already answers for wake alarms.
pub fn next_sleep(entries: &[RtcScheduleEntry], after: RtcDateTime) -> Option<(u8, u8, u8)> {
    next_occurrence(entries, after, RtcEventKind::Sleep)
}

/// The `Sleep` entry (if any) matching `now` exactly — same day-of-week and
/// hour:minute. At most the caller cares about *whether* one matched, but
/// the entry itself is returned in case a future caller wants its detail.
pub fn matching_sleep_entry(
    entries: &[RtcScheduleEntry],
    now: RtcDateTime,
) -> Option<&RtcScheduleEntry> {
    entries.iter().find(|e| {
        e.kind == RtcEventKind::Sleep
            && e.days & (1 << now.weekday) != 0
            && e.hour == now.hour
            && e.minute == now.minute
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RtcEventKind;

    fn dt(weekday: u8, hour: u8, minute: u8) -> RtcDateTime {
        RtcDateTime {
            year: 2026,
            month: 1,
            day: 1,
            weekday,
            hour,
            minute,
            second: 0,
        }
    }

    fn wake(days: u8, hour: u8, minute: u8) -> RtcScheduleEntry {
        RtcScheduleEntry {
            kind: RtcEventKind::Wake,
            days,
            hour,
            minute,
        }
    }

    fn sleep(days: u8, hour: u8, minute: u8) -> RtcScheduleEntry {
        RtcScheduleEntry {
            kind: RtcEventKind::Sleep,
            days,
            hour,
            minute,
        }
    }

    #[test]
    fn next_wake_picks_later_time_same_day() {
        let entries = vec![wake(0x7f, 7, 0)];
        // Tuesday (weekday=2) at 06:00 -> same-day 07:00 wake.
        assert_eq!(next_wake(&entries, dt(2, 6, 0)), Some((2, 7, 0)));
    }

    #[test]
    fn next_wake_wraps_to_next_matching_day_when_todays_time_passed() {
        let entries = vec![wake(0x7f, 7, 0)];
        // Tuesday at 08:00 -> today's 07:00 already passed, wraps to Wednesday.
        assert_eq!(next_wake(&entries, dt(2, 8, 0)), Some((3, 7, 0)));
    }

    #[test]
    fn next_wake_respects_day_mask_weekdays_only() {
        // Mon-Fri only: bits 1..5 (Mon=1 .. Fri=5).
        let weekdays_mask = 0b0111110;
        let entries = vec![wake(weekdays_mask, 7, 0)];
        // Saturday (weekday=6) -> soonest match is Monday (weekday=1).
        assert_eq!(next_wake(&entries, dt(6, 12, 0)), Some((1, 7, 0)));
    }

    #[test]
    fn next_wake_wraps_at_week_boundary() {
        let entries = vec![wake(0x7f, 7, 0)];
        // Saturday (weekday=6) at 23:00 -> soonest is Sunday (weekday=0) 07:00.
        assert_eq!(next_wake(&entries, dt(6, 23, 0)), Some((0, 7, 0)));
    }

    #[test]
    fn next_wake_picks_soonest_across_multiple_entries() {
        let entries = vec![wake(0x7f, 9, 0), wake(0x7f, 6, 30)];
        assert_eq!(next_wake(&entries, dt(2, 5, 0)), Some((2, 6, 30)));
    }

    #[test]
    fn next_wake_ignores_sleep_entries() {
        let entries = vec![sleep(0x7f, 6, 0)];
        assert_eq!(next_wake(&entries, dt(2, 5, 0)), None);
    }

    #[test]
    fn next_wake_none_when_no_entries() {
        assert_eq!(next_wake(&[], dt(2, 5, 0)), None);
    }

    #[test]
    fn next_sleep_picks_soonest_and_ignores_wake_entries() {
        let entries = vec![wake(0x7f, 6, 30), sleep(0x7f, 23, 0)];
        assert_eq!(next_sleep(&entries, dt(2, 5, 0)), Some((2, 23, 0)));
    }

    #[test]
    fn next_sleep_wraps_to_next_week_when_nothing_later_this_week() {
        // Only fires Mondays at 23:00; asking from Monday 23:30 should
        // wrap forward to next Monday, not return None.
        let entries = vec![sleep(0b0000010, 23, 0)];
        assert_eq!(next_sleep(&entries, dt(1, 23, 30)), Some((1, 23, 0)));
    }

    #[test]
    fn matching_sleep_entry_finds_exact_time_and_day() {
        let entries = vec![sleep(0x7f, 23, 0)];
        assert!(matching_sleep_entry(&entries, dt(3, 23, 0)).is_some());
        assert!(matching_sleep_entry(&entries, dt(3, 23, 1)).is_none());
    }

    #[test]
    fn matching_sleep_entry_respects_day_mask() {
        // Only Sunday (bit 0).
        let entries = vec![sleep(0b0000001, 23, 0)];
        assert!(matching_sleep_entry(&entries, dt(0, 23, 0)).is_some());
        assert!(matching_sleep_entry(&entries, dt(1, 23, 0)).is_none());
    }

    #[test]
    fn matching_sleep_entry_ignores_wake_entries() {
        let entries = vec![wake(0x7f, 23, 0)];
        assert!(matching_sleep_entry(&entries, dt(2, 23, 0)).is_none());
    }
}

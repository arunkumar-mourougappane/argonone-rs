//! Critical-state webhook notification (v0.7.0, W§3.8) — this app's first
//! outbound network call, so scoped deliberately: one operator-set URL,
//! POSTed once per severity transition into critical, covering exactly
//! the three triggers `13-notifications.html` documents (CPU temp,
//! disk temp, RAID degraded). Not a multi-channel alerting engine — a
//! value that stays critical doesn't re-notify until it clears and
//! re-triggers, which is what [`AlertTracker`] exists to track.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One operator-configured webhook — off by default, matching the
/// mockup's "this is the only outbound network call this app makes"
/// framing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebhookConfig {
    pub enabled: bool,
    pub url: Option<String>,
}

/// One transition-into-critical event, ready to POST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub source: &'static str,
    pub message: String,
}

/// Tracks each monitored source's last-seen critical/non-critical state
/// so [`check`](AlertTracker::check) only reports a *transition* into
/// critical, not every poll where a value happens to still be critical.
/// Purely in-memory (like `service::run`'s other one-shot trigger state,
/// e.g. `rtc_last_sleep_trigger`) — resetting on daemon restart just
/// means a still-critical value re-notifies once after restart, which is
/// the safer default for a monitoring feature.
#[derive(Debug, Default)]
pub struct AlertTracker {
    cpu_temp_crit: bool,
    disk_temp_crit: HashMap<String, bool>,
    raid_degraded: HashMap<String, bool>,
}

impl AlertTracker {
    /// `disks` is `(device name, temperature)`; `raid` is `(array name,
    /// is degraded)`. Returns every source that just transitioned from
    /// non-critical to critical this call.
    pub fn check(
        &mut self,
        cpu_temp_c: Option<f32>,
        disks: &[(String, Option<f32>)],
        raid: &[(String, bool)],
    ) -> Vec<Alert> {
        let mut alerts = Vec::new();

        let cpu_crit = cpu_temp_c.is_some_and(|t| t >= crate::config::SAFETY_CEILING_C);
        if cpu_crit && !self.cpu_temp_crit {
            alerts.push(Alert {
                source: "cpu_temp",
                message: format!(
                    "CPU temperature reached {:.1}\u{b0}C (>= {:.0}\u{b0}C critical threshold)",
                    cpu_temp_c.unwrap(),
                    crate::config::SAFETY_CEILING_C
                ),
            });
        }
        self.cpu_temp_crit = cpu_crit;

        for (name, temp_c) in disks {
            let crit = temp_c.is_some_and(|t| crate::web::storage::temp_severity(t) == "crit");
            let was_crit = self.disk_temp_crit.get(name).copied().unwrap_or(false);
            if crit && !was_crit {
                alerts.push(Alert {
                    source: "disk_temp",
                    message: format!(
                        "Disk {name} temperature reached {:.1}\u{b0}C",
                        temp_c.unwrap()
                    ),
                });
            }
            self.disk_temp_crit.insert(name.clone(), crit);
        }

        for (name, degraded) in raid {
            let was_degraded = self.raid_degraded.get(name).copied().unwrap_or(false);
            if *degraded && !was_degraded {
                alerts.push(Alert {
                    source: "raid_degraded",
                    message: format!("RAID array {name} is degraded"),
                });
            }
            self.raid_degraded.insert(name.clone(), *degraded);
        }

        alerts
    }
}

/// POSTs a plain JSON body — no vendor-specific envelope, matching the
/// mockup's "Discord, ntfy, Home Assistant, or anything accepting a
/// plain JSON POST" framing. A 2xx response is the only thing checked;
/// callers log failures but never retry (an opt-in best-effort notifier,
/// not a delivery-guaranteed queue).
pub async fn send(url: &str, alert: &Alert) -> Result<(), String> {
    let body = serde_json::json!({
        "source": alert.source,
        "severity": "crit",
        "message": alert.message,
    });
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("webhook returned HTTP {}", response.status()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_temp_alert_fires_once_on_transition_into_critical() {
        let mut tracker = AlertTracker::default();
        assert!(tracker.check(Some(50.0), &[], &[]).is_empty());
        let alerts = tracker.check(Some(80.0), &[], &[]);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].source, "cpu_temp");
        // Stays critical -> no re-fire.
        assert!(tracker.check(Some(85.0), &[], &[]).is_empty());
        // Clears, then re-crosses -> fires again.
        assert!(tracker.check(Some(50.0), &[], &[]).is_empty());
        assert_eq!(tracker.check(Some(80.0), &[], &[]).len(), 1);
    }

    #[test]
    fn disk_temp_alerts_are_tracked_independently_per_disk() {
        let mut tracker = AlertTracker::default();
        let disks = vec![
            ("sda".to_string(), Some(60.0)),
            ("sdb".to_string(), Some(30.0)),
        ];
        let alerts = tracker.check(None, &disks, &[]);
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].message.contains("sda"));

        // sdb crosses later -> its own alert, sda doesn't refire.
        let disks = vec![
            ("sda".to_string(), Some(60.0)),
            ("sdb".to_string(), Some(60.0)),
        ];
        let alerts = tracker.check(None, &disks, &[]);
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].message.contains("sdb"));
    }

    #[test]
    fn raid_degraded_alert_fires_on_transition() {
        let mut tracker = AlertTracker::default();
        let raid = vec![("md0".to_string(), false)];
        assert!(tracker.check(None, &[], &raid).is_empty());

        let raid = vec![("md0".to_string(), true)];
        let alerts = tracker.check(None, &[], &raid);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].source, "raid_degraded");

        // Still degraded -> no re-fire.
        assert!(tracker.check(None, &[], &raid).is_empty());
    }

    #[test]
    fn no_alerts_when_nothing_is_critical() {
        let mut tracker = AlertTracker::default();
        let disks = vec![("sda".to_string(), Some(30.0))];
        let raid = vec![("md0".to_string(), false)];
        assert!(tracker.check(Some(40.0), &disks, &raid).is_empty());
    }
}

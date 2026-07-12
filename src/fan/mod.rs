//! Temperature→fan-speed control loop (`argononed.py` thread parity):
//! poll every 30s, apply the configured curve, but hold off on *decreasing*
//! speed until the lower reading has been sustained for a full hysteresis
//! window — otherwise the fan flaps audibly on every temperature blip
//! (W§1.4). Increases always apply immediately; only decreases are held.

use crate::config::FanCurve;
use crate::hardware::FanBackend;
use std::time::{Duration, Instant};

pub const POLL_INTERVAL: Duration = Duration::from_secs(30);
pub const DECREASE_HYSTERESIS: Duration = Duration::from_secs(30);

pub trait TempSource: Send {
    fn read_cpu_temp_c(&mut self) -> Option<f32>;
}

pub struct FanController<T: TempSource> {
    curve: FanCurve,
    temp_source: T,
    current_speed: u8,
    pending_decrease: Option<PendingDecrease>,
}

struct PendingDecrease {
    target: u8,
    since: Instant,
}

impl<T: TempSource> FanController<T> {
    pub fn new(curve: FanCurve, temp_source: T) -> Self {
        FanController {
            curve,
            temp_source,
            current_speed: 0,
            pending_decrease: None,
        }
    }

    /// Run one poll cycle: read temp, decide the new speed per the
    /// hysteresis rule, and apply it via `backend` if it changed. Returns
    /// the speed actually applied (or the unchanged current speed).
    pub fn tick(&mut self, backend: &dyn FanBackend, now: Instant) -> u8 {
        let Some(temp) = self.temp_source.read_cpu_temp_c() else {
            tracing::warn!("fan control: temperature unavailable this poll, holding current speed");
            return self.current_speed;
        };
        let target = self.curve.speed_for(temp);

        let new_speed = if target >= self.current_speed {
            self.pending_decrease = None;
            target
        } else {
            match &self.pending_decrease {
                Some(pending)
                    if pending.target == target
                        && now.duration_since(pending.since) >= DECREASE_HYSTERESIS =>
                {
                    self.pending_decrease = None;
                    target
                }
                Some(pending) if pending.target == target => self.current_speed,
                _ => {
                    self.pending_decrease = Some(PendingDecrease { target, since: now });
                    self.current_speed
                }
            }
        };

        if new_speed != self.current_speed {
            if let Err(e) = backend.set_speed(new_speed) {
                tracing::error!(error = %e, "failed to apply fan speed");
                return self.current_speed;
            }
            tracing::info!(
                temp_c = temp,
                from = self.current_speed,
                to = new_speed,
                "fan speed changed"
            );
            self.current_speed = new_speed;
        }
        self.current_speed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{FanCapability, HwResult};

    struct FixedTemp(f32);
    impl TempSource for FixedTemp {
        fn read_cpu_temp_c(&mut self) -> Option<f32> {
            Some(self.0)
        }
    }

    struct RecordingFan(std::sync::Mutex<Vec<u8>>);
    impl FanBackend for RecordingFan {
        fn capability(&self) -> FanCapability {
            FanCapability::Registers
        }
        fn set_speed(&self, percent: u8) -> HwResult<()> {
            self.0.lock().unwrap().push(percent);
            Ok(())
        }
        fn signal_poweroff(&self) -> HwResult<()> {
            Ok(())
        }
    }

    #[test]
    fn increase_applies_immediately() {
        let mut ctl = FanController::new(FanCurve::default_curve(), FixedTemp(70.0));
        let fan = RecordingFan(std::sync::Mutex::new(vec![]));
        let speed = ctl.tick(&fan, Instant::now());
        assert_eq!(speed, 100);
        assert_eq!(fan.0.lock().unwrap().as_slice(), &[100]);
    }

    #[test]
    fn decrease_is_held_until_hysteresis_window_elapses() {
        let mut ctl = FanController::new(FanCurve::default_curve(), FixedTemp(70.0));
        let fan = RecordingFan(std::sync::Mutex::new(vec![]));
        let t0 = Instant::now();
        assert_eq!(ctl.tick(&fan, t0), 100);

        // Temp drops; the request to decrease shouldn't apply immediately.
        ctl.temp_source = FixedTemp(40.0);
        let held = ctl.tick(&fan, t0 + Duration::from_secs(5));
        assert_eq!(
            held, 100,
            "speed should still be held at 100 before hysteresis elapses"
        );

        // After the hysteresis window, with the same lower target sustained,
        // the decrease should apply.
        let applied = ctl.tick(&fan, t0 + Duration::from_secs(35));
        assert_eq!(applied, 0);
    }

    #[test]
    fn missing_temp_reading_holds_current_speed() {
        struct FlakyTemp(bool);
        impl TempSource for FlakyTemp {
            fn read_cpu_temp_c(&mut self) -> Option<f32> {
                if self.0 { Some(70.0) } else { None }
            }
        }
        let mut ctl = FanController::new(FanCurve::default_curve(), FlakyTemp(true));
        let fan = RecordingFan(std::sync::Mutex::new(vec![]));
        assert_eq!(ctl.tick(&fan, Instant::now()), 100);
        ctl.temp_source = FlakyTemp(false);
        assert_eq!(ctl.tick(&fan, Instant::now()), 100);
    }
}

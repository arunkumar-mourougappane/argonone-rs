//! No-op hardware backends — the seam that lets the daemon run (and be
//! tested) on a Pi without the Argon case attached, per W§1.4.

use super::{ButtonEvent, FanBackend, FanCapability, HwResult, PowerButtonBackend};
use tokio::sync::mpsc;

pub struct NoopFan;

impl FanBackend for NoopFan {
    fn capability(&self) -> FanCapability {
        FanCapability::None
    }

    fn set_speed(&self, _percent: u8) -> HwResult<()> {
        Ok(())
    }

    fn signal_poweroff(&self) -> HwResult<()> {
        Ok(())
    }
}

pub struct NoopPowerButton;

impl PowerButtonBackend for NoopPowerButton {
    fn spawn(self: Box<Self>) -> mpsc::Receiver<ButtonEvent> {
        // Sender is dropped immediately; the receiver simply never yields.
        let (_tx, rx) = mpsc::channel(1);
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_fan_reports_no_capability_and_always_succeeds() {
        let fan = NoopFan;
        assert_eq!(fan.capability(), FanCapability::None);
        assert!(fan.set_speed(50).is_ok());
        assert!(fan.signal_poweroff().is_ok());
    }

    #[tokio::test]
    async fn noop_power_button_channel_closes_immediately() {
        let mut rx = Box::new(NoopPowerButton).spawn();
        assert_eq!(rx.recv().await, None);
    }
}

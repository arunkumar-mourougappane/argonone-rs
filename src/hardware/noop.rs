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

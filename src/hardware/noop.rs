//! No-op hardware backends — the seam that lets the daemon run (and be
//! tested) on a Pi without the Argon case attached, per W§1.4.

use super::{
    ButtonEvent, FanBackend, FanCapability, HwResult, OledBackend, PowerButtonBackend, RtcBackend,
    RtcDateTime,
};
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

pub struct NoopOled;

impl OledBackend for NoopOled {
    fn render(
        &mut self,
        _screen: crate::oled::Screen,
        _data: &crate::oled::OledData,
    ) -> HwResult<()> {
        Ok(())
    }

    fn clear(&mut self) -> HwResult<()> {
        Ok(())
    }
}

pub struct NoopRtc;

impl RtcBackend for NoopRtc {
    fn read_time(&mut self) -> HwResult<RtcDateTime> {
        Err(super::HwError::Bus("no RTC attached".to_string()))
    }

    fn set_wake_alarm(&mut self, _hour: u8, _minute: u8) -> HwResult<()> {
        Err(super::HwError::Bus("no RTC attached".to_string()))
    }

    fn clear_alarm(&mut self) -> HwResult<()> {
        Err(super::HwError::Bus("no RTC attached".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_oled_render_and_clear_always_succeed() {
        let mut oled = NoopOled;
        let data = crate::oled::OledData {
            now_utc: time::OffsetDateTime::UNIX_EPOCH,
            ip: None,
            cpu_pct: None,
            cpu_temp_c: None,
            ram_pct: None,
            disks: vec![],
            raid: vec![],
            unit: crate::config::TempUnit::Celsius,
            pi_model: None,
        };
        assert!(oled.render(crate::oled::Screen::Clock, &data).is_ok());
        assert!(oled.clear().is_ok());
    }

    #[test]
    fn noop_rtc_reports_unavailable() {
        let mut rtc = NoopRtc;
        assert!(rtc.read_time().is_err());
        assert!(rtc.set_wake_alarm(7, 30).is_err());
        assert!(rtc.clear_alarm().is_err());
    }

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

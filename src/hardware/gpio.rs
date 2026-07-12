//! Power-button GPIO monitor (`argonpowerbutton-libgpiod.py` parity).
//! BCM pin 4, pulse width on the button-press cycle classifies the action:
//! 20-30ms = reboot, 40-50ms = shutdown, 60-70ms = OLED screen switch
//! (W§1.1). Uses the character-device v2 uAPI (`gpiod` crate) rather than
//! replicating the old sysfs/RPi.GPIO paths — no reason to carry that
//! forward in a fresh implementation.

use super::{ButtonEvent, HwError, HwResult, PowerButtonBackend};
use gpiod::{Chip, Edge, EdgeDetect, Options};
use std::time::Duration;
use tokio::sync::mpsc;

const BUTTON_LINE_OFFSET: u32 = 4;
/// Pi 5 renumbered gpiochips; Pi 4 and earlier expose the SoC's own lines
/// on gpiochip0. Try both, first match wins.
const CANDIDATE_CHIPS: &[&str] = &["gpiochip4", "gpiochip0"];

pub struct GpiodPowerButton {
    chip_name: &'static str,
}

impl GpiodPowerButton {
    /// Confirm a chip with the expected line is actually present before
    /// committing to it — cheap enough to just open+request here.
    pub fn open() -> HwResult<Self> {
        for &name in CANDIDATE_CHIPS {
            let chip = match Chip::new(name) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let opts = Options::input([BUTTON_LINE_OFFSET])
                .edge(EdgeDetect::Both)
                .consumer("argonone-rs-powerbutton");
            if chip.request_lines(opts).is_ok() {
                return Ok(GpiodPowerButton { chip_name: name });
            }
        }
        Err(HwError::Bus(format!(
            "no GPIO chip among {CANDIDATE_CHIPS:?} exposes line {BUTTON_LINE_OFFSET}"
        )))
    }
}

impl PowerButtonBackend for GpiodPowerButton {
    fn spawn(self: Box<Self>) -> mpsc::Receiver<ButtonEvent> {
        let (tx, rx) = mpsc::channel(8);
        let chip_name = self.chip_name;
        std::thread::spawn(move || monitor_loop(chip_name, tx));
        rx
    }
}

fn monitor_loop(chip_name: &str, tx: mpsc::Sender<ButtonEvent>) {
    let chip = match Chip::new(chip_name) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(chip_name, error = %e, "power button monitor: chip reopen failed");
            return;
        }
    };
    let opts = Options::input([BUTTON_LINE_OFFSET])
        .edge(EdgeDetect::Both)
        .consumer("argonone-rs-powerbutton");
    let mut lines = match chip.request_lines(opts) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "power button monitor: line request failed");
            return;
        }
    };

    let mut press_start: Option<Duration> = None;
    loop {
        let event = match lines.read_event() {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(error = %e, "power button monitor: read_event failed, stopping");
                return;
            }
        };

        match event.edge {
            Edge::Rising => press_start = Some(event.time),
            Edge::Falling => {
                let Some(start) = press_start.take() else {
                    continue;
                };
                let width = event.time.saturating_sub(start);
                if let Some(action) = classify(width)
                    && tx.blocking_send(action).is_err()
                {
                    // Receiver dropped — daemon shutting down.
                    return;
                }
            }
        }
    }
}

/// Bucket a pulse width into the button action it represents, matching the
/// Python daemon's 10ms-tick ranges (W§1.1). Widths outside every bucket
/// are noise/bounce and ignored.
fn classify(width: Duration) -> Option<ButtonEvent> {
    let ms = width.as_millis();
    match ms {
        20..=30 => Some(ButtonEvent::Reboot),
        40..=50 => Some(ButtonEvent::Shutdown),
        60..=70 => Some(ButtonEvent::OledSwitch),
        _ => None,
    }
}

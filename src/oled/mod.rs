//! EON OLED support: screen content, the screen-rotation state machine, and
//! the splash screen — all built against `embedded-graphics`'s `DrawTarget`
//! trait so it's testable without a real panel (W§1.2, §1.5, §1.7).
//!
//! Fonts/backgrounds are *not* Argon40's originals: per the licensing
//! research (W§1.5), this project regenerates dashboard content from
//! `embedded-graphics`'s bundled, permissively-licensed monospace fonts
//! instead of vendoring or fetching Argon40's `.bin` assets. That also means
//! there's no reason to replicate Argon40's bespoke per-plane font packing
//! (W§1.7) — we own the whole rendering path end to end, so a plain
//! `DrawTarget`-based blitter is simpler and just as correct.

pub mod render;
pub mod splash;

use std::time::{Duration, Instant};

/// One EON dashboard screen. `Splash` is the boot/rotation-switch screen
/// (W§1.5's resolution), the rest mirror the Python daemon's `screenlist`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    Clock,
    Ip,
    Cpu,
    Ram,
    Storage,
    Temp,
    Raid,
    Splash,
}

impl Screen {
    fn from_token(token: &str) -> Option<Screen> {
        match token.trim().to_ascii_lowercase().as_str() {
            "clock" => Some(Screen::Clock),
            "ip" => Some(Screen::Ip),
            "cpu" => Some(Screen::Cpu),
            "ram" => Some(Screen::Ram),
            "storage" => Some(Screen::Storage),
            "temp" => Some(Screen::Temp),
            "raid" => Some(Screen::Raid),
            "splash" => Some(Screen::Splash),
            _ => None,
        }
    }

    /// Parse `/etc/argoneonoled.conf`'s `screenlist="clock ip cpu storage
    /// temp"` value: whitespace-separated tokens, unknown tokens skipped
    /// rather than erroring (forward-compat with a config written by a
    /// newer version listing a screen this build doesn't know about yet).
    pub fn parse_list(screenlist: &str) -> Vec<Screen> {
        screenlist
            .split_whitespace()
            .filter_map(Screen::from_token)
            .collect()
    }
}

/// Everything a screen might need to render itself, gathered once per tick
/// so `render` stays free of I/O and is trivially unit-testable.
#[derive(Debug, Clone)]
pub struct OledData {
    pub now_utc: time::OffsetDateTime,
    pub ip: Option<std::net::IpAddr>,
    pub cpu_pct: Option<f32>,
    pub cpu_temp_c: Option<f32>,
    pub ram_pct: Option<f32>,
    pub disks: Vec<(String, u8)>,
    pub raid: Vec<(String, String)>,
    pub unit: crate::config::TempUnit,
    pub pi_model: Option<String>,
}

/// Screen-rotation state machine (W§1.2): advances through `screens` every
/// `switch_duration`, blanks after `screensaver_idle` of no button
/// activity (`None` disables the screensaver), and the power button's
/// `OledSwitch` event both wakes the panel and force-advances to the next
/// screen.
pub struct Rotation {
    screens: Vec<Screen>,
    idx: usize,
    switch_duration: Duration,
    screensaver_idle: Option<Duration>,
    last_switch: Instant,
    last_activity: Instant,
    blanked: bool,
}

impl Rotation {
    pub fn new(
        screens: Vec<Screen>,
        switch_duration: Duration,
        screensaver_idle: Option<Duration>,
        now: Instant,
    ) -> Self {
        Rotation {
            screens,
            idx: 0,
            switch_duration,
            screensaver_idle,
            last_switch: now,
            last_activity: now,
            blanked: false,
        }
    }

    /// The screen to render right now, or `None` if blanked (screensaver)
    /// or the rotation list is empty.
    pub fn current(&self) -> Option<Screen> {
        if self.blanked {
            return None;
        }
        self.screens.get(self.idx).copied()
    }

    /// Advance rotation/screensaver state for the given wall-clock instant.
    /// Call this on every render tick, not just on a timer that matches
    /// `switch_duration` exactly — `now` may arrive late.
    pub fn tick(&mut self, now: Instant) {
        if let Some(idle) = self.screensaver_idle
            && now.duration_since(self.last_activity) >= idle
        {
            self.blanked = true;
        }
        if !self.blanked
            && !self.screens.is_empty()
            && now.duration_since(self.last_switch) >= self.switch_duration
        {
            self.idx = (self.idx + 1) % self.screens.len();
            self.last_switch = now;
        }
    }

    /// The power button's `OledSwitch` pulse: wake from the screensaver and
    /// force-advance to the next screen, matching the Python daemon's
    /// button-driven `OLEDSWITCH` queue message.
    pub fn on_button_switch(&mut self, now: Instant) {
        self.blanked = false;
        self.last_activity = now;
        if !self.screens.is_empty() {
            self.idx = (self.idx + 1) % self.screens.len();
            self.last_switch = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_matches_python_default_screenlist() {
        let screens = Screen::parse_list("clock ip cpu storage temp");
        assert_eq!(
            screens,
            vec![
                Screen::Clock,
                Screen::Ip,
                Screen::Cpu,
                Screen::Storage,
                Screen::Temp
            ]
        );
    }

    #[test]
    fn parse_list_skips_unknown_tokens() {
        assert_eq!(
            Screen::parse_list("clock bogus ip"),
            vec![Screen::Clock, Screen::Ip]
        );
    }

    #[test]
    fn rotation_advances_after_switch_duration() {
        let t0 = Instant::now();
        let mut rot = Rotation::new(
            vec![Screen::Clock, Screen::Ip],
            Duration::from_secs(10),
            None,
            t0,
        );
        assert_eq!(rot.current(), Some(Screen::Clock));
        rot.tick(t0 + Duration::from_secs(5));
        assert_eq!(rot.current(), Some(Screen::Clock));
        rot.tick(t0 + Duration::from_secs(11));
        assert_eq!(rot.current(), Some(Screen::Ip));
        rot.tick(t0 + Duration::from_secs(22));
        assert_eq!(rot.current(), Some(Screen::Clock));
    }

    #[test]
    fn rotation_blanks_after_idle_and_button_wakes_it() {
        let t0 = Instant::now();
        let mut rot = Rotation::new(
            vec![Screen::Clock, Screen::Ip],
            Duration::from_secs(1000),
            Some(Duration::from_secs(30)),
            t0,
        );
        rot.tick(t0 + Duration::from_secs(31));
        assert_eq!(rot.current(), None);

        rot.on_button_switch(t0 + Duration::from_secs(32));
        assert_eq!(rot.current(), Some(Screen::Ip));
    }

    #[test]
    fn empty_screen_list_never_advances_and_current_is_none_only_when_blanked() {
        let t0 = Instant::now();
        let mut rot = Rotation::new(vec![], Duration::from_secs(1), None, t0);
        rot.tick(t0 + Duration::from_secs(5));
        assert_eq!(rot.current(), None);
    }

    #[test]
    fn button_switch_on_empty_list_does_not_panic() {
        let t0 = Instant::now();
        let mut rot = Rotation::new(vec![], Duration::from_secs(1), None, t0);
        rot.on_button_switch(t0);
        assert_eq!(rot.current(), None);
    }
}

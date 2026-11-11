// SPDX-License-Identifier: GPL-3.0-only
//! The screen's on/off state (D2 display): off after `display.timeout` seconds without a touch or key, or by a short
//! press of the power key; on by the power key or an incoming call (touch can't wake it: the kernel puts the
//! touchscreen to sleep). The state machine is pure and tested on the host; `lib.rs` applies it through the display
//! HAL (`EDisplayHalSetState`, `EDisplayHalSetDisplayBrightness`).

/// What the UI server does with the screen after an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    On,
    Off,
}

pub struct Screen {
    on: bool,
    last_activity_ms: u64,
    /// The power key's press woke the screen: its release must not turn it off again, nor its hold open the menu.
    press_woke: bool,
}

impl Screen {
    pub fn new(now_ms: u64) -> Self {
        Self {
            on: true,
            last_activity_ms: now_ms,
            press_woke: false,
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// A touch or key while on: the timeout starts again.
    pub fn activity(&mut self, now_ms: u64) {
        if self.on {
            self.last_activity_ms = now_ms;
        }
    }

    /// The power key went down: an off screen wakes at once.
    pub fn power_down(&mut self, now_ms: u64) -> Option<Change> {
        self.press_woke = !self.on;
        self.wake(now_ms)
    }

    /// The power key came up after `held_ms`: a short press of an on screen turns it off (a long one opened the
    /// Power menu instead; a press that woke the screen does nothing more).
    pub fn power_up(&mut self, held_ms: u64, hold_ms: u64) -> Option<Change> {
        let woke = core::mem::take(&mut self.press_woke);
        if self.on && !woke && held_ms < hold_ms {
            self.on = false;
            return Some(Change::Off);
        }
        None
    }

    /// Did the power key's current press wake the screen (then its hold must not open the Power menu)?
    pub fn press_woke(&self) -> bool {
        self.press_woke
    }

    /// An incoming call (or anything else that must be seen).
    pub fn wake(&mut self, now_ms: u64) -> Option<Change> {
        self.last_activity_ms = now_ms;
        if self.on {
            return None;
        }
        self.on = true;
        Some(Change::On)
    }

    /// Once per loop: off after `timeout_s` seconds without activity (0 = never; `hold` keeps it on, e.g. a call).
    pub fn tick(&mut self, now_ms: u64, timeout_s: i32, hold: bool) -> Option<Change> {
        if hold {
            self.last_activity_ms = now_ms;
        }
        if !self.on || timeout_s <= 0 {
            return None;
        }
        if now_ms.saturating_sub(self.last_activity_ms) >= timeout_s as u64 * 1000 {
            self.on = false;
            return Some(Change::Off);
        }
        None
    }
}

/// `display.brightness` (0..=255) to the panel's level (0..=`max`).
pub fn panel_level(setting: i32, max: i32) -> i32 {
    setting.clamp(0, 255) * max / 255
}

#[cfg(test)]
mod tests {
    use super::*;
    const HOLD: u64 = 1000;

    #[test]
    fn times_out_after_the_setting_and_activity_restarts_it() {
        let mut s = Screen::new(0);
        assert_eq!(s.tick(59_999, 60, false), None);
        s.activity(30_000);
        assert_eq!(s.tick(60_000, 60, false), None);
        assert_eq!(s.tick(90_000, 60, false), Some(Change::Off));
        assert!(!s.is_on());
        assert_eq!(s.tick(200_000, 60, false), None);
    }

    #[test]
    fn never_and_a_call_keep_it_on() {
        let mut s = Screen::new(0);
        assert_eq!(s.tick(10_000_000, 0, false), None);
        assert_eq!(s.tick(20_000_000, 15, true), None);
        assert_eq!(s.tick(20_014_000, 15, false), None);
        assert_eq!(s.tick(20_015_000, 15, false), Some(Change::Off));
    }

    #[test]
    fn a_short_press_toggles_and_a_waking_press_does_not_turn_it_off_again() {
        let mut s = Screen::new(0);
        assert_eq!(s.power_down(100), None);
        assert!(!s.press_woke());
        assert_eq!(s.power_up(200, HOLD), Some(Change::Off));
        assert_eq!(s.power_down(5_000), Some(Change::On));
        assert!(s.press_woke());
        assert_eq!(s.power_up(150, HOLD), None);
        assert!(s.is_on());
        // a long press of an on screen is the Power menu's, not an off
        s.power_down(9_000);
        assert_eq!(s.power_up(1_200, HOLD), None);
        assert!(s.is_on());
    }

    #[test]
    fn a_call_wakes_it_and_activity_while_off_does_not() {
        let mut s = Screen::new(0);
        s.tick(60_000, 60, false);
        s.activity(61_000);
        assert!(!s.is_on());
        assert_eq!(s.wake(70_000), Some(Change::On));
        assert_eq!(s.tick(129_999, 60, false), None);
        assert_eq!(s.tick(130_000, 60, false), Some(Change::Off));
    }

    #[test]
    fn brightness_scales_to_the_panel() {
        assert_eq!(panel_level(0, 1023), 0);
        assert_eq!(panel_level(255, 1023), 1023);
        assert_eq!(panel_level(160, 1023), 641);
        assert_eq!(panel_level(300, 1023), 1023);
    }
}

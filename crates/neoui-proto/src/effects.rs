// SPDX-License-Identifier: Apache-2.0
//! What some settings do to the hardware, as pure functions the UI server calls (Settings SE3a): the panel's level
//! with Battery saver, and the system keyboard's key pulse. Their defaults leave the phone as it was before SE3a.

/// Battery saver's cap: the panel never goes above this share of its maximum while the saver is on.
// shortcut: 40 % is the coordinator's placeholder; the owner picks the cap (docs neoui-shell/settings.md, SE3 "For
// the owner"), and whether the saver switches on by itself at 15 % (today: by hand only).
pub const SAVER_CAP_PERCENT: i32 = 40;

/// The pulse each key of the system keyboard gives with `keyboard.vibrate` on: 12 ms at amplitude 70, the old apps'
/// pads' medium tick (legacy `host/legacy.cpp`).
pub const KEY_PULSE: (u32, u32) = (12, 70);

/// The panel's level (0..=`max`) for `display.brightness` (0..=255). With `saver` on it's capped at
/// [`SAVER_CAP_PERCENT`] of `max`; the setting itself doesn't move, so turning the saver off restores it at once.
#[must_use]
pub fn panel_level(setting: i32, max: i32, saver: bool) -> i32 {
    let level = setting.clamp(0, 255) * max / 255;
    if saver {
        level.min(max * SAVER_CAP_PERCENT / 100)
    } else {
        level
    }
}

/// The pulse a key press gives (`(ms, amplitude)` for the haptic HAL), or none with `keyboard.vibrate` off.
#[must_use]
pub fn key_pulse(vibrate: bool) -> Option<(u32, u32)> {
    vibrate.then_some(KEY_PULSE)
}

/// (D6) What an accepted control gives ([`crate::ipc::CONTROL_FEEDBACK`]): a pulse with vibrate on touch, and for a
/// dial key out of a call its DTMF pair with keypad tones in an audible profile (not Silent, ring volume above 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Feedback {
    pub pulse: Option<(u32, u32)>,
    /// the key's two frequencies (Hz)
    pub tone: Option<(u32, u32)>,
}

/// The active profile's feedback as the shell holds it (`sound.touch_vibrate`, `sound.keypad_tones`, whether it
/// rings) and whether a call is up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeedbackPolicy {
    pub touch: bool,
    pub keypad: bool,
    pub audible: bool,
    pub in_call: bool,
}

impl FeedbackPolicy {
    /// The answer to one accepted control: `kind` and `key` as [`crate::ipc::CONTROL_FEEDBACK`] carries them.
    #[must_use]
    pub fn answer(&self, kind: i32, key: u8) -> Feedback {
        let tone =
            (kind == crate::ipc::FEEDBACK_DIAL_KEY && self.keypad && self.audible && !self.in_call)
                .then(|| dtmf(key))
                .flatten();
        Feedback {
            pulse: self.touch.then_some(KEY_PULSE),
            tone,
        }
    }
}

/// A dial key's DTMF pair (ITU-T Q.23): the row's and the column's frequency.
#[must_use]
pub fn dtmf(key: u8) -> Option<(u32, u32)> {
    const KEYS: &[u8; 12] = b"123456789*0#";
    let i = KEYS.iter().position(|k| *k == key)?;
    Some(([697, 770, 852, 941][i / 3], [1209, 1336, 1477][i % 3]))
}

/// One answer per press: the shell counts the presses it delivered to the app in front, and a
/// [`crate::ipc::CONTROL_FEEDBACK`] takes one (a second for the same press, or one with no press, is refused).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PressGate {
    delivered: u32,
    answered: u32,
}

impl PressGate {
    /// Finger 0 went down in the app in front.
    pub fn pressed(&mut self) {
        self.delivered = self.delivered.wrapping_add(1);
    }

    /// Another app came to the front (or the screen went off): presses before it get no answer.
    pub fn forget(&mut self) {
        self.answered = self.delivered;
    }

    /// An accepted control: true when its press hasn't been answered yet.
    pub fn take(&mut self) -> bool {
        let fresh = self.answered != self.delivered;
        self.answered = self.delivered;
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::registry::keys;
    use crate::settings::store::Store;

    /// The Pixel's panel maximum (display HAL).
    const MAX: i32 = 1023;

    #[test]
    fn a_fresh_store_keeps_the_phone_as_it_was() {
        let s = Store::new();
        let brightness = s.get(keys::DISPLAY_BRIGHTNESS).unwrap();
        // today's level: the setting scaled to the panel, uncapped
        assert_eq!(
            panel_level(brightness, MAX, s.get(keys::BATTERY_SAVER).unwrap()),
            160 * MAX / 255
        );
        // no key pulse, the percent in the status bar, no touch rings
        assert_eq!(key_pulse(s.get(keys::KEYBOARD_VIBRATE).unwrap()), None);
        assert!(s.get(keys::BATTERY_SHOW_PERCENT).unwrap());
        assert!(!s.get(keys::DEVELOPER_SHOW_TOUCHES).unwrap());
    }

    #[test]
    fn the_saver_caps_and_lifts_at_once() {
        let cap = MAX * SAVER_CAP_PERCENT / 100;
        assert_eq!(panel_level(255, MAX, true), cap);
        // off: the full level again, nothing lingers
        assert_eq!(panel_level(255, MAX, false), MAX);
        // below the cap the saver changes nothing; the ends hold
        assert_eq!(panel_level(40, MAX, true), panel_level(40, MAX, false));
        assert_eq!(panel_level(0, MAX, true), 0);
        assert_eq!(panel_level(300, MAX, false), MAX);
        assert_eq!(panel_level(160, MAX, false), 641); // the old screen::panel_level's values
        assert_eq!(panel_level(-5, MAX, true), 0);
        // no panel (QEMU): nothing to cap
        assert_eq!(panel_level(255, 0, true), 0);
    }

    #[test]
    fn one_answer_per_press_and_the_dial_tone_only_where_it_may_sound() {
        let mut g = PressGate::default();
        assert!(!g.take()); // no press: nothing
        g.pressed();
        assert!(g.take());
        assert!(!g.take()); // the same press twice: once
        g.pressed();
        g.forget();
        assert!(!g.take());
        let on = FeedbackPolicy {
            touch: true,
            keypad: true,
            audible: true,
            in_call: false,
        };
        use crate::ipc::{FEEDBACK_CONTROL, FEEDBACK_DIAL_KEY};
        assert_eq!(
            on.answer(FEEDBACK_DIAL_KEY, b'5'),
            Feedback {
                pulse: Some(KEY_PULSE),
                tone: Some((770, 1336))
            }
        );
        assert_eq!(on.answer(FEEDBACK_CONTROL, b'5').tone, None);
        assert_eq!(dtmf(b'#'), Some((941, 1477)));
        assert_eq!(dtmf(b'1'), Some((697, 1209)));
        assert_eq!(dtmf(b'x'), None);
        // in a call (DTMF is the call's), Silent or volume 0, or keypad tones off: no tone
        for p in [
            FeedbackPolicy {
                in_call: true,
                ..on
            },
            FeedbackPolicy {
                audible: false,
                ..on
            },
            FeedbackPolicy {
                keypad: false,
                ..on
            },
        ] {
            assert_eq!(p.answer(FEEDBACK_DIAL_KEY, b'5').tone, None);
            assert_eq!(p.answer(FEEDBACK_DIAL_KEY, b'5').pulse, Some(KEY_PULSE));
        }
        assert_eq!(
            FeedbackPolicy::default().answer(FEEDBACK_CONTROL, 0),
            Feedback::default()
        );
    }

    #[test]
    fn the_key_pulse_is_the_pads_tick() {
        assert_eq!(key_pulse(true), Some((12, 70)));
        assert!(KEY_PULSE.0 <= crate::ipc::HAPTIC_MS_MAX as u32);
    }
}

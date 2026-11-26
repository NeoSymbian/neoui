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
    fn the_key_pulse_is_the_pads_tick() {
        assert_eq!(key_pulse(true), Some((12, 70)));
        assert!(KEY_PULSE.0 <= crate::ipc::HAPTIC_MS_MAX as u32);
    }
}

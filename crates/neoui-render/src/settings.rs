// SPDX-License-Identifier: Apache-2.0
//! The binding between the UI server's settings store (`neoui_proto::settings`) and the shell's `SystemSettings`
//! Slint global (`ui/system-settings.slint`): one [`Binding`] row per setting the shell shows, with the value
//! conversion in one place. The store stays the source of truth; the UI is a view that reports edits.
//!
//! Adding a shown setting: a registry row (neoui-proto), a property and `changed` handler in
//! `system-settings.slint`, and a row in [`BINDINGS`]. The tests check every row against the registry.

use crate::SystemSettings;
use neoui_proto::settings::{Key, Setting, Value, keys};

/// How one registry key maps onto the `SystemSettings` global.
pub(crate) struct Binding {
    pub key: &'static str,
    /// The UI's current value, as the store's type.
    pub read: fn(&SystemSettings<'_>) -> Value,
    /// Shows `value` in the UI; false when it has the wrong type.
    pub write: fn(&SystemSettings<'_>, &Value) -> bool,
}

/// A `bool` setting shown by a `bool` property.
macro_rules! bool_binding {
    ($key:expr, $get:ident, $set:ident) => {
        Binding {
            key: key_name::<bool>($key),
            read: |g| Value::Bool(g.$get()),
            write: |g, v| match v {
                Value::Bool(b) => {
                    g.$set(*b);
                    true
                }
                _ => false,
            },
        }
    };
}

const fn key_name<T: Setting>(key: Key<T>) -> &'static str {
    key.name()
}

/// The `float` brightness slider (0.0..=1.0) against `display.brightness` (0..=255).
const BRIGHTNESS_MAX: f32 = 255.0;

fn brightness_to_store(slider: f32) -> i32 {
    // in range after the clamp, so the cast cannot saturate; +0.5 rounds to nearest (no `round` in core)
    (slider.clamp(0.0, 1.0) * BRIGHTNESS_MAX + 0.5) as i32
}

fn brightness_to_slider(level: i32) -> f32 {
    level.clamp(0, BRIGHTNESS_MAX as i32) as f32 / BRIGHTNESS_MAX
}

/// Every setting the shell shows.
pub(crate) static BINDINGS: &[Binding] = &[
    bool_binding!(keys::RADIO_WIFI, get_wifi, set_wifi),
    bool_binding!(keys::RADIO_BT, get_bluetooth, set_bluetooth),
    bool_binding!(keys::RADIO_DATA, get_mobile_data, set_mobile_data),
    bool_binding!(keys::RADIO_AIRPLANE, get_airplane, set_airplane),
    bool_binding!(keys::SOUND_SILENT, get_silent, set_silent),
    bool_binding!(keys::SOUND_VIBRATE, get_vibrate, set_vibrate),
    Binding {
        key: key_name::<i32>(keys::SOUND_RING_VOLUME),
        read: |g| Value::Int(g.get_ring_volume()),
        write: |g, v| match v {
            Value::Int(volume) => {
                g.set_ring_volume(*volume);
                true
            }
            _ => false,
        },
    },
    bool_binding!(keys::DISPLAY_DARK, get_dark, set_dark),
    bool_binding!(
        keys::DEVELOPER_USB_DEBUGGING,
        get_usb_debugging,
        set_usb_debugging
    ),
    Binding {
        key: key_name::<i32>(keys::DISPLAY_BRIGHTNESS),
        read: |g| Value::Int(brightness_to_store(g.get_brightness())),
        write: |g, v| match v {
            Value::Int(level) => {
                // the slider's own rounding must not report an edit back: only move it for a real difference
                if brightness_to_store(g.get_brightness()) != *level {
                    g.set_brightness(brightness_to_slider(*level));
                }
                true
            }
            _ => false,
        },
    },
    bool_binding!(keys::CLOCK_24H, get_clock_24h, set_clock_24h),
    bool_binding!(keys::KEYBOARD_EN, get_keyboard_en, set_keyboard_en),
    bool_binding!(keys::KEYBOARD_HY, get_keyboard_hy, set_keyboard_hy),
    bool_binding!(keys::KEYBOARD_RU, get_keyboard_ru, set_keyboard_ru),
    bool_binding!(
        keys::KEYBOARD_SAME_HEIGHT,
        get_keyboard_same_height,
        set_keyboard_same_height
    ),
    Binding {
        key: key_name::<i32>(keys::KEYBOARD_LAYOUT),
        read: |g| Value::Int(g.get_keyboard_layout()),
        write: |g, v| match v {
            Value::Int(i) => {
                g.set_keyboard_layout(*i);
                true
            }
            _ => false,
        },
    },
    Binding {
        key: key_name::<i32>(keys::DISPLAY_TIMEOUT),
        read: |g| Value::Int(g.get_screen_timeout()),
        write: |g, v| match v {
            Value::Int(s) => {
                g.set_screen_timeout(*s);
                true
            }
            _ => false,
        },
    },
];

/// The binding for `key`, if the shell shows it.
pub(crate) fn binding(key: &str) -> Option<&'static Binding> {
    BINDINGS.iter().find(|b| b.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use neoui_proto::settings::registry;

    #[test]
    fn every_binding_is_a_registered_key_bound_once() {
        for (i, b) in BINDINGS.iter().enumerate() {
            assert!(
                registry::lookup(b.key).is_some(),
                "{} not registered",
                b.key
            );
            assert!(
                BINDINGS[i + 1..].iter().all(|o| o.key != b.key),
                "{} bound twice",
                b.key
            );
        }
    }

    #[test]
    fn brightness_converts_both_ways_without_drift() {
        for level in [0, 1, 127, 160, 254, 255] {
            assert_eq!(brightness_to_store(brightness_to_slider(level)), level);
        }
        assert_eq!(brightness_to_store(-1.0), 0);
        assert_eq!(brightness_to_store(2.0), 255);
    }
}

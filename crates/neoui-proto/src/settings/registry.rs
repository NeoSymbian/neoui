// SPDX-License-Identifier: Apache-2.0
//! The registry: every system setting with its type, default, limit and owner, declared once in the
//! [`registry!`] table below. The table generates both the typed [`keys`] and [`REGISTRY`], and checks at compile
//! time that each default has the key's type, so the two cannot drift apart.
//!
//! **Adding a system setting:** add one row to the table. If the shell shows it, bind it in `neoui-render`'s
//! settings binding table too. Keys an app keeps under its own namespace (`clock.*`) need no row: they are allowed,
//! limited only by the value size limits, and only that app may write them.

use super::key::{Bytes, Key, Setting};
use super::{Value, ValueType};

/// Who may write a registered key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The UI server only (system settings shown in Settings and the shade).
    Shell,
    /// The app with this key (`"clock"`), and the UI server.
    App(&'static str),
}

/// What values a registered key accepts beyond its type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    None,
    /// An `Int` within `min..=max`.
    Range {
        min: i32,
        max: i32,
    },
    /// A `Str` or `Bytes` of at most this many bytes (tighter than the type-wide limit).
    MaxLen(usize),
}

/// A default value, in a form that can be written in a `const` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultValue {
    Bool(bool),
    Int(i32),
    Str(&'static str),
    Bytes(&'static [u8]),
}

impl DefaultValue {
    #[must_use]
    pub const fn ty(&self) -> ValueType {
        match self {
            DefaultValue::Bool(_) => ValueType::Bool,
            DefaultValue::Int(_) => ValueType::Int,
            DefaultValue::Str(_) => ValueType::Str,
            DefaultValue::Bytes(_) => ValueType::Bytes,
        }
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        match *self {
            DefaultValue::Bool(v) => Value::Bool(v),
            DefaultValue::Int(v) => Value::Int(v),
            DefaultValue::Str(s) => Value::Str(s.into()),
            DefaultValue::Bytes(b) => Value::Bytes(b.to_vec()),
        }
    }

    /// Whether `v` equals this default (without allocating).
    #[must_use]
    pub fn matches(&self, v: &Value) -> bool {
        match (self, v) {
            (DefaultValue::Bool(a), Value::Bool(b)) => a == b,
            (DefaultValue::Int(a), Value::Int(b)) => a == b,
            (DefaultValue::Str(a), Value::Str(b)) => *a == b.as_str(),
            (DefaultValue::Bytes(a), Value::Bytes(b)) => *a == b.as_slice(),
            _ => false,
        }
    }
}

/// A registered setting.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub name: &'static str,
    pub default: DefaultValue,
    pub limit: Limit,
    pub owner: Owner,
}

impl Spec {
    #[must_use]
    pub const fn ty(&self) -> ValueType {
        self.default.ty()
    }

    /// Whether `v` fits this key's type and limit.
    ///
    /// # Errors
    /// [`super::Error::WrongType`], [`super::Error::OutOfRange`] or [`super::Error::TooLong`].
    pub fn check(&self, v: &Value) -> Result<(), super::Error> {
        if v.ty() != self.ty() {
            return Err(super::Error::WrongType);
        }
        match (self.limit, v) {
            (Limit::Range { min, max }, Value::Int(i)) if !(min..=max).contains(i) => {
                Err(super::Error::OutOfRange)
            }
            (Limit::MaxLen(n), Value::Str(s)) if s.len() > n => Err(super::Error::TooLong),
            (Limit::MaxLen(n), Value::Bytes(b)) if b.len() > n => Err(super::Error::TooLong),
            _ => Ok(()),
        }
    }
}

/// The registered spec for `name`, if any.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static Spec> {
    REGISTRY.iter().find(|s| s.name == name)
}

/// The spec of a typed key, if registered.
#[must_use]
pub fn spec_of<T: Setting>(key: Key<T>) -> Option<&'static Spec> {
    lookup(key.name())
}

macro_rules! registry {
    ($(
        $(#[$doc:meta])*
        $konst:ident: $ty:ty = $name:literal, default $default:expr, limit $limit:expr, owner $owner:expr;
    )*) => {
        /// Typed handles to every registered setting.
        pub mod keys {
            #[allow(unused_imports)]
            use super::super::key::{Bytes, Key};
            $(
                $(#[$doc])*
                pub const $konst: Key<$ty> = Key::new($name);
            )*
        }

        /// Every registered setting.
        pub static REGISTRY: &[Spec] = &[$(
            Spec { name: $name, default: $default, limit: $limit, owner: $owner },
        )*];

        // Each default has its key's type.
        $(const _: () = assert!(
            $default.ty() as u8 == <$ty as Setting>::TYPE as u8,
            concat!("default of ", $name, " does not match its type"),
        );)*
    };
}

registry! {
    /// Wi-Fi radio on. UI only until there is a Wi-Fi driver (telephony P4).
    RADIO_WIFI: bool = "radio.wifi", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// Bluetooth radio on. UI only until there is a driver.
    RADIO_BT: bool = "radio.bt", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// Mobile data on.
    RADIO_DATA: bool = "radio.data", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// Airplane mode: every radio off.
    RADIO_AIRPLANE: bool = "radio.airplane", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// Silent: no ringer or notification sounds.
    SOUND_SILENT: bool = "sound.silent", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// Vibrate for calls and notifications.
    SOUND_VIBRATE: bool = "sound.vibrate", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// Dark theme.
    DISPLAY_DARK: bool = "display.dark", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// Screen brightness, 0 (dimmest) to 255; the UI server sends it to the panel (display HAL, 0..1023).
    DISPLAY_BRIGHTNESS: i32 = "display.brightness", default DefaultValue::Int(160),
        limit Limit::Range { min: 0, max: 255 }, owner Owner::Shell;
    /// Seconds without a touch before the screen turns off; 0 = never.
    DISPLAY_TIMEOUT: i32 = "display.timeout", default DefaultValue::Int(60),
        limit Limit::Range { min: 0, max: 600 }, owner Owner::Shell;
    /// 24-hour clock (else 12-hour).
    CLOCK_24H: bool = "clock.24h", default DefaultValue::Bool(true), limit Limit::None, owner Owner::App("clock");
    /// USB debugging (Developer options): fshell's USB serial port and the shell on it (`phone.py sh`), like
    /// Android's adb. The UI server turns the port on (kernel HAL 0x4e04) and supervises fshell.
    DEVELOPER_USB_DEBUGGING: bool = "developer.usb_debugging", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// The system keyboard has English (Settings -> Language & keyboard; the globe key cycles the languages on).
    KEYBOARD_EN: bool = "keyboard.en", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// The system keyboard has Armenian (phonetic).
    KEYBOARD_HY: bool = "keyboard.hy", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// The system keyboard has Russian (ЙЦУКЕН).
    KEYBOARD_RU: bool = "keyboard.ru", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// The system keyboard's language: an index into ui/components/keyboard/layouts.slint's list (append only).
    KEYBOARD_LAYOUT: i32 = "keyboard.layout", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 15 }, owner Owner::Shell;
    /// Every keyboard language as tall as the tallest one on (shorter ones get a digit row); else each its own height.
    KEYBOARD_SAME_HEIGHT: bool = "keyboard.same_height", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::Shell;
    /// The Clock app's face: 0 digital, 1 analog (Clock settings).
    CLOCK_FACE: i32 = "clock.face", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 1 }, owner Owner::App("clock");
    /// The Clock app shows the seconds (the seconds hand on the analog face).
    CLOCK_SECONDS: bool = "clock.seconds", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::App("clock");
    /// The Clock app shows the date under the face.
    CLOCK_DATE: bool = "clock.date", default DefaultValue::Bool(true), limit Limit::None, owner Owner::App("clock");
    /// The Clock app's open page: 0 Clock, 1 Alarms, 2 World, 3 Timer, 4 Stopwatch.
    CLOCK_TAB: i32 = "clock.tab", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 4 }, owner Owner::App("clock");
    /// The alarms (`clock::alarm::Alarms` blob; empty = none). The UI server's scheduler reads it too.
    CLOCK_ALARMS: Bytes = "clock.alarms", default DefaultValue::Bytes(&[]), limit Limit::None, owner Owner::App("clock");
    /// The world clocks (`clock::world::Cities` blob; empty = the default cities).
    CLOCK_CITIES: Bytes = "clock.cities", default DefaultValue::Bytes(&[]), limit Limit::None, owner Owner::App("clock");
    /// The countdown timer (`clock::timer::Timer` blob; empty = a new 5-minute timer).
    CLOCK_TIMER: Bytes = "clock.timer", default DefaultValue::Bytes(&[]), limit Limit::None, owner Owner::App("clock");
    /// The stopwatch (`clock::stopwatch::Stopwatch` blob; empty = reset).
    CLOCK_STOPWATCH: Bytes = "clock.stopwatch", default DefaultValue::Bytes(&[]), limit Limit::None,
        owner Owner::App("clock");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::key::is_valid_name;

    #[test]
    fn names_are_valid_and_unique() {
        for (i, s) in REGISTRY.iter().enumerate() {
            assert!(is_valid_name(s.name), "{}", s.name);
            assert!(
                REGISTRY[i + 1..].iter().all(|t| t.name != s.name),
                "duplicate {}",
                s.name
            );
        }
    }

    #[test]
    fn defaults_satisfy_their_own_limits() {
        for s in REGISTRY {
            assert_eq!(s.check(&s.default.to_value()), Ok(()), "{}", s.name);
            assert!(s.default.matches(&s.default.to_value()), "{}", s.name);
        }
    }

    #[test]
    fn typed_keys_resolve_to_their_spec() {
        let spec = spec_of(keys::DISPLAY_BRIGHTNESS).expect("registered");
        assert_eq!(spec.ty(), ValueType::Int);
        assert_eq!(
            spec.check(&Value::Int(256)),
            Err(super::super::Error::OutOfRange)
        );
        assert_eq!(
            spec.check(&Value::Bool(true)),
            Err(super::super::Error::WrongType)
        );
        assert!(lookup("radio.nothing").is_none());
    }
}

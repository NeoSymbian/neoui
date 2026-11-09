// SPDX-License-Identifier: Apache-2.0
//! Key names and typed keys. A [`Key<T>`] ties a name to its Rust type, so `store.get(keys::CLOCK_24H)` is a `bool`
//! at compile time; the untyped `*_raw` APIs exist only for the IPC boundary.

use super::{Value, ValueType};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;

/// Longest key name in bytes.
pub const KEY_MAX: usize = 32;

/// Whether `name` is a valid key: `segment(.segment)+`, each segment non-empty `[a-z0-9_]`, at most [`KEY_MAX`]
/// bytes. The first segment is the namespace (`radio`, `clock`, an app key).
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    name.len() <= KEY_MAX && name.contains('.') && name.split('.').all(is_valid_segment)
}

/// Whether `prefix` is a valid watch prefix: empty (everything), or a key name or the start of one, optionally
/// ending with `.` (`"clock."`, `"radio.wifi"`).
#[must_use]
pub fn is_valid_prefix(prefix: &str) -> bool {
    let body = prefix.strip_suffix('.').unwrap_or(prefix);
    prefix.len() <= KEY_MAX && (prefix.is_empty() || body.split('.').all(is_valid_segment))
}

fn is_valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The namespace of a valid key name: everything before the first `.`.
#[must_use]
pub fn namespace(name: &str) -> &str {
    name.split_once('.').map_or(name, |(ns, _)| ns)
}

mod sealed {
    pub trait Sealed {}
}

/// A Rust type a setting can have. Sealed: the four [`ValueType`]s are the whole set.
pub trait Setting: Sized + sealed::Sealed {
    const TYPE: ValueType;
    fn into_value(self) -> Value;
    /// `None` when `v` is another type.
    fn from_value(v: Value) -> Option<Self>;
}

/// A byte-blob setting (structured state such as a layout), distinct from `Vec<u8>` so the intent is explicit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl sealed::Sealed for bool {}
impl Setting for bool {
    const TYPE: ValueType = ValueType::Bool;
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl sealed::Sealed for i32 {}
impl Setting for i32 {
    const TYPE: ValueType = ValueType::Int;
    fn into_value(self) -> Value {
        Value::Int(self)
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Int(i) => Some(i),
            _ => None,
        }
    }
}

impl sealed::Sealed for String {}
impl Setting for String {
    const TYPE: ValueType = ValueType::Str;
    fn into_value(self) -> Value {
        Value::Str(self)
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

impl sealed::Sealed for Bytes {}
impl Setting for Bytes {
    const TYPE: ValueType = ValueType::Bytes;
    fn into_value(self) -> Value {
        Value::Bytes(self.0)
    }
    fn from_value(v: Value) -> Option<Self> {
        match v {
            Value::Bytes(b) => Some(Bytes(b)),
            _ => None,
        }
    }
}

/// A typed handle to a setting. Global keys live in [`super::keys`]; an app declares its own the same way
/// (`const ALARMS: Key<Bytes> = Key::new("clock.alarms");`).
pub struct Key<T: Setting> {
    name: &'static str,
    _type: PhantomData<fn() -> T>,
}

impl<T: Setting> Key<T> {
    /// # Panics
    /// At compile time (keys are consts) when `name` is not a valid key name.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        assert!(const_valid_name(name), "invalid setting key name");
        Self {
            name,
            _type: PhantomData,
        }
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

// Manual impls: derives would require `T: Clone`/`T: Debug`, which a handle doesn't need.
impl<T: Setting> Clone for Key<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Setting> Copy for Key<T> {}
impl<T: Setting> fmt::Debug for Key<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key({}: {:?})", self.name, T::TYPE)
    }
}

/// [`is_valid_name`] in a `const fn`, so invalid key consts fail to compile.
const fn const_valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    if b.is_empty() || b.len() > KEY_MAX {
        return false;
    }
    let (mut i, mut dots, mut seg_len) = (0, 0, 0);
    while i < b.len() {
        let c = b[i];
        if c == b'.' {
            if seg_len == 0 {
                return false;
            }
            dots += 1;
            seg_len = 0;
        } else if c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' {
            seg_len += 1;
        } else {
            return false;
        }
        i += 1;
    }
    dots > 0 && seg_len > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        for ok in [
            "radio.wifi",
            "a.b",
            "clock.24h",
            "widget.clock.next",
            "x_1.y_2",
        ] {
            assert!(is_valid_name(ok), "{ok}");
            assert!(const_valid_name(ok), "{ok}");
        }
        let long = "a.bcdefghijklmnopqrstuvwxyz012345";
        assert_eq!(long.len(), KEY_MAX + 1);
        for bad in [
            "",
            "radio",
            ".wifi",
            "radio.",
            "radio..wifi",
            "Radio.wifi",
            "radio.wi-fi",
            long,
        ] {
            assert!(!is_valid_name(bad), "{bad}");
            assert!(!const_valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn prefixes() {
        for ok in ["", "clock.", "clock", "radio.wifi", "widget.clock."] {
            assert!(is_valid_prefix(ok), "{ok}");
        }
        for bad in [".", "..", "Clock.", "clock..", "clock.a-b"] {
            assert!(!is_valid_prefix(bad), "{bad}");
        }
    }

    #[test]
    fn namespaces() {
        assert_eq!(namespace("radio.wifi"), "radio");
        assert_eq!(namespace("widget.clock.next"), "widget");
    }

    #[test]
    fn typed_values_convert_only_to_their_type() {
        assert_eq!(bool::from_value(true.into_value()), Some(true));
        assert_eq!(i32::from_value(Value::Bool(true)), None);
        assert_eq!(
            Bytes::from_value(Bytes(alloc::vec![1]).into_value()),
            Some(Bytes(alloc::vec![1]))
        );
        assert_eq!(String::from_value(Value::Int(1)), None);
    }
}

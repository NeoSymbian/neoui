// SPDX-License-Identifier: Apache-2.0
//! Setting values and their encoding, shared by the IPC protocol and the settings file: a type tag byte, then the
//! payload (`Bool`: one byte 0/1; `Int`: i32 little-endian; `Str`: UTF-8; `Bytes`: raw). The payload's length is
//! the length of what follows the tag, so the encoding is not self-delimiting: containers carry the length.

use super::Error;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Longest [`Value::Str`] in bytes.
pub const STR_MAX: usize = 64;
/// Longest [`Value::Bytes`] in bytes (room for structured blobs such as the home layout).
pub const BYTES_MAX: usize = 2048;
/// Longest encoded value: the tag plus the largest payload.
pub const ENCODED_MAX: usize = 1 + BYTES_MAX;

/// The type of a setting. The discriminants are the encoding's tag byte and must never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ValueType {
    Bool = 1,
    Int = 2,
    Str = 3,
    Bytes = 4,
}

impl ValueType {
    const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Bool),
            2 => Some(Self::Int),
            3 => Some(Self::Str),
            4 => Some(Self::Bytes),
            _ => None,
        }
    }
}

/// A setting's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    /// At most [`STR_MAX`] bytes of UTF-8.
    Str(String),
    /// At most [`BYTES_MAX`] bytes.
    Bytes(Vec<u8>),
}

impl Value {
    #[must_use]
    pub const fn ty(&self) -> ValueType {
        match self {
            Value::Bool(_) => ValueType::Bool,
            Value::Int(_) => ValueType::Int,
            Value::Str(_) => ValueType::Str,
            Value::Bytes(_) => ValueType::Bytes,
        }
    }

    /// Checks the type-wide length limits ([`STR_MAX`], [`BYTES_MAX`]).
    ///
    /// # Errors
    /// [`Error::TooLong`] when a string or byte value exceeds its limit.
    pub fn check_len(&self) -> Result<(), Error> {
        match self {
            Value::Str(s) if s.len() > STR_MAX => Err(Error::TooLong),
            Value::Bytes(b) if b.len() > BYTES_MAX => Err(Error::TooLong),
            _ => Ok(()),
        }
    }

    /// The encoded length in bytes.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        1 + match self {
            Value::Bool(_) => 1,
            Value::Int(_) => 4,
            Value::Str(s) => s.len(),
            Value::Bytes(b) => b.len(),
        }
    }

    /// Appends the encoding to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.reserve(self.encoded_len());
        out.push(self.ty() as u8);
        match self {
            Value::Bool(v) => out.push(u8::from(*v)),
            Value::Int(v) => out.extend_from_slice(&v.to_le_bytes()),
            Value::Str(s) => out.extend_from_slice(s.as_bytes()),
            Value::Bytes(b) => out.extend_from_slice(b),
        }
    }

    /// The encoding as a new buffer.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    /// Decodes exactly one value from `b` (the whole slice).
    ///
    /// # Errors
    /// [`Error::Corrupt`] for an unknown tag, a wrong payload size, a bool other than 0/1 or invalid UTF-8;
    /// [`Error::TooLong`] for a payload over its type's limit.
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        let (&tag, payload) = b.split_first().ok_or(Error::Corrupt)?;
        let value = match ValueType::from_tag(tag).ok_or(Error::Corrupt)? {
            ValueType::Bool => match payload {
                [0] => Value::Bool(false),
                [1] => Value::Bool(true),
                _ => return Err(Error::Corrupt),
            },
            ValueType::Int => {
                let raw: [u8; 4] = payload.try_into().map_err(|_| Error::Corrupt)?;
                Value::Int(i32::from_le_bytes(raw))
            }
            ValueType::Str => {
                let s = core::str::from_utf8(payload).map_err(|_| Error::Corrupt)?;
                Value::Str(s.into())
            }
            ValueType::Bytes => Value::Bytes(payload.to_vec()),
        };
        value.check_len()?;
        Ok(value)
    }
}

/// For logs: `true`, `160`, `"text"`, `<12 bytes>`.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(v) => write!(f, "{v}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Bytes(b) => write!(f, "<{} bytes>", b.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn every_type_round_trips() {
        let all = [
            Value::Bool(true),
            Value::Bool(false),
            Value::Int(i32::MIN),
            Value::Int(160),
            Value::Str("Hëllo".into()),
            Value::Str(String::new()),
            Value::Bytes(vec![0, 1, 2, 255]),
            Value::Bytes(Vec::new()),
        ];
        for v in all {
            let b = v.to_bytes();
            assert_eq!(b.len(), v.encoded_len());
            assert_eq!(Value::decode(&b), Ok(v));
        }
    }

    #[test]
    fn malformed_encodings_are_rejected() {
        for bad in [
            &[][..],
            &[0][..],
            &[9, 1][..],
            &[1][..],
            &[1, 2][..],
            &[1, 1, 1][..],
            &[2, 1, 2, 3][..],
            &[3, 0xff, 0xfe][..],
        ] {
            assert_eq!(Value::decode(bad), Err(Error::Corrupt), "{bad:?}");
        }
    }

    #[test]
    fn length_limits_hold() {
        let mut long = vec![ValueType::Str as u8];
        long.extend(core::iter::repeat_n(b'a', STR_MAX + 1));
        assert_eq!(Value::decode(&long), Err(Error::TooLong));
        let mut big = vec![ValueType::Bytes as u8];
        big.extend(core::iter::repeat_n(0, BYTES_MAX + 1));
        assert_eq!(Value::decode(&big), Err(Error::TooLong));
        big.pop();
        assert!(Value::decode(&big).is_ok());
        assert_eq!(big.len(), ENCODED_MAX);
    }
}

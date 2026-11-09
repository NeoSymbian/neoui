// SPDX-License-Identifier: Apache-2.0
//! Errors of the settings system, and their one mapping to Symbian error codes.

use core::fmt;

/// Symbian error codes (`e32err.h`) the settings system reports. Kept here so this crate needs no e32 dependency.
pub mod kerr {
    pub const NOT_FOUND: i32 = -1;
    pub const NOT_SUPPORTED: i32 = -5;
    pub const ARGUMENT: i32 = -6;
    pub const OVERFLOW: i32 = -9;
    pub const CORRUPT: i32 = -20;
    pub const TOO_BIG: i32 = -40;
    pub const PERMISSION_DENIED: i32 = -46;
}

/// Why a settings operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The key name is not `namespace.name` in `[a-z0-9_]`, or longer than [`super::KEY_MAX`].
    BadKey,
    /// No stored value and no registered default.
    NotFound,
    /// The value's type differs from the key's registered type.
    WrongType,
    /// An integer outside the key's registered range.
    OutOfRange,
    /// A string or byte value longer than its limit.
    TooLong,
    /// The caller may not write this key.
    Denied,
    /// A fixed capacity is used up (the file size, a session's watches).
    Full,
    /// Bytes that do not decode (a value, a file record or header).
    Corrupt,
    /// A file written by a newer format version than this build reads.
    Unsupported,
}

impl Error {
    /// The Symbian error code an IPC client receives for this error.
    #[must_use]
    pub const fn to_symbian(self) -> i32 {
        match self {
            Error::BadKey | Error::WrongType | Error::OutOfRange => kerr::ARGUMENT,
            Error::NotFound => kerr::NOT_FOUND,
            Error::TooLong => kerr::TOO_BIG,
            Error::Denied => kerr::PERMISSION_DENIED,
            Error::Full => kerr::OVERFLOW,
            Error::Corrupt => kerr::CORRUPT,
            Error::Unsupported => kerr::NOT_SUPPORTED,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::BadKey => "invalid setting key",
            Error::NotFound => "no such setting",
            Error::WrongType => "wrong value type for setting",
            Error::OutOfRange => "setting value out of range",
            Error::TooLong => "setting value too long",
            Error::Denied => "not allowed to write setting",
            Error::Full => "settings capacity exhausted",
            Error::Corrupt => "corrupt settings data",
            Error::Unsupported => "unsupported settings format version",
        })
    }
}

impl core::error::Error for Error {}

// SPDX-License-Identifier: Apache-2.0
//! The byte codec the Clock's settings blobs share (`clock.alarms`, `clock.cities`, `clock.timer`,
//! `clock.stopwatch`). Little-endian integers and length-prefixed strings. Each blob starts with its own format
//! version byte, so a later build can read an older blob or reject a newer one.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Bytes that don't decode as the expected blob (damaged, truncated, or from a newer format).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corrupt;

impl fmt::Display for Corrupt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("corrupt clock data")
    }
}

impl core::error::Error for Corrupt {}

/// Appends values to a blob.
#[derive(Debug, Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    /// A blob of format `version`.
    #[must_use]
    pub fn new(version: u8) -> Self {
        Self(alloc::vec![version])
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// A string of at most 255 bytes (longer ones are cut at a character boundary).
    pub fn str(&mut self, s: &str) -> &mut Self {
        let mut end = s.len().min(usize::from(u8::MAX));
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        // end <= 255, so it fits
        self.0.push(end as u8);
        self.0.extend_from_slice(&s.as_bytes()[..end]);
        self
    }

    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

/// Reads values back, failing with [`Corrupt`] on any shortfall.
#[derive(Debug)]
pub struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    /// Starts reading a blob, checking its version byte against `version` (the only one this build reads).
    ///
    /// # Errors
    /// [`Corrupt`] for an empty blob or another version.
    pub fn new(bytes: &'a [u8], version: u8) -> Result<Self, Corrupt> {
        match bytes.split_first() {
            Some((&v, rest)) if v == version => Ok(Self { rest }),
            _ => Err(Corrupt),
        }
    }

    fn take<const N: usize>(&mut self) -> Result<[u8; N], Corrupt> {
        let (head, rest) = self.rest.split_first_chunk::<N>().ok_or(Corrupt)?;
        self.rest = rest;
        Ok(*head)
    }

    /// # Errors
    /// [`Corrupt`] when the blob ends early.
    pub fn u8(&mut self) -> Result<u8, Corrupt> {
        Ok(self.take::<1>()?[0])
    }

    /// # Errors
    /// [`Corrupt`] when the blob ends early.
    pub fn u16(&mut self) -> Result<u16, Corrupt> {
        Ok(u16::from_le_bytes(self.take()?))
    }

    /// # Errors
    /// [`Corrupt`] when the blob ends early.
    pub fn u32(&mut self) -> Result<u32, Corrupt> {
        Ok(u32::from_le_bytes(self.take()?))
    }

    /// # Errors
    /// [`Corrupt`] when the blob ends early.
    pub fn i64(&mut self) -> Result<i64, Corrupt> {
        Ok(i64::from_le_bytes(self.take()?))
    }

    /// # Errors
    /// [`Corrupt`] when the blob ends early or the string is not UTF-8.
    pub fn str(&mut self) -> Result<String, Corrupt> {
        let len = usize::from(self.u8()?);
        if self.rest.len() < len {
            return Err(Corrupt);
        }
        let (s, rest) = self.rest.split_at(len);
        self.rest = rest;
        core::str::from_utf8(s)
            .map(String::from)
            .map_err(|_| Corrupt)
    }

    /// Checks that nothing is left over.
    ///
    /// # Errors
    /// [`Corrupt`] when bytes remain.
    pub fn end(self) -> Result<(), Corrupt> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(Corrupt)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        let mut w = Writer::new(3);
        w.u8(7)
            .u16(0xBEEF)
            .u32(1 << 31)
            .i64(-5)
            .str("Yerevan")
            .str("");
        let b = w.finish();
        let mut r = Reader::new(&b, 3).expect("version");
        assert_eq!(r.u8(), Ok(7));
        assert_eq!(r.u16(), Ok(0xBEEF));
        assert_eq!(r.u32(), Ok(1 << 31));
        assert_eq!(r.i64(), Ok(-5));
        assert_eq!(r.str().as_deref(), Ok("Yerevan"));
        assert_eq!(r.str().as_deref(), Ok(""));
        assert_eq!(r.end(), Ok(()));
    }

    #[test]
    fn shortfalls_and_versions_are_corrupt() {
        assert_eq!(Reader::new(&[], 1).err(), Some(Corrupt));
        assert_eq!(Reader::new(&[2], 1).err(), Some(Corrupt));
        let mut r = Reader::new(&[1, 0xAA], 1).expect("version");
        assert_eq!(r.u16(), Err(Corrupt));
        let mut r = Reader::new(&[1, 5, b'a'], 1).expect("version");
        assert_eq!(r.str(), Err(Corrupt));
        assert_eq!(
            Reader::new(&[1, 0], 1).expect("version").end(),
            Err(Corrupt)
        );
    }

    #[test]
    fn long_strings_are_cut_at_a_char_boundary() {
        let s = "é".repeat(200); // 400 bytes
        let mut w = Writer::new(1);
        w.str(&s);
        let b = w.finish();
        let got = Reader::new(&b, 1).expect("version").str().expect("utf-8");
        assert_eq!(got.len(), 254);
        assert!(s.starts_with(&got));
    }
}

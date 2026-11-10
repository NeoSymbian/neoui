// SPDX-License-Identifier: Apache-2.0
//! The settings file format (version 1), all integers little-endian:
//!
//! ```text
//! header:  "NSET" magic, version: u16, count: u16
//! record:  key_len: u8, key (ASCII), value_len: u16, value (see `value.rs`)
//! ```
//!
//! Reading is forgiving: records are taken up to the first damaged one, and each is re-validated against the
//! current registry, so a value whose key changed type in a newer build is dropped instead of misread. Writing
//! refuses a store larger than [`FILE_MAX`].

use super::store::Store;
use super::{Error, KEY_MAX, Value};
use alloc::vec::Vec;

/// File magic.
pub const MAGIC: [u8; 4] = *b"NSET";
/// The format version this build writes. Older versions are migrated on load; newer ones are refused.
pub const VERSION: u16 = 1;
/// Largest settings file in bytes.
pub const FILE_MAX: usize = 16 * 1024;
const HEADER_LEN: usize = 8;

/// What [`decode`] found.
#[derive(Debug)]
pub struct Loaded {
    pub store: Store,
    /// The file's format version.
    pub version: u16,
    /// Records in the file that were not loaded (damaged, or no longer valid).
    pub dropped: usize,
}

/// Encodes the stored (non-default) values.
///
/// # Errors
/// [`Error::Full`] when the file would exceed [`FILE_MAX`].
pub fn encode(store: &Store) -> Result<Vec<u8>, Error> {
    let count = u16::try_from(store.len()).map_err(|_| Error::Full)?;
    let mut out = Vec::with_capacity(HEADER_LEN + 64 * store.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    for (key, value) in store.iter() {
        // both fit: keys are at most KEY_MAX (< 256) bytes, values at most ENCODED_MAX (< 65536)
        let key_len = u8::try_from(key.len()).map_err(|_| Error::BadKey)?;
        let value_len = u16::try_from(value.encoded_len()).map_err(|_| Error::TooLong)?;
        out.push(key_len);
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(&value_len.to_le_bytes());
        value.encode_into(&mut out);
        if out.len() > FILE_MAX {
            return Err(Error::Full);
        }
    }
    Ok(out)
}

/// Decodes a settings file.
///
/// # Errors
/// [`Error::Corrupt`] for a missing or wrong header or a file over [`FILE_MAX`]; [`Error::Unsupported`] for a
/// version newer than [`VERSION`]. Damaged records are not errors: they are counted in [`Loaded::dropped`].
pub fn decode(bytes: &[u8]) -> Result<Loaded, Error> {
    if bytes.len() < HEADER_LEN || bytes.len() > FILE_MAX || bytes[..4] != MAGIC {
        return Err(Error::Corrupt);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    let count = usize::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    if version == 0 {
        return Err(Error::Corrupt);
    }
    if version > VERSION {
        return Err(Error::Unsupported);
    }
    let mut store = Store::new();
    let mut rest = &bytes[HEADER_LEN..];
    let mut dropped = 0;
    for seen in 0..count {
        let Some((key, value, tail)) = record(rest) else {
            // damaged framing: nothing after it can be found
            dropped += count - seen;
            break;
        };
        rest = tail;
        // a well-framed but invalid record is skipped; the next one is still readable
        let loaded = core::str::from_utf8(key)
            .ok()
            .zip(Value::decode(value).ok())
            .is_some_and(|(key, value)| store.load_raw(key, value).is_ok());
        if !loaded {
            dropped += 1;
        }
    }
    migrate(version, &mut store)?;
    Ok(Loaded {
        store,
        version,
        dropped,
    })
}

/// Splits one framed record off `b`: `(key, value, rest)`, or `None` when the framing is damaged.
fn record(b: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    let (&key_len, b) = b.split_first()?;
    let key_len = usize::from(key_len);
    if key_len == 0 || key_len > KEY_MAX || b.len() < key_len + 2 {
        return None;
    }
    let (key, b) = b.split_at(key_len);
    let value_len = usize::from(u16::from_le_bytes([b[0], b[1]]));
    let b = &b[2..];
    (b.len() >= value_len).then(|| {
        let (value, rest) = b.split_at(value_len);
        (key, value, rest)
    })
}

/// Brings a store read from an older format version up to [`VERSION`]: renamed or retyped keys go here, one step
/// per version. Version 1 is the first, so there is nothing to do yet.
fn migrate(from: u16, _store: &mut Store) -> Result<(), Error> {
    match from {
        VERSION => Ok(()),
        _ => Err(Error::Unsupported),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::keys;
    use alloc::vec;

    fn sample() -> Store {
        let mut s = Store::new();
        let _ = s.set(keys::RADIO_WIFI, false);
        let _ = s.set(keys::DISPLAY_BRIGHTNESS, 42);
        let _ = s.set_raw("clock.scratch", Value::Bytes(vec![7; 100]));
        s
    }

    #[test]
    fn a_store_round_trips() {
        let bytes = encode(&sample()).expect("fits");
        let loaded = decode(&bytes).expect("valid");
        assert_eq!(loaded.version, VERSION);
        assert_eq!(loaded.dropped, 0);
        assert_eq!(loaded.store.get(keys::RADIO_WIFI), Ok(false));
        assert_eq!(loaded.store.get(keys::DISPLAY_BRIGHTNESS), Ok(42));
        assert_eq!(
            loaded.store.get_raw("clock.scratch"),
            Ok(Value::Bytes(vec![7; 100]))
        );
        assert_eq!(loaded.store.seq(), 0, "loading is not a change");
    }

    #[test]
    fn an_empty_store_is_just_a_header() {
        let bytes = encode(&Store::new()).expect("fits");
        assert_eq!(bytes.len(), HEADER_LEN);
        assert!(decode(&bytes).expect("valid").store.is_empty());
    }

    #[test]
    fn bad_headers_are_refused() {
        let good = encode(&sample()).expect("fits");
        assert_eq!(decode(&[]).err(), Some(Error::Corrupt));
        assert_eq!(decode(&good[..5]).err(), Some(Error::Corrupt));
        let mut magic = good.clone();
        magic[0] = b'X';
        assert_eq!(decode(&magic).err(), Some(Error::Corrupt));
        let mut newer = good.clone();
        newer[4..6].copy_from_slice(&(VERSION + 1).to_le_bytes());
        assert_eq!(decode(&newer).err(), Some(Error::Unsupported));
        let mut zero = good;
        zero[4..6].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(decode(&zero).err(), Some(Error::Corrupt));
        assert_eq!(decode(&vec![0; FILE_MAX + 1]).err(), Some(Error::Corrupt));
    }

    #[test]
    fn a_truncated_file_keeps_the_records_before_the_damage() {
        let bytes = encode(&sample()).expect("fits");
        // records are in key order: clock.alarms, display.brightness, radio.wifi; cut into the last
        let loaded = decode(&bytes[..bytes.len() - 1]).expect("header ok");
        assert_eq!(loaded.store.len(), 2);
        assert_eq!(loaded.dropped, 1);
        assert_eq!(
            loaded.store.get(keys::RADIO_WIFI),
            Ok(true),
            "back to default"
        );
    }

    #[test]
    fn an_invalid_record_is_skipped_and_the_rest_kept() {
        let mut s = Store::new();
        let _ = s.set_raw("clock.x", Value::Int(1));
        let _ = s.set(keys::RADIO_WIFI, false);
        let mut bytes = encode(&s).expect("fits");
        // make clock.x's value an invalid bool encoding of the right length: tag 1 + 4 bytes
        let at = HEADER_LEN + 1 + "clock.x".len() + 2;
        bytes[at] = 1;
        let loaded = decode(&bytes).expect("header ok");
        assert_eq!(loaded.dropped, 1);
        assert_eq!(loaded.store.get(keys::RADIO_WIFI), Ok(false));
    }

    #[test]
    fn a_retyped_registered_key_is_dropped() {
        // a file from a build where radio.wifi was an Int
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.push(10);
        bytes.extend_from_slice(b"radio.wifi");
        let v = Value::Int(0).to_bytes();
        bytes.extend_from_slice(&(v.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&v);
        let loaded = decode(&bytes).expect("header ok");
        assert_eq!(loaded.dropped, 1);
        assert_eq!(loaded.store.get(keys::RADIO_WIFI), Ok(true));
    }

    #[test]
    fn an_oversized_store_is_refused() {
        let mut s = Store::new();
        for i in 0..10 {
            let name = alloc::format!("clock.blob{i}");
            let _ = s.set_raw(&name, Value::Bytes(vec![0; 2000]));
        }
        assert_eq!(encode(&s).err(), Some(Error::Full));
    }
}

// SPDX-License-Identifier: Apache-2.0
//! The settings service the UI server runs: the [`Store`], persisted through a [`Backend`] with a save delay so a
//! dragged slider writes the file once, access checks, and change subscriptions per client session.
//!
//! It is pure logic (no clock, no file system): the caller passes the time in and gets save outcomes and the
//! sessions to notify back, so it is tested on the host with an in-memory backend.

use super::access::Caller;
use super::file::{self, Loaded};
use super::key::{Key, Setting, is_valid_prefix};
use super::store::{Changed, Store};
use super::{Error, Value};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// A save happens this long after the last change.
pub const SAVE_DELAY_MS: u64 = 1000;
/// Most watch prefixes per session.
pub const WATCHES_MAX: usize = 8;

/// Where the settings bytes are kept.
pub trait Backend {
    /// The saved bytes, or `None` when nothing was saved yet.
    ///
    /// # Errors
    /// A Symbian error code when the storage can't be read.
    fn load(&mut self) -> Result<Option<Vec<u8>>, i32>;

    /// Replaces the saved bytes. Must be atomic: after a power cut, either the old or the new bytes are loaded.
    ///
    /// # Errors
    /// A Symbian error code when the storage can't be written.
    fn save(&mut self, bytes: &[u8]) -> Result<(), i32>;
}

/// A client session, as the IPC server numbers them.
pub type SessionId = u32;

/// Why loading or saving failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    /// The backend failed, with this Symbian error code.
    Backend(i32),
    /// The bytes or the store were invalid.
    Format(Error),
}

/// What [`Service::open`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadReport {
    /// Values loaded (non-default).
    pub keys: usize,
    /// The file's format version (`None`: nothing saved yet, or unreadable).
    pub version: Option<u16>,
    /// Records skipped as damaged or no longer valid.
    pub dropped: usize,
    /// Why nothing could be loaded; the service then starts from defaults.
    pub error: Option<StorageError>,
}

/// A save's result: the bytes written, or why it failed (the service stays dirty and retries).
pub type SaveOutcome = Result<usize, StorageError>;

/// The UI server's settings: a store, its backend, the save delay and the subscriptions.
#[derive(Debug)]
pub struct Service<B: Backend> {
    store: Store,
    backend: B,
    /// When the oldest unsaved change was due to be saved from: the last change, or the last failed save.
    dirty_at: Option<u64>,
    /// Each session's watch prefixes.
    watches: BTreeMap<SessionId, Vec<String>>,
    /// The store's change counter as of the last [`Service::take_notifications`].
    notified: u32,
}

impl<B: Backend> Service<B> {
    /// Loads the saved settings. Never fails: unreadable or invalid storage starts from defaults, as reported.
    pub fn open(mut backend: B) -> (Self, LoadReport) {
        let mut report = LoadReport {
            keys: 0,
            version: None,
            dropped: 0,
            error: None,
        };
        let store = match backend.load() {
            Ok(None) => Store::new(),
            Ok(Some(bytes)) => match file::decode(&bytes) {
                Ok(Loaded {
                    store,
                    version,
                    dropped,
                }) => {
                    report.keys = store.len();
                    report.version = Some(version);
                    report.dropped = dropped;
                    store
                }
                Err(e) => {
                    report.error = Some(StorageError::Format(e));
                    Store::new()
                }
            },
            Err(e) => {
                report.error = Some(StorageError::Backend(e));
                Store::new()
            }
        };
        let notified = store.seq();
        let service = Self {
            store,
            backend,
            dirty_at: None,
            watches: BTreeMap::new(),
            notified,
        };
        (service, report)
    }

    /// The settings, for reading.
    #[must_use]
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The effective value of a typed key.
    ///
    /// # Errors
    /// As [`Store::get`].
    pub fn get<T: Setting>(&self, key: Key<T>) -> Result<T, Error> {
        self.store.get(key)
    }

    /// The effective value of `name`.
    ///
    /// # Errors
    /// As [`Store::get_raw`].
    pub fn get_raw(&self, name: &str) -> Result<Value, Error> {
        self.store.get_raw(name)
    }

    /// Writes a typed key on behalf of `caller`.
    ///
    /// # Errors
    /// As [`Service::set_raw`].
    pub fn set<T: Setting>(
        &mut self,
        caller: Caller<'_>,
        key: Key<T>,
        value: T,
        now_ms: u64,
    ) -> Result<Changed, Error> {
        self.set_raw(caller, key.name(), value.into_value(), now_ms)
    }

    /// Writes `name` on behalf of `caller` at time `now_ms`; a change is saved [`SAVE_DELAY_MS`] later.
    ///
    /// # Errors
    /// [`Error::Denied`] when the caller may not write the key, else as [`Store::set_raw`].
    pub fn set_raw(
        &mut self,
        caller: Caller<'_>,
        name: &str,
        value: Value,
        now_ms: u64,
    ) -> Result<Changed, Error> {
        if !caller.may_write(name) {
            return Err(Error::Denied);
        }
        let changed = self.store.set_raw(name, value)?;
        if changed == Changed::Yes {
            self.dirty_at = Some(now_ms);
        }
        Ok(changed)
    }

    /// Whether there are unsaved changes.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty_at.is_some()
    }

    /// Saves when the save delay has passed since the last change (or failed save). Call once per frame.
    pub fn tick(&mut self, now_ms: u64) -> Option<SaveOutcome> {
        let due = self.dirty_at? + SAVE_DELAY_MS;
        if now_ms < due {
            return None;
        }
        let outcome = self.save();
        if outcome.is_err() {
            self.dirty_at = Some(now_ms); // retry one delay later, not every frame
        }
        Some(outcome)
    }

    /// Saves now if there are unsaved changes (before a restart or power-off).
    pub fn flush(&mut self) -> Option<SaveOutcome> {
        self.dirty_at?;
        Some(self.save())
    }

    fn save(&mut self) -> SaveOutcome {
        let bytes = file::encode(&self.store).map_err(StorageError::Format)?;
        self.backend.save(&bytes).map_err(StorageError::Backend)?;
        self.dirty_at = None;
        Ok(bytes.len())
    }

    /// Subscribes `session` to changes of keys starting with `prefix` (empty: every key).
    ///
    /// # Errors
    /// [`Error::BadKey`] for an invalid prefix, [`Error::Full`] past [`WATCHES_MAX`] prefixes.
    pub fn watch(&mut self, session: SessionId, prefix: &str) -> Result<(), Error> {
        if !is_valid_prefix(prefix) {
            return Err(Error::BadKey);
        }
        let prefixes = self.watches.entry(session).or_default();
        if prefixes.iter().any(|p| p == prefix) {
            return Ok(());
        }
        if prefixes.len() >= WATCHES_MAX {
            return Err(Error::Full);
        }
        prefixes.push(prefix.into());
        Ok(())
    }

    /// Forgets a closed session's subscriptions.
    pub fn drop_session(&mut self, session: SessionId) {
        self.watches.remove(&session);
    }

    /// The sessions with a watch matching a key changed since the last call, and the change counter to tell them.
    /// Call once per frame: a session is told at most once per call however many of its keys changed.
    pub fn take_notifications(&mut self) -> (Vec<SessionId>, u32) {
        let (since, now) = (self.notified, self.store.seq());
        self.notified = now;
        if since == now {
            return (Vec::new(), now);
        }
        let changed: Option<Vec<&str>> = self.store.changes_since(since).map(Iterator::collect);
        let sessions = self
            .watches
            .iter()
            .filter(|(_, prefixes)| match &changed {
                Some(keys) => prefixes
                    .iter()
                    .any(|p| keys.iter().any(|k| k.starts_with(p.as_str()))),
                None => true, // the journal lost track: tell everyone who watches anything
            })
            .map(|(&id, _)| id)
            .collect();
        (sessions, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::keys;
    use alloc::vec;

    /// An in-memory backend that can be told to fail.
    #[derive(Debug, Default)]
    struct Mem {
        bytes: Option<Vec<u8>>,
        saves: usize,
        fail: Option<i32>,
    }

    impl Backend for Mem {
        fn load(&mut self) -> Result<Option<Vec<u8>>, i32> {
            match self.fail {
                Some(e) => Err(e),
                None => Ok(self.bytes.clone()),
            }
        }
        fn save(&mut self, bytes: &[u8]) -> Result<(), i32> {
            if let Some(e) = self.fail {
                return Err(e);
            }
            self.bytes = Some(bytes.to_vec());
            self.saves += 1;
            Ok(())
        }
    }

    fn fresh() -> Service<Mem> {
        Service::open(Mem::default()).0
    }

    #[test]
    fn first_boot_starts_from_defaults() {
        let (s, report) = Service::open(Mem::default());
        assert_eq!(report.keys, 0);
        assert_eq!(report.error, None);
        assert_eq!(s.get(keys::RADIO_WIFI), Ok(true));
    }

    #[test]
    fn unreadable_or_invalid_storage_starts_from_defaults() {
        let failing = Mem {
            fail: Some(-20),
            ..Mem::default()
        };
        let (_, report) = Service::open(failing);
        assert_eq!(report.error, Some(StorageError::Backend(-20)));
        let garbage = Mem {
            bytes: Some(vec![1, 2, 3]),
            ..Mem::default()
        };
        let (s, report) = Service::open(garbage);
        assert_eq!(report.error, Some(StorageError::Format(Error::Corrupt)));
        assert!(s.store().is_empty());
    }

    #[test]
    fn a_change_is_saved_after_the_delay_and_survives_a_reopen() {
        let mut s = fresh();
        assert_eq!(
            s.set(Caller::Shell, keys::RADIO_WIFI, false, 100),
            Ok(Changed::Yes)
        );
        assert!(s.tick(100 + SAVE_DELAY_MS - 1).is_none());
        assert!(matches!(s.tick(100 + SAVE_DELAY_MS), Some(Ok(_))));
        assert!(!s.is_dirty());
        assert!(s.tick(10_000).is_none(), "nothing more to save");
        let (again, report) = Service::open(Mem {
            bytes: s.backend.bytes.clone(),
            ..Mem::default()
        });
        assert_eq!(report.keys, 1);
        assert_eq!(report.version, Some(file::VERSION));
        assert_eq!(again.get(keys::RADIO_WIFI), Ok(false));
    }

    #[test]
    fn each_change_restarts_the_delay() {
        let mut s = fresh();
        for (t, v) in [(0, 10), (600, 20), (1200, 30)] {
            let _ = s.set(Caller::Shell, keys::DISPLAY_BRIGHTNESS, v, t);
            assert!(s.tick(t).is_none());
        }
        assert!(s.tick(1200 + SAVE_DELAY_MS - 1).is_none());
        assert!(s.tick(1200 + SAVE_DELAY_MS).is_some());
        assert_eq!(s.backend.saves, 1);
    }

    #[test]
    fn an_unchanged_write_is_not_saved() {
        let mut s = fresh();
        assert_eq!(
            s.set(Caller::Shell, keys::RADIO_WIFI, true, 0),
            Ok(Changed::No)
        );
        assert!(!s.is_dirty());
    }

    #[test]
    fn a_failed_save_stays_dirty_and_retries_after_the_delay() {
        let mut s = fresh();
        let _ = s.set(Caller::Shell, keys::RADIO_WIFI, false, 0);
        s.backend.fail = Some(-26);
        assert_eq!(s.tick(SAVE_DELAY_MS), Some(Err(StorageError::Backend(-26))));
        assert!(s.is_dirty());
        s.backend.fail = None;
        assert!(s.tick(SAVE_DELAY_MS + 1).is_none(), "waits a delay");
        assert!(matches!(s.tick(2 * SAVE_DELAY_MS), Some(Ok(_))));
    }

    #[test]
    fn flush_saves_at_once_and_only_when_dirty() {
        let mut s = fresh();
        assert!(s.flush().is_none());
        let _ = s.set(Caller::Shell, keys::CLOCK_24H, false, 0);
        assert!(matches!(s.flush(), Some(Ok(_))));
        assert!(s.flush().is_none());
    }

    #[test]
    fn writes_are_access_checked() {
        let mut s = fresh();
        assert_eq!(
            s.set(Caller::App("clock"), keys::RADIO_WIFI, false, 0),
            Err(Error::Denied)
        );
        assert_eq!(
            s.set_raw(Caller::App("clock"), "clock.scratch", Value::Int(3), 0),
            Ok(Changed::Yes)
        );
    }

    #[test]
    fn watchers_are_told_once_per_call_about_matching_keys() {
        let mut s = fresh();
        s.watch(1, "clock.").expect("valid");
        s.watch(2, "radio.").expect("valid");
        s.watch(3, "").expect("valid");
        let _ = s.set(Caller::Shell, keys::CLOCK_24H, false, 0);
        let _ = s.set_raw(Caller::Shell, "clock.scratch", Value::Int(1), 0);
        let (sessions, seq) = s.take_notifications();
        assert_eq!(sessions, [1, 3]);
        assert_eq!(seq, 2);
        assert_eq!(s.take_notifications().0, [] as [SessionId; 0]);
    }

    #[test]
    fn watches_are_validated_capped_and_dropped_with_the_session() {
        let mut s = fresh();
        assert_eq!(s.watch(1, "Bad."), Err(Error::BadKey));
        for i in 0..WATCHES_MAX {
            s.watch(1, &alloc::format!("app{i}.")).expect("room");
        }
        s.watch(1, "app0.").expect("a repeat is not a new watch");
        assert_eq!(s.watch(1, "more."), Err(Error::Full));
        s.drop_session(1);
        let _ = s.set_raw(Caller::Shell, "app0.x", Value::Int(1), 0);
        assert!(s.take_notifications().0.is_empty());
    }
}

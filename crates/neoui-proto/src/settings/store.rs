// SPDX-License-Identifier: Apache-2.0
//! The settings store: validated values in memory, with defaults from the registry and a journal of recent
//! changes for change notifications.
//!
//! Only values that differ from their registered default are kept, so a default changed in a later build reaches
//! every phone that never touched the setting.

use super::key::{Key, Setting, is_valid_name};
use super::registry::{self, Spec};
use super::{Error, Value};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;

/// How many recent changes the journal keeps. A watcher that falls further behind is told "everything changed".
pub const JOURNAL_LEN: usize = 64;

/// Whether a write changed the effective value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Changed {
    No,
    Yes,
}

/// Validated settings in memory.
#[derive(Debug, Default)]
pub struct Store {
    /// Values that differ from their default (or keys without one).
    values: BTreeMap<String, Value>,
    /// Bumped by every change; 0 = nothing changed since load.
    seq: u32,
    /// `(seq, key)` of the last [`JOURNAL_LEN`] changes, oldest first.
    journal: VecDeque<(u32, String)>,
}

impl Store {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The effective value of a typed key.
    ///
    /// # Errors
    /// [`Error::NotFound`] when the key has neither a stored value nor a registered default.
    pub fn get<T: Setting>(&self, key: Key<T>) -> Result<T, Error> {
        // registered keys are type-checked on write; an unregistered app key written untyped (over IPC) with
        // another type than `T` is reported, not coerced
        T::from_value(self.get_raw(key.name())?).ok_or(Error::WrongType)
    }

    /// The effective value of `name`: stored, else its registered default.
    ///
    /// # Errors
    /// [`Error::BadKey`] for an invalid name, [`Error::NotFound`] when there is no value and no default.
    pub fn get_raw(&self, name: &str) -> Result<Value, Error> {
        if !is_valid_name(name) {
            return Err(Error::BadKey);
        }
        match (self.values.get(name), registry::lookup(name)) {
            (Some(v), _) => Ok(v.clone()),
            (None, Some(spec)) => Ok(spec.default.to_value()),
            (None, None) => Err(Error::NotFound),
        }
    }

    /// Writes a typed key.
    ///
    /// # Errors
    /// As [`Store::set_raw`].
    pub fn set<T: Setting>(&mut self, key: Key<T>, value: T) -> Result<Changed, Error> {
        self.set_raw(key.name(), value.into_value())
    }

    /// Writes `name`, validating the name, the type-wide length limits and, for a registered key, its type and
    /// limit. A value equal to the default is removed rather than stored. Access rights are not checked here (see
    /// [`super::access`]).
    ///
    /// # Errors
    /// [`Error::BadKey`], [`Error::WrongType`], [`Error::OutOfRange`] or [`Error::TooLong`].
    pub fn set_raw(&mut self, name: &str, value: Value) -> Result<Changed, Error> {
        let spec = Self::validate(name, &value)?;
        let changed = self.put(name, value, spec);
        if changed == Changed::Yes {
            self.record(name);
        }
        Ok(changed)
    }

    /// Like [`Store::set_raw`] but without a journal entry: for values read back from storage.
    pub(super) fn load_raw(&mut self, name: &str, value: Value) -> Result<(), Error> {
        let spec = Self::validate(name, &value)?;
        let _ = self.put(name, value, spec);
        Ok(())
    }

    fn validate(name: &str, value: &Value) -> Result<Option<&'static Spec>, Error> {
        if !is_valid_name(name) {
            return Err(Error::BadKey);
        }
        value.check_len()?;
        let spec = registry::lookup(name);
        if let Some(spec) = spec {
            spec.check(value)?;
        }
        Ok(spec)
    }

    fn put(&mut self, name: &str, value: Value, spec: Option<&Spec>) -> Changed {
        if spec.is_some_and(|s| s.default.matches(&value)) {
            return match self.values.remove(name) {
                Some(_) => Changed::Yes,
                None => Changed::No,
            };
        }
        match self.values.get_mut(name) {
            Some(old) if *old == value => Changed::No,
            Some(old) => {
                *old = value;
                Changed::Yes
            }
            None => {
                self.values.insert(name.into(), value);
                Changed::Yes
            }
        }
    }

    fn record(&mut self, name: &str) {
        self.seq = self.seq.wrapping_add(1);
        if self.journal.len() == JOURNAL_LEN {
            self.journal.pop_front();
        }
        self.journal.push_back((self.seq, name.into()));
    }

    /// The change counter: bumped by every change.
    #[must_use]
    pub fn seq(&self) -> u32 {
        self.seq
    }

    /// The keys changed after `seq`, oldest first, or `None` when the journal no longer reaches back that far
    /// (treat it as "anything may have changed").
    #[must_use]
    pub fn changes_since(&self, seq: u32) -> Option<impl Iterator<Item = &str>> {
        let behind = self.seq.wrapping_sub(seq) as usize;
        (behind <= self.journal.len()).then(|| {
            self.journal
                .iter()
                .skip(self.journal.len() - behind)
                .map(|(_, k)| k.as_str())
        })
    }

    /// The stored (non-default) values, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// How many values are stored (differ from their default).
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::keys;
    use alloc::vec::Vec;

    #[test]
    fn defaults_are_read_but_not_stored() {
        let mut s = Store::new();
        assert_eq!(s.get(keys::RADIO_WIFI), Ok(true));
        assert_eq!(s.get(keys::DISPLAY_BRIGHTNESS), Ok(160));
        assert_eq!(s.set(keys::RADIO_WIFI, true), Ok(Changed::No));
        assert!(s.is_empty());
        assert_eq!(s.seq(), 0);
    }

    #[test]
    fn a_change_is_stored_and_setting_the_default_removes_it() {
        let mut s = Store::new();
        assert_eq!(s.set(keys::RADIO_WIFI, false), Ok(Changed::Yes));
        assert_eq!(s.get(keys::RADIO_WIFI), Ok(false));
        assert_eq!(s.len(), 1);
        assert_eq!(s.set(keys::RADIO_WIFI, false), Ok(Changed::No));
        assert_eq!(s.set(keys::RADIO_WIFI, true), Ok(Changed::Yes));
        assert!(s.is_empty());
        assert_eq!(s.seq(), 2);
    }

    #[test]
    fn registered_keys_are_validated() {
        let mut s = Store::new();
        assert_eq!(
            s.set_raw("radio.wifi", Value::Int(1)),
            Err(Error::WrongType)
        );
        assert_eq!(s.set(keys::DISPLAY_BRIGHTNESS, 300), Err(Error::OutOfRange));
        assert_eq!(
            s.set_raw("Radio.wifi", Value::Bool(true)),
            Err(Error::BadKey)
        );
        assert_eq!(s.get_raw("radio"), Err(Error::BadKey));
        assert_eq!(s.seq(), 0);
    }

    #[test]
    fn unregistered_keys_hold_any_type_within_size_limits() {
        let mut s = Store::new();
        assert_eq!(s.get_raw("clock.alarms"), Err(Error::NotFound));
        assert_eq!(
            s.set_raw("clock.alarms", Value::Bytes(alloc::vec![1, 2])),
            Ok(Changed::Yes)
        );
        assert_eq!(
            s.get_raw("clock.alarms"),
            Ok(Value::Bytes(alloc::vec![1, 2]))
        );
        let long = Value::Str("x".repeat(super::super::value::STR_MAX + 1));
        assert_eq!(s.set_raw("clock.name", long), Err(Error::TooLong));
    }

    #[test]
    fn the_journal_lists_changes_since_a_seq() {
        let mut s = Store::new();
        let start = s.seq();
        let _ = s.set(keys::RADIO_WIFI, false);
        let mid = s.seq();
        let _ = s.set(keys::CLOCK_24H, false);
        let all: Vec<_> = s.changes_since(start).expect("in journal").collect();
        assert_eq!(all, ["radio.wifi", "clock.24h"]);
        let late: Vec<_> = s.changes_since(mid).expect("in journal").collect();
        assert_eq!(late, ["clock.24h"]);
        assert_eq!(s.changes_since(s.seq()).expect("in journal").count(), 0);
    }

    #[test]
    fn a_watcher_too_far_behind_is_told_so() {
        let mut s = Store::new();
        for i in 0..=JOURNAL_LEN as i32 {
            let _ = s.set_raw("clock.n", Value::Int(i + 1));
        }
        assert!(s.changes_since(0).is_none());
        assert!(s.changes_since(1).is_some());
    }

    #[test]
    fn loading_does_not_journal() {
        let mut s = Store::new();
        s.load_raw("radio.wifi", Value::Bool(false)).expect("valid");
        assert_eq!(s.get(keys::RADIO_WIFI), Ok(false));
        assert_eq!(s.seq(), 0);
    }
}

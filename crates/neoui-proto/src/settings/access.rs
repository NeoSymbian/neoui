// SPDX-License-Identifier: Apache-2.0
//! Who may write which setting. Reading is open to every client; writing follows the key's owner:
//! - the UI server (the shell) may write everything;
//! - an app may write registered keys it owns ([`Owner::App`]) and unregistered keys in its own namespace.
//!
//! Step 0 of docs/neoui-shell.md (caller identity by secure ID and capabilities) extends [`Caller`] here.

use super::key::namespace;
use super::registry::{self, Owner};
use core::fmt;

/// Who is asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller<'a> {
    /// The UI server itself.
    Shell,
    /// An app, by its key (`"clock"`).
    App(&'a str),
}

impl Caller<'_> {
    /// Whether this caller may write `name` (a valid key name).
    #[must_use]
    pub fn may_write(&self, name: &str) -> bool {
        match *self {
            Caller::Shell => true,
            Caller::App(app) => match registry::lookup(name) {
                Some(spec) => match spec.owner {
                    Owner::App(owner) => owner == app,
                    Owner::Apps(owners) => owners.contains(&app),
                    Owner::Any => true,
                    Owner::Shell => false,
                },
                None => namespace(name) == app,
            },
        }
    }
}

/// For logs: `shell` or the app key.
impl fmt::Display for Caller<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Caller::Shell => f.write_str("shell"),
            Caller::App(app) => f.write_str(app),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shell_writes_everything() {
        assert!(Caller::Shell.may_write("radio.wifi"));
        assert!(Caller::Shell.may_write("clock.scratch"));
    }

    #[test]
    fn an_app_writes_only_its_own_namespace() {
        let clock = Caller::App("clock");
        assert!(clock.may_write("clock.scratch"));
        assert!(!clock.may_write("files.sort"));
        assert!(!clock.may_write("radio.wifi"));
    }

    #[test]
    fn a_registered_key_in_an_app_namespace_still_follows_its_owner() {
        // clock.24h is registered as the clock app's (its settings page sets it); another app can't
        assert!(Caller::App("clock").may_write("clock.24h"));
        assert!(!Caller::App("files").may_write("clock.24h"));
        // shared keys: Phone (its Settings page) and Contacts (a card's speed dial key)
        assert!(Caller::App("phone").may_write("phone.speed"));
        assert!(Caller::App("contacts").may_write("phone.speed"));
        assert!(!Caller::App("messages").may_write("phone.speed"));
        assert!(Caller::App("contacts").may_write("contacts.name_order"));
        // an unregistered key in the app's own namespace
        assert!(Caller::App("contacts").may_write("contacts.rev"));
        assert!(!Caller::App("phone").may_write("contacts.rev"));
        // clock.alarms is registered as the clock app's own
        assert!(Caller::App("clock").may_write("clock.alarms"));
        assert!(!Caller::App("files").may_write("clock.alarms"));
    }
}

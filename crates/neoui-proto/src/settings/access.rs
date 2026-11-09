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
                Some(spec) => matches!(spec.owner, Owner::App(owner) if owner == app),
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
        assert!(Caller::Shell.may_write("clock.alarms"));
    }

    #[test]
    fn an_app_writes_only_its_own_namespace() {
        let clock = Caller::App("clock");
        assert!(clock.may_write("clock.alarms"));
        assert!(!clock.may_write("files.sort"));
        assert!(!clock.may_write("radio.wifi"));
    }

    #[test]
    fn a_registered_key_in_an_app_namespace_still_follows_its_owner() {
        // clock.24h is a system setting (Owner::Shell) even though it sits under "clock."
        assert!(!Caller::App("clock").may_write("clock.24h"));
    }
}

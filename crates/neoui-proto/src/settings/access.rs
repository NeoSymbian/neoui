// SPDX-License-Identifier: Apache-2.0
//! Who may write which setting. Reading is open to every client; writing follows the key's owner:
//! - the UI server (the shell) may write everything;
//! - an app may write registered keys it owns ([`Owner::App`]) and unregistered keys in its own namespace;
//! - the Settings app (`NEOSET.EXE`, recognised by its secure ID, not its app key) may also write every shell-owned
//!   key and the few app keys it shows ([`SETTINGS_ALSO`]).
//!
//! Step 0 of docs/neoui-shell.md (caller identity by secure ID) started with [`Caller::Settings`].

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
    /// The Settings app, whose messages carry its secure ID ([`SETTINGS_SID`]).
    Settings,
}

/// `NEOSET.EXE`'s UID3, which the kernel reports as the secure ID of its messages.
pub const SETTINGS_SID: u32 = 0xE000_2007;

/// Who an app's message comes from: the Settings app only when both its screen key and its secure ID say so (a
/// process named "settings" isn't enough).
#[must_use]
pub fn caller(key: &str, sid: u32) -> Caller<'_> {
    if key == "settings" && sid == SETTINGS_SID {
        Caller::Settings
    } else {
        Caller::App(key)
    }
}

/// Shell keys only the shell writes, even for the Settings app: the sound profiles go through the shell's reducer
/// (`ipc::ACTION_PROFILE`) so the profile, its blob and its live values change together; home's layout is the shell's
/// home editor's (D10: Settings opens the editor with `ipc::ACTION_EDIT_HOME`), and the tile order the control
/// centre's editor's (TP7, `ipc::ACTION_EDIT_TILES`); whether a lock PIN is set only SYSSTART's PIN dialog knows (TP9).
pub const SHELL_ONLY: &[&str] = &[
    "sound.profile",
    "sound.profiles",
    "sound.tones",
    "sound.feedback",
    "sound.your_tones",
    "sound.tones_yours",
    "sound.keypad_tones",
    "sound.touch_vibrate",
    "home.layout",
    "shell.tiles",
    "lock.pin_set",
];

/// App-owned keys the Settings app shows and may change (Date & time's 24-hour time is Clock's key).
pub const SETTINGS_ALSO: &[&str] = &["clock.24h"];

impl Caller<'_> {
    /// Whether this caller may write `name` (a valid key name).
    #[must_use]
    pub fn may_write(&self, name: &str) -> bool {
        match *self {
            Caller::Shell => true,
            Caller::Settings => match registry::lookup(name) {
                Some(spec) => {
                    (spec.owner == Owner::Shell && !SHELL_ONLY.contains(&name))
                        || SETTINGS_ALSO.contains(&name)
                }
                None => namespace(name) == "settings",
            },
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
            Caller::Settings => f.write_str("settings"),
            Caller::App(app) => f.write_str(app),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_needs_its_key_and_its_secure_id() {
        assert_eq!(caller("settings", SETTINGS_SID), Caller::Settings);
        assert_eq!(caller("settings", 0xE000_2001), Caller::App("settings"));
        assert_eq!(caller("clock", SETTINGS_SID), Caller::App("clock"));
    }

    #[test]
    fn the_shell_writes_everything() {
        assert!(Caller::Shell.may_write("radio.wifi"));
        assert!(Caller::Shell.may_write("clock.scratch"));
    }

    #[test]
    fn the_settings_app_writes_shell_keys_and_its_own() {
        let s = Caller::Settings;
        assert!(s.may_write("radio.wifi"));
        assert!(s.may_write("legacy.pad.b"));
        assert!(s.may_write("clock.24h"));
        assert!(s.may_write("settings.root_grid"));
        // an app's own records stay the app's
        assert!(!s.may_write("clock.alarms"));
        assert!(!s.may_write("phone.speed"));
        assert!(!s.may_write("files.sort"));
        // the shell's own editors and the PIN dialog (1.22)
        assert!(!s.may_write("shell.tiles") && !s.may_write("lock.pin_set"));
        assert!(s.may_write("lock.on") && s.may_write("ui.language"));
        assert!(Caller::Shell.may_write("lock.pin_set"));
        // the widgets' content is the shell's to keep, not the app's (`widget.<app>.<name>`)
        assert!(
            !Caller::App("clock").may_write("widget.clock.next")
                && !s.may_write("widget.clock.next")
        );
        // an app that calls itself "settings" is still only an app: no shell keys
        assert!(!Caller::App("settings").may_write("radio.wifi"));
        assert_eq!(alloc::format!("{s}"), "settings");
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

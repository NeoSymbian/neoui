// SPDX-License-Identifier: GPL-3.0-only
//! Notification channels and the user's policy in SYSSTART (top-panel TP4, protocol 1.21; the model is
//! [`neoui_proto::notification_policy`]): the apps' declarations (`CHANNEL`, kept in [`CATALOGUE_PATH`] so they outlive
//! their app and a restart), a card's admission (an app or channel turned off refuses it), what a new card does now
//! (a banner, a chime and a buzz, held by Do not disturb and the rate limit), the badge and glyph projection, Do not
//! disturb's end, and INFO 7 for Settings. A child of `host`: it works on the host's cards.
//!
//! Calls and SMS tones are MODEMD's, and Clock rings its alarms: the shell plays no sound for Phone, Messages and
//! DTSRV (one owner per alert) until NeoModem's alert policy lets the shell hold those too.

use super::{Host, read_bytes};
use crate::settings;
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use e32::server::Message;
use e32::sys::err;
use neoui_proto::apps;
use neoui_proto::ipc::{self as proto};
use neoui_proto::notification_policy::{
    self as policy, Alert, CHANNEL_MAX, CHANNELS_MAX, Channel, Effective, OWNERS, Policy, Post,
    Rate,
};
use neoui_proto::page::{NOTIFY_ONGOING, NOTIFY_SILENT_UPDATE, Notification};
use neoui_proto::settings::{Caller, Value, kerr, keys};

/// The declarations across restarts.
const CATALOGUE_PATH: &str = "C:\\private\\e0002001\\channels.dat";
/// The most a saved catalogue can be.
const CATALOGUE_MAX: usize = 3 + OWNERS.len() * CHANNELS_MAX * (1 + 16 + 1 + CHANNEL_MAX);

/// Whose alert sounds aren't the shell's to play (MODEMD rings calls and SMS, Clock its alarms).
fn sound_owned_elsewhere(app: &str) -> bool {
    matches!(app, "phone" | "messages" | "dtsrv")
}

/// What a new card asked the main loop to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub key: &'static str,
    pub id: u32,
    pub banner: bool,
    /// A tone id (`sound_profile` 0–3) to chime and whether to buzz; `None`: silent.
    pub chime: Option<u8>,
    pub buzz: bool,
}

/// The host's TP4 state.
pub struct Policies {
    catalogue: Vec<(&'static str, Channel)>,
    /// The saved catalogue was read (or there is none): until then a declaration answers `KErrNotReady` and nothing
    /// is saved, so a failed read never costs the other apps' declarations.
    loaded: bool,
    rate: Rate,
    /// The store's change counter the policy was last applied at.
    seq: u32,
    /// The alert of the newest card, for the main loop.
    pub pending: Option<Pending>,
}

/// A damaged catalogue, kept as it was (the last one only) before a fresh one is written.
const CATALOGUE_DAMAGED: &str = "C:\\private\\e0002001\\channels.bad";

/// The file a save writes first, then puts in [`CATALOGUE_PATH`]'s place.
const CATALOGUE_STAGING: &str = "C:\\private\\e0002001\\channels.tmp";

impl Policies {
    /// The saved declarations; read again on the next declaration if that failed.
    pub fn boot() -> Self {
        let mut p = Self {
            catalogue: Vec::new(),
            loaded: false,
            rate: Rate::default(),
            seq: 0,
            pending: None,
        };
        p.load();
        p
    }

    /// Read the saved catalogue, if it isn't yet. A damaged one is kept aside and its apps declare again on their next
    /// connect; an unreadable one (or a damaged one that can't be kept aside) is tried again later.
    fn load(&mut self) -> bool {
        if self.loaded {
            return true;
        }
        let read = || -> Result<Vec<u8>, i32> {
            let fs = Fs::connect()?;
            let f = File::open(&fs, CATALOGUE_PATH, mode::READ | mode::SHARE_READERS_ONLY)?;
            let size = usize::try_from(f.size()?).map_err(|_| kerr::CORRUPT)?;
            if size > CATALOGUE_MAX {
                return Err(kerr::CORRUPT);
            }
            let mut b = alloc::vec![0; size];
            // a short read is a failed one, never taken as the catalogue
            if f.read_at(0, &mut b)? != size {
                return Err(kerr::CORRUPT);
            }
            Ok(b)
        };
        self.catalogue = match read() {
            Ok(b) => match policy::decode_catalogue(&b) {
                Some(c) => c,
                // never saved over: the damaged file is kept aside first (`channels.bad`), and until it is, every
                // write is refused (read again on the next declaration)
                None => match Fs::connect()
                    .and_then(|fs| fs.replace_file(CATALOGUE_PATH, CATALOGUE_DAMAGED))
                {
                    Ok(()) => {
                        dprintln!(
                            "neoui: channels file damaged, kept as channels.bad; apps declare again"
                        );
                        Vec::new()
                    }
                    Err(e) => {
                        dprintln!(
                            "neoui: channels file damaged and can't be kept aside ({}), writes refused",
                            e
                        );
                        return false;
                    }
                },
            },
            Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => Vec::new(),
            Err(e) => {
                dprintln!("neoui: channels file unreadable ({}), trying again", e);
                return false;
            }
        };
        self.loaded = true;
        true
    }

    /// Write `list` to [`CATALOGUE_STAGING`], flushed and closed, which then replaces [`CATALOGUE_PATH`] in one
    /// file-server operation: a failed save leaves the last catalogue whole.
    fn save(list: &[(&'static str, Channel)]) -> Result<(), i32> {
        let refs: Vec<(&str, &Channel)> = list.iter().map(|(k, c)| (*k, c)).collect();
        let bytes = policy::encode_catalogue(&refs);
        let fs = Fs::connect()?;
        for dir in ["C:\\private\\", "C:\\private\\e0002001\\"] {
            match fs.mkdir(dir) {
                Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
                Err(e) => return Err(e),
            }
        }
        {
            let f = File::replace(&fs, CATALOGUE_STAGING, mode::SHARE_EXCLUSIVE)?;
            f.write(&bytes)?;
            f.flush()?;
        }
        fs.replace_file(CATALOGUE_STAGING, CATALOGUE_PATH)
    }

    fn declared(&self, app: &str, channel: u8) -> Option<u8> {
        self.catalogue
            .iter()
            .find(|(k, c)| *k == app && c.id == channel)
            .map(|(_, c)| c.importance)
    }
}

impl Host {
    /// App `app`'s `n.<app>` (the defaults for an app without one, or a value that can't be read).
    fn app_policy(&self, app: &str) -> Policy {
        policy::key_of(app)
            .and_then(|k| match self.settings.get_raw(k) {
                Ok(Value::Bytes(b)) => Policy::decode(&b),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn lock_global(&self) -> u8 {
        policy::global_lock(self.settings.get(keys::NOTIFY_LOCK).unwrap_or(0))
    }

    /// The effective policy of app `app`'s channel `channel` (the one function: badge, shade, banner, sound, lock).
    pub fn effective(&self, app: &str, channel: u8) -> Effective {
        let declared = self.policies.declared(app, channel);
        policy::effective(&self.app_policy(app), channel, declared, self.lock_global())
    }

    /// Do not disturb now.
    pub fn dnd(&self) -> bool {
        let until = match self.settings.get_raw(keys::NOTIFY_DNDTO.name()) {
            Ok(Value::Bytes(b)) => b,
            _ => Vec::new(),
        };
        policy::dnd_active(
            self.settings.get(keys::NOTIFY_DND).unwrap_or(false),
            &until,
            e32::time::utc_now().0,
        )
    }

    /// `CHANNEL [Des8 declaration]` from app `key` (its own EXE, checked by the caller).
    pub(super) fn channel(&mut self, key: &'static str, m: &Message) -> i32 {
        if !OWNERS.contains(&key) {
            return kerr::PERMISSION_DENIED;
        }
        let Some(c) = read_bytes(m, 0, CHANNEL_MAX)
            .ok()
            .and_then(|b| Channel::decode(&b))
        else {
            return kerr::ARGUMENT;
        };
        // the saved catalogue first: a declaration never replaces what couldn't be read
        if !self.policies.load() {
            return err::KERR_NOT_READY;
        }
        // the new catalogue is saved before it is used or acknowledged; a failed save changes nothing
        let mut list = self.policies.catalogue.clone();
        let mine = list.iter().filter(|(k, _)| *k == key).count();
        match list.iter_mut().find(|(k, d)| *k == key && d.id == c.id) {
            // saved already (only a saved catalogue is ever in use)
            Some((_, d)) if *d == c => return 0,
            // new metadata; the user's override lives in n.<app> and stays
            Some((_, d)) => *d = c,
            None if mine >= CHANNELS_MAX => return kerr::OVERFLOW,
            None => list.push((key, c)),
        }
        if let Err(e) = Policies::save(&list) {
            dprintln!("neoui: channels save failed ({})", e);
            return e;
        }
        self.policies.catalogue = list;
        dprintln!("neoui: {} declared a channel", key);
        self.info_changed(proto::INFO_CHANNELS);
        0
    }

    /// A card app `key` posted: `Err(KErrPermissionDenied)` when its app or channel is off; else what it does now
    /// goes to [`Policies::pending`]. `old` is the card it replaces, if any. The shell's own card is never held.
    pub(super) fn admit(
        &mut self,
        key: &'static str,
        n: &Notification,
        old: Option<&Notification>,
        now_ms: u64,
    ) -> Result<(), i32> {
        let e = self.effective(key, n.channel);
        if !e.accepts() {
            return Err(kerr::PERMISSION_DENIED);
        }
        let post = Post {
            new: old.is_none(),
            quiet: n.flags & (NOTIFY_ONGOING | NOTIFY_SILENT_UPDATE) != 0,
        };
        let dnd = self.dnd();
        let rate = &mut self.policies.rate;
        let a: Alert = policy::alert(&e, post, dnd, || rate.take(key, now_ms));
        if !a.any() {
            return Ok(());
        }
        let (silent, vibrate) = (
            self.settings.get(keys::SOUND_SILENT).unwrap_or(false),
            self.settings.get(keys::SOUND_VIBRATE).unwrap_or(true),
        );
        let ours = a.sound && !sound_owned_elsewhere(key);
        let tone = match self.app_policy(key).tone {
            policy::TONE_PROFILE => self.settings.get(keys::SOUND_MESSAGE_TONE).unwrap_or(0) as u8,
            t => t,
        };
        self.policies.pending = Some(Pending {
            key,
            id: n.id,
            banner: a.banner,
            chime: (ours && !silent).then_some(tone),
            buzz: ours && vibrate,
        });
        Ok(())
    }

    /// Badge counts the user allows (an app's Badge switch and Allow).
    pub(super) fn badge_allowed(&self, app: &str) -> bool {
        let p = self.app_policy(app);
        p.flags & policy::flag::ALLOW != 0 && p.flags & policy::flag::BADGE != 0
    }

    /// (TP4) The policy owners whose badge is off (the renderer drops MODEMD's fallback counts for them too).
    pub fn badges_off(&self) -> Vec<&'static str> {
        OWNERS
            .iter()
            .copied()
            .filter(|k| !self.badge_allowed(k))
            .collect()
    }

    /// Apps with cards that show their glyph in the status bar, newest first (Default and Urgent channels).
    pub fn glyph_keys(&self) -> Vec<&'static str> {
        self.notifications
            .iter()
            .filter(|(k, n)| Self::shell_card(k, n) || self.effective(k, n.channel).glyph)
            .map(|(k, _)| *k)
            .collect()
    }

    /// The cards that fold under Silent (a Min channel): (app, id).
    pub fn silent_cards(&self) -> Vec<(&'static str, u32)> {
        self.notifications
            .iter()
            .filter(|(k, n)| {
                !Self::shell_card(k, n)
                    && self.effective(k, n.channel).importance == policy::importance::MIN
            })
            .map(|(k, n)| (*k, n.id))
            .collect()
    }

    /// Every loop: Do not disturb ends at its time, and a changed `n.*` / `notify.*` takes effect: the cards and
    /// badges of what is off go (their running apps hear `NotifyGone(id, GONE_OFF)`), and INFO 7 moves on.
    pub fn policy_frame(&mut self, now_ms: u64) {
        let dnd = self.settings.get(keys::NOTIFY_DND).unwrap_or(false);
        if dnd && !self.dnd() {
            dprintln!("neoui: do not disturb ended");
            for (k, v) in [
                (keys::NOTIFY_DND.name(), Value::Bool(false)),
                (keys::NOTIFY_DNDTO.name(), Value::Bytes(Vec::new())),
            ] {
                let _ = settings::set(&mut self.settings, Caller::Shell, k, v, now_ms);
            }
        }
        let seq = self.settings.store().seq();
        if seq == self.policies.seq {
            return;
        }
        let ours = |k: &str| k.starts_with("n.") || k.starts_with("notify.");
        let touched = self
            .settings
            .store()
            .changes_since(self.policies.seq)
            .is_none_or(|mut keys| keys.any(ours));
        self.policies.seq = seq;
        if !touched {
            return;
        }
        let gone: Vec<(&'static str, u32)> = self
            .notifications
            .iter()
            .filter(|(k, n)| !Self::shell_card(k, n) && !self.effective(k, n.channel).accepts())
            .map(|(k, n)| (*k, n.id))
            .collect();
        if !gone.is_empty() {
            self.notifications
                .retain(|(k, n)| !gone.contains(&(*k, n.id)));
            dprintln!("neoui: {} cards turned off", gone.len());
            for (k, id) in gone {
                self.post_gone(k, id, proto::GONE_OFF);
            }
        }
        self.notes_changed = true;
        self.badges_changed = true;
        self.info_changed(proto::INFO_CHANNELS);
    }

    /// INFO 7: every policy owner's app record and channel records.
    pub(super) fn channel_records(&self) -> Vec<Vec<u8>> {
        let global = self.lock_global();
        let mut out = Vec::new();
        for owner in OWNERS {
            if apps::find(owner).is_none() {
                continue;
            }
            let declared: Vec<Channel> = self
                .policies
                .catalogue
                .iter()
                .filter(|(k, _)| k == owner)
                .map(|(_, c)| c.clone())
                .collect();
            let p = self.app_policy(owner);
            out.extend(
                policy::records(owner, &p, &declared, global)
                    .iter()
                    .map(policy::Record::encode),
            );
        }
        out
    }
}

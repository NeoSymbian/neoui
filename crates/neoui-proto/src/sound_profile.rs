// SPDX-License-Identifier: Apache-2.0
//! Sound profiles (docs neoui-shell/settings.md "B. Sound profiles", S05, SE2a): General, Silent, Meeting, Outdoor,
//! Offline and the ones the user adds. One reducer: the shell runs it for the Settings app (`ACTION_PROFILE`), the
//! shade's Silent tile and the power menu, and commits its result in one synchronous call
//! ([`store::commit`]): `sound.profile`, `sound.profiles`, the live mirrors MODEMD reads (`sound.silent`,
//! `sound.vibrate`, `sound.ring_volume`) and, entering or leaving Offline, the radios.
//!
//! - Silent (and any profile without a ring) remembers the last audible profile; turning silent off goes back to it.
//! - Offline is airplane mode: entering it remembers which radios were on and turns them off; leaving it (another
//!   profile, or airplane mode turned off anywhere) restores them and the profile it was entered from. Its sound is
//!   General's.
//! - A write to a mirror by anyone else is folded in ([`Profiles::reconcile`]): silent on chooses Silent, off goes
//!   back, a volume or vibrate change edits the active profile.
//!
//! The built-ins' values are the owner's call (settings.md SE2 plan): defaults until answered, each editable.

use alloc::string::String;
use alloc::vec::Vec;

pub const GENERAL: u8 = 0;
pub const SILENT: u8 = 1;
pub const MEETING: u8 = 2;
pub const OUTDOOR: u8 = 3;
pub const OFFLINE: u8 = 4;
/// The first id of a profile the user adds.
pub const FIRST_ADDED: u8 = 5;
/// Most profiles, built-ins included.
pub const MAX_PROFILES: usize = 16;
/// The longest name of an added profile, in bytes.
pub const NAME_MAX: usize = 24;
/// The longest `sound.profiles` blob.
pub const BLOB_MAX: usize = 1024;
const VERSION: u8 = 1;

/// One profile: what an incoming call does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub id: u8,
    pub name: String,
    /// The ringtone sounds (off: silent).
    pub ring: bool,
    /// 0..=10
    pub volume: u8,
    pub vibrate: bool,
}

/// The settings MODEMD reads: the active profile's live values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mirrors {
    pub silent: bool,
    pub vibrate: bool,
    pub volume: u8,
}

/// The radios Offline turns off and gives back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Radios {
    pub airplane: bool,
    pub wifi: bool,
    pub bt: bool,
}

impl Radios {
    const OFF: Radios = Radios {
        airplane: true,
        wifi: false,
        bt: false,
    };

    fn bits(self) -> u8 {
        u8::from(self.airplane) | u8::from(self.wifi) << 1 | u8::from(self.bt) << 2
    }

    fn from_bits(b: u8) -> Self {
        Radios {
            airplane: b & 1 != 0,
            wifi: b & 2 != 0,
            bt: b & 4 != 0,
        }
    }
}

/// What the user asks for (`ipc::ACTION_PROFILE`'s request, [`Command::encode`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Choose(u8),
    /// A profile's sound (not Offline's: it uses General's).
    Set {
        id: u8,
        ring: bool,
        volume: u8,
        vibrate: bool,
    },
    Add(String),
    Rename(u8, String),
    Delete(u8),
}

/// Why a command was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// No such profile, or a built-in where only added ones may be changed (rename, delete), or Offline's sound.
    NotFound,
    /// A volume over 10, an empty or too long name.
    Argument,
    /// No room for another profile.
    Full,
}

/// All the profiles and which is active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profiles {
    pub active: u8,
    /// The profile silent off goes back to (always one with a ring).
    pub last_audible: u8,
    /// The profile leaving Offline goes back to.
    pub before_offline: u8,
    /// The radios as they were when Offline was entered.
    pub radios: Radios,
    pub list: Vec<Profile>,
}

fn builtin(id: u8, name: &str, ring: bool, volume: u8, vibrate: bool) -> Profile {
    Profile {
        id,
        name: String::from(name),
        ring,
        volume,
        vibrate,
    }
}

/// The built-ins as they come (owner's call: defaults until answered). Offline's sound fields are unused.
#[must_use]
pub fn builtins() -> Vec<Profile> {
    alloc::vec![
        builtin(GENERAL, "General", true, 8, true),
        builtin(SILENT, "Silent", false, 8, true),
        builtin(MEETING, "Meeting", true, 2, true),
        builtin(OUTDOOR, "Outdoor", true, 10, true),
        builtin(OFFLINE, "Offline", true, 8, true),
    ]
}

impl Profiles {
    /// Today's three settings, as profiles that sound the same: General takes the volume and vibrate; silent on makes
    /// Silent active, with General the profile it goes back to.
    #[must_use]
    pub fn upgrade(m: Mirrors) -> Self {
        let mut list = builtins();
        list[GENERAL as usize].volume = m.volume.min(10);
        list[GENERAL as usize].vibrate = m.vibrate;
        list[SILENT as usize].vibrate = m.vibrate;
        Profiles {
            active: if m.silent { SILENT } else { GENERAL },
            last_audible: GENERAL,
            before_offline: GENERAL,
            radios: Radios::default(),
            list,
        }
    }

    #[must_use]
    pub fn get(&self, id: u8) -> Option<&Profile> {
        self.list.iter().find(|p| p.id == id)
    }

    fn get_mut(&mut self, id: u8) -> Option<&mut Profile> {
        self.list.iter_mut().find(|p| p.id == id)
    }

    fn audible(&self, id: u8) -> bool {
        id != OFFLINE && self.get(id).is_some_and(|p| p.ring)
    }

    /// The live values MODEMD rings with. A profile without a ring keeps the last audible one's volume (so silent on
    /// and off changes nothing else); Offline sounds like General.
    #[must_use]
    pub fn mirrors(&self) -> Mirrors {
        let id = if self.active == OFFLINE {
            GENERAL
        } else {
            self.active
        };
        let p = self.get(id).or_else(|| self.get(GENERAL));
        let ring = p.is_some_and(|p| p.ring);
        let volume_from = if ring { id } else { self.last_audible };
        Mirrors {
            silent: !ring,
            vibrate: p.is_some_and(|p| p.vibrate),
            volume: self.get(volume_from).map_or(8, |p| p.volume),
        }
    }

    /// Run a command. `radios` is how the radios are now (Offline remembers them). Returns the radios to set, when
    /// entering or leaving Offline.
    ///
    /// # Errors
    /// [`Refused`] when the command doesn't apply; nothing changes then.
    pub fn apply(&mut self, cmd: Command, radios: Radios) -> Result<Option<Radios>, Refused> {
        match cmd {
            Command::Choose(id) => {
                if self.get(id).is_none() {
                    return Err(Refused::NotFound);
                }
                Ok(self.choose(id, radios))
            }
            Command::Set {
                id,
                ring,
                volume,
                vibrate,
            } => {
                if volume > 10 {
                    return Err(Refused::Argument);
                }
                if id == OFFLINE {
                    return Err(Refused::NotFound);
                }
                let p = self.get_mut(id).ok_or(Refused::NotFound)?;
                // Silent never rings: muting chooses it, so a ringing Silent would undo the mute
                p.ring = ring && id != SILENT;
                p.volume = volume;
                p.vibrate = vibrate;
                self.repair();
                Ok(None)
            }
            Command::Add(name) => {
                check_name(&name)?;
                if self.list.len() >= MAX_PROFILES {
                    return Err(Refused::Full);
                }
                let id = (FIRST_ADDED..=u8::MAX)
                    .find(|i| self.get(*i).is_none())
                    .ok_or(Refused::Full)?;
                let g = self
                    .get(GENERAL)
                    .cloned()
                    .unwrap_or_else(|| builtins().remove(0));
                self.list.push(Profile { id, name, ..g });
                Ok(None)
            }
            Command::Rename(id, name) => {
                check_name(&name)?;
                if id < FIRST_ADDED {
                    return Err(Refused::NotFound);
                }
                self.get_mut(id).ok_or(Refused::NotFound)?.name = name;
                Ok(None)
            }
            Command::Delete(id) => {
                if id < FIRST_ADDED || self.get(id).is_none() {
                    return Err(Refused::NotFound);
                }
                self.list.retain(|p| p.id != id);
                if self.active == id {
                    self.active = GENERAL;
                }
                self.repair();
                Ok(None)
            }
        }
    }

    fn choose(&mut self, id: u8, radios: Radios) -> Option<Radios> {
        if id == self.active {
            return None;
        }
        let mut set = None;
        if self.active == OFFLINE {
            set = Some(self.radios); // leaving Offline: the radios as they were
        }
        if id == OFFLINE {
            self.radios = radios;
            self.before_offline = self.active;
            set = Some(Radios::OFF);
        }
        if self.audible(self.active) {
            self.last_audible = self.active;
        }
        self.active = id;
        if self.audible(id) {
            self.last_audible = id;
        }
        set
    }

    /// The references after a profile changed or went: each names a profile that exists, and the last audible one
    /// rings.
    fn repair(&mut self) {
        if self.get(self.active).is_none() {
            self.active = GENERAL;
        }
        if self.get(self.before_offline).is_none() || self.before_offline == OFFLINE {
            self.before_offline = GENERAL;
        }
        if !self.audible(self.last_audible) {
            // the first profile that rings (General when none does: unmuting can't do better then)
            self.last_audible = self
                .list
                .iter()
                .map(|p| p.id)
                .find(|id| self.audible(*id))
                .unwrap_or(GENERAL);
        }
    }

    /// Fold in a write to the mirrors or to airplane mode made by anyone but the reducer. Returns the radios to set
    /// when it means leaving Offline, `Some(None)` when the profiles changed otherwise, `None` when nothing did.
    pub fn reconcile(&mut self, now: Mirrors, airplane: bool) -> Option<Option<Radios>> {
        // first a volume or vibrate change, into the profile that supplies it now: a silent flip or Offline ending in
        // the same frame must not drop it
        let expected = self.mirrors();
        let edited = now.volume != expected.volume || now.vibrate != expected.vibrate;
        if edited {
            let sound = if self.active == OFFLINE {
                GENERAL
            } else {
                self.active
            };
            let volume_from = if self.audible(sound) {
                sound
            } else {
                self.last_audible
            };
            if let Some(p) = self.get_mut(volume_from) {
                p.volume = now.volume.min(10);
            }
            if let Some(p) = self.get_mut(sound) {
                p.vibrate = now.vibrate;
            }
        }
        if self.active == OFFLINE && !airplane {
            // airplane mode turned off elsewhere: Offline ends, the other radios come back; airplane stays off
            let back = self.before_offline;
            let mut set = self.choose(back, Radios::default());
            if let Some(r) = &mut set {
                r.airplane = false;
            }
            return Some(set);
        }
        if now.silent != self.mirrors().silent {
            let to = if now.silent {
                SILENT
            } else {
                self.last_audible
            };
            return Some(self.choose(to, Radios::default()));
        }
        edited.then_some(None)
    }

    /// `sound.profiles`: the built-ins' overrides and the added profiles, with what Silent and Offline remember.
    /// `[version][last_audible][before_offline][radios][n]` then n × `[id][flags: ring 1, vibrate 2][volume][len]
    /// [name]` (a built-in's name is not stored).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let defaults = builtins();
        let mut records = Vec::new();
        for p in &self.list {
            let name = if p.id < FIRST_ADDED {
                let d = &defaults[usize::from(p.id)];
                if (d.ring, d.volume, d.vibrate) == (p.ring, p.volume, p.vibrate) {
                    continue; // as it comes: not stored, so a new default reaches it
                }
                ""
            } else {
                p.name.as_str()
            };
            records.push((p, name));
        }
        let mut b = alloc::vec![
            VERSION,
            self.last_audible,
            self.before_offline,
            self.radios.bits(),
            records.len() as u8
        ];
        for (p, name) in records {
            b.extend_from_slice(&[
                p.id,
                u8::from(p.ring) | u8::from(p.vibrate) << 1,
                p.volume,
                name.len() as u8,
            ]);
            b.extend_from_slice(name.as_bytes());
        }
        b
    }

    /// The profiles from `sound.profile` and `sound.profiles`; `None` if the blob is damaged (the caller upgrades
    /// from the mirrors instead). References to missing profiles fall back to General.
    #[must_use]
    pub fn decode(active: u8, b: &[u8]) -> Option<Self> {
        let (&[version, last_audible, before_offline, radios, n], mut rest) =
            b.split_first_chunk::<5>()?;
        if version != VERSION || b.len() > BLOB_MAX || usize::from(n) > MAX_PROFILES {
            return None;
        }
        let mut list = builtins();
        for _ in 0..n {
            let (&[id, flags, volume, len], r) = rest.split_first_chunk::<4>()?;
            let len = usize::from(len);
            let name = core::str::from_utf8(r.get(..len)?).ok()?;
            rest = &r[len..];
            if volume > 10 || len > NAME_MAX || id == u8::MAX {
                return None;
            }
            let (ring, vibrate) = (flags & 1 != 0, flags & 2 != 0);
            if id < FIRST_ADDED {
                let p = &mut list[usize::from(id)];
                (p.ring, p.volume, p.vibrate) = (ring, volume, vibrate);
            } else {
                if name.trim().is_empty() || list.iter().any(|p| p.id == id) {
                    return None;
                }
                list.push(Profile {
                    id,
                    name: String::from(name),
                    ring,
                    volume,
                    vibrate,
                });
            }
        }
        if !rest.is_empty() || list.len() > MAX_PROFILES {
            return None;
        }
        let mut p = Profiles {
            active,
            last_audible,
            before_offline,
            radios: Radios::from_bits(radios),
            list,
        };
        p.repair();
        Some(p)
    }
}

/// The profiles from the store's three inputs (`sound.profile`, `sound.profiles`, today's mirrors): the blob, or the
/// upgrade when there is none yet or it is damaged. The shell and the Settings app read them the same way.
#[must_use]
pub fn read(active: i32, blob: &[u8], mirrors: Mirrors) -> Profiles {
    let active = active.clamp(0, 254) as u8;
    if blob.is_empty() {
        return Profiles::upgrade(mirrors);
    }
    Profiles::decode(active, blob).unwrap_or_else(|| Profiles::upgrade(mirrors))
}

impl Profile {
    /// What a profile does, as its row says: "Built-in ringtone · volume 7 · vibrate", "No ringing · vibrate",
    /// "Airplane mode: radios off".
    #[must_use]
    pub fn summary(&self) -> String {
        if self.id == OFFLINE {
            return String::from("Airplane mode: radios off");
        }
        let vibrate = if self.vibrate {
            "vibrate"
        } else {
            "no vibration"
        };
        if self.ring {
            alloc::format!("Built-in ringtone · volume {} · {vibrate}", self.volume)
        } else {
            alloc::format!("No ringing · {vibrate}")
        }
    }
}

fn check_name(name: &str) -> Result<(), Refused> {
    if name.trim().is_empty() || name.len() > NAME_MAX {
        return Err(Refused::Argument);
    }
    Ok(())
}

impl Command {
    /// `[op][id][flags][volume][name…]`: 0 choose, 1 set, 2 add, 3 rename, 4 delete.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (op, id, flags, volume, name) = match self {
            Command::Choose(id) => (0, *id, 0, 0, ""),
            Command::Set {
                id,
                ring,
                volume,
                vibrate,
            } => (
                1,
                *id,
                u8::from(*ring) | u8::from(*vibrate) << 1,
                *volume,
                "",
            ),
            Command::Add(name) => (2, 0, 0, 0, name.as_str()),
            Command::Rename(id, name) => (3, *id, 0, 0, name.as_str()),
            Command::Delete(id) => (4, *id, 0, 0, ""),
        };
        let mut b = alloc::vec![op, id, flags, volume];
        b.extend_from_slice(name.as_bytes());
        b
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let (&[op, id, flags, volume], name) = b.split_first_chunk::<4>()?;
        if name.len() > NAME_MAX {
            return None;
        }
        let name = String::from(core::str::from_utf8(name).ok()?);
        let plain = name.is_empty();
        Some(match op {
            0 if plain => Command::Choose(id),
            1 if plain => Command::Set {
                id,
                ring: flags & 1 != 0,
                volume,
                vibrate: flags & 2 != 0,
            },
            2 => Command::Add(name),
            3 => Command::Rename(id, name),
            4 if plain => Command::Delete(id),
            _ => return None,
        })
    }
}

/// The profiles in the settings store: load, run, commit in one synchronous call.
pub mod store {
    use super::{Command, Mirrors, Profiles, Radios, Refused};
    use crate::settings::service::{Backend, Service};
    use crate::settings::{Bytes, Caller, Error, keys};

    fn mirrors<B: Backend>(s: &Service<B>) -> Mirrors {
        Mirrors {
            silent: s.get(keys::SOUND_SILENT).unwrap_or(false),
            vibrate: s.get(keys::SOUND_VIBRATE).unwrap_or(true),
            volume: s.get(keys::SOUND_RING_VOLUME).unwrap_or(8).clamp(0, 10) as u8,
        }
    }

    fn radios<B: Backend>(s: &Service<B>) -> Radios {
        Radios {
            airplane: s.get(keys::RADIO_AIRPLANE).unwrap_or(false),
            wifi: s.get(keys::RADIO_WIFI).unwrap_or(true),
            bt: s.get(keys::RADIO_BT).unwrap_or(false),
        }
    }

    /// The stored profiles, or today's mirrors as profiles (the upgrade, or a damaged blob).
    #[must_use]
    pub fn load<B: Backend>(s: &Service<B>) -> Profiles {
        let blob = s.get(keys::SOUND_PROFILES).map(|b| b.0).unwrap_or_default();
        super::read(s.get(keys::SOUND_PROFILE).unwrap_or(0), &blob, mirrors(s))
    }

    /// Write the profiles, their mirrors and (Offline) the radios, all before any watcher or MODEMD hears of it: the
    /// store notifies and saves later, from the main loop.
    ///
    /// # Errors
    /// The store's, for the first write it refused (the ones before it stay: the next [`reconcile`] repairs).
    pub fn commit<B: Backend>(
        s: &mut Service<B>,
        p: &Profiles,
        set: Option<Radios>,
        now_ms: u64,
    ) -> Result<(), Error> {
        let m = p.mirrors();
        s.set(
            Caller::Shell,
            keys::SOUND_PROFILES,
            Bytes(p.encode()),
            now_ms,
        )
        .map(drop)?;
        s.set(
            Caller::Shell,
            keys::SOUND_PROFILE,
            i32::from(p.active),
            now_ms,
        )
        .map(drop)?;
        s.set(Caller::Shell, keys::SOUND_SILENT, m.silent, now_ms)
            .map(drop)?;
        s.set(Caller::Shell, keys::SOUND_VIBRATE, m.vibrate, now_ms)
            .map(drop)?;
        s.set(
            Caller::Shell,
            keys::SOUND_RING_VOLUME,
            i32::from(m.volume),
            now_ms,
        )
        .map(drop)?;
        if let Some(r) = set {
            s.set(Caller::Shell, keys::RADIO_AIRPLANE, r.airplane, now_ms)
                .map(drop)?;
            s.set(Caller::Shell, keys::RADIO_WIFI, r.wifi, now_ms)
                .map(drop)?;
            s.set(Caller::Shell, keys::RADIO_BT, r.bt, now_ms)
                .map(drop)?;
        }
        Ok(())
    }

    /// Run a command from the Settings app, the shade or the power menu.
    ///
    /// # Errors
    /// [`Refused`] (nothing written), or the store refused a write.
    pub fn apply<B: Backend>(
        s: &mut Service<B>,
        cmd: Command,
        now_ms: u64,
    ) -> Result<(), ApplyError> {
        // a write the reconciler hasn't folded in yet (this frame) goes in first, or the commit would undo it
        reconcile(s, now_ms);
        let mut p = load(s);
        let set = p.apply(cmd, radios(s)).map_err(ApplyError::Refused)?;
        commit(s, &p, set, now_ms).map_err(ApplyError::Store)
    }

    /// Once per frame, after the store took other writes: fold changed mirrors or airplane mode into the profiles.
    /// True when it wrote something.
    pub fn reconcile<B: Backend>(s: &mut Service<B>, now_ms: u64) -> bool {
        let mut p = load(s);
        let stored = s.get(keys::SOUND_PROFILES).map(|b| b.0).unwrap_or_default();
        // the upgrade's first write, or a damaged blob read as the upgrade: write what is in force now, so
        // `sound.profile` agrees with it
        let active = s.get(keys::SOUND_PROFILE).unwrap_or(0).clamp(0, 254) as u8;
        let fresh = stored.is_empty() || Profiles::decode(active, &stored).is_none();
        match p.reconcile(mirrors(s), radios(s).airplane) {
            Some(set) => commit(s, &p, set, now_ms).is_ok(),
            // the upgrade's first write: the profiles go into the store as they were found
            None if fresh => commit(s, &p, None, now_ms).is_ok(),
            None => false,
        }
    }

    /// Why [`apply`] did nothing (or not everything).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ApplyError {
        Refused(Refused),
        Store(Error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const ON: Radios = Radios {
        airplane: false,
        wifi: true,
        bt: true,
    };

    fn today(silent: bool) -> Profiles {
        Profiles::upgrade(Mirrors {
            silent,
            vibrate: false,
            volume: 6,
        })
    }

    #[test]
    fn the_upgrade_sounds_the_same() {
        for silent in [false, true] {
            let p = today(silent);
            assert_eq!(
                p.mirrors(),
                Mirrors {
                    silent,
                    vibrate: false,
                    volume: 6
                }
            );
            assert_eq!(p.active, if silent { SILENT } else { GENERAL });
            assert_eq!(p.last_audible, GENERAL);
        }
    }

    #[test]
    fn silent_remembers_and_restores() {
        let mut p = today(false);
        p.apply(Command::Choose(MEETING), ON).unwrap();
        assert_eq!(p.mirrors().volume, 2);
        p.apply(Command::Choose(SILENT), ON).unwrap();
        assert_eq!(
            p.mirrors(),
            Mirrors {
                silent: true,
                vibrate: false,
                volume: 2
            }
        );
        // the shade's tile off: back to Meeting
        let mut off = p.mirrors();
        off.silent = false;
        assert_eq!(p.reconcile(off, false), Some(None));
        assert_eq!(p.active, MEETING);
        // the shade's tile on from Meeting: Silent
        let mut on = p.mirrors();
        on.silent = true;
        p.reconcile(on, false);
        assert_eq!((p.active, p.last_audible), (SILENT, MEETING));
    }

    #[test]
    fn offline_turns_the_radios_off_and_gives_them_back() {
        let mut p = today(false);
        p.apply(Command::Choose(OUTDOOR), ON).unwrap();
        let set = p.apply(Command::Choose(OFFLINE), ON).unwrap();
        assert_eq!(set, Some(Radios::OFF));
        assert_eq!(p.mirrors(), today(false).mirrors()); // General's sound
        // a reboot while Offline: the blob keeps the radios and the profile to go back to
        let mut p = Profiles::decode(OFFLINE, &p.encode()).unwrap();
        assert_eq!(
            p.apply(Command::Choose(MEETING), Radios::OFF).unwrap(),
            Some(ON)
        );
        assert_eq!(p.active, MEETING);
        // in again, then airplane off from the shade: back to where it was, radios but airplane restored
        p.apply(Command::Choose(OFFLINE), ON).unwrap();
        let back = p.reconcile(p.mirrors(), false).unwrap();
        assert_eq!(back, Some(ON));
        assert_eq!(p.active, MEETING);
    }

    #[test]
    fn edits_reach_the_profile_that_rings() {
        let mut p = today(false);
        // Louder on Sounds while General is active
        let mut m = p.mirrors();
        m.volume = 9;
        p.reconcile(m, false);
        assert_eq!(p.get(GENERAL).unwrap().volume, 9);
        // while Silent, the volume belongs to the profile it goes back to
        p.apply(Command::Choose(SILENT), ON).unwrap();
        let mut m = p.mirrors();
        m.volume = 3;
        p.reconcile(m, false);
        assert_eq!(p.get(GENERAL).unwrap().volume, 3);
        assert_eq!(p.reconcile(p.mirrors(), false), None);
    }

    #[test]
    fn added_profiles_and_their_limits() {
        let mut p = today(false);
        p.apply(Command::Add("Gym".into()), ON).unwrap();
        let gym = p.list.last().unwrap().id;
        assert_eq!(gym, FIRST_ADDED);
        p.apply(Command::Rename(gym, "Gym, loud".into()), ON)
            .unwrap();
        p.apply(
            Command::Set {
                id: gym,
                ring: true,
                volume: 10,
                vibrate: false,
            },
            ON,
        )
        .unwrap();
        p.apply(Command::Choose(gym), ON).unwrap();
        assert_eq!(p.mirrors().volume, 10);
        // deleting the active one falls back to General
        p.apply(Command::Delete(gym), ON).unwrap();
        assert_eq!((p.active, p.last_audible), (GENERAL, GENERAL));
        // refusals change nothing
        let before = p.clone();
        assert_eq!(
            p.apply(Command::Delete(MEETING), ON),
            Err(Refused::NotFound)
        );
        assert_eq!(
            p.apply(Command::Rename(SILENT, "x".into()), ON),
            Err(Refused::NotFound)
        );
        assert_eq!(
            p.apply(Command::Add(" ".into()), ON),
            Err(Refused::Argument)
        );
        assert_eq!(
            p.apply(Command::Add("x".repeat(25)), ON),
            Err(Refused::Argument)
        );
        assert_eq!(
            p.apply(
                Command::Set {
                    id: OFFLINE,
                    ring: true,
                    volume: 1,
                    vibrate: true
                },
                ON
            ),
            Err(Refused::NotFound)
        );
        assert_eq!(p.apply(Command::Choose(99), ON), Err(Refused::NotFound));
        assert_eq!(p, before);
        for i in 0..11 {
            p.apply(Command::Add(alloc::format!("p{i}")), ON).unwrap();
        }
        assert_eq!(
            p.apply(Command::Add("one more".into()), ON),
            Err(Refused::Full)
        );
    }

    #[test]
    fn writes_in_one_frame_are_all_kept() {
        // silent on and a new volume together: the volume reaches General, then Silent
        let mut p = today(false);
        let mut m = p.mirrors();
        m.silent = true;
        m.volume = 3;
        p.reconcile(m, false);
        assert_eq!((p.active, p.get(GENERAL).unwrap().volume), (SILENT, 3));
        assert_eq!(p.mirrors().volume, 3);
        // Offline ending (airplane off) with a vibrate change in the same frame: both kept
        let mut p = today(false);
        p.apply(Command::Choose(OFFLINE), ON).unwrap();
        let mut m = p.mirrors();
        m.vibrate = true;
        assert_eq!(p.reconcile(m, false), Some(Some(ON)));
        assert_eq!(p.active, GENERAL);
        assert!(p.get(GENERAL).unwrap().vibrate);
    }

    #[test]
    fn mute_and_unmute_do_what_was_asked() {
        // Silent never rings, so muting never writes silent off
        let mut p = today(false);
        p.apply(
            Command::Set {
                id: SILENT,
                ring: true,
                volume: 5,
                vibrate: true,
            },
            ON,
        )
        .unwrap();
        assert!(!p.get(SILENT).unwrap().ring);
        // General stops ringing while it is the one unmute goes back to: unmute goes to one that rings
        p.apply(
            Command::Set {
                id: GENERAL,
                ring: false,
                volume: 6,
                vibrate: false,
            },
            ON,
        )
        .unwrap();
        assert_eq!(p.last_audible, MEETING);
        p.apply(Command::Choose(SILENT), ON).unwrap();
        let mut m = p.mirrors();
        m.silent = false;
        p.reconcile(m, false);
        assert_eq!(p.active, MEETING);
        assert!(!p.mirrors().silent);
    }

    #[test]
    fn the_blob_round_trips_within_its_limit() {
        let mut p = today(true);
        for i in 0..11 {
            p.apply(Command::Add("x".repeat(NAME_MAX - i)), ON).unwrap();
        }
        p.apply(Command::Choose(OFFLINE), ON).unwrap();
        let b = p.encode();
        assert!(b.len() <= BLOB_MAX, "{}", b.len());
        assert_eq!(Profiles::decode(p.active, &b), Some(p.clone()));
        // built-ins as they come aren't stored
        assert_eq!(
            Profiles::upgrade(Mirrors {
                silent: false,
                vibrate: true,
                volume: 8
            })
            .encode(),
            vec![1, 0, 0, 0, 0]
        );
        // damaged
        assert_eq!(Profiles::decode(0, &b[..b.len() - 1]), None);
        assert_eq!(Profiles::decode(0, &[2, 0, 0, 0, 0]), None);
        assert_eq!(Profiles::decode(0, &[1, 0, 0, 0, 1, 2, 1, 11, 0]), None);
        assert_eq!(Profiles::decode(0, &[1, 0, 0, 0, 1, 5, 1, 1, 0]), None); // an added one without a name
        // a missing active profile falls back to General
        assert_eq!(
            Profiles::decode(42, &[1, 0, 0, 0, 0]).unwrap().active,
            GENERAL
        );
    }

    #[test]
    fn commands_round_trip() {
        for c in [
            Command::Choose(3),
            Command::Set {
                id: 5,
                ring: false,
                volume: 7,
                vibrate: true,
            },
            Command::Add("Gym".into()),
            Command::Rename(6, "Car".into()),
            Command::Delete(7),
        ] {
            assert_eq!(Command::decode(&c.encode()), Some(c));
        }
        assert_eq!(Command::decode(&[0, 1, 0]), None);
        assert_eq!(Command::decode(&[9, 0, 0, 0]), None);
        assert_eq!(Command::decode(&[0, 1, 0, 0, b'x']), None);
    }

    mod in_the_store {
        use super::super::store;
        use super::super::*;
        use crate::settings::service::{Backend, Service};
        use crate::settings::{Caller, keys};

        #[derive(Default)]
        struct Mem(Option<alloc::vec::Vec<u8>>);
        impl Backend for Mem {
            fn load(&mut self) -> Result<Option<alloc::vec::Vec<u8>>, i32> {
                Ok(self.0.clone())
            }
            fn save(&mut self, bytes: &[u8]) -> Result<(), i32> {
                self.0 = Some(bytes.to_vec());
                Ok(())
            }
        }

        fn stored(s: &Service<Mem>) -> Mirrors {
            Mirrors {
                silent: s.get(keys::SOUND_SILENT).unwrap(),
                vibrate: s.get(keys::SOUND_VIBRATE).unwrap(),
                volume: s.get(keys::SOUND_RING_VOLUME).unwrap() as u8,
            }
        }

        #[test]
        fn a_command_keeps_a_write_made_earlier_in_the_frame() {
            let (mut s, _) = Service::open(Mem::default());
            assert!(store::reconcile(&mut s, 0));
            // an app turns the ring volume down, then (same frame, before reconcile) a profile is added
            s.set(Caller::Shell, keys::SOUND_RING_VOLUME, 3, 1)
                .map(drop)
                .unwrap();
            store::apply(&mut s, Command::Add("Car".into()), 1).unwrap();
            assert_eq!(s.get(keys::SOUND_RING_VOLUME).unwrap(), 3);
            assert_eq!(store::load(&s).get(GENERAL).unwrap().volume, 3);
        }

        #[test]
        fn a_damaged_blob_is_rewritten_as_what_is_in_force() {
            let (mut s, _) = Service::open(Mem::default());
            s.set(Caller::Shell, keys::SOUND_PROFILE, i32::from(OUTDOOR), 0)
                .map(drop)
                .unwrap();
            s.set(
                Caller::Shell,
                keys::SOUND_PROFILES,
                crate::settings::Bytes(alloc::vec![9, 9]),
                0,
            )
            .map(drop)
            .unwrap();
            assert!(store::reconcile(&mut s, 1));
            assert_eq!(s.get(keys::SOUND_PROFILE).unwrap(), i32::from(GENERAL));
            assert!(!store::reconcile(&mut s, 2));
        }

        #[test]
        fn a_watcher_sees_only_the_final_state() {
            let (mut s, _) = Service::open(Mem::default());
            s.set(Caller::Shell, keys::SOUND_RING_VOLUME, 6, 0)
                .map(drop)
                .unwrap();
            assert!(store::reconcile(&mut s, 0)); // the upgrade's first write
            s.watch(1, "sound.").unwrap();
            s.watch(2, "radio.").unwrap();
            let seq = s.store().seq();
            store::apply(&mut s, Command::Choose(MEETING), 1).unwrap();
            // one batch of notifications, taken after the whole commit; every mirror already final
            let (sessions, _) = s.take_notifications();
            assert_eq!(sessions, alloc::vec![1]);
            assert_eq!(stored(&s), store::load(&s).mirrors());
            assert_eq!(stored(&s).volume, 2);
            let changed: alloc::vec::Vec<&str> = s.store().changes_since(seq).unwrap().collect();
            for k in ["sound.profile", "sound.profiles", "sound.ring_volume"] {
                assert!(changed.contains(&k), "{k}");
            }
            // Offline: the radios change in the same call
            store::apply(&mut s, Command::Choose(OFFLINE), 2).unwrap();
            assert!(s.get(keys::RADIO_AIRPLANE).unwrap());
            assert!(!s.get(keys::RADIO_WIFI).unwrap());
            // airplane off from elsewhere: Offline ends, Wi-Fi comes back, and Meeting (where it was entered from) again
            s.set(Caller::Shell, keys::RADIO_AIRPLANE, false, 3)
                .map(drop)
                .unwrap();
            assert!(store::reconcile(&mut s, 3));
            assert_eq!(s.get(keys::SOUND_PROFILE).unwrap(), i32::from(MEETING));
            assert!(s.get(keys::RADIO_WIFI).unwrap());
            assert!(!store::reconcile(&mut s, 4));
        }
    }
}

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
/// The longest `sound.tones` record: an id and a tones byte per profile.
pub const TONES_MAX: usize = 2 * MAX_PROFILES;
/// (D6) The longest `sound.feedback` record: a version, a count, an id and flags per profile.
pub const FEEDBACK_MAX: usize = 2 + 2 * MAX_PROFILES;
const FEEDBACK_VERSION: u8 = 1;

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
    /// (SE2b) the call's ringtone and a message's tone ([`tone`])
    pub ringtone: u8,
    pub message_tone: u8,
    /// (D6) the dial keys sound (out of a call; in-call DTMF doesn't depend on it)
    pub keypad: bool,
    /// (D6) a short pulse for each control a touch works (not the system keyboard: `keyboard.vibrate` is its own)
    pub touch: bool,
}

/// (SE2b) The tones, NeoAudio's `tones` and modem-ipc's `tone` ids (the shell passes them through unchanged).
pub mod tone {
    pub const BUILT_IN: u8 = 0;
    pub const BEEP: u8 = 1;
    pub const TWO_BEEPS: u8 = 2;
    /// no sound: a call vibrates, a message buzzes
    pub const NONE: u8 = 3;
    pub const ALL: [u8; 4] = [BUILT_IN, BEEP, TWO_BEEPS, NONE];
    /// (D6) the user's own sounds (modem-ipc 2.8, NeoAudio 1.1): the first and the last id
    pub const FIRST_YOURS: u8 = 4;
    pub const LAST: u8 = 15;

    /// A ringtone's name.
    #[must_use]
    pub fn ring_name(t: u8) -> &'static str {
        match t {
            BUILT_IN => "Built-in ringtone",
            BEEP => "Beep once",
            TWO_BEEPS => "Two beeps",
            NONE => "None",
            _ => "Your sound",
        }
    }

    /// A message tone's name.
    #[must_use]
    pub fn message_name(t: u8) -> &'static str {
        match t {
            BUILT_IN => "Built-in chime",
            BEEP => "Beep",
            TWO_BEEPS => "Two beeps",
            NONE => "None (buzz only)",
            _ => "Your sound",
        }
    }
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
    /// (SE2b) A profile's ringtone and message tone (not Offline's: it uses General's).
    SetTones {
        id: u8,
        ring: u8,
        message: u8,
    },
    /// (D6, SE2c-1) A sound of the user's, its file already in place: register it under `id` (4..=15, unused).
    AddTone(u8, String),
    RenameTone(u8, String),
    /// Every profile that uses it goes back to the built-in tone; then NEOSET deletes its file.
    DeleteTone(u8),
    /// (D6) A profile's keypad tones and vibrate on touch (not Offline's: it uses General's).
    SetFeedback {
        id: u8,
        keypad: bool,
        touch: bool,
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
    /// (D6) The user's own sounds (`sound.your_tones`): tone ids [`tone::FIRST_YOURS`]..=[`tone::LAST`].
    pub yours: Vec<YourTone>,
}

/// (D6, SE2c-1) A sound of the user's: its tone id and name. Its PCM is NEOSET's `tones\<id>.pcm` (NeoAudio
/// `tones::file`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YourTone {
    pub id: u8,
    pub name: String,
}

/// The longest `sound.your_tones` record.
pub const YOURS_MAX: usize = 512;

fn builtin(id: u8, name: &str, ring: bool, volume: u8, vibrate: bool) -> Profile {
    Profile {
        id,
        name: String::from(name),
        ring,
        volume,
        vibrate,
        ringtone: tone::BUILT_IN,
        message_tone: tone::BUILT_IN,
        keypad: false,
        touch: false,
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
            yours: Vec::new(),
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

    /// (SE2b) The tones MODEMD plays: the sound profile's (Offline's is General's).
    #[must_use]
    pub fn tones(&self) -> (u8, u8) {
        let id = if self.active == OFFLINE {
            GENERAL
        } else {
            self.active
        };
        self.get(id)
            .or_else(|| self.get(GENERAL))
            .map_or((tone::BUILT_IN, tone::BUILT_IN), |p| {
                (p.ringtone, p.message_tone)
            })
    }

    /// (D6) A tone id that plays: a built-in one, or a sound of the user's that is in the catalogue.
    #[must_use]
    pub fn known(&self, t: u8) -> bool {
        t <= tone::NONE || self.your(t).is_some()
    }

    /// (D6) A sound of the user's by its id.
    #[must_use]
    pub fn your(&self, id: u8) -> Option<&YourTone> {
        self.yours.iter().find(|t| t.id == id)
    }

    /// (D6) The lowest free id for a new sound of the user's; `None` when all twelve are taken.
    #[must_use]
    pub fn free_tone(&self) -> Option<u8> {
        (tone::FIRST_YOURS..=tone::LAST).find(|id| self.your(*id).is_none())
    }

    /// (D6) A ringtone's name: a built-in one's, or the user's name for theirs.
    #[must_use]
    pub fn ring_name(&self, t: u8) -> String {
        self.your(t)
            .map_or_else(|| String::from(tone::ring_name(t)), |y| y.name.clone())
    }

    /// (D6) A profile's row ("Morning bell · volume 7 · vibrate"), with the user's names for their sounds.
    #[must_use]
    pub fn summary(&self, p: &Profile) -> String {
        p.summary_named(&self.ring_name(p.ringtone))
    }

    /// (D6) `sound.your_tones`: `[id][len][name]` per sound of the user's.
    #[must_use]
    pub fn encode_yours(&self) -> Vec<u8> {
        let mut b = Vec::new();
        for t in &self.yours {
            b.extend_from_slice(&[t.id, t.name.len() as u8]);
            b.extend_from_slice(t.name.as_bytes());
        }
        b
    }

    /// The sounds in `sound.your_tones`; a record with a bad id, a repeated id or a bad name is skipped, a cut one
    /// ends the list.
    pub fn apply_yours(&mut self, mut b: &[u8]) {
        self.yours.clear();
        if b.len() > YOURS_MAX {
            return;
        }
        while let Some((&[id, len], rest)) = b.split_first_chunk::<2>() {
            let Some(name) = rest.get(..usize::from(len)) else {
                return;
            };
            b = &rest[usize::from(len)..];
            if let Ok(name) = core::str::from_utf8(name)
                && check_tone_name(name).is_ok()
                && (tone::FIRST_YOURS..=tone::LAST).contains(&id)
                && self.your(id).is_none()
            {
                self.yours.push(YourTone {
                    id,
                    name: String::from(name),
                });
            }
        }
    }

    /// (D6) The active keypad tones and vibrate on touch (`sound.keypad_tones`, `sound.touch_vibrate`): the sound
    /// profile's (Offline's is General's).
    #[must_use]
    pub fn feedback(&self) -> (bool, bool) {
        let id = if self.active == OFFLINE {
            GENERAL
        } else {
            self.active
        };
        self.get(id)
            .or_else(|| self.get(GENERAL))
            .map_or((false, false), |p| (p.keypad, p.touch))
    }

    /// (D6) `sound.feedback`, the profiles' feedback sidecar beside the v1 blob: `[1][n]` then n × `[id][keypad 1 |
    /// touch 2]`, every profile, so its presence says the profiles have their feedback (an empty key: from before
    /// D6, all off).
    #[must_use]
    pub fn encode_feedback(&self) -> Vec<u8> {
        let mut b = alloc::vec![FEEDBACK_VERSION, self.list.len() as u8];
        for p in &self.list {
            b.extend_from_slice(&[p.id, u8::from(p.keypad) | u8::from(p.touch) << 1]);
        }
        b
    }

    /// The feedback in `sound.feedback` onto the profiles. A record with reserved bits or for a profile that isn't
    /// there is skipped; a damaged key (another version, a wrong length) is ignored, leaving the feedback off.
    pub fn apply_feedback(&mut self, b: &[u8]) {
        let Some((&[FEEDBACK_VERSION, n], records)) = b.split_first_chunk::<2>() else {
            return;
        };
        if records.len() != 2 * usize::from(n) || b.len() > FEEDBACK_MAX {
            return;
        }
        for r in records.as_chunks::<2>().0 {
            if r[1] & !3 != 0 {
                continue;
            }
            if let Some(p) = self.get_mut(r[0]) {
                (p.keypad, p.touch) = (r[1] & 1 != 0, r[1] & 2 != 0);
            }
        }
    }

    /// (SE2b) Fold in a write to `sound.ringtone` / `sound.message_tone` by anyone but the reducer: into the sound
    /// profile. True when it changed something.
    pub fn reconcile_tones(&mut self, now: (u8, u8)) -> bool {
        if now == self.tones() || !self.known(now.0) || !self.known(now.1) {
            return false;
        }
        let id = if self.active == OFFLINE {
            GENERAL
        } else {
            self.active
        };
        let Some(p) = self.get_mut(id) else {
            return false;
        };
        (p.ringtone, p.message_tone) = now;
        p.keep_a_signal();
        true
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
                p.keep_a_signal();
                self.repair();
                Ok(None)
            }
            Command::SetTones { id, ring, message } => {
                if !self.known(ring) || !self.known(message) {
                    return Err(Refused::Argument);
                }
                if id == OFFLINE {
                    return Err(Refused::NotFound);
                }
                let p = self.get_mut(id).ok_or(Refused::NotFound)?;
                p.ringtone = ring;
                p.message_tone = message;
                p.keep_a_signal();
                Ok(None)
            }
            Command::SetFeedback { id, keypad, touch } => {
                if id == OFFLINE {
                    return Err(Refused::NotFound);
                }
                let p = self.get_mut(id).ok_or(Refused::NotFound)?;
                (p.keypad, p.touch) = (keypad, touch);
                Ok(None)
            }
            Command::AddTone(id, name) => {
                check_tone_name(&name)?;
                if !(tone::FIRST_YOURS..=tone::LAST).contains(&id) || self.your(id).is_some() {
                    return Err(Refused::Argument);
                }
                self.yours.push(YourTone { id, name });
                self.yours.sort_by_key(|t| t.id);
                Ok(None)
            }
            Command::RenameTone(id, name) => {
                check_tone_name(&name)?;
                let t = self.yours.iter_mut().find(|t| t.id == id);
                t.ok_or(Refused::NotFound)?.name = name;
                Ok(None)
            }
            Command::DeleteTone(id) => {
                if self.your(id).is_none() {
                    return Err(Refused::NotFound);
                }
                // every profile that uses it (active or not) goes back to the built-in tone first
                self.yours.retain(|t| t.id != id);
                for p in &mut self.list {
                    if p.ringtone == id {
                        p.ringtone = tone::BUILT_IN;
                    }
                    if p.message_tone == id {
                        p.message_tone = tone::BUILT_IN;
                    }
                }
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
                p.keep_a_signal();
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

    /// (SE2b) `sound.tones`: `[id][ringtone | message tone << 4]` for each profile whose tones aren't the built-in
    /// ones. A key of its own, so a shell from before SE2b (which drops unknown keys) keeps every profile.
    #[must_use]
    ///
    /// (D6) The user's own sounds go in as the built-in tone (their fallback): a shell from before them, which skips
    /// a record with an id it doesn't know, keeps the profile's other tone. The real ids are
    /// [`Self::encode_tones_yours`], read after these.
    pub fn encode_tones(&self) -> Vec<u8> {
        let legacy = |t: u8| if t > tone::NONE { tone::BUILT_IN } else { t };
        let mut b = Vec::new();
        for p in &self.list {
            let (r, m) = (legacy(p.ringtone), legacy(p.message_tone));
            if (r, m) != (tone::BUILT_IN, tone::BUILT_IN) {
                b.extend_from_slice(&[p.id, r | m << 4]);
            }
        }
        b
    }

    /// (D6) `sound.tones_yours`: `[id][ringtone | message tone << 4]`, the real pair, for each profile that uses a
    /// sound of the user's. Read after `sound.tones` (the same records, [`Self::apply_tones`]), so they win; a record
    /// naming a sound no longer in the catalogue is skipped and the fallback stays.
    #[must_use]
    pub fn encode_tones_yours(&self) -> Vec<u8> {
        let mut b = Vec::new();
        for p in &self.list {
            if p.ringtone > tone::NONE || p.message_tone > tone::NONE {
                b.extend_from_slice(&[p.id, p.ringtone | p.message_tone << 4]);
            }
        }
        b
    }

    /// The tones in `sound.tones` onto the profiles; a damaged record (or one for a profile that isn't there) is
    /// skipped, the rest kept.
    pub fn apply_tones(&mut self, b: &[u8]) {
        for r in b.as_chunks::<2>().0 {
            let (ring, message) = (r[1] & 0xf, r[1] >> 4);
            if !self.known(ring) || !self.known(message) {
                continue;
            }
            if let Some(p) = self.get_mut(r[0]) {
                (p.ringtone, p.message_tone) = (ring, message);
                p.keep_a_signal();
            }
        }
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
                    ringtone: tone::BUILT_IN,
                    message_tone: tone::BUILT_IN,
                    keypad: false,
                    touch: false,
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
            yours: Vec::new(),
        };
        p.repair();
        Some(p)
    }
}

/// The profiles from the store's three inputs (`sound.profile`, `sound.profiles`, today's mirrors): the blob, or the
/// upgrade when there is none yet or it is damaged. The shell and the Settings app read them the same way.
#[must_use]
///
/// (D6) `feedback` is `sound.feedback`: empty on a fresh phone (no blob either) gives General keypad tones (the
/// board's defaults); empty beside a blob (profiles from before D6) leaves every profile silent and still, as they were.
/// (D6) `yours` is `sound.your_tones`: tones that name a sound no longer in it play the built-in one.
pub fn read(
    active: i32,
    blob: &[u8],
    tones: &[u8],
    feedback: &[u8],
    yours: &[u8],
    mirrors: Mirrors,
) -> Profiles {
    let active = active.clamp(0, 254) as u8;
    let mut p = if blob.is_empty() {
        Profiles::upgrade(mirrors)
    } else {
        Profiles::decode(active, blob).unwrap_or_else(|| Profiles::upgrade(mirrors))
    };
    p.apply_yours(yours);
    p.apply_tones(tones);
    if feedback.is_empty() && blob.is_empty() {
        if let Some(g) = p.get_mut(GENERAL) {
            g.keypad = true;
        }
    } else {
        p.apply_feedback(feedback);
    }
    p
}

impl Profile {
    /// A ringing profile whose ringtone is None vibrates: a call must make some signal (whether a fully silent ring
    /// is ever allowed is the owner's call, settings.md SE2 plan).
    fn keep_a_signal(&mut self) {
        if self.ring && self.ringtone == tone::NONE {
            self.vibrate = true;
        }
    }

    /// What a profile does, as its row says: "Built-in ringtone · volume 7 · vibrate", "No ringing · vibrate",
    /// "Airplane mode: radios off".
    #[must_use]
    pub fn summary(&self) -> String {
        self.summary_named(tone::ring_name(self.ringtone))
    }

    /// [`Self::summary`] with the ringtone's name given (the user's name for a sound of theirs).
    #[must_use]
    pub fn summary_named(&self, ringtone: &str) -> String {
        if self.id == OFFLINE {
            return String::from("Airplane mode: radios off");
        }
        let vibrate = if self.vibrate {
            "vibrate"
        } else {
            "no vibration"
        };
        if self.ring && self.ringtone == tone::NONE {
            alloc::format!("No ringtone · {vibrate}")
        } else if self.ring {
            alloc::format!("{ringtone} · volume {} · {vibrate}", self.volume)
        } else {
            alloc::format!("No ringing · {vibrate}")
        }
    }
}

/// (D6) A preview's place: how far it played (NeoAudio `INFO`'s frames, capped at the clip's) and its line on the
/// Ringtone page.
pub mod preview {
    /// Milliseconds played of a clip of `clip_frames` at `rate`.
    #[must_use]
    pub fn elapsed_ms(frames: u64, clip_frames: u64, rate: u32) -> u32 {
        (frames.min(clip_frames) * 1000 / u64::from(rate.max(1))) as u32
    }

    /// "Playing · 0:01", "Paused · 0:01".
    #[must_use]
    pub fn label(paused: bool, ms: u32) -> alloc::string::String {
        let s = ms / 1000;
        let state = if paused { "Paused" } else { "Playing" };
        alloc::format!("{state} · {}:{:02}", s / 60, s % 60)
    }
}

/// (D6) Messages' alert setting as MODEMD keeps it (modem-ipc `sms::ALERT_*`, NeoModem `SMS_SETTINGS`).
pub mod alert {
    pub const TONE_VIBRATE: u8 = 0;
    pub const VIBRATE: u8 = 1;
    pub const SILENT: u8 = 2;
}

impl Profile {
    /// (D6) What a new message does while this profile is active, as MODEMD alerts: two buzzes whatever the call's
    /// Vibrate says, and with Messages' "tone and vibrate" the first 0.8 s of the message tone when the profile rings
    /// at a volume above 0. `alert`: Messages' setting, `None` when MODEMD couldn't say.
    #[must_use]
    pub fn message_summary(&self, alert: Option<u8>) -> String {
        const TONE: &str = "Two buzzes + the first 0.8 s";
        let plays = self.ring && self.volume > 0 && self.message_tone != tone::NONE;
        match alert {
            _ if self.id == OFFLINE => String::from("No alerts in airplane mode"),
            Some(alert::SILENT) => String::from("Silent"),
            Some(alert::TONE_VIBRATE) if plays && self.message_tone == tone::BUILT_IN => {
                String::from(TONE)
            }
            Some(alert::TONE_VIBRATE) if plays => {
                alloc::format!("{TONE} of {}", tone::message_name(self.message_tone))
            }
            Some(alert::TONE_VIBRATE | alert::VIBRATE) => String::from("Two buzzes"),
            _ => alloc::format!(
                "{} · Message alert setting unavailable",
                tone::message_name(self.message_tone)
            ),
        }
    }
}

/// (D6) A sound's name: 1–24 bytes, not blank, no control characters.
fn check_tone_name(name: &str) -> Result<(), Refused> {
    check_name(name)?;
    if name.chars().any(char::is_control) {
        return Err(Refused::Argument);
    }
    Ok(())
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
            Command::SetTones { id, ring, message } => (5, *id, *ring, *message, ""),
            Command::SetFeedback { id, keypad, touch } => {
                (6, *id, u8::from(*keypad) | u8::from(*touch) << 1, 0, "")
            }
            Command::AddTone(id, name) => (7, *id, 0, 0, name.as_str()),
            Command::RenameTone(id, name) => (8, *id, 0, 0, name.as_str()),
            Command::DeleteTone(id) => (9, *id, 0, 0, ""),
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
            5 if plain => Command::SetTones {
                id,
                ring: flags,
                message: volume,
            },
            6 if plain && flags & !3 == 0 && volume == 0 => Command::SetFeedback {
                id,
                keypad: flags & 1 != 0,
                touch: flags & 2 != 0,
            },
            7 if !plain && flags == 0 && volume == 0 => Command::AddTone(id, name),
            8 if !plain && flags == 0 && volume == 0 => Command::RenameTone(id, name),
            9 if plain && flags == 0 && volume == 0 => Command::DeleteTone(id),
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
        // the fallbacks, then the user's own sounds over them
        let mut tones = s.get(keys::SOUND_TONES).map(|b| b.0).unwrap_or_default();
        tones.extend(
            s.get(keys::SOUND_TONES_YOURS)
                .map(|b| b.0)
                .unwrap_or_default(),
        );
        let feedback = s.get(keys::SOUND_FEEDBACK).map(|b| b.0).unwrap_or_default();
        let yours = s
            .get(keys::SOUND_YOUR_TONES)
            .map(|b| b.0)
            .unwrap_or_default();
        super::read(
            s.get(keys::SOUND_PROFILE).unwrap_or(0),
            &blob,
            &tones,
            &feedback,
            &yours,
            mirrors(s),
        )
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
            keys::SOUND_YOUR_TONES,
            Bytes(p.encode_yours()),
            now_ms,
        )
        .map(drop)?;
        s.set(
            Caller::Shell,
            keys::SOUND_FEEDBACK,
            Bytes(p.encode_feedback()),
            now_ms,
        )
        .map(drop)?;
        let (keypad, touch) = p.feedback();
        s.set(Caller::Shell, keys::SOUND_KEYPAD_TONES, keypad, now_ms)
            .map(drop)?;
        s.set(Caller::Shell, keys::SOUND_TOUCH_VIBRATE, touch, now_ms)
            .map(drop)?;
        s.set(
            Caller::Shell,
            keys::SOUND_TONES_YOURS,
            Bytes(p.encode_tones_yours()),
            now_ms,
        )
        .map(drop)?;
        s.set(
            Caller::Shell,
            keys::SOUND_TONES,
            Bytes(p.encode_tones()),
            now_ms,
        )
        .map(drop)?;
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
        let (ringtone, message_tone) = p.tones();
        s.set(
            Caller::Shell,
            keys::SOUND_RINGTONE,
            i32::from(ringtone),
            now_ms,
        )
        .map(drop)?;
        s.set(
            Caller::Shell,
            keys::SOUND_MESSAGE_TONE,
            i32::from(message_tone),
            now_ms,
        )
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
        let tones = (
            s.get(keys::SOUND_RINGTONE).unwrap_or(0).clamp(0, 255) as u8,
            s.get(keys::SOUND_MESSAGE_TONE).unwrap_or(0).clamp(0, 255) as u8,
        );
        let tones_changed = p.reconcile_tones(tones);
        // (D6) profiles from before the feedback sidecar read as all off, the mirrors' defaults: nothing to write at
        // boot (the sidecar comes with the first explicit edit)
        match p.reconcile(mirrors(s), radios(s).airplane) {
            Some(set) => commit(s, &p, set, now_ms).is_ok(),
            // the upgrade's first write: the profiles go into the store as they were found
            None if fresh || tones_changed => commit(s, &p, None, now_ms).is_ok(),
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
        assert_eq!(Profiles::decode(0, &[3, 0, 0, 0, 0]), None);
        assert_eq!(Profiles::decode(0, &[1, 0, 0, 0, 1, 2, 1, 11, 0]), None);
        assert_eq!(Profiles::decode(0, &[1, 0, 0, 0, 1, 5, 1, 1, 0]), None); // an added one without a name
        // a missing active profile falls back to General
        assert_eq!(
            Profiles::decode(42, &[1, 0, 0, 0, 0]).unwrap().active,
            GENERAL
        );
    }

    #[test]
    fn feedback_per_profile_fresh_and_upgraded() {
        let m = Mirrors {
            silent: false,
            vibrate: true,
            volume: 7,
        };
        // a fresh phone: General's keypad tones on, no touch pulses (the board)
        let fresh = read(0, &[], &[], &[], &[], m);
        assert_eq!(fresh.feedback(), (true, false));
        assert!(!fresh.get(MEETING).unwrap().keypad);
        // profiles from before D6 (a blob, no sidecar): as silent and still as they were
        let old = read(0, &fresh.encode(), &[], &[], &[], m);
        assert_eq!(old.feedback(), (false, false));
        // set, choose, round trip; Offline feels like General
        let mut p = old;
        p.apply(
            Command::SetFeedback {
                id: MEETING,
                keypad: false,
                touch: true,
            },
            ON,
        )
        .unwrap();
        assert_eq!(
            p.apply(
                Command::SetFeedback {
                    id: OFFLINE,
                    keypad: true,
                    touch: true
                },
                ON
            ),
            Err(Refused::NotFound)
        );
        p.apply(Command::Choose(MEETING), ON).unwrap();
        assert_eq!(p.feedback(), (false, true));
        p.apply(Command::Choose(OFFLINE), ON).unwrap();
        assert_eq!(p.feedback(), (false, false));
        let b = p.encode_feedback();
        assert!(b.len() <= FEEDBACK_MAX);
        assert_eq!(&b[..2], &[1, 5]);
        let back = read(i32::from(p.active), &p.encode(), &[], &b, &[], m);
        assert_eq!(back, p);
        // an added profile takes General's
        p.apply(Command::Add("Car".into()), ON).unwrap();
        assert!(!p.list.last().unwrap().touch);
        // damaged sidecars: reserved bits skip a record, another version or length leaves all off
        let mut d = read(
            0,
            &p.encode(),
            &[],
            &[1, 2, GENERAL, 1, MEETING, 0x06],
            &[],
            m,
        );
        assert!(d.get(GENERAL).unwrap().keypad);
        assert!(!d.get(MEETING).unwrap().touch);
        d.apply_feedback(&[2, 1, GENERAL, 0]);
        assert!(d.get(GENERAL).unwrap().keypad);
        let d = read(0, &p.encode(), &[], &[1, 2, GENERAL, 1], &[], m);
        assert!(!d.get(GENERAL).unwrap().keypad);
    }

    #[test]
    fn your_sounds_register_play_and_go_back_to_built_in_when_deleted() {
        let mut p = today(false);
        assert_eq!(p.free_tone(), Some(tone::FIRST_YOURS));
        // not registered: refused
        let use_it = |id, ring| Command::SetTones {
            id,
            ring,
            message: tone::BUILT_IN,
        };
        assert_eq!(p.apply(use_it(GENERAL, 4), ON), Err(Refused::Argument));
        p.apply(Command::AddTone(4, "Morning bell".into()), ON)
            .unwrap();
        p.apply(Command::AddTone(9, "Gong".into()), ON).unwrap();
        // ids are the user's, not consecutive; a taken id, one out of range, a bad name: refused
        for bad in [
            Command::AddTone(4, "Again".into()),
            Command::AddTone(3, "Low".into()),
            Command::AddTone(16, "High".into()),
            Command::AddTone(5, " ".into()),
            Command::AddTone(5, "a\nb".into()),
            Command::AddTone(5, "x".repeat(25)),
        ] {
            assert_eq!(p.apply(bad, ON), Err(Refused::Argument));
        }
        assert_eq!(p.free_tone(), Some(5));
        p.apply(use_it(GENERAL, 4), ON).unwrap();
        p.apply(use_it(MEETING, 4), ON).unwrap();
        p.apply(
            Command::SetTones {
                id: OUTDOOR,
                ring: 9,
                message: 4,
            },
            ON,
        )
        .unwrap();
        assert_eq!(p.tones(), (4, tone::BUILT_IN));
        assert_eq!(
            p.summary(p.get(GENERAL).unwrap()),
            "Morning bell · volume 6 · no vibration"
        );
        p.apply(Command::RenameTone(4, "Bell".into()), ON).unwrap();
        assert_eq!(p.ring_name(4), "Bell");
        assert_eq!(
            p.apply(Command::RenameTone(5, "X".into()), ON),
            Err(Refused::NotFound)
        );
        // the catalogue and the tones round trip; a catalogue without the sound drops its uses
        let m = p.mirrors();
        let both = [p.encode_tones(), p.encode_tones_yours()].concat();
        let back = read(
            0,
            &p.encode(),
            &both,
            &p.encode_feedback(),
            &p.encode_yours(),
            m,
        );
        assert_eq!(back, p);
        // a shell from before your sounds reads only sound.tones, and skips records with ids it doesn't know: each
        // profile keeps its other tone, the user's sound reads as the built-in one
        let old_decoder = |b: &[u8]| -> Vec<(u8, u8, u8)> {
            b.as_chunks::<2>()
                .0
                .iter()
                .filter(|r| r[1] & 0xf <= tone::NONE && r[1] >> 4 <= tone::NONE)
                .map(|r| (r[0], r[1] & 0xf, r[1] >> 4))
                .collect()
        };
        let mut q = p.clone();
        q.apply(
            Command::SetTones {
                id: OUTDOOR,
                ring: 9,
                message: tone::BEEP,
            },
            ON,
        )
        .unwrap();
        assert!(old_decoder(&q.encode_tones()).contains(&(OUTDOOR, tone::BUILT_IN, tone::BEEP)));
        assert_eq!(
            old_decoder(&p.encode_tones()).len(),
            p.encode_tones().len() / 2
        );
        assert!(
            p.encode_tones()
                .as_chunks::<2>()
                .0
                .iter()
                .all(|r| r[1] & 0xf <= 3 && r[1] >> 4 <= 3)
        );
        let lost = read(0, &p.encode(), &p.encode_tones(), &[], &[], m);
        assert_eq!(lost.get(GENERAL).unwrap().ringtone, tone::BUILT_IN);
        // deleted: every profile, active or not, rings with the built-in tone again
        p.apply(Command::DeleteTone(4), ON).unwrap();
        assert_eq!(p.tones(), (tone::BUILT_IN, tone::BUILT_IN));
        assert_eq!(p.get(MEETING).unwrap().ringtone, tone::BUILT_IN);
        assert_eq!(
            (
                p.get(OUTDOOR).unwrap().ringtone,
                p.get(OUTDOOR).unwrap().message_tone
            ),
            (9, tone::BUILT_IN)
        );
        assert_eq!(p.apply(Command::DeleteTone(4), ON), Err(Refused::NotFound));
        // twelve at most
        for id in tone::FIRST_YOURS..=tone::LAST {
            let _ = p.apply(Command::AddTone(id, "S".into()), ON);
        }
        assert_eq!(p.yours.len(), 12);
        assert_eq!(p.free_tone(), None);
        assert!(p.encode_yours().len() <= YOURS_MAX);
        // a damaged catalogue: bad records skipped, a cut one ends it
        p.apply_yours(&[
            4, 1, b'A', 4, 1, b'B', 2, 1, b'C', 7, 2, 0xff, 0xfe, 8, 9, b'x',
        ]);
        assert_eq!(
            p.yours,
            [YourTone {
                id: 4,
                name: "A".into()
            }]
        );
        // the commands' bytes
        for c in [
            Command::AddTone(4, "Bell".into()),
            Command::RenameTone(15, "Gong".into()),
            Command::DeleteTone(9),
        ] {
            assert_eq!(Command::decode(&c.encode()), Some(c));
        }
        assert_eq!(Command::DeleteTone(9).encode(), [9, 9, 0, 0]);
        assert_eq!(Command::decode(&[7, 4, 0, 0]), None); // no name
        assert_eq!(Command::decode(&[9, 4, 0, 0, b'x']), None);
        assert_eq!(Command::decode(&[8, 4, 1, 0, b'x']), None);
    }

    #[test]
    fn a_preview_counts_what_played() {
        // a 2 s clip at 48 kHz: frames past its end (the queue's padding) don't count
        assert_eq!(preview::elapsed_ms(0, 96_000, 48_000), 0);
        assert_eq!(preview::elapsed_ms(48_000, 96_000, 48_000), 1_000);
        assert_eq!(preview::elapsed_ms(500_000, 96_000, 48_000), 2_000);
        assert_eq!(preview::elapsed_ms(10, 96_000, 0), 10_000); // no rate: no division by zero
        assert_eq!(preview::label(false, 1_999), "Playing · 0:01");
        assert_eq!(preview::label(true, 63_000), "Paused · 1:03");
    }

    #[test]
    fn the_message_summary_follows_modemd() {
        let mut p = today(false);
        let g = p.get(GENERAL).unwrap().clone();
        assert_eq!(
            g.message_summary(Some(alert::TONE_VIBRATE)),
            "Two buzzes + the first 0.8 s"
        );
        assert_eq!(g.message_summary(Some(alert::VIBRATE)), "Two buzzes");
        assert_eq!(g.message_summary(Some(alert::SILENT)), "Silent");
        assert_eq!(
            g.message_summary(None),
            "Built-in chime · Message alert setting unavailable"
        );
        assert_eq!(
            g.message_summary(Some(9)),
            "Built-in chime · Message alert setting unavailable"
        );
        // no ring, volume 0 or the tone None: only the buzzes
        let silent = p.get(SILENT).unwrap().clone();
        assert_eq!(
            silent.message_summary(Some(alert::TONE_VIBRATE)),
            "Two buzzes"
        );
        p.apply(
            Command::Set {
                id: MEETING,
                ring: true,
                volume: 0,
                vibrate: true,
            },
            ON,
        )
        .unwrap();
        let meeting = p.get(MEETING).unwrap();
        assert_eq!(
            meeting.message_summary(Some(alert::TONE_VIBRATE)),
            "Two buzzes"
        );
        p.apply(
            Command::SetTones {
                id: OUTDOOR,
                ring: tone::BUILT_IN,
                message: tone::BEEP,
            },
            ON,
        )
        .unwrap();
        assert_eq!(
            p.get(OUTDOOR)
                .unwrap()
                .message_summary(Some(alert::TONE_VIBRATE)),
            "Two buzzes + the first 0.8 s of Beep"
        );
        p.apply(
            Command::SetTones {
                id: OUTDOOR,
                ring: tone::BUILT_IN,
                message: tone::NONE,
            },
            ON,
        )
        .unwrap();
        assert_eq!(
            p.get(OUTDOOR)
                .unwrap()
                .message_summary(Some(alert::TONE_VIBRATE)),
            "Two buzzes"
        );
    }

    #[test]
    fn tones_per_profile_keep_a_signal_and_old_blobs_read() {
        let mut p = today(false);
        p.apply(
            Command::SetTones {
                id: MEETING,
                ring: tone::BEEP,
                message: tone::NONE,
            },
            ON,
        )
        .unwrap();
        p.apply(Command::Choose(MEETING), ON).unwrap();
        assert_eq!(p.tones(), (tone::BEEP, tone::NONE));
        // Offline plays General's tones
        p.apply(Command::Choose(OFFLINE), ON).unwrap();
        assert_eq!(p.tones(), (tone::BUILT_IN, tone::BUILT_IN));
        // ringtone None with vibrate off: the call would make no signal, so vibrate stays on
        let mut p = today(false);
        p.apply(
            Command::Set {
                id: GENERAL,
                ring: true,
                volume: 5,
                vibrate: false,
            },
            ON,
        )
        .unwrap();
        p.apply(
            Command::SetTones {
                id: GENERAL,
                ring: tone::NONE,
                message: tone::BUILT_IN,
            },
            ON,
        )
        .unwrap();
        assert!(p.get(GENERAL).unwrap().vibrate);
        p.apply(
            Command::Set {
                id: GENERAL,
                ring: true,
                volume: 5,
                vibrate: false,
            },
            ON,
        )
        .unwrap();
        assert!(p.get(GENERAL).unwrap().vibrate);
        let mut m = p.mirrors();
        m.vibrate = false;
        p.reconcile(m, false);
        assert!(p.get(GENERAL).unwrap().vibrate);
        // unknown tones are refused
        assert_eq!(
            p.apply(
                Command::SetTones {
                    id: GENERAL,
                    ring: 4,
                    message: 0
                },
                ON
            ),
            Err(Refused::Argument)
        );
        // the tones go in sound.tones, beside a blob a shell from before them still reads; a blob alone reads with
        // the built-in tones
        let mut d = Profiles::decode(p.active, &p.encode()).unwrap();
        d.apply_tones(&p.encode_tones());
        assert_eq!(d, p);
        assert!(p.encode_tones().len() <= TONES_MAX);
        // a damaged tones record is skipped, the rest kept
        let mut d = Profiles::decode(p.active, &p.encode()).unwrap();
        d.apply_tones(&[GENERAL, 0x04, 99, 0x11, MEETING]);
        assert_eq!(d.get(GENERAL).unwrap().ringtone, tone::BUILT_IN);
        let v1 = [1, 0, 0, 0, 1, 2, 1, 4, 0];
        let d = Profiles::decode(0, &v1).unwrap();
        assert_eq!(d.get(MEETING).unwrap().volume, 4);
        assert_eq!(
            (
                d.get(MEETING).unwrap().ringtone,
                d.get(MEETING).unwrap().message_tone
            ),
            (tone::BUILT_IN, tone::BUILT_IN)
        );
        assert_eq!(
            Profiles::decode(0, &[2, 0, 0, 0, 1, 2, 1, 4, 0x04, 0]),
            None
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
            Command::SetTones {
                id: 2,
                ring: 1,
                message: 3,
            },
            Command::SetFeedback {
                id: 0,
                keypad: true,
                touch: false,
            },
            Command::SetFeedback {
                id: 6,
                keypad: false,
                touch: true,
            },
        ] {
            assert_eq!(Command::decode(&c.encode()), Some(c));
        }
        // (D6) feedback: [6, id, flags, 0], no reserved bits
        assert_eq!(
            Command::SetFeedback {
                id: 2,
                keypad: true,
                touch: true
            }
            .encode(),
            [6, 2, 3, 0]
        );
        assert_eq!(Command::decode(&[6, 0, 4, 0]), None);
        assert_eq!(Command::decode(&[6, 0, 1, 1]), None);
        assert_eq!(Command::decode(&[6, 0, 1, 0, b'x']), None);
        assert_eq!(Command::decode(&[0, 1, 0]), None);
        assert_eq!(Command::decode(&[10, 0, 0, 0]), None);
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
        fn the_feedback_mirrors_are_written_once_and_follow_the_profile() {
            // a fresh store: the reducer's first write says General's keypad tones
            let (mut s, _) = Service::open(Mem::default());
            assert!(store::reconcile(&mut s, 1));
            assert!(s.get(keys::SOUND_KEYPAD_TONES).unwrap());
            assert!(!s.get(keys::SOUND_TOUCH_VIBRATE).unwrap());
            assert!(!store::reconcile(&mut s, 2));
            // profiles from before D6 (the owner's phone): all off, as the mirrors' defaults say, and nothing written
            // at boot: the store stays clean until an explicit edit
            let (mut s, _) = Service::open(Mem::default());
            let blob = Profiles::upgrade(stored(&s)).encode();
            s.set(
                Caller::Shell,
                keys::SOUND_PROFILES,
                crate::settings::Bytes(blob),
                0,
            )
            .map(drop)
            .unwrap();
            assert!(s.flush().unwrap().is_ok());
            assert!(!store::reconcile(&mut s, 1));
            assert!(!s.is_dirty());
            assert!(s.get(keys::SOUND_FEEDBACK).unwrap().0.is_empty());
            assert_eq!(store::load(&s).feedback(), (false, false));
            assert!(!s.get(keys::SOUND_KEYPAD_TONES).unwrap());
            store::apply(
                &mut s,
                Command::SetFeedback {
                    id: GENERAL,
                    keypad: true,
                    touch: true,
                },
                3,
            )
            .unwrap();
            assert!(s.get(keys::SOUND_TOUCH_VIBRATE).unwrap());
            store::apply(&mut s, Command::Choose(SILENT), 4).unwrap();
            assert!(!s.get(keys::SOUND_TOUCH_VIBRATE).unwrap());
            // only the shell's reducer writes them
            assert!(
                s.set(Caller::Settings, keys::SOUND_TOUCH_VIBRATE, true, 5)
                    .is_err()
            );
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

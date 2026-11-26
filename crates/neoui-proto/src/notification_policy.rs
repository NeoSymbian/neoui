// SPDX-License-Identifier: Apache-2.0
//! Notification channels and the user's policy (top-panel TP4, protocol 1.21; docs neoui-shell/protocol.md "TP4
//! policy"): what an app declares (`ipc::CHANNEL`, [`Channel`]), what the user chose (`n.<app>`, [`Policy`]; the
//! `notify.*` keys), and the one function that turns both into what a card may do ([`effective`]), which the shell
//! uses for the badge, the shade, the banner, the sound and the lock screen, and Settings shows (INFO 7,
//! [`AppRecord`], [`ChannelRecord`]). Also the shell's rate limit ([`Rate`]) and its saved catalogue
//! ([`encode_catalogue`]).

use crate::ipc;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

/// Importances (`CHANNEL`'s `default_importance`, an override 1–4).
pub mod importance {
    /// The Silent fold: no sound, no glyph, not on the lock screen.
    pub const MIN: u8 = 1;
    /// Quietly in the shade.
    pub const LOW: u8 = 2;
    /// Sound and the status bar's glyph.
    pub const DEFAULT: u8 = 3;
    /// Sound, glyph and a banner.
    pub const URGENT: u8 = 4;
}

/// A channel override byte: 0 inherits the declared importance, 1–4 override it, this turns the channel off.
pub const OVERRIDE_OFF: u8 = 5;

/// `n.<app>`'s flags (and INFO 7's `requested_flags`/`effective_flags`).
pub mod flag {
    pub const ALLOW: u8 = 1;
    pub const BANNER: u8 = 2;
    pub const SOUND: u8 = 4;
    pub const BADGE: u8 = 8;
    pub const ALL: u8 = 15;
}

/// `n.<app>`'s and INFO 7's lock byte, strictest last.
pub mod lock {
    pub const INHERIT: u8 = 0;
    pub const SHOW: u8 = 1;
    pub const HIDE_CONTENT: u8 = 2;
    pub const COUNTS: u8 = 3;
    pub const OFF: u8 = 4;
}

/// `n.<app>`'s tone: the active profile's message tone.
pub const TONE_PROFILE: u8 = 255;
/// The highest tone id (`sound_profile` ids 0–15).
pub const TONE_MAX: u8 = 15;
/// Explicit declarations per publisher.
pub const CHANNELS_MAX: usize = 16;
pub const NAME_MAX: usize = 32;
pub const DESCRIPTION_MAX: usize = 64;
/// The longest `CHANNEL` declaration.
pub const CHANNEL_MAX: usize = 3 + 1 + NAME_MAX + 1 + DESCRIPTION_MAX;
/// The longest `n.<app>`: every declared channel and channel 0.
pub const POLICY_MAX: usize = 5 + 2 * (CHANNELS_MAX + 1);
/// At most this many alerting posts per app in [`RATE_WINDOW_MS`].
pub const RATE_MAX: usize = 5;
pub const RATE_WINDOW_MS: u64 = 60_000;
/// `notify.bannert`'s default and range (seconds).
pub const BANNER_S: i32 = 5;

/// The apps with an `n.<app>` key (registered in `settings::registry`): every app that posts cards. DTSRV posts for
/// Clock and Calendar; Contacts, Clock and Calendar post none of their own.
pub const OWNERS: &[&str] = &[
    "phone", "messages", "dtsrv", "files", "browser", "devtools", "terminal", "kit",
];

/// The `n.<app>` key of `app`, if it has one.
#[must_use]
pub fn key_of(app: &str) -> Option<&'static str> {
    Some(match app {
        "phone" => "n.phone",
        "messages" => "n.messages",
        "dtsrv" => "n.dtsrv",
        "files" => "n.files",
        "browser" => "n.browser",
        "devtools" => "n.devtools",
        "terminal" => "n.terminal",
        "kit" => "n.kit",
        _ => return None,
    })
}

/// What Settings calls an owner (DTSRV is "Clock & Calendar", Phone "Phone & Contacts").
#[must_use]
pub fn caption(app: &str) -> &'static str {
    match app {
        "phone" => "Phone & Contacts",
        k => crate::apps::find(k).map_or("", |a| a.name),
    }
}

/// One channel an app declares (`ipc::CHANNEL`): `1:b, id:b, default_importance:b, name:s32, description:s64`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// 1–255 (0 is the implicit General channel of cards that name none).
    pub id: u8,
    pub importance: u8,
    /// 1–32 bytes.
    pub name: String,
    /// At most 64 bytes.
    pub description: String,
}

impl Channel {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Vec::with_capacity(CHANNEL_MAX);
        w.extend_from_slice(&[1, self.id, self.importance]);
        put_s(&mut w, &self.name, NAME_MAX);
        put_s(&mut w, &self.description, DESCRIPTION_MAX);
        w
    }

    /// `None` for a malformed declaration: another version, id 0, an importance outside 1–4, an empty or long name,
    /// bad UTF-8 or trailing bytes.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader(b);
        if r.b()? != 1 {
            return None;
        }
        let c = Self {
            id: r.b().filter(|&id| id != 0)?,
            importance: r
                .b()
                .filter(|i| (importance::MIN..=importance::URGENT).contains(i))?,
            name: r.s(NAME_MAX).filter(|n| !n.is_empty())?,
            description: r.s(DESCRIPTION_MAX)?,
        };
        r.end()?;
        Some(c)
    }
}

/// The user's choices for one app (`n.<app>`, typed `Bytes`): `1:b, flags:b, lock:b, tone:b, count:b,
/// (channel:b, override:b)*`. An empty value is the default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub flags: u8,
    pub lock: u8,
    pub tone: u8,
    /// `(channel, override 1–5)`, sorted by channel, no zeros.
    pub overrides: Vec<(u8, u8)>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            flags: flag::ALL,
            lock: lock::INHERIT,
            tone: TONE_PROFILE,
            overrides: Vec::new(),
        }
    }
}

impl Policy {
    /// The stored form; the default encodes empty (the store then keeps nothing).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        if *self == Self::default() {
            return Vec::new();
        }
        let mut w = Vec::with_capacity(5 + 2 * self.overrides.len());
        w.extend_from_slice(&[
            1,
            self.flags,
            self.lock,
            self.tone,
            self.overrides.len() as u8,
        ]);
        for &(c, o) in &self.overrides {
            w.extend_from_slice(&[c, o]);
        }
        w
    }

    /// The validator for SET, a stored value on load, and reset: `None` for another version, a reserved flag, a
    /// lock or tone or override out of range, a duplicate channel, a wrong count or trailing bytes.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.is_empty() {
            return Some(Self::default());
        }
        let mut r = Reader(b);
        if r.b()? != 1 {
            return None;
        }
        let flags = r.b().filter(|f| f & !flag::ALL == 0)?;
        let lock = r.b().filter(|&l| l <= lock::OFF)?;
        let tone = r.b().filter(|&t| t == TONE_PROFILE || t <= TONE_MAX)?;
        let count = usize::from(r.b()?);
        if count > CHANNELS_MAX + 1 {
            return None;
        }
        let mut overrides: Vec<(u8, u8)> = Vec::with_capacity(count);
        for _ in 0..count {
            let (c, o) = (r.b()?, r.b().filter(|&o| o <= OVERRIDE_OFF)?);
            if overrides.iter().any(|(d, _)| *d == c) {
                return None;
            }
            if o != 0 {
                overrides.push((c, o));
            }
        }
        r.end()?;
        overrides.sort_unstable();
        Some(Self {
            flags,
            lock,
            tone,
            overrides,
        })
    }

    /// Channel `channel`'s override (0 = inherit).
    #[must_use]
    pub fn override_of(&self, channel: u8) -> u8 {
        self.overrides
            .iter()
            .find(|(c, _)| *c == channel)
            .map_or(0, |(_, o)| *o)
    }

    /// Set channel `channel`'s override (0 removes it); `false` when `value` is out of range or there are too many.
    pub fn set_override(&mut self, channel: u8, value: u8) -> bool {
        if value > OVERRIDE_OFF {
            return false;
        }
        self.overrides.retain(|(c, _)| *c != channel);
        if value != 0 {
            if self.overrides.len() > CHANNELS_MAX {
                return false;
            }
            self.overrides.push((channel, value));
            self.overrides.sort_unstable();
        }
        true
    }
}

/// `notify.lock` (0 show, 1 private, 2 off) in [`lock`] terms.
#[must_use]
pub fn global_lock(notify_lock: i32) -> u8 {
    match notify_lock {
        1 => lock::HIDE_CONTENT,
        2 => lock::OFF,
        _ => lock::SHOW,
    }
}

/// What a channel's cards may do, after the app's and the user's choices (never DND: that holds the alerts of a
/// moment, see [`alert`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effective {
    /// 0: the card is refused (the app or the channel is off); else 1–4.
    pub importance: u8,
    /// [`flag`] bits: accepted, banner, sound, badge.
    pub flags: u8,
    /// The status bar's app glyph.
    pub glyph: bool,
    /// [`lock`] 1–4.
    pub lock: u8,
}

impl Effective {
    #[must_use]
    pub fn accepts(&self) -> bool {
        self.flags & flag::ALLOW != 0
    }
}

/// The one policy function: channel `channel` of an app with `policy`, which declared that channel with `declared`
/// importance (`None`: undeclared, or channel 0, General: Default). `global` is [`global_lock`].
#[must_use]
pub fn effective(policy: &Policy, channel: u8, declared: Option<u8>, global: u8) -> Effective {
    let over = policy.override_of(channel);
    if policy.flags & flag::ALLOW == 0 || over == OVERRIDE_OFF {
        return Effective {
            lock: lock::OFF,
            ..Effective::default()
        };
    }
    let imp = if over != 0 {
        over
    } else {
        declared.unwrap_or(importance::DEFAULT)
    };
    let mut flags = flag::ALLOW | policy.flags & flag::BADGE;
    if imp == importance::URGENT && policy.flags & flag::BANNER != 0 {
        flags |= flag::BANNER;
    }
    if imp >= importance::DEFAULT && policy.flags & flag::SOUND != 0 {
        flags |= flag::SOUND;
    }
    let by_importance = match imp {
        importance::MIN => lock::OFF,
        importance::LOW => lock::COUNTS,
        _ => lock::SHOW,
    };
    let app = if policy.lock == lock::INHERIT {
        lock::SHOW
    } else {
        policy.lock
    };
    Effective {
        importance: imp,
        flags,
        glyph: imp >= importance::DEFAULT,
        lock: app.max(global).max(by_importance),
    }
}

/// A post as the shell saw it, for [`alert`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Post {
    /// A card the shade didn't have (a replacement never alerts again).
    pub new: bool,
    /// `NOTIFY_SILENT_UPDATE` or ongoing: never an alert, and free.
    pub quiet: bool,
}

/// What a post does now: the banner, the sound, a vibration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Alert {
    pub banner: bool,
    pub sound: bool,
}

impl Alert {
    #[must_use]
    pub fn any(&self) -> bool {
        self.banner || self.sound
    }
}

/// What card `post` with effective policy `e` does under Do not disturb `dnd`; `rate` says whether the app may
/// alert again (asked only when it would: [`Rate::take`]). DND keeps the card in the shade and holds the rest.
#[must_use]
pub fn alert(e: &Effective, post: Post, dnd: bool, rate: impl FnOnce() -> bool) -> Alert {
    if !e.accepts() || !post.new || post.quiet || dnd {
        return Alert::default();
    }
    let a = Alert {
        banner: e.flags & flag::BANNER != 0,
        sound: e.flags & flag::SOUND != 0,
    };
    if a.any() && rate() {
        a
    } else {
        Alert::default()
    }
}

/// Do not disturb now: `notify.dnd` on and `notify.dndto` (empty or 0: until turned off; else a UTC second) not
/// passed at `now_utc`.
#[must_use]
pub fn dnd_active(dnd: bool, until: &[u8], now_utc: i64) -> bool {
    dnd && match dnd_until(until) {
        Some(t) if t != 0 => now_utc < t,
        _ => true,
    }
}

/// `notify.dndto`'s UTC second (0 = until turned off); `None` for a malformed value.
#[must_use]
pub fn dnd_until(b: &[u8]) -> Option<i64> {
    match b.len() {
        0 => Some(0),
        8 => Some(i64::from_le_bytes(b.try_into().ok()?)),
        _ => None,
    }
}

/// The alerting posts per app in the last minute (monotonic ms): at most [`RATE_MAX`].
#[derive(Default)]
pub struct Rate(Vec<(&'static str, VecDeque<u64>)>);

impl Rate {
    /// One more alert for `app` at `now_ms`, if the window has room (then it counts).
    pub fn take(&mut self, app: &'static str, now_ms: u64) -> bool {
        let i = match self.0.iter().position(|(k, _)| *k == app) {
            Some(i) => i,
            None => {
                self.0.push((app, VecDeque::new()));
                self.0.len() - 1
            }
        };
        let q = &mut self.0[i].1;
        while q
            .front()
            .is_some_and(|&t| now_ms.saturating_sub(t) >= RATE_WINDOW_MS)
        {
            q.pop_front();
        }
        if q.len() >= RATE_MAX {
            return false;
        }
        q.push_back(now_ms);
        true
    }
}

/// INFO 7 app record: `0:b, owner:key, caption:s48, requested_flags:b, effective_flags:b, requested_lock:b,
/// effective_lock:b, tone:b`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRecord {
    pub owner: String,
    pub caption: String,
    pub requested_flags: u8,
    pub effective_flags: u8,
    pub requested_lock: u8,
    pub effective_lock: u8,
    pub tone: u8,
}

/// INFO 7 channel record: `1:b, owner:key, channel:b, declared_importance:b, override:b, effective_importance:b,
/// effective_flags:b, effective_lock:b, name:s32, description:s64`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelRecord {
    pub owner: String,
    pub channel: u8,
    pub declared: u8,
    pub over: u8,
    pub effective: u8,
    pub effective_flags: u8,
    pub effective_lock: u8,
    pub name: String,
    pub description: String,
}

/// One INFO 7 record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    App(AppRecord),
    Channel(ChannelRecord),
}

impl Record {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Vec::new();
        match self {
            Record::App(a) => {
                w.push(0);
                put_s(&mut w, &a.owner, ipc::KEY_MAX);
                put_s(&mut w, &a.caption, ipc::TITLE_MAX);
                w.extend_from_slice(&[
                    a.requested_flags,
                    a.effective_flags,
                    a.requested_lock,
                    a.effective_lock,
                    a.tone,
                ]);
            }
            Record::Channel(c) => {
                w.push(1);
                put_s(&mut w, &c.owner, ipc::KEY_MAX);
                w.extend_from_slice(&[
                    c.channel,
                    c.declared,
                    c.over,
                    c.effective,
                    c.effective_flags,
                    c.effective_lock,
                ]);
                put_s(&mut w, &c.name, NAME_MAX);
                put_s(&mut w, &c.description, DESCRIPTION_MAX);
            }
        }
        w
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader(b);
        let rec = match r.b()? {
            0 => Record::App(AppRecord {
                owner: r.s(ipc::KEY_MAX)?,
                caption: r.s(ipc::TITLE_MAX)?,
                requested_flags: r.b().filter(|f| f & !flag::ALL == 0)?,
                effective_flags: r.b().filter(|f| f & !flag::ALL == 0)?,
                requested_lock: r.b().filter(|&l| l <= lock::OFF)?,
                effective_lock: r.b().filter(|&l| l <= lock::OFF)?,
                tone: r.b()?,
            }),
            1 => Record::Channel(ChannelRecord {
                owner: r.s(ipc::KEY_MAX)?,
                channel: r.b()?,
                declared: r.b().filter(|&i| i <= importance::URGENT)?,
                over: r.b().filter(|&o| o <= OVERRIDE_OFF)?,
                effective: r.b().filter(|&i| i <= importance::URGENT)?,
                effective_flags: r.b().filter(|f| f & !flag::ALL == 0)?,
                effective_lock: r.b().filter(|&l| l <= lock::OFF)?,
                name: r.s(NAME_MAX)?,
                description: r.s(DESCRIPTION_MAX)?,
            }),
            _ => return None,
        };
        r.end()?;
        Some(rec)
    }
}

/// INFO 7's records for one owner: its app record, then a record per declared channel (by id; `General`, channel
/// 0, when it declared none). `policy` is its `n.<app>`, `global` [`global_lock`].
#[must_use]
pub fn records(owner: &str, policy: &Policy, declared: &[Channel], global: u8) -> Vec<Record> {
    let general = effective(policy, 0, None, global);
    let mut out = alloc::vec![Record::App(AppRecord {
        owner: owner.into(),
        caption: caption(owner).into(),
        requested_flags: policy.flags,
        effective_flags: if policy.flags & flag::ALLOW != 0 {
            policy.flags
        } else {
            0
        },
        requested_lock: policy.lock,
        effective_lock: general.lock,
        tone: policy.tone,
    })];
    let mut sorted: Vec<&Channel> = declared.iter().collect();
    sorted.sort_by_key(|c| c.id);
    let general_row = Channel {
        id: 0,
        importance: importance::DEFAULT,
        name: "General".into(),
        description: String::new(),
    };
    if sorted.is_empty() {
        sorted.push(&general_row);
    }
    for c in sorted {
        let e = effective(policy, c.id, Some(c.importance), global);
        out.push(Record::Channel(ChannelRecord {
            owner: owner.into(),
            channel: c.id,
            declared: c.importance,
            over: policy.override_of(c.id),
            effective: e.importance,
            effective_flags: e.flags,
            effective_lock: e.lock,
            name: c.name.clone(),
            description: c.description.clone(),
        }));
    }
    out
}

/// An importance as Settings names it (nt-settings): 0 (the channel is off) "Off".
#[must_use]
pub fn importance_name(i: u8) -> &'static str {
    match i {
        importance::URGENT => "Urgent",
        importance::DEFAULT => "Default",
        importance::LOW => "Low",
        importance::MIN => "Silent",
        _ => "Off",
    }
}

/// An app's line on Notifications (set2-notifications: "Banners · sound", "Missed calls · banners · sound", "Off"):
/// its one declared channel's name if it has only one, banners if on, sound or no sound. The Settings page, search
/// and the root read this one text.
#[must_use]
pub fn app_summary(app: &AppRecord, channels: &[ChannelRecord]) -> String {
    if app.requested_flags & flag::ALLOW == 0 {
        return String::from("Off");
    }
    let mut parts: Vec<&str> = Vec::new();
    let declared: Vec<&ChannelRecord> = channels
        .iter()
        .filter(|c| c.owner == app.owner && c.channel != 0)
        .collect();
    if let [one] = declared.as_slice() {
        parts.push(&one.name);
    }
    if app.requested_flags & flag::BANNER != 0 {
        parts.push("banners");
    }
    parts.push(if app.requested_flags & flag::SOUND != 0 {
        "sound"
    } else {
        "no sound"
    });
    let line = parts.join(" · ");
    let mut c = line.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_uppercase().chain(c).collect::<String>()
    })
}

/// The shell's saved declarations (`C:\private\e0002001\channels.dat`): `1:b, count:h, (owner:s16, len:b,
/// declaration[len])*`.
#[must_use]
pub fn encode_catalogue(list: &[(&str, &Channel)]) -> Vec<u8> {
    let mut w = alloc::vec![1];
    w.extend_from_slice(&(list.len() as u16).to_le_bytes());
    for (owner, c) in list {
        put_s(&mut w, owner, ipc::KEY_MAX);
        let d = c.encode();
        w.push(d.len() as u8);
        w.extend_from_slice(&d);
    }
    w
}

/// The saved declarations, for [`OWNERS`] only and at most [`CHANNELS_MAX`] each; `None` when damaged.
#[must_use]
pub fn decode_catalogue(b: &[u8]) -> Option<Vec<(&'static str, Channel)>> {
    let mut r = Reader(b);
    if r.b()? != 1 {
        return None;
    }
    let count = u16::from_le_bytes([r.b()?, r.b()?]);
    let mut out: Vec<(&'static str, Channel)> = Vec::new();
    for _ in 0..count {
        let owner = r.s(ipc::KEY_MAX)?;
        let len = usize::from(r.b()?);
        let c = Channel::decode(r.take(len)?)?;
        let owner = OWNERS.iter().copied().find(|o| *o == owner)?;
        if out.iter().any(|(o, d)| *o == owner && d.id == c.id)
            || out.iter().filter(|(o, _)| *o == owner).count() >= CHANNELS_MAX
        {
            return None;
        }
        out.push((owner, c));
    }
    r.end()?;
    Some(out)
}

fn put_s(w: &mut Vec<u8>, s: &str, max: usize) {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    w.push(end as u8);
    w.extend_from_slice(&s.as_bytes()[..end]);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.0.len() {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn b(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn s(&mut self, max: usize) -> Option<String> {
        let n = usize::from(self.b()?);
        if n > max {
            return None;
        }
        core::str::from_utf8(self.take(n)?).ok().map(String::from)
    }
    fn end(self) -> Option<()> {
        self.0.is_empty().then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn ch(id: u8, imp: u8, name: &str) -> Channel {
        Channel {
            id,
            importance: imp,
            name: name.into(),
            description: String::new(),
        }
    }

    #[test]
    fn declarations_round_trip_and_refuse_bad_ones() {
        let c = Channel {
            id: 1,
            importance: importance::URGENT,
            name: "Conversations".into(),
            description: "New messages".into(),
        };
        let b = c.encode();
        assert_eq!(&b[..4], &[1, 1, 4, 13]);
        assert_eq!(Channel::decode(&b), Some(c));
        for bad in [
            vec![2, 1, 4, 1, b'a', 0],    // version
            vec![1, 0, 4, 1, b'a', 0],    // id 0
            vec![1, 1, 5, 1, b'a', 0],    // importance
            vec![1, 1, 0, 1, b'a', 0],    // importance
            vec![1, 1, 3, 0, 0],          // empty name
            vec![1, 1, 3, 1, 0xff, 0],    // UTF-8
            vec![1, 1, 3, 1, b'a', 0, 0], // trailing
            vec![1, 1, 3, 1, b'a'],       // truncated
        ] {
            assert_eq!(Channel::decode(&bad), None, "{bad:?}");
        }
        let mut long = vec![1, 1, 3, 33];
        long.extend_from_slice(&[b'a'; 33]);
        long.push(0);
        assert_eq!(Channel::decode(&long), None);
    }

    #[test]
    fn policies_validate_and_default_empty() {
        assert_eq!(Policy::decode(&[]), Some(Policy::default()));
        assert!(Policy::default().encode().is_empty());
        let mut p = Policy {
            flags: flag::ALLOW | flag::BADGE,
            ..Policy::default()
        };
        assert!(p.set_override(3, importance::LOW));
        assert!(p.set_override(1, OVERRIDE_OFF));
        let b = p.encode();
        assert_eq!(b, vec![1, 9, 0, 255, 2, 1, 5, 3, 2]);
        assert_eq!(Policy::decode(&b), Some(p.clone()));
        assert!(p.set_override(3, 0));
        assert_eq!(p.overrides, vec![(1, 5)]);
        for bad in [
            vec![2, 15, 0, 255, 0],             // version
            vec![1, 16, 0, 255, 0],             // reserved flag
            vec![1, 15, 5, 255, 0],             // lock
            vec![1, 15, 0, 16, 0],              // tone
            vec![1, 15, 0, 255, 1, 1, 6],       // override
            vec![1, 15, 0, 255, 2, 1, 1, 1, 2], // duplicate
            vec![1, 15, 0, 255, 1, 1],          // truncated
            vec![1, 15, 0, 255, 0, 0],          // trailing
        ] {
            assert_eq!(Policy::decode(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn one_function_for_every_surface() {
        let p = Policy::default();
        let urgent = effective(&p, 1, Some(importance::URGENT), lock::SHOW);
        assert_eq!(urgent.flags, flag::ALL);
        assert!(urgent.glyph);
        let default = effective(&p, 1, Some(importance::DEFAULT), lock::SHOW);
        assert_eq!(default.flags, flag::ALLOW | flag::SOUND | flag::BADGE);
        let low = effective(&p, 1, Some(importance::LOW), lock::SHOW);
        assert_eq!(low.flags, flag::ALLOW | flag::BADGE);
        assert!(!low.glyph);
        assert_eq!(low.lock, lock::COUNTS);
        assert_eq!(
            effective(&p, 1, Some(importance::MIN), lock::SHOW).lock,
            lock::OFF
        );
        // undeclared and General: Default
        assert_eq!(
            effective(&p, 0, None, lock::SHOW).importance,
            importance::DEFAULT
        );
        // the four banner/sound combinations
        for (flags, banner, sound) in [
            (15, true, true),
            (13, false, true),
            (11, true, false),
            (9, false, false),
        ] {
            let p = Policy {
                flags,
                ..Policy::default()
            };
            let e = effective(&p, 1, Some(importance::URGENT), lock::SHOW);
            let a = alert(
                &e,
                Post {
                    new: true,
                    quiet: false,
                },
                false,
                || true,
            );
            assert_eq!((a.banner, a.sound), (banner, sound), "{flags}");
        }
        // off: the app, or one channel
        let off = Policy {
            flags: flag::ALL & !flag::ALLOW,
            ..Policy::default()
        };
        assert!(!effective(&off, 1, Some(4), lock::SHOW).accepts());
        let mut one = Policy::default();
        one.set_override(2, OVERRIDE_OFF);
        assert!(!effective(&one, 2, Some(4), lock::SHOW).accepts());
        assert!(effective(&one, 1, Some(4), lock::SHOW).accepts());
        // an override wins over the declaration
        one.set_override(1, importance::LOW);
        assert_eq!(
            effective(&one, 1, Some(4), lock::SHOW).importance,
            importance::LOW
        );
        // the stricter of the app's lock and the global one
        let private = Policy {
            lock: lock::HIDE_CONTENT,
            ..Policy::default()
        };
        assert_eq!(
            effective(&private, 1, Some(4), global_lock(0)).lock,
            lock::HIDE_CONTENT
        );
        assert_eq!(
            effective(&private, 1, Some(4), global_lock(2)).lock,
            lock::OFF
        );
    }

    #[test]
    fn alerts_hold_for_dnd_silent_updates_replacements_and_the_rate() {
        let e = effective(&Policy::default(), 1, Some(importance::URGENT), lock::SHOW);
        let new = Post {
            new: true,
            quiet: false,
        };
        assert!(alert(&e, new, false, || true).any());
        assert!(!alert(&e, new, true, || true).any(), "DND");
        assert!(
            !alert(
                &e,
                Post {
                    new: false,
                    quiet: false
                },
                false,
                || true
            )
            .any()
        );
        assert!(
            !alert(
                &e,
                Post {
                    new: true,
                    quiet: true
                },
                false,
                || true
            )
            .any()
        );
        assert!(!alert(&e, new, false, || false).any());
        // a Low card never costs the rate
        let low = effective(&Policy::default(), 1, Some(importance::LOW), lock::SHOW);
        assert!(!alert(&low, new, false, || panic!("asked")).any());
        let mut r = Rate::default();
        for i in 0..5 {
            assert!(r.take("files", i * 1000));
        }
        assert!(!r.take("files", 10_000));
        assert!(r.take("browser", 10_000), "per app");
        assert!(r.take("files", 60_000), "the first one left the window");
    }

    #[test]
    fn dnd_ends_at_its_time() {
        assert!(!dnd_active(false, &[], 10));
        assert!(dnd_active(true, &[], 10));
        assert!(dnd_active(true, &0i64.to_le_bytes(), 10));
        assert!(dnd_active(true, &20i64.to_le_bytes(), 19));
        assert!(!dnd_active(true, &20i64.to_le_bytes(), 20));
        assert_eq!(dnd_until(&[1, 2, 3]), None);
    }

    #[test]
    fn info_records_round_trip_with_general_for_no_declarations() {
        let recs = records("files", &Policy::default(), &[], lock::SHOW);
        assert_eq!(recs.len(), 2);
        let Record::Channel(g) = &recs[1] else {
            panic!()
        };
        assert_eq!((g.channel, g.name.as_str()), (0, "General"));
        let decl = [ch(3, 2, "Delivery reports"), ch(1, 4, "Conversations")];
        let recs = records("messages", &Policy::default(), &decl, lock::SHOW);
        assert_eq!(recs.len(), 3);
        for r in &recs {
            assert_eq!(Record::decode(&r.encode()).as_ref(), Some(r));
        }
        let Record::Channel(first) = &recs[1] else {
            panic!()
        };
        assert_eq!(first.channel, 1, "sorted by id");
        let Record::App(a) = &recs[0] else { panic!() };
        assert_eq!(a.caption, "Messages");
        assert_eq!(Record::decode(&[2]), None);
    }

    #[test]
    fn summaries_read_as_the_board_writes_them() {
        let app = |flags| AppRecord {
            owner: "files".into(),
            caption: "Files".into(),
            requested_flags: flags,
            effective_flags: flags,
            requested_lock: 0,
            effective_lock: 1,
            tone: TONE_PROFILE,
        };
        let recs = records(
            "phone",
            &Policy::default(),
            &[ch(2, 3, "Missed calls")],
            lock::SHOW,
        );
        let chans: Vec<ChannelRecord> = recs
            .into_iter()
            .filter_map(|r| match r {
                Record::Channel(c) => Some(c),
                Record::App(_) => None,
            })
            .collect();
        let mut phone = app(flag::ALL);
        phone.owner = "phone".into();
        assert_eq!(
            app_summary(&phone, &chans),
            "Missed calls · banners · sound"
        );
        assert_eq!(app_summary(&app(flag::ALL), &[]), "Banners · sound");
        assert_eq!(
            app_summary(&app(flag::ALLOW | flag::BANNER), &[]),
            "Banners · no sound"
        );
        assert_eq!(app_summary(&app(flag::BANNER), &[]), "Off");
        assert_eq!(importance_name(0), "Off");
        assert_eq!(importance_name(importance::MIN), "Silent");
    }

    #[test]
    fn the_catalogue_round_trips_and_refuses_strangers() {
        let a = ch(1, 4, "Conversations");
        let b = ch(2, 3, "Failed to send");
        let bytes = encode_catalogue(&[("messages", &a), ("messages", &b)]);
        let back = decode_catalogue(&bytes).unwrap();
        assert_eq!(back, vec![("messages", a.clone()), ("messages", b)]);
        assert_eq!(decode_catalogue(&encode_catalogue(&[("nobody", &a)])), None);
        assert_eq!(
            decode_catalogue(&encode_catalogue(&[("messages", &a), ("messages", &a)])),
            None
        );
        for k in OWNERS {
            let key = key_of(k).unwrap();
            assert_eq!(&key[2..], *k);
            let spec = crate::settings::registry::lookup(key).expect("registered");
            assert_eq!(spec.limit, crate::settings::registry::Limit::Policy);
            assert!(crate::apps::find(k).is_some(), "{k}");
        }
    }
}

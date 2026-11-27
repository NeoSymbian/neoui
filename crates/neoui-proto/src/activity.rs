// SPDX-License-Identifier: Apache-2.0
//! Live activities (top-panel TP6, protocol 1.22; docs neoui-shell/protocol.md "ACTIVITY"): what is happening now,
//! drawn by the shell in the island, at the top of the shade's Live section and on the lock screen. An app sends
//! [`Activity`] with `ipc::ACTIVITY`; a tap comes back as `ipc::AppEvent::ActivityTap { id, i }` (`i` 0 the island
//! or card, `1..=2` the app's button `i - 1`, [`key`] the shell's own call keys).
//!
//! Record: `1:b, id:b, kind:b, flags:b, tint:w, lead:h, page:h, title:s32, text:s64, trail:b, trail payload,
//! buttons:b, button:s16*`. Trail payloads: Text `s8`, Since and Until `q` (unix ms; the shell ticks them), Ring
//! `value:h, max:h`. An empty title (every other field zero) ends activity `id`.
//!
//! A Call-kind activity has no buttons of its own: the shell draws Mute · Keypad · Speaker · End (owner decision O1)
//! and reports them as [`key`]s; [`flag::MUTED`] and [`flag::SPEAKER`] say how to draw the first and third.

use crate::settings_app::wire::{R, W};
use alloc::string::String;
use alloc::vec::Vec;

/// [`Activity::kind`], in the island's order (the two highest show).
pub mod kind {
    pub const CALL: u8 = 1;
    pub const RECORDING: u8 = 2;
    pub const TIMER: u8 = 3;
    /// The shell's own, derived from the media session (`ipc::MEDIA`): a player never sends it.
    pub const MEDIA: u8 = 4;
    pub const PROGRESS: u8 = 5;
    pub const OTHER: u8 = 6;
    pub(crate) const LAST: u8 = OTHER;
}

/// [`Activity::flags`]: a call's state (Call only).
pub mod flag {
    pub const MUTED: u8 = 1;
    pub const SPEAKER: u8 = 2;
    pub(crate) const ALL: u8 = 3;
}

/// `ActivityTap`'s `i` for the keys the shell draws on a Call activity (O1).
pub mod key {
    pub const MUTE: u8 = 0x10;
    pub const KEYPAD: u8 = 0x11;
    pub const SPEAKER: u8 = 0x12;
    pub const END: u8 = 0x13;
}

/// Ids per app: 0–3.
pub const IDS: u8 = 4;
pub const TITLE_MAX: usize = 32;
pub const TEXT_MAX: usize = 64;
pub const TRAIL_TEXT_MAX: usize = 8;
pub const BUTTON_MAX: usize = 16;
pub const BUTTONS_MAX: usize = 2;
/// The longest encoded [`Activity`].
pub const ACTIVITY_MAX: usize =
    4 + 4 + 2 + 2 + (1 + TITLE_MAX) + (1 + TEXT_MAX) + 1 + 8 + 1 + BUTTONS_MAX * (1 + BUTTON_MAX);

/// What the island shows at its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trail {
    /// At most 8 bytes ("2 of 5").
    Text(String),
    /// Counting up from this unix ms (a call's duration).
    Since(u64),
    /// Counting down to this unix ms (a timer).
    Until(u64),
    /// A ring, `value` of `max` (`max` > 0); progress is a value, never a string.
    Ring { value: u16, max: u16 },
}

impl Default for Trail {
    fn default() -> Self {
        Trail::Text(String::new())
    }
}

/// One activity of the calling app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// `0..IDS`.
    pub id: u8,
    /// One of [`kind`].
    pub kind: u8,
    /// [`flag`] bits (Call only).
    pub flags: u8,
    /// 0xRRGGBB; the shell clamps it to readable colours.
    pub tint: u32,
    /// A system glyph (the kit's icon number); 0 = the app's own.
    pub lead: u16,
    /// The app's page a tap opens.
    pub page: u16,
    /// 1–32 bytes (empty ends the activity).
    pub title: String,
    /// The expanded card's line, at most 64 bytes.
    pub text: String,
    pub trail: Trail,
    /// Up to two labels ("Pause", "Cancel"); none for a Call.
    pub buttons: Vec<String>,
}

impl Activity {
    /// The record that ends activity `id`.
    #[must_use]
    pub fn end(id: u8) -> Self {
        Self {
            id,
            ..Self::default()
        }
    }

    /// Whether this ends the activity rather than showing it.
    #[must_use]
    pub fn is_end(&self) -> bool {
        self.title.is_empty()
    }

    /// The record; strings are cut on a character boundary, buttons past two are left out.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.0.extend_from_slice(&[1, self.id, self.kind, self.flags]);
        w.w(self.tint);
        w.h(self.lead);
        w.h(self.page);
        w.s(&self.title, TITLE_MAX);
        w.s(&self.text, TEXT_MAX);
        match &self.trail {
            Trail::Text(t) => {
                w.b(0);
                w.s(t, TRAIL_TEXT_MAX);
            }
            Trail::Since(t) => {
                w.b(1);
                w.q(*t);
            }
            Trail::Until(t) => {
                w.b(2);
                w.q(*t);
            }
            Trail::Ring { value, max } => {
                w.b(3);
                w.h(*value);
                w.h(*max);
            }
        }
        let n = self.buttons.len().min(BUTTONS_MAX);
        w.b(n as u8);
        for b in &self.buttons[..n] {
            w.s(b, BUTTON_MAX);
        }
        w.0
    }

    /// `None` for a malformed record: another version, an id past 3, an unknown kind or trail, a reserved bit (or a
    /// flag on anything but a Call), a long or non-UTF-8 string, an empty button, a Call with buttons, a ring past its
    /// max, an end with anything else in it, a short record or trailing bytes.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() > ACTIVITY_MAX {
            return None;
        }
        let mut r = R(b);
        if r.b()? != 1 {
            return None;
        }
        let id = r.b().filter(|&i| i < IDS)?;
        let kind = r.max(kind::LAST)?;
        let flags = r.max(flag::ALL)?;
        let (tint, lead, page) = (r.w()?, r.h()?, r.h()?);
        let title = r.s(TITLE_MAX)?;
        let text = r.s(TEXT_MAX)?;
        let trail = match r.b()? {
            0 => Trail::Text(r.s(TRAIL_TEXT_MAX)?),
            1 => Trail::Since(r.q()?),
            2 => Trail::Until(r.q()?),
            3 => {
                let (value, max) = (r.h()?, r.h()?);
                if max == 0 || value > max {
                    return None;
                }
                Trail::Ring { value, max }
            }
            _ => return None,
        };
        let n = usize::from(r.b()?);
        if n > BUTTONS_MAX {
            return None;
        }
        let buttons = (0..n)
            .map(|_| r.s(BUTTON_MAX).filter(|s| !s.is_empty()))
            .collect::<Option<Vec<_>>>()?;
        r.end()?;
        let a = Self {
            id,
            kind,
            flags,
            tint,
            lead,
            page,
            title,
            text,
            trail,
            buttons,
        };
        let ok = if a.is_end() {
            a == Self::end(id)
        } else {
            kind != 0
                && (kind == kind::CALL || flags == 0)
                && (kind != kind::CALL || a.buttons.is_empty())
        };
        ok.then_some(a)
    }
}

/// `island.deny`: the app keys turned off for the island, each `s16`.
#[must_use]
pub fn encode_deny(keys: &[&str]) -> Vec<u8> {
    let mut w = W::default();
    for k in keys {
        w.s(k, crate::ipc::KEY_MAX);
    }
    w.0
}

/// The keys of `island.deny`; `None` when damaged (an empty or long key, bad UTF-8, a cut-off entry).
#[must_use]
pub fn decode_deny(b: &[u8]) -> Option<Vec<String>> {
    let mut r = R(b);
    let mut out = Vec::new();
    while !r.0.is_empty() {
        out.push(r.s(crate::ipc::KEY_MAX).filter(|k| !k.is_empty())?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn timer() -> Activity {
        Activity {
            id: 1,
            kind: kind::TIMER,
            flags: 0,
            tint: 0xff9500,
            lead: 7,
            page: 2,
            title: "Timer".into(),
            text: "Tea".into(),
            trail: Trail::Until(1_700_000_300_000),
            buttons: vec!["Pause".into(), "Cancel".into()],
        }
    }

    #[test]
    fn round_trips_every_trail() {
        let trails = [
            Trail::Text("2 of 5".into()),
            Trail::Since(1),
            Trail::Until(u64::MAX),
            Trail::Ring { value: 3, max: 10 },
            Trail::Ring { value: 10, max: 10 },
        ];
        for trail in trails {
            let a = Activity { trail, ..timer() };
            assert_eq!(Activity::decode(&a.encode()), Some(a));
        }
        let call = Activity {
            id: 0,
            kind: kind::CALL,
            flags: flag::MUTED | flag::SPEAKER,
            trail: Trail::Since(5),
            buttons: vec![],
            ..timer()
        };
        assert_eq!(Activity::decode(&call.encode()), Some(call));
        let end = Activity::end(3);
        assert!(end.is_end());
        assert_eq!(Activity::decode(&end.encode()), Some(end));
        // the longest
        let big = Activity {
            title: "t".repeat(40),
            text: "x".repeat(70),
            trail: Trail::Since(0),
            buttons: vec!["b".repeat(20), "c".repeat(16), "dropped".into()],
            ..timer()
        };
        let b = big.encode();
        assert_eq!(b.len(), ACTIVITY_MAX);
        assert_eq!(Activity::decode(&b).unwrap().buttons.len(), 2);
    }

    #[test]
    fn malformed_records_are_refused() {
        let good = timer().encode();
        for n in 0..good.len() {
            assert_eq!(Activity::decode(&good[..n]), None, "{n}");
        }
        let mut long = good.clone();
        long.push(0);
        assert_eq!(Activity::decode(&long), None);
        // version, id 4, kind 0 and 7, flags on a timer, a reserved flag
        for (at, v) in [(0, 2), (1, 4), (2, 0), (2, 7), (3, 1), (3, 4)] {
            let mut b = good.clone();
            b[at] = v;
            assert_eq!(Activity::decode(&b), None, "byte {at} = {v}");
        }
        let bad = [
            Activity {
                kind: kind::CALL,
                ..timer()
            }, // a call with buttons
            Activity {
                trail: Trail::Ring { value: 1, max: 0 },
                ..timer()
            },
            Activity {
                trail: Trail::Ring { value: 4, max: 3 },
                ..timer()
            },
            Activity {
                buttons: vec![String::new()],
                ..timer()
            },
            Activity {
                title: String::new(),
                ..timer()
            }, // an end that still carries the rest
        ];
        for a in bad {
            assert_eq!(Activity::decode(&a.encode()), None, "{a:?}");
        }
        // an unknown trail; three buttons on the wire
        let mut b = Activity {
            trail: Trail::Since(0),
            buttons: vec![],
            ..timer()
        }
        .encode();
        let trail_at = b.len() - 1 - 8 - 1;
        b[trail_at] = 4;
        assert_eq!(Activity::decode(&b), None);
        let mut b = timer().encode();
        let count_at = b.len() - 1 - (1 + 5) - (1 + 6); // before "Pause" and "Cancel"
        assert_eq!(b[count_at], 2);
        b[count_at] = 3;
        assert_eq!(Activity::decode(&b), None);
        assert_eq!(Activity::decode(&[0u8; ACTIVITY_MAX + 1]), None);
    }

    #[test]
    fn the_deny_list() {
        let b = encode_deny(&["files", "browser"]);
        assert_eq!(
            decode_deny(&b),
            Some(vec!["files".into(), "browser".into()])
        );
        assert_eq!(decode_deny(&[]), Some(vec![]));
        assert_eq!(decode_deny(&[0]), None);
        assert_eq!(decode_deny(&[3, b'a']), None);
        assert_eq!(decode_deny(&[17; 18]), None);
    }
}

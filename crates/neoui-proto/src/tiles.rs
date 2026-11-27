// SPDX-License-Identifier: Apache-2.0
//! The control centre's tiles (top-panel TP7, protocol 1.22; docs neoui-shell/protocol.md "TP7 tiles"): an app
//! offers up to four tiles with `ipc::TILE` ([`Offer`]); a tap or hold comes back as `ipc::AppEvent::Tile { id, hold
//! }` with the offer's local id. An offer never places a tile: the user does, in the editor. The order is
//! `shell.tiles` ([`decode_order`]); Settings lists every tile with INFO 12 ([`Record`]) and opens the editor with
//! `ACTION_EDIT_TILES` (an empty request, [`edit_tiles_request`]).
//!
//! **Tile ids** (15 bits, [`Id`]): 1–255 are the shell's own ([`builtin`]); from 256 an app's, `256 + 4 × slot +
//! local_id`, where `slot` is a per-publisher number the shell hands out once and never gives to another app (so a
//! new app never inherits a removed one's place).

use crate::settings_app::wire::{R, W};
use alloc::string::String;
use alloc::vec::Vec;

/// [`Offer::kind`]: 0 withdraws the offer.
pub mod kind {
    pub const WITHDRAW: u8 = 0;
    pub const TOGGLE: u8 = 1;
    pub const ACTION: u8 = 2;
    /// Opens the app's `page`.
    pub const DETAIL: u8 = 3;
    pub(crate) const LAST: u8 = DETAIL;
}

/// The shell's own tiles. Only some are backed today ([`backed`]); the rest stay hidden, not drawn dead.
pub mod builtin {
    pub const WIFI: u16 = 1;
    pub const MOBILE: u16 = 2;
    pub const SILENT: u16 = 3;
    pub const DND: u16 = 4;
    pub const AIRPLANE: u16 = 5;
    pub const BATTERY_SAVER: u16 = 6;
    pub const DARK: u16 = 7;
    pub const BLUETOOTH: u16 = 8;
    pub const FLASHLIGHT: u16 = 9;
    pub const ROTATE: u16 = 10;
    pub const HOTSPOT: u16 = 11;
    pub const CAST: u16 = 12;
    pub const LOCATION: u16 = 13;
    pub const NFC: u16 = 14;
    pub const SCREEN_RECORD: u16 = 15;
    pub const NIGHT_LIGHT: u16 = 16;
    pub(crate) const LAST: u16 = NIGHT_LIGHT;
}

/// Whether built-in tile `id` has something behind it on this build.
#[must_use]
pub fn backed(id: u16) -> bool {
    use builtin::*;
    matches!(
        id,
        WIFI | MOBILE | SILENT | DND | AIRPLANE | BATTERY_SAVER | DARK
    )
}

/// The order of a fresh phone (an empty `shell.tiles`): Wi-Fi and Mobile wide, then the small ones.
pub const DEFAULT_ORDER: &[(u16, bool)] = &[
    (builtin::WIFI, true),
    (builtin::MOBILE, true),
    (builtin::SILENT, false),
    (builtin::DND, false),
    (builtin::AIRPLANE, false),
    (builtin::BATTERY_SAVER, false),
    (builtin::DARK, false),
];

/// Local ids per app: 0–3.
pub const LOCAL_IDS: u8 = 4;
/// The first app tile id.
pub const APP_FIRST: u16 = 256;
/// The highest id (15 bits).
pub const ID_MAX: u16 = 0x7fff;
/// The highest publisher slot.
pub const SLOT_MAX: u16 = (ID_MAX - APP_FIRST) / LOCAL_IDS as u16;
/// At most this many placed tiles.
pub const PLACED_MAX: usize = 32;
/// `shell.tiles`'s high bit: drawn wide.
pub const WIDE: u16 = 0x8000;
pub const LABEL_MAX: usize = 16;
pub const SUBTITLE_MAX: usize = 24;
/// The longest encoded [`Offer`].
pub const OFFER_MAX: usize = 4 + 2 + 2 + (1 + LABEL_MAX) + (1 + SUBTITLE_MAX);

/// A tile id split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    Builtin(u16),
    App { slot: u16, local: u8 },
}

impl Id {
    /// `None` for 0, an unknown built-in or past 15 bits.
    #[must_use]
    pub fn split(id: u16) -> Option<Self> {
        match id {
            1..=builtin::LAST => Some(Id::Builtin(id)),
            APP_FIRST..=ID_MAX => Some(Id::App {
                slot: (id - APP_FIRST) / u16::from(LOCAL_IDS),
                local: ((id - APP_FIRST) % u16::from(LOCAL_IDS)) as u8,
            }),
            _ => None,
        }
    }

    /// The wire id; `None` for a slot or local id out of range.
    #[must_use]
    pub fn join(self) -> Option<u16> {
        match self {
            Id::Builtin(n) => (1..=builtin::LAST).contains(&n).then_some(n),
            Id::App { slot, local } => (slot <= SLOT_MAX && local < LOCAL_IDS)
                .then(|| APP_FIRST + slot * u16::from(LOCAL_IDS) + u16::from(local)),
        }
    }
}

/// `shell.tiles`: each placed tile as `id | WIDE` (LE u16), in order. Ids are checked when decoded.
#[must_use]
pub fn encode_order(tiles: &[(u16, bool)]) -> Vec<u8> {
    tiles
        .iter()
        .flat_map(|&(id, wide)| (id | if wide { WIDE } else { 0 }).to_le_bytes())
        .collect()
}

/// The placed tiles of `shell.tiles` (empty: [`DEFAULT_ORDER`]); `None` for an odd length, more than 32, an invalid
/// or repeated id.
#[must_use]
pub fn decode_order(b: &[u8]) -> Option<Vec<(u16, bool)>> {
    if b.is_empty() {
        return Some(DEFAULT_ORDER.to_vec());
    }
    let (words, rest) = b.as_chunks::<2>();
    if !rest.is_empty() || words.len() > PLACED_MAX {
        return None;
    }
    let mut out: Vec<(u16, bool)> = Vec::with_capacity(words.len());
    for c in words {
        let w = u16::from_le_bytes(*c);
        let id = w & !WIDE;
        if Id::split(id).is_none() || out.iter().any(|t| t.0 == id) {
            return None;
        }
        out.push((id, w & WIDE != 0));
    }
    Some(out)
}

/// An app's tile (`ipc::TILE`): `1:b, local_id:b, kind:b, on:b, glyph:h, page:h, label:s16, subtitle:s24`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Offer {
    /// `0..LOCAL_IDS`.
    pub local_id: u8,
    /// One of [`kind`].
    pub kind: u8,
    /// A toggle's state (0 for the other kinds).
    pub on: bool,
    /// 0 = the app's own glyph; 1 stopwatch, 2 terminal, 3 shuffle, 4 phone; a generic glyph for numbers the shell
    /// doesn't know yet.
    pub glyph: u16,
    /// The page a Detail tile (and a hold on any tile) opens.
    pub page: u16,
    /// 1–16 bytes (empty only in a withdrawal).
    pub label: String,
    /// At most 24 bytes.
    pub subtitle: String,
}

impl Offer {
    /// The record that withdraws tile `local_id`.
    #[must_use]
    pub fn withdraw(local_id: u8) -> Self {
        Self {
            local_id,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.0.extend_from_slice(&[1, self.local_id, self.kind, u8::from(self.on)]);
        w.h(self.glyph);
        w.h(self.page);
        w.s(&self.label, LABEL_MAX);
        w.s(&self.subtitle, SUBTITLE_MAX);
        w.0
    }

    /// `None` for a malformed offer: another version, a local id past 3, an unknown kind, `on` that isn't 0 or 1 (or
    /// set on a non-toggle), an empty label on an offer, a withdrawal with anything else, a long or non-UTF-8 string,
    /// a short record or trailing bytes.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        if r.b()? != 1 {
            return None;
        }
        let o = Self {
            local_id: r.b().filter(|&l| l < LOCAL_IDS)?,
            kind: r.max(kind::LAST)?,
            on: r.max(1)? == 1,
            glyph: r.h()?,
            page: r.h()?,
            label: r.s(LABEL_MAX)?,
            subtitle: r.s(SUBTITLE_MAX)?,
        };
        r.end()?;
        let ok = if o.kind == kind::WITHDRAW {
            o == Self::withdraw(o.local_id)
        } else {
            !o.label.is_empty() && (o.kind == kind::TOGGLE || !o.on)
        };
        ok.then_some(o)
    }
}

/// INFO 12 `TILES` state flags.
pub mod state {
    pub const ON: u8 = 1;
    pub const BACKED: u8 = 2;
    pub const WIDE: u8 = 4;
    pub(crate) const ALL: u8 = 7;
}

/// `order` of a tile offered but not placed.
pub const NOT_PLACED: u16 = 0xffff;

/// One INFO 12 record: `global_id:h, owner:s16, local_id:b, kind:b, state_flags:b, glyph:h, page:h, order:h,
/// label:s16, subtitle:s24`. The placed tiles first in their order, then the offers by `(owner, local_id)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub id: u16,
    /// The publisher's app key; empty for the shell's own.
    pub owner: String,
    pub local_id: u8,
    pub kind: u8,
    /// [`state`] bits.
    pub state: u8,
    pub glyph: u16,
    pub page: u16,
    /// The place in `shell.tiles`, or [`NOT_PLACED`].
    pub order: u16,
    pub label: String,
    pub subtitle: String,
}

impl Record {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.h(self.id);
        w.s(&self.owner, crate::ipc::KEY_MAX);
        w.0.extend_from_slice(&[self.local_id, self.kind, self.state]);
        w.h(self.glyph);
        w.h(self.page);
        w.h(self.order);
        w.s(&self.label, LABEL_MAX);
        w.s(&self.subtitle, SUBTITLE_MAX);
        w.0
    }

    /// `None` for an invalid id, a built-in with an owner (or an app tile without one), a local id past 3, an unknown
    /// kind (a withdrawal is never listed), a reserved state bit, an order past 31 that isn't [`NOT_PLACED`], an empty
    /// label, a short record or trailing bytes.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        let rec = Self {
            id: r.h()?,
            owner: r.s(crate::ipc::KEY_MAX)?,
            local_id: r.b().filter(|&l| l < LOCAL_IDS)?,
            kind: r.b().filter(|k| (1..=kind::LAST).contains(k))?,
            state: r.max(state::ALL)?,
            glyph: r.h()?,
            page: r.h()?,
            order: r
                .h()
                .filter(|&o| o == NOT_PLACED || usize::from(o) < PLACED_MAX)?,
            label: r.s(LABEL_MAX).filter(|l| !l.is_empty())?,
            subtitle: r.s(SUBTITLE_MAX)?,
        };
        r.end()?;
        let ok = match Id::split(rec.id)? {
            Id::Builtin(_) => rec.owner.is_empty() && rec.local_id == 0,
            Id::App { local, .. } => !rec.owner.is_empty() && local == rec.local_id,
        };
        ok.then_some(rec)
    }
}

/// `ACTION_EDIT_TILES`: the request is empty (the editor needs nothing from Settings; `KErrArgument` otherwise).
#[must_use]
pub const fn edit_tiles_request(b: &[u8]) -> bool {
    b.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn ids_split_and_join() {
        assert_eq!(Id::split(builtin::WIFI), Some(Id::Builtin(1)));
        assert_eq!(Id::split(0), None);
        assert_eq!(Id::split(builtin::LAST + 1), None);
        assert_eq!(Id::split(255), None, "unassigned built-ins are invalid");
        assert_eq!(Id::split(APP_FIRST), Some(Id::App { slot: 0, local: 0 }));
        assert_eq!(Id::split(263), Some(Id::App { slot: 1, local: 3 }));
        assert_eq!(Id::split(ID_MAX + 1), None);
        for id in [1, 16, 256, 263, ID_MAX] {
            assert_eq!(Id::split(id).unwrap().join(), Some(id));
        }
        assert_eq!(Id::App { slot: 0, local: 4 }.join(), None);
        assert_eq!(
            Id::App {
                slot: SLOT_MAX + 1,
                local: 0
            }
            .join(),
            None
        );
        assert!(backed(builtin::DARK) && !backed(builtin::FLASHLIGHT));
    }

    #[test]
    fn the_order() {
        assert_eq!(decode_order(&[]), Some(DEFAULT_ORDER.to_vec()));
        let o = vec![(builtin::DND, true), (300, false), (builtin::WIFI, false)];
        let b = encode_order(&o);
        assert_eq!(b, [4, 0x80, 44, 1, 1, 0]);
        assert_eq!(decode_order(&b), Some(o));
        // odd, a repeat (wide or not), an invalid id, too many
        assert_eq!(decode_order(&[1]), None);
        assert_eq!(decode_order(&encode_order(&[(1, false), (1, true)])), None);
        assert_eq!(decode_order(&encode_order(&[(0, false)])), None);
        assert_eq!(decode_order(&encode_order(&[(200, false)])), None);
        let many: Vec<(u16, bool)> = (0..33).map(|i| (APP_FIRST + i, false)).collect();
        assert!(decode_order(&encode_order(&many[..32])).is_some());
        assert_eq!(decode_order(&encode_order(&many)), None);
    }

    fn stopwatch() -> Offer {
        Offer {
            local_id: 2,
            kind: kind::TOGGLE,
            on: true,
            glyph: 1,
            page: 3,
            label: "Stopwatch".into(),
            subtitle: "0:42".into(),
        }
    }

    #[test]
    fn offers() {
        for o in [
            stopwatch(),
            Offer::withdraw(3),
            Offer {
                kind: kind::DETAIL,
                on: false,
                ..stopwatch()
            },
        ] {
            assert_eq!(Offer::decode(&o.encode()), Some(o));
        }
        let good = stopwatch().encode();
        assert_eq!(good.len(), 4 + 4 + 10 + 5);
        for n in 0..good.len() {
            assert_eq!(Offer::decode(&good[..n]), None, "{n}");
        }
        let mut long = good.clone();
        long.push(0);
        assert_eq!(Offer::decode(&long), None);
        for (at, v) in [(0, 2), (1, 4), (2, 4), (3, 2)] {
            let mut b = good.clone();
            b[at] = v;
            assert_eq!(Offer::decode(&b), None, "byte {at} = {v}");
        }
        for bad in [
            Offer {
                label: String::new(),
                ..stopwatch()
            },
            Offer {
                kind: kind::ACTION,
                ..stopwatch()
            }, // on, not a toggle
            Offer {
                glyph: 1,
                ..Offer::withdraw(0)
            },
        ] {
            assert_eq!(Offer::decode(&bad.encode()), None, "{bad:?}");
        }
        let full = Offer {
            label: "l".repeat(20),
            subtitle: "s".repeat(30),
            ..stopwatch()
        };
        assert_eq!(full.encode().len(), OFFER_MAX);
    }

    #[test]
    fn info_records() {
        let wifi = Record {
            id: builtin::WIFI,
            owner: String::new(),
            local_id: 0,
            kind: kind::DETAIL,
            state: state::ON | state::BACKED | state::WIDE,
            glyph: 0,
            page: 0,
            order: 0,
            label: "Wi-Fi".into(),
            subtitle: "Home".into(),
        };
        let app = Record {
            id: 258,
            owner: "clock".into(),
            local_id: 2,
            kind: kind::TOGGLE,
            state: state::BACKED,
            order: NOT_PLACED,
            label: "Stopwatch".into(),
            ..wifi.clone()
        };
        for r in [wifi.clone(), app.clone()] {
            assert_eq!(Record::decode(&r.encode()), Some(r));
        }
        let good = app.encode();
        for n in 0..good.len() {
            assert_eq!(Record::decode(&good[..n]), None, "{n}");
        }
        for bad in [
            Record {
                owner: "clock".into(),
                ..wifi.clone()
            },
            Record {
                owner: String::new(),
                ..app.clone()
            },
            Record {
                local_id: 1,
                ..app.clone()
            }, // not the id's local
            Record {
                kind: kind::WITHDRAW,
                ..app.clone()
            },
            Record {
                state: 8,
                ..app.clone()
            },
            Record {
                order: 32,
                ..app.clone()
            },
            Record {
                id: 0,
                ..wifi.clone()
            },
        ] {
            assert_eq!(Record::decode(&bad.encode()), None, "{bad:?}");
        }
        assert!(edit_tiles_request(&[]) && !edit_tiles_request(&[0]));
    }
}

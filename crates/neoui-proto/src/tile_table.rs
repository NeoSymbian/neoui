// SPDX-License-Identifier: Apache-2.0
//! The control centre's state in the shell (top-panel TP7, docs neoui-shell/protocol.md "Slots and cold owners"):
//! - [`Slots`]: the persistent per-app slot table. An app's first offer gets the next slot, kept for good (never
//!   handed to another app), with the app's last offer per local id, so a placed tile still draws after a reboot
//!   while its app isn't running;
//! - [`drawn`]: what of `shell.tiles` is drawn (backed built-ins, offered app tiles);
//! - [`grid`] / [`index_at`] and the editor's edits ([`move_tile`], [`toggle_wide`], [`remove`], [`add`]);
//! - [`Held`]: taps on a tile whose app isn't running, held until it binds with minor ≥ 22;
//! - [`records`]: INFO 12.

use crate::ipc::AppEvent;
use crate::settings_app::wire::{R, W};
use crate::tiles::{
    self, APP_FIRST, Id, LOCAL_IDS, NOT_PLACED, Offer, PLACED_MAX, Record, builtin,
};
use alloc::string::String;
use alloc::vec::Vec;

/// The table's format version.
const VERSION: u8 = 1;
/// The control centre's grid: 4 cells a row, a wide tile takes 2.
pub const COLUMNS: u8 = 4;

/// One slot: the app and its last offer per local id (`None`: never offered, or withdrawn).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Owner {
    pub key: String,
    pub offers: [Option<Offer>; LOCAL_IDS as usize],
}

/// The slot table: slot `n` is `owners[n]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slots {
    owners: Vec<Owner>,
}

impl Slots {
    /// App `key`'s slot.
    #[must_use]
    pub fn slot(&self, key: &str) -> Option<u16> {
        self.owners
            .iter()
            .position(|o| o.key == key)
            .and_then(|i| u16::try_from(i).ok())
    }

    /// Takes app `key`'s offer (checked by the caller: its own, decoded); the tile id it is about, or `None` when no
    /// slot is left. A withdrawal of an app without a slot takes none.
    pub fn offer(&mut self, key: &str, o: Offer) -> Option<u16> {
        let slot = match self.slot(key) {
            Some(s) => s,
            None if o.kind == tiles::kind::WITHDRAW => return None,
            None => {
                let s = u16::try_from(self.owners.len())
                    .ok()
                    .filter(|&s| s <= tiles::SLOT_MAX)?;
                self.owners.push(Owner {
                    key: key.into(),
                    ..Owner::default()
                });
                s
            }
        };
        let local = o.local_id;
        let id = Id::App { slot, local }.join()?;
        self.owners[usize::from(slot)].offers[usize::from(local)] =
            (o.kind != tiles::kind::WITHDRAW).then_some(o);
        Some(id)
    }

    /// The app and its current offer behind app tile `id`.
    #[must_use]
    pub fn owner_of(&self, id: u16) -> Option<(&str, &Offer)> {
        let Some(Id::App { slot, local }) = Id::split(id) else {
            return None;
        };
        let o = self.owners.get(usize::from(slot))?;
        Some((&o.key, o.offers[usize::from(local)].as_ref()?))
    }

    /// Every current offer: (tile id, app key, offer), by slot then local id.
    pub fn offers(&self) -> impl Iterator<Item = (u16, &str, &Offer)> {
        self.owners.iter().enumerate().flat_map(|(s, o)| {
            o.offers.iter().flatten().filter_map(move |offer| {
                let slot = u16::try_from(s).ok()?;
                let id = Id::App {
                    slot,
                    local: offer.local_id,
                }
                .join()?;
                Some((id, o.key.as_str(), offer))
            })
        })
    }

    /// `VERSION:b, count:h`, then per slot `key:s16, mask:b` and, for each bit of `mask`, `len:b` + the offer.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.b(VERSION);
        w.h(self.owners.len() as u16);
        for o in &self.owners {
            w.s(&o.key, crate::ipc::KEY_MAX);
            let mask = o
                .offers
                .iter()
                .enumerate()
                .fold(0u8, |m, (i, x)| m | u8::from(x.is_some()) << i);
            w.b(mask);
            for x in o.offers.iter().flatten() {
                let b = x.encode();
                w.b(b.len() as u8);
                w.0.extend_from_slice(&b);
            }
        }
        w.0
    }

    /// `None` for another version, a damaged table, an offer under another local id than its place, a key twice.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        if r.b()? != VERSION {
            return None;
        }
        let n = r.h()?;
        let mut owners: Vec<Owner> = Vec::new();
        for _ in 0..n {
            let key = r.s(crate::ipc::KEY_MAX).filter(|k| !k.is_empty())?;
            if owners.iter().any(|o| o.key == key) {
                return None;
            }
            let mask = r.max((1 << LOCAL_IDS) - 1)?;
            let mut o = Owner {
                key,
                ..Owner::default()
            };
            for i in 0..LOCAL_IDS {
                if mask & 1 << i != 0 {
                    let len = r.b()?;
                    let offer = Offer::decode(r.take(usize::from(len))?)
                        .filter(|x| x.local_id == i && x.kind != tiles::kind::WITHDRAW)?;
                    o.offers[usize::from(i)] = Some(offer);
                }
            }
            owners.push(o);
        }
        r.end()?;
        Some(Self { owners })
    }
}

/// The stored order without app tiles whose slot isn't assigned (a lost table, or a value from elsewhere); `None` when
/// there are none. The shell writes it back at load: a slot handed out later never inherits a place in the value.
#[must_use]
pub fn prune(order: &[(u16, bool)], slots: &Slots) -> Option<Vec<(u16, bool)>> {
    let keep = |id: u16| match Id::split(id) {
        Some(Id::App { slot, .. }) => usize::from(slot) < slots.owners.len(),
        _ => true,
    };
    order
        .iter()
        .any(|t| !keep(t.0))
        .then(|| order.iter().copied().filter(|t| keep(t.0)).collect())
}

/// What of the stored order is drawn: the backed built-ins and the app tiles whose slot is assigned and whose app
/// offers that local id now. The rest stays out (and leaves the value at the editor's next save).
#[must_use]
pub fn drawn(order: &[(u16, bool)], slots: &Slots) -> Vec<(u16, bool)> {
    order
        .iter()
        .copied()
        .filter(|&(id, _)| match Id::split(id) {
            Some(Id::Builtin(b)) => tiles::backed(b),
            Some(Id::App { .. }) => slots.owner_of(id).is_some(),
            None => false,
        })
        .collect()
}

/// The first stop's tiles: the first four drawn.
#[must_use]
pub fn first_stop(drawn: &[(u16, bool)]) -> &[(u16, bool)] {
    &drawn[..drawn.len().min(4)]
}

/// Each tile's cell (column, row) on the grid: left to right, a wide tile that doesn't fit the row starts the next.
#[must_use]
pub fn grid(order: &[(u16, bool)]) -> Vec<(u8, u8)> {
    let (mut col, mut row) = (0u8, 0u8);
    order
        .iter()
        .map(|&(_, wide)| {
            let span = if wide { 2 } else { 1 };
            if col + span > COLUMNS {
                (col, row) = (0, row + 1);
            }
            let at = (col, row);
            col += span;
            if col >= COLUMNS {
                (col, row) = (0, row + 1);
            }
            at
        })
        .collect()
}

/// The place a tile dropped on cell (`col`, `row`) goes: the tile there, else the end.
#[must_use]
pub fn index_at(order: &[(u16, bool)], col: i32, row: i32) -> usize {
    grid(order)
        .iter()
        .zip(order)
        .position(|(&(c, r), &(_, wide))| {
            i32::from(r) == row && (i32::from(c)..i32::from(c) + 1 + i32::from(wide)).contains(&col)
        })
        .unwrap_or(order.len())
}

/// Tile `from` moves to place `to` (clamped).
pub fn move_tile(order: &mut Vec<(u16, bool)>, from: usize, to: usize) {
    if from < order.len() {
        let t = order.remove(from);
        order.insert(to.min(order.len()), t);
    }
}

/// Tile `i` turns wide or small.
pub fn toggle_wide(order: &mut [(u16, bool)], i: usize) {
    if let Some(t) = order.get_mut(i) {
        t.1 = !t.1;
    }
}

/// Removes tile `i`; false for the last one (the editor keeps at least one).
pub fn remove(order: &mut Vec<(u16, bool)>, i: usize) -> bool {
    let ok = order.len() > 1 && i < order.len();
    if ok {
        order.remove(i);
    }
    ok
}

/// Places tile `id` at the end (small); false when it is there already or the order is full.
pub fn add(order: &mut Vec<(u16, bool)>, id: u16) -> bool {
    let ok =
        order.len() < PLACED_MAX && Id::split(id).is_some() && !order.iter().any(|t| t.0 == id);
    if ok {
        order.push((id, false));
    }
    ok
}

/// What the editor offers to add: the backed built-ins and the app offers not in `order`.
#[must_use]
pub fn available(order: &[(u16, bool)], slots: &Slots) -> Vec<u16> {
    (1..=builtin::NIGHT_LIGHT)
        .filter(|&b| tiles::backed(b))
        .chain(slots.offers().map(|(id, _, _)| id))
        .filter(|id| !order.iter().any(|t| t.0 == *id))
        .collect()
}

/// A built-in tile's label.
#[must_use]
pub fn builtin_label(id: u16) -> &'static str {
    use builtin::*;
    match id {
        WIFI => "Wi-Fi",
        MOBILE => "Mobile data",
        SILENT => "Silent",
        DND => "Do not disturb",
        AIRPLANE => "Airplane",
        BATTERY_SAVER => "Battery saver",
        DARK => "Dark theme",
        BLUETOOTH => "Bluetooth",
        FLASHLIGHT => "Flashlight",
        ROTATE => "Auto-rotate",
        HOTSPOT => "Hotspot",
        CAST => "Cast",
        LOCATION => "Location",
        NFC => "NFC",
        SCREEN_RECORD => "Screen record",
        NIGHT_LIGHT => "Night light",
        _ => "",
    }
}

/// INFO 12: the drawn order, then the offers not placed (built-ins first, then by `(owner, local_id)`). `on(id)`
/// is a tile's state as the shell knows it (built-ins; an app toggle's comes with its offer).
#[must_use]
pub fn records(order: &[(u16, bool)], slots: &Slots, on: impl Fn(u16) -> bool) -> Vec<Record> {
    let record = |id: u16, wide: bool, place: u16| -> Option<Record> {
        let wide = if wide { tiles::state::WIDE } else { 0 };
        Some(match Id::split(id)? {
            Id::Builtin(b) => Record {
                id,
                kind: tiles::kind::TOGGLE,
                state: tiles::state::BACKED | wide | if on(b) { tiles::state::ON } else { 0 },
                order: place,
                label: builtin_label(b).into(),
                ..Record::default()
            },
            Id::App { local, .. } => {
                let (key, o) = slots.owner_of(id)?;
                Record {
                    id,
                    owner: key.into(),
                    local_id: local,
                    kind: o.kind,
                    state: tiles::state::BACKED | wide | if o.on { tiles::state::ON } else { 0 },
                    glyph: o.glyph,
                    page: o.page,
                    order: place,
                    label: o.label.clone(),
                    subtitle: o.subtitle.clone(),
                }
            }
        })
    };
    let mut out: Vec<Record> = order
        .iter()
        .enumerate()
        .filter_map(|(i, &(id, wide))| record(id, wide, i as u16))
        .collect();
    let mut rest: Vec<Record> = available(order, slots)
        .into_iter()
        .filter_map(|id| record(id, false, NOT_PLACED))
        .collect();
    rest.sort_by(|a, b| (&a.owner, a.local_id).cmp(&(&b.owner, b.local_id)));
    out.extend(rest);
    out
}

/// At most this many held taps (a tap more drops the oldest).
const HELD_MAX: usize = 8;
/// A held tap older than this is dropped (its app didn't come up).
pub const HELD_MS: u64 = 20_000;

/// Taps and holds on tiles whose app isn't running: held until it binds.
#[derive(Default, Debug)]
pub struct Held {
    q: Vec<(String, u8, bool, u64)>,
}

impl Held {
    /// A tap (`hold` false) or hold on app `key`'s tile `local`, at `now` ms.
    pub fn hold(&mut self, key: &str, local: u8, hold: bool, now: u64) {
        if self.q.len() >= HELD_MAX {
            self.q.remove(0);
        }
        self.q.push((key.into(), local, hold, now));
    }

    /// Whether anything is held for `key` (its app is being started).
    #[must_use]
    pub fn waiting(&self, key: &str) -> bool {
        self.q.iter().any(|h| h.0 == key)
    }

    /// App `key` bound with protocol minor `minor`: its held events, in order; none for an app before 1.22 (it can't
    /// read them). Either way they are gone.
    pub fn bound(&mut self, key: &str, minor: u8) -> Vec<AppEvent> {
        let mut out = Vec::new();
        self.q.retain(|(k, id, hold, _)| {
            if k != key {
                return true;
            }
            let e = AppEvent::Tile {
                id: *id,
                hold: *hold,
            };
            if minor >= e.needs_minor() {
                out.push(e);
            }
            false
        });
        out
    }

    /// Drops what waited longer than [`HELD_MS`] (the millisecond counter may wrap: a time ahead of `now` restarts).
    pub fn expire(&mut self, now: u64) {
        self.q
            .retain(|h| now < h.3 || now.saturating_sub(h.3) < HELD_MS);
    }
}

/// The first app tile id (for callers that only need to tell app tiles from built-ins).
#[must_use]
pub const fn is_app(id: u16) -> bool {
    id >= APP_FIRST
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn stopwatch(local: u8) -> Offer {
        Offer {
            local_id: local,
            kind: tiles::kind::TOGGLE,
            on: false,
            glyph: 1,
            page: 3,
            label: "Stopwatch".into(),
            subtitle: String::new(),
        }
    }

    #[test]
    fn slots_are_handed_out_once_and_kept() {
        let mut s = Slots::default();
        assert_eq!(
            s.offer("clock", Offer::withdraw(0)),
            None,
            "no slot for a withdrawal"
        );
        assert_eq!(s.offer("clock", stopwatch(2)), Some(258));
        assert_eq!(s.offer("music", stopwatch(0)), Some(260));
        // clock withdraws everything: its slot stays its own, music's doesn't move
        assert_eq!(s.offer("clock", Offer::withdraw(2)), Some(258));
        assert_eq!(s.owner_of(258), None);
        assert_eq!(s.slot("clock"), Some(0));
        assert_eq!(
            s.offer("files", stopwatch(1)),
            Some(265),
            "a new app gets a new slot"
        );
        assert_eq!(s.owner_of(260).map(|o| o.0), Some("music"));
        assert_eq!(s.offers().map(|o| o.0).collect::<Vec<_>>(), [260, 265]);
        // across a restart
        let b = s.encode();
        assert_eq!(Slots::decode(&b), Some(s.clone()));
        for n in 0..b.len() {
            assert_eq!(Slots::decode(&b[..n]), None, "{n}");
        }
        let mut long = b.clone();
        long.push(0);
        assert_eq!(Slots::decode(&long), None);
        assert_eq!(Slots::decode(&[2, 0, 0]), None);
        assert_eq!(Slots::decode(&[]), None);
    }

    /// protocol.md "Slots and cold owners": on load a tile whose slot isn't assigned, or whose local id the slot's
    /// owner doesn't offer, stays out of the drawn order and never reaches another app.
    #[test]
    fn tiles_slot_table_drops_unassigned_on_load() {
        let mut s = Slots::default();
        s.offer("clock", stopwatch(1)); // slot 0: 257
        let stored = tiles::encode_order(&[
            (builtin::DND, true),
            (257, false),                 // clock's, offered
            (256, false),                 // clock's local 0: never offered
            (264, false),                 // slot 2: unassigned
            (builtin::FLASHLIGHT, false), // not backed
            (builtin::WIFI, false),
        ])
        .unwrap();
        let order = tiles::decode_order(&stored).unwrap();
        let after_reboot = Slots::decode(&s.encode()).unwrap();
        assert_eq!(
            drawn(&order, &after_reboot),
            vec![(builtin::DND, true), (257, false), (builtin::WIFI, false)]
        );
        // the shell takes the unassigned ones out of the value at once, so a later app's slot never inherits them
        let pruned = prune(&order, &after_reboot).unwrap();
        assert!(!pruned.iter().any(|t| t.0 == 264));
        assert!(
            pruned.contains(&(256, false)),
            "an assigned slot keeps its place until the editor saves"
        );
        assert_eq!(prune(&pruned, &after_reboot), None);
        let mut later = after_reboot.clone();
        assert_eq!(later.offer("terminal", stopwatch(0)), Some(260));
        assert_eq!(later.offer("music", stopwatch(0)), Some(264));
        assert!(!drawn(&pruned, &later).iter().any(|t| t.0 == 264));
        let saved = drawn(&order, &after_reboot);
        assert_eq!(first_stop(&saved).len(), 3);
    }

    /// A tap on a tile whose app isn't running waits for it to bind; a 1.21 app gets nothing (it can't read it).
    #[test]
    fn tile_tap_waits_for_a_1_22_owner() {
        let mut h = Held::default();
        h.hold("clock", 2, false, 1000);
        h.hold("clock", 2, true, 1100);
        h.hold("music", 0, false, 1200);
        assert!(h.waiting("clock"));
        assert_eq!(
            h.bound("clock", 22),
            vec![
                AppEvent::Tile { id: 2, hold: false },
                AppEvent::Tile { id: 2, hold: true }
            ]
        );
        assert!(!h.waiting("clock"));
        assert_eq!(
            h.bound("music", 21),
            vec![],
            "a 1.21 owner gets no Tile event"
        );
        assert!(!h.waiting("music"));
        h.hold("files", 0, false, 1000);
        h.expire(1000 + HELD_MS - 1);
        assert!(h.waiting("files"));
        h.expire(1000 + HELD_MS);
        assert!(!h.waiting("files"));
        for i in 0..10 {
            h.hold("files", 0, false, i);
        }
        assert_eq!(h.bound("files", 22).len(), HELD_MAX);
    }

    #[test]
    fn the_grid_and_the_editor() {
        let order = tiles::DEFAULT_ORDER.to_vec();
        // Wi-Fi and Mobile wide on row 0, then 4 small, then Dark
        assert_eq!(
            grid(&order),
            vec![(0, 0), (2, 0), (0, 1), (1, 1), (2, 1), (3, 1), (0, 2)]
        );
        // a wide tile at column 3 starts the next row
        assert_eq!(
            grid(&[(3, false), (4, false), (5, false), (1, true)]),
            vec![(0, 0), (1, 0), (2, 0), (0, 1)]
        );
        assert_eq!(index_at(&order, 3, 0), 1, "the right half of Mobile");
        assert_eq!(index_at(&order, 2, 1), 4);
        assert_eq!(
            index_at(&order, 3, 2),
            order.len(),
            "an empty cell: the end"
        );
        let mut o = order.clone();
        move_tile(&mut o, 6, 0);
        assert_eq!(o[0], (builtin::DARK, false));
        move_tile(&mut o, 0, 99);
        assert_eq!(o, order);
        toggle_wide(&mut o, 0);
        assert_eq!(o[0], (builtin::WIFI, false));
        assert!(!add(&mut o, builtin::WIFI));
        assert!(!add(&mut o, 0));
        let mut one = vec![(builtin::DND, false)];
        assert!(!remove(&mut one, 0), "the editor keeps one");
        assert!(remove(&mut o, 0));
        assert_eq!(available(&o, &Slots::default()), vec![builtin::WIFI]);
        assert!(add(&mut o, builtin::WIFI));
        assert_eq!(o.last(), Some(&(builtin::WIFI, false)));
        assert!(is_app(256) && !is_app(7));
    }

    #[test]
    fn info_12_lists_the_order_then_the_offers() {
        let mut s = Slots::default();
        s.offer(
            "clock",
            Offer {
                on: true,
                ..stopwatch(2)
            },
        );
        let order = vec![(builtin::WIFI, true), (builtin::DND, false)];
        let r = records(&order, &s, |id| id == builtin::DND);
        let ids: Vec<(u16, u16)> = r.iter().map(|x| (x.id, x.order)).collect();
        assert_eq!(
            ids,
            vec![
                (1, 0),
                (4, 1),
                (2, NOT_PLACED),
                (3, NOT_PLACED),
                (5, NOT_PLACED),
                (6, NOT_PLACED),
                (7, NOT_PLACED),
                (258, NOT_PLACED)
            ]
        );
        assert_eq!(r[0].state, tiles::state::BACKED | tiles::state::WIDE);
        assert_eq!(r[1].state, tiles::state::BACKED | tiles::state::ON);
        assert_eq!(r[7].owner, "clock");
        assert_eq!(r[7].state, tiles::state::BACKED | tiles::state::ON);
        for x in &r {
            assert_eq!(Record::decode(&x.encode()).as_ref(), Some(x));
        }
    }
}

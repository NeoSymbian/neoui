// SPDX-License-Identifier: Apache-2.0
//! (TP7) The control centre's tiles in the shell (docs `neoui-shell/top-panel.md` "Tiles and the control centre"):
//! `shell.tiles` as stored, the apps' offers (the UI server's slot table, [`Renderer::set_tile_slots`]), the editor's
//! draft, and their projection into `TilesModel` (ui/components/tiles.slint). Only drawn tiles show
//! ([`tile_table::drawn`]: backed built-ins, offered app tiles). Done queues the order as a setting edit, like home's
//! editor; a tap or hold on an app's tile waits in [`Renderer::take_tile_taps`] for the UI server. Also the Wi-Fi
//! sheet's list ([`Renderer::set_wifi_networks`]).

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use neoui_proto::tile_table::{self, Slots};
use neoui_proto::tiles::{self, Id};
use neoui_proto::wifi::Network;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{Renderer, Shell, TileCell, TilesModel, WifiRow};

/// The settings key the editor saves to.
pub(crate) const KEY: &str = "shell.tiles";

#[derive(Default)]
pub(crate) struct State {
    /// `shell.tiles` as the store last showed it, and its order (the default for an empty or unreadable value)
    raw: Vec<u8>,
    order: Vec<(u16, bool)>,
    /// the UI server's slot table (the apps' offers) and the apps that can't take a tap (bound before 1.22)
    slots: Slots,
    stale: Vec<String>,
    /// the editor's order while it is open
    draft: Option<Vec<(u16, bool)>>,
    /// Done's bytes, until `take_setting_edits` takes them
    queued: Option<Vec<u8>>,
    /// taps (false) and holds (true) on app tiles, by tile id
    taps: Vec<(u16, bool)>,
    message: &'static str,
}

impl State {
    pub(crate) fn take_save(&mut self) -> Option<Vec<u8>> {
        self.queued.take()
    }
    pub(crate) fn saved_bytes(&self) -> Vec<u8> {
        self.raw.clone()
    }
}

fn cell(st: &State, id: u16, wide: bool, (col, row): (u8, u8)) -> Option<TileCell> {
    let mut c = TileCell {
        id: i32::from(id),
        wide,
        col: i32::from(col),
        row: i32::from(row),
        available: true,
        ..TileCell::default()
    };
    match Id::split(id)? {
        Id::Builtin(b) => c.label = tile_table::builtin_label(b).into(),
        Id::App { .. } => {
            let (key, o) = st.slots.owner_of(id)?;
            let name = neoui_proto::apps::find(key).map_or(key, |a| a.name);
            c.label = o.label.as_str().into();
            c.sub = o.subtitle.as_str().into();
            c.owner = name.to_uppercase().as_str().into();
            c.glyph = i32::from(o.glyph);
            c.kind = i32::from(o.kind);
            c.on = o.on;
            c.available = !st.stale.iter().any(|k| k == key);
        }
    }
    Some(c)
}

fn cells(st: &State, order: &[(u16, bool)]) -> Vec<TileCell> {
    order
        .iter()
        .zip(tile_table::grid(order))
        .filter_map(|(&(id, wide), at)| cell(st, id, wide, at))
        .collect()
}

fn rows(c: &[TileCell]) -> i32 {
    c.iter().map(|c| c.row + 1).max().unwrap_or(0)
}

/// Shows the order (the draft while editing) in `TilesModel`.
fn project(ui: &Shell, st: &State) {
    let g = ui.global::<TilesModel>();
    let saved = tile_table::drawn(&st.order, &st.slots);
    let shown = match &st.draft {
        Some(d) => tile_table::drawn(d, &st.slots),
        None => saved.clone(),
    };
    let placed = cells(st, &shown);
    g.set_rows(rows(&placed));
    g.set_placed(ModelRc::new(VecModel::from(placed)));
    g.set_first(ModelRc::new(VecModel::from(cells(
        st,
        tile_table::first_stop(&saved),
    ))));
    let spare: Vec<(u16, bool)> = tile_table::available(&shown, &st.slots)
        .into_iter()
        .map(|id| (id, false))
        .collect();
    let spare = cells(st, &spare);
    g.set_spare_rows(rows(&spare));
    g.set_spare(ModelRc::new(VecModel::from(spare)));
    g.set_editing(st.draft.is_some());
    g.set_message(st.message.into());
}

/// One `TilesModel.op`. `cell` is `row * 4 + col` for a drop (negative: below the grid, removed).
pub(crate) fn apply(st: &mut State, op: &str, a: i32, cell: i32) {
    let id = u16::try_from(a).unwrap_or(0);
    let i = usize::try_from(a).unwrap_or(usize::MAX);
    st.message = "";
    let one_left = "One tile always stays";
    match (op, st.draft.as_mut()) {
        ("tap", _) => st.taps.push((id, false)),
        ("hold", _) => st.taps.push((id, true)),
        ("edit", None) => st.draft = Some(tile_table::drawn(&st.order, &st.slots)),
        ("drop", Some(d)) if cell < 0 => {
            if !tile_table::remove(d, i) {
                st.message = one_left;
            }
        }
        ("drop", Some(d)) => {
            let cols = i32::from(tile_table::COLUMNS);
            let to = tile_table::index_at(d, cell % cols, cell / cols);
            tile_table::move_tile(d, i, to);
        }
        ("wide", Some(d)) => tile_table::toggle_wide(d, i),
        ("remove", Some(d)) => {
            if !tile_table::remove(d, i) {
                st.message = one_left;
            }
        }
        ("add", Some(d)) => {
            tile_table::add(d, id);
        }
        ("reset", Some(d)) => *d = tile_table::drawn(tiles::DEFAULT_ORDER, &st.slots),
        ("done", Some(_)) => {
            let d = st.draft.take().unwrap_or_default();
            // what isn't drawn any more leaves the value too (protocol.md "Slots and cold owners")
            if d != st.order
                && let Some(b) = tiles::encode_order(&d)
            {
                st.queued = Some(b.clone());
                st.raw = b;
                st.order = d;
            }
        }
        ("cancel", _) => st.draft = None,
        _ => {}
    }
}

/// The UI server's slot table. The editor's ops name places in the drawn draft, so a tile withdrawn while it is
/// open leaves the draft at once (else every place after it would name the next tile).
fn set_slots(st: &mut State, slots: &Slots, stale: &[&str]) {
    st.slots = slots.clone();
    st.stale = stale.iter().map(|k| String::from(*k)).collect();
    if let Some(d) = &mut st.draft {
        *d = tile_table::drawn(d, &st.slots);
    }
}

/// `shell.tiles` as stored: shown, and the editor's base.
pub(crate) fn stored(ui: &Shell, st: &mut State, bytes: &[u8]) {
    st.raw = bytes.to_vec();
    st.order = tiles::decode_order(bytes).unwrap_or_else(|| tiles::DEFAULT_ORDER.to_vec());
    project(ui, st);
}

/// Sets up `TilesModel` with the default order and answers its `op`s; `edited` hears of a queued save.
pub(crate) fn wire(ui: &Shell, edited: Rc<RefCell<Vec<String>>>) -> Rc<RefCell<State>> {
    let st: Rc<RefCell<State>> = Rc::default();
    st.borrow_mut().order = tiles::DEFAULT_ORDER.to_vec();
    project(ui, &st.borrow());
    let (state, weak) = (st.clone(), ui.as_weak());
    ui.global::<TilesModel>().on_op(move |op, a, b| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = state.borrow_mut();
        apply(&mut st, op.as_str(), a, b);
        if st.queued.is_some() {
            edited.borrow_mut().push(String::from(KEY));
        }
        project(&ui, &st);
    });
    st
}

/// The Wi-Fi tile's line: the joined network's name ("" when none).
pub(crate) fn wifi_sub(ui: &Shell, joined: &str) {
    ui.global::<TilesModel>().set_wifi_sub(joined.into());
}

/// The Mobile data tile's line: "Ucom · LTE" ("" without a network: "No service").
pub(crate) fn mobile_sub(ui: &Shell, bars: i32, operator: &str, rat: &str) {
    let sub = match (bars >= 0 && !operator.is_empty(), rat.is_empty()) {
        (false, _) => String::new(),
        (true, true) => String::from(operator),
        (true, false) => alloc::format!("{operator} · {rat}"),
    };
    ui.global::<TilesModel>()
        .set_mobile_sub(sub.as_str().into());
}

fn row(n: &Network, sub: &str) -> WifiRow {
    WifiRow {
        name: n.title().into(),
        sub: SharedString::from(sub),
        secured: n.secured,
        bars: i32::from(n.bars()),
    }
}

impl Renderer {
    /// The UI server's slot table (every app's current offers) and the apps that can't take a tap (bound before
    /// 1.22: their tiles show unavailable).
    pub fn set_tile_slots(&mut self, slots: &Slots, stale: &[&str]) {
        let mut st = self.tiles.borrow_mut();
        set_slots(&mut st, slots, stale);
        project(&self.ui, &st);
    }

    /// Taps (false) and holds (true) on app tiles since the last call, by tile id.
    pub fn take_tile_taps(&mut self) -> Vec<(u16, bool)> {
        core::mem::take(&mut self.tiles.borrow_mut().taps)
    }

    /// Opens the control centre (the shade's second stop).
    pub fn open_control(&mut self) {
        self.ui.global::<TilesModel>().set_stop(2);
        self.ui.set_shade_open(true);
    }

    /// The control centre (the second stop) is on screen.
    #[must_use]
    pub fn control_open(&self) -> bool {
        self.ui.get_shade_open() && self.ui.global::<TilesModel>().get_stop() == 2
    }

    /// Opens the tile editor over the control centre (Settings' `ACTION_EDIT_TILES`).
    pub fn edit_tiles(&mut self) {
        self.open_control();
        self.ui
            .global::<TilesModel>()
            .invoke_op("edit".into(), 0, 0);
    }

    /// The tile editor is open.
    #[must_use]
    pub fn tiles_editing(&self) -> bool {
        self.ui.global::<TilesModel>().get_editing()
    }

    /// The Wi-Fi sheet (tp-wifi) is open: the UI server scans while it is.
    #[must_use]
    pub fn wifi_sheet_open(&self) -> bool {
        self.ui.get_shade_open() && self.ui.global::<TilesModel>().get_wifi_sheet()
    }

    /// The Wi-Fi sheet's networks: the joined one, the others in range (each with whether WIFID remembers it), and
    /// whether a scan is running.
    pub fn set_wifi_networks(
        &mut self,
        joined: Option<&Network>,
        others: &[(&Network, bool)],
        scanning: bool,
    ) {
        let g = self.ui.global::<TilesModel>();
        g.set_joined(joined.map_or_else(WifiRow::default, |n| {
            row(n, &alloc::format!("Connected · {}", n.band()))
        }));
        let rows: Vec<WifiRow> = others
            .iter()
            .map(|(n, saved)| {
                row(
                    n,
                    if *saved {
                        "Saved"
                    } else if n.secured {
                        ""
                    } else {
                        "Open"
                    },
                )
            })
            .collect();
        g.set_networks(ModelRc::new(VecModel::from(rows)));
        g.set_scanning(scanning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use neoui_proto::tiles::builtin;

    #[test]
    fn the_editor_saves_once_and_keeps_one() {
        let mut st = State {
            order: tiles::DEFAULT_ORDER.to_vec(),
            ..State::default()
        };
        apply(&mut st, "edit", 0, 0);
        apply(&mut st, "done", 0, 0);
        assert_eq!(st.take_save(), None, "nothing changed: nothing saved");
        apply(&mut st, "edit", 0, 0);
        // Dark (place 6, cell (0, 2)) onto Wi-Fi's cell
        apply(&mut st, "drop", 6, 0);
        apply(&mut st, "wide", 0, 0);
        apply(&mut st, "drop", 1, -1); // Wi-Fi dragged below: removed
        apply(&mut st, "done", 0, 0);
        let saved = st.take_save().unwrap();
        assert_eq!(
            tiles::decode_order(&saved).unwrap()[..2],
            [(builtin::DARK, true), (builtin::MOBILE, true)]
        );
        assert_eq!(st.take_save(), None, "once");
        apply(&mut st, "edit", 0, 0);
        for _ in 0..6 {
            apply(&mut st, "remove", 0, 0);
        }
        assert_eq!(st.draft.as_ref().map(Vec::len), Some(1));
        apply(&mut st, "remove", 0, 0);
        assert_eq!(st.message, "One tile always stays");
        apply(&mut st, "add", i32::from(builtin::WIFI), 0);
        apply(&mut st, "reset", 0, 0);
        assert_eq!(st.draft.as_deref(), Some(tiles::DEFAULT_ORDER));
        apply(&mut st, "cancel", 0, 0);
        assert!(st.draft.is_none());
        apply(&mut st, "tap", 257, 0);
        apply(&mut st, "hold", 257, 0);
        assert_eq!(st.taps, [(257, false), (257, true)]);
    }

    /// The editor's ops come by place in the drawn draft: an app tile withdrawn while it is open leaves the draft, so
    /// the places after it still name the tiles on screen.
    #[test]
    fn a_tile_withdrawn_while_editing_leaves_the_draft() {
        let mut slots = Slots::default();
        let stopwatch = neoui_proto::tiles::Offer {
            local_id: 1,
            kind: tiles::kind::TOGGLE,
            label: "Stopwatch".into(),
            ..neoui_proto::tiles::Offer::default()
        };
        let id = slots.offer("clock", stopwatch).unwrap();
        let mut st = State {
            order: vec![(id, false), (builtin::WIFI, false), (builtin::DND, false)],
            slots: slots.clone(),
            ..State::default()
        };
        apply(&mut st, "edit", 0, 0);
        slots.offer("clock", neoui_proto::tiles::Offer::withdraw(1));
        set_slots(&mut st, &slots, &[]);
        apply(&mut st, "wide", 1, 0); // place 1 on screen: Do not disturb
        apply(&mut st, "done", 0, 0);
        assert_eq!(
            tiles::decode_order(&st.take_save().unwrap()).unwrap(),
            [(builtin::WIFI, false), (builtin::DND, true)]
        );
    }
}

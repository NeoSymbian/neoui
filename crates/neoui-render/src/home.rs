// SPDX-License-Identifier: Apache-2.0
//! (D10) Home's layout in the shell and its editor (docs `neoui-shell/home.md` Part 3): the saved layout
//! (`home.layout`, shown like any other setting), the editor's draft while edit mode is on, and the projection of
//! either into `HomeModel` (ui/screens/home-edit.slint). Done (or Back) trims empty pages at the end and queues the
//! bytes as a setting edit; the store's answer settles it: the same bytes back mean saved, anything else (the store
//! refused them) puts the editor back with its draft and says so. Also (G3) the switcher's thumbnails.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use neoui_proto::home::{self, CATALOGUE, Error, Layout, Widget};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{HomeCell, HomeModel, Shell};

/// The settings key the editor saves to.
pub(crate) const KEY: &str = "home.layout";

#[derive(Default)]
pub(crate) struct State {
    saved: Layout,
    /// `home.layout` as the store last showed it (empty: never saved, the default)
    raw: Vec<u8>,
    /// the editor's layout while edit mode is on (kept when Settings asks for the editor again)
    draft: Option<Layout>,
    /// Done's bytes and draft, until the store answers
    pending: Option<(Vec<u8>, Layout)>,
    /// Done queued a save: the next `take_setting_edits` carries it
    queued: bool,
}

impl State {
    pub(crate) fn editing(&self) -> bool {
        self.draft.is_some()
    }
    /// The bytes to store, once.
    pub(crate) fn take_save(&mut self) -> Option<Vec<u8>> {
        if !core::mem::take(&mut self.queued) {
            return None;
        }
        self.pending.as_ref().map(|(b, _)| b.clone())
    }
    pub(crate) fn saved_bytes(&self) -> Vec<u8> {
        self.raw.clone()
    }
}

fn message(e: Error) -> &'static str {
    match e {
        Error::Full => "No room on this page",
        Error::TooManyPages => "Home has 5 pages at most",
        Error::Invalid => "That can't go on home",
        Error::NotFound => "",
    }
}

fn cells(l: &Layout) -> Vec<HomeCell> {
    let cell = |page: usize, index: usize, it: &home::Item, at: &home::Placed| HomeCell {
        page: page as i32,
        index: index as i32,
        kind: i32::from(matches!(it, home::Item::Widget { .. })),
        key: it.app().into(),
        widget: match it {
            home::Item::Widget { widget, .. } => *widget as i32,
            home::Item::Shortcut(_) => 0,
        },
        col: i32::from(at.col),
        row: i32::from(at.row),
        w: i32::from(at.w),
        h: i32::from(at.h),
    };
    l.pages
        .iter()
        .enumerate()
        .flat_map(|(p, page)| {
            home::pack(&page.items)
                .unwrap_or_default()
                .into_iter()
                .map(move |at| cell(p, at.index, &page.items[at.index], &at))
        })
        .collect()
}

/// Shows `l` (the draft while editing) in `HomeModel`.
fn project(ui: &Shell, st: &State) {
    let l = st.draft.as_ref().unwrap_or(&st.saved);
    let g = ui.global::<HomeModel>();
    g.set_pages(l.pages.len() as i32);
    g.set_cells(ModelRc::new(VecModel::from(cells(l))));
    let lane: Vec<HomeCell> = home::pack_lane(l)
        .into_iter()
        .map(|(p, i, at)| {
            let mut c = cells(&Layout {
                pages: alloc::vec![home::Page {
                    items: alloc::vec![l.pages[p].items[i].clone()]
                }],
                dock: Default::default(),
            })
            .remove(0);
            (c.page, c.index, c.col, c.row) =
                (p as i32, i as i32, i32::from(at.col), i32::from(at.row));
            c
        })
        .collect();
    g.set_lane_rows(lane.iter().map(|c| c.row + c.h).max().unwrap_or(0));
    g.set_lane_cells(ModelRc::new(VecModel::from(lane)));
    let dock: Vec<SharedString> = l
        .dock
        .iter()
        .map(|k| k.as_deref().unwrap_or("").into())
        .collect();
    g.set_dock(ModelRc::new(VecModel::from(dock)));
    g.set_editing(st.editing());
}

/// Sets up `HomeModel` with the default layout and answers its `op`s; `edited` hears of a queued save.
pub(crate) fn wire(ui: &Shell, edited: Rc<RefCell<Vec<String>>>) -> Rc<RefCell<State>> {
    let st: Rc<RefCell<State>> = Rc::default();
    let g = ui.global::<HomeModel>();
    let names: Vec<SharedString> = CATALOGUE.iter().map(|e| e.3.into()).collect();
    g.set_widgets(ModelRc::new(VecModel::from(names)));
    project(ui, &st.borrow());
    let (state, weak) = (st.clone(), ui.as_weak());
    g.on_op(move |op, a, b, c, d, key| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = state.borrow_mut();
        let r = apply(&mut st, op.as_str(), [a, b, c, d], key.as_str());
        let g = ui.global::<HomeModel>();
        g.set_message(r.err().map_or("", message).into());
        if matches!(op.as_str(), "pin" | "widget" | "dock") && r.is_ok() {
            g.set_sheet(SharedString::new());
        }
        if st.queued {
            edited.borrow_mut().push(String::from(KEY));
        }
        project(&ui, &st);
    });
    st
}

fn idx(v: i32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// One editor operation (`HomeModel.op`) on the draft.
pub(crate) fn apply(
    st: &mut State,
    op: &str,
    [a, b, c, d]: [i32; 4],
    key: &str,
) -> Result<(), Error> {
    if op == "enter" {
        if st.draft.is_none() {
            st.draft = Some(st.saved.clone());
        }
        return Ok(());
    }
    let Some(l) = st.draft.as_mut() else {
        return Ok(()); // not editing: nothing to change
    };
    match op {
        "done" => {
            // untouched: nothing to save (not even the trim)
            if *l != st.saved {
                l.drop_empty_pages();
                // a draft that can't be encoded stays in the editor
                let bytes = l.encode()?;
                st.pending = Some((bytes, l.clone()));
                st.queued = true;
            }
            st.draft = None;
            Ok(())
        }
        "remove" => l.remove(idx(a), idx(b)),
        "drop" => {
            let (page, index) = (idx(a), idx(b));
            let items = &l.pages.get(page).ok_or(Error::NotFound)?.items;
            let to = home::drop_index(items, index, c.clamp(0, 3) as u8, d.clamp(0, 5) as u8);
            l.move_to(page, index, page, to)
        }
        "edge" => {
            let to = a
                .checked_add(c)
                .filter(|&t| t >= 0)
                .ok_or(Error::NotFound)?;
            l.move_to(idx(a), idx(b), idx(to), usize::MAX)
        }
        "pin" => l.pin_anywhere(key, idx(a)).map(drop),
        "widget" => {
            let w = CATALOGUE
                .iter()
                .find(|e| e.0 as i32 == b)
                .ok_or(Error::Invalid)?;
            let (w, size): (Widget, _) = (w.0, w.2[0]);
            let from = idx(a);
            let order = core::iter::once(from).chain((0..l.pages.len()).filter(|&p| p != from));
            let mut last = Err(Error::Full);
            for p in order {
                last = l.add_widget(w, size, p);
                if last.is_ok() {
                    break;
                }
            }
            last
        }
        "dock" => l.pin_dock((!key.is_empty()).then_some(key), idx(a)),
        _ => Ok(()),
    }
}

/// The store showed `bytes` for `home.layout`: what was saved (Done's bytes back), a refusal of Done's save (the
/// draft comes back with a message), or a value from elsewhere (a reset, the boot load).
pub(crate) fn stored(ui: &Shell, st: &mut State, bytes: &[u8]) {
    let g = ui.global::<HomeModel>();
    st.raw = bytes.to_vec();
    match st.pending.take() {
        Some((sent, draft)) if sent == bytes => {
            st.saved = draft;
        }
        Some((_, draft)) => {
            st.saved = Layout::load(bytes);
            st.draft = Some(draft);
            g.set_message("Couldn't save home: try Done again".into());
        }
        None => st.saved = Layout::load(bytes),
    }
    project(ui, st);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_saves_once_and_only_a_change() {
        let mut st = State::default();
        apply(&mut st, "enter", [0; 4], "").unwrap();
        assert!(st.editing());
        // nothing changed: Done just leaves
        apply(&mut st, "done", [0; 4], "").unwrap();
        assert!(!st.editing() && st.take_save().is_none());
        // a change: Done trims the empty pages and queues one save
        apply(&mut st, "enter", [0; 4], "").unwrap();
        apply(&mut st, "pin", [1, 0, 0, 0], "files").unwrap();
        // a repeated enter keeps the draft
        apply(&mut st, "enter", [0; 4], "").unwrap();
        assert_eq!(st.draft.as_ref().unwrap().pages[1].items.len(), 1);
        apply(&mut st, "done", [0; 4], "").unwrap();
        let bytes = st.take_save().unwrap();
        assert!(st.take_save().is_none());
        let saved = Layout::decode(&bytes).unwrap();
        assert_eq!(saved.pages.len(), 2);
        assert_eq!(st.pending.as_ref().unwrap().1, saved);
    }

    #[test]
    fn edits_report_why_not() {
        let mut st = State::default();
        // not editing: nothing changes
        apply(&mut st, "remove", [0, 0, 0, 0], "").unwrap();
        assert_eq!(st.saved, Layout::default());
        apply(&mut st, "enter", [0; 4], "").unwrap();
        // page 0 is full: an app goes to page 1, a widget to page 1 too
        apply(&mut st, "pin", [0, 0, 0, 0], "files").unwrap();
        apply(&mut st, "widget", [0, 4, 0, 0], "").unwrap();
        let d = st.draft.clone().unwrap();
        assert_eq!(d.pages[1].items.len(), 2);
        // the edge hold: page 1's first item to page 0 (full) is refused; to page 2 is fine; left of page 0 nothing
        assert_eq!(apply(&mut st, "edge", [1, 0, -1, 0], ""), Err(Error::Full));
        apply(&mut st, "edge", [1, 0, 1, 0], "").unwrap();
        assert_eq!(st.draft.as_ref().unwrap().pages[2].items.len(), 1);
        assert_eq!(
            apply(&mut st, "edge", [0, 0, -1, 0], ""),
            Err(Error::NotFound)
        );
        // a drop over page 0's first cell moves Music first
        apply(&mut st, "drop", [0, 3, 0, 0], "").unwrap();
        assert_eq!(
            st.draft.as_ref().unwrap().pages[0].items[0],
            home::Item::Widget {
                widget: Widget::Media,
                size: home::Size::Wide
            }
        );
        // the dock: replace slot 0, empty slot 3
        apply(&mut st, "dock", [0, 0, 0, 0], "files").unwrap();
        apply(&mut st, "dock", [3, 0, 0, 0], "").unwrap();
        let d = st.draft.as_ref().unwrap();
        assert_eq!(
            (d.dock[0].as_deref(), d.dock[3].as_deref()),
            (Some("files"), None)
        );
        assert_eq!(message(Error::Full), "No room on this page");
    }
}

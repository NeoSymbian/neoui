// SPDX-License-Identifier: Apache-2.0
//! (D10) Home's layout in the shell and its editor (docs `neoui-shell/home.md` Part 3): the saved layout
//! (`home.layout`, shown like any other setting), the editor's draft while edit mode is on, and the projection of
//! either into `HomeModel` (ui/screens/home-edit.slint). Done (or Back) trims empty pages at the end and queues the
//! bytes as a setting edit; the store's answer settles it: the same bytes back mean saved, anything else (the store
//! refused them) puts the editor back with its draft and says so. The Apps menu's long press (Add to home, Remove from
//! home) saves the same way at once, outside edit mode. Also (home.md Part 4) the apps' widget content, drawn in
//! their cells, and (G3) the switcher's thumbnails.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use neoui_proto::home::{self, CATALOGUE, Error, Layout, Size};
use neoui_proto::widget::{self, Body, Content};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::{AddWidget, HomeCell, HomeModel, HomePin, Shell, WidgetContent, WidgetPerson};

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
    /// (Part 4) the apps' widget content the store keeps (`widget.<app>.<name>`)
    content: Vec<(String, String, Content)>,
    /// (Part 4) app widgets tapped: the app and its content's page, until the server takes them
    opened: Vec<(String, u16)>,
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
    /// The app widgets tapped since the last call: (app, page).
    pub(crate) fn take_opened(&mut self) -> Vec<(String, u16)> {
        core::mem::take(&mut self.opened)
    }
    pub(crate) fn saved_bytes(&self) -> Vec<u8> {
        self.raw.clone()
    }
}

/// The layout an edit outside the editor starts from: Done's or the menu's last save while the store hasn't answered,
/// else the saved one.
fn latest(st: &State) -> Layout {
    st.pending
        .as_ref()
        .map_or_else(|| st.saved.clone(), |(_, l)| l.clone())
}

fn message(e: Error) -> &'static str {
    match e {
        Error::Full => "No room on this page",
        Error::TooManyPages => "Home has 5 pages at most",
        Error::Invalid => "That can't go on home",
        Error::NotFound => "",
    }
}

fn rgb(c: u32) -> Color {
    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// An app widget's content for `WidgetContent` (template 0: none yet, the label shows greyed).
fn shown(c: Option<&Content>) -> WidgetContent {
    let Some(c) = c else {
        return WidgetContent::default();
    };
    let (title, lines, value, people) = match &c.body {
        Body::Text { title, lines, .. } => (title.as_str(), lines.as_slice(), 0, &[][..]),
        Body::Count { value, label, .. } => (label.as_str(), &[][..], *value, &[][..]),
        Body::People(p) => ("", &[][..], 0, p.as_slice()),
    };
    let lines: Vec<SharedString> = lines.iter().map(|l| l.as_str().into()).collect();
    let people: Vec<WidgetPerson> = people
        .iter()
        .map(|p| WidgetPerson {
            initials: p.initials.as_str().into(),
            tint: rgb(p.tint),
            name: p.name.as_str().into(),
        })
        .collect();
    WidgetContent {
        template: c.body.template() as i32,
        title: title.into(),
        page: i32::from(c.page),
        lines: ModelRc::new(VecModel::from(lines)),
        value: alloc::format!("{value}").into(),
        people: ModelRc::new(VecModel::from(people)),
    }
}

fn cells(l: &Layout, content: &[(String, String, Content)]) -> Vec<HomeCell> {
    let cell = |page: usize, index: usize, it: &home::Item, at: &home::Placed| HomeCell {
        page: page as i32,
        index: index as i32,
        // 0 a shortcut, 1 a built-in widget, 2 an app's widget (v2; drawn with home.md Part 4)
        kind: match it {
            home::Item::Shortcut(_) => 0,
            home::Item::Widget { .. } => 1,
            home::Item::AppWidget { .. } => 2,
        },
        key: it.app().into(),
        widget: match it {
            home::Item::Widget { widget, .. } => *widget as i32,
            home::Item::Shortcut(_) | home::Item::AppWidget { .. } => 0,
        },
        col: i32::from(at.col),
        row: i32::from(at.row),
        w: i32::from(at.w),
        h: i32::from(at.h),
        label: match it {
            home::Item::Widget { widget, .. } => home::entry(*widget).3,
            home::Item::AppWidget { app, name, .. } => widget::find(app, name).map_or("", |e| e.4),
            home::Item::Shortcut(_) => "",
        }
        .into(),
        content: match it {
            home::Item::AppWidget { app, name, .. } => shown(
                content
                    .iter()
                    .find(|(a, n, _)| a == app && n == name)
                    .map(|c| &c.2),
            ),
            _ => WidgetContent::default(),
        },
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
    g.set_cells(ModelRc::new(VecModel::from(cells(l, &st.content))));
    let lane: Vec<HomeCell> = home::pack_lane(l)
        .into_iter()
        .map(|(p, i, at)| {
            let mut c = cells(
                &Layout {
                    pages: alloc::vec![home::Page {
                        items: alloc::vec![l.pages[p].items[i].clone()]
                    }],
                    dock: Default::default(),
                },
                &st.content,
            )
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
    g.set_widgets(ModelRc::new(VecModel::from(add_rows())));
    project(ui, &st.borrow());
    // the Apps menu's long press: Remove from home for an app with an icon on home
    let pin = ui.global::<HomePin>();
    let state = st.clone();
    pin.on_pinned(move |key| {
        let st = state.borrow();
        let l = st.draft.clone().unwrap_or_else(|| latest(&st));
        l.pages.iter().any(|p| {
            p.items
                .iter()
                .any(|i| matches!(i, home::Item::Shortcut(k) if *k == key.as_str()))
        })
    });
    let (state, weak, queued) = (st.clone(), ui.as_weak(), edited.clone());
    pin.on_op(move |op, key| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = state.borrow_mut();
        // Add to home: the page home shows (pin_anywhere tries the others after it)
        let r = apply(
            &mut st,
            op.as_str(),
            [ui.get_home_page(), 0, 0, 0],
            key.as_str(),
        );
        let note = match r {
            Ok(()) if op == "add-home" => "Added to home",
            Ok(()) => "Removed from home",
            Err(e) => message(e),
        };
        ui.global::<HomePin>().set_note(note.into());
        if st.queued {
            queued.borrow_mut().push(String::from(KEY));
        }
        project(&ui, &st);
    });
    let (state, weak) = (st.clone(), ui.as_weak());
    g.on_op(move |op, a, b, c, d, key| {
        let Some(ui) = weak.upgrade() else { return };
        let mut st = state.borrow_mut();
        let r = apply(&mut st, op.as_str(), [a, b, c, d], key.as_str());
        if op == "open" {
            return; // home's model is unchanged
        }
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

/// The Add sheet's Widgets tab: the shell's widgets, then the apps' (each with every size it comes in).
fn add_rows() -> Vec<AddWidget> {
    let sizes = |s: &[Size]| {
        let v: Vec<i32> = s.iter().map(|z| *z as i32).collect();
        ModelRc::new(VecModel::from(v))
    };
    CATALOGUE
        .iter()
        .map(|e| AddWidget {
            label: e.3.into(),
            widget: e.0 as i32,
            key: SharedString::new(),
            sizes: sizes(e.2),
        })
        .chain(widget::CATALOGUE.iter().map(|e| AddWidget {
            label: e.4.into(),
            widget: 0,
            key: alloc::format!("{}.{}", e.0, e.1).into(),
            sizes: sizes(e.3),
        }))
        .collect()
}

fn size_of(v: i32) -> Result<Size, Error> {
    Ok(match v {
        1 => Size::Wide,
        2 => Size::Tall,
        3 => Size::Square,
        _ => return Err(Error::Invalid),
    })
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
    if op == "open" {
        let page = u16::try_from(a).map_err(|_| Error::Invalid)?;
        st.opened.push((key.into(), page));
        return Ok(());
    }
    if op == "enter" {
        if st.draft.is_none() {
            st.draft = Some(latest(st));
        }
        return Ok(());
    }
    // the Apps menu's long press: into the draft while editing (Done saves it), else saved at once
    if matches!(op, "add-home" | "remove-home") {
        let mut l = st.draft.clone().unwrap_or_else(|| latest(st));
        if op == "add-home" {
            l.pin_anywhere(key, idx(a))?;
        } else {
            l.unpin(key);
        }
        if st.draft.is_some() {
            st.draft = Some(l);
        } else {
            st.pending = Some((l.encode()?, l));
            st.queued = true;
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
        // a widget at size `c`: the shell's widget `b`, or (b = 0) the app's widget `key` ("app.name")
        "widget" => {
            let size = size_of(c)?;
            let app = key.split_once('.');
            let w = CATALOGUE.iter().find(|e| e.0 as i32 == b).map(|e| e.0);
            if w.is_none() && app.is_none() {
                return Err(Error::Invalid);
            }
            let from = idx(a);
            let order = core::iter::once(from).chain((0..l.pages.len()).filter(|&p| p != from));
            let mut last = Err(Error::Full);
            for p in order {
                last = match (w, app) {
                    (Some(w), _) => l.add_widget(w, size, p),
                    (None, Some((app, name))) => l.add_app_widget(app, name, size, p),
                    (None, None) => Err(Error::Invalid),
                };
                if !matches!(last, Err(Error::Full)) {
                    break;
                }
            }
            last
        }
        "dock" => l.pin_dock((!key.is_empty()).then_some(key), idx(a)),
        _ => Ok(()),
    }
}

/// (Part 4) The content the store keeps for app `app`'s widget (`widget.<app>.<name>`), or none: home redraws its
/// cells.
pub(crate) fn set_content(ui: &Shell, st: &mut State, app: &str, name: &str, c: Option<Content>) {
    st.content.retain(|(a, n, _)| !(a == app && n == name));
    if let Some(c) = c {
        st.content.push((app.into(), name.into(), c));
    }
    project(ui, st);
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
    fn a_widget_tap_queues_its_page_once() {
        let mut st = State::default();
        apply(&mut st, "open", [7, 0, 0, 0], "kit").unwrap();
        assert_eq!(
            apply(&mut st, "open", [-1, 0, 0, 0], "kit"),
            Err(Error::Invalid)
        );
        assert_eq!(st.take_opened(), [("kit".into(), 7)]);
        assert!(st.take_opened().is_empty() && !st.queued && !st.editing());
    }

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
        apply(&mut st, "widget", [0, 4, 1, 0], "").unwrap();
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
                widget: home::Widget::Media,
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

    #[test]
    fn the_add_sheet_takes_a_size() {
        let mut st = State::default();
        apply(&mut st, "enter", [0; 4], "").unwrap();
        // Agenda at 2 × 2, Music at 2 × 2 beside it (both to page 1: page 0 is full); a size it lacks; no size
        apply(&mut st, "widget", [0, 2, 3, 0], "").unwrap();
        apply(&mut st, "widget", [1, 4, 3, 0], "").unwrap();
        assert_eq!(
            apply(&mut st, "widget", [1, 4, 2, 0], ""),
            Err(Error::Invalid)
        );
        assert_eq!(
            apply(&mut st, "widget", [1, 4, 0, 0], ""),
            Err(Error::Invalid)
        );
        // an app's widget by its key, at a size of its own
        apply(&mut st, "widget", [1, 0, 1, 0], "kit.demo").unwrap();
        assert_eq!(
            apply(&mut st, "widget", [1, 0, 2, 0], "kit.demo"),
            Err(Error::Invalid)
        );
        assert_eq!(
            apply(&mut st, "widget", [1, 0, 3, 0], "kit.nope"),
            Err(Error::Invalid)
        );
        let items = &st.draft.as_ref().unwrap().pages[1].items;
        assert_eq!(
            items[..2],
            [
                home::Item::Widget {
                    widget: home::Widget::Agenda,
                    size: Size::Square
                },
                home::Item::Widget {
                    widget: home::Widget::Media,
                    size: Size::Square
                }
            ]
        );
        assert_eq!(
            items[2],
            home::Item::AppWidget {
                app: "kit".into(),
                name: "demo".into(),
                size: Size::Wide
            }
        );
        let rows = add_rows();
        assert_eq!(rows.len(), CATALOGUE.len() + widget::CATALOGUE.len());
        assert!(rows.iter().any(|r| r.key == "kit.demo"));
    }

    #[test]
    fn add_to_home_from_the_menu_saves_at_once() {
        let mut st = State::default();
        // not editing: one save, page 0 full so page 1
        apply(&mut st, "add-home", [0; 4], "clock").unwrap();
        let bytes = st.take_save().unwrap();
        let l = Layout::decode(&bytes).unwrap();
        assert_eq!(l.pages[1].items, [home::Item::Shortcut("clock".into())]);
        assert!(!st.editing());
        // before the store answers, the next one builds on it
        apply(&mut st, "add-home", [0; 4], "files").unwrap();
        let l = Layout::decode(&st.take_save().unwrap()).unwrap();
        assert_eq!(l.pages[1].items.len(), 2);
        // Remove from home takes every icon of the app
        apply(&mut st, "remove-home", [0; 4], "clock").unwrap();
        let l = Layout::decode(&st.take_save().unwrap()).unwrap();
        assert_eq!(l.pages[1].items, [home::Item::Shortcut("files".into())]);
        // while editing it goes into the draft; Done saves
        apply(&mut st, "enter", [0; 4], "").unwrap();
        apply(&mut st, "add-home", [2, 0, 0, 0], "music").unwrap();
        assert!(st.take_save().is_none());
        assert_eq!(st.draft.as_ref().unwrap().pages[2].items.len(), 1);
        // a bad key changes nothing
        assert_eq!(apply(&mut st, "add-home", [0; 4], ""), Err(Error::Invalid));
    }
}

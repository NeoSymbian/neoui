// SPDX-License-Identifier: GPL-3.0-only
//! The Files app (`NEOFILES.EXE`, NeoSymbian R8): a neoui app process ([`neoapp`]) on Symbian's file server
//! ([`e32::fs`]). It shows drives with their free space, folders, and text files, and makes, renames and deletes.
//! It draws its own header with the kit (protocol 1.8): ← goes up a level, ⋮ has New folder, a long press picks rows
//! (Rename, Delete), and the shell asks before a delete and shows the toasts.
#![no_std]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use e32::dprintln;
use e32::fs::{self, File, Fs};
use neoapp::{App, AppEvent, Page, Query};
use neoui_files::{self as model, Item};
use neoui_proto::Rect;
use neoui_proto::ipc as proto;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

e32::main!(main);

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("files: failed {}", e);
            e
        }
    }
}

/// What the UI asked for (its callbacks only queue; the loop acts with the file server).
enum Action {
    Drive(usize),
    Row(usize),
    Toggle(usize),
    Up,
    NewFolder,
    Rename,
    Delete,
    ClearSelection,
    Refresh,
    FormCancel,
    FormSave,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Drives,
    Folder,
    Text,
}

/// The form page over the folder.
enum Form {
    NewFolder,
    Rename(Item),
}

/// The shell's dialog that is open, and what its first button does.
enum Ask {
    /// Leave the form without its name.
    Discard,
    Delete(Vec<Item>),
}

struct State {
    fs: Fs,
    view: View,
    /// Drive letters in card order.
    drives: Vec<char>,
    /// The open folder (`X:\dir\`) and its sorted entries, with which are picked.
    dir: String,
    items: Vec<Item>,
    checked: Vec<bool>,
    form: Option<Form>,
    asking: Option<Ask>,
    /// The page last described to the shell.
    page: Page,
}

fn run() -> Result<(), i32> {
    let app = App::connect()?;
    let ui = FilesApp::new().map_err(|_| -2)?;
    ui.global::<Theme>().set_dp(app.dp);
    let actions: Rc<RefCell<VecDeque<Action>>> = Rc::default();
    let queue = |a: fn(i32) -> Action| {
        let q = actions.clone();
        move |i: i32| q.borrow_mut().push_back(a(i))
    };
    ui.on_drive(queue(|i| Action::Drive(i as usize)));
    ui.on_open_row(queue(|i| Action::Row(i as usize)));
    ui.on_toggle(queue(|i| Action::Toggle(i as usize)));
    let plain = |a: fn() -> Action| {
        let q = actions.clone();
        move || q.borrow_mut().push_back(a())
    };
    ui.on_up(plain(|| Action::Up));
    ui.on_new_folder(plain(|| Action::NewFolder));
    ui.on_rename(plain(|| Action::Rename));
    ui.on_delete(plain(|| Action::Delete));
    ui.on_clear_selection(plain(|| Action::ClearSelection));
    ui.on_refresh(plain(|| Action::Refresh));
    ui.on_form_cancel(plain(|| Action::FormCancel));
    ui.on_form_save(plain(|| Action::FormSave));
    ui.show().map_err(|_| -2)?;
    let weak = ui.as_weak();
    app.set_scrolled(move || {
        weak.upgrade()
            .map_or(((0, 0), Rect::default()), |ui| scrolled(&ui))
    });

    let mut st = State {
        fs: Fs::connect()?,
        view: View::Drives,
        drives: Vec::new(),
        dir: String::new(),
        items: Vec::new(),
        checked: Vec::new(),
        form: None,
        asking: None,
        page: Page::default(),
    };
    show_drives(&app, &ui, &mut st);
    describe(&app, &ui, &mut st)?;
    app.draw()?;

    app.run(|e| {
        match e {
            AppEvent::Back if neoapp::kit_back!(ui) => {}
            AppEvent::Back => back(&app, &ui, &mut st),
            AppEvent::Answer { i } => answer(&app, &ui, &mut st, i == 0),
            AppEvent::Keyboard { h } => ui.set_kb_h(h as f32),
            _ => {}
        }
        loop {
            let next = actions.borrow_mut().pop_front();
            match next {
                Some(a) => act(&app, &ui, &mut st, a),
                None => break,
            }
        }
        describe(&app, &ui, &mut st)?;
        Ok(true)
    })?;
    dprintln!("files: quit");
    Ok(())
}

/// The page: Back stays in the app below the drive list, in a form, or with a kit layer (selection, ⋮) open.
fn describe(app: &App, ui: &FilesApp, st: &mut State) -> Result<(), i32> {
    let page = Page {
        title: "Files".into(),
        can_back: st.view != View::Drives
            || st.form.is_some()
            || ui.global::<Kit>().get_layers() > 0,
        ..Page::default()
    };
    if page != st.page {
        app.page(&page)?;
        st.page = page;
    }
    Ok(())
}

/// What the lists scrolled since the last frame (smooth-ui S2d), cleared for the next.
fn scrolled(ui: &FilesApp) -> ((i32, i32), Rect) {
    let sc = ui.global::<Scroll>();
    let moved = (sc.get_dy(), sc.get_lists());
    sc.set_dy(0);
    sc.set_lists(0);
    let list = Rect::new(
        sc.get_x() as i32,
        sc.get_y() as i32,
        sc.get_width() as u32,
        sc.get_height() as u32,
    );
    (moved, list)
}

fn act(app: &App, ui: &FilesApp, st: &mut State, a: Action) {
    match a {
        Action::Drive(i) => {
            if let Some(&d) = st.drives.get(i) {
                open_dir(app, ui, st, alloc::format!("{d}:\\"));
            }
        }
        Action::Row(i) => {
            let Some(item) = st.items.get(i).cloned() else {
                return;
            };
            if item.is_dir {
                open_dir(app, ui, st, model::join_dir(&st.dir, &item.name));
            } else {
                view_file(app, ui, st, &item);
            }
        }
        Action::Toggle(i) => {
            if let Some(c) = st.checked.get_mut(i) {
                *c = !*c;
                show_rows(ui, st);
            }
        }
        Action::ClearSelection => clear_selection(ui, st),
        Action::Up => back(app, ui, st),
        Action::NewFolder => open_form(app, ui, st, Form::NewFolder, ""),
        Action::Rename => {
            let picked = picked(st);
            if let [one] = picked.as_slice() {
                let name = one.name.clone();
                clear_selection(ui, st);
                open_form(app, ui, st, Form::Rename(picked[0].clone()), &name);
            }
        }
        Action::Delete => {
            let picked = picked(st);
            let text = match picked.as_slice() {
                [] => return,
                [one] => alloc::format!("Delete {}?", one.name),
                many => alloc::format!("Delete {} items?", many.len()),
            };
            ask(app, st, text, "Delete", Ask::Delete(picked));
        }
        Action::Refresh => {
            let dir = st.dir.clone();
            open_dir(app, ui, st, dir);
        }
        Action::FormCancel => cancel_form(app, ui, st),
        Action::FormSave => save_form(app, ui, st),
    }
}

/// Back: the form (asking when a name was typed), else up a level (the drive list doesn't keep Back, so the shell
/// closes the app). The kit's layers went first ([`neoapp::kit_back!`]).
fn back(app: &App, ui: &FilesApp, st: &mut State) {
    if st.form.is_some() {
        cancel_form(app, ui, st);
    } else if st.view == View::Text {
        let dir = st.dir.clone();
        open_dir(app, ui, st, dir);
    } else if let Some(up) = model::parent(&st.dir) {
        open_dir(app, ui, st, up);
    } else {
        show_drives(app, ui, st);
    }
}

fn picked(st: &State) -> Vec<Item> {
    st.items
        .iter()
        .zip(&st.checked)
        .filter(|(_, c)| **c)
        .map(|(i, _)| i.clone())
        .collect()
}

fn clear_selection(ui: &FilesApp, st: &mut State) {
    st.checked.iter_mut().for_each(|c| *c = false);
    show_rows(ui, st);
}

fn ask(app: &App, st: &mut State, text: String, yes: &str, what: Ask) {
    let q = Query {
        text,
        buttons: alloc::vec![yes.into(), "Cancel".into()],
        destructive: true,
    };
    if app.query(&q).is_ok() {
        st.asking = Some(what);
    }
}

/// The shell's dialog was answered (`yes`: its first button). Back answers the last, Cancel.
fn answer(app: &App, ui: &FilesApp, st: &mut State, yes: bool) {
    match st.asking.take() {
        Some(Ask::Discard) if yes => close_form(app, ui, st),
        Some(Ask::Delete(items)) if yes => {
            clear_selection(ui, st);
            let mut failed = None;
            for i in &items {
                let r = if i.is_dir {
                    st.fs.rmdir(&model::join_dir(&st.dir, &i.name))
                } else {
                    st.fs.delete(&model::join_file(&st.dir, &i.name))
                };
                dprintln!("files: delete {} {}", i.name, r.err().unwrap_or(0));
                failed = failed.or(r.err());
            }
            let dir = st.dir.clone();
            open_dir(app, ui, st, dir);
            match failed {
                Some(e) => toast(app, "", e),
                None if items.len() == 1 => note(app, &alloc::format!("{} deleted", items[0].name)),
                None => note(app, &alloc::format!("{} items deleted", items.len())),
            }
        }
        _ => {}
    }
}

fn show_drives(app: &App, ui: &FilesApp, st: &mut State) {
    let list = match st.fs.drives() {
        Ok(l) => l,
        Err(e) => return toast(app, "drives", e),
    };
    let mut rows = Vec::new();
    st.drives.clear();
    let mut log = String::new();
    for (i, &present) in list.iter().enumerate() {
        if present == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let Ok(v) = st.fs.volume(i as i32) else {
            continue; // not ready (no media)
        };
        let label = model::drive_label(letter, v.drive_att, v.media);
        rows.push(DriveRow {
            label: alloc::format!("{letter}: {label}").into(),
            detail: model::volume_text(v.size, v.free).into(),
            used: model::used_fraction(v.size, v.free),
            rom: v.drive_att & fs::drive_att::ROM != 0,
        });
        st.drives.push(letter);
        log.push_str(&alloc::format!(" {letter}({}/{})", v.free, v.size));
    }
    dprintln!("files: drives{}", log);
    ui.set_drives(ModelRc::from(Rc::new(VecModel::from(rows))));
    st.view = View::Drives;
    st.dir.clear();
    ui.set_view("drives".into());
}

fn open_dir(app: &App, ui: &FilesApp, st: &mut State, dir: String) {
    ui.set_refreshing(false);
    let entries = match st.fs.read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            dprintln!("files: list {} {}", dir, e);
            return toast(app, "", e);
        }
    };
    let mut items: Vec<Item> = entries
        .into_iter()
        .map(|e| Item {
            is_dir: e.is_dir(),
            name: e.name,
            size: e.size,
            modified: e.modified,
        })
        .collect();
    model::sort(&mut items);
    dprintln!("files: list {} {}", dir, items.len());
    st.checked = alloc::vec![false; items.len()];
    st.items = items;
    ui.set_path(dir.as_str().into());
    ui.set_name(dir_name(&dir).into());
    st.dir = dir;
    st.view = View::Folder;
    show_rows(ui, st);
    ui.set_view("folder".into());
}

/// The last part of `X:\a\b\` ("b"), or the drive ("X:").
fn dir_name(dir: &str) -> &str {
    let t = dir.trim_end_matches('\\');
    t.rsplit('\\').next().unwrap_or(t)
}

fn show_rows(ui: &FilesApp, st: &State) {
    let rows: Vec<FileRow> = st
        .items
        .iter()
        .zip(&st.checked)
        .map(|(i, c)| FileRow {
            name: i.name.as_str().into(),
            detail: model::detail_text(i).into(),
            kind: model::kind(&i.name, i.is_dir).into(),
            checked: *c,
        })
        .collect();
    ui.set_selected(st.checked.iter().filter(|c| **c).count() as i32);
    ui.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
}

fn view_file(app: &App, ui: &FilesApp, st: &mut State, item: &Item) {
    let path = model::join_file(&st.dir, &item.name);
    if model::kind(&item.name, false) != "text" {
        note(app, &alloc::format!("No viewer for {}", item.name));
        return;
    }
    let mut bytes = alloc::vec![0u8; model::PREVIEW_MAX + 1];
    let read = File::open(&st.fs, &path, fs::mode::SHARE_READERS_ONLY).and_then(|f| {
        let mut n = 0;
        while n < bytes.len() {
            let got = f.read_at(n as i32, &mut bytes[n..])?;
            if got == 0 {
                break;
            }
            n += got;
        }
        Ok(n)
    });
    let n = match read {
        Ok(n) => n,
        Err(e) => return toast(app, "", e),
    };
    dprintln!("files: view {} {}", path, n);
    ui.set_text(model::text_preview(&bytes[..n]).into());
    ui.set_path(path.as_str().into());
    ui.set_name(item.name.as_str().into());
    st.view = View::Text;
    ui.set_view("text".into());
}

fn open_form(app: &App, ui: &FilesApp, st: &mut State, form: Form, name: &str) {
    let kb = app.keyboard(proto::KEYBOARD_TEXT).unwrap_or(0);
    ui.set_kb_h(kb as f32);
    ui.set_name_text(name.into());
    ui.set_form(
        if matches!(form, Form::NewFolder) {
            "new"
        } else {
            "rename"
        }
        .into(),
    );
    st.form = Some(form);
}

/// ✕ or Back on the form: leave at once when nothing was typed, else ask (D14: the app asks, not the kit).
fn cancel_form(app: &App, ui: &FilesApp, st: &mut State) {
    let typed = ui.get_name_text();
    let unchanged = match &st.form {
        Some(Form::NewFolder) => typed.is_empty(),
        Some(Form::Rename(i)) => typed.as_str() == i.name,
        None => return,
    };
    if unchanged {
        close_form(app, ui, st);
    } else {
        ask(app, st, "Discard the name?".into(), "Discard", Ask::Discard);
    }
}

fn close_form(app: &App, ui: &FilesApp, st: &mut State) {
    st.form = None;
    let _ = app.keyboard(proto::KEYBOARD_HIDDEN);
    ui.set_kb_h(0.0);
    ui.set_form(SharedString::new());
}

/// ✓: make or rename, then show the folder again.
fn save_form(app: &App, ui: &FilesApp, st: &mut State) {
    let name = ui.get_name_text().trim().to_string();
    if !model::valid_name(&name) {
        return note(app, "That name can't be used");
    }
    let Some(form) = st.form.take() else { return };
    close_form(app, ui, st);
    let (what, path, r) = match form {
        Form::NewFolder => {
            let p = model::join_dir(&st.dir, &name);
            let r = st.fs.mkdir(&p);
            ("mkdir", p, r)
        }
        Form::Rename(i) => {
            let (from, to) = (
                model::join_file(&st.dir, &i.name),
                model::join_file(&st.dir, &name),
            );
            let r = st.fs.rename(&from, &to);
            ("rename", from, r)
        }
    };
    dprintln!("files: {} {} {}", what, path, r.err().unwrap_or(0));
    let dir = st.dir.clone();
    open_dir(app, ui, st, dir);
    if let Err(e) = r {
        toast(app, "", e);
    }
}

/// A short message as the shell's toast (NOTE).
fn note(app: &App, text: &str) {
    let mut t = String::from(text);
    while t.len() > neoui_proto::page::NOTE_TEXT_MAX {
        t.pop();
    }
    if let Err(e) = app.note(&t, "") {
        dprintln!("files: note {}", e);
    }
}

fn toast(app: &App, what: &str, code: i32) {
    let msg = model::error_text(code);
    if what.is_empty() {
        note(app, &msg);
    } else {
        note(app, &alloc::format!("{what}: {msg}"));
    }
}

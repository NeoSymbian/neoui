// SPDX-License-Identifier: GPL-3.0-only
//! The Files app (`NEOFILES.EXE`, NeoSymbian R8): a neoui app process ([`neoapp`]) on Symbian's file server
//! ([`e32::fs`]). It shows drives with their free space, folders, and text files, and makes, renames and deletes.
//! It claims the toolbar's Back (up a level; at the drive list Back closes it as usual) and Options (folder actions).
#![no_std]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;
use e32::dprintln;
use e32::fs::{self, File, Fs};
use e32::ipc::TIpcArgs;
use e32::sys::TPtr8;
use neoapp::App;
use neoui_files::{self as model, Item};
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::{InputEvent, Rect};
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
    Option(SharedString),
    OptionsDismissed,
    DialogDone(bool),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Drives,
    Folder,
    Text,
}

/// The next tap on a row picks it for this.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    None,
    Rename,
    Delete,
}

enum Dialog {
    NewFolder,
    Rename(Item),
    Delete(Item),
}

struct State {
    fs: Fs,
    view: View,
    /// Drive letters in card order.
    drives: Vec<char>,
    /// The open folder (`X:\dir\`) and its sorted entries.
    dir: String,
    items: Vec<Item>,
    pick: Pick,
    dialog: Option<Dialog>,
    options_open: bool,
    /// What the toolbar claim is now: (back, options).
    claims: (bool, bool),
}

const FOLDER_OPTIONS: [&str; 3] = ["New folder", "Rename", "Delete"];

fn run() -> Result<(), i32> {
    let app = App::connect()?;
    let ui = FilesApp::new().map_err(|_| -2)?;
    ui.global::<Theme>().set_dp(app.dp);
    ui.set_options(ModelRc::from(Rc::new(VecModel::from(
        FOLDER_OPTIONS
            .iter()
            .map(|s| SharedString::from(*s))
            .collect::<Vec<_>>(),
    ))));
    let actions: Rc<RefCell<VecDeque<Action>>> = Rc::default();
    let q = actions.clone();
    ui.on_drive(move |i| q.borrow_mut().push_back(Action::Drive(i as usize)));
    let q = actions.clone();
    ui.on_open_row(move |i| q.borrow_mut().push_back(Action::Row(i as usize)));
    let q = actions.clone();
    ui.on_option(move |s| q.borrow_mut().push_back(Action::Option(s)));
    let q = actions.clone();
    ui.on_options_dismissed(move || q.borrow_mut().push_back(Action::OptionsDismissed));
    let q = actions.clone();
    ui.on_dialog_done(move |ok| q.borrow_mut().push_back(Action::DialogDone(ok)));
    ui.show().map_err(|_| -2)?;

    let mut st = State {
        fs: Fs::connect()?,
        view: View::Drives,
        drives: Vec::new(),
        dir: String::new(),
        items: Vec::new(),
        pick: Pick::None,
        dialog: None,
        options_open: false,
        claims: (false, false),
    };
    show_drives(&ui, &mut st);
    app.draw()?;

    loop {
        let mut ev = [0u8; proto::EVENT_LEN];
        {
            let mut des = TPtr8::new(&mut ev);
            let mut args = TIpcArgs::new();
            args.des8(0, &mut des);
            app.server.send_receive(proto::NEXT_EVENT, &args)?;
        }
        match AppEvent::decode(&ev) {
            Some(AppEvent::Quit) => {
                dprintln!("files: quit");
                return Ok(());
            }
            Some(AppEvent::Back) => back(&app, &ui, &mut st),
            Some(AppEvent::Options) => {
                st.options_open = !st.options_open && st.view == View::Folder;
                ui.set_options_open(st.options_open);
            }
            Some(AppEvent::Input(InputEvent::Key { code, down: true })) => app.dispatch_key(code),
            Some(AppEvent::Input(touch @ InputEvent::Touch { .. })) => app.dispatch_touch(touch),
            _ => {}
        }
        loop {
            let next = actions.borrow_mut().pop_front();
            match next {
                Some(a) => act(&app, &ui, &mut st, a),
                None => break,
            }
        }
        let claims = (
            st.view != View::Drives || st.dialog.is_some(),
            st.view == View::Folder,
        );
        if claims != st.claims {
            app.claim_nav(claims.0, claims.1)?;
            st.claims = claims;
        }
        app.draw_scrolled(|| scrolled(&ui))?;
    }
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
    ui.set_toast(SharedString::new());
    match a {
        Action::Drive(i) => {
            if let Some(&d) = st.drives.get(i) {
                open_dir(ui, st, alloc::format!("{d}:\\"));
            }
        }
        Action::Row(i) => {
            let Some(item) = st.items.get(i).cloned() else {
                return;
            };
            match st.pick {
                Pick::Rename => {
                    ui.set_name_text(item.name.as_str().into());
                    open_dialog(app, ui, st, Dialog::Rename(item));
                }
                Pick::Delete => open_dialog(app, ui, st, Dialog::Delete(item)),
                Pick::None if item.is_dir => open_dir(ui, st, model::join_dir(&st.dir, &item.name)),
                Pick::None => view_file(ui, st, &item),
            }
            set_pick(ui, st, Pick::None);
        }
        Action::Option(s) => {
            st.options_open = false;
            ui.set_options_open(false);
            match s.as_str() {
                "New folder" => {
                    ui.set_name_text(SharedString::new());
                    open_dialog(app, ui, st, Dialog::NewFolder);
                }
                "Rename" => set_pick(ui, st, Pick::Rename),
                "Delete" => set_pick(ui, st, Pick::Delete),
                _ => {}
            }
        }
        Action::OptionsDismissed => {
            st.options_open = false;
            ui.set_options_open(false);
        }
        Action::DialogDone(ok) => {
            let Some(d) = st.dialog.take() else { return };
            close_dialog(app, ui);
            if ok {
                change(ui, st, d);
            }
        }
    }
}

/// Back: close what is open, else go up a level (the drive list doesn't claim Back, so the shell closes the app).
fn back(app: &App, ui: &FilesApp, st: &mut State) {
    if st.dialog.take().is_some() {
        close_dialog(app, ui);
    } else if st.options_open {
        st.options_open = false;
        ui.set_options_open(false);
    } else if st.pick != Pick::None {
        set_pick(ui, st, Pick::None);
    } else if st.view == View::Text {
        let dir = st.dir.clone();
        open_dir(ui, st, dir);
    } else if let Some(up) = model::parent(&st.dir) {
        open_dir(ui, st, up);
    } else {
        show_drives(ui, st);
    }
}

fn show_drives(ui: &FilesApp, st: &mut State) {
    let list = match st.fs.drives() {
        Ok(l) => l,
        Err(e) => return toast(ui, "drives", e),
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

fn open_dir(ui: &FilesApp, st: &mut State, dir: String) {
    let entries = match st.fs.read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            dprintln!("files: list {} {}", dir, e);
            return toast(ui, "", e);
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
    let rows: Vec<FileRow> = items
        .iter()
        .map(|i| FileRow {
            name: i.name.as_str().into(),
            detail: model::detail_text(i).into(),
            kind: model::kind(&i.name, i.is_dir).into(),
        })
        .collect();
    ui.set_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
    ui.set_path(dir.as_str().into());
    st.items = items;
    st.dir = dir;
    st.view = View::Folder;
    ui.set_view("folder".into());
}

fn view_file(ui: &FilesApp, st: &mut State, item: &Item) {
    let path = model::join_file(&st.dir, &item.name);
    if model::kind(&item.name, false) != "text" {
        ui.set_toast(alloc::format!("No viewer for {}", item.name).into());
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
        Err(e) => return toast(ui, "", e),
    };
    dprintln!("files: view {} {}", path, n);
    ui.set_text(model::text_preview(&bytes[..n]).into());
    ui.set_path(path.as_str().into());
    st.view = View::Text;
    ui.set_view("text".into());
}

fn set_pick(ui: &FilesApp, st: &mut State, p: Pick) {
    st.pick = p;
    ui.set_banner(
        match p {
            Pick::None => "",
            Pick::Rename => "Tap an item to rename · Back cancels",
            Pick::Delete => "Tap an item to delete · Back cancels",
        }
        .into(),
    );
}

fn open_dialog(app: &App, ui: &FilesApp, st: &mut State, d: Dialog) {
    let (kind, title, ok) = match &d {
        Dialog::NewFolder => ("name", "New folder".to_string(), "Create"),
        Dialog::Rename(i) => ("name", alloc::format!("Rename {}", i.name), "Rename"),
        Dialog::Delete(i) => ("confirm", alloc::format!("Delete {}?", i.name), "Delete"),
    };
    if kind == "name" {
        let kb = app.keyboard(true).unwrap_or(0);
        ui.set_kb_h(kb as f32);
    }
    ui.set_dialog_title(title.into());
    ui.set_dialog_ok(ok.into());
    ui.set_dialog(kind.into());
    st.dialog = Some(d);
}

fn close_dialog(app: &App, ui: &FilesApp) {
    if ui.get_dialog() == "name" {
        let _ = app.keyboard(false);
        ui.set_kb_h(0.0);
    }
    ui.set_dialog(SharedString::new());
}

/// Make, rename or delete, then show the folder again.
fn change(ui: &FilesApp, st: &mut State, d: Dialog) {
    let name = ui.get_name_text().trim().to_string();
    let (what, path, r) = match d {
        Dialog::NewFolder | Dialog::Rename(_) if !model::valid_name(&name) => {
            return ui.set_toast("That name can't be used".into());
        }
        Dialog::NewFolder => {
            let p = model::join_dir(&st.dir, &name);
            let r = st.fs.mkdir(&p);
            ("mkdir", p, r)
        }
        Dialog::Rename(i) => {
            let (from, to) = (
                model::join_file(&st.dir, &i.name),
                model::join_file(&st.dir, &name),
            );
            let r = st.fs.rename(&from, &to);
            ("rename", from, r)
        }
        Dialog::Delete(i) if i.is_dir => {
            let p = model::join_dir(&st.dir, &i.name);
            let r = st.fs.rmdir(&p);
            ("delete", p, r)
        }
        Dialog::Delete(i) => {
            let p = model::join_file(&st.dir, &i.name);
            let r = st.fs.delete(&p);
            ("delete", p, r)
        }
    };
    dprintln!("files: {} {} {}", what, path, r.err().unwrap_or(0));
    let dir = st.dir.clone();
    open_dir(ui, st, dir);
    if let Err(e) = r {
        toast(ui, "", e);
    }
}

fn toast(ui: &FilesApp, what: &str, code: i32) {
    let msg = model::error_text(code);
    ui.set_toast(
        if what.is_empty() {
            msg
        } else {
            alloc::format!("{what}: {msg}")
        }
        .into(),
    );
}

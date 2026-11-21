// SPDX-License-Identifier: GPL-3.0-only
//! NEOKIT.EXE: the neoui kit gallery on the device, and the smallest complete neoui app (docs `neoui-shell/kit.md`):
//! connect, show the UI, describe the page, then [`neoapp::App::run`]: input, resizes, Slint timers and drawing are
//! neoapp's; the app handles Back (through the kit's layers first) and what is its own. Its "Files" page shows how an
//! app uses Files' file picker and takes files from Open with (protocol 1.12: `open_file`, `result_file`, `arg_file`).
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::Cell;
use e32::dprintln;
use e32::fs::SharedFile;
use neoapp::open::{files, kit};
use neoapp::{App, AppEvent, Page};
use slint::ComponentHandle;

slint::include_modules!();

e32::main!(main);

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("kit: failed {}", e);
            e
        }
    }
}

/// The "Files" page's index in the header menu.
const FILES_PAGE: i32 = 9;
/// What the kit is waiting for from Files.
#[derive(Clone, Copy, PartialEq)]
enum Asked {
    Nothing,
    Pick,
    Save,
}

fn run() -> Result<(), i32> {
    let app = Rc::new(App::connect()?);
    let ui = KitApp::new().map_err(|_| -2)?;
    ui.global::<Theme>().set_dp(app.dp);
    let asked = Rc::new(Cell::new(Asked::Nothing));
    let demo = ui.global::<FileDemo>();
    let (a, q) = (app.clone(), asked.clone());
    demo.on_pick(move || {
        q.set(Asked::Pick);
        let _ = a.open(files::KEY, files::PICK, "");
    });
    let (a, q) = (app.clone(), asked.clone());
    demo.on_save(move || {
        q.set(Asked::Save);
        let _ = a.open(files::KEY, files::SAVE, "note.txt");
    });
    ui.show().map_err(|_| -2)?;
    let mut shown = Page::default();
    describe(&app, &ui, &mut shown)?;
    app.draw()?;
    app.run(|e| {
        match e {
            // Back closes the top kit layer (the ⋮ menu, a sheet); with none open the page's can_back is false and
            // Back leaves the app (the shell's)
            AppEvent::Back => {
                neoapp::kit_back!(ui);
            }
            AppEvent::Page { page } => ui.global::<GalleryState>().set_page(i32::from(page)),
            // Open with: another app (Files) opened a file here
            AppEvent::Open { page } if page == kit::VIEW => {
                ui.global::<GalleryState>().set_page(FILES_PAGE);
                match app.arg_file() {
                    Some(f) => show(&ui, &f, "opened", "Opened here with Open with."),
                    None => ui
                        .global::<FileDemo>()
                        .set_status("Opened without a file.".into()),
                }
            }
            // back from Files: what it answered
            AppEvent::Visible(true) if asked.get() != Asked::Nothing => {
                let was = asked.replace(Asked::Nothing);
                match app.result_file() {
                    Some(f) if was == Asked::Save => {
                        let r = f.file().write(b"hello from kit\n");
                        let status = match r {
                            Ok(()) => "Saved through Files.",
                            Err(_) => "Files gave a file, but writing it failed.",
                        };
                        show(&ui, &f, "saved", status);
                    }
                    Some(f) => show(&ui, &f, "picked", "Picked in Files."),
                    None => ui
                        .global::<FileDemo>()
                        .set_status("Cancelled in Files.".into()),
                }
            }
            _ => {}
        }
        describe(&app, &ui, &mut shown)?;
        Ok(true)
    })
}

/// The page: Back stays in the app while a kit layer is open.
fn describe(app: &App, ui: &KitApp, shown: &mut Page) -> Result<(), i32> {
    let page = Page {
        title: "UI kit".into(),
        can_back: ui.global::<Kit>().get_layers() > 0,
        ..Page::default()
    };
    if page != *shown {
        app.page(&page)?;
        *shown = page;
    }
    Ok(())
}

/// Show a file the kit got: its full name, size and first line (at most 200 bytes, lossy UTF-8).
fn show(ui: &KitApp, f: &SharedFile, how: &str, status: &str) {
    let file = f.file();
    let name = file.full_name().unwrap_or_default();
    let size = file.size().unwrap_or(-1);
    let mut buf = [0u8; 200];
    let n = file.read_at(0, &mut buf).unwrap_or(0);
    let line = String::from_utf8_lossy(&buf[..n]);
    let line = line.lines().next().unwrap_or("");
    dprintln!(
        "kit: {} {} {} bytes, first line {:?}",
        how,
        name,
        size,
        line
    );
    let demo = ui.global::<FileDemo>();
    demo.set_status(status.into());
    demo.set_name(name.as_str().into());
    demo.set_size(format!("{size} bytes").into());
    demo.set_first_line(line.into());
}

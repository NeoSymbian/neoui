// SPDX-License-Identifier: GPL-3.0-only
//! The Terminal app (`NEOTERM.EXE`, NeoSymbian R6b): a neoui app process ([`neoapp`]) that is also the Symbian text
//! console server ([`console`], `!E32WindowServer`) and starts ESHELL, so ESHELL and the 2010 text programs run in
//! a window of the Rust UI. Keys come from the UI server (`NEXT_EVENT`): hardware keys and the system keyboard in its
//! terminal mode (Esc, Tab and the arrows in a row above the letters).
#![no_std]

extern crate alloc;

mod console;

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::pin::pin;
use e32::ipc::TIpcArgs;
use e32::process::Process;
use e32::rt::{self, Either};
use e32::server::{self, Server};
use e32::sys::TPtr8;
use e32::{dprintln, time};
use neoapp::App;
use neoui_proto::InputEvent;
use neoui_proto::ipc::{self as proto, AppEvent};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

e32::main!(main);

/// Longest a change waits before it is drawn (and the redraw cap while ESHELL writes).
const FRAME_US: i32 = 33_000;
const IDLE_US: i32 = 500_000;

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("term: failed {}", e);
            e
        }
    }
}

fn run() -> Result<(), i32> {
    let app = App::connect()?;
    // The phone has no keys: the system keyboard covers the bottom, and the grid fits above it.
    // shortcut: always shown, no hide toggle, and the grid keeps its start size (term.slint lifts it when the keyboard
    // grows); add a reflow when ESHELL can be told of a resize.
    let kb_h = app.keyboard(proto::KEYBOARD_TERMINAL)? as f32;
    let ui = TermApp::new().map_err(|_| -2)?;
    let dp = app.dp;
    ui.global::<Theme>().set_dp(dp);

    // The grid from the surface: DejaVu Sans Mono advances 1233/2048 em; 12 dp text keeps the prototype's columns
    // (it had Ubuntu Mono's 0.5 em at 14.5 dp).
    // shortcut: assumed glyph advance; measure it when Slint exposes text metrics.
    let font_px = round(12.0 * dp);
    let (cell_w, cell_h) = (font_px * 1233.0 / 2048.0, round(font_px * 1.45));
    let pad = round(8.0 * dp);
    let cols = ((app.width as f32 - 2.0 * pad) / cell_w) as usize;
    let rows = ((app.height as f32 - kb_h - 2.0 * pad) / cell_h) as usize;
    ui.set_font_px(font_px);
    ui.set_cell_w(cell_w);
    ui.set_cell_h(cell_h);
    ui.set_pad(pad);
    ui.set_kb_h(kb_h);
    let model = Rc::new(VecModel::from(
        (0..rows).map(|_| SharedString::new()).collect::<Vec<_>>(),
    ));
    ui.set_rows(ModelRc::from(model.clone()));
    ui.show().map_err(|_| -2)?;
    dprintln!(
        "term: console {}x{} in {}x{}",
        cols,
        rows,
        app.width,
        app.height
    );

    let mut con = console::Console::new(cols, rows);
    let server = Server::create(console::NAME)?;
    let eshell = Process::spawn("eshell", "/p")?;
    eshell.resume();
    dprintln!("term: eshell started");
    let mut eshell_running = true;
    let mut last_draw = 0u64;

    let mut receive = pin!(server.receive());
    rt::block_on(async {
        loop {
            let mut ev = [0u8; proto::EVENT_LEN];
            let got = {
                let mut des = TPtr8::new(&mut ev);
                let mut args = TIpcArgs::new();
                args.des8(0, &mut des);
                let mut next = pin!(app.server.send(proto::NEXT_EVENT, &args));
                loop {
                    let wait = if con.dirty { FRAME_US } else { IDLE_US };
                    match rt::select(
                        next.as_mut(),
                        rt::select(receive.as_mut(), time::delay(wait)),
                    )
                    .await
                    {
                        Either::Left(r) => break r,
                        Either::Right(Either::Left(msg)) => {
                            receive.set(server.receive());
                            let _ = server::dispatch(&mut con, msg?);
                        }
                        Either::Right(Either::Right(())) => {}
                    }
                    if eshell_running && eshell.exit_info().is_some() {
                        eshell_running = false;
                        dprintln!("term: eshell exited");
                    }
                    let now = time::millis();
                    if con.dirty && now.saturating_sub(last_draw) * 1000 >= FRAME_US as u64 {
                        show(&ui, &model, &mut con, eshell_running);
                        app.draw()?;
                        last_draw = now;
                    }
                }
            };
            got?;
            match AppEvent::decode(&ev) {
                Some(AppEvent::Quit) => {
                    dprintln!("term: quit");
                    // Held messages first: they die with the server handle, which is dropped before `con`.
                    con.close();
                    // shortcut: ESHELL (and anything it started) is killed with the terminal; no detach yet.
                    if eshell_running {
                        eshell.kill(0);
                    }
                    return Ok(());
                }
                Some(AppEvent::Input(InputEvent::Key { code, down: true })) => {
                    con.key(console_key(code))
                }
                Some(AppEvent::Keyboard { h }) => {
                    ui.set_kb_h(h as f32);
                    con.dirty = true;
                }
                _ => {}
            }
        }
    })
}

fn round(v: f32) -> f32 {
    (v + 0.5) as i32 as f32
}

/// A key from the UI server (Unicode, or Symbian's private-use codes for arrows) as a Symbian `TKeyCode`.
fn console_key(code: u32) -> i32 {
    match code {
        0x0A | 0x0D => 13, // EKeyEnter
        c => c as i32,
    }
}

/// Copy the top window into the UI (changed rows only), log finished lines, and clear the dirty flag.
fn show(
    ui: &TermApp,
    model: &VecModel<SharedString>,
    con: &mut console::Console,
    eshell_running: bool,
) {
    for w in con.windows.iter_mut() {
        for line in w.grid.take_lines() {
            // shortcut: every console line goes to the debug port too (CI checks it); rate-limit when trusted.
            dprintln!("term| {}", line);
        }
    }
    con.dirty = false;
    let Some(top) = con.top() else {
        let note = if eshell_running {
            ""
        } else {
            "[eshell exited]"
        };
        model.set_row_data(0, note.into());
        ui.set_cursor_on(false);
        return;
    };
    let (_, rows) = top.grid.size();
    for y in 0..rows.min(model.row_count()) {
        let text = SharedString::from(top.grid.row_string(y).as_str());
        if model.row_data(y).as_ref() != Some(&text) {
            model.set_row_data(y, text);
        }
    }
    let (cx, cy) = top.grid.cursor();
    ui.set_cursor_x(cx as i32);
    ui.set_cursor_y(cy as i32);
    ui.set_cursor_on(top.cursor_on);
}

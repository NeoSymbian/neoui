// SPDX-License-Identifier: GPL-3.0-only
//! The neoui launcher on the 2010 Symbian user side (NeoSymbian R5), started by estart as `SYSSTART.EXE` in place of
//! the window server. It owns the screen and the input:
//! - draws with `neoui-render` (Slint's software renderer) into a RAM frame, then copies the damaged rectangles to
//!   the frame buffer (`e32::display`; video RAM is uncached, and the renderer reads back the previous frame);
//! - registers as the window server and takes raw pointer/key events from the kernel (`e32::event`);
//! - runs apps as separate processes (R6, [`host`]): the `NeoUi` IPC server, a shared surface per app shown in the
//!   shell's body, input routed to the app, `Quit` when the shell navigates away;
//! - reports over the debug port (`neoui: ...` lines, checked by CI).
#![no_std]

extern crate alloc;

mod host;

use alloc::vec;
use core::pin::pin;
use e32::display::Display;
use e32::event::{EventHook, TRawEvent, raw};
use e32::rt::{self, Either};
use e32::server::{self, Server};
use e32::{dprintln, time};
use host::Host;
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::{Damage, InputEvent, Rect, TouchPhase};
use neoui_render::Renderer;

e32::main!(main);

/// Frame interval while an animation runs.
const FRAME_US: i32 = 33_000;
/// Wake-up interval otherwise (also how often a vanished app process is noticed).
const IDLE_US: i32 = 1_000_000;

fn now_ms() -> u64 {
    time::millis()
}

fn main() -> i32 {
    let display = match Display::current() {
        Ok(d) => d,
        Err(e) => {
            dprintln!("neoui: no 32-bit display ({})", e);
            return e;
        }
    };
    dprintln!(
        "neoui: display {}x{}, stride {}",
        display.width,
        display.height,
        display.stride
    );
    let hook = EventHook::capture();
    let (w, h) = (display.width, display.height);
    let mut ui = match Renderer::new(w as u32, h as u32, now_ms) {
        Ok(r) => r,
        Err(e) => {
            dprintln!("neoui: renderer failed: {:?}", e);
            return -2;
        }
    };
    let server = match Server::create(proto::SERVER) {
        Ok(s) => s,
        Err(e) => {
            dprintln!("neoui: cannot create the {} server ({})", proto::SERVER, e);
            return e;
        }
    };
    let mut frame = vec![0u32; w * h];

    let t0 = time::millis();
    let (px, _) = present(&mut ui, &mut frame, &display);
    dprintln!(
        "neoui: launcher shown ({} px drawn in {} ms)",
        px,
        time::millis() - t0
    );
    // the shell's design unit (theme.slint: min(w, h) / 360), in 1/256 px
    let dp256 = (w.min(h) * 256 / 360) as u32;
    let mut host = Host::new(ui.body(), dp256);

    rt::block_on(async {
        // Both stay outstanding across wake-ups, so no input event or message is lost to a cancel.
        let mut input = pin!(hook.next());
        let mut receive = pin!(server.receive());
        let mut screen = ui.screen();
        let mut down_at = None;
        let mut to_app = false;
        loop {
            let tick = if ui.has_active_animations() {
                FRAME_US
            } else {
                IDLE_US
            };
            let mut tap_end = false;
            match rt::select(
                rt::select(input.as_mut(), receive.as_mut()),
                time::delay(tick),
            )
            .await
            {
                Either::Left(Either::Left(ev)) => {
                    input.set(hook.next());
                    let ev = match ev {
                        Ok(ev) => ev,
                        Err(e) => {
                            dprintln!("neoui: input failed ({})", e);
                            return e;
                        }
                    };
                    let Some(event) = to_input(&ev) else { continue };
                    let app_on_screen = host.visible(&screen).is_some();
                    match event {
                        InputEvent::Touch { id, x, y, phase } => {
                            let body = host.body;
                            if phase == TouchPhase::Down {
                                down_at = Some((x, y));
                                // A touch belongs to the app from press to release if it starts in its body.
                                to_app = app_on_screen && contains(&body, x, y);
                            }
                            if phase == TouchPhase::Up {
                                dprintln!("neoui: tap {},{}", x, y);
                                tap_end = true;
                            }
                            if to_app {
                                let (x, y) = (x - body.x, y - body.y);
                                host.send(AppEvent::Input(InputEvent::Touch { id, x, y, phase }));
                            } else {
                                ui.dispatch(event);
                            }
                        }
                        InputEvent::Key { .. } if app_on_screen => {
                            host.send(AppEvent::Input(event))
                        }
                        InputEvent::Key { .. } => ui.dispatch(event),
                    }
                }
                Either::Left(Either::Right(msg)) => {
                    receive.set(server.receive());
                    match msg {
                        Ok(msg) => {
                            let _ = server::dispatch(&mut host, msg);
                        }
                        Err(e) => {
                            dprintln!("neoui: receive failed ({})", e);
                            return e;
                        }
                    }
                }
                Either::Right(()) => {}
            }

            // App lifecycle: the shell's screen decides which app runs.
            let mut repaint_all = false;
            if let Some(key) = host.reap() {
                if ui.screen() == key {
                    ui.show("home", "");
                }
                repaint_all = true;
            }
            let now = ui.screen();
            if now != screen {
                dprintln!("neoui: screen {}", now);
                if host.app.as_ref().is_some_and(|a| a.key != now) {
                    host.quit();
                    repaint_all = true;
                }
                if host.app.is_none() {
                    // The body depends on the screen (app screens have a toolbar), so take it at launch.
                    host.body = ui.body();
                    host.launch(&now);
                }
                screen = now;
            }

            let (px, damage) = present(&mut ui, &mut frame, &display);
            if repaint_all {
                display.blit(&frame, w, 0, 0, w, h);
            }
            compose(&mut host, &screen, &damage, &display);
            if tap_end {
                dprintln!("neoui: tap from {:?} redrew {} px", down_at.take(), px);
            }
        }
    })
}

fn contains(r: &Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w as i32 && y < r.y + r.h as i32
}

/// Put the foreground app's surface over the shell's body: all of it when the shell drew under it, else the
/// rectangles the app reported.
fn compose(host: &mut Host, screen: &str, shell_damage: &Damage, display: &Display) {
    let blits = core::mem::take(&mut host.blits);
    let body = host.body;
    let Some(surface) = host.visible(screen).and_then(|a| a.surface.as_ref()) else {
        return;
    };
    let (bw, bh) = (body.w as usize, body.h as usize);
    // SAFETY: the chunk is `bw * bh * 4` bytes, committed at creation and mapped here; the app only writes pixels.
    let src = unsafe { core::slice::from_raw_parts(surface.base().cast::<u32>(), bw * bh) };
    let to_screen = |r: Rect| {
        let (x, y) = (r.x.max(0) as usize, r.y.max(0) as usize);
        let at = (body.x as usize + x, body.y as usize + y);
        display.blit_to(src, bw, (x, y, r.w as usize, r.h as usize), at);
    };
    if shell_damage
        .rects()
        .iter()
        .any(|r| !r.intersection(&body).is_empty())
    {
        to_screen(Rect::new(0, 0, body.w, body.h));
    } else {
        blits.into_iter().for_each(to_screen);
    }
}

/// Render what changed and copy it to the screen; returns the number of pixels copied and the damage.
fn present(ui: &mut Renderer, frame: &mut [u32], display: &Display) -> (usize, Damage) {
    let damage: Damage = ui.render(frame, display.width);
    let px = damage
        .rects()
        .iter()
        .map(|r| {
            let (x, y) = (r.x.max(0) as usize, r.y.max(0) as usize);
            display.blit(frame, display.width, x, y, r.w as usize, r.h as usize);
            r.w as usize * r.h as usize
        })
        .sum();
    (px, damage)
}

/// A raw kernel event as neoui input: button 1 is the (single) touch; keys only where a character is obvious.
fn to_input(ev: &TRawEvent) -> Option<InputEvent> {
    let touch = |phase| {
        // Raw pointer events are screen pixels on every device (Syborg's driver scaled x wrongly until NeoSymbian's
        // adaptation fix, f3b63c6); the UI takes the display size from the HAL and nothing else.
        let (x, y) = ev.pos();
        Some(InputEvent::Touch { id: 0, x, y, phase })
    };
    let key = |down| {
        key_char(ev.scan_code()).map(|c| InputEvent::Key {
            code: c as u32,
            down,
        })
    };
    match ev.i_type {
        raw::EBUTTON1_DOWN => touch(TouchPhase::Down),
        raw::EPOINTER_MOVE => touch(TouchPhase::Move),
        raw::EBUTTON1_UP => touch(TouchPhase::Up),
        raw::EKEY_DOWN => key(true),
        raw::EKEY_UP => key(false),
        _ => None,
    }
}

/// `TStdScanCode` (`e32keys.h`) to a character: letters and digits are their ASCII codes; a few controls.
// shortcut: no modifiers, no layouts; a real keymap comes with text input (R6).
fn key_char(scan: i32) -> Option<char> {
    match scan {
        1 => Some('\u{8}'),  // EStdKeyBackspace
        2 => Some('\t'),     // EStdKeyTab
        3 => Some('\n'),     // EStdKeyEnter
        4 => Some('\u{1b}'), // EStdKeyEscape
        5 => Some(' '),      // EStdKeySpace
        0x30..=0x39 => char::from_u32(scan as u32),
        0x41..=0x5A => char::from_u32(scan as u32 + 0x20),
        _ => None,
    }
}

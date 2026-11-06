// SPDX-License-Identifier: GPL-3.0-only
//! The neoui launcher on the 2010 Symbian user side (NeoSymbian R5), started by estart as `SYSSTART.EXE` in place of
//! the window server. It owns the screen and the input:
//! - draws with `neoui-render` (Slint's software renderer) into a RAM frame, then copies the damaged rectangles to
//!   the frame buffer (`e32::display`; video RAM is uncached, and the renderer reads back the previous frame);
//! - registers as the window server and takes raw pointer/key events from the kernel (`e32::event`);
//! - reports over the debug port (`neoui: ...` lines, checked by CI).
#![no_std]

extern crate alloc;

use alloc::vec;
use e32::display::Display;
use e32::event::{EventHook, TRawEvent, raw};
use e32::rt::{self, Either};
use e32::{dprintln, time};
use neoui_proto::{Damage, InputEvent, TouchPhase};
use neoui_render::Renderer;

e32::main!(main);

/// Frame interval while an animation runs.
const FRAME_US: i32 = 33_000;

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
    let mut frame = vec![0u32; w * h];

    let t0 = time::millis();
    let px = present(&mut ui, &mut frame, &display);
    dprintln!(
        "neoui: launcher shown ({} px drawn in {} ms)",
        px,
        time::millis() - t0
    );

    rt::block_on(async {
        let mut down_at = None;
        loop {
            let next = if ui.has_active_animations() {
                rt::select(hook.next(), time::delay(FRAME_US)).await
            } else {
                Either::Left(hook.next().await)
            };
            let tap_end = match next {
                Either::Left(Ok(ev)) => match to_input(&ev) {
                    Some(input) => {
                        if let InputEvent::Touch { x, y, phase, .. } = input {
                            match phase {
                                TouchPhase::Down => down_at = Some((x, y)),
                                TouchPhase::Up => {
                                    dprintln!("neoui: tap {},{}", x, y);
                                }
                                TouchPhase::Move => {}
                            }
                        }
                        ui.dispatch(input);
                        matches!(
                            input,
                            InputEvent::Touch {
                                phase: TouchPhase::Up,
                                ..
                            }
                        )
                    }
                    None => false,
                },
                Either::Left(Err(e)) => {
                    dprintln!("neoui: input failed ({})", e);
                    return e;
                }
                Either::Right(()) => false,
            };
            let px = present(&mut ui, &mut frame, &display);
            if tap_end {
                dprintln!("neoui: tap from {:?} redrew {} px", down_at.take(), px);
            }
        }
    })
}

/// Render what changed and copy it to the screen; returns the number of pixels copied.
fn present(ui: &mut Renderer, frame: &mut [u32], display: &Display) -> usize {
    let damage: Damage = ui.render(frame, display.width);
    damage
        .rects()
        .iter()
        .map(|r| {
            let (x, y) = (r.x.max(0) as usize, r.y.max(0) as usize);
            display.blit(frame, display.width, x, y, r.w as usize, r.h as usize);
            r.w as usize * r.h as usize
        })
        .sum()
}

/// A raw kernel event as neoui input: button 1 is the (single) touch; keys only where a character is obvious.
fn to_input(ev: &TRawEvent) -> Option<InputEvent> {
    let touch = |phase| {
        let (x, y) = ev.pos();
        // The Syborg pointer driver scales QEMU's absolute 0..0x7FFF by width/0x6000 but height/0x8000
        // (`syborg_pointer.cpp:118-119`), so x arrives 4/3 too large.
        // shortcut: Syborg-only correction; the Pixel's touch driver (H4) reports real pixels.
        Some(InputEvent::Touch {
            id: 0,
            x: x * 0x6000 / 0x8000,
            y,
            phase,
        })
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

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

mod clock;
mod host;
mod perf;

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Poll, Waker};
use e32::display::Display;
use e32::event::{EventHook, TRawEvent, raw};
use e32::rt::{self, Either};
use e32::server::{self, Server};
use e32::{dprintln, time};
use host::Host;
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::{Damage, InputEvent, Rect, TouchPhase, coalesce_moves, next_deadline};
use neoui_render::Renderer;
use neoui_render::layer::{self, Transition};

e32::main!(main);

/// Frame period (60 fps): frames start on a 16.7 ms grid while there is something to draw (smooth-ui S1).
const FRAME_US: u64 = 16_667;
/// Wake-up interval otherwise (also how often a vanished app process is noticed).
const IDLE_US: u64 = 1_000_000;
/// A screen change's fade (ui/theme.slint `open`, Belle's app start).
const TRANSITION_MS: u32 = 250;
/// How long Power is held before the Power menu opens (H3a).
const POWER_HOLD_MS: u64 = 1000;
/// `EStdKeyDevice0` (`e32keys.h`): the Pixel's Power key (kernel `a64_keys.cpp`).
const SCAN_POWER: i32 = 0xa4;

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
    // the shell frame Slint renders into; the screen before a change, which a transition fades out (smooth-ui S2b);
    // two rows to compose a transition frame in
    let mut frame = vec![0u32; w * h];
    let mut old = vec![0u32; w * h];
    let mut rows = (vec![0u32; w], vec![0u32; w]);
    ui.set_server_transitions(true);
    // which kernel, ROM and neoui this is: in the log of every run, and in Settings -> About phone
    let build = neoapp::build::lines();
    for line in &build {
        dprintln!("neoui: build {}", line);
    }
    ui.set_build_info(&build);

    let t0 = time::millis();
    let mut perf = perf::Perf::new();
    let (px, _, render_us, copy_us) = present(&mut ui, &mut frame, &display, &perf, true);
    dprintln!(
        "neoui: launcher shown ({} px drawn in {} ms: render {} us, copy {} us)",
        px,
        time::millis() - t0,
        render_us,
        copy_us
    );
    // "this boot worked": the kernel marks the booted A/B slot successful, so ABL keeps it (NeoSymbian kernel
    // ENeoHalBootOk, EHalGroupVariant 0x4e06; docs/filesystem.md FS5b). -5 (not supported) on boards without slots.
    // SAFETY: the function takes no arguments; both pointers are ignored.
    let boot_ok = unsafe {
        e32::sys::ui::hal_function(1, 0x4e06, core::ptr::null_mut(), core::ptr::null_mut())
    };
    dprintln!("neoui: boot OK -> {}", boot_ok);
    // the shell's design unit (theme.slint: min(w, h) / 360), in 1/256 px
    let dp256 = (w.min(h) * 256 / 360) as u32;
    let mut host = Host::new(ui.body(), dp256, ui.keyboard_h());
    let mut keyboard_shown = false;
    let mut nav_shown = 0;
    let mut running_shown: Vec<&'static str> = Vec::new();
    let mut clock_shown = (u32::MAX, u32::MAX);
    let mut keeper = clock::Keeper::boot();
    // when Power went down, while it is held
    let mut power_down_at: Option<u64> = None;
    // the Power menu covers the whole screen: no app surface over it, every touch to the shell
    let mut power_menu = false;
    // a refused HAL query must not freeze the clock below, which would hold every frame back
    let tick_us = match time::tick_period_us() {
        0 => 1000,
        us => u64::from(us),
    };
    let now_us = || u64::from(time::ticks()) * tick_us;

    rt::block_on(async {
        // Both stay outstanding across wake-ups, so no input event or message is lost to a cancel.
        let mut input = pin!(hook.next());
        let mut receive = pin!(server.receive());
        let mut screen = ui.screen();
        let mut down_at = None;
        let mut to_app = false;
        // the press of the last release, for the frame that shows it (one frame may show several taps)
        let mut tap_end = None;
        // the next frame's start, and whether something waits for it
        let mut next_frame = 0u64;
        let mut due = false;
        let mut transition: Option<Transition> = None;
        loop {
            let wait = if due
                || transition.is_some()
                || ui.has_active_animations()
                || power_down_at.is_some()
            {
                next_frame.saturating_sub(now_us())
            } else {
                IDLE_US
            };
            match rt::select(
                rt::select(input.as_mut(), receive.as_mut()),
                time::delay(wait as i32),
            )
            .await
            {
                Either::Left(Either::Left(ev)) => {
                    // Take every event already queued and merge the moves: a drag costs one render per frame, not
                    // one per move, and the kernel's queue doesn't fill up and drop releases (smooth-ui S1).
                    let mut evs = Vec::new();
                    let mut next = Some(ev);
                    while let Some(ev) = next {
                        input.set(hook.next());
                        match ev {
                            // Power is the system's: held, it opens the Power menu (below)
                            Ok(ev)
                                if matches!(ev.i_type, raw::EKEY_DOWN | raw::EKEY_UP)
                                    && ev.scan_code() == SCAN_POWER =>
                            {
                                power_down_at = (ev.i_type == raw::EKEY_DOWN).then(time::millis);
                            }
                            Ok(ev) => evs.extend(to_input(&ev)),
                            Err(e) => {
                                dprintln!("neoui: input failed ({})", e);
                                return e;
                            }
                        }
                        next = poll_now(input.as_mut());
                    }
                    coalesce_moves(&mut evs);
                    for event in evs {
                        let app_on_screen = host.front().is_some();
                        match event {
                            InputEvent::Touch { id, x, y, phase } => {
                                let body = host.body;
                                if phase == TouchPhase::Down {
                                    down_at = Some((x, y));
                                    // A touch belongs to the app from press to release if it starts in its body.
                                    to_app = app_on_screen
                                        && !power_menu
                                        && contains(&host.app_area(), x, y);
                                }
                                if phase == TouchPhase::Up {
                                    let down = down_at.take();
                                    dprintln!("neoui: tap {},{} down {:?}", x, y, down);
                                    tap_end = Some(down);
                                }
                                if to_app {
                                    let (x, y) = (x - body.x, y - body.y);
                                    host.send(AppEvent::Input(InputEvent::Touch {
                                        id,
                                        x,
                                        y,
                                        phase,
                                    }));
                                } else {
                                    ui.dispatch(event);
                                    // the system keyboard types into the app like a hardware key
                                    for code in ui.take_typed() {
                                        host.send(AppEvent::Input(InputEvent::Key {
                                            code,
                                            down: true,
                                        }));
                                    }
                                    for b in ui.take_nav() {
                                        host.send(if b == proto::NAV_BACK {
                                            AppEvent::Back
                                        } else {
                                            AppEvent::Options
                                        });
                                    }
                                    for key in ui.take_closed() {
                                        dprintln!("neoui: app {} closed", key);
                                        host.close(&key, time::millis());
                                    }
                                }
                            }
                            InputEvent::Key { .. } if app_on_screen => {
                                host.send(AppEvent::Input(event))
                            }
                            InputEvent::Key { .. } => ui.dispatch(event),
                        }
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
            // Draw at the next frame's start, with everything that came in until then.
            let frame_at = now_us();
            if frame_at < next_frame {
                due = true;
                continue;
            }
            next_frame = next_deadline(next_frame, frame_at, FRAME_US);
            due = false;

            // App lifecycle: the shell's screen decides which app is in front; the others keep running (R10).
            let mut repaint_all = false;
            let ms = time::millis();
            for key in host.reap(ms) {
                if ui.screen() == key {
                    ui.show("home", "");
                }
                repaint_all = true;
            }
            let now = ui.screen();
            if now != screen {
                dprintln!("neoui: screen {}", now);
                // what is on screen now (the old app with it, before set_screen hides it) fades out
                snapshot(&host, &frame, &mut old, w);
                transition = Some(Transition::new(ms, TRANSITION_MS));
                // The body depends on the screen (app screens have a toolbar): take it before an app is shown.
                host.body = ui.body();
                host.set_screen(&now, ms);
                screen = now;
            }
            let running = host.running();
            if running != running_shown {
                ui.set_running(&running);
                running_shown = running;
            }

            let want_keyboard = host.front().is_some_and(|a| a.keyboard);
            if want_keyboard != keyboard_shown {
                ui.set_keyboard(want_keyboard);
                keyboard_shown = want_keyboard;
            }
            let want_nav = host.front().map_or(0, |a| a.nav);
            if want_nav != nav_shown {
                ui.set_app_nav(
                    want_nav & proto::NAV_BACK != 0,
                    want_nav & proto::NAV_OPTIONS != 0,
                );
                nav_shown = want_nav;
            }
            if power_down_at.is_some_and(|t| ms.saturating_sub(t) >= POWER_HOLD_MS) {
                power_down_at = None;
                dprintln!("neoui: power menu");
                ui.set_power_menu(true);
                power_menu = true;
            }
            let mut power = host.power.take();
            for a in ui.take_power() {
                ui.set_power_menu(false);
                power_menu = false;
                host.refresh = true; // the app's surface again, whole
                if a >= 0 {
                    power = Some(a);
                }
            }
            if let Some(a) = power {
                clock::power(a, &mut keeper);
            }
            let t = time::now();
            if (t.hour, t.minute) != clock_shown {
                if t.minute != clock_shown.1 {
                    keeper.check(); // once a minute: did someone set the time?
                }
                clock_shown = (t.hour, t.minute);
                let text = format!("{:02}:{:02}", t.hour, t.minute);
                dprintln!("neoui: clock {}", text);
                ui.set_clock(&text);
            }

            let fading = transition.is_some();
            let (px, damage, render_us, mut copy_us) =
                present(&mut ui, &mut frame, &display, &perf, !fading);
            match transition.and_then(|t| t.progress(time::millis())) {
                Some(e) => {
                    perf.frame(render_us, 0, 0, px);
                    let m = perf.mark();
                    fade(&old, &frame, &host, e, (w, h), &mut rows, &display);
                    perf.layer(perf.since(&m));
                }
                None => {
                    if transition.take().is_some() {
                        // the fade is over: the new screen as it is, the app's surface whole
                        repaint_all = true;
                        host.refresh = true;
                    }
                    if repaint_all {
                        let m = perf.mark();
                        display.blit(&frame, w, 0, 0, w, h);
                        copy_us += perf.since(&m);
                    }
                    let m = perf.mark();
                    let app_px = if power_menu {
                        0
                    } else {
                        compose(&mut host, &damage, &display)
                    };
                    perf.frame(render_us, copy_us, perf.since(&m), px + app_px);
                }
            }
            perf.idle();
            if let Some(down) = tap_end.take() {
                dprintln!("neoui: tap from {:?} redrew {} px", down, px);
            }
        }
    })
}

/// The future's output if it is ready without waiting. Two polls: an event request is made by the first and, when the
/// kernel had an event queued, is already complete at the second (`RequestEvent` delivers at once). That completion's
/// signal is left for `block_on`, which only polls once more for it.
fn poll_now<F: Future>(mut f: Pin<&mut F>) -> Option<F::Output> {
    let mut cx = Context::from_waker(Waker::noop());
    (0..2).find_map(|_| match f.as_mut().poll(&mut cx) {
        Poll::Ready(v) => Some(v),
        Poll::Pending => None,
    })
}

/// The front app's surface: its pixels (`body.w` per row) and the screen rectangle they show in.
fn front_surface(host: &Host) -> Option<(&[u32], usize, Rect)> {
    let body = host.body;
    let surface = host.front()?.surface.as_ref()?;
    // SAFETY: the chunk is `body.w * body.h * 4` bytes, committed at creation and mapped here; the app only writes
    // pixels.
    let src = unsafe {
        core::slice::from_raw_parts(
            surface.base().cast::<u32>(),
            body.w as usize * body.h as usize,
        )
    };
    Some((src, body.w as usize, host.app_area()))
}

/// Put row `y` of the front app's surface (if it shows on that row) into the screen row `row`.
fn app_row(app: Option<(&[u32], usize, Rect)>, y: usize, row: &mut [u32]) {
    let Some((src, stride, area)) = app else {
        return;
    };
    let (ax, ay) = (area.x.max(0) as usize, area.y.max(0) as usize);
    if y >= ay && y < ay + area.h as usize {
        let n = area.w as usize;
        row[ax..ax + n].copy_from_slice(&src[(y - ay) * stride..][..n]);
    }
}

/// The screen as it is: the shell frame with the front app's surface over its body (smooth-ui S2b).
fn snapshot(host: &Host, frame: &[u32], old: &mut [u32], w: usize) {
    old.copy_from_slice(frame);
    let app = front_surface(host);
    for (y, row) in old.chunks_exact_mut(w).enumerate() {
        app_row(app, y, row);
    }
}

/// One frame of a screen change (smooth-ui S2b), written to the screen row by row: the old screen fades out while
/// the new one (the shell frame with the front app over its body) fades in, its body rising 16 dp into place, as the
/// shell's `Slot` did. `e` is the eased progress, 0..=256.
fn fade(
    old: &[u32],
    frame: &[u32],
    host: &Host,
    e: u32,
    (w, h): (usize, usize),
    (new, out): &mut (Vec<u32>, Vec<u32>),
    display: &Display,
) {
    let (by, bh) = (host.body.y.max(0) as usize, host.body.h as usize);
    let lift = w.min(h) * 16 / 360 * (256 - e as usize) / 256;
    let app = front_surface(host);
    for y in 0..h {
        let sy = if y >= by && y < by + bh {
            y.saturating_sub(lift).max(by)
        } else {
            y
        };
        new.copy_from_slice(&frame[sy * w..][..w]);
        app_row(app, sy, new);
        layer::blend_row(out, &old[y * w..][..w], new, e);
        display.blit_to(out, w, (0, 0, w, 1), (0, y));
    }
}

fn contains(r: &Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w as i32 && y < r.y + r.h as i32
}

/// Put the foreground app's surface over the shell's body (less the open keyboard): all of it when the shell drew
/// under it, else the rectangles the app reported.
fn compose(host: &mut Host, shell_damage: &Damage, display: &Display) -> usize {
    let blits = core::mem::take(&mut host.blits);
    let body = host.body;
    let area = host.app_area();
    // the visible part of the surface, in surface coordinates
    let shown = Rect::new(0, 0, area.w, area.h);
    let refresh = core::mem::take(&mut host.refresh);
    let Some(surface) = host.front().and_then(|a| a.surface.as_ref()) else {
        return 0;
    };
    let (bw, bh) = (body.w as usize, body.h as usize);
    // SAFETY: the chunk is `bw * bh * 4` bytes, committed at creation and mapped here; the app only writes pixels.
    let src = unsafe { core::slice::from_raw_parts(surface.base().cast::<u32>(), bw * bh) };
    let mut px = 0;
    let mut to_screen = |r: Rect| {
        let r = r.intersection(&shown);
        if r.is_empty() {
            return;
        }
        px += r.w as usize * r.h as usize;
        let (x, y) = (r.x.max(0) as usize, r.y.max(0) as usize);
        let at = (body.x as usize + x, body.y as usize + y);
        display.blit_to(src, bw, (x, y, r.w as usize, r.h as usize), at);
    };
    if refresh
        || shell_damage
            .rects()
            .iter()
            .any(|r| !r.intersection(&area).is_empty())
    {
        to_screen(shown);
    } else {
        blits.into_iter().for_each(&mut to_screen);
    }
    px
}

/// Render what changed and copy it to the screen (unless a transition composes the screen); returns the pixels
/// rendered, the damage, and the render and copy times (µs).
fn present(
    ui: &mut Renderer,
    frame: &mut [u32],
    display: &Display,
    perf: &perf::Perf,
    to_screen: bool,
) -> (usize, Damage, u32, u32) {
    let m = perf.mark();
    let damage: Damage = ui.render(frame, display.width);
    let render_us = perf.since(&m);
    let m = perf.mark();
    let px = damage
        .rects()
        .iter()
        .map(|r| {
            let (x, y) = (r.x.max(0) as usize, r.y.max(0) as usize);
            if to_screen {
                display.blit(frame, display.width, x, y, r.w as usize, r.h as usize);
            }
            r.w as usize * r.h as usize
        })
        .sum();
    (px, damage, render_us, perf.since(&m))
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
        1 => Some('\u{8}'),     // EStdKeyBackspace
        2 => Some('\t'),        // EStdKeyTab
        3 => Some('\r'),        // EStdKeyEnter: EKeyEnter is 13
        4 => Some('\u{1b}'),    // EStdKeyEscape
        5 => Some(' '),         // EStdKeySpace
        0x0D => Some('\u{7f}'), // EStdKeyDelete
        // EStdKeyLeft/Right/Up/DownArrow -> EKeyLeftArrow.. (0xF807..0xF80A, Unicode private use)
        0x0E => Some('\u{F807}'),
        0x0F => Some('\u{F808}'),
        0x10 => Some('\u{F809}'),
        0x11 => Some('\u{F80A}'),
        0x79 => Some(','),
        0x7A => Some('.'),
        0x7B => Some('/'),
        0x7C => Some('\\'),
        0x7D => Some(';'),
        0x82 => Some('-'),
        0x83 => Some('='),
        0x30..=0x39 => char::from_u32(scan as u32),
        0x41..=0x5A => char::from_u32(scan as u32 + 0x20),
        _ => None,
    }
}

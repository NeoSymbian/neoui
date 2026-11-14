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

mod battery;
mod clock;
mod clockwork;
mod contacts;
mod host;
mod netclock;
mod perf;
mod phone;
mod screen;
mod settings;
mod usb_debug;
mod wifi;

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
use neoui_proto::settings::keys;
use neoui_proto::{Damage, InputEvent, Rect, TouchPhase, coalesce_moves, next_deadline};
use neoui_render::layer::{self, Slide, Transition};
use neoui_render::{HOME_PAGES, Renderer};

e32::main!(main);

/// Frame period (60 fps): frames start on a 16.7 ms grid while there is something to draw (smooth-ui S1).
const FRAME_US: u64 = 16_667;
/// Wake-up interval otherwise (also how often a vanished app process is noticed).
const IDLE_US: u64 = 1_000_000;
/// A screen change's fade (ui/theme.slint `open`, Belle's app start).
const TRANSITION_MS: u32 = 250;
/// A home page swipe settling onto a page or back (smooth-ui S2c).
const SWIPE_MS: u32 = 250;
/// How long Power is held before the Power menu opens (H3a).
const POWER_HOLD_MS: u64 = 1000;
/// `EHalGroupDisplay` and its `TDisplayHalFunction`s (`u32hal.h`): screen on/off and the panel's brightness (D2).
const HAL_DISPLAY: i32 = 4;
const DISPLAY_MAX_BRIGHTNESS: i32 = 12;
const DISPLAY_SET_BRIGHTNESS: i32 = 13;
const DISPLAY_SET_STATE: i32 = 20;
/// `EStdKeyDevice0` (`e32keys.h`): the Pixel's Power key (kernel `a64_keys.cpp`).
const SCAN_POWER: i32 = 0xa4;
/// `EStdKeyIncVolume` / `EStdKeyDecVolume`: during a call they set its volume (telephony C6)
const SCAN_VOL_UP: i32 = 0xa2;
const SCAN_VOL_DOWN: i32 = 0xa3;

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
    // the shell frame Slint renders into; the screen before a change, which a transition fades out (smooth-ui S2b:
    // a whole screen, 9 MB on the Pixel, so it is held only while a fade runs); two rows to compose a fade frame in
    let mut frame = vec![0u32; w * h];
    let mut old: Vec<u32> = Vec::new();
    let mut rows = (vec![0u32; w], vec![0u32; w]);
    ui.set_server_transitions(true);
    // the settings (docs/neoui-shell.md, Part 1), shown before the first frame
    let settings = settings::open();
    let mut mirror = settings::UiMirror::new(&mut ui, &settings);
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
    let mut phone = phone::Phone::start();
    // telephony C8: the contacts book, shown before the first frame
    let mut contacts = contacts::Contacts::load();
    contacts.show_list(&mut ui);
    // Settings -> Developer options -> USB debugging: fshell on a second USB port, on a worker thread (usb_debug.rs);
    // the UI only tells it the switch
    let usb_debugging =
        |s: &settings::Settings| s.get(keys::DEVELOPER_USB_DEBUGGING).unwrap_or(false);
    let mut usb_debug = match usb_debug::UsbDebug::start(usb_debugging(&settings)) {
        Ok(worker) => Some(worker),
        Err(e) => {
            dprintln!("neoui: USB debugging unavailable ({})", e);
            None
        }
    };
    // the shell's design unit (theme.slint: min(w, h) / 360), in 1/256 px
    let dp256 = (w.min(h) * 256 / 360) as u32;
    let mut host = Host::new(ui.body(), dp256, ui.keyboard_heights(), settings);
    phone.show_speed(&mut ui, &contacts, &host.settings);
    let mut keyboard_shown = 0;
    let mut nav_shown = 0;
    let mut running_shown: Vec<&'static str> = Vec::new();
    let mut clockwork = clockwork::Clockwork::boot();
    let mut netclock = netclock::NetClock::default();
    let mut battery = battery::Battery::boot();
    let mut wifi = wifi::Wifi::new();
    // when Power went down, while it is held
    let mut power_down_at: Option<u64> = None;
    // the screen (screen.rs): off after display.timeout, by a short Power press; on by Power or a ringing call
    let mut scr = screen::Screen::new(time::millis());
    let panel_max = display_max_brightness();
    let mut brightness_sent = -1;
    // the Power menu covers the whole screen: no app surface over it, every touch to the shell
    let mut power_menu = false;
    let mut overlay_shown = false;
    let mut front_shown = false; // the shell knows an app is in front // the shade or the Options menu over the app (last frame)
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
        let mut modem_update = pin!(phone::Watch::new(phone.present()).next());
        let mut screen = ui.screen();
        let mut down_at = None;
        let mut to_app = false;
        // the press of the last release, for the frame that shows it (one frame may show several taps)
        let mut tap_end = None;
        // the next frame's start, and whether something waits for it
        let mut next_frame = 0u64;
        let mut due = false;
        let mut transition: Option<Transition> = None;
        // smooth-ui S2c: a press on the home pages (where it went down), the server's page slide once it moves
        // sideways (with the page it started from), and a touch that comes while the slide settles (ignored)
        let mut home_press: Option<i32> = None;
        let mut swipe: Option<(Slide, i32)> = None;
        let mut swallow = false;
        let mut phone_modal = false;
        let (slop, turn) = ((8 * dp256 / 256) as i32, (60 * dp256 / 256) as i32);
        loop {
            // a dark screen draws nothing: no frame pacing for animations or a pending draw
            let wait = if scr.is_on()
                && (due || transition.is_some() || swipe.is_some() || ui.has_active_animations())
                || power_down_at.is_some()
            {
                next_frame.saturating_sub(now_us())
            } else if host.settings.get(keys::STATUS_SECONDS).unwrap_or(false) && scr.is_on() {
                // the status bar's seconds: wake at the next whole second
                let us = e32::sys::time_now().0.rem_euclid(1_000_000) as u64;
                1_000_000 - us
            } else {
                IDLE_US
            };
            // the phone's own timers (a note, the next tone, the call's ticking time)
            let wait = phone
                .next_wake_ms(time::millis(), &host.settings)
                .map_or(wait, |ms| wait.min(ms * 1000));
            match rt::select(
                rt::select(input.as_mut(), receive.as_mut()),
                rt::select(modem_update.as_mut(), time::delay(wait as i32)),
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
                                let now = time::millis();
                                let change = if ev.i_type == raw::EKEY_DOWN {
                                    let c = scr.power_down(now);
                                    // a press that woke the screen opens no Power menu
                                    power_down_at = (!scr.press_woke()).then_some(now);
                                    c
                                } else {
                                    let held = power_down_at
                                        .take()
                                        .map_or(POWER_HOLD_MS, |t| now.saturating_sub(t));
                                    scr.power_up(held, POWER_HOLD_MS)
                                };
                                if let Some(c) = change {
                                    set_screen(c, &mut host.refresh);
                                }
                            }
                            Ok(ev)
                                if ev.i_type == raw::EKEY_DOWN
                                    && matches!(ev.scan_code(), SCAN_VOL_UP | SCAN_VOL_DOWN)
                                    && phone.volume_key(ev.scan_code() == SCAN_VOL_UP) => {}
                            // a dark screen takes no touches or keys (the kernel's touchscreen sleeps too)
                            Ok(ev) if scr.is_on() => {
                                scr.activity(time::millis());
                                evs.extend(to_input(&ev));
                            }
                            Ok(_) => {}
                            Err(e) => {
                                dprintln!("neoui: input failed ({})", e);
                                return e;
                            }
                        }
                        next = poll_now(input.as_mut());
                    }
                    coalesce_moves(&mut evs);
                    for event in evs {
                        let app_on_screen = host.front().is_some() && !phone.modal();
                        match event {
                            InputEvent::Touch { id, x, y, phase } => {
                                let body = host.body;
                                if phase == TouchPhase::Down {
                                    swallow = swipe.is_some();
                                    home_press = (screen == "home"
                                        && !app_on_screen
                                        && !power_menu
                                        && !phone.modal()
                                        && transition.is_none()
                                        && !swallow
                                        && contains(&ui.home_strip(), x, y))
                                    .then_some(x);
                                    down_at = Some((x, y));
                                    // A touch belongs to the app from press to release if it starts in its body.
                                    to_app = app_on_screen
                                        && !power_menu
                                        && !ui.overlay_open()
                                        && contains(&host.app_area(), x, y);
                                }
                                if phase == TouchPhase::Up {
                                    let down = down_at.take();
                                    // PIN keypad coordinates disclose the entered digits just as text would.
                                    if !phone.modal() {
                                        dprintln!("neoui: tap {},{} down {:?}", x, y, down);
                                        tap_end = Some(down);
                                    }
                                }
                                if swallow {
                                    swallow = phase != TouchPhase::Up;
                                    continue;
                                }
                                if let Some((s, _)) = swipe.as_mut() {
                                    // the server's slide owns the touch now, Slint doesn't see it
                                    match phase {
                                        TouchPhase::Move => s.drag(x),
                                        TouchPhase::Up => {
                                            let dx = x - home_press.take().unwrap_or(x);
                                            s.release(
                                                time::millis(),
                                                dx * s.dir() > turn,
                                                SWIPE_MS,
                                            );
                                            dprintln!("neoui: swipe {} px", dx);
                                        }
                                        TouchPhase::Down => {}
                                    }
                                    continue;
                                }
                                if let (TouchPhase::Move, Some(x0)) = (phase, home_press)
                                    && (x - x0).abs() >= slop
                                {
                                    let (dir, page) = ((x - x0).signum(), ui.home_page());
                                    let to = page - dir;
                                    if (0..HOME_PAGES).contains(&to)
                                        && (old.len() == w * h
                                            || old.try_reserve_exact(w * h).is_ok())
                                    {
                                        // Slint's touch is cancelled (no page change, no tap on the widget it
                                        // went down on), the screen as it is becomes the current page's layer,
                                        // and Slint draws the neighbour once into the frame (below); from here
                                        // the server only copies
                                        ui.cancel_touch();
                                        old.resize(w * h, 0);
                                        snapshot(&host, &frame, &mut old, w);
                                        ui.set_home_page(to);
                                        let mut s = Slide::start(dir, w as i32, x0, 0);
                                        s.drag(x);
                                        swipe = Some((s, page));
                                        continue;
                                    }
                                    home_press = None; // no neighbour, or no memory: Slint's own swipe
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
                Either::Right(Either::Left((watch, status))) => {
                    phone.observe(status);
                    modem_update.set(watch.next());
                }
                Either::Right(Either::Right(())) => {}
            }
            // Apply modem navigation before the app lifecycle/compositor decides which surface is visible.
            phone.poll(&mut ui, time::millis(), &mut contacts, &mut host.settings);
            let modal = phone.modal();
            let modal_changed = modal != phone_modal;
            phone_modal = modal;
            if modal {
                transition = None;
                swipe = None;
                home_press = None;
                to_app = false;
            }
            if modal_changed {
                host.refresh = true;
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
            let mut repaint_all = modal_changed;
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
                // what is on screen now (the old app with it, before set_screen hides it) fades out; when memory is
                // short there is no fade, the new screen just appears (the Pixel's kernel has 87 MB of RAM for now)
                if old.len() == w * h || old.try_reserve_exact(w * h).is_ok() {
                    old.resize(w * h, 0);
                    snapshot(&host, &frame, &mut old, w);
                    transition = Some(Transition::new(ms, TRANSITION_MS));
                } else {
                    dprintln!(
                        "neoui: no memory for a fade ({} bytes), switching at once",
                        w * h * 4
                    );
                }
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

            // the phone's full-screen call or SIM view covers the app: no app keyboard or nav buttons then
            let want_keyboard = if phone_modal {
                0
            } else {
                host.front().map_or(0, |a| a.keyboard)
            };
            if want_keyboard != keyboard_shown {
                ui.set_keyboard(want_keyboard);
                keyboard_shown = want_keyboard;
            }
            host.set_keyboard_heights(ui.keyboard_heights());
            let front = !phone_modal && host.front().is_some();
            if front != front_shown {
                ui.set_app_front(front);
                front_shown = front;
            }
            let want_nav = if phone_modal {
                0
            } else {
                host.front().map_or(0, |a| a.nav)
            };
            if want_nav != nav_shown {
                ui.set_app_nav(
                    want_nav & proto::NAV_BACK != 0,
                    want_nav & proto::NAV_OPTIONS != 0,
                );
                nav_shown = want_nav;
            }
            let overlay = ui.overlay_open();
            if overlay_shown && !overlay {
                host.refresh = true; // the app's surface again, whole
            }
            overlay_shown = overlay;
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
                settings::log_save(host.settings.flush());
                battery.save();
                clockwork.power(a);
            }
            battery.poll(&mut ui, time::millis());
            wifi.poll(
                &mut ui,
                time::millis(),
                host.settings.get(keys::RADIO_WIFI).unwrap_or(true),
            );
            mirror.sync(&mut ui, &mut host.settings, ms);
            phone.sync_settings(&host.settings, ms); // (phone.poll runs right after the select, C7)
            if let Some(worker) = usb_debug.as_mut() {
                worker.set(usb_debugging(&host.settings));
            }
            host.deliver_settings();
            settings::log_save(host.settings.tick(ms));
            if let Some(source) =
                netclock.frame(&host.settings, phone.nitz(), wifi.take_http_time(), ms)
            {
                clockwork.set_synced(source);
            }
            clockwork.frame(&mut ui, &mut host, ms);
            let woke = if phone.take_ring() {
                scr.wake(ms)
            } else {
                None
            };
            // no panel (QEMU): the screen never times out (a blank UI would drop the CI's taps)
            let timeout = if panel_max > 0 {
                host.settings.get(keys::DISPLAY_TIMEOUT).unwrap_or(60)
            } else {
                0
            };
            if let Some(c) = woke.or_else(|| scr.tick(ms, timeout, phone.in_call() || power_menu)) {
                set_screen(c, &mut host.refresh);
            }
            if panel_max > 0 && scr.is_on() {
                let want = screen::panel_level(
                    host.settings.get(keys::DISPLAY_BRIGHTNESS).unwrap_or(160),
                    panel_max,
                );
                if want != brightness_sent {
                    set_brightness(want);
                    brightness_sent = want;
                }
            }
            if !scr.is_on() {
                host.release_frames(); // apps go on (their frames aren't shown)
                continue; // nothing drawn while dark; the pollers above keep running
            }

            let fading = transition.is_some() || swipe.is_some();
            let (px, damage, render_us, mut copy_us) =
                present(&mut ui, &mut frame, &display, &perf, !fading);
            perf.shifts(ui.take_shifts());
            if let Some((s, from)) = swipe.as_mut() {
                perf.frame(render_us, 0, 0, px);
                let m = perf.mark();
                let live = s.step(time::millis());
                slide(
                    &old,
                    &frame,
                    ui.home_strip(),
                    s.offset,
                    w,
                    &mut rows.1,
                    &display,
                );
                perf.layer(perf.since(&m));
                if !live {
                    if s.committed() {
                        // the frame holds the new page (and its dots) already
                        display.blit(&frame, w, 0, 0, w, h);
                    } else {
                        // the screen shows the old page again; Slint goes back to it unseen
                        ui.set_home_page(*from);
                        present(&mut ui, &mut frame, &display, &perf, false);
                    }
                    dprintln!("neoui: home page {}", ui.home_page());
                    swipe = None;
                    old = Vec::new();
                }
                host.release_frames();
                perf.idle();
                continue;
            }
            match transition.and_then(|t| t.progress(time::millis())) {
                Some(e) => {
                    perf.frame(render_us, 0, 0, px);
                    let m = perf.mark();
                    fade(&old, &frame, &host, e, (w, h), &mut rows, &display);
                    perf.layer(perf.since(&m));
                }
                None => {
                    if transition.take().is_some() {
                        // the fade is over: the new screen as it is, the app's surface whole; the snapshot goes
                        old = Vec::new();
                        repaint_all = true;
                        host.refresh = true;
                    }
                    if repaint_all {
                        let m = perf.mark();
                        display.blit(&frame, w, 0, 0, w, h);
                        copy_us += perf.since(&m);
                    }
                    let m = perf.mark();
                    let app_px = if power_menu || phone_modal || ui.overlay_open() {
                        host.blits.clear(); // the overlay covers the app; it is copied whole when it closes
                        0
                    } else {
                        compose(&mut host, &damage, &display)
                    };
                    perf.frame(render_us, copy_us, perf.since(&m), px + app_px);
                }
            }
            host.release_frames(); // the app's frame is on screen: it may draw the next
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

/// One frame of a home page swipe (smooth-ui S2c): the pages band only (the rest of the screen stays as it was),
/// the current page (`old`) at `offset` with the neighbour (in `frame`) beside it.
fn slide(
    old: &[u32],
    frame: &[u32],
    strip: Rect,
    offset: i32,
    w: usize,
    out: &mut [u32],
    display: &Display,
) {
    let y0 = strip.y.max(0) as usize;
    for y in y0..(y0 + strip.h as usize).min(frame.len() / w) {
        layer::slide_row(out, &old[y * w..][..w], &frame[y * w..][..w], offset);
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

/// The panel's highest brightness level (display HAL), or 0 when it has none (QEMU).
fn display_max_brightness() -> i32 {
    let mut max: i32 = 0;
    // SAFETY: a1 is a valid i32 out-parameter for EDisplayHalMaxDisplayBrightness; a2 is unused.
    let r = unsafe {
        e32::sys::ui::hal_function(
            HAL_DISPLAY,
            DISPLAY_MAX_BRIGHTNESS,
            (&mut max as *mut i32).cast(),
            core::ptr::null_mut(),
        )
    };
    dprintln!("neoui: panel brightness max {} ({})", max, r);
    if r == 0 { max } else { 0 }
}

/// The panel's brightness, 0..=display_max_brightness() (EDisplayHalSetDisplayBrightness).
fn set_brightness(level: i32) {
    // SAFETY: the function takes the level by value in a1; a2 is unused.
    let r = unsafe {
        e32::sys::ui::hal_function(
            HAL_DISPLAY,
            DISPLAY_SET_BRIGHTNESS,
            level as usize as *mut u8,
            core::ptr::null_mut(),
        )
    };
    if r != 0 {
        dprintln!("neoui: brightness {} failed ({})", level, r);
    }
}

/// Screen on or off (EDisplayHalSetState): the kernel sleeps or wakes the panel and the touchscreen. On wake the
/// app's surface is composed again whole.
fn set_screen(c: screen::Change, refresh: &mut bool) {
    let on = c == screen::Change::On;
    // SAFETY: the function takes the state by value in a1; a2 is unused.
    let r = unsafe {
        e32::sys::ui::hal_function(
            HAL_DISPLAY,
            DISPLAY_SET_STATE,
            usize::from(on) as *mut u8,
            core::ptr::null_mut(),
        )
    };
    dprintln!("neoui: screen {} ({})", if on { "on" } else { "off" }, r);
    if on {
        *refresh = true;
    }
}

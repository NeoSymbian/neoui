// SPDX-License-Identifier: GPL-3.0-only
//! The neoui launcher on the 2010 Symbian user side (NeoSymbian R5), started by estart as `SYSSTART.EXE` in place of
//! the window server. It owns the screen and the input:
//! - draws with `neoui-render` (Slint's software renderer) into a RAM frame, then copies the damaged rectangles to
//!   the frame buffer (`e32::display`; video RAM is uncached, and the renderer reads back the previous frame);
//! - registers as the window server and takes raw pointer/key events from the kernel (`e32::event`);
//! - runs apps as separate processes (R6, [`host`]): the `NeoUi` IPC server, a shared surface per app shown in the
//!   shell's body, input routed to the app, `Visible` as the shell's screen changes and `Quit` when one is closed;
//! - reports over the debug port (`neoui: ...` lines, checked by CI).
#![no_std]

extern crate alloc;

mod battery;
mod caps;
mod clock;
mod clockwork;
mod host;
mod keyboard;
mod netclock;
mod peek;
mod perf;
mod phone;
mod screen;
mod settings;
mod sound;
mod usb_debug;
mod wifi;

use alloc::string::String;
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
use neoui_proto::gesture::{Edge, Gesture, Map, Recognizer, Step};
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::settings::keys;
use neoui_proto::settings_app::info::encode_ip;
use neoui_proto::sound_profile;
use neoui_proto::{Damage, InputEvent, Rect, TouchPhase, coalesce_moves, next_deadline};
use neoui_render::Renderer;
use neoui_render::layer::{self, Slide, Transition};

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
/// How long a tap shows Buttons' key row over a full-screen page (rule 13).
const PEEK_MS: u64 = 3000;
/// `EHalGroupDisplay` and its `TDisplayHalFunction`s (`u32hal.h`): screen on/off and the panel's brightness (D2).
const HAL_DISPLAY: i32 = 4;
const DISPLAY_MAX_BRIGHTNESS: i32 = 12;
const DISPLAY_SET_BRIGHTNESS: i32 = 13;
const DISPLAY_SET_STATE: i32 = 20;
const DISPLAY_STATE: i32 = 21;
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
    // which kernel, ROM and neoui this is: in the log of every run (the Settings app's About phone reads them too)
    for line in neoapp::build::lines() {
        dprintln!("neoui: build {}", line);
    }

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
    let mut sound = sound::Sound::new();
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
    let mut host = Host::new(
        ui.app_body(),
        h as u32,
        dp256,
        ui.keyboard_heights(),
        settings,
    );
    let mut keyboard_shown = 0;
    // the foreground app's page as last given to the shell
    let mut chrome_shown = neoui_render::Page::default();
    // the front app is the legacy bridge (the shell's header over it), and whether it has Options
    let mut legacy_shown = (false, false);
    let mut query_shown: Option<neoui_render::Query> = None;
    // when Buttons' key row, shown over a full-screen page by a tap, goes again
    let mut peek_until = 0;
    // (TP4) when the heads-up banner goes (ms; 0: none shown)
    let mut banner_until: u64 = 0;
    // the screen and view last checked for an app page (host::page_for)
    let mut page_shown = (String::new(), String::new());
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
    let mut brightness_retry_at = 0u64;
    let mut saver_logged: Option<bool> = None;
    let mut pulse_logged = false;
    // the Power menu covers the whole screen: no app surface over it, every touch to the shell
    let mut power_menu = false;
    // the store's change counter the sound profiles last looked at (u32::MAX: not yet, so the upgrade runs once)
    let mut profiles_seq = u32::MAX;
    let mut overlay_shown = false;
    let mut front_shown = false; // the shell knows an app is in front // the shade or the Options menu over the app (last frame)
    // a refused HAL query must not freeze the clock below, which would hold every frame back
    let tick_us = match time::tick_period_us() {
        0 => 1000,
        us => u64::from(us),
    };
    let now_us = || battery::ticks() * tick_us; // widened: a wrapped counter would stall the frames

    rt::block_on(async {
        // Both stay outstanding across wake-ups, so no input event or message is lost to a cancel.
        let mut input = pin!(hook.next());
        let mut receive = pin!(server.receive());
        let mut modem_update = pin!(phone::Watch::new(phone.present()).next());
        let mut screen = ui.screen();
        let mut down_at = None;
        let mut to_app = false;
        // fingers after the first: the app's when they go down in its area (an old app's key pad)
        let mut fingers = neoui_proto::Fingers::default();
        // the app those fingers went down in: they are dropped when another app comes to the front
        let mut fingers_app: Option<&'static str> = None;
        // the press of the last release, for the frame that shows it (one frame may show several taps)
        let mut tap_end = None;
        // the next frame's start, and whether something waits for it
        let mut next_frame = 0u64;
        let mut due = false;
        let mut transition: Option<Transition> = None;
        // smooth-ui S2c: a press on the home pages (where it went down), the server's page slide once it moves
        // sideways (with the page it started from), and a touch that comes while the slide settles (ignored)
        let mut home_press: Option<i32> = None;
        // the minute the shade's card times were written in
        let mut notes_minute = 0i64;
        // the press on the home pages became a pull downwards (top-panel TP1): Slint lost it, the shade may open
        let mut home_pull = false;
        let mut swipe: Option<(Slide, i32)> = None;
        let mut swallow = false;
        let mut phone_modal = false;
        let (slop, turn) = ((8 * dp256 / 256) as i32, (60 * dp256 / 256) as i32);
        let pull = (48 * dp256 / 256) as i32; // a swipe down on home this long opens the shade
        // gesture navigation (docs neoui-shell/navigation.md G1): the edge recognizer, the touches it holds back (the press and the
        // latest move), the hint it asks for (edge, progress, the finger across the edge) and where it was drawn
        let mut gest = Recognizer::new(w as u32, h as u32, dp256);
        let mut held: Vec<InputEvent> = Vec::new();
        let mut hint: Option<(Edge, u32, i32)> = None;
        let mut hint_drawn: Option<Rect> = None;
        // Show touches (SE3a): the fingers down (id, x, y) and the rings drawn for them
        let mut touches: Vec<(u8, i32, i32)> = Vec::new();
        let mut touches_drawn: Vec<Rect> = Vec::new();
        // (G4) swipe down closes apps: where finger 0 went down, the app being dragged down and how far (px), and
        // that the screen must be put back whole when the drag ends
        let mut press_y = 0;
        let mut close_drag: Option<(&'static str, i32)> = None;
        let mut close_restore = false;
        // (G2) the peek under way, and that the gesture it settled onto changes the screen with no fade; (G3) pinch
        let mut peek: Option<peek::Peeking> = None;
        let mut no_fade = false;
        let mut pinch = neoui_proto::pinch::Pinch::default();
        // a hardware keyboard's Shift, Caps Lock and Ctrl
        let mut hw = neoui_proto::hwkeys::State::default();
        loop {
            // a dark screen draws nothing: no frame pacing for animations or a pending draw
            let wait = if scr.is_on()
                && (due
                    || transition.is_some()
                    || swipe.is_some()
                    || peek.is_some()
                    || ui.has_active_animations())
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
            // a Slint timer (a long press, the kit's spinner), the toast's, banner's or volume HUD's end, or a held gesture's
            // pause is due sooner
            let wait = [
                gest.busy().then_some(50_000),
                ui.next_timer().map(|d| d.as_micros() as u64),
                host.note.as_ref().and_then(|(_, _, until)| {
                    neoui_proto::settings_app::privacy::toast_wake_us(*until, time::millis())
                }),
                (banner_until != 0).then(|| banner_until.saturating_sub(time::millis()) * 1000),
                host.media_wake_us(time::millis()),
            ]
            .into_iter()
            .flatten()
            .fold(wait, u64::min);
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
                                    battery.display(set_screen(c, &mut host));
                                    if c == screen::Change::Off
                                        && (core::mem::take(&mut to_app) | fingers.take())
                                    {
                                        host.send(AppEvent::InputReset); // its finger's lift never comes
                                    }
                                }
                            }
                            Ok(ev)
                                if ev.i_type == raw::EKEY_DOWN
                                    && matches!(ev.scan_code(), SCAN_VOL_UP | SCAN_VOL_DOWN)
                                    && (phone.volume_key(ev.scan_code() == SCAN_VOL_UP)
                                        || sound.volume_key(
                                            ev.scan_code() == SCAN_VOL_UP,
                                            &mut host.settings,
                                            time::millis(),
                                        )) =>
                            {
                                host.volume_hud(&mut ui, phone.call_volume(), time::millis());
                            }
                            // a dark screen takes no touches or keys (the kernel's touchscreen sleeps too)
                            Ok(ev) if scr.is_on() => {
                                scr.activity(time::millis());
                                evs.extend(
                                    to_input(&ev)
                                        .or_else(|| keyboard::hardware_key(&ev, &mut hw, &ui)),
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                dprintln!("neoui: input failed ({})", e);
                                return e;
                            }
                        }
                        next = poll_now(input.as_mut());
                    }
                    // Edge gestures see every move (before they are merged: a merged move could hide the
                    // direction); what they don't take is routed as usual below.
                    // Show touches (SE3a) only watches the fingers: the events are routed below as they would be
                    // (D11) none while a PIN may be typed: the rings would show the digits
                    if host
                        .settings
                        .get(keys::DEVELOPER_SHOW_TOUCHES)
                        .unwrap_or(false)
                        && !phone.modal()
                        && !ui.number_pad_up()
                    {
                        // a finger from its press to its lift (a pointer moving unpressed starts nothing)
                        for ev in &evs {
                            if let InputEvent::Touch { id, x, y, phase } = *ev {
                                let known = touches.iter().any(|t| t.0 == id);
                                touches.retain(|t| t.0 != id);
                                if phase == TouchPhase::Down || (phase == TouchPhase::Move && known)
                                {
                                    touches.push((id, x, y));
                                }
                            }
                        }
                    } else {
                        touches.clear();
                    }
                    // Gestures (the Thumb map) and N9 gestures; Buttons has no edge gestures (D18)
                    let mode = host.settings.get(keys::NAV_MODE).unwrap_or(1);
                    let map = if mode == 2 { Map::N9 } else { Map::Thumb };
                    gest.set_map(map);
                    let gestures = mode != 0
                        && !power_menu
                        && !phone.modal()
                        && !ui.overlay_open()
                        && transition.is_none()
                        && swipe.is_none()
                        && peek.is_none();
                    // home keeps its side edges for the pages (Back does nothing there); N9's right edge is Open apps
                    // (G4) the status bar over an app (not a full-screen page: it has none) drags the app down
                    let top = gestures
                        && host.settings.get(keys::NAV_SWIPE_CLOSE).unwrap_or(false)
                        && host.front().is_some()
                        && !host.front_full();
                    let live: &[Edge] = match (gestures, mode, screen.as_str()) {
                        (false, _, _) => &[],
                        (true, 2, "home") => &[Edge::Right, Edge::Bottom],
                        (true, _, "home") => &[Edge::Bottom],
                        _ if top => &[Edge::Left, Edge::Right, Edge::Bottom, Edge::Top],
                        _ => &[Edge::Left, Edge::Right, Edge::Bottom],
                    };
                    let ms = time::millis();
                    let mut routed = Vec::with_capacity(evs.len());
                    for ev in evs {
                        let InputEvent::Touch { id, x, y, phase } = ev else {
                            routed.push(ev);
                            continue;
                        };
                        let busy = gest.busy() || peek.is_some();
                        if peek::pinch(&mut pinch, &mut ui, &mut host, &ev, ms, busy) {
                            continue;
                        }
                        if id != 0 {
                            routed.push(ev); // gestures follow the first finger only; in order with it (below)
                            continue;
                        }
                        if phase == TouchPhase::Down {
                            press_y = y;
                        }
                        match gest.feed(phase, x, y, ms, live) {
                            Step::Pass => routed.push(ev),
                            Step::Hold => {
                                if phase == TouchPhase::Move && held.len() > 1 {
                                    held.pop();
                                }
                                held.push(ev);
                            }
                            Step::Replay => {
                                routed.append(&mut held);
                                routed.push(ev);
                            }
                            Step::Drop => {}
                            // (G4) the app follows the finger; nothing closes before the release
                            Step::Track {
                                edge: Edge::Top, ..
                            } => {
                                held.clear();
                                let key = close_drag
                                    .map(|(k, _)| k)
                                    .or_else(|| host.front().map(|a| a.key));
                                close_drag = key.map(|k| (k, y - press_y));
                            }
                            Step::Commit(Gesture::Close) => {
                                // the app the drag started on, once, if it is still the one in front
                                if let Some((key, _)) = close_drag.take()
                                    && host.front().map(|a| a.key) == Some(key)
                                {
                                    dprintln!("neoui: gesture close {}", key);
                                    host.close(key, ms);
                                    ui.gesture(Gesture::Close);
                                    peek::pulse(&host);
                                }
                                close_restore = true;
                            }
                            Step::Track { edge, progress } => {
                                held.clear();
                                hint = Some((
                                    edge,
                                    progress,
                                    if edge == Edge::Bottom { x } else { y },
                                ));
                                let bufs = (&frame[..], &mut old, (w, h));
                                let along = gest.along_dx();
                                let at = Some((x, y));
                                peek::track(
                                    &mut peek,
                                    &mut ui,
                                    &host,
                                    bufs,
                                    (map, along),
                                    (edge, progress),
                                    at,
                                    dp256,
                                    ms,
                                );
                            }
                            Step::Commit(g) => {
                                hint = None;
                                dprintln!("neoui: gesture {}", g.name());
                                // (G2) a peek settling onto the target applies the gesture once it is there
                                if !peek::release(
                                    &mut peek,
                                    &mut ui,
                                    ms,
                                    Some(g),
                                    &mut close_restore,
                                ) {
                                    ui.gesture(g);
                                    for n in ui.take_nav() {
                                        host.send_nav(n);
                                    }
                                    peek::pulse(&host);
                                }
                            }
                            Step::Cancel => {
                                hint = None;
                                close_restore |= close_drag.take().is_some();
                                peek::release(&mut peek, &mut ui, ms, None, &mut close_restore);
                            }
                        }
                    }
                    let mut evs = routed;
                    coalesce_moves(&mut evs);
                    for event in evs {
                        let app_on_screen = host.front().is_some() && !phone.modal();
                        match event {
                            // fingers after the first: the app's when they press in it (the shell's widgets and
                            // gestures follow finger 0 only), until they lift or another app comes to the front
                            InputEvent::Touch { id, x, y, phase } if id != 0 => {
                                let front = host.front().map(|a| a.key);
                                if fingers_app.is_some() && fingers_app != front {
                                    fingers.take(); // the app they pressed in went away: Visible(false) told it
                                }
                                let in_app = front.is_some()
                                    && !phone.modal()
                                    && !power_menu
                                    && !ui.overlay_open()
                                    && contains(&host.app_area(), x, y)
                                    && !ui.banner_rect().is_some_and(|r| contains(&r, x, y));
                                if phase == TouchPhase::Down && in_app {
                                    fingers_app = front;
                                }
                                if fingers.route(id, phase, in_app) {
                                    let body = host.surface_rect();
                                    host.send(AppEvent::Input(InputEvent::Touch {
                                        id,
                                        x: x - body.x,
                                        y: y - body.y,
                                        phase,
                                    }));
                                }
                            }
                            InputEvent::Touch { id, x, y, phase } => {
                                let body = host.surface_rect();
                                if phase == TouchPhase::Down {
                                    swallow = swipe.is_some() || peek.is_some();
                                    // the shade over home takes its own drags: no page slide under it
                                    home_press = (screen == "home"
                                        && !ui.home_editing()
                                        && !app_on_screen
                                        && !power_menu
                                        && !ui.overlay_open()
                                        && !phone.modal()
                                        && transition.is_none()
                                        && !swallow
                                        && contains(&ui.home_strip(), x, y))
                                    .then_some(x);
                                    down_at = Some((x, y));
                                    home_pull = false;
                                    // A touch belongs to the app from press to release if it starts in its body.
                                    // (TP4) not on the banner over it: the banner is the shell's
                                    to_app = app_on_screen
                                        && !power_menu
                                        && !ui.overlay_open()
                                        && contains(&host.app_area(), x, y)
                                        && !ui.banner_rect().is_some_and(|r| contains(&r, x, y));
                                    // Buttons over a full-screen page: the first tap shows the key row for a moment
                                    // (rule 13), the app doesn't get it
                                    if to_app
                                        && !host.peek
                                        && host.front_full()
                                        && host.settings.get(keys::NAV_MODE).unwrap_or(1) == 0
                                    {
                                        host.peek = true;
                                        peek_until = time::millis() + PEEK_MS;
                                        host.refresh = true;
                                        ui.set_key_row_peek(true);
                                        to_app = false;
                                        swallow = true;
                                    }
                                }
                                if phase == TouchPhase::Up {
                                    let down = down_at.take();
                                    // PIN keypad coordinates disclose the entered digits just as text would.
                                    // (D11) so do the number pad's, wherever a PIN is typed
                                    if !phone.modal() && !ui.number_pad_up() {
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
                                // a swipe down on home opens the shade (top-panel TP1, `shell.swipehome`): a press
                                // that first moves steeply downwards is a pull. It never turns into a page slide, and
                                // Slint loses it when the shade opens or the finger lifts short of it (no widget
                                // click); until then Slint follows it as before (cancelling earlier left Slint's
                                // next press dead). Anything else stays the page swipe it always was. Home must still
                                // be what the finger is on: a call may have come to the front since the press.
                                let dy = y - down_at.map_or(y, |d| d.1);
                                let on_home = ui.screen() == "home"
                                    && host.front().is_none()
                                    && !ui.overlay_open()
                                    && !phone.modal();
                                if let (TouchPhase::Move, Some(x0)) = (phase, home_press)
                                    && !home_pull
                                    && on_home
                                    && dy >= slop
                                    && dy > 2 * (x - x0).abs()
                                    && host.settings.get(keys::SHELL_SWIPEHOME).unwrap_or(true)
                                {
                                    home_pull = true;
                                    ui.stop_home_hold(); // nor is it a long press any more
                                }
                                if home_pull {
                                    if phase == TouchPhase::Move && dy >= pull && on_home {
                                        ui.cancel_touch();
                                        ui.open_shade();
                                        home_pull = false;
                                        home_press = None;
                                        swallow = true; // the rest of this touch goes nowhere
                                        dprintln!("neoui: shade from home");
                                        continue;
                                    }
                                    if phase == TouchPhase::Up {
                                        ui.cancel_touch(); // short of the shade: no click on what it went down on
                                        home_pull = false;
                                        home_press = None;
                                        continue;
                                    }
                                    if !on_home {
                                        home_pull = false;
                                        home_press = None;
                                    }
                                }
                                if let (TouchPhase::Move, Some(x0)) = (phase, home_press)
                                    && !home_pull
                                    && (x - x0).abs() >= slop
                                {
                                    let (dir, page) = ((x - x0).signum(), ui.home_page());
                                    let to = page - dir;
                                    if (0..ui.home_pages()).contains(&to)
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
                                    if phase == TouchPhase::Down {
                                        ui.touch_outside_keyboard(y);
                                    }
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
                                    for n in ui.take_nav() {
                                        host.send_nav(n);
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
            // Apply modem navigation (a call brings the Phone app up) before the app lifecycle/compositor decides which
            // surface is visible.
            phone.poll(&mut ui, &host.settings);
            let modal = phone.modal();
            let modal_changed = modal != phone_modal;
            phone_modal = modal;
            if modal {
                transition = None;
                swipe = None;
                home_press = None;
                if core::mem::take(&mut to_app) | fingers.take() {
                    host.send(AppEvent::InputReset); // the call took the screen from under its finger
                }
            }
            // a gesture under way ends when the screen changes under it (a call, an alarm, the PIN prompt)
            // (G4) an overlay (the shade, a dialog) or another app in front ends a drag down too
            let drag_lost = close_drag
                .is_some_and(|(k, _)| ui.overlay_open() || host.front().map(|a| a.key) != Some(k));
            // (G2) a peek shows its target in Slint: only another screen than that ends it
            let expected = peek.as_ref().map_or(screen.as_str(), |p| p.showing());
            if modal || ui.screen() != expected || drag_lost {
                gest.reset();
                held.clear();
                hint = None;
                close_restore |= close_drag.take().is_some() | peek.is_some();
                peek::abort(&mut peek, &mut ui);
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
            let mut repaint_all = modal_changed
                | core::mem::take(&mut host.repaint)
                | core::mem::take(&mut close_restore);
            if repaint_all {
                host.refresh = true;
            }
            // (D10) Settings' Edit home; (G3) the thumbnails of apps that went to the back
            if core::mem::take(&mut host.d10.edit_home) {
                ui.edit_home();
            }
            // (D7) a wallpaper picture installed (or read at boot)
            if let Some(px) = host.wallpaper.pending.take() {
                let (w, h) = (
                    neoui_proto::settings_app::wallpaper::W,
                    neoui_proto::settings_app::wallpaper::H,
                );
                ui.set_wallpaper_picture(Some((&px, u32::from(w), u32::from(h))));
            }
            host.show_thumbs(&mut ui);
            let ms = time::millis();
            host.tick_jobs(ms);
            let reaped = host.reap(ms);
            // (G2) the host moving the screen (an app's exit, OPEN or leave) ends a peek first: it would put it back
            if peek.is_some()
                && (!reaped.is_empty() || host.requested_screen.is_some() || host.leave)
            {
                peek::abort(&mut peek, &mut ui);
                repaint_all = true;
            }
            for key in reaped {
                // its screen goes with it: home, or Classic apps for the bridge (its guest's list)
                if ui.screen() == key {
                    ui.show(neoui_proto::apps::after_exit(key), "");
                }
                // the whole frame goes out, so the front app's surface must too (a hidden owner's exit, D4,
                // left Settings with only its damage over the shell's frame)
                repaint_all = true;
                host.refresh = true;
            }
            if let Some(key) = host.requested_screen.take() {
                match host.requested_from.take() {
                    Some(from) => ui.show_from(key, from),
                    None => ui.show(key, ""),
                }
            }
            if core::mem::take(&mut host.leave) {
                ui.leave_app();
            }
            let now = ui.screen();
            // (G2) the peek's slide was the animation of the change it committed (once: a gesture that changed
            // nothing must not take the next fade)
            let slid = peek.is_none() && core::mem::take(&mut no_fade);
            // (G2) a peek's preview is no screen change: the app stays in front until it commits
            if now != screen && peek.is_none() {
                dprintln!("neoui: screen {}", now);
                gest.reset();
                held.clear();
                hint = None;
                // what is on screen now (the old app with it, before set_screen hides it) fades out; when memory is
                // short there is no fade, the new screen just appears (the Pixel's kernel has 87 MB of RAM for now)
                if slid {
                    // no fade: the slide showed the change
                } else if old.len() == w * h || old.try_reserve_exact(w * h).is_ok() {
                    old.resize(w * h, 0);
                    snapshot(&host, &frame, &mut old, w);
                    transition = Some(Transition::new(ms, TRANSITION_MS));
                } else {
                    dprintln!(
                        "neoui: no memory for a fade ({} bytes), switching at once",
                        w * h * 4
                    );
                }
                host.set_screen(&now, ms);
                screen = now;
            }
            host.keep_resident(ms);
            // an icon, notification or Settings row opening an app's page: the app is told, and the view is cleared
            // so the same page can be asked for again
            let view = ui.view();
            if peek.is_none()
                && (screen.as_str(), view.as_str())
                    != (page_shown.0.as_str(), page_shown.1.as_str())
            {
                if let Some((key, page)) = host::page_for(&screen, &view) {
                    dprintln!("neoui: {} page {}", key, page);
                    // without the app (not in this ROM) the shell's own screen keeps its view
                    if host.post(key, AppEvent::Page { page }) && !view.is_empty() {
                        ui.show(&screen, "");
                    }
                }
                page_shown = (screen.clone(), ui.view());
            }
            host::widget::open_pages(&mut host, &mut ui);
            let running = host.running();
            if running != running_shown {
                ui.set_running(&running);
                running_shown = running;
            }

            // the SIM prompt covers the app: no app keyboard or nav buttons then
            let want_keyboard = if phone_modal {
                0
            } else {
                host.front().map_or(0, |a| a.keyboard)
            };
            if want_keyboard != keyboard_shown {
                ui.set_keyboard(want_keyboard);
                keyboard_shown = want_keyboard;
            }
            // an app asking for the keyboard brings it back; while the user put it away the app has its whole body
            if core::mem::take(&mut host.reopen) {
                ui.clear_keyboard_dismissed();
            }
            host.set_keyboard_heights(if ui.keyboard_dismissed() {
                [0; proto::KEYBOARD_MODES]
            } else {
                ui.keyboard_heights()
            });
            let front = !phone_modal && host.front().is_some();
            if front != front_shown {
                ui.set_app_front(front);
                front_shown = front;
            }
            let want_page = match host.front() {
                Some(a) if !phone_modal => a.page(),
                _ => Default::default(),
            };
            if want_page != chrome_shown {
                ui.set_app_page(&want_page);
                chrome_shown = want_page;
            }
            let want_legacy = match host.front() {
                Some(a) if !phone_modal => a.legacy(),
                _ => (false, false),
            };
            if want_legacy != legacy_shown {
                ui.set_app_legacy(want_legacy.0, want_legacy.1);
                legacy_shown = want_legacy;
                host.refresh = true; // the surface moved under (or out from under) the header
            }
            // a mode switch moves the bottom: apps get their new height (rule 16)
            host.set_body(ui.app_body());
            // the toast, the shade's app entries and the launcher's badges (NOTE, NOTIFY, BADGE)
            host.expire_note(ms);
            // the cards' times ("2 min") move on with the minute
            let (unix, offset) = time::utc_now();
            if notes_minute != unix / 60 {
                notes_minute = unix / 60;
                host.notes_changed = true;
            }
            if core::mem::take(&mut host.notes_changed) {
                ui.set_note(host.note.as_ref().map(|(_, n, _)| n));
                // (TP4) a Min channel's cards fold under Silent; only Default and Urgent ones show their glyph
                ui.set_silent_notes(&host.silent_cards());
                let notes: Vec<(&str, &neoui_render::Notification)> =
                    host.notifications.iter().map(|(k, n)| (*k, n)).collect();
                let h24 = host.settings.get(keys::CLOCK_24H).unwrap_or(true);
                ui.set_notes(&notes, u64::try_from(unix).unwrap_or(0) * 1000, offset, h24);
                ui.set_status_apps(&host.glyph_keys());
                ui.set_dnd(host.dnd());
            }
            // (TP4) a new card's alert: the banner (never over a full-screen call or the keyboard's field), its chime
            // and buzz; the banner goes after notify.bannert seconds
            if let Some(p) = host.policies.pending.take() {
                let covered = (phone.in_call() && ui.screen() == "phone") || ui.keyboard_h() > 0;
                if p.banner && !covered && scr.is_on() {
                    ui.set_banner(Some((p.key, p.id)));
                    let secs = host
                        .settings
                        .get(keys::NOTIFY_BANNERT)
                        .unwrap_or(5)
                        .clamp(1, 30);
                    banner_until = ms + secs as u64 * 1000;
                }
                if let Some(tone) = p.chime {
                    let volume = host.settings.get(keys::SOUND_RING_VOLUME).unwrap_or(8);
                    sound::chime(tone, volume.clamp(0, 10) as u8);
                }
                if p.buzz {
                    let _ = haptic(200, 100);
                }
            }
            if banner_until != 0 && (ms >= banner_until || !ui.banner_shown()) {
                banner_until = 0;
                ui.set_banner(None);
            }
            // (TP3) the media session: the player, its buttons, the HUD's end
            host.media_frame(&mut ui, ms);
            // by (app, id): whatever moved in the list since, the card the user touched (or nothing, if it went)
            for (key, id, action) in ui.take_notes_opened() {
                host.open_notification(&key, id, action, ms);
            }
            for (key, id) in ui.take_notes_dismissed() {
                host.dismiss_notification(&key, id);
            }
            if ui.take_notes_cleared() {
                host.clear_notifications();
            }
            if core::mem::take(&mut host.badges_changed) {
                ui.set_badges_off(&host.badges_off());
                ui.set_badges(&host.badges());
            }
            let want_query = match host.front() {
                Some(a) if !phone_modal => a.query.clone(),
                _ => None,
            };
            if want_query != query_shown {
                ui.set_app_query(want_query.as_ref());
                query_shown = want_query;
            }
            if host.peek && (!host.front_full() || ms >= peek_until) {
                host.peek = false;
                host.refresh = true;
                ui.set_key_row_peek(false);
            }
            let overlay = ui.overlay_open();
            if overlay_shown && !overlay {
                host.refresh = true; // the app's surface again, whole
            }
            overlay_shown = overlay;
            // the shade's ⏻ (TP1) opens it like a held Power key, without touching the key's own timer
            let asked = ui.take_power_asked() && scr.is_on() && !power_menu;
            if asked || power_down_at.is_some_and(|t| ms.saturating_sub(t) >= POWER_HOLD_MS) {
                if !asked {
                    power_down_at = None;
                }
                dprintln!("neoui: power menu");
                let p = sound_profile::store::load(&host.settings);
                // the active profile first: the menu shows only as many as fit
                let mut names: Vec<(u8, &str)> =
                    p.list.iter().map(|x| (x.id, x.name.as_str())).collect();
                names.sort_by_key(|(id, _)| *id != p.active);
                ui.set_power_profiles(&names, p.active);
                ui.set_power_menu(true);
                power_menu = true;
            }
            let mut power = host.power.take();
            for a in ui.take_power() {
                ui.set_power_menu(false);
                power_menu = false;
                host.refresh = true; // the app's surface again, whole
                if a >= 100 {
                    // a sound profile (SE2a)
                    let r = sound_profile::store::apply(
                        &mut host.settings,
                        sound_profile::Command::Choose((a - 100) as u8),
                        ms,
                    );
                    dprintln!("neoui: power menu profile {} ({:?})", a - 100, r);
                } else if a >= 0 {
                    power = Some(a);
                }
            }
            if let Some(a) = power {
                settings::log_save(host.settings.flush());
                battery.save();
                clockwork.power(a);
            }
            if battery.poll(&mut ui, battery::mono_ms()) {
                host.set_info(proto::INFO_BATTERY, battery.info());
                host.set_info(proto::INFO_BATTERY_USAGE, battery.usage());
                host.set_battery_cycle(battery.cycle());
            }
            wifi.poll(
                &mut ui,
                time::millis(),
                host.settings.get(keys::RADIO_WIFI).unwrap_or(true),
            );
            wifi.sheet(&mut ui, time::millis()); // (TP7) the Wi-Fi tile's sheet
            // a Wi-Fi login page (browser UI5): the shade's entry, and the Settings app's Sign in (SETTINGS_ACTION)
            host.set_portal(wifi.portal());
            host.set_info(proto::INFO_NET, encode_ip(wifi.ip()));
            mirror.sync(&mut ui, &mut host.settings, ms);
            // the sound profiles fold in what others wrote (the shade's Silent tile, airplane mode off) before MODEMD
            // hears of it: the profile, its blob and its live values change together (SE2a)
            if host.settings.store().seq() != profiles_seq {
                if sound_profile::store::reconcile(&mut host.settings, ms) {
                    dprintln!(
                        "neoui: sound profile {}",
                        host.settings.get(keys::SOUND_PROFILE).unwrap_or(0)
                    );
                }
                profiles_seq = host.settings.store().seq();
            }
            phone.sync_settings(&host.settings, ms); // (phone.poll runs right after the select, C7)
            sound.poll(&host.settings, ms);
            if let Some(worker) = usb_debug.as_mut() {
                worker.set(usb_debugging(&host.settings));
            }
            host.deliver_settings();
            host.policy_frame(ms);
            host.tiles_frame(&mut ui, ms);
            settings::log_save(host.settings.tick(ms));
            if let Some(source) =
                netclock.frame(&host.settings, phone.nitz(), wifi.take_http_time(), ms)
            {
                clockwork.set_synced(source);
            }
            clockwork.frame(&mut ui, &mut host, ms);
            // a ringing call, or a service's OPEN_WAKE (an alarm, 1.19)
            let woke = if phone.take_ring() | core::mem::take(&mut host.wake) {
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
            let hold = phone.in_call() || power_menu || host.front_awake();
            if let Some(c) = woke.or_else(|| scr.tick(ms, timeout, hold)) {
                battery.display(set_screen(c, &mut host));
                if c == screen::Change::Off && (core::mem::take(&mut to_app) | fingers.take()) {
                    host.send(AppEvent::InputReset);
                }
            }
            if panel_max > 0 && scr.is_on() {
                // Battery saver caps the level without moving the setting (SE3a); off, the setting's level at once
                let setting = host.settings.get(keys::DISPLAY_BRIGHTNESS).unwrap_or(160);
                let saver = host.settings.get(keys::BATTERY_SAVER).unwrap_or(false);
                let want = neoui_proto::effects::panel_level(setting, panel_max, saver);
                if saver_logged != Some(saver) {
                    dprintln!(
                        "neoui: battery saver {}: panel {} of {} (setting {})",
                        if saver { "on" } else { "off" },
                        want,
                        panel_max,
                        setting
                    );
                    saver_logged = Some(saver);
                }
                // a refused write (the saver lifting too) is tried again a second later, not taken as done
                if want != brightness_sent && ms >= brightness_retry_at {
                    if set_brightness(want) {
                        brightness_sent = want;
                    } else {
                        brightness_retry_at = ms + 1000;
                    }
                }
            }
            // Keypress vibration (SE3a): one pulse per press of the system keyboard, whatever it typed and wherever
            let presses = ui.take_key_presses();
            let pulse = neoui_proto::effects::key_pulse(
                host.settings.get(keys::KEYBOARD_VIBRATE).unwrap_or(false),
            );
            if presses > 0
                && let Some((ms, amp)) = pulse
            {
                let r = haptic(ms, amp);
                // the first pulse since the setting came on, and any that fails (the log stays quiet otherwise)
                if !pulse_logged || r != 0 {
                    dprintln!("neoui: key pulse {} ms amplitude {} -> {}", ms, amp, r);
                    pulse_logged = true;
                }
            }
            if pulse.is_none() {
                pulse_logged = false;
            }
            // D6: a control that accepted a touch (the app in front's, CONTROL_FEEDBACK, or the shell's own)
            if let Some((kind, key)) = host.feedback.take() {
                sound::feedback(&host.settings, phone.in_call(), kind, key);
            } else if ui.take_accepted() {
                sound::feedback(
                    &host.settings,
                    phone.in_call(),
                    neoui_proto::ipc::FEEDBACK_CONTROL,
                    0,
                );
            }
            if !scr.is_on() {
                touches.clear(); // a sleeping touchscreen sends no lifts; the rings go at the next frame
                close_restore |= peek.is_some(); // (G2) nor a peek's: it would wait for one at wake
                peek::abort(&mut peek, &mut ui);
                host.release_frames(); // apps go on (their frames aren't shown)
                continue; // nothing drawn while dark; the pollers above keep running
            }

            // the Thumb map's up-and-hold: a still finger sends no moves, so time tells the recognizer
            if let Some(Step::Track { edge, progress }) = gest.tick(ms) {
                let at = hint.map_or(0, |(_, _, at)| at);
                hint = Some((edge, progress, at));
                let bufs = (&frame[..], &mut old, (w, h));
                peek::track(
                    &mut peek,
                    &mut ui,
                    &host,
                    bufs,
                    (Map::Thumb, None),
                    (edge, progress),
                    None,
                    dp256,
                    ms,
                );
            }
            let fading =
                transition.is_some() || swipe.is_some() || close_drag.is_some() || peek.is_some();
            let (px, damage, render_us, mut copy_us) =
                present(&mut ui, &mut frame, &display, &perf, !fading);
            keyboard::after_frame(&mut ui, &mut host);
            perf.shifts(ui.take_shifts());
            // (G2) the peek: the page over its target, then the gesture it settled onto, with no fade
            if let Some(p) = peek.as_mut() {
                perf.frame(render_us, 0, 0, px);
                let m = perf.mark();
                let thumb = host.settings.get(keys::NAV_MODE).unwrap_or(1) == 1;
                let shown = hint.map(|(e, p, at)| {
                    let back = e == Edge::Left || (e == Edge::Right && thumb);
                    hint_rect(e, p, at, back, (w, h), dp256)
                });
                if !peek::frame(
                    p,
                    time::millis(),
                    (&old, &frame),
                    &host,
                    shown,
                    &mut rows,
                    &display,
                ) {
                    if let Some(g) = peek.take().and_then(|p| peek::finish(p, &mut ui)) {
                        ui.gesture(g);
                        for n in ui.take_nav() {
                            host.send_nav(n);
                        }
                        peek::pulse(&host);
                        no_fade = true;
                    }
                    old = Vec::new();
                    close_restore = true;
                    hint_drawn = None;
                    due = true;
                }
                perf.layer(perf.since(&m));
                host.release_frames();
                perf.idle();
                continue;
            }
            if let Some((_, dy)) = close_drag
                && front_surface(&host).is_some()
            {
                drag_down(&frame, &host, dy, (w, h), &mut rows.1, &display);
                host.release_frames();
                perf.idle();
                continue;
            }
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
                    hint_drawn = None; // the fade writes the whole screen
                    touches_drawn.clear(); // and the rings with it: none while it runs
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
                    let covered = power_menu || phone_modal || ui.overlay_open();
                    let app_px = if covered {
                        host.blits.clear(); // the overlay covers the app; it is copied whole when it closes
                        0
                    } else {
                        compose(&mut host, &damage, &display)
                    };
                    // the toast lies over the app's body: put it back over what was composed; it's on screen now
                    if !covered && let Some(r) = ui.toast_rect() {
                        if app_px > 0 {
                            let (x, y) = (r.x.max(0) as usize, r.y.max(0) as usize);
                            let rw = (r.w as usize).min(w.saturating_sub(x));
                            let rh = (r.h as usize).min(h.saturating_sub(y));
                            display.blit(&frame, w, x, y, rw, rh);
                        }
                        host.note_displayed(time::millis());
                    }
                    // (TP4) the banner too, composited last over the app
                    if !covered
                        && app_px > 0
                        && let Some(r) = ui.banner_rect()
                    {
                        let (x, y) = (r.x.max(0) as usize, r.y.max(0) as usize);
                        let rw = (r.w as usize).min(w.saturating_sub(x));
                        let rh = (r.h as usize).min(h.saturating_sub(y));
                        display.blit(&frame, w, x, y, rw, rh);
                    }
                    // (TP3) the volume HUD too
                    if !covered
                        && app_px > 0
                        && let Some(r) = ui.hud_rect()
                    {
                        let (x, y) = (r.x.max(0) as usize, r.y.max(0) as usize);
                        let rw = (r.w as usize).min(w.saturating_sub(x));
                        let rh = (r.h as usize).min(h.saturating_sub(y));
                        display.blit(&frame, w, x, y, rw, rh);
                    }
                    // Show touches: put back what the rings covered before the hint (which may lie under them) is
                    // drawn, then the rings over everything; their cost counts in the perf line's app time
                    if !host
                        .settings
                        .get(keys::DEVELOPER_SHOW_TOUCHES)
                        .unwrap_or(false)
                    {
                        touches.clear(); // switched off with a finger still down
                    }
                    let rings = !touches.is_empty() || !touches_drawn.is_empty();
                    // under the shade or a dialog the app isn't on screen: its pixels mustn't show through
                    let under = if covered { None } else { front_surface(&host) };
                    if rings {
                        restore_touches(
                            &mut touches_drawn,
                            &frame,
                            under,
                            w,
                            &mut rows.1,
                            &display,
                        );
                    }
                    if hint.is_some() || hint_drawn.is_some() {
                        // Back: the left edge, and the right one in the Thumb map (N9's right edge is Open apps)
                        let thumb = host.settings.get(keys::NAV_MODE).unwrap_or(1) == 1;
                        let r = hint.map(|(e, p, at)| {
                            let back = e == Edge::Left || (e == Edge::Right && thumb);
                            hint_rect(e, p, at, back, (w, h), dp256)
                        });
                        draw_hint(r, &mut hint_drawn, &frame, &host, w, &mut rows.1, &display);
                    }
                    if rings {
                        draw_touches(
                            &touches,
                            &mut touches_drawn,
                            &frame,
                            under,
                            (w, h),
                            dp256,
                            &mut rows.1,
                            &display,
                        );
                    }
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
    // SAFETY: the chunk is `body.w * surface_rows * 4` bytes, committed at creation and mapped here; the app only
    // writes pixels.
    let src = unsafe {
        core::slice::from_raw_parts(
            surface.base().cast::<u32>(),
            body.w as usize * host.surface_rows(),
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

/// (G4) One frame of a swipe down to close: the front app's body `dy` px lower, the shell's frame dimmed above it (the
/// status bar stays).
fn drag_down(
    frame: &[u32],
    host: &Host,
    dy: i32,
    (w, h): (usize, usize),
    out: &mut [u32],
    display: &Display,
) {
    let app = front_surface(host);
    let Some((_, _, area)) = app else { return };
    let (y0, dy) = (area.y.max(0) as usize, dy.max(0) as usize);
    for y in y0..h {
        out.copy_from_slice(&frame[y * w..][..w]);
        if y >= y0 + dy {
            app_row(app, y - dy, out);
        } else {
            out.iter_mut()
                .for_each(|p| *p = 0xff00_0000 | (*p >> 1 & 0x007f_7f7f));
        }
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

/// The edge gesture's hint (docs neoui-shell/navigation.md): for Back (rule 09, `back`) a 36 dp bubble with ← that follows the
/// finger in from the edge and turns accent at the commit point; otherwise a 4 dp bar along the edge, longer as the
/// finger goes in, accent once letting go acts, white at the bottom's second point (`progress` 512: N9's Apps menu,
/// the Thumb map's Open apps).
fn hint_rect(
    edge: Edge,
    progress: u32,
    at: i32,
    back: bool,
    (w, h): (usize, usize),
    dp256: u32,
) -> Hint {
    let dp = |n: usize| n * dp256 as usize / 256;
    let p = progress.min(512) as usize;
    if back && edge != Edge::Bottom {
        let d = dp(36);
        let travel = dp(8) + dp(40) * p.min(256) / 256;
        let x = if edge == Edge::Left {
            travel
        } else {
            w.saturating_sub(travel + d)
        };
        let y = (at.max(0) as usize)
            .saturating_sub(d / 2)
            .min(h.saturating_sub(d));
        return Hint {
            rect: Rect::new(x as i32, y as i32, d as u32, d as u32),
            colour: 0,
            bubble: Some((p >= 256, edge == Edge::Left)),
        };
    }
    let t = dp(4);
    let colour = match p {
        512.. => 0x00ff_ffff,
        256.. => 0x003d_8bfd, // Theme.accent
        _ => 0x007d_838c,     // Theme.text-dim
    };
    let rect = match edge {
        Edge::Left | Edge::Right => {
            let len = (dp(32) + dp(96) * p.min(256) / 256).min(h);
            let y = (at.max(0) as usize).saturating_sub(len / 2).min(h - len);
            let x = if edge == Edge::Left { 0 } else { w - t };
            Rect::new(x as i32, y as i32, t as u32, len as u32)
        }
        Edge::Bottom => {
            let len = (dp(48) + w.saturating_sub(dp(48)) * p / 512).min(w);
            Rect::new(((w - len) / 2) as i32, (h - t) as i32, len as u32, t as u32)
        }
        // (G4) the top edge draws the app itself ([`drag_down`]), not a hint
        Edge::Top => Rect::new(0, 0, 0, 0),
    };
    Hint {
        rect,
        colour,
        bubble: None,
    }
}

/// What [`draw_hint`] draws: a bar of `colour`, or the Back bubble (committed, pointing left).
#[derive(Clone, Copy)]
struct Hint {
    rect: Rect,
    colour: u32,
    bubble: Option<(bool, bool)>,
}

/// Put back what the last hint covered (the frame with the front app over it), then draw the new one, if any,
/// straight to the screen. `row` is a screen-wide scratch row.
fn draw_hint(
    hint: Option<Hint>,
    drawn: &mut Option<Rect>,
    frame: &[u32],
    host: &Host,
    w: usize,
    row: &mut [u32],
    display: &Display,
) {
    let rows = |r: Rect| r.y.max(0) as usize..r.y.max(0) as usize + r.h as usize;
    let app = front_surface(host);
    if let Some(r) = drawn.take() {
        for y in rows(r) {
            row.copy_from_slice(&frame[y * w..][..w]);
            app_row(app, y, row);
            display.blit_to(
                row,
                w,
                (r.x as usize, 0, r.w as usize, 1),
                (r.x as usize, y),
            );
        }
    }
    let Some(h) = hint else { return };
    let r = h.rect;
    for y in rows(r) {
        match h.bubble {
            // the bubble over what is under it: the frame and the front app's row
            Some((committed, left)) => {
                row.copy_from_slice(&frame[y * w..][..w]);
                app_row(app, y, row);
                for x in 0..r.w as i32 {
                    if let Some(px) = neoui_render::back_bubble_pixel(
                        x,
                        y as i32 - r.y,
                        r.w as i32,
                        committed,
                        left,
                    ) {
                        row[r.x as usize + x as usize] = px;
                    }
                }
                display.blit_to(
                    row,
                    w,
                    (r.x as usize, 0, r.w as usize, 1),
                    (r.x as usize, y),
                );
            }
            None => {
                row[..r.w as usize].fill(h.colour);
                display.blit_to(row, w, (0, 0, r.w as usize, 1), (r.x as usize, y));
            }
        }
    }
    *drawn = Some(r);
}

/// Show touches (SE3a): what the screen shows in columns `x0..x1` of row `y` (the frame, with the front app's surface
/// over it when it is composed), into the same columns of `row`.
fn screen_span(
    frame: &[u32],
    app: Option<(&[u32], usize, Rect)>,
    w: usize,
    y: usize,
    (x0, x1): (usize, usize),
    row: &mut [u32],
) {
    row[x0..x1].copy_from_slice(&frame[y * w + x0..y * w + x1]);
    let Some((src, stride, area)) = app else {
        return;
    };
    let (ax, ay) = (area.x.max(0) as usize, area.y.max(0) as usize);
    if y >= ay && y < ay + area.h as usize {
        let (s0, s1) = (x0.max(ax), x1.min(ax + area.w as usize));
        if s0 < s1 {
            row[s0..s1].copy_from_slice(&src[(y - ay) * stride + (s0 - ax)..][..s1 - s0]);
        }
    }
}

/// Show touches (SE3a): put back what the rings covered.
fn restore_touches(
    drawn: &mut Vec<Rect>,
    frame: &[u32],
    app: Option<(&[u32], usize, Rect)>,
    w: usize,
    row: &mut [u32],
    display: &Display,
) {
    for r in drawn.drain(..) {
        let (x0, x1) = (r.x as usize, r.x as usize + r.w as usize);
        for y in r.y as usize..r.y as usize + r.h as usize {
            screen_span(frame, app, w, y, (x0, x1), row);
            display.blit_to(row, w, (x0, 0, x1 - x0, 1), (x0, y));
        }
    }
}

/// Show touches (SE3a): a 24 dp ring under each finger, straight to the screen over what is there (drawing only: the
/// touches were routed already). Only the circle's pixels are written, so a ring leaves its neighbours and the Back
/// hint around it alone. `drawn` gets the squares for [`restore_touches`].
#[allow(clippy::too_many_arguments)]
fn draw_touches(
    touches: &[(u8, i32, i32)],
    drawn: &mut Vec<Rect>,
    frame: &[u32],
    app: Option<(&[u32], usize, Rect)>,
    (w, h): (usize, usize),
    dp256: u32,
    row: &mut [u32],
    display: &Display,
) {
    let d = (24 * dp256 / 256) as i32;
    for &(_, cx, cy) in touches {
        let (left, top) = (cx - d / 2, cy - d / 2);
        // the ring's square, cut to the screen
        let (x0, y0) = (left.max(0), top.max(0));
        let (x1, y1) = ((left + d).min(w as i32), (top + d).min(h as i32));
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        for y in y0..y1 {
            let yu = y as usize;
            screen_span(frame, app, w, yu, (x0 as usize, x1 as usize), row);
            let (mut first, mut last) = (i32::MAX, i32::MIN);
            for x in x0..x1 {
                let px = &mut row[x as usize];
                if let Some(c) = neoui_render::touch_ring_pixel(x - left, y - top, d, *px) {
                    *px = c;
                    first = first.min(x);
                    last = last.max(x);
                }
            }
            if first <= last {
                let span = (last - first + 1) as usize;
                display.blit_to(row, w, (first as usize, 0, span, 1), (first as usize, yu));
            }
        }
        drawn.push(Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32));
    }
}

fn contains(r: &Rect, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w as i32 && y < r.y + r.h as i32
}

/// Put the foreground app's surface over the shell's body (less the open keyboard): all of it when the shell drew
/// under it, else the rectangles the app reported.
fn compose(host: &mut Host, shell_damage: &Damage, display: &Display) -> usize {
    let blits = core::mem::take(&mut host.blits);
    let body = host.surface_rect();
    let rows = host.surface_rows();
    let area = host.app_area();
    // the visible part of the surface, in surface coordinates
    let shown = Rect::new(0, 0, area.w, area.h);
    let refresh = core::mem::take(&mut host.refresh);
    let Some(surface) = host.front().and_then(|a| a.surface.as_ref()) else {
        return 0;
    };
    let bw = body.w as usize;
    // SAFETY: the chunk is `bw * rows * 4` bytes, committed at creation and mapped here; the app only writes pixels.
    let src = unsafe { core::slice::from_raw_parts(surface.base().cast::<u32>(), bw * rows) };
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

/// A raw kernel event as neoui input: button 1 is the (single) touch; keys go through `keyboard::hardware_key`.
fn to_input(ev: &TRawEvent) -> Option<InputEvent> {
    let touch = |phase| {
        // Raw pointer events are screen pixels on every device (Syborg's driver scaled x wrongly until NeoSymbian's
        // adaptation fix, f3b63c6); the UI takes the display size from the HAL and nothing else.
        let (x, y) = ev.pos();
        Some(InputEvent::Touch {
            id: ev.i_pointer_number,
            x,
            y,
            phase,
        })
    };
    // keys: keyboard::hardware_key (a layout per language, with the modifiers)
    match ev.i_type {
        raw::EBUTTON1_DOWN => touch(TouchPhase::Down),
        raw::EPOINTER_MOVE => touch(TouchPhase::Move),
        raw::EBUTTON1_UP => touch(TouchPhase::Up),
        _ => None,
    }
}

/// A short vibration (HAPTIC: the old apps' key pads): the kernel's `EHalGroupVariant` 0x4e27 (`qcom_telhal.cpp`,
/// `{ms, amplitude}`, CommDD). `KErrNotSupported` where there is no motor (QEMU).
pub(crate) fn haptic(ms: u32, amp: u32) -> i32 {
    let mut io = [ms, amp, 0u32, 0u32];
    // SAFETY: the function reads a 4-word {a, b, len, buf} block from a1 (len 0: no buffer); a2 is unused.
    unsafe { e32::sys::ui::hal_function(1, 0x4e27, io.as_mut_ptr().cast(), core::ptr::null_mut()) }
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
/// Whether the panel took it.
fn set_brightness(level: i32) -> bool {
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
    r == 0
}

/// Screen on or off (EDisplayHalSetState): the kernel sleeps or wakes the panel and the touchscreen. The front app
/// is told it is hidden or shown, and on wake its surface is composed again whole (TP3: `Host::screen_power`). What
/// the HAL says afterwards (a failed set leaves the old state), for D5.
fn set_screen(c: screen::Change, host: &mut Host) -> Option<bool> {
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
    host.screen_power(on);
    display_state()
}

/// The display HAL's on/off state (EDisplayHalState); `None` when it can't say.
fn display_state() -> Option<bool> {
    let mut state: i32 = -1;
    // SAFETY: a1 is a valid i32 out-parameter for EDisplayHalState; a2 is unused.
    let r = unsafe {
        e32::sys::ui::hal_function(
            HAL_DISPLAY,
            DISPLAY_STATE,
            (&mut state as *mut i32).cast(),
            core::ptr::null_mut(),
        )
    };
    match (r, state) {
        (0, 0 | 1) => Some(state == 1),
        _ => None,
    }
}

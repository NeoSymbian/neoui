// SPDX-License-Identifier: GPL-3.0-only
//! App processes (NeoSymbian R6, R10): the `NeoUi` IPC service apps talk to, and the apps that are running.
//!
//! An app (e.g. `NEOCLOCK.EXE`) is started when the shell first navigates to its key, connects to `NeoUi`, asks for
//! its geometry and its surface — an unnamed global chunk the server creates and hands over with
//! `complete_with_handle` (same address in both processes) — draws, reports `DAMAGE`, and keeps one `NEXT_EVENT`
//! outstanding, which the server completes with input, `Visible`, or `Quit`.
//!
//! Apps keep running when the shell moves away (R10): the one whose key is the current screen is the foreground app
//! (its surface is shown, it gets input and its keyboard and page apply); the others are told they are hidden.
//! They end when closed in the switcher, or when more than [`MAX_RUNNING`] run (the least recently used).
//! Launcher apps start hidden behind the shell grid; ENTRY data outlives their process, and FRONT switches
//! between that grid and their surface. A tile tap queues Entry even before the restarted app connects.

use crate::settings::{self, Settings};
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::ControlFlow;
use e32::chunk::Chunk;
use e32::dprintln;
use e32::fs::SharedFile;
use e32::process::Process;
use e32::server::{Message, Service};
use e32::sys::{TVersion, err};
use neoui_proto::Rect;
// the screen keys that are separate processes, their EXEs and command lines: shared with Settings' Apps page
use neoui_proto::apps::{APPS, LEGACY};
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::page::{NOTE_MAX, NOTIFICATION_MAX, PAGE_MAX, QUERY_MAX};
use neoui_proto::recents::Recents;
use neoui_proto::settings::value::ENCODED_MAX;
use neoui_proto::settings::{Caller, KEY_MAX, Value, access, kerr};
use neoui_proto::settings_app::info::OldApps;
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::{LauncherEntry, Nav, Note, Notification, Page, Query};

/// Apps whose shell screen is a launcher grid until they request FRONT(1).
// shortcut: a fixed table like APPS; apps declare launcher support when apps are installable.
pub const LAUNCHERS: &[&str] = &["legacy"];

/// Shell views that open an app on a page (`ui.show(app, view)`, from icons, notifications, Settings): (app, view,
/// page). The app's own numbering (NeoSymbian/phone `phone_core::page`).
// shortcut: a fixed table like APPS; apps declare their pages when apps are installable.
pub const PAGES: &[(&str, &str, u16)] = &[
    ("phone", "recents", 0),
    ("phone", "dial", 1),
    ("phone", "call", 2),
    ("phone", "settings", 3),
    ("phone", "speed", 4),
    // home's long-press menu (Edit home, Wallpaper) and the browser's Check Wi-Fi (neoui_proto::settings_app::page)
    ("settings", "wifi", 1),
    ("settings", "display", 5),
    ("settings", "navigation", 6),
];

/// Apps started (hidden) at boot and never closed for room: an incoming call must show at once; Messages posts a
/// shade entry per conversation as texts arrive.
pub const RESIDENT: &[&str] = &["phone", "messages"];

/// The browser's key, and its page for a Wi-Fi login (NeoSymbian/browser NEOWEB `PAGE_PORTAL`, UI5)
const BROWSER: &str = "browser";
const PORTAL_PAGE: u16 = 5;
/// The id of the shell's "Sign in to Wi-Fi network" entry (posted as the browser's; the browser numbers its own
/// download entries from 1)
const PORTAL_ENTRY: u32 = 0xE000_0001;
/// A resident app that ended is started again after this long.
const RESIDENT_RETRY_MS: u64 = 10_000;

/// The app key and page that screen `screen` with view `view` opens, if it is an app page.
pub fn page_for(screen: &str, view: &str) -> Option<(&'static str, u16)> {
    PAGES
        .iter()
        .find(|(k, v, _)| *k == screen && *v == view)
        .map(|(k, _, p)| (*k, *p))
}

/// Most apps kept running; starting one more asks the least recently used to quit.
// shortcut: a fixed count; size it by free RAM when the kernel reports it (a surface is ~2 MB on Syborg, ~7 MB on
// the phone).
const MAX_RUNNING: usize = 4;
/// An app asked to quit that is still running after this long is killed.
const QUIT_GRACE_MS: u64 = 3000;

pub struct App {
    pub key: &'static str,
    /// Its `NeoUi` session, once it connected.
    session: Option<u32>,
    pub process: Process,
    pub surface: Option<Chunk>,
    /// The app's outstanding `NEXT_EVENT`, if any.
    waiting: Option<Message>,
    queue: VecDeque<AppEvent>,
    /// When `Quit` was sent (ms): it is leaving, and is killed if it hasn't after [`QUIT_GRACE_MS`].
    quit_at: Option<u64>,
    /// The system keyboard mode it asked for (`KEYBOARD`: 0 none, 1 text, 2 terminal, 3 number, 4 phone); applies
    /// while it is in front.
    pub keyboard: u8,
    /// The keyboard height it was last told (the `KEYBOARD` reply or `AppEvent::Keyboard`).
    keyboard_h: u32,
    /// The page it declared (`PAGE`); applies while it is in front. None yet: Back leaves the app.
    pub declared: Option<Page>,
    /// Its open dialog (`QUERY`); shown while it is in front.
    pub query: Option<Query>,
    /// The legacy bridge's claims (`NAV`: `NAV_BACK | NAV_OPTIONS`).
    nav: i32,
    /// It has been on screen (the switcher lists only those; a resident app starts hidden).
    shown: bool,
    /// The argument of the last `OPEN` of it, until it reads it (`ARG`).
    arg: String,
    /// The file of the last `OPEN_FILE` of it, until it takes it (`ARG_FILE`).
    file: Option<SharedFile>,
    /// The app that last opened it (`OPEN`, `OPEN_FILE`): where a `DONE_FILE` result goes.
    opened_by: Option<&'static str>,
    /// The file an app it opened answered with (`DONE_FILE`), until it takes it (`RESULT_FILE`).
    result: Option<SharedFile>,
}

impl App {
    fn send(&mut self, e: AppEvent) {
        match self.waiting.take() {
            Some(m) => deliver(m, e),
            None => {
                // a move replaces the queued move of the same finger: a slow app catches up with the finger instead
                // of replaying its path
                if let AppEvent::Input(InputEvent::Touch {
                    id,
                    phase: TouchPhase::Move,
                    ..
                }) = e
                    && let Some(
                        last @ AppEvent::Input(InputEvent::Touch {
                            phase: TouchPhase::Move,
                            ..
                        }),
                    ) = self.queue.back_mut()
                    && matches!(last, AppEvent::Input(InputEvent::Touch { id: q, .. }) if *q == id)
                {
                    *last = e;
                    return;
                }
                self.queue.push_back(e);
            }
        }
    }

    /// Its page now (an app that declared none yet: Back leaves it unless the legacy bridge claimed it).
    pub fn page(&self) -> Page {
        self.declared.clone().unwrap_or_else(|| Page {
            can_back: self.nav & proto::NAV_BACK != 0,
            ..Page::default()
        })
    }

    /// The legacy bridge (a launcher, no kit): the shell draws a header over it until it declares a page (an old
    /// app in front: its key pad has the softkeys); ⋮ when it claimed Options.
    pub fn legacy(&self) -> (bool, bool) {
        (
            LAUNCHERS.contains(&self.key) && self.declared.is_none(),
            self.nav & proto::NAV_OPTIONS != 0,
        )
    }

    /// What the user did through the shell, as the event the app gets.
    pub fn event(&self, n: Nav) -> AppEvent {
        match n {
            Nav::Back => AppEvent::Back,
            Nav::Options => AppEvent::Options,
            Nav::Answer(i) => AppEvent::Answer { i },
            Nav::NoteAction => AppEvent::NoteAction,
        }
    }

    fn quit(&mut self, now_ms: u64) {
        if self.quit_at.is_none() {
            self.quit_at = Some(now_ms);
            self.send(AppEvent::Quit);
        }
    }
}

pub struct Host {
    /// Where app surfaces go on the screen (the shell's body on an app screen).
    pub body: Rect,
    /// The whole screen's height: a full-screen page's surface (surfaces are allocated this tall).
    screen_h: u32,
    /// Classic's strip shown for a moment over a full-screen page (a tap asked for it).
    pub peek: bool,
    /// The shell's design unit in 1/256 px, for apps.
    pub dp256: u32,
    /// The keyboard's height in pixels in each mode ([0, text, terminal, number, phone]); it changes with the
    /// language and settings, and is all 0 while the user put the keyboard away.
    keyboard_h: [u32; proto::KEYBOARD_MODES],
    /// The app in front asked for the keyboard (`KEYBOARD` with a mode above 0): the main loop brings it back if
    /// the user had put it away.
    pub reopen: bool,
    apps: Recents<App>,
    /// The selected shell screen when it is an app's key (including a hidden launcher).
    screen: Option<&'static str>,
    /// The app whose surface is in front.
    front: Option<&'static str>,
    /// FRONT asks the main loop to select the calling app's screen.
    pub requested_screen: Option<&'static str>,
    /// With [`Self::requested_screen`] from `OPEN`: the app that opened it (Back at its root returns there).
    pub requested_from: Option<&'static str>,
    /// What the Settings app reads with `SETTINGS_INFO` (kinds `INFO_BATTERY`, `INFO_NET`, `INFO_CLOCK` at 0..=2),
    /// kept fresh by the main loop ([`Self::set_info`]).
    info: [Vec<u8>; 3],
    /// The joined network wants its login page: the browser's argument for it (`<url>\n<network>`, UI5)
    portal: Option<String>,
    /// The front app said `DONE`: the main loop takes the shell's Back past it.
    pub leave: bool,
    /// Kept by app key, not process: installed entries survive exit and restart.
    entries: Vec<(&'static str, Vec<LauncherEntry>)>,
    pub entries_changed: bool,
    /// Entries refused for want of room (`ENTRIES_MAX`), by app key and id: Settings says "64 of N".
    refused: Vec<(&'static str, u32)>,
    next_session: u32,
    /// The foreground app's damage not yet shown (surface coordinates).
    pub blits: Vec<Rect>,
    /// Show the foreground app's whole surface on the next compose (it just came back).
    pub refresh: bool,
    /// The foreground app's `DAMAGE` calls (with their session), answered once that frame is on screen
    /// ([`Self::release_frames`]): the app doesn't draw over pixels the server hasn't copied yet.
    frames: Vec<(u32, Message)>,
    /// An app asked for a restart or power-off (`proto::POWER`); the main loop carries it out.
    pub power: Option<i32>,
    /// The settings store apps read, write and watch (`proto::SETTING_*`); the shell's own view syncs with it in
    /// the main loop.
    pub settings: Settings,
    /// When resident apps were last started.
    resident_at: Option<u64>,
    /// The toast on show (`NOTE`): the app that posted it, the note, and when it goes (ms).
    pub note: Option<(&'static str, Note, u64)>,
    /// The apps' entries in the shade (`NOTIFY`), newest first; kept when the app ends (a tap starts it again).
    pub notifications: Vec<(&'static str, Notification)>,
    /// The launcher's badge counters (`BADGE`): app, counter id, count (never 0).
    badges: Vec<(&'static str, u32, u32)>,
    /// The toast, the shade's entries or the badges changed: the main loop gives them to the shell.
    pub notes_changed: bool,
    pub badges_changed: bool,
    /// The clipboard (`CLIP_SET`, `CLIP_GET`), shared by every app.
    clipboard: String,
    /// What the clipboard holds (`proto::CLIP_TEXT`, `CLIP_PATHS_COPY`, `CLIP_PATHS_MOVE`; `CLIP_KIND`).
    clip_kind: i32,
}

impl Host {
    pub fn new(
        body: Rect,
        screen_h: u32,
        dp256: u32,
        keyboard_h: [u32; proto::KEYBOARD_MODES],
        settings: Settings,
    ) -> Self {
        Self {
            body,
            screen_h,
            peek: false,
            dp256,
            keyboard_h,
            reopen: false,
            apps: Recents::default(),
            screen: None,
            front: None,
            requested_screen: None,
            requested_from: None,
            info: Default::default(),
            portal: None,
            leave: false,
            entries: Vec::new(),
            entries_changed: false,
            refused: Vec::new(),
            next_session: 1,
            blits: Vec::new(),
            refresh: false,
            frames: Vec::new(),
            power: None,
            settings,
            resident_at: None,
            note: None,
            notifications: Vec::new(),
            badges: Vec::new(),
            notes_changed: false,
            badges_changed: false,
            clipboard: String::new(),
            clip_kind: proto::CLIP_TEXT,
        }
    }

    /// The header the shell draws over the legacy bridge, in pixels (48 dp).
    fn compat_header(&self) -> u32 {
        48 * self.dp256 / 256
    }

    /// The app area moved to `body` (a navigation mode switch: Buttons 639 dp, Gestures 679 dp; rule 16): every app
    /// that isn't full screen is told its new height (`AppEvent::Area`) and resizes once.
    pub fn set_body(&mut self, body: Rect) {
        if body == self.body {
            return;
        }
        self.body = body;
        let header = self.compat_header();
        for app in self.apps.iter_mut() {
            if app.quit_at.is_none() && !app.declared.as_ref().is_some_and(|p| p.full) {
                let h = if app.legacy().0 {
                    body.h - header
                } else {
                    body.h
                };
                app.send(AppEvent::Area { h });
            }
        }
        self.refresh = true;
    }

    /// The launcher's badge counts by app (its counters summed).
    pub fn badges(&self) -> Vec<(&'static str, u32)> {
        let mut out: Vec<(&'static str, u32)> = Vec::new();
        for (key, _, n) in &self.badges {
            match out.iter_mut().find(|(k, _)| k == key) {
                Some((_, sum)) => *sum = sum.saturating_add(*n),
                None => out.push((key, *n)),
            }
        }
        out
    }

    /// The toast's time is up: take it away. Call once per frame.
    pub fn expire_note(&mut self, now_ms: u64) {
        if self
            .note
            .as_ref()
            .is_some_and(|(_, _, until)| now_ms >= *until)
        {
            self.note = None;
            self.notes_changed = true;
        }
    }

    /// The user tapped the shade's entry `i` (`action` 0) or one of its actions (`i + 1`): its app comes to the
    /// front on the entry's page and is told which; the entry goes.
    pub fn open_notification(&mut self, i: usize, action: u16, now_ms: u64) {
        if i >= self.notifications.len() {
            return;
        }
        let (key, n) = self.notifications.remove(i);
        self.notes_changed = true;
        dprintln!(
            "neoui: notification {} of {} opened ({})",
            n.id,
            key,
            action
        );
        // the shell's own entry for a Wi-Fi login: the browser opens on the login page
        if key == BROWSER && n.id == PORTAL_ENTRY {
            self.open_portal(now_ms);
            return;
        }
        self.limit_running(key, now_ms);
        self.start(key);
        if action == 0 {
            self.post(key, AppEvent::Page { page: n.page });
        }
        self.post(key, AppEvent::Notified { id: n.id, action });
        self.requested_screen = Some(key);
    }

    /// The joined network wants a login page (`arg` = `<url>\n<network>`), or no longer does: the shade's "Sign in to
    /// Wi-Fi network" entry (posted as the browser's) comes or goes.
    pub fn set_portal(&mut self, arg: Option<String>) {
        if self.portal == arg {
            return;
        }
        let network = arg
            .as_deref()
            .and_then(|a| a.split_once('\n'))
            .map_or("", |(_, n)| n);
        let n = Notification {
            id: PORTAL_ENTRY,
            title: if arg.is_some() {
                "Sign in to Wi-Fi network".into()
            } else {
                String::new()
            },
            text: alloc::format!("'{network}' · tap to open"),
            page: PORTAL_PAGE,
            ..Notification::default()
        };
        let _ = notify(&mut self.notifications, BROWSER, n);
        self.notes_changed = true;
        self.portal = arg;
    }

    /// The Settings app's `SETTINGS_INFO` of `kind` (1..=3) is now `bytes` (`neoui_proto::settings_app::info`).
    pub fn set_info(&mut self, kind: i32, bytes: Vec<u8>) {
        if let Some(slot) = usize::try_from(kind - 1)
            .ok()
            .and_then(|i| self.info.get_mut(i))
        {
            *slot = bytes;
        }
    }

    /// Settings' Sign in, or the shade's entry: the browser on the network's login page.
    pub fn open_portal(&mut self, now_ms: u64) {
        if let Some(arg) = self.portal.clone() {
            self.open_app("settings", BROWSER, PORTAL_PAGE, arg, None, now_ms);
        }
    }

    /// App `from` opens app `key` on `page` with `arg` (ipc `OPEN`) or a file (`OPEN_FILE`): started or brought to
    /// the front, told, and shown with `from` as where Back returns.
    fn open_app(
        &mut self,
        from: &'static str,
        key: &'static str,
        page: u16,
        arg: String,
        file: Option<SharedFile>,
        now_ms: u64,
    ) {
        dprintln!(
            "neoui: {} opens {} page {}{}",
            from,
            key,
            page,
            if file.is_some() { " with a file" } else { "" }
        );
        self.limit_running(key, now_ms);
        self.start(key);
        if let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none()) {
            app.arg = arg;
            app.file = file;
            app.opened_by = Some(from);
            app.send(AppEvent::Open { page });
        }
        self.requested_screen = Some(key);
        self.requested_from = Some(from);
    }

    /// Start a [`RESIDENT`] app that isn't running (hidden), at most every [`RESIDENT_RETRY_MS`]. Call once per
    /// frame.
    pub fn keep_resident(&mut self, now_ms: u64) {
        if self
            .resident_at
            .is_some_and(|t| now_ms.saturating_sub(t) < RESIDENT_RETRY_MS)
        {
            return;
        }
        // one at a time, and not while an app is still connecting: `connect` binds the newest app without a session
        if self
            .apps
            .find(|a| a.session.is_none() && a.quit_at.is_none())
            .is_some()
        {
            return;
        }
        let Some(key) = RESIDENT
            .iter()
            .copied()
            .find(|k| self.apps.find(|a| a.key == *k).is_none())
        else {
            return;
        };
        self.resident_at = Some(now_ms);
        self.launch(key);
    }

    /// Answer the held `DAMAGE` calls: the frame they asked for is on screen (or won't be shown). Call once per server
    /// frame after compose, and on every path that skips it.
    pub fn release_frames(&mut self) {
        for (_, m) in self.frames.drain(..) {
            m.complete(0);
        }
    }

    /// Tells each app watching a changed setting (`AppEvent::Settings`). Call once per frame.
    pub fn deliver_settings(&mut self) {
        let (sessions, seq) = self.settings.take_notifications();
        for session in sessions {
            if let Some(app) = self.apps.find_mut(|a| a.session == Some(session)) {
                app.send(AppEvent::Settings { seq });
            }
        }
    }

    /// The foreground app, if it is on screen with its surface and not leaving.
    pub fn front(&self) -> Option<&App> {
        let key = self.front?;
        self.apps
            .find(|a| a.key == key && a.quit_at.is_none() && a.surface.is_some())
    }

    /// Whether the front app shows a full-screen page (no status bar, no strip).
    pub fn front_full(&self) -> bool {
        self.front()
            .is_some_and(|a| a.declared.as_ref().is_some_and(|p| p.full))
    }

    /// Where the front app's surface starts and how tall it shows (screen coordinates): the body, or the whole
    /// screen for a full-screen page (less Classic's strip while it peeks).
    pub fn surface_rect(&self) -> Rect {
        let b = self.body;
        if self.front().is_some_and(|a| a.legacy().0) {
            // under the header the shell draws for it
            let header = self.compat_header();
            return Rect::new(b.x, b.y + header as i32, b.w, b.h - header);
        }
        if !self.front_full() {
            return b;
        }
        let strip = self.screen_h - (b.y.max(0) as u32 + b.h);
        Rect::new(
            b.x,
            0,
            b.w,
            self.screen_h - if self.peek { strip } else { 0 },
        )
    }

    /// The part of the screen the app's surface shows in: [`Host::surface_rect`] but the open keyboard.
    pub fn app_area(&self) -> Rect {
        let covered = self
            .front()
            .map_or(0, |a| self.keyboard_h[usize::from(a.keyboard)]);
        let r = self.surface_rect();
        Rect::new(r.x, r.y, r.w, r.h.saturating_sub(covered))
    }

    /// The front app's surface (`body.w` per row, [`Host::surface_rows`] rows).
    pub fn surface_rows(&self) -> usize {
        self.screen_h as usize
    }

    /// The keyboard's heights now ([`Renderer::keyboard_heights`](neoui_render::Renderer::keyboard_heights)): each
    /// app with the keyboard open whose height changed is told (`AppEvent::Keyboard`). Call once per frame; pass all
    /// 0 while the user put the keyboard away (the app gets its whole body back).
    pub fn set_keyboard_heights(&mut self, heights: [u32; proto::KEYBOARD_MODES]) {
        self.keyboard_h = heights;
        let front = self.front;
        for app in self.apps.iter_mut() {
            let h = heights[usize::from(app.keyboard)];
            if app.keyboard != 0 && app.keyboard_h != h && app.quit_at.is_none() {
                app.keyboard_h = h;
                app.send(AppEvent::Keyboard { h });
                if front == Some(app.key) {
                    self.refresh = true; // the part the keyboard gave back (or now covers) shows again
                }
            }
        }
    }

    /// The shell moved to `screen`: hide the app that was in front, and show (or start) the one for `screen`.
    pub fn set_screen(&mut self, screen: &str, now_ms: u64) {
        let key = APPS.iter().find(|a| a.key == screen).map(|a| a.key);
        if self.screen == key {
            return;
        }
        self.screen = key;
        self.set_front(key.filter(|k| !LAUNCHERS.contains(k)));
        let Some(key) = key else { return };
        self.limit_running(key, now_ms);
        self.start(key);
    }

    /// Switch surface visibility independently of the shell's selected screen.
    fn set_front(&mut self, key: Option<&'static str>) {
        if self.front == key {
            return;
        }
        if let Some(old) = self.front.take()
            && let Some(app) = self.apps.find_mut(|a| a.key == old && a.quit_at.is_none())
        {
            app.send(AppEvent::Visible(false));
        }
        self.blits.clear();
        self.release_frames();
        self.front = key;
        if let Some(key) = key
            && let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none())
        {
            app.send(AppEvent::Visible(true));
        }
        self.refresh = true;
    }

    fn start(&mut self, key: &'static str) {
        if self.apps.touch(|a| a.key == key && a.quit_at.is_none()) {
            dprintln!("neoui: app {} resumed", key);
        } else {
            self.launch(key);
        }
        if let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none()) {
            app.shown = true;
        }
    }

    /// Close the apps used longest ago (never a resident) so that `key` runs within [`MAX_RUNNING`]: before
    /// starting it, so the new app has the memory (the closed ones get [`QUIT_GRACE_MS`] to save).
    fn limit_running(&mut self, key: &str, now_ms: u64) {
        let running = self
            .apps
            .find(|a| a.key == key && a.quit_at.is_none())
            .is_some();
        let limit = MAX_RUNNING - usize::from(!running);
        let over: Vec<&'static str> = self
            .apps
            .over_limit(
                limit,
                // a resident never opened (or dismissed) holds no slot: it has no surface yet
                |a| a.quit_at.is_some() || (RESIDENT.contains(&a.key) && !a.shown),
                |a| a.key == key || RESIDENT.contains(&a.key),
            )
            .map(|a| a.key)
            .collect();
        for k in over {
            dprintln!("neoui: app {} closed (too many running)", k);
            self.close(k, now_ms);
        }
    }

    fn launch(&mut self, key: &'static str) {
        let (exe, args) = APPS
            .iter()
            .find(|a| a.key == key)
            .map_or(("", ""), |a| (a.exe, a.args));
        match Process::spawn(exe, args) {
            Ok(process) => {
                process.resume();
                dprintln!("neoui: app {} started", key);
                self.apps.push(App {
                    key,
                    session: None,
                    process,
                    surface: None,
                    waiting: None,
                    // A launcher (like a resident) starts hidden, even if its grid is selected.
                    queue: VecDeque::from([AppEvent::Visible(self.front == Some(key))]),
                    quit_at: None,
                    keyboard: 0,
                    keyboard_h: 0,
                    declared: None,
                    query: None,
                    nav: 0,
                    shown: false,
                    arg: String::new(),
                    file: None,
                    opened_by: None,
                    result: None,
                });
            }
            Err(e) => dprintln!("neoui: app {} failed to start ({})", key, e),
        }
    }

    pub fn entries(&self, key: &str) -> &[LauncherEntry] {
        self.entries
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(&[], |(_, entries)| entries.as_slice())
    }

    /// A cached tile can restart its owner; the normal event queue holds the UID until it connects.
    /// The legacy bridge's entries for Settings' Apps page (`INFO_APPS`), with how many it had to refuse.
    fn old_apps(&self) -> OldApps {
        let apps: Vec<(u32, String)> = self
            .entries(LEGACY)
            .iter()
            .map(|e| (e.id, e.caption.clone()))
            .collect();
        let refused = self.refused.iter().filter(|(k, _)| *k == LEGACY).count();
        OldApps {
            total: u16::try_from(apps.len() + refused).unwrap_or(u16::MAX),
            apps,
        }
    }

    /// Start launcher app `key` if needed and tell it entry `id` was chosen; false if it isn't one, or didn't start.
    pub fn open_entry(&mut self, key: &str, id: u32, now_ms: u64) -> bool {
        let Some(key) = APPS.iter().find(|a| a.key == key).map(|a| a.key) else {
            return false;
        };
        if !self.entries(key).iter().any(|entry| entry.id == id) {
            return false;
        }
        self.limit_running(key, now_ms);
        self.start(key);
        self.post(key, AppEvent::Entry { id })
    }

    /// Sends `e` to app `key` (queued until it asks for events); false when it is not running (or leaving).
    pub fn post(&mut self, key: &str, e: AppEvent) -> bool {
        self.apps
            .find_mut(|a| a.key == key && a.quit_at.is_none())
            .map(|app| app.send(e))
            .is_some()
    }

    /// What the user did through the shell (Back, a dialog's button, the toast's action) goes to its app: the toast's
    /// to the app that posted it, the rest to the foreground app.
    pub fn send_nav(&mut self, n: Nav) {
        dprintln!("neoui: nav {:?}", n);
        if n == Nav::NoteAction {
            if let Some((key, _, _)) = self.note.take() {
                self.notes_changed = true;
                self.post(key, AppEvent::NoteAction);
            }
            return;
        }
        let Some(key) = self.front else { return };
        if let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none()) {
            if let Nav::Answer(_) = n {
                app.query = None; // answered: the shell closed it
            }
            let e = app.event(n);
            app.send(e);
        }
    }

    pub fn send(&mut self, e: AppEvent) {
        let Some(key) = self.front else { return };
        if let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none()) {
            app.send(e);
        }
    }

    /// Ask app `key` to quit (the switcher's ×, or too many running). A resident app (the phone) is only
    /// dismissed: it leaves Open apps and keeps running.
    pub fn close(&mut self, key: &str, now_ms: u64) {
        if let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none()) {
            if RESIDENT.contains(&app.key) {
                dprintln!("neoui: app {} dismissed", app.key);
                app.shown = false;
            } else {
                app.quit(now_ms);
            }
        }
    }

    /// Running apps, most recent first (not those leaving, nor a resident app never opened), for the switcher.
    pub fn running(&self) -> Vec<&'static str> {
        self.apps
            .newest_first()
            .filter(|a| a.quit_at.is_none() && a.shown)
            .map(|a| a.key)
            .collect()
    }

    /// Forget apps whose process ended (and say which), and kill those that were asked to quit too long ago.
    pub fn reap(&mut self, now_ms: u64) -> Vec<&'static str> {
        for app in self.apps.iter_mut() {
            if let Some(at) = app.quit_at
                && now_ms.saturating_sub(at) > QUIT_GRACE_MS
                && app.process.exit_info().is_none()
            {
                dprintln!("neoui: app {} killed (did not quit)", app.key);
                app.process.kill(0);
                app.quit_at = Some(u64::MAX / 2); // don't kill twice
            }
        }
        let gone = self.apps.remove(|a| a.process.exit_info().is_some());
        // A cached tile can restart an app while its previous process is still quitting. Reaping the old
        // process must not hide the replacement's surface.
        if let Some(key) = self.front
            && gone.iter().any(|a| a.key == key)
            && self
                .apps
                .find(|a| a.key == key && a.quit_at.is_none())
                .is_none()
        {
            self.set_front(None);
        }
        gone.iter()
            .map(|app| {
                let (t, r) = app.process.exit_info().unwrap_or((0, 0));
                dprintln!("neoui: app {} exited (type {}, reason {})", app.key, t, r);
                app.key
            })
            .collect()
    }
}

fn deliver(m: Message, e: AppEvent) {
    let r = m.write8(0, &e.encode(), 0);
    m.complete(r.map_or_else(|e| e, |()| 0));
}

/// Reads descriptor argument `param`: at most `max` bytes.
fn read_bytes(m: &Message, param: usize, max: usize) -> Result<Vec<u8>, i32> {
    if m.des_len(param)? > max {
        return Err(kerr::TOO_BIG);
    }
    m.read8_vec(param)
}

/// Reads descriptor argument `param` as text of at most `max` bytes.
fn read_text(m: &Message, param: usize, max: usize) -> Result<String, i32> {
    if m.des_len(param)? > max {
        return Err(kerr::TOO_BIG);
    }
    String::from_utf8(m.read8_vec(param)?).map_err(|_| kerr::ARGUMENT)
}

/// Validate descriptors before changing the cache, including removals and replacements at capacity.
/// The most refused launcher entries remembered per app (`INFO_APPS`' total counts no more).
const REFUSED_MAX: usize = 1024;

fn entry(
    entries: &mut Vec<(&'static str, Vec<LauncherEntry>)>,
    refused: &mut Vec<(&'static str, u32)>,
    key: &'static str,
    m: &Message,
) -> Result<i32, i32> {
    let icon_max = 4 + usize::from(proto::ENTRY_ICON_MAX).pow(2) * 4;
    if m.des_len(1).map_err(|_| kerr::ARGUMENT)? > proto::TITLE_MAX
        || m.des_len(2).map_err(|_| kerr::ARGUMENT)? > icon_max
    {
        return Err(kerr::ARGUMENT);
    }
    let caption = String::from_utf8(m.read8_vec(1).map_err(|_| kerr::ARGUMENT)?)
        .map_err(|_| kerr::ARGUMENT)?;
    let icon = m.read8_vec(2).map_err(|_| kerr::ARGUMENT)?;
    proto::decode_entry_icon(&icon).map_err(|_| kerr::ARGUMENT)?;
    let id = m.int(0) as u32;
    if caption.is_empty() {
        refused.retain(|r| *r != (key, id));
    }
    let index = match entries.iter().position(|(k, _)| *k == key) {
        Some(index) => index,
        None if caption.is_empty() => return Ok(0),
        None => {
            entries.push((key, Vec::new()));
            entries.len() - 1
        }
    };
    let entries = &mut entries[index].1;
    if let Some(index) = entries.iter().position(|entry| entry.id == id) {
        if caption.is_empty() {
            entries.remove(index);
        } else {
            entries[index] = LauncherEntry { id, caption, icon };
        }
    } else if !caption.is_empty() {
        if entries.len() >= proto::ENTRIES_MAX {
            // remembered so Settings can say how many there are (at most REFUSED_MAX per app: the total says "at least")
            let mine = refused.iter().filter(|(k, _)| *k == key).count();
            if !refused.contains(&(key, id)) && mine < REFUSED_MAX {
                refused.push((key, id));
            }
            return Err(kerr::OVERFLOW);
        }
        refused.retain(|r| *r != (key, id));
        entries.push(LauncherEntry { id, caption, icon });
    }
    Ok(0)
}

/// Adds, replaces or (empty title) removes app `key`'s shade entry; the newest goes first.
fn notify(
    list: &mut Vec<(&'static str, Notification)>,
    key: &'static str,
    n: Notification,
) -> Result<i32, i32> {
    let old = list.iter().position(|(k, o)| *k == key && o.id == n.id);
    if let Some(i) = old {
        list.remove(i);
    }
    if n.title.is_empty() {
        return Ok(0);
    }
    if list.iter().filter(|(k, _)| *k == key).count() >= proto::NOTIFICATIONS_MAX {
        return Err(kerr::OVERFLOW);
    }
    list.insert(0, (key, n));
    Ok(0)
}

/// `CLIP_GET [out text]` → the text's length.
fn clip_get(clip: &str, m: &Message) -> Result<i32, i32> {
    if clip.len() > m.des_max_len(0)? {
        return Err(kerr::OVERFLOW);
    }
    m.write8(0, clip.as_bytes(), 0)?;
    i32::try_from(clip.len()).map_err(|_| kerr::OVERFLOW)
}

/// `SETTING_GET [key, out value]` → the value's length.
fn setting_get(settings: &Settings, m: &Message) -> Result<i32, i32> {
    let key = read_text(m, 0, KEY_MAX)?;
    let value = settings
        .get_raw(&key)
        .map_err(|e| e.to_symbian())?
        .to_bytes();
    if value.len() > m.des_max_len(1)? {
        return Err(kerr::OVERFLOW);
    }
    m.write8(1, &value, 0)?;
    // at most ENCODED_MAX bytes, so it fits
    i32::try_from(value.len()).map_err(|_| kerr::OVERFLOW)
}

/// `SETTINGS_INFO [kind, cursor, out]` → the snapshot's length.
fn settings_info(info: &[Vec<u8>; 3], m: &Message) -> Result<i32, i32> {
    let bytes = m
        .int(0)
        .checked_sub(1)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| info.get(i))
        .ok_or(kerr::ARGUMENT)?;
    write_info(m, bytes)
}

/// A `SETTINGS_INFO` answer into the caller's `out` (slot 2): its length, `KErrOverflow` when it doesn't fit.
fn write_info(m: &Message, bytes: &[u8]) -> Result<i32, i32> {
    if bytes.len() > m.des_max_len(2)? {
        return Err(kerr::OVERFLOW);
    }
    m.write8(2, bytes, 0)?;
    i32::try_from(bytes.len()).map_err(|_| kerr::OVERFLOW)
}

/// `SETTING_SET [key, value]` → 0.
fn setting_set(settings: &mut Settings, caller: Caller<'_>, m: &Message) -> Result<i32, i32> {
    let key = read_text(m, 0, KEY_MAX)?;
    if m.des_len(1)? > ENCODED_MAX {
        return Err(kerr::TOO_BIG);
    }
    let value = Value::decode(&m.read8_vec(1)?).map_err(|e| e.to_symbian())?;
    settings::set(settings, caller, &key, value, e32::time::millis())
        .map(|_| 0)
        .map_err(|e| e.to_symbian())
}

/// `SETTING_WATCH [prefix]` → 0.
fn setting_watch(settings: &mut Settings, session: u32, m: &Message) -> Result<i32, i32> {
    let prefix = read_text(m, 0, KEY_MAX)?;
    settings
        .watch(session, &prefix)
        .map(|()| 0)
        .map_err(|e| e.to_symbian())
}

impl Service for Host {
    /// Which app the session is.
    type Session = u32;

    fn connect(&mut self, version: TVersion) -> Result<u32, i32> {
        if version.major != proto::VERSION.0 {
            return Err(err::KERR_NOT_SUPPORTED);
        }
        // The newest app without a session is the one connecting: apps start one at a time.
        // shortcut: identify the client's process (MessageClient) if two launches ever overlap.
        let id = self.next_session;
        let app = self.apps.iter_mut().rev().find(|a| a.session.is_none());
        let Some(app) = app else {
            return Err(err::KERR_NOT_SUPPORTED);
        };
        app.session = Some(id);
        self.next_session += 1;
        Ok(id)
    }

    fn message(&mut self, id: &mut u32, m: Message) {
        let (body, dp256, keyboard_h, front) = (self.body, self.dp256, self.keyboard_h, self.front);
        let header = self.compat_header();
        let screen_h = self.screen_h;
        let Some(app) = self.apps.find_mut(|a| a.session == Some(*id)) else {
            return m.complete(err::KERR_NOT_SUPPORTED);
        };
        match m.function() {
            proto::GEOMETRY => {
                let mut g = [0u8; proto::GEOMETRY_LEN];
                // the legacy bridge sits under the header the shell draws for it
                let h = if app.legacy().0 {
                    body.h - header
                } else {
                    body.h
                };
                for (i, v) in [body.w, h, dp256].iter().enumerate() {
                    g[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                let r = m.write8(0, &g, 0);
                m.complete(r.map_or_else(|e| e, |()| 0));
            }
            proto::SURFACE => {
                if app.surface.is_none() {
                    // as tall as the screen: a full-screen page draws there too (GEOMETRY still gives the body)
                    match Chunk::create_global(body.w as usize * screen_h as usize * 4) {
                        Ok(c) => app.surface = Some(c),
                        Err(e) => return m.complete(e),
                    }
                    dprintln!("neoui: app {} surface {}x{}", app.key, body.w, body.h);
                }
                let h = app.surface.as_ref().map_or(0, Chunk::handle);
                m.complete_with_handle(h);
            }
            proto::DAMAGE => {
                // a background app's drawing shows when it comes back (the whole surface is shown then)
                if front == Some(app.key) {
                    self.blits.push(Rect::new(
                        m.int(0),
                        m.int(1),
                        m.int(2) as u32,
                        m.int(3) as u32,
                    ));
                    self.frames.push((*id, m)); // answered after compose
                } else {
                    m.complete(0);
                }
            }
            proto::KEYBOARD => {
                // an unknown mode is the text keyboard (1.2 apps sent show = 1)
                app.keyboard = match m.int(0) {
                    proto::KEYBOARD_HIDDEN => 0,
                    proto::KEYBOARD_TERMINAL => 2,
                    proto::KEYBOARD_NUMBER => 3,
                    proto::KEYBOARD_PHONE => 4,
                    _ => 1,
                };
                if app.keyboard != 0 && front == Some(app.key) {
                    self.reopen = true;
                }
                app.keyboard_h = keyboard_h[usize::from(app.keyboard)];
                dprintln!("neoui: keyboard {} for {}", app.keyboard, app.key);
                m.complete(app.keyboard_h as i32);
            }
            proto::NAV => {
                app.nav = m.int(0) & (proto::NAV_BACK | proto::NAV_OPTIONS);
                m.complete(0);
            }
            proto::PAGE => match read_bytes(&m, 0, PAGE_MAX)
                .and_then(|b| Page::decode(&b).map_err(|_| kerr::ARGUMENT))
            {
                Ok(p) => {
                    let h = if p.full { screen_h } else { body.h };
                    if front == Some(app.key)
                        && app.declared.as_ref().map(|d| d.full) != Some(p.full)
                    {
                        self.refresh = true; // the surface moves (full screen or back)
                    }
                    app.declared = Some(p);
                    m.complete(h as i32);
                }
                Err(e) => m.complete(e),
            },
            proto::QUERY => match read_bytes(&m, 0, QUERY_MAX) {
                Ok(b) if b.is_empty() => {
                    app.query = None;
                    m.complete(0);
                }
                Ok(b) => match Query::decode(&b) {
                    Ok(q) => {
                        app.query = Some(q);
                        m.complete(0);
                    }
                    Err(_) => m.complete(kerr::ARGUMENT),
                },
                Err(e) => m.complete(e),
            },
            proto::NOTE => match read_bytes(&m, 0, NOTE_MAX)
                .and_then(|b| Note::decode(&b).map_err(|_| kerr::ARGUMENT))
            {
                Ok(n) => {
                    if n.text.is_empty() {
                        if self.note.as_ref().is_some_and(|(k, _, _)| *k == app.key) {
                            self.note = None;
                        }
                    } else {
                        let until = e32::time::millis() + proto::NOTE_MS;
                        self.note = Some((app.key, n, until));
                    }
                    self.notes_changed = true;
                    m.complete(0);
                }
                Err(e) => m.complete(e),
            },
            proto::NOTIFY => {
                let key = app.key;
                let r = read_bytes(&m, 0, NOTIFICATION_MAX)
                    .and_then(|b| Notification::decode(&b).map_err(|_| kerr::ARGUMENT))
                    .and_then(|n| notify(&mut self.notifications, key, n));
                if r.is_ok() {
                    self.notes_changed = true;
                }
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::BADGE => {
                let (key, id, count) = (app.key, m.int(0) as u32, m.int(1) as u32);
                self.badges.retain(|(k, i, _)| !(*k == key && *i == id));
                if count > 0 {
                    self.badges.push((key, id, count));
                }
                self.badges_changed = true;
                m.complete(0);
            }
            proto::CLIP_SET => {
                let kind = m.int(1);
                if !(proto::CLIP_TEXT..=proto::CLIP_PATHS_MOVE).contains(&kind) {
                    return m.complete(kerr::ARGUMENT);
                }
                match read_text(&m, 0, proto::CLIP_MAX) {
                    Ok(t) => {
                        self.clipboard = t;
                        self.clip_kind = kind;
                        m.complete(0);
                    }
                    Err(e) => m.complete(e),
                }
            }
            proto::CLIP_KIND => m.complete(if self.clipboard.is_empty() {
                proto::CLIP_TEXT
            } else {
                self.clip_kind
            }),
            proto::CLIP_GET => {
                let r = clip_get(&self.clipboard, &m);
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::POWER => {
                dprintln!("neoui: power {} asked by {}", m.int(0), app.key);
                self.power = Some(m.int(0));
                m.complete(0);
            }
            proto::SETTING_GET => {
                let r = setting_get(&self.settings, &m);
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::SETTING_SET => {
                let caller = access::caller(app.key, m.client_sid());
                let r = setting_set(&mut self.settings, caller, &m);
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::SETTING_WATCH => {
                let r = setting_watch(&mut self.settings, *id, &m);
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::SETTINGS_INFO if m.int(0) == proto::INFO_APPS => {
                let r = write_info(&m, &self.old_apps().encode());
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::SETTINGS_INFO => {
                let r = settings_info(&self.info, &m);
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::SETTINGS_ACTION => {
                let r = match m.int(0) {
                    proto::ACTION_SIGN_IN if self.portal.is_some() => {
                        dprintln!("neoui: {} asks to sign in to the Wi-Fi network", app.key);
                        self.open_portal(e32::time::millis());
                        0
                    }
                    proto::ACTION_SIGN_IN => err::KERR_NOT_READY,
                    // open an old app: Settings only (by its secure ID), an id the legacy bridge registered
                    proto::ACTION_OPEN_ENTRY => {
                        if access::caller(app.key, m.client_sid()) != Caller::Settings {
                            kerr::PERMISSION_DENIED
                        } else {
                            let mut b = [0u8; 4];
                            match (m.des_len(1), m.read8(1, &mut b, 0)) {
                                (Ok(4), Ok(4)) => {
                                    let id = u32::from_le_bytes(b);
                                    if !self.entries(LEGACY).iter().any(|e| e.id == id) {
                                        err::KERR_NOT_FOUND
                                    } else if self.open_entry(LEGACY, id, e32::time::millis()) {
                                        dprintln!("neoui: settings opens old app {:08x}", id);
                                        0
                                    } else {
                                        err::KERR_NOT_READY // the bridge didn't start
                                    }
                                }
                                _ => kerr::ARGUMENT,
                            }
                        }
                    }
                    _ => kerr::ARGUMENT,
                };
                m.complete(r);
            }
            proto::ENTRY => {
                let r = entry(&mut self.entries, &mut self.refused, app.key, &m);
                if r.is_ok() {
                    self.entries_changed = true;
                }
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::FRONT => {
                if app.quit_at.is_some() {
                    return m.complete(err::KERR_CANCEL);
                }
                let show = match m.int(0) {
                    0 => false,
                    1 => true,
                    _ => return m.complete(kerr::ARGUMENT),
                };
                let key = app.key;
                // Select the caller's shell screen without briefly foregrounding an ordinary app on FRONT(0).
                let was_front = self.front == Some(key);
                dprintln!("neoui: app {} front {}", key, show);
                self.screen = Some(key);
                self.set_front(if show { Some(key) } else { None });
                if show == was_front {
                    // set_front only sends visibility changes; each FRONT also acknowledges its caller's state.
                    self.post(key, AppEvent::Visible(show));
                }
                self.limit_running(key, e32::time::millis());
                self.start(key);
                self.requested_screen = Some(key);
                m.complete(0);
            }
            proto::OPEN | proto::OPEN_FILE => {
                let from = app.key;
                let key = read_text(&m, 0, proto::KEY_MAX).ok();
                let target = key.and_then(|k| APPS.iter().find(|a| a.key == k).map(|a| a.key));
                let Some(target) = target else {
                    return m.complete(err::KERR_NOT_FOUND);
                };
                let Ok(page) = u16::try_from(m.int(1)) else {
                    return m.complete(kerr::ARGUMENT);
                };
                let (arg, file) = if m.function() == proto::OPEN {
                    match read_text(&m, 2, proto::ARG_MAX) {
                        Ok(arg) => (arg, None),
                        Err(_) => return m.complete(kerr::ARGUMENT),
                    }
                } else {
                    match SharedFile::adopt_from_client(&m, 2, 3) {
                        Ok(f) => (String::new(), Some(f)),
                        Err(e) => {
                            dprintln!("neoui: {} open file failed {}", from, e);
                            return m.complete(e);
                        }
                    }
                };
                self.open_app(from, target, page, arg, file, e32::time::millis());
                m.complete(0);
            }
            proto::ARG_FILE | proto::RESULT_FILE => {
                let slot = if m.function() == proto::ARG_FILE {
                    &mut app.file
                } else {
                    &mut app.result
                };
                match slot.take() {
                    // the server's copy closes when `f` drops; the app has its own handle by then
                    Some(f) => drop(f.transfer_to_client(m, 0)),
                    None => m.complete(err::KERR_NOT_FOUND),
                }
            }
            proto::DONE_FILE => {
                let file = match SharedFile::adopt_from_client(&m, 0, 1) {
                    Ok(f) => f,
                    Err(e) => return m.complete(e),
                };
                let (key, opener) = (app.key, app.opened_by);
                if front == Some(key) {
                    dprintln!("neoui: app {} done with a file for {:?}", key, opener);
                    self.leave = true;
                }
                match opener.and_then(|o| self.apps.find_mut(|a| a.key == o && a.quit_at.is_none()))
                {
                    Some(o) => {
                        o.result = Some(file);
                        m.complete(0);
                    }
                    None => m.complete(err::KERR_NOT_FOUND),
                }
            }
            proto::ARG => {
                let r = if app.arg.len() > m.des_max_len(0).unwrap_or(0) {
                    Err(kerr::OVERFLOW)
                } else {
                    m.write8(0, app.arg.as_bytes(), 0)
                        .map(|()| core::mem::take(&mut app.arg).len() as i32)
                };
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::HAPTIC => {
                let ms = m.int(0).clamp(0, proto::HAPTIC_MS_MAX);
                let amp = m.int(1).clamp(0, 127);
                m.complete(crate::haptic(ms as u32, amp as u32));
            }
            proto::DONE => {
                // only the app in front can hand the screen back
                if front == Some(app.key) {
                    dprintln!("neoui: app {} done", app.key);
                    self.leave = true;
                }
                m.complete(0);
            }
            proto::NEXT_EVENT => match app.queue.pop_front() {
                Some(e) => deliver(m, e),
                None => app.waiting = Some(m),
            },
            _ => m.complete(err::KERR_NOT_SUPPORTED),
        }
    }

    fn disconnect(&mut self, id: u32) -> ControlFlow<()> {
        self.settings.drop_session(id);
        // Complete what the session still has outstanding before its disconnect completes.
        let (gone, kept): (Vec<_>, Vec<_>) = self.frames.drain(..).partition(|(s, _)| *s == id);
        self.frames = kept;
        for (_, m) in gone {
            m.complete(err::KERR_CANCEL);
        }
        if let Some(m) = self
            .apps
            .find_mut(|a| a.session == Some(id))
            .and_then(|a| a.waiting.take())
        {
            m.complete(err::KERR_CANCEL);
        }
        ControlFlow::Continue(())
    }
}

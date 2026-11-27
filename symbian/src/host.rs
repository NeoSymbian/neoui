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
use neoui_proto::apps::{self, APPS, LEGACY};
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::page::{NOTE_MAX, NOTIFICATION_MAX, PAGE_MAX, QUERY_MAX, action};
use neoui_proto::recents::Recents;
use neoui_proto::settings::value::ENCODED_MAX;
use neoui_proto::settings::{Caller, KEY_MAX, Value, access, kerr, keys};
use neoui_proto::settings_app::info::{OldApps, encode_running};
use neoui_proto::settings_app::privacy::{self, ClipNotices};
use neoui_proto::sound_profile;
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::{LauncherEntry, Nav, Note, Notification, Page, Query};

// 1.20's paged Settings kinds, owner work and thumbnails (a child module: it works on the apps here)
#[path = "notifications.rs"]
pub mod notifications;
#[path = "settings_info.rs"]
mod settings_info;
// (D10) Settings' Edit home and Open apps' thumbnails
#[path = "home.rs"]
mod home;

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
    // D7: home's long-press Wallpaper opens the picker itself
    ("settings", "wallpaper", 25),
    // TP4: the shade's Manage opens Panel & notifications
    ("settings", "panel", 27),
];

/// Apps started (hidden) at boot, in this order, and never closed for room: DTSRV (a service, 1.19) rings alarms
/// whatever runs; an incoming call must show at once; Messages posts a shade entry per conversation as texts arrive.
pub const RESIDENT: &[&str] = &["dtsrv", "phone", "messages"];

/// The browser's key, and its page for a Wi-Fi login (NeoSymbian/browser NEOWEB `PAGE_PORTAL`, UI5)
const BROWSER: &str = "browser";
const PORTAL_PAGE: u16 = 5;
/// The id of the shell's "Sign in to Wi-Fi network" entry (posted as the browser's; the browser numbers its own
/// download entries from 1)
const PORTAL_ENTRY: u32 = 0xE000_0001;
/// The clipboard toast's key in `Host::note`: the shell's own, so no app's NOTE or action touches it (D8).
const CLIP_NOTE: &str = "clipboard";
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
    /// Its `NeoUi` session, once it connected, and the protocol minor version it connected with (1.18's
    /// `NotifyGone` would end an older app: its decoder refuses unknown events).
    session: Option<u32>,
    minor: u8,
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
        // an older decoder refuses a newer event, and its app ends
        if self.minor < e.needs_minor() {
            return;
        }
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
                // one hint per kind is enough: it carries the newest revision
                if let AppEvent::SettingsInfo { kind, .. } = e
                    && let Some(old) = self
                        .queue
                        .iter_mut()
                        .find(|q| matches!(q, AppEvent::SettingsInfo { kind: k, .. } if *k == kind))
                {
                    *old = e;
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
    /// A service's `OPEN` with `OPEN_WAKE` (1.19): the main loop turns the screen on.
    pub wake: bool,
    /// (D6) Finger 0's presses delivered to an app, and which app got the last one: a `CONTROL_FEEDBACK` answers one.
    presses: neoui_proto::effects::PressGate,
    press_app: Option<&'static str>,
    /// (D6) An accepted control the main loop answers (`[kind, key]` as `CONTROL_FEEDBACK` carries them).
    pub feedback: Option<(i32, u8)>,
    /// What the Settings app reads with `SETTINGS_INFO` (kinds `INFO_BATTERY`, `INFO_NET`, `INFO_CLOCK` at 0..=2,
    /// `INFO_BATTERY_USAGE` at 4; `INFO_APPS` is answered from the bridge's entries),
    /// kept fresh by the main loop ([`Self::set_info`]).
    info: [Vec<u8>; 5],
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
    /// Sessions connected but not yet bound to their app, with their protocol minor ([`Host::bind`]).
    unbound: Vec<(u32, u8)>,
    /// The foreground app's damage not yet shown (surface coordinates).
    pub blits: Vec<Rect>,
    /// Show the foreground app's whole surface on the next compose (it just came back).
    pub refresh: bool,
    /// Copy the shell's whole frame to the screen at the next frame (`proto::REPAINT`: the kernel console drew over it).
    pub repaint: bool,
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
    /// 1.20: revisions, Settings' subscriptions, owner work and thumbnails ([`settings_info`]).
    shelf: settings_info::Shelf,
    /// (D8) The "Files read the clipboard" toasts (`privacy.clip_notice`): readers cooling down and waiting.
    clip_notices: ClipNotices,
    /// The clipboard toast on show hasn't reached the screen yet: its time starts when it does
    /// ([`Self::note_displayed`]), and a toast that replaces it first sends it back to the queue.
    clip_unseen: bool,
    /// (TP4) The apps' channels, the rate limit and the newest card's alert ([`notifications`]).
    pub policies: notifications::Policies,
    /// (D10) the home editor asked for, thumbnails taken ([`home`])
    pub d10: home::D10,
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
            wake: false,
            presses: neoui_proto::effects::PressGate::default(),
            press_app: None,
            feedback: None,
            info: Default::default(),
            portal: None,
            leave: false,
            entries: Vec::new(),
            entries_changed: false,
            refused: Vec::new(),
            next_session: 1,
            unbound: Vec::new(),
            blits: Vec::new(),
            refresh: false,
            repaint: false,
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
            shelf: settings_info::Shelf::boot(),
            clip_notices: ClipNotices::default(),
            clip_unseen: false,
            policies: notifications::Policies::boot(),
            d10: home::D10::default(),
        }
    }

    /// Session `id`'s first message binds it to its app: the started app without a session whose process sent it,
    /// by the kernel's word twice: its process ID is the one SYSSTART started (`Process::id` against
    /// `Message::client_process_id`), and its secure ID fits the app's EXE (`apps::sid_fits`). So the kit and the
    /// legacy bridge, which share a SID, can't take each other's session, whatever order the starts connect in. False
    /// when it fits none (it stays unbound and everything it sends is refused).
    fn bind(&mut self, id: u32, m: &Message) -> bool {
        let Some(i) = self.unbound.iter().position(|(s, _)| *s == id) else {
            return true;
        };
        let (sid, minor) = (m.client_sid(), self.unbound[i].1);
        let pid = m.client_process_id();
        let Some(app) = self.apps.iter_mut().rev().find(|a| {
            a.session.is_none() && apps::sid_fits(a.key, sid) && pid == Ok(a.process.id())
        }) else {
            dprintln!(
                "neoui: session {} (SID {:#x}, process {:?}) is no started app",
                id,
                sid,
                pid
            );
            return false;
        };
        app.session = Some(id);
        app.minor = minor;
        self.unbound.remove(i);
        true
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
            if !self.badge_allowed(key) {
                continue;
            }
            match out.iter_mut().find(|(k, _)| k == key) {
                Some((_, sum)) => *sum = sum.saturating_add(*n),
                None => out.push((key, *n)),
            }
        }
        out
    }

    /// The toast's time is up: take it away. Call once per frame.
    pub fn expire_note(&mut self, now_ms: u64) {
        // a clipboard toast not shown yet has no deadline (`privacy::UNSEEN`) until `note_displayed` gives it one
        if self
            .note
            .as_ref()
            .is_some_and(|(_, _, until)| now_ms >= *until)
        {
            self.note = None;
            self.notes_changed = true;
        }
        self.clip_notice();
    }

    /// (D8) The next waiting clipboard toast, once no toast shows (it never covers another app's toast and its
    /// action); with the notice turned off, the waiting ones go.
    fn clip_notice(&mut self) {
        if !self.settings.get(keys::PRIVACY_CLIP_NOTICE).unwrap_or(true) {
            self.clip_notices.clear();
        } else if self.note.is_none()
            && let Some(text) = self.clip_notices.take_next()
        {
            let note = Note {
                text,
                action: String::new(),
            };
            // its time starts when it reaches the screen, so a covered one neither expires nor wakes the loop
            self.note = Some((CLIP_NOTE, note, privacy::UNSEEN));
            self.clip_unseen = true;
            self.notes_changed = true;
        }
    }

    /// A frame with the toast reached the screen, over the app (the main loop): the clipboard toast's time starts.
    pub fn note_displayed(&mut self, now_ms: u64) {
        if core::mem::take(&mut self.clip_unseen)
            && let Some((CLIP_NOTE, _, until)) = self.note.as_mut()
        {
            *until = now_ms + proto::NOTE_MS;
        }
    }

    /// The shell's own card under an app's key (the Wi-Fi login's, under Browser): the app didn't post it, so it
    /// isn't the app's to clear and the app isn't told when it goes.
    fn shell_card(key: &str, n: &Notification) -> bool {
        key == BROWSER && n.id == PORTAL_ENTRY
    }

    /// `NotifyGone` to app `key`, if it runs and speaks 1.18: an older decoder refuses the event and its app ends.
    fn post_gone(&mut self, key: &'static str, id: u32, why: u8) {
        let speaks = self
            .apps
            .find(|a| a.key == key && a.quit_at.is_none())
            .is_some_and(|a| a.session.is_some() && a.minor >= 18);
        if speaks {
            self.post(key, AppEvent::NotifyGone { id, why });
        }
    }

    /// The user swiped app `key`'s card `id` away (gone already: nothing): it goes, and its app hears why. Ongoing
    /// cards stay.
    pub fn dismiss_notification(&mut self, key: &str, id: u32) {
        let Some(i) = self
            .notifications
            .iter()
            .position(|(k, n)| *k == key && n.id == id && !n.ongoing())
        else {
            return;
        };
        let (key, n) = self.notifications.remove(i);
        self.notes_changed = true;
        dprintln!("neoui: notification {} of {} dismissed", n.id, key);
        if !Self::shell_card(key, &n) {
            self.post_gone(key, n.id, proto::GONE_SWIPED);
        }
    }

    /// Clear all: every card but the ongoing ones goes, and each running app hears it.
    pub fn clear_notifications(&mut self) {
        let (keep, gone): (Vec<_>, Vec<_>) = core::mem::take(&mut self.notifications)
            .into_iter()
            .partition(|(_, n)| n.ongoing());
        self.notifications = keep;
        self.notes_changed = true;
        dprintln!("neoui: {} notifications cleared", gone.len());
        for (key, n) in gone {
            if !Self::shell_card(key, &n) {
                self.post_gone(key, n.id, proto::GONE_CLEARED);
            }
        }
    }

    /// The user tapped app `key`'s card `id` (`action` 0) or one of its actions (`i + 1`): its app comes to the
    /// front on the card's page and is told which; the card goes, unless it is ongoing (its app ends it). A card
    /// already gone does nothing.
    pub fn open_notification(&mut self, key: &str, id: u32, action: u16, now_ms: u64) {
        let Some(i) = self
            .notifications
            .iter()
            .position(|(k, n)| *k == key && n.id == id)
        else {
            return;
        };
        let (key, n) = if self.notifications[i].1.ongoing() {
            (self.notifications[i].0, self.notifications[i].1.clone())
        } else {
            self.notes_changed = true;
            self.notifications.remove(i)
        };
        dprintln!(
            "neoui: notification {} of {} opened ({})",
            n.id,
            key,
            action
        );
        // an in-place action (Mark read): the app hears it where it is, started hidden if it isn't running; a
        // service (1.19) hears every tap so, and opens what it wants itself
        let in_place = apps::is_service(key)
            || (action > 0 && n.action_kind(usize::from(action - 1)) == action::IN_PLACE);
        if in_place {
            let running = self
                .apps
                .find(|a| a.key == key && a.quit_at.is_none())
                .is_some();
            // `connect` binds the newest app without a session: no hidden start while another app connects
            let connecting = self
                .apps
                .find(|a| a.session.is_none() && a.quit_at.is_none())
                .is_some();
            if !running && !connecting {
                self.launch(key);
            }
            if !self.post(key, AppEvent::Notified { id: n.id, action }) {
                dprintln!("neoui: in-place action for {} not delivered", key);
            }
            return;
        }
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
        let _ = notify(&mut self.notifications, BROWSER, n, unix_ms());
        self.notes_changed = true;
        self.portal = arg;
    }

    /// The Settings app's `SETTINGS_INFO` of `kind` (1..=3, 5; 4 is answered from the bridge's entries) is now `bytes` (`neoui_proto::settings_app::info`).
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
            self.open_app("settings", BROWSER, PORTAL_PAGE, arg, None, 0, now_ms);
        }
    }

    /// App `from` opens app `key` on `page` with `arg` (ipc `OPEN`) or a file (`OPEN_FILE`): started or brought to
    /// the front, told, and shown with `from` as where Back returns. From a service (1.19) Back returns home and
    /// `flags` count (`OPEN_WAKE`).
    #[allow(clippy::too_many_arguments)]
    fn open_app(
        &mut self,
        from: &'static str,
        key: &'static str,
        page: u16,
        arg: String,
        file: Option<SharedFile>,
        flags: i32,
        now_ms: u64,
    ) {
        let service = apps::is_service(from);
        dprintln!(
            "neoui: {} opens {} page {}{}{}",
            from,
            key,
            page,
            if file.is_some() { " with a file" } else { "" },
            if service && flags & proto::OPEN_WAKE != 0 {
                " (wake)"
            } else {
                ""
            }
        );
        self.limit_running(key, now_ms);
        self.start(key);
        if let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none()) {
            app.arg = arg;
            app.file = file;
            app.opened_by = (!service).then_some(from);
            app.send(AppEvent::Open { page });
        }
        self.requested_screen = Some(key);
        // a service has no screen to return to: Back at the app's root goes home (not to a stale origin)
        self.requested_from = Some(if service { "home" } else { from });
        if service && flags & proto::OPEN_WAKE != 0 {
            self.wake = true; // ui.show closes the shade
        }
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
        // in order; one whose EXE is missing (no DTSRV in this ROM) doesn't hold up the others
        for &key in RESIDENT {
            if self.apps.find(|a| a.key == key).is_none() {
                self.resident_at = Some(now_ms);
                if self.launch(key) {
                    return;
                }
            }
        }
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

    /// Whether the front app asks for the screen to stay on (a video playing).
    pub fn front_awake(&self) -> bool {
        self.front()
            .is_some_and(|a| a.declared.as_ref().is_some_and(|p| p.awake))
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
        // a service is never the screen
        let key = APPS
            .iter()
            .find(|a| a.key == screen && !a.service)
            .map(|a| a.key);
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
        self.take_thumb(); // (G3) before its frame is released below
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

    /// Start app `key` (hidden); false when its process couldn't be made (logged).
    fn launch(&mut self, key: &'static str) -> bool {
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
                    minor: 0,
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
                true
            }
            Err(e) => {
                dprintln!("neoui: app {} failed to start ({})", key, e);
                false
            }
        }
    }

    pub fn entries(&self, key: &str) -> &[LauncherEntry] {
        self.entries
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(&[], |(_, entries)| entries.as_slice())
    }

    /// A cached tile can restart its owner; the normal event queue holds the UID until it connects.
    /// `ACTION_STOP` (SE4a): close a running app as Open apps' × does; never a resident (it would respawn) or Settings.
    fn stop_action(&mut self, m: &Message) -> i32 {
        let Some(key) = action_app(m) else {
            return kerr::ARGUMENT;
        };
        if RESIDENT.contains(&key) || key == apps::SETTINGS {
            return kerr::PERMISSION_DENIED;
        }
        if self
            .apps
            .find(|a| a.key == key && a.quit_at.is_none())
            .is_none()
        {
            return err::KERR_NOT_FOUND;
        }
        dprintln!("neoui: settings stops app {}", key);
        self.close(key, e32::time::millis());
        0
    }

    /// `ACTION_CLEAR` (SE4a): the app clears its own data on `ClearData`; it is started hidden first if it isn't
    /// running (the event waits in its queue until it connects).
    fn clear_action(&mut self, m: &Message) -> i32 {
        let Some(key) = action_app(m) else {
            return kerr::ARGUMENT;
        };
        if !apps::clears_data(key) {
            return kerr::NOT_SUPPORTED;
        }
        if self
            .apps
            .find(|a| a.key == key && a.quit_at.is_none())
            .is_none()
        {
            // `connect` binds the newest app without a session: while another is still connecting (a resident
            // starting), a hidden start could hand ClearData to the wrong process; Settings asks again later
            if self
                .apps
                .find(|a| a.session.is_none() && a.quit_at.is_none())
                .is_some()
            {
                return err::KERR_NOT_READY;
            }
            self.launch(key);
        }
        match self.apps.find_mut(|a| a.key == key && a.quit_at.is_none()) {
            Some(app) => {
                dprintln!("neoui: settings clears the data of app {}", key);
                app.send(AppEvent::ClearData);
                0
            }
            None => err::KERR_NOT_READY, // it didn't start
        }
    }

    /// `ACTION_RESET` (SE4a): the job now, then the restart (the main loop saves the settings first). An error
    /// answers at once and nothing restarts.
    fn reset_action(&mut self, m: &Message) -> i32 {
        let mut b = [0u8; 1];
        let what = match (m.des_len(1), m.read8(1, &mut b, 0)) {
            (Ok(1), Ok(1)) => b[0],
            _ => return kerr::ARGUMENT,
        };
        match what {
            proto::RESET_NETWORK => {
                if let Err(e) = crate::wifi::reset() {
                    dprintln!("neoui: reset network settings failed ({})", e);
                    return e;
                }
                dprintln!("neoui: reset network settings: every Wi-Fi network forgotten");
            }
            proto::RESET_SETTINGS => {
                match self.settings.reset_settings(e32::time::millis()) {
                    Ok(n) => dprintln!("neoui: reset settings: {} back to their defaults", n),
                    Err(e) => {
                        dprintln!("neoui: reset settings failed ({:?})", e);
                        return e.to_symbian();
                    }
                }
                // saved now, or no restart: the old file would bring the settings back
                let saved = self.settings.flush();
                settings::log_save(saved);
                if let Some(Err(_)) = saved {
                    return err::KERR_GENERAL;
                }
            }
            _ => return kerr::ARGUMENT,
        }
        dprintln!("neoui: restarting after the reset");
        self.power = Some(proto::POWER_RESTART);
        0
    }

    /// `SETTINGS_ACTION` `ACTION_PROFILE` (SE2a): a sound profile command from Settings, through the reducer.
    fn profile_action(&mut self, key: &str, m: &Message) -> i32 {
        if access::caller(key, m.client_sid()) != Caller::Settings {
            return kerr::PERMISSION_DENIED;
        }
        let mut b = [0u8; 4 + sound_profile::NAME_MAX];
        let cmd = match m.des_len(1) {
            Ok(n) if (4..=b.len()).contains(&n) => match m.read8(1, &mut b, 0) {
                Ok(n) => sound_profile::Command::decode(&b[..n]),
                Err(e) => return e,
            },
            _ => None,
        };
        let Some(cmd) = cmd else {
            return kerr::ARGUMENT;
        };
        let r = sound_profile::store::apply(&mut self.settings, cmd.clone(), e32::time::millis());
        dprintln!("neoui: settings profile {:?} ({:?})", cmd, r);
        let g = |k| self.settings.get(k).unwrap_or(false);
        dprintln!(
            "neoui: sound profile {}: airplane {}, wifi {}, bt {}",
            self.settings.get(keys::SOUND_PROFILE).unwrap_or(0),
            g(keys::RADIO_AIRPLANE),
            g(keys::RADIO_WIFI),
            g(keys::RADIO_BT)
        );
        use sound_profile::{Refused, store::ApplyError};
        match r {
            // (D6) a sound of the user's goes only once no profile names it on disk: Settings deletes its file after
            // this answer, so the store is saved now (an unsaved or unsavable store answers an error, the file stays)
            Ok(()) if matches!(cmd, sound_profile::Command::DeleteTone(_)) => {
                match self.settings.flush() {
                    Some(Err(e)) => {
                        dprintln!("neoui: settings save for a deleted sound failed ({:?})", e);
                        err::KERR_NOT_READY
                    }
                    _ => 0,
                }
            }
            Ok(()) => 0,
            Err(ApplyError::Refused(Refused::NotFound)) => err::KERR_NOT_FOUND,
            Err(ApplyError::Refused(Refused::Full)) => kerr::OVERFLOW,
            Err(_) => kerr::ARGUMENT,
        }
    }

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
        if let AppEvent::Input(InputEvent::Touch {
            id: 0,
            phase: TouchPhase::Down,
            ..
        }) = e
        {
            self.presses.pressed();
            self.press_app = Some(key);
        }
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

/// A 1.22 opcode whose shell side isn't built yet: `KErrArgument` (or `KErrTooBig`, `KErrNotFound` for a widget not in
/// the app's catalogue) for what a later shell would refuse too, else `KErrNotSupported`.
fn contract_1_22(key: &str, m: &Message) -> i32 {
    use neoui_proto::{activity, media, tiles, widget};
    let check = |max: usize, ok: &dyn Fn(&[u8]) -> Result<(), i32>| {
        read_bytes(m, 0, max).and_then(|b| ok(&b))
    };
    let r = match m.function() {
        proto::MEDIA => check(media::MEDIA_MAX, &|b| {
            (b.is_empty() || media::Session::decode(b).is_some())
                .then_some(())
                .ok_or(kerr::ARGUMENT)
        }),
        proto::ACTIVITY => check(activity::ACTIVITY_MAX, &|b| {
            activity::Activity::decode(b)
                .map(drop)
                .ok_or(kerr::ARGUMENT)
        }),
        proto::TILE => check(tiles::OFFER_MAX, &|b| {
            tiles::Offer::decode(b).map(drop).ok_or(kerr::ARGUMENT)
        }),
        proto::WIDGET => check(widget::WIDGET_MAX, &|b| {
            let c = widget::Content::decode(b).ok_or(kerr::ARGUMENT)?;
            c.fits(key).map_err(|template| {
                if template {
                    kerr::ARGUMENT
                } else {
                    err::KERR_NOT_FOUND
                }
            })
        }),
        _ => Ok(()), // REPLY_TEXT: no reply is ever pending yet
    };
    r.err().unwrap_or(err::KERR_NOT_SUPPORTED)
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

/// Adds, replaces or (empty title) removes app `key`'s shade entry. A new card goes first; a card posted again keeps
/// its place (top-panel TP2a: an update doesn't jump). `when` 0 becomes `now_ms` (unix ms); a 25th card of one app
/// pushes out its oldest.
fn notify(
    list: &mut Vec<(&'static str, Notification)>,
    key: &'static str,
    mut n: Notification,
    now_ms: u64,
) -> Result<i32, i32> {
    let old = list.iter().position(|(k, o)| *k == key && o.id == n.id);
    if n.title.is_empty() {
        if let Some(i) = old {
            list.remove(i);
        }
        return Ok(0);
    }
    if n.when == 0 {
        // an update keeps the time the card first came, unless the app gives one
        n.when = old.map_or(now_ms, |i| list[i].1.when);
    }
    if let Some(i) = old {
        list[i].1 = n;
        return Ok(0);
    }
    // the shell's own card under the app's key (the Wi-Fi login's) isn't in the app's quota, nor pushed out by it
    let mine = |(k, o): &(&'static str, Notification)| *k == key && !Host::shell_card(k, o);
    if list.iter().filter(|e| mine(e)).count() >= proto::NOTIFICATIONS_MAX
        && let Some(oldest) = list.iter().rposition(mine)
    {
        list.remove(oldest);
    }
    list.insert(0, (key, n));
    Ok(0)
}

/// Now as unix milliseconds (a card's `when`).
fn unix_ms() -> u64 {
    u64::try_from(e32::time::utc_now().0).unwrap_or(0) * 1000
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
fn settings_info(info: &[Vec<u8>; 5], m: &Message) -> Result<i32, i32> {
    let bytes = m
        .int(0)
        .checked_sub(1)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| info.get(i))
        .ok_or(kerr::ARGUMENT)?;
    write_info(m, bytes)
}

/// A `SETTINGS_INFO` answer into the caller's `out` (slot 2): its length, `KErrOverflow` when it doesn't fit.
/// An `ACTION_STOP`/`ACTION_CLEAR` request: a ROM app's key (argument 1).
fn action_app(m: &Message) -> Option<&'static str> {
    let key = read_text(m, 1, proto::KEY_MAX).ok()?;
    apps::find(&key).map(|a| a.key)
}

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
        if self.apps.find(|a| a.session.is_none()).is_none() {
            return Err(err::KERR_NOT_SUPPORTED);
        }
        // bound to its app at its first message, by the secure ID the kernel gives that message ([`Host::bind`])
        let id = self.next_session;
        self.unbound.push((id, version.minor));
        self.next_session += 1;
        Ok(id)
    }

    fn message(&mut self, id: &mut u32, m: Message) {
        if !self.bind(*id, &m) {
            return m.complete(err::KERR_NOT_SUPPORTED);
        }
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
                        // a clipboard toast not yet seen waits for this one, first in line (D8)
                        if self.clip_unseen
                            && let Some((CLIP_NOTE, clip, _)) = self.note.take()
                        {
                            self.clip_notices.requeue(clip.text);
                        }
                        self.clip_unseen = false;
                        let until = e32::time::millis() + proto::NOTE_MS;
                        self.note = Some((app.key, n, until));
                    }
                    self.notes_changed = true;
                    m.complete(0);
                }
                Err(e) => m.complete(e),
            },
            // cards and badges only from the app's own EXE (1.18, top-panel TP2a "Step 0"): the session binding
            // ("the newest app without a session") is checked against the caller's secure ID
            proto::NOTIFY
            | proto::NOTIFY_CLEAR
            | proto::BADGE
            | proto::CHANNEL
            | proto::MEDIA
            | proto::REPLY_TEXT
            | proto::ACTIVITY
            | proto::TILE
            | proto::WIDGET
                if apps::sid(app.key).is_some_and(|sid| sid != m.client_sid()) =>
            {
                dprintln!(
                    "neoui: {} refused for {}: SID {:#x}",
                    m.function(),
                    app.key,
                    m.client_sid()
                );
                m.complete(kerr::PERMISSION_DENIED);
            }
            proto::NOTIFY => {
                let key = app.key;
                let now = e32::time::millis();
                let r = read_bytes(&m, 0, NOTIFICATION_MAX)
                    .and_then(|b| Notification::decode(&b).map_err(|_| kerr::ARGUMENT))
                    .and_then(|n| {
                        // the shell's card id under this key is reserved: the app can't replace or remove it
                        if Self::shell_card(key, &n) {
                            return Err(kerr::PERMISSION_DENIED);
                        }
                        // (TP4) the user's policy: an app or channel turned off refuses it; a removal always goes
                        if !n.title.is_empty() {
                            let old = self
                                .notifications
                                .iter()
                                .find(|(k, o)| *k == key && o.id == n.id)
                                .map(|(_, o)| o.clone());
                            self.admit(key, &n, old.as_ref(), now)?;
                        }
                        notify(&mut self.notifications, key, n, unix_ms())
                    });
                if r.is_ok() {
                    self.notes_changed = true;
                }
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::CHANNEL => {
                let key = app.key;
                let r = self.channel(key, &m);
                m.complete(r);
            }
            // 1.22's contract: checked, not acted on yet (their producers and consumers land in W1–W3)
            proto::MEDIA | proto::REPLY_TEXT | proto::ACTIVITY | proto::TILE | proto::WIDGET => {
                let r = contract_1_22(app.key, &m);
                m.complete(r);
            }
            proto::NOTIFY_CLEAR => {
                let key = app.key;
                self.notifications
                    .retain(|(k, n)| *k != key || Self::shell_card(k, n));
                self.notes_changed = true;
                m.complete(0);
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
                // (D8) a read that gave the app something: the toast names it by its process's secure ID
                if r.is_ok_and(|n| n > 0)
                    && self.settings.get(keys::PRIVACY_CLIP_NOTICE).unwrap_or(true)
                {
                    let now = e32::time::millis();
                    if self.clip_notices.read(m.client_sid(), now) {
                        self.clip_notice();
                    }
                }
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
            proto::SETTINGS_INFO
                if m.int(0) == proto::INFO_RUNNING
                    && access::caller(app.key, m.client_sid()) != Caller::Settings =>
            {
                m.complete(kerr::PERMISSION_DENIED);
            }
            proto::SETTINGS_INFO if m.int(0) == proto::INFO_RUNNING => {
                let running: Vec<(&str, bool)> = self
                    .apps
                    .newest_first()
                    .filter(|a| a.quit_at.is_none())
                    .map(|a| (a.key, RESIDENT.contains(&a.key)))
                    .collect();
                let r = write_info(&m, &encode_running(&running));
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::SETTINGS_INFO if m.int(0) > proto::INFO_RUNNING => {
                let key = app.key;
                let r = self.paged_info(key, *id, &m);
                m.complete(r);
            }
            // D6: an accepted control of the app in front, for the last press it got
            proto::CONTROL_FEEDBACK => {
                let (key, kind, code) = (app.key, m.int(0), m.int(1));
                let r = if app.minor < 20 {
                    err::KERR_NOT_SUPPORTED
                } else if !matches!(kind, proto::FEEDBACK_CONTROL | proto::FEEDBACK_DIAL_KEY) {
                    kerr::ARGUMENT
                } else if front == Some(key) && self.press_app == Some(key) && self.presses.take() {
                    self.feedback = Some((kind, u8::try_from(code).unwrap_or(0)));
                    0
                } else {
                    err::KERR_NOT_READY
                };
                m.complete(r);
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
                    // a sound profile (SE2a): Settings only; the shell's reducer changes everything at once
                    proto::ACTION_PROFILE => {
                        let key = app.key;
                        self.profile_action(key, &m)
                    }
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
                    // Settings → Apps → an app (SE4a): Settings only, by its secure ID
                    proto::ACTION_STOP
                    | proto::ACTION_CLEAR
                    | proto::ACTION_RESET
                    | proto::ACTION_MEASURE_APP
                        if access::caller(app.key, m.client_sid()) != Caller::Settings =>
                    {
                        kerr::PERMISSION_DENIED
                    }
                    proto::ACTION_STOP => self.stop_action(&m),
                    // 1.20: a confirmed clear is acknowledged by its owner, never downgraded to the 1.17 event
                    proto::ACTION_CLEAR if settings_info::is_confirmed_clear(&m) => {
                        self.confirmed_clear(&m)
                    }
                    proto::ACTION_CLEAR => self.clear_action(&m),
                    proto::ACTION_MEASURE_APP => self.measure_action(&m),
                    proto::ACTION_RESET => self.reset_action(&m),
                    // (D10) the home editor
                    proto::ACTION_EDIT_HOME => {
                        let key = app.key;
                        self.edit_home_action(key, &m)
                    }
                    _ => kerr::ARGUMENT,
                };
                m.complete(r);
            }
            proto::ENTRY => {
                let key = app.key;
                let r = entry(&mut self.entries, &mut self.refused, key, &m);
                if r.is_ok() {
                    self.entries_changed = true;
                    if key == LEGACY {
                        self.legacy_entries_changed();
                    }
                }
                m.complete(r.unwrap_or_else(|e| e));
            }
            proto::OWNER_REPORT => {
                let (key, minor) = (app.key, app.minor);
                let r = self.owner_report(key, minor, *id, &m);
                m.complete(r);
            }
            proto::ENTRY_THUMB => {
                let key = app.key;
                let r = self.entry_thumb(key, &m);
                m.complete(r);
            }
            proto::FRONT if apps::is_service(app.key) => m.complete(err::KERR_NOT_SUPPORTED),
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
                if apps::is_service(target) {
                    return m.complete(err::KERR_NOT_SUPPORTED); // never the screen
                }
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
                // 1.19: OPEN's slot 3 is its flags (OPEN_FILE's file is in 2 and 3)
                let flags = if m.function() == proto::OPEN {
                    m.int(3)
                } else {
                    0
                };
                self.open_app(from, target, page, arg, file, flags, e32::time::millis());
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
            proto::REPAINT => {
                self.repaint = true;
                self.refresh = true; // the app's surface too, whole
                m.complete(0);
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
        self.unbound.retain(|(s, _)| *s != id);
        self.settings.drop_session(id);
        self.settings_session_gone(id);
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

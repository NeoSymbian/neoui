// SPDX-License-Identifier: GPL-3.0-only
//! App processes (NeoSymbian R6, R10): the `NeoUi` IPC service apps talk to, and the apps that are running.
//!
//! An app (e.g. `NEOCLOCK.EXE`) is started when the shell first navigates to its key, connects to `NeoUi`, asks for
//! its geometry and its surface — an unnamed global chunk the server creates and hands over with
//! `complete_with_handle` (same address in both processes) — draws, reports `DAMAGE`, and keeps one `NEXT_EVENT`
//! outstanding, which the server completes with input, `Visible`, or `Quit`.
//!
//! Apps keep running when the shell moves away (R10): the one whose key is the current screen is the foreground app
//! (its surface is shown, it gets input and its keyboard/toolbar claims apply); the others are told they are hidden.
//! They end when closed in the switcher, or when more than [`MAX_RUNNING`] run (the least recently used).

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::ops::ControlFlow;
use e32::chunk::Chunk;
use e32::dprintln;
use e32::process::Process;
use e32::server::{Message, Service};
use e32::sys::{TVersion, err};
use neoui_proto::Rect;
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::recents::Recents;

/// Screen keys that are separate processes, and their EXEs.
// shortcut: a fixed table; a registry of installed apps comes with app installation.
pub const APPS: &[(&str, &str)] = &[
    ("clock", "neoclock"),
    ("terminal", "neoterm"),
    ("files", "neofiles"),
];

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
    /// It asked for the system keyboard (`KEYBOARD`); applies while it is in front.
    pub keyboard: bool,
    /// Toolbar buttons it claimed (`NAV_BACK | NAV_OPTIONS`); apply while it is in front.
    pub nav: i32,
}

impl App {
    fn send(&mut self, e: AppEvent) {
        match self.waiting.take() {
            Some(m) => deliver(m, e),
            None => self.queue.push_back(e),
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
    /// The shell's design unit in 1/256 px, for apps.
    pub dp256: u32,
    /// The keyboard's height in pixels when open.
    pub keyboard_h: u32,
    apps: Recents<App>,
    /// The current screen when it is an app's key.
    front: Option<&'static str>,
    next_session: u32,
    /// The foreground app's damage not yet shown (surface coordinates).
    pub blits: Vec<Rect>,
    /// Show the foreground app's whole surface on the next compose (it just came back).
    pub refresh: bool,
}

impl Host {
    pub fn new(body: Rect, dp256: u32, keyboard_h: u32) -> Self {
        Self {
            body,
            dp256,
            keyboard_h,
            apps: Recents::default(),
            front: None,
            next_session: 1,
            blits: Vec::new(),
            refresh: false,
        }
    }

    /// The foreground app, if it is on screen with its surface and not leaving.
    pub fn front(&self) -> Option<&App> {
        let key = self.front?;
        self.apps
            .find(|a| a.key == key && a.quit_at.is_none() && a.surface.is_some())
    }

    /// The part of the body the app's surface shows in (screen coordinates): all of it but the open keyboard.
    pub fn app_area(&self) -> Rect {
        let covered = if self.front().is_some_and(|a| a.keyboard) {
            self.keyboard_h
        } else {
            0
        };
        let b = self.body;
        Rect::new(b.x, b.y, b.w, b.h.saturating_sub(covered))
    }

    /// The shell moved to `screen`: hide the app that was in front, and show (or start) the one for `screen`.
    pub fn set_screen(&mut self, screen: &str, now_ms: u64) {
        let key = APPS.iter().find(|(k, _)| *k == screen).map(|(k, _)| *k);
        if self.front == key {
            return;
        }
        if let Some(old) = self.front.take()
            && let Some(app) = self.apps.find_mut(|a| a.key == old)
        {
            app.send(AppEvent::Visible(false));
        }
        self.blits.clear();
        let Some(key) = key else { return };
        self.front = Some(key);
        if self.apps.touch(|a| a.key == key && a.quit_at.is_none()) {
            if let Some(app) = self.apps.find_mut(|a| a.key == key) {
                app.send(AppEvent::Visible(true));
            }
            self.refresh = true;
            dprintln!("neoui: app {} resumed", key);
        } else {
            self.launch(key);
        }
        let over: Vec<&'static str> = self
            .apps
            .over_limit(MAX_RUNNING, |a| a.key == key || a.quit_at.is_some())
            .map(|a| a.key)
            .collect();
        for k in over {
            dprintln!("neoui: app {} closed (too many running)", k);
            self.close(k, now_ms);
        }
    }

    fn launch(&mut self, key: &'static str) {
        let exe = APPS.iter().find(|(k, _)| *k == key).map_or("", |(_, e)| *e);
        match Process::spawn(exe, "") {
            Ok(process) => {
                process.resume();
                dprintln!("neoui: app {} started", key);
                self.apps.push(App {
                    key,
                    session: None,
                    process,
                    surface: None,
                    waiting: None,
                    queue: VecDeque::new(),
                    quit_at: None,
                    keyboard: false,
                    nav: 0,
                });
            }
            Err(e) => dprintln!("neoui: app {} failed to start ({})", key, e),
        }
    }

    /// Input and claimed toolbar buttons go to the foreground app.
    pub fn send(&mut self, e: AppEvent) {
        let Some(key) = self.front else { return };
        if let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none()) {
            app.send(e);
        }
    }

    /// Ask app `key` to quit (the switcher's ×, or too many running).
    pub fn close(&mut self, key: &str, now_ms: u64) {
        if let Some(app) = self.apps.find_mut(|a| a.key == key) {
            app.quit(now_ms);
        }
    }

    /// Running apps, most recent first (not those leaving), for the switcher.
    pub fn running(&self) -> Vec<&'static str> {
        self.apps
            .newest_first()
            .filter(|a| a.quit_at.is_none())
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
        let Some(app) = self.apps.find_mut(|a| a.session == Some(*id)) else {
            return m.complete(err::KERR_NOT_SUPPORTED);
        };
        match m.function() {
            proto::GEOMETRY => {
                let mut g = [0u8; proto::GEOMETRY_LEN];
                for (i, v) in [body.w, body.h, dp256].iter().enumerate() {
                    g[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                let r = m.write8(0, &g, 0);
                m.complete(r.map_or_else(|e| e, |()| 0));
            }
            proto::SURFACE => {
                if app.surface.is_none() {
                    match Chunk::create_global(body.w as usize * body.h as usize * 4) {
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
                }
                m.complete(0);
            }
            proto::KEYBOARD => {
                app.keyboard = m.int(0) != 0;
                dprintln!("neoui: keyboard {} for {}", app.keyboard, app.key);
                m.complete(if app.keyboard { keyboard_h as i32 } else { 0 });
            }
            proto::NAV => {
                app.nav = m.int(0) & (proto::NAV_BACK | proto::NAV_OPTIONS);
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
        // Complete what the session still has outstanding before its disconnect completes.
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

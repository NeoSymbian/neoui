// SPDX-License-Identifier: GPL-3.0-only
//! App processes (NeoSymbian R6): the `NeoUi` IPC service an app talks to, and the one foreground app's state.
//!
//! An app (e.g. `NEOCLOCK.EXE`) is started when the shell navigates to its key, connects to `NeoUi`, asks for its
//! geometry and its surface — an unnamed global chunk the server creates and hands over with
//! `complete_with_handle` (same address in both processes) — draws, reports `DAMAGE`, and keeps one `NEXT_EVENT`
//! outstanding, which the server completes with input or `Quit`.

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

/// Screen keys that are separate processes, and their EXEs.
// shortcut: a fixed table; a registry of installed apps comes with app installation.
pub const APPS: &[(&str, &str)] = &[
    ("clock", "neoclock"),
    ("terminal", "neoterm"),
    ("files", "neofiles"),
];

pub struct App {
    pub key: &'static str,
    pub process: Process,
    pub surface: Option<Chunk>,
    /// The app's outstanding `NEXT_EVENT`, if any.
    waiting: Option<Message>,
    queue: VecDeque<AppEvent>,
    /// `Quit` was sent: the app is hidden and should exit.
    pub quitting: bool,
}

pub struct Host {
    /// Where app surfaces go on the screen (the shell's body).
    pub body: Rect,
    /// The shell's design unit in 1/256 px, for apps.
    pub dp256: u32,
    pub app: Option<App>,
    /// App damage not yet shown (surface coordinates).
    pub blits: Vec<Rect>,
    /// The foreground app asked for the system keyboard (`KEYBOARD`).
    pub keyboard: bool,
    /// The keyboard's height in pixels when open.
    pub keyboard_h: u32,
    /// Toolbar buttons the foreground app claimed (`NAV_BACK | NAV_OPTIONS`).
    pub nav: i32,
}

impl Host {
    pub fn new(body: Rect, dp256: u32, keyboard_h: u32) -> Self {
        Self {
            body,
            dp256,
            app: None,
            blits: Vec::new(),
            keyboard: false,
            keyboard_h,
            nav: 0,
        }
    }

    /// The part of the body the app's surface shows in (screen coordinates): all of it but the open keyboard.
    pub fn app_area(&self) -> Rect {
        let covered = if self.keyboard { self.keyboard_h } else { 0 };
        let b = self.body;
        Rect::new(b.x, b.y, b.w, b.h.saturating_sub(covered))
    }

    /// Start the app for screen `key`, if it is one.
    pub fn launch(&mut self, key: &str) {
        let Some(&(key, exe)) = APPS.iter().find(|(k, _)| *k == key) else {
            return;
        };
        match Process::spawn(exe, "") {
            Ok(process) => {
                process.resume();
                dprintln!("neoui: app {} started", key);
                self.app = Some(App {
                    key,
                    process,
                    surface: None,
                    waiting: None,
                    queue: VecDeque::new(),
                    quitting: false,
                });
            }
            Err(e) => dprintln!("neoui: app {} failed to start ({})", key, e),
        }
    }

    /// Queue `e` for the app, or answer its outstanding `NEXT_EVENT` now.
    pub fn send(&mut self, e: AppEvent) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        match app.waiting.take() {
            Some(m) => deliver(m, e),
            None => app.queue.push_back(e),
        }
    }

    /// Ask the app to leave (the shell moved away from it).
    pub fn quit(&mut self) {
        if let Some(app) = self.app.as_mut().filter(|a| !a.quitting) {
            app.quitting = true;
            self.keyboard = false;
            self.nav = 0;
            self.send(AppEvent::Quit);
        }
    }

    /// The foreground app is on screen with its surface: `screen` is its key and it is not leaving.
    pub fn visible(&self, screen: &str) -> Option<&App> {
        self.app
            .as_ref()
            .filter(|a| a.key == screen && !a.quitting && a.surface.is_some())
    }

    /// If the app process has ended, forget it and say how it ended.
    pub fn reap(&mut self) -> Option<&'static str> {
        let (t, r) = self.app.as_ref()?.process.exit_info()?;
        let app = self.app.take()?;
        self.keyboard = false;
        self.nav = 0;
        dprintln!("neoui: app {} exited (type {}, reason {})", app.key, t, r);
        Some(app.key)
    }
}

fn deliver(m: Message, e: AppEvent) {
    let r = m.write8(0, &e.encode(), 0);
    m.complete(r.map_or_else(|e| e, |()| 0));
}

impl Service for Host {
    type Session = ();

    fn connect(&mut self, version: TVersion) -> Result<(), i32> {
        if version.major != proto::VERSION.0 {
            return Err(err::KERR_NOT_SUPPORTED);
        }
        Ok(())
    }

    fn message(&mut self, _: &mut (), m: Message) {
        let body = self.body;
        let dp256 = self.dp256;
        let Some(app) = self.app.as_mut() else {
            // shortcut: no access policy — only the foreground app gets service; others are refused.
            m.complete(err::KERR_NOT_SUPPORTED);
            return;
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
                self.blits.push(Rect::new(
                    m.int(0),
                    m.int(1),
                    m.int(2) as u32,
                    m.int(3) as u32,
                ));
                m.complete(0);
            }
            proto::KEYBOARD => {
                self.keyboard = m.int(0) != 0;
                dprintln!("neoui: keyboard {} for {}", self.keyboard, app.key);
                m.complete(if self.keyboard {
                    self.keyboard_h as i32
                } else {
                    0
                });
            }
            proto::NAV => {
                self.nav = m.int(0) & (proto::NAV_BACK | proto::NAV_OPTIONS);
                m.complete(0);
            }
            proto::NEXT_EVENT => match app.queue.pop_front() {
                Some(e) => deliver(m, e),
                None => app.waiting = Some(m),
            },
            _ => m.complete(err::KERR_NOT_SUPPORTED),
        }
    }

    fn disconnect(&mut self, _: ()) -> ControlFlow<()> {
        // Complete what the session still has outstanding before its disconnect completes.
        if let Some(m) = self.app.as_mut().and_then(|a| a.waiting.take()) {
            m.complete(err::KERR_CANCEL);
        }
        ControlFlow::Continue(())
    }
}

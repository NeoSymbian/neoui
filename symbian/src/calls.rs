// SPDX-License-Identifier: Apache-2.0
//! (TP5, TP4) Calls and inline replies in SYSSTART (docs `neoui-shell/top-panel.md` "Calls and messages from the
//! panel", protocol.md "Cards: STICKY and the Call style's keys"), a child of `host`:
//! - a ringing call while the phone is in use is NEOPHONE's sticky Call card as the heads-up banner; with the screen
//!   off, locked or on home (or no card within [`calls::CARD_WAIT_MS`], a Phone before 1.22) the call screen comes
//!   up full screen as before;
//! - the banner's round Answer and Reject (`page::CALL_ANSWER`/`CALL_REJECT`) go to the card's app where it is; an
//!   app that isn't running is started hidden and gets them once it binds, only if it speaks 1.22 (`calls::Held`);
//! - an inline reply waits for its app's `REPLY_TEXT` (one per card, 60 s, never logged); `NotifyReply` tells the
//!   app, held like the call keys.

use super::Host;
use alloc::vec::Vec;
use e32::dprintln;
use e32::server::Message;
use neoui_proto::calls::{self, Held, Replies};
use neoui_proto::contract;
use neoui_proto::ipc::AppEvent;
use neoui_proto::page::REPLY_MAX;
use neoui_render::Renderer;

/// The host's TP5 / TP4 reply state.
#[derive(Default)]
pub struct Calls {
    held: Held<&'static str>,
    replies: Replies<&'static str>,
    /// the banner shows this sticky card (it stays until the card stops ringing)
    pub sticky: Option<(&'static str, u32)>,
    /// a call rang while the phone was in use: until when (ms) the shell waits for NEOPHONE's card
    wait: Option<u64>,
    /// the sticky banner as last logged
    logged: Option<(&'static str, u32)>,
    /// the apps whose Reply opens the inline field, as last given to the renderer
    inline: Vec<&'static str>,
}

impl Host {
    /// Event `e` for app `key` where it is: posted, or (not running or not bound yet) held until it binds, the app
    /// started hidden.
    fn in_place(&mut self, key: &'static str, e: AppEvent, now_ms: u64) {
        match self.apps.find(|a| a.key == key && a.quit_at.is_none()) {
            Some(a) if a.session.is_some() => {
                self.post(key, e);
            }
            running => {
                if running.is_none() {
                    self.launch(key);
                }
                dprintln!("neoui: {:?} held for {}", e, key);
                self.calls.held.hold(key, e, now_ms);
            }
        }
    }

    /// (TP5, O1) The banner's Answer or Reject on card `id` of app `key`: its app hears it where it is.
    pub(super) fn call_key(&mut self, key: &'static str, id: u32, action: u16, now_ms: u64) {
        dprintln!("neoui: call key {:#x} for {} card {}", action, key, id);
        self.in_place(key, AppEvent::Notified { id, action }, now_ms);
    }

    /// App `key` bound with protocol minor `minor`: its held events, those it can read.
    pub(super) fn calls_bound(&mut self, key: &'static str, minor: u8) {
        for e in self.calls.held.bound(key, minor) {
            dprintln!("neoui: held event for {}", key);
            self.post(key, e);
        }
    }

    /// The user sent an inline reply on card `id` of app `key`: it waits for the app's `REPLY_TEXT`, and the app
    /// hears `NotifyReply`. The text itself is never logged.
    pub fn reply_sent(&mut self, key: &str, id: u32, text: &str, now_ms: u64) {
        let Some(key) = self
            .notifications
            .iter()
            .map(|(k, _)| *k)
            .find(|k| *k == key)
        else {
            return;
        };
        if !self.calls.replies.put(key, id, text, now_ms) {
            return;
        }
        dprintln!(
            "neoui: reply for {} card {} ({} bytes)",
            key,
            id,
            text.len()
        );
        self.in_place(key, AppEvent::NotifyReply { id }, now_ms);
    }

    /// `REPLY_TEXT [card_id, Des8 out]` from app `key`: the text's length, written to `out`.
    pub(super) fn reply_text_message(&mut self, key: &'static str, m: &Message) -> i32 {
        if let Err(e) = contract::publisher(key, m.client_sid()) {
            return e;
        }
        let id = m.int(0) as u32;
        let room = match m.des_max_len(1) {
            Ok(n) => n.min(REPLY_MAX),
            Err(e) => return e,
        };
        let text = match self.calls.replies.take(key, id, room) {
            Ok(t) => t,
            Err(e) => return e,
        };
        match m.write8(1, text.as_bytes(), 0) {
            Ok(()) => {
                dprintln!("neoui: reply text taken by {} card {}", key, id);
                text.len() as i32
            }
            Err(e) => {
                // not delivered: it waits again
                let _ = self.calls.replies.put(key, id, &text, e32::time::millis());
                e
            }
        }
    }

    /// A call started ringing (`rang`, phone.rs) with the screen on or off: the full-screen call when the screen is
    /// off, locked or on home, or Phone is in front; else the shell waits for NEOPHONE's card as the banner.
    pub fn ringing(&mut self, ui: &mut Renderer, rang: bool, screen_on: bool, ms: u64) {
        if !rang {
            return;
        }
        // shortcut: no lock screen yet (TP8 is W2-C's): `locked` is false until it lands
        let locked = false;
        let screen = ui.screen();
        if calls::full_screen(screen_on, locked, screen == "home") || screen == "phone" {
            dprintln!("neoui: call full screen");
            ui.show("phone", "call");
            self.calls.wait = None;
        } else {
            dprintln!("neoui: call rings in use, waiting for its card");
            self.calls.wait = Some(ms + calls::CARD_WAIT_MS);
        }
    }

    /// Each pass of the main loop: a ringing call's banner (or the call screen when its card didn't come or can't
    /// show), the sticky banner going when its card stops ringing, the inline replies' apps for the renderer, the
    /// sent replies, and what is held too long.
    pub fn calls_frame(&mut self, ui: &mut Renderer, ms: u64) {
        if self.calls.sticky != self.calls.logged {
            if let Some((k, id)) = self.calls.sticky {
                dprintln!("neoui: sticky banner {} {}", k, id);
            }
            self.calls.logged = self.calls.sticky;
        }
        if let Some(until) = self.calls.wait {
            let card = self.calls.sticky.is_some_and(|(k, _)| k == "phone") && ui.banner_shown();
            if card {
                dprintln!("neoui: call rings as a banner");
                self.calls.wait = None;
            } else if ms >= until || ui.keyboard_h() > 0 {
                dprintln!("neoui: call full screen (no banner)");
                ui.show("phone", "call");
                self.calls.wait = None;
            }
        }
        if let Some((k, id)) = self.calls.sticky {
            let rings = self
                .notifications
                .iter()
                .any(|(key, n)| *key == k && n.id == id && calls::rings(n));
            if !rings || !ui.banner_shown() {
                self.calls.sticky = None;
                if !rings {
                    ui.set_banner(None);
                }
            }
        }
        // Reply inline for the running apps that read REPLY_TEXT (1.22); for the rest it opens the app
        let inline: Vec<&'static str> = self
            .apps
            .newest_first()
            .filter(|a| a.quit_at.is_none() && a.session.is_some() && a.minor >= 22)
            .map(|a| a.key)
            .collect();
        if inline != self.calls.inline {
            ui.set_reply_apps(&inline);
            self.calls.inline = inline;
            self.notes_changed = true;
        }
        for (key, id, text) in ui.take_replies() {
            self.reply_sent(&key, id, &text, ms);
        }
        self.calls.held.expire(ms);
        // a pending reply goes with its card, its app, or after 60 s
        let now = ms;
        let cards: Vec<(&'static str, u32)> =
            self.notifications.iter().map(|(k, n)| (*k, n.id)).collect();
        let running: Vec<&'static str> = self
            .apps
            .newest_first()
            .filter(|a| a.quit_at.is_none())
            .map(|a| a.key)
            .collect();
        let held = &self.calls.held;
        self.calls.replies.retain(now, |k, id| {
            cards.contains(&(k, id)) && (running.contains(&k) || held.waiting(k))
        });
    }

    /// A card was posted again or removed: a reply waiting for it goes.
    pub(super) fn forget_reply(&mut self, key: &'static str, id: u32) {
        self.calls.replies.forget(key, id);
    }
}

// SPDX-License-Identifier: Apache-2.0
//! (TP5, TP4) Calls and inline replies in the shell (docs neoui-shell/top-panel.md "Calls and messages from the
//! panel", protocol.md "Cards: STICKY and the Call style's keys"):
//! - [`rings`]: a ringing call's card (Call style, `NOTIFY_STICKY`), the banner that doesn't time out;
//! - [`full_screen`]: whether a ringing call takes the whole screen (off, locked, on home) or shows as the banner;
//! - [`Held`]: the events a card's owner hears in place (Answer, Reject, a reply's `NotifyReply`) while it isn't
//!   running yet: held until it binds, then given only to a 1.22 owner;
//! - [`Replies`]: the inline replies waiting for their owner's `REPLY_TEXT` (one per card, 60 s, never logged).

use crate::ipc::AppEvent;
use crate::page::{self, NOTIFY_STICKY, Notification, style};
use crate::settings::kerr;
use alloc::string::String;
use alloc::vec::Vec;

/// A ringing call's card: Call style with `NOTIFY_STICKY` (O1). On an Urgent channel it gets past Do not disturb.
#[must_use]
pub fn rings(n: &Notification) -> bool {
    n.style == style::CALL && n.flags & NOTIFY_STICKY != 0
}

/// How long the shell waits for NEOPHONE's ringing card before it pulls the call screen up after all (a Phone
/// before 1.22 posts none).
pub const CARD_WAIT_MS: u64 = 1500;

/// A call started ringing: the full-screen call (`p2-call-incoming`) when the screen is off, locked or on home;
/// else the heads-up banner of NEOPHONE's card (TPD3).
#[must_use]
pub fn full_screen(screen_on: bool, locked: bool, on_home: bool) -> bool {
    !screen_on || locked || on_home
}

/// At most this many held events (one more drops the oldest).
const HELD_MAX: usize = 8;
/// A held event older than this is dropped (its app didn't come up).
pub const HELD_MS: u64 = 20_000;

/// Events for apps that aren't running yet, by app key: held until the app binds.
#[derive(Default, Debug)]
pub struct Held<K> {
    q: Vec<(K, AppEvent, u64)>,
}

impl<K: Copy + PartialEq> Held<K> {
    /// `e` for app `key`, at `now` ms.
    pub fn hold(&mut self, key: K, e: AppEvent, now: u64) {
        if self.q.len() >= HELD_MAX {
            self.q.remove(0);
        }
        self.q.push((key, e, now));
    }

    /// Whether anything is held for `key`.
    #[must_use]
    pub fn waiting(&self, key: K) -> bool {
        self.q.iter().any(|h| h.0 == key)
    }

    /// App `key` bound with protocol minor `minor`: its held events in order, those its decoder reads
    /// (`AppEvent::needs_minor`, applied after binding). Either way they are gone.
    pub fn bound(&mut self, key: K, minor: u8) -> Vec<AppEvent> {
        let mut out = Vec::new();
        self.q.retain(|(k, e, _)| {
            if *k != key {
                return true;
            }
            if minor >= e.needs_minor() {
                out.push(*e);
            }
            false
        });
        out
    }

    /// Drops what waited longer than [`HELD_MS`] (the millisecond counter may wrap: a time ahead of `now` restarts).
    pub fn expire(&mut self, now: u64) {
        self.q.retain_mut(|h| {
            h.2 = h.2.min(now);
            now - h.2 < HELD_MS
        });
    }
}

/// The quick replies under the inline field (nt-reply).
pub const QUICK_REPLIES: [&str; 3] = ["OK", "On my way", "Call you later"];

/// A pending reply goes after this long (protocol.md "TP4 policy").
pub const REPLY_MS: u64 = 60_000;

/// Inline replies waiting for `REPLY_TEXT`: (owner, card id, text, when sent). One per card; the text is never
/// logged or kept anywhere else.
#[derive(Default)]
pub struct Replies<K> {
    list: Vec<(K, u32, String, u64)>,
}

impl<K: Copy + PartialEq> Replies<K> {
    /// The user sent `text` on app `key`'s card `id` at `now` ms; false (nothing kept) unless it is 1–480 bytes. A
    /// second one for the same card replaces the first.
    pub fn put(&mut self, key: K, id: u32, text: &str, now: u64) -> bool {
        if page::reply_text(text.as_bytes()).is_none() {
            return false;
        }
        self.forget(key, id);
        self.list.push((key, id, text.into(), now));
        true
    }

    /// `REPLY_TEXT` from app `key` for card `id` into a buffer of `room` bytes: the text, which ends the pending
    /// reply; `KErrNotFound` with none pending, `KErrOverflow` for a short buffer (the text stays).
    ///
    /// # Errors
    /// As above.
    pub fn take(&mut self, key: K, id: u32, room: usize) -> Result<String, i32> {
        let i = self
            .list
            .iter()
            .position(|(k, c, _, _)| *k == key && *c == id)
            .ok_or(kerr::NOT_FOUND)?;
        if self.list[i].2.len() > room {
            return Err(kerr::OVERFLOW);
        }
        Ok(self.list.remove(i).2)
    }

    /// Card `id` of `key` was replaced or removed: its reply goes.
    pub fn forget(&mut self, key: K, id: u32) {
        self.list.retain(|(k, c, _, _)| !(*k == key && *c == id));
    }

    /// Keep only the replies `keep(key, id)` says still have their card and owner; drop those older than
    /// [`REPLY_MS`] at `now` (a time ahead of `now`, a wrapped counter, restarts).
    pub fn retain(&mut self, now: u64, mut keep: impl FnMut(K, u32) -> bool) {
        self.list.retain_mut(|(k, c, _, at)| {
            *at = (*at).min(now);
            now - *at < REPLY_MS && keep(*k, *c)
        });
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::{CALL_ANSWER, CALL_REJECT, NOTIFY_ONGOING};
    use alloc::vec;

    #[test]
    fn a_ring_is_a_sticky_call_card() {
        let mut n = Notification {
            style: style::CALL,
            flags: NOTIFY_STICKY | NOTIFY_ONGOING,
            ..Notification::default()
        };
        assert!(rings(&n));
        n.flags = NOTIFY_ONGOING;
        assert!(!rings(&n), "the call in progress");
        n.flags = NOTIFY_STICKY;
        n.style = style::CONVERSATION;
        assert!(!rings(&n));
    }

    #[test]
    fn full_screen_when_off_locked_or_home() {
        assert!(full_screen(false, false, false));
        assert!(full_screen(true, true, false));
        assert!(full_screen(true, false, true));
        assert!(!full_screen(true, false, false), "in use: the banner");
    }

    /// protocol.md "Call keys for a cold owner": Answer on a card whose Phone isn't running waits for it to bind.
    #[test]
    fn call_key_waits_for_a_1_22_owner() {
        let mut h = Held::default();
        let answer = AppEvent::Notified {
            id: 3,
            action: CALL_ANSWER,
        };
        h.hold("phone", answer, 1000);
        h.hold("messages", AppEvent::NotifyReply { id: 1 }, 1000);
        assert!(h.waiting("phone"));
        assert_eq!(h.bound("phone", 22), vec![answer]);
        assert!(!h.waiting("phone") && h.waiting("messages"));
        h.hold("phone", answer, 1000);
        h.expire(1000 + HELD_MS - 1);
        assert!(h.waiting("phone"));
        h.expire(1000 + HELD_MS);
        assert!(!h.waiting("phone") && !h.waiting("messages"));
        for i in 0..10 {
            h.hold("phone", AppEvent::Notified { id: i, action: 0 }, 5);
        }
        assert_eq!(h.bound("phone", 22).len(), HELD_MAX);
    }

    /// A 1.21 Phone can't read 0xA0/0xA1 (its decoder would refuse them and it would end): it gets none, but still
    /// gets a held event of its own minor.
    #[test]
    fn call_key_dropped_for_a_1_21_owner() {
        let mut h = Held::default();
        let reject = AppEvent::Notified {
            id: 3,
            action: CALL_REJECT,
        };
        let open = AppEvent::Notified { id: 3, action: 1 };
        h.hold("phone", reject, 0);
        h.hold("phone", open, 0);
        assert_eq!(h.bound("phone", 21), vec![open]);
        assert!(!h.waiting("phone"));
    }

    #[test]
    fn replies_wait_one_per_card_then_go() {
        let mut r = Replies::default();
        assert!(!r.put("messages", 1, "", 0), "empty");
        assert!(!r.put("messages", 1, &"x".repeat(481), 0), "past 480 bytes");
        assert!(r.put("messages", 1, "On my way", 0));
        assert!(
            r.put("messages", 1, "OK", 10),
            "the second replaces the first"
        );
        assert_eq!(r.take("messages", 2, 480), Err(kerr::NOT_FOUND));
        assert_eq!(
            r.take("phone", 1, 480),
            Err(kerr::NOT_FOUND),
            "another owner"
        );
        assert_eq!(r.take("messages", 1, 1), Err(kerr::OVERFLOW));
        assert_eq!(r.take("messages", 1, 480).as_deref(), Ok("OK"));
        assert_eq!(
            r.take("messages", 1, 480),
            Err(kerr::NOT_FOUND),
            "taken once"
        );
        r.put("messages", 1, "OK", 0);
        r.put("messages", 2, "OK", 0);
        r.forget("messages", 1);
        r.retain(REPLY_MS - 1, |_, _| true);
        assert_eq!(r.take("messages", 1, 480), Err(kerr::NOT_FOUND));
        assert!(!r.is_empty());
        r.retain(REPLY_MS, |_, _| true);
        assert!(r.is_empty(), "60 s");
        r.put("messages", 3, "OK", 0);
        r.retain(1, |_, id| id != 3);
        assert!(r.is_empty(), "the card went");
    }
}

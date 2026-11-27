// SPDX-License-Identifier: Apache-2.0
//! (TP4) The inline reply in the shell (docs `neoui-shell/top-panel.md` "Messages", nt-reply): Reply on a card whose
//! app reads `REPLY_TEXT` opens a field in the banner with the shell's keyboard, the quick replies and, for Messages,
//! the SMS counter (neohw's `sms::count`); Send hands the text to the UI server for the card's app. The text is never
//! logged.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use neoui_proto::page::REPLY_MAX;
use slint::{ComponentHandle, Model};

use crate::{KeyboardFocus, Renderer, Reply, Shell};

/// The apps whose Reply opens the inline field, and the replies sent and not yet taken: (app key, card id, text).
#[derive(Default)]
pub(crate) struct State {
    apps: Rc<RefCell<Vec<String>>>,
    sent: Rc<RefCell<Vec<(String, u32, String)>>>,
}

/// The line under the field: Messages' SMS count ("Unicode · 145 left · 1 SMS"), who sends it and, with an app in
/// front, that the user stays there; and whether Send is allowed (1–480 bytes, at most 10 SMS).
#[must_use]
pub fn counter(key: &str, text: &str, front: Option<&str>) -> (String, bool) {
    let name = neoui_proto::apps::find(key).map_or(key, |a| a.name);
    let stay = front.map_or(String::new(), |f| format!(", you stay in {f}"));
    if text.len() > REPLY_MAX {
        return (
            format!("Too long · {} of {REPLY_MAX} bytes", text.len()),
            false,
        );
    }
    let mut ok = !text.trim().is_empty();
    let line = if key == "messages" {
        let c = sms::count(text);
        ok &= !c.too_long();
        format!(
            "{} · {} left · {} SMS · sent by {name}{stay}",
            if c.unicode { "Unicode" } else { "GSM" },
            c.left,
            c.parts
        )
    } else {
        format!("Sent by {name}{stay}")
    };
    (line, ok)
}

/// The app in front's name (the shell's own screens: none).
fn front(ui: &Shell) -> Option<&'static str> {
    ui.get_app_front()
        .then(|| neoui_proto::apps::find(ui.get_screen().as_str()).map(|a| a.name))
        .flatten()
}

pub(crate) fn wire(
    ui: &Shell,
    banner: Rc<RefCell<Option<(String, u32)>>>,
    rows: Rc<RefCell<Vec<(String, u32)>>>,
) -> State {
    let s = State::default();
    let r = ui.global::<Reply>();
    let (weak, shown) = (ui.as_weak(), banner.clone());
    r.on_edited(move |t| {
        let (Some(ui), Some((key, _))) = (weak.upgrade(), shown.borrow().clone()) else {
            return;
        };
        let (line, ok) = counter(&key, t.as_str(), front(&ui));
        let r = ui.global::<Reply>();
        r.set_counter(line.into());
        r.set_can_send(ok);
    });
    let (weak, shown, sink) = (ui.as_weak(), banner.clone(), s.sent.clone());
    r.on_send(move |t| {
        let Some(ui) = weak.upgrade() else { return };
        let Some((key, id)) = shown.borrow().clone() else {
            return;
        };
        if !counter(&key, t.as_str(), None).1 {
            return;
        }
        sink.borrow_mut().push((key, id, t.as_str().into()));
        let r = ui.global::<Reply>();
        r.set_open(false);
        r.set_text("".into());
        ui.set_banner_shown(false);
        ui.global::<KeyboardFocus>().invoke_close();
    });
    // Reply on a shade card: the banner takes the card and opens its field
    let weak = ui.as_weak();
    ui.on_reply_asked(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let Some(i) = usize::try_from(i).ok() else {
            return;
        };
        let (Some(card), Some(note)) = (rows.borrow().get(i).cloned(), ui.get_notes().row_data(i))
        else {
            return;
        };
        let (line, _) = counter(&card.0, "", front(&ui));
        *banner.borrow_mut() = Some(card);
        ui.set_banner(note);
        ui.set_banner_seconds(0);
        ui.set_banner_shown(true);
        let r = ui.global::<Reply>();
        r.set_text("".into());
        r.set_counter(line.into());
        r.set_can_send(false);
        r.set_open(true);
    });
    s
}

impl Renderer {
    /// (TP4) The apps whose Reply opens the inline field (running, protocol 1.22: they read `REPLY_TEXT`); for the
    /// rest Reply opens the app as before. Before [`Self::set_notes`].
    pub fn set_reply_apps(&mut self, keys: &[&str]) {
        *self.reply.apps.borrow_mut() = keys.iter().map(|k| (*k).into()).collect();
    }

    pub(crate) fn inline_reply(&self, key: &str) -> bool {
        self.reply.apps.borrow().iter().any(|k| k == key)
    }

    /// The banner went or took another card: a reply being written goes with it (its keyboard too).
    pub(crate) fn close_reply(&self) {
        let r = self.ui.global::<Reply>();
        if r.get_open() {
            r.set_open(false);
            r.set_text("".into());
            self.ui.global::<KeyboardFocus>().invoke_close();
        }
    }

    /// The replies sent since the last call: (app key, card id, text). The text goes to its app only.
    pub fn take_replies(&mut self) -> Vec<(String, u32, String)> {
        core::mem::take(&mut *self.reply.sent.borrow_mut())
    }

    /// The banner's reply field is open (the banner doesn't time out; it covers the app like an overlay).
    #[must_use]
    pub fn replying(&self) -> bool {
        self.ui.global::<Reply>().get_open() && self.ui.get_banner_shown()
    }

    /// (TP4) How long a banner shows (`notify.bannert`, its bar runs out over it); 0 for a sticky card's.
    pub fn set_banner_seconds(&mut self, secs: u32) {
        self.ui
            .set_banner_seconds(i32::try_from(secs).unwrap_or(i32::MAX));
    }
}

#[cfg(test)]
mod tests {
    use super::counter;

    #[test]
    fn the_sms_counter() {
        let (line, ok) = counter("messages", "On my way, 10 min", Some("Files"));
        assert_eq!(
            line,
            "GSM · 143 left · 1 SMS · sent by Messages, you stay in Files"
        );
        assert!(ok);
        let (line, _) = counter("messages", "Բարեւ", None);
        assert!(line.starts_with("Unicode · 65 left · 1 SMS"), "{line}");
        assert!(!counter("messages", "  ", None).1, "nothing to send");
        let (line, ok) = counter("messages", &"x".repeat(481), None);
        assert!(!ok && line.starts_with("Too long"));
        assert!(
            counter("messages", &"ա".repeat(240), None).1,
            "480 bytes, 4 SMS"
        );
        assert_eq!(counter("kit", "OK", None).0, "Sent by UI kit");
    }
}

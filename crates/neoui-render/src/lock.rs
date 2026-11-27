// SPDX-License-Identifier: Apache-2.0
//! (TP8, TP9) The lock screen, its PIN pad and SYSSTART's PIN dialog (ui/components/lock.slint's `Lock` global):
//! what the UI server shows there, and what the user did. A digit comes back as a [`LockEvent::Digit`] the moment it
//! is pressed; Slint only gets the count back ([`Renderer::set_lock_entry`]), never the PIN.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use neoui_proto::lock::CardRow;
use slint::ComponentHandle;

use crate::{Lock, LockCard, Renderer, Shell};

/// What the lock screen, the pad or the dialog asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockEvent {
    /// Swipe up (any edge in Gestures/N9) or a card tapped: unlock, or bring the PIN pad up.
    Swiped,
    /// A digit key, 0–9.
    Digit(u8),
    Delete,
    /// The dialog's Done or ✓.
    Submit,
    /// The pad's or the dialog's Cancel / ✕.
    Cancel,
    /// The emergency dialer's Call with what was typed (the server dials only an emergency number).
    EmergencyCall(String),
}

pub(crate) type Events = Rc<RefCell<Vec<LockEvent>>>;

pub(crate) fn wire(ui: &Shell) -> Events {
    let sink = Events::default();
    let g = ui.global::<Lock>();
    let out = sink.clone();
    g.on_key(move |k| {
        out.borrow_mut().push(match u8::try_from(k) {
            Ok(d @ 0..=9) => LockEvent::Digit(d),
            _ => LockEvent::Delete,
        });
    });
    let out = sink.clone();
    g.on_submit(move || out.borrow_mut().push(LockEvent::Submit));
    let out = sink.clone();
    g.on_cancel(move || out.borrow_mut().push(LockEvent::Cancel));
    let out = sink.clone();
    g.on_swiped(move || out.borrow_mut().push(LockEvent::Swiped));
    let out = sink.clone();
    g.on_emergency_call(move |n| {
        out.borrow_mut()
            .push(LockEvent::EmergencyCall(n.as_str().into()))
    });
    sink
}

/// The PIN dialog's texts (set2-lock-pin): its title, the line under it and the step.
pub struct DialogText<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub step: &'a str,
}

impl Renderer {
    /// The lock layer (`shown`) and the PIN pad over it (`pad`). Hiding the pad leaves its emergency dialer too.
    pub fn set_lock(&mut self, shown: bool, pad: bool) {
        let g = self.ui.global::<Lock>();
        g.set_shown(shown);
        g.set_pad(shown && pad);
        if !(shown && pad) {
            g.set_emergency(false);
        }
    }

    /// The lock layer covers the screen: the UI server composes no app over it and gives it every touch.
    #[must_use]
    pub fn lock_shown(&self) -> bool {
        self.ui.global::<Lock>().get_shown()
    }

    /// The PIN pad or the PIN dialog is up: no tap is logged and no Show touches ring drawn (the digits).
    #[must_use]
    pub fn lock_pin_up(&self) -> bool {
        let g = self.ui.global::<Lock>();
        (g.get_shown() && g.get_pad()) || g.get_dialog()
    }

    /// The cards the lock screen shows and how many more there are.
    pub fn set_lock_cards(&mut self, rows: &[CardRow], more: usize) {
        let model: Vec<LockCard> = rows
            .iter()
            .map(|r| LockCard {
                key: r.key.as_str().into(),
                head: r.head.as_str().into(),
                line: r.line.as_str().into(),
                time: r.time.as_str().into(),
            })
            .collect();
        let g = self.ui.global::<Lock>();
        g.set_cards(slint::ModelRc::new(slint::VecModel::from(model)));
        g.set_more(i32::try_from(more).unwrap_or(i32::MAX));
    }

    /// The live activity over the media player ("Clock · Timer · Pasta", "03:12", its colour), or none.
    pub fn set_lock_activity(&mut self, activity: Option<(&str, &str, u32)>) {
        let g = self.ui.global::<Lock>();
        let (title, time, rgb) = activity.unwrap_or(("", "", 0));
        g.set_activity_title(title.into());
        g.set_activity_time(time.into());
        if activity.is_some() {
            g.set_activity_tint(slint::Color::from_argb_encoded(0xFF00_0000 | rgb));
        }
    }

    /// How many digits are typed (the dots) and the pad's or dialog's line ("" for none).
    pub fn set_lock_entry(&mut self, digits: usize, message: &str) {
        let g = self.ui.global::<Lock>();
        g.set_digits(i32::try_from(digits).unwrap_or(0));
        g.set_message(message.into());
    }

    /// SYSSTART's PIN dialog (set2-lock-pin) at a step, or closed.
    pub fn set_lock_dialog(&mut self, d: Option<&DialogText<'_>>) {
        let g = self.ui.global::<Lock>();
        g.set_dialog(d.is_some());
        if let Some(d) = d {
            g.set_dialog_title(d.title.into());
            g.set_dialog_text(d.text.into());
            g.set_dialog_step(d.step.into());
        }
    }

    /// What the lock screen, the pad and the dialog did since the last call.
    pub fn take_lock_events(&mut self) -> Vec<LockEvent> {
        core::mem::take(&mut *self.lock.borrow_mut())
    }
}

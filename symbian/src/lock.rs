// SPDX-License-Identifier: GPL-3.0-only
//! (TP8, TP9) The lock screen and the screen-lock PIN in SYSSTART (docs `neoui-shell/top-panel.md` "Lock screen
//! (TP8)", `settings-data.md` D15), a child of `host`. The rules are `neoui_proto::lock`'s; this file applies them:
//! - the lock layer at screen-on (`lock.on`, or always with a PIN), a ringing call or alarm over it, an Urgent card
//!   waking the screen when `notify.lock` shows cards, the cards by their lock level;
//! - the PIN pad over it and the dialog `ACTION_LOCK_DIALOG` opens for Settings: the digits come here one key at a
//!   time, are checked against the verifier in `C:\private\e0002001\lock.dat` (attempts and back-off saved with it),
//!   and are never logged, stored or sent anywhere; only SYSSTART writes `lock.pin_set`.

use super::Host;
use crate::phone::Phone;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use e32::sys::err;
use neoui_proto::apps;
use neoui_proto::lock::{
    self as rules, CardIn, CardRow, Check, Enrol, ITERATIONS, Lock, Outcome, Pin, RECORD_LEN,
    Record, SALT_LEN, Step,
};
use neoui_proto::notification_policy::lock as level;
use neoui_proto::page::NOTIFY_PRIVATE;
use neoui_proto::settings::{Caller, Value, keys};
use neoui_render::Renderer;
use neoui_render::lock::{DialogText, LockEvent};

const PATH: &str = "C:\\private\\e0002001\\lock.dat";
const STAGING: &str = "C:\\private\\e0002001\\lock.new";
const DAMAGED: &str = "C:\\private\\e0002001\\lock.bad";

/// The lock's state in the host.
pub struct LockScreen {
    lock: Lock,
    /// The PIN's verifier, attempts and back-off (`None`: no PIN).
    record: Option<Record>,
    /// The digits typed on the pad or in the dialog.
    pin: Pin,
    message: String,
    dialog: Option<Enrol>,
    /// An Urgent card asked for the screen.
    urgent: bool,
    /// What the renderer shows: (layer, pad), (digits, line), the dialog's step, the cards.
    shown: Option<(bool, bool)>,
    entry: Option<(usize, String)>,
    dialog_shown: Option<Option<(u8, Step)>>,
    cards: Option<(Vec<CardRow>, usize)>,
}

impl LockScreen {
    /// Reads `lock.dat`: a PIN locks the phone from boot. A damaged file is kept aside (`lock.bad`) and the phone
    /// has no PIN: it was written whole or not at all (staged, then replaced), so damage means the drive's own.
    pub fn boot() -> Self {
        let record = match read() {
            Ok(b) => match Record::decode(&b) {
                Some(r) => Some(r),
                None => {
                    let r = Fs::connect().and_then(|fs| fs.replace_file(PATH, DAMAGED));
                    dprintln!("neoui: lock PIN file damaged, kept as lock.bad ({:?})", r);
                    None
                }
            },
            Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => None,
            Err(e) => {
                dprintln!("neoui: lock PIN file unreadable ({})", e);
                None
            }
        };
        dprintln!(
            "neoui: lock at boot: {}",
            if record.is_some() { "PIN" } else { "none" }
        );
        Self {
            lock: Lock::boot(record.is_some()),
            record,
            pin: Pin::default(),
            message: String::new(),
            dialog: None,
            urgent: false,
            shown: None,
            entry: None,
            dialog_shown: None,
            cards: None,
        }
    }

    fn pin_set(&self) -> bool {
        self.record.is_some()
    }

    /// An Urgent card woke the screen since the last call.
    pub fn take_urgent(&mut self) -> bool {
        core::mem::take(&mut self.urgent)
    }

    fn type_key(&mut self, ev: &LockEvent) {
        match ev {
            LockEvent::Digit(d) => {
                self.pin.push(*d);
            }
            LockEvent::Delete => self.pin.pop(),
            _ => {}
        }
    }

    /// Saves the record (or deletes it): staged and replaced in one file-server step.
    fn save(&self) -> Result<(), i32> {
        let fs = Fs::connect()?;
        let Some(r) = &self.record else {
            return match fs.delete(PATH) {
                Ok(()) | Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => Ok(()),
                Err(e) => Err(e),
            };
        };
        for dir in ["C:\\private\\", "C:\\private\\e0002001\\"] {
            match fs.mkdir(dir) {
                Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
                Err(e) => return Err(e),
            }
        }
        {
            let f = File::replace(&fs, STAGING, mode::SHARE_EXCLUSIVE)?;
            f.write(&r.encode())?;
            f.flush()?;
        }
        fs.replace_file(STAGING, PATH)
    }

    fn save_logged(&self, what: &str) {
        if let Err(e) = self.save() {
            dprintln!("neoui: lock PIN file not saved after {} ({})", what, e);
        }
    }
}

fn read() -> Result<Vec<u8>, i32> {
    let fs = Fs::connect()?;
    let f = File::open(&fs, PATH, mode::READ | mode::SHARE_READERS_ONLY)?;
    let mut b = alloc::vec![0; RECORD_LEN + 1];
    let n = f.read_at(0, &mut b)?;
    b.truncate(n);
    Ok(b)
}

/// A salt from the kernel's secure RNG (`Math::RandomL`).
fn salt() -> [u8; SALT_LEN] {
    let mut s = [0u8; SALT_LEN];
    if e32::sys::math_secure_random(&mut s) == 0 {
        return s;
    }
    // shortcut: the RNG isn't seeded yet (KErrNotReady): counters and clocks give a unique, guessable salt; it still
    // separates phones' verifiers, but upgrade to wait for the RNG once the kernel seeds it early on every board
    dprintln!("neoui: lock salt from counters (no secure random yet)");
    let mut seed = [0u8; 24];
    seed[..4].copy_from_slice(&e32::sys::fast_counter().to_le_bytes());
    seed[4..8].copy_from_slice(&e32::sys::ntick_count().to_le_bytes());
    seed[8..16].copy_from_slice(&e32::time::utc_now().0.to_le_bytes());
    seed[16..24].copy_from_slice(&e32::time::millis().to_le_bytes());
    rules::salt_from(&seed)
}

fn now_s() -> i64 {
    e32::time::utc_now().0
}

fn check_line(c: Check) -> String {
    match c {
        Check::Right => String::new(),
        Check::Wrong { left: 0 } => format!(
            "Wrong PIN · wait {} s",
            rules::wait_after(rules::FREE_TRIES)
        ),
        Check::Wrong { left: 1 } => "Wrong PIN · 1 try before a wait".into(),
        Check::Wrong { left } => format!("Wrong PIN · {left} tries before a wait"),
        Check::Wait(0) => "The PIN can't be checked".into(),
        Check::Wait(s) => format!("Too many wrong PINs · try again in {s} s"),
    }
}

impl Host {
    /// (TP8) The screen went off (or on): the lock closes over whatever was on screen.
    pub fn lock_screen_power(&mut self, on: bool) {
        if on {
            return;
        }
        let lock_on = self.settings.get(keys::LOCK_ON).unwrap_or(true);
        let l = &mut self.lock;
        let pin_set = l.pin_set();
        l.lock.screen_off(lock_on, pin_set);
        l.pin.clear();
        l.message.clear();
        // the PIN dialog never outlives the screen: it would show over the lock screen at wake
        if l.dialog.take().is_some() {
            dprintln!("neoui: lock PIN dialog closed by the screen going off");
        }
        if l.lock.locked() {
            dprintln!("neoui: locked");
        }
    }

    /// (TP8) What woke the screen: a ringing call or a service's `OPEN_WAKE` (an alarm) shows over the lock screen
    /// (`screen`, what `OPEN` brought up).
    pub fn lock_woke(&mut self, ring: bool, alarm: bool, screen: &str) {
        if ring {
            self.lock.lock.woke_for("phone");
        } else if alarm {
            self.lock.lock.woke_for(screen);
        }
    }

    /// (TP8) A new card's alert: an Urgent one (it has a banner) wakes a dark screen only when `notify.lock` shows
    /// cards and its own lock level does too.
    pub fn lock_card_alert(&mut self, key: &str, id: u32, banner: bool) {
        if !banner || self.settings.get(keys::NOTIFY_LOCK).unwrap_or(0) != 0 {
            return;
        }
        let shows = self
            .notifications
            .iter()
            .find(|(k, n)| *k == key && n.id == id)
            .is_some_and(|(k, n)| self.effective(k, n.channel).lock == level::SHOW);
        if shows {
            self.lock.urgent = true;
        }
    }

    /// (TP8, TP9) Once per frame, before the dark screen's early out: the layer, the pad, the dialog and the cards.
    pub fn lock_frame(&mut self, ui: &mut Renderer, phone: &mut Phone) {
        // only SYSSTART writes lock.pin_set: it follows the file (a reset keeps it, an old store learns it)
        let pin_set = self.lock.pin_set();
        if self.settings.get(keys::LOCK_PIN_SET).ok() != Some(pin_set) {
            let _ = crate::settings::set(
                &mut self.settings,
                Caller::Shell,
                keys::LOCK_PIN_SET.name(),
                Value::Bool(pin_set),
                e32::time::millis(),
            );
        }
        if let Some(mode) = self.lock_dialog.take() {
            self.lock_open_dialog(mode);
        }
        for ev in ui.take_lock_events() {
            if self.lock.dialog.is_some() {
                self.lock_dialog_event(ev);
            } else {
                self.lock_pad_event(ev, ui, phone);
            }
        }
        let shown = self.lock.lock.frame(&ui.screen(), phone.in_call());
        let pad = self.lock.lock.pad();
        if self.lock.shown != Some((shown, pad)) {
            if self.lock.shown.is_some_and(|(s, _)| s != shown) {
                dprintln!("neoui: lock {}", if shown { "shown" } else { "gone" });
            }
            self.lock.shown = Some((shown, pad));
            ui.set_lock(shown, pad);
            if !pad && self.lock.dialog.is_none() {
                self.lock.pin.clear();
                self.lock.message.clear();
            }
        }
        // shortcut: no live activity on the lock screen until the shell holds ACTIVITY records (TP6, W2 stream J):
        // then `ui.set_lock_activity` with the island's first one
        let entry = (self.lock.pin.len(), self.lock.message.clone());
        if self.lock.entry.as_ref() != Some(&entry) {
            ui.set_lock_entry(entry.0, &entry.1);
            self.lock.entry = Some(entry);
        }
        let step = self.lock.dialog.as_ref().map(|d| (d.mode, d.step));
        if self.lock.dialog_shown != Some(step) {
            self.lock.dialog_shown = Some(step);
            ui.set_lock_dialog(step.map(dialog_text).as_ref());
        }
        if shown {
            let cards = self.lock_cards();
            if self.lock.cards.as_ref() != Some(&cards) {
                ui.set_lock_cards(&cards.0, cards.1);
                self.lock.cards = Some(cards);
            }
        }
    }

    /// The cards the lock screen shows: one row per app by its cards' lock level, fewer under a media player.
    fn lock_cards(&self) -> (Vec<CardRow>, usize) {
        let (unix, offset) = e32::time::utc_now();
        let now_ms = u64::try_from(unix).unwrap_or(0) * 1000;
        let h24 = self.settings.get(keys::CLOCK_24H).unwrap_or(true);
        let times: Vec<String> = self
            .notifications
            .iter()
            .map(|(_, n)| {
                if n.when == 0 || !n.time.is_empty() {
                    n.time.clone()
                } else {
                    neoui_proto::shade::when_text(n.when, now_ms, offset, h24)
                }
            })
            .collect();
        let cards: Vec<CardIn<'_>> = self
            .notifications
            .iter()
            .zip(&times)
            .map(|((k, n), time)| CardIn {
                key: k,
                app: apps::find(k).map_or(k, |a| a.name),
                level: self.effective(k, n.channel).lock,
                private: n.flags & NOTIFY_PRIVATE != 0,
                title: &n.title,
                text: &n.text,
                time,
            })
            .collect();
        let max = if self.media.current.shown().is_some_and(|(_, _, live)| live) {
            3
        } else {
            4
        };
        rules::card_rows(&cards, max)
    }

    fn lock_pad_event(&mut self, ev: LockEvent, ui: &mut Renderer, phone: &mut Phone) {
        let l = &mut self.lock;
        match ev {
            LockEvent::Swiped => {
                let pin_set = l.pin_set();
                l.lock.swiped(pin_set);
                if !l.lock.locked() {
                    dprintln!("neoui: unlocked (swipe)");
                }
            }
            LockEvent::Cancel => {
                l.lock.cancel_pad();
                l.pin.clear();
                l.message.clear();
            }
            LockEvent::EmergencyCall(number) => {
                // the dialer is the pad's: only an emergency number is dialled from a locked phone
                if !phone.emergency(ui, &number) {
                    ui.set_phone_feedback(false, "Only emergency numbers can be called");
                }
            }
            LockEvent::Digit(_) | LockEvent::Delete => {
                l.type_key(&ev);
                l.message.clear();
                // the pad has no OK key (nt-lock-pin): it checks once the PIN's own number of digits is typed
                if l.record
                    .as_ref()
                    .is_some_and(|r| l.pin.len() == usize::from(r.digits))
                {
                    self.lock_check();
                }
            }
            LockEvent::Submit => self.lock_check(),
        }
    }

    /// The pad's PIN against the verifier; every check counts towards the back-off.
    fn lock_check(&mut self) {
        let l = &mut self.lock;
        let Some(rec) = l.record.as_mut() else {
            l.lock.unlock();
            return;
        };
        let c = rec.check(&l.pin, now_s());
        l.pin.clear();
        if c == Check::Right {
            l.lock.unlock();
            dprintln!("neoui: unlocked (PIN)");
        } else {
            l.message = check_line(c);
            dprintln!("neoui: lock PIN refused ({} wrong in a row)", rec.failures);
        }
        l.save_logged("a PIN check");
    }

    /// `ACTION_LOCK_DIALOG` from Settings (checked in `host.rs`): the dialog over it.
    fn lock_open_dialog(&mut self, mode: u8) {
        let l = &mut self.lock;
        l.pin.clear();
        l.message.clear();
        l.dialog = Enrol::new(mode, l.pin_set());
        dprintln!(
            "neoui: lock PIN dialog {}",
            match l.dialog.as_ref().map(|d| d.mode) {
                Some(rules::mode::ENROL) => "enrol",
                Some(rules::mode::CHANGE) => "change",
                Some(_) => "remove",
                None => "not needed (no PIN)",
            }
        );
    }

    fn lock_dialog_event(&mut self, ev: LockEvent) {
        let l = &mut self.lock;
        match ev {
            LockEvent::Digit(_) | LockEvent::Delete => {
                l.type_key(&ev);
                l.message.clear();
            }
            LockEvent::Cancel => {
                l.dialog = None;
                l.pin.clear();
                l.message.clear();
                dprintln!("neoui: lock PIN dialog cancelled");
            }
            LockEvent::Submit => {
                let Some(d) = l.dialog.as_mut() else { return };
                let salt = if d.step == Step::Confirm {
                    salt()
                } else {
                    [0; SALT_LEN]
                };
                let out = d.submit(&l.pin, l.record.as_mut(), now_s(), salt, ITERATIONS);
                l.pin.clear();
                l.message = match &out {
                    Outcome::Short => "4 to 8 digits".into(),
                    Outcome::Mismatch => "The PINs didn't match. Choose a PIN again.".into(),
                    Outcome::Refused(c) => check_line(*c),
                    _ => String::new(),
                };
                match out {
                    Outcome::Refused(_) => l.save_logged("a wrong PIN"),
                    Outcome::Done(rec) => {
                        let set = rec.is_some();
                        l.record = rec;
                        l.dialog = None;
                        l.save_logged("the PIN dialog");
                        // a removed PIN leaves the swipe lock (lock.on), a new one locks from the next screen-off
                        dprintln!("neoui: lock PIN {}", if set { "set" } else { "removed" });
                    }
                    _ => {}
                }
            }
            LockEvent::Swiped | LockEvent::EmergencyCall(_) => {}
        }
    }
}

fn dialog_text((m, step): (u8, Step)) -> DialogText<'static> {
    match (m, step) {
        (rules::mode::REMOVE, _) => DialogText {
            title: "Enter your PIN",
            text: "To turn the screen lock PIN off.",
            step: "",
        },
        (_, Step::Current) => DialogText {
            title: "Enter your current PIN",
            text: "Then you choose a new one.",
            step: "",
        },
        (_, Step::New) => DialogText {
            title: "Choose a PIN",
            text: "4 to 8 digits. You enter it when the phone wakes.",
            step: "Step 1 of 2 · next you enter it again",
        },
        (_, Step::Confirm) => DialogText {
            title: "Enter it again",
            text: "4 to 8 digits. You enter it when the phone wakes.",
            step: "Step 2 of 2",
        },
    }
}

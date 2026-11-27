// SPDX-License-Identifier: GPL-3.0-only
//! D11, SIM card lock (set2-sim-lock): MODEMD's SIM PIN protection (NeoModem 2.9's extended `STATUS`) on the
//! Mobile row and the page, and Use SIM PIN / Change SIM PIN on a worker thread with its own NeoModem session, so
//! the card's answer (up to MODEMD's 15 s) never holds the UI.
//!
//! The PINs: the sheet's masked fields are copied into fixed buffers and cleared at once (submit, Back, hide, another
//! page); the buffers are wiped after the request. They never reach a log, the settings store, the row model's facts
//! or search. Slint's own string memory can't be wiped (shortcut: cleared, not zeroed, until Slint offers a secret
//! string). Nothing here retries: a lost answer is "Result unknown".
use alloc::boxed::Box;
use alloc::string::String;
use core::cell::Cell;
use e32::dprintln;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sync::Mutex;
use e32::sys::{TPtr8, TPtrC8};
use modem_ipc::{Protection, Status, op, pin, sim};
use neoui_proto::settings_app::sim_lock::{self as sl, Gate, Lock};
use settings_ui::{SetState, SettingsApp};
use slint::{ComponentHandle, SharedString};

/// What MODEMD said last: the lock, the card's tries, the epoch and generation a request must name.
#[derive(Clone, Copy)]
struct Seen {
    lock: Lock,
    tries: Option<u8>,
    epoch: u32,
    generation: u32,
}

/// One request for the worker; its PINs are wiped as soon as the worker has copied them.
#[derive(Default)]
struct Job {
    /// None: a new PIN (CHANGE_PIN); Some(on): SET_PIN_PROTECTION
    on: Option<bool>,
    old: [u8; 8],
    old_n: usize,
    new: [u8; 8],
    new_n: usize,
    epoch: u32,
    generation: u32,
    tries: u8,
}

#[derive(Default)]
struct Shared {
    busy: bool,
    job: Option<Job>,
    /// the finished request: (result, protection asked for)
    done: Option<(i32, Option<bool>)>,
}

/// D11: the PIN changes are held back until their review's P1s are fixed (MODEMD refuses them too).
const PIN_CHANGES_ENABLED: bool = false;

pub struct SimLock {
    shared: Option<&'static Mutex<Shared>>,
    seen: Cell<Option<Seen>>,
    /// the one-try warning was shown: the next press sends
    confirmed: Cell<bool>,
}

impl Default for SimLock {
    fn default() -> Self {
        SimLock {
            shared: Mutex::new(Shared::default())
                .ok()
                .map(|m| &*Box::leak(Box::new(m))),
            seen: Cell::new(None),
            confirmed: Cell::new(false),
        }
    }
}

/// MODEMD's answer as the page's lock: `None` when it doesn't answer; a 2.8 MODEMD's 533 bytes have no protection.
fn lock_of(st: Option<&(Status, Option<Protection>)>) -> Lock {
    let Some((st, prot)) = st else {
        return Lock::NoModem;
    };
    match (st.sim, prot) {
        (sim::ABSENT, _) => Lock::NoSim,
        (sim::UNKNOWN | sim::ERROR, _) => Lock::NotReady,
        (_, None) => Lock::OldModem,
        (_, Some(p)) => match p.state {
            pin::REQUIRED => Lock::Required,
            pin::VERIFIED => Lock::On,
            pin::DISABLED => Lock::Off,
            pin::BLOCKED => Lock::Blocked,
            pin::PERM_BLOCKED => Lock::PermBlocked,
            _ => Lock::Unknown,
        },
    }
}

/// The extended STATUS on `session`, decoded by the length MODEMD wrote.
pub fn read(session: &Session) -> Option<(Status, Option<Protection>)> {
    let mut b = [0u8; Status::EXT_SIZE];
    let mut des = TPtr8::new(&mut b);
    let mut args = TIpcArgs::new();
    args.des8(0, &mut des).int(2, pin::EXTENDED);
    session.send_receive(op::STATUS, &args).ok()?;
    let n = des.len();
    Status::decode(b.get(..n)?)
}

impl SimLock {
    /// MODEMD's latest status (from Modem::poll) onto the Mobile row and the page; a finished request's outcome.
    pub fn show(&self, ui: &SettingsApp, st: Option<&(Status, Option<Protection>)>) {
        let s = ui.global::<SetState>();
        let lock = lock_of(st);
        let tries = st.and_then(|(st, _)| sl::attempts(st.pin_attempts));
        self.seen.set(st.map(|(st, p)| Seen {
            lock,
            tries,
            epoch: p.map_or(0, |p| p.epoch),
            generation: st.generation,
        }));
        s.set_sim_lock_text(lock.summary().into());
        s.set_sim_pin_on(matches!(lock, Lock::On | Lock::Required));
        // shortcut: changing the PIN is held back (MODEMD answers KErrNotSupported) until the D11 review's P1s are
        // fixed; the page reads the card only. Enable with MODEMD's PIN_CHANGES_ENABLED.
        s.set_sim_can_toggle(PIN_CHANGES_ENABLED && lock.can_toggle());
        s.set_sim_can_change(PIN_CHANGES_ENABLED && lock.can_change());
        s.set_sim_attempts(
            tries
                .map_or_else(String::new, |n| alloc::format!("{n}"))
                .into(),
        );
        s.set_sim_note(if PIN_CHANGES_ENABLED {
            lock.note().into()
        } else {
            "Read only for now: turning the SIM PIN on or off and changing it come after their security review.".into()
        });
        let Some(shared) = self.shared else { return };
        let mut g = shared.lock();
        s.set_sim_busy(g.busy);
        if let Some((r, on)) = g.done.take() {
            dprintln!("settings: SIM PIN request finished ({})", r);
            s.set_sim_outcome(sl::outcome(r, on, tries).into());
            // done: the sheet closes; refused: it stays with the outcome (its fields are empty already)
            if r == 0 && s.get_sheet().starts_with("sim-") {
                s.set_sheet(SharedString::new());
            }
            s.set_sim_warning(SharedString::new());
        }
    }

    /// The page's and the sheets' actions: "sim-toggle" + "1"/"0" (the switch's wish: its sheet), "sim-change" (its
    /// sheet), "sim-submit" (the sheet's fields), "sim-cancel".
    pub fn act(&self, ui: &SettingsApp, action: &str, value: &str) {
        let s = ui.global::<SetState>();
        let seen = self.seen.get();
        let lock = seen.map_or(Lock::NoModem, |v| v.lock);
        match action {
            "sim-toggle" | "sim-change" => {
                clear(ui);
                self.confirmed.set(false);
                s.set_sim_warning(SharedString::new());
                s.set_sim_outcome(SharedString::new());
                let change = action == "sim-change";
                let ok = if change {
                    lock.can_change()
                } else {
                    lock.can_toggle()
                };
                if !ok {
                    s.set_sim_outcome(lock.note().into());
                    return;
                }
                s.set_sim_want_on(value == "1");
                s.set_sheet(if change { "sim-change" } else { "sim-pin" }.into());
            }
            "sim-submit" => self.submit(ui),
            _ => {
                clear(ui);
                self.confirmed.set(false);
            }
        }
    }

    fn submit(&self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let change = s.get_sheet() == "sim-change";
        let mut job = Job {
            on: (!change).then(|| s.get_sim_want_on()),
            ..Job::default()
        };
        // copy the digits out, then empty the fields whatever happens next
        let copy = |t: &str, buf: &mut [u8; 8]| -> Result<usize, &'static str> {
            sl::check(t.as_bytes())?;
            buf[..t.len()].copy_from_slice(t.as_bytes());
            Ok(t.len())
        };
        let checked = (|| {
            job.old_n = copy(s.get_sim_old().as_str(), &mut job.old)?;
            if change {
                job.new_n = copy(s.get_sim_new().as_str(), &mut job.new)?;
                if s.get_sim_new() != s.get_sim_again() {
                    return Err("The new PINs don't match");
                }
            }
            Ok(())
        })();
        let warned = self.confirmed.replace(false);
        clear(ui);
        if let Err(why) = checked {
            wipe(&mut job);
            s.set_sim_warning(why.into());
            return;
        }
        let Some(seen) = self.seen.get() else {
            wipe(&mut job);
            s.set_sim_warning(Lock::NoModem.note().into());
            return;
        };
        match sl::gate(seen.lock, seen.tries, change, warned) {
            Gate::Send => {}
            Gate::ConfirmLast => {
                // the fields are empty again: the second press needs the PIN typed again, knowingly
                wipe(&mut job);
                self.confirmed.set(true);
                s.set_sim_warning(sl::LAST_TRY.into());
                return;
            }
            Gate::Refuse(why) => {
                wipe(&mut job);
                s.set_sim_warning(why.into());
                return;
            }
        }
        (job.epoch, job.generation, job.tries) =
            (seen.epoch, seen.generation, seen.tries.unwrap_or(0));
        let Some(shared) = self.shared else {
            wipe(&mut job);
            return;
        };
        {
            let mut g = shared.lock();
            if g.busy {
                drop(g);
                wipe(&mut job);
                s.set_sim_warning("Still waiting for the SIM's answer".into());
                return;
            }
            g.busy = true;
            g.job = Some(job);
        }
        s.set_sim_busy(true);
        s.set_sim_warning(SharedString::new());
        s.set_sim_outcome(SharedString::new());
        // shortcut: the JoinHandle is dropped (as storage.rs's scan), a few bytes per request until e32 frees it
        let spawned = e32::thread::spawn(move || {
            let (r, on) = work(shared);
            let mut g = shared.lock();
            g.busy = false;
            g.done = Some((r, on));
            0
        });
        if let Err(e) = spawned {
            dprintln!("settings: SIM PIN worker didn't start ({})", e);
            let mut g = shared.lock();
            if let Some(mut j) = g.job.take() {
                wipe(&mut j);
            }
            g.busy = false;
            s.set_sim_busy(false);
            s.set_sim_warning("Couldn't start; nothing was sent".into());
        }
    }
}

/// The worker: its own session, the status on it (MODEMD checks the session saw the epoch and generation it
/// names), the same card and tries the user saw, then the one request. Its PINs are wiped before it returns.
fn work(shared: &'static Mutex<Shared>) -> (i32, Option<bool>) {
    let Some(mut job) = shared.lock().job.take() else {
        return (-18, None);
    };
    let on = job.on;
    let r = send(&job);
    wipe(&mut job);
    (r, on)
}

fn send(job: &Job) -> i32 {
    let (major, minor, build) = modem_ipc::VERSION;
    let v = TVersion {
        major: major as u8,
        minor: minor as u8,
        build: build as u16,
    };
    let Ok(session) = Session::connect(modem_ipc::NAME, v) else {
        return -18;
    };
    match read(&session) {
        Some((st, Some(p)))
            if p.epoch == job.epoch
                && st.generation == job.generation
                && st.pin_attempts == job.tries => {}
        Some((_, None)) => return -5, // a 2.8 MODEMD: no PIN changes
        _ => return pin::STALE,
    }
    let old = TPtrC8::new(&job.old[..job.old_n]);
    let new = TPtrC8::new(&job.new[..job.new_n]);
    let mut args = TIpcArgs::new();
    args.desc8(0, &old)
        .int(2, job.epoch as i32)
        .int(3, job.generation as i32);
    let function = match job.on {
        Some(on) => {
            args.int(1, i32::from(on));
            op::SET_PIN_PROTECTION
        }
        None => {
            args.desc8(1, &new);
            op::CHANGE_PIN
        }
    };
    session
        .send_receive(function, &args)
        .map_or_else(|e| e, |_| 0)
}

fn wipe(job: &mut Job) {
    for b in job.old.iter_mut().chain(job.new.iter_mut()) {
        // SAFETY: a valid, exclusive reference.
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    job.old_n = 0;
    job.new_n = 0;
}

/// Empty the sheet's PIN fields (submit, Back, hide, another page).
pub fn clear(ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    for set in [
        SetState::set_sim_old,
        SetState::set_sim_new,
        SetState::set_sim_again,
    ] {
        set(&s, SharedString::new());
    }
}

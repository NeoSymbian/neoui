// SPDX-License-Identifier: Apache-2.0
//! (TP8, TP9) The lock screen and the screen-lock PIN (docs `neoui-shell/top-panel.md` "Lock screen (TP8)",
//! `settings-data.md` D15): the rules SYSSTART's `lock.rs` applies, pure so they are tested here.
//!
//! - [`Lock`]: when the lock layer covers the screen. It locks when the screen goes off (`lock.on`, or always with a
//!   PIN), at boot only with a PIN, and lets a ringing call or alarm show over it until that ends.
//! - [`Pin`]: the digits as typed, wiped when dropped. They never leave SYSSTART: Slint only sees how many.
//! - [`Record`]: the PIN's salted, versioned PBKDF2-HMAC-SHA1 verifier plus the wrong attempts and the back-off,
//!   the whole of `C:\private\e0002001\lock.dat`. No file: no PIN.
//! - [`Enrol`]: `LOCK_DIALOG`'s two-step enrolment, change and removal.
//! - [`card_rows`]: the cards the lock screen shows, by `notify.lock` and each card's own policy.

use crate::notification_policy::lock as level;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A PIN has 4 to 8 digits.
pub const PIN_MIN: usize = 4;
pub const PIN_MAX: usize = 8;
/// Wrong PINs before the first wait.
pub const FREE_TRIES: u16 = 5;
/// The first wait (s); each wrong PIN after it doubles the wait, up to [`WAIT_MAX_S`].
pub const WAIT_FIRST_S: u64 = 30;
pub const WAIT_MAX_S: u64 = 86_400;
/// PBKDF2 rounds for a new verifier. shortcut: picked, not measured (about 40k SHA-1 blocks); tune on the Pixel to
/// ~250 ms a check and raise it here: old records keep their own count.
pub const ITERATIONS: u32 = 20_000;
/// The record's layout version (1: PBKDF2-HMAC-SHA1).
pub const RECORD_VERSION: u8 = 1;
pub const SALT_LEN: usize = 16;
const HASH_LEN: usize = 20;
/// `version:b, iterations:w, salt:16, hash:20, failures:h, until_utc:q, digits:b`.
pub const RECORD_LEN: usize = 1 + 4 + SALT_LEN + HASH_LEN + 2 + 8 + 1;

/// The digits as typed (ASCII), wiped when cleared or dropped.
#[derive(Default)]
pub struct Pin {
    d: [u8; PIN_MAX],
    n: usize,
}

impl Pin {
    /// Adds digit 0–9; false when full or not a digit.
    pub fn push(&mut self, digit: u8) -> bool {
        if digit > 9 || self.n == PIN_MAX {
            return false;
        }
        self.d[self.n] = b'0' + digit;
        self.n += 1;
        true
    }

    pub fn pop(&mut self) {
        if self.n > 0 {
            self.n -= 1;
            self.d[self.n] = 0;
        }
    }

    pub fn clear(&mut self) {
        for b in &mut self.d {
            // SAFETY: a valid, aligned byte of our own array; volatile so the wipe isn't optimised away.
            unsafe { core::ptr::write_volatile(b, 0) };
        }
        self.n = 0;
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Long enough to submit.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.n >= PIN_MIN
    }

    fn bytes(&self) -> &[u8] {
        &self.d[..self.n]
    }

    /// The same digits (no early exit on the first difference).
    #[must_use]
    pub fn same(&self, other: &Pin) -> bool {
        self.n == other.n && eq(&self.d, &other.d)
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        self.clear();
    }
}

fn eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The wait after `failures` wrong PINs in a row (s): none for the first [`FREE_TRIES`], then 30 s doubling.
#[must_use]
pub fn wait_after(failures: u16) -> u64 {
    if failures < FREE_TRIES {
        return 0;
    }
    let doublings = u32::from(failures - FREE_TRIES).min(20);
    (WAIT_FIRST_S << doublings).min(WAIT_MAX_S)
}

/// `lock.dat`: the verifier, and the wrong attempts with the back-off they earned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub iterations: u32,
    pub salt: [u8; SALT_LEN],
    hash: [u8; HASH_LEN],
    pub failures: u16,
    /// When the wait ends (unix s; 0: none).
    pub until_utc: i64,
    /// How many digits the PIN has: the pad checks once that many are typed (it has no OK key, nt-lock-pin), and
    /// every check counts. Knowing it narrows a guess to one length of 4–8, as on other phones' auto-confirming pads.
    pub digits: u8,
}

/// What a PIN check said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Right,
    /// Wrong: how many tries are left before a wait (0: the wait started).
    Wrong {
        left: u16,
    },
    /// Not checked: wait this long first (s).
    Wait(u64),
}

impl Record {
    /// A new verifier for `pin` with `salt` (from the kernel's secure RNG when it has one).
    #[must_use]
    pub fn new(pin: &Pin, salt: [u8; SALT_LEN], iterations: u32) -> Self {
        Self {
            iterations,
            salt,
            hash: derive(pin, &salt, iterations),
            failures: 0,
            until_utc: 0,
            digits: pin.len() as u8,
        }
    }

    #[must_use]
    pub fn encode(&self) -> [u8; RECORD_LEN] {
        let mut b = [0u8; RECORD_LEN];
        b[0] = RECORD_VERSION;
        b[1..5].copy_from_slice(&self.iterations.to_le_bytes());
        b[5..21].copy_from_slice(&self.salt);
        b[21..41].copy_from_slice(&self.hash);
        b[41..43].copy_from_slice(&self.failures.to_le_bytes());
        b[43..51].copy_from_slice(&self.until_utc.to_le_bytes());
        b[51] = self.digits;
        b
    }

    /// `None` for another version, length, no rounds or a digit count outside 4–8.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != RECORD_LEN || b[0] != RECORD_VERSION {
            return None;
        }
        let iterations = u32::from_le_bytes(b[1..5].try_into().ok()?);
        (iterations > 0).then_some(())?;
        Some(Self {
            iterations,
            salt: b[5..21].try_into().ok()?,
            hash: b[21..41].try_into().ok()?,
            failures: u16::from_le_bytes(b[41..43].try_into().ok()?),
            until_utc: i64::from_le_bytes(b[43..51].try_into().ok()?),
            digits: Some(b[51]).filter(|d| (PIN_MIN..=PIN_MAX).contains(&usize::from(*d)))?,
        })
    }

    /// How long until a PIN may be tried (s). Never longer than the current wait: a clock set back (or reset at
    /// boot) restarts the wait instead of stretching it, and never shorter.
    #[must_use]
    pub fn wait_s(&self, now: i64) -> u64 {
        let full = wait_after(self.failures);
        let left = u64::try_from(self.until_utc.saturating_sub(now)).unwrap_or(0);
        if left > full { full } else { left }
    }

    /// Check `pin` at `now` (unix s): the attempt counts, and the record changes (save it after).
    pub fn check(&mut self, pin: &Pin, now: i64) -> Check {
        let wait = self.wait_s(now);
        if wait > 0 {
            // a clock set back: the wait restarts from now
            self.until_utc = now.saturating_add(wait as i64);
            return Check::Wait(wait);
        }
        if eq(&derive(pin, &self.salt, self.iterations), &self.hash) {
            self.failures = 0;
            self.until_utc = 0;
            return Check::Right;
        }
        self.failures = self.failures.saturating_add(1);
        let wait = wait_after(self.failures);
        self.until_utc = if wait > 0 {
            now.saturating_add(wait as i64)
        } else {
            0
        };
        Check::Wrong {
            left: FREE_TRIES.saturating_sub(self.failures),
        }
    }
}

fn derive(pin: &Pin, salt: &[u8], iterations: u32) -> [u8; HASH_LEN] {
    let mut out = [0u8; HASH_LEN];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(pin.bytes(), salt, iterations, &mut out);
    out
}

/// A salt from weak entropy (counters and clocks), for when the kernel's secure RNG isn't seeded: a salt needs to be
/// unique, not secret, so mixing them through one PBKDF2 round is enough to tell two phones' verifiers apart.
#[must_use]
pub fn salt_from(seed: &[u8]) -> [u8; SALT_LEN] {
    let mut out = [0u8; HASH_LEN];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(seed, b"neosymbian lock salt", 1, &mut out);
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&out[..SALT_LEN]);
    salt
}

/// `LOCK_DIALOG`'s modes (`settings_app::wire::lock_dialog`).
pub use crate::settings_app::wire::lock_dialog as mode;

/// Where the enrolment dialog is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The PIN set now (change, remove).
    Current,
    /// The new PIN (step 1 of 2).
    New,
    /// The new PIN again (step 2 of 2).
    Confirm,
}

/// What a submitted PIN did in the dialog.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Too short: nothing happened.
    Short,
    /// On to the next step.
    Next,
    /// The two new PINs differ: step 1 again.
    Mismatch,
    /// The current PIN was wrong, or must wait (the record changed: save it).
    Refused(Check),
    /// Finished: the new record (`None`: the PIN was removed). Save it, then set `lock.pin_set`.
    Done(Option<Record>),
}

/// `LOCK_DIALOG`'s dialog: enrol (new, again), change (current, new, again), remove (current).
pub struct Enrol {
    pub mode: u8,
    pub step: Step,
    first: Pin,
}

impl Enrol {
    /// The dialog for `mode` (0 enrol, 1 change, 2 remove). With a PIN set, enrol asks for it first like change;
    /// without one, change enrols and remove has nothing to do (`None`).
    #[must_use]
    pub fn new(mode: u8, pin_set: bool) -> Option<Self> {
        let mode = match (mode, pin_set) {
            (mode::REMOVE, false) => return None,
            (mode::ENROL | mode::CHANGE, true) => mode::CHANGE,
            (mode::ENROL | mode::CHANGE, false) => mode::ENROL,
            (m, _) => m,
        };
        Some(Self {
            mode,
            step: if pin_set { Step::Current } else { Step::New },
            first: Pin::default(),
        })
    }

    /// `pin` submitted at this step. `rec` is the PIN set now (checked at [`Step::Current`], counting towards the
    /// back-off); `salt` is used only when the new PIN is confirmed.
    pub fn submit(
        &mut self,
        pin: &Pin,
        rec: Option<&mut Record>,
        now: i64,
        salt: [u8; SALT_LEN],
        iterations: u32,
    ) -> Outcome {
        if !pin.complete() {
            return Outcome::Short;
        }
        match self.step {
            Step::Current => match rec.map(|r| r.check(pin, now)) {
                Some(Check::Right) if self.mode == mode::REMOVE => Outcome::Done(None),
                Some(Check::Right) => {
                    self.step = Step::New;
                    Outcome::Next
                }
                Some(c) => Outcome::Refused(c),
                // no record to check against (damaged): refuse, the PIN stays
                None => Outcome::Refused(Check::Wait(0)),
            },
            Step::New => {
                self.first.clear();
                for &b in pin.bytes() {
                    self.first.push(b - b'0');
                }
                self.step = Step::Confirm;
                Outcome::Next
            }
            Step::Confirm => {
                if pin.same(&self.first) {
                    self.first.clear();
                    Outcome::Done(Some(Record::new(pin, salt, iterations)))
                } else {
                    self.first.clear();
                    self.step = Step::New;
                    Outcome::Mismatch
                }
            }
        }
    }
}

/// The lock screen's state in SYSSTART.
#[derive(Debug, Default)]
pub struct Lock {
    locked: bool,
    /// The PIN pad is up over it.
    pad: bool,
    /// The app a ringing call ("phone") or an alarm (a service's `OPEN_WAKE`) brought over the lock screen.
    over: Option<String>,
}

impl Lock {
    /// At boot: locked only with a PIN (`lock.on` alone waits for the first screen-off, so a boot is usable at once).
    #[must_use]
    pub fn boot(pin_set: bool) -> Self {
        Self {
            locked: pin_set,
            ..Self::default()
        }
    }

    /// The lock screen is on: `lock.on`, or a PIN (a setting must never bypass an enrolled PIN).
    #[must_use]
    pub fn enabled(lock_on: bool, pin_set: bool) -> bool {
        lock_on || pin_set
    }

    /// The screen went off.
    pub fn screen_off(&mut self, lock_on: bool, pin_set: bool) {
        if Self::enabled(lock_on, pin_set) {
            self.locked = true;
        }
        self.pad = false;
        self.over = None;
    }

    #[must_use]
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// A ringing call or alarm brought `app` to the front: it shows over the lock screen until it ends.
    pub fn woke_for(&mut self, app: &str) {
        if self.locked {
            self.over = Some(app.into());
            self.pad = false;
        }
    }

    fn over_showing(&self, screen: &str, in_call: bool) -> bool {
        self.over
            .as_deref()
            .is_some_and(|o| o == screen && (o != "phone" || in_call))
    }

    /// Once per frame with the screen in front (`ui.screen()`) and whether a call is up: the call or alarm that was
    /// over the lock screen has ended once its app left the front (or the call ended). True while the lock layer
    /// covers the screen.
    pub fn frame(&mut self, screen: &str, in_call: bool) -> bool {
        if self.over.is_some() && !self.over_showing(screen, in_call) {
            self.over = None;
        }
        self.locked && self.over.is_none()
    }

    /// Swipe up (any edge in Gestures/N9): unlocked without a PIN, else the pad comes up.
    pub fn swiped(&mut self, pin_set: bool) {
        if !self.locked || self.over.is_some() {
            return;
        }
        if pin_set {
            self.pad = true;
        } else {
            self.unlock();
        }
    }

    /// The right PIN (or no lock any more).
    pub fn unlock(&mut self) {
        self.locked = false;
        self.pad = false;
        self.over = None;
    }

    #[must_use]
    pub fn pad(&self) -> bool {
        self.pad && self.locked && self.over.is_none()
    }

    /// The pad's Cancel: the lock screen again.
    pub fn cancel_pad(&mut self) {
        self.pad = false;
    }
}

/// What Settings calls the lock (set2-lock's "Lock screen style"): derived from `lock.on` and `lock.pin_set`, never
/// stored.
#[must_use]
pub fn style(lock_on: bool, pin_set: bool) -> &'static str {
    match (pin_set, lock_on) {
        (true, _) => "PIN",
        (false, true) => "Swipe",
        (false, false) => "None",
    }
}

/// One card as the lock screen gets it: the app (key and name), its effective lock level
/// (`notification_policy::lock`), whether it is marked private (`NOTIFY_PRIVATE`), its title and text, and "now" /
/// "10:24" as the shade writes it.
pub struct CardIn<'a> {
    pub key: &'a str,
    pub app: &'a str,
    pub level: u8,
    pub private: bool,
    pub title: &'a str,
    pub text: &'a str,
    pub time: &'a str,
}

/// A row of the lock screen (nt-lock): "Messages · 2" over "Ava Lindqvist · content hidden", and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardRow {
    pub key: String,
    pub head: String,
    pub line: String,
    pub time: String,
}

/// The lock screen's rows from the cards (newest first), one per app (its newest card, with the app's count), at
/// most `max`, and how many cards more ("+3 more"). Cards the policy keeps off the lock screen don't count.
#[must_use]
pub fn card_rows(cards: &[CardIn<'_>], max: usize) -> (Vec<CardRow>, usize) {
    let shown: Vec<&CardIn<'_>> = cards.iter().filter(|c| c.level != level::OFF).collect();
    let mut rows: Vec<(CardRow, usize)> = Vec::new();
    for c in &shown {
        if let Some((_, n)) = rows.iter_mut().find(|(r, _)| r.key == c.key) {
            *n += 1;
            continue;
        }
        let line = match c.level {
            level::COUNTS => String::new(),
            level::HIDE_CONTENT => hidden(c.title),
            _ if c.private => hidden(c.title),
            _ => {
                let newest = c.text.rsplit('\n').next().unwrap_or("");
                match (c.title.is_empty(), newest.is_empty()) {
                    (_, true) => c.title.into(),
                    (true, false) => newest.into(),
                    (false, false) => format!("{} · {}", c.title, newest),
                }
            }
        };
        rows.push((
            CardRow {
                key: c.key.into(),
                head: c.app.into(),
                line,
                time: c.time.into(),
            },
            1,
        ));
    }
    let more = rows.iter().skip(max).map(|(_, n)| n).sum();
    let rows = rows
        .into_iter()
        .take(max)
        .map(|(mut r, n)| {
            if n > 1 {
                r.head = format!("{} · {}", r.head, n);
            }
            if r.line.is_empty() {
                r.line = if n == 1 {
                    "1 notification".into()
                } else {
                    format!("{n} notifications")
                };
            }
            r
        })
        .collect();
    (rows, more)
}

fn hidden(title: &str) -> String {
    if title.is_empty() {
        "Content hidden".into()
    } else {
        format!("{title} · content hidden")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(s: &str) -> Pin {
        let mut p = Pin::default();
        for c in s.bytes() {
            assert!(p.push(c - b'0'));
        }
        p
    }

    const SALT: [u8; SALT_LEN] = *b"0123456789abcdef";

    #[test]
    fn pbkdf2_hmac_sha1_matches_rfc_6070() {
        let mut out = [0u8; 20];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(b"password", b"salt", 2, &mut out);
        assert_eq!(
            out,
            [
                0xea, 0x6c, 0x01, 0x4d, 0xc7, 0x2d, 0x6f, 0x8c, 0xcd, 0x1e, 0xd9, 0x2a, 0xce, 0x1d,
                0x41, 0xf0, 0xd8, 0xde, 0x89, 0x57
            ]
        );
    }

    #[test]
    fn a_pin_takes_4_to_8_digits_and_wipes() {
        let mut p = pin("1234");
        assert!(p.complete());
        p.pop();
        assert!(!p.complete());
        assert!(!p.push(10));
        let mut long = pin("12345678");
        assert!(!long.push(9));
        long.clear();
        assert!(long.is_empty() && long.d == [0; PIN_MAX]);
        assert!(pin("1234").same(&pin("1234")) && !pin("1234").same(&pin("12345")));
    }

    #[test]
    fn the_record_round_trips_and_refuses_another_version() {
        let r = Record::new(&pin("2468"), SALT, 3);
        let b = r.encode();
        assert_eq!(Record::decode(&b), Some(r.clone()));
        let mut v2 = b;
        v2[0] = 2;
        assert_eq!(Record::decode(&v2), None);
        assert_eq!(Record::decode(&b[..RECORD_LEN - 1]), None);
        let mut zero = b;
        zero[1..5].copy_from_slice(&[0; 4]);
        assert_eq!(Record::decode(&zero), None);
        assert_eq!(r.digits, 4);
        let mut long = b;
        long[RECORD_LEN - 1] = 9;
        assert_eq!(Record::decode(&long), None);
        // the salt matters: the same PIN, another salt, another hash
        assert_ne!(Record::new(&pin("2468"), [7; SALT_LEN], 3).hash, r.hash);
    }

    #[test]
    fn five_wrong_then_30_s_doubling() {
        assert_eq!(
            (1..=8).map(wait_after).collect::<Vec<_>>(),
            [0, 0, 0, 0, 30, 60, 120, 240]
        );
        assert_eq!(wait_after(u16::MAX), WAIT_MAX_S);
        let mut r = Record::new(&pin("2468"), SALT, 2);
        let t = 1_000_000;
        for left in (0..4).rev() {
            assert_eq!(r.check(&pin("0000"), t), Check::Wrong { left: left + 1 });
        }
        assert_eq!(r.check(&pin("0000"), t), Check::Wrong { left: 0 });
        // even the right PIN waits
        assert_eq!(r.check(&pin("2468"), t + 10), Check::Wait(20));
        assert_eq!(r.check(&pin("0000"), t + 30), Check::Wrong { left: 0 });
        assert_eq!(r.wait_s(t + 30), 60);
        // across a reboot: the saved record keeps counting
        let mut again = Record::decode(&r.encode()).unwrap();
        assert_eq!(again.check(&pin("2468"), t + 90), Check::Right);
        assert_eq!((again.failures, again.until_utc), (0, 0));
    }

    #[test]
    fn a_clock_set_back_restarts_the_wait_but_never_stretches_it() {
        let mut r = Record::new(&pin("2468"), SALT, 2);
        for _ in 0..5 {
            r.check(&pin("1111"), 5_000);
        }
        assert_eq!(r.wait_s(5_000), 30);
        // the clock went back to 1970 at boot: a whole wait from now, not 5000 s
        assert_eq!(r.wait_s(0), 30);
        assert_eq!(r.check(&pin("2468"), 0), Check::Wait(30));
        assert_eq!(r.until_utc, 30);
        assert_eq!(r.check(&pin("2468"), 31), Check::Right);
    }

    #[test]
    fn enrolment_takes_the_pin_twice() {
        let mut e = Enrol::new(mode::ENROL, false).unwrap();
        assert_eq!(e.step, Step::New);
        assert_eq!(e.submit(&pin("12"), None, 0, SALT, 2), Outcome::Short);
        assert_eq!(e.submit(&pin("1357"), None, 0, SALT, 2), Outcome::Next);
        assert_eq!(e.step, Step::Confirm);
        assert_eq!(e.submit(&pin("1358"), None, 0, SALT, 2), Outcome::Mismatch);
        assert_eq!(e.step, Step::New);
        e.submit(&pin("1357"), None, 0, SALT, 2);
        let Outcome::Done(Some(mut r)) = e.submit(&pin("1357"), None, 0, SALT, 2) else {
            panic!("not enrolled");
        };
        assert_eq!(r.check(&pin("1357"), 0), Check::Right);
    }

    #[test]
    fn change_and_remove_ask_for_the_current_pin() {
        let mut r = Record::new(&pin("2468"), SALT, 2);
        assert!(Enrol::new(mode::REMOVE, false).is_none());
        // enrol over a PIN is a change
        let mut e = Enrol::new(mode::ENROL, true).unwrap();
        assert_eq!((e.mode, e.step), (mode::CHANGE, Step::Current));
        assert_eq!(
            e.submit(&pin("1111"), Some(&mut r), 0, SALT, 2),
            Outcome::Refused(Check::Wrong { left: 4 })
        );
        assert_eq!(
            e.submit(&pin("2468"), Some(&mut r), 0, SALT, 2),
            Outcome::Next
        );
        assert_eq!(r.failures, 0);
        e.submit(&pin("97531"), None, 0, SALT, 2);
        assert!(matches!(
            e.submit(&pin("97531"), None, 0, SALT, 2),
            Outcome::Done(Some(_))
        ));
        let mut e = Enrol::new(mode::REMOVE, true).unwrap();
        assert_eq!(
            e.submit(&pin("2468"), Some(&mut r), 0, SALT, 2),
            Outcome::Done(None)
        );
        // a damaged record never lets a PIN go
        let mut e = Enrol::new(mode::REMOVE, true).unwrap();
        assert!(matches!(
            e.submit(&pin("2468"), None, 0, SALT, 2),
            Outcome::Refused(_)
        ));
    }

    #[test]
    fn locks_at_screen_off_and_at_boot_only_with_a_pin() {
        assert!(!Lock::boot(false).locked() && Lock::boot(true).locked());
        let mut l = Lock::boot(false);
        l.screen_off(false, false);
        assert!(!l.frame("home", false));
        // lock.on off can't bypass a PIN
        l.screen_off(false, true);
        assert!(l.frame("home", false));
        l.swiped(true);
        assert!(l.pad() && l.locked());
        l.cancel_pad();
        assert!(!l.pad());
        l.unlock();
        l.screen_off(true, false);
        assert!(l.frame("settings", false));
        l.swiped(false);
        assert!(!l.frame("settings", false));
    }

    #[test]
    fn a_call_or_alarm_shows_over_it_and_returns_to_it() {
        let mut l = Lock::boot(true);
        l.woke_for("phone");
        assert!(!l.frame("phone", true), "the ringing call over the lock");
        l.swiped(true);
        assert!(!l.pad(), "no pad over the call");
        assert!(l.frame("phone", false), "the call ended: locked again");
        assert!(
            l.frame("phone", true),
            "a later call screen doesn't skip it"
        );
        l.woke_for("clock");
        assert!(!l.frame("clock", false));
        assert!(l.frame("home", false), "the alarm's app left");
        // unlocked: nothing is over anything
        l.unlock();
        l.woke_for("clock");
        assert!(!l.frame("home", false));
    }

    #[test]
    fn weak_salts_still_differ() {
        assert_ne!(salt_from(&[1, 2, 3]), salt_from(&[1, 2, 4]));
        assert_eq!(salt_from(&[9]), salt_from(&[9]));
    }

    #[test]
    fn the_style_is_derived() {
        assert_eq!(style(false, false), "None");
        assert_eq!(style(true, false), "Swipe");
        assert_eq!(style(false, true), "PIN");
    }

    #[test]
    fn cards_follow_their_lock_level() {
        let c = |key, app, level, title, text| CardIn {
            key,
            app,
            level,
            private: false,
            title,
            text,
            time: "now",
        };
        let cards = [
            c(
                "messages",
                "Messages",
                level::HIDE_CONTENT,
                "Ava Lindqvist",
                "Hi\nAre we on?",
            ),
            c("messages", "Messages", level::SHOW, "Ben", "Yes"),
            c(
                "calendar",
                "Calendar",
                level::SHOW,
                "Lunch with Ben",
                "13:00",
            ),
            c("files", "Files", level::OFF, "Copied", ""),
            c("phone", "Phone", level::COUNTS, "Missed call", "Ben"),
            c("browser", "Browser", level::SHOW, "Downloaded", "a.pdf"),
            c("browser", "Browser", level::SHOW, "Downloaded", "b.pdf"),
        ];
        let (rows, more) = card_rows(&cards, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].head.as_str(), rows[0].line.as_str()),
            ("Messages · 2", "Ava Lindqvist · content hidden")
        );
        assert_eq!(rows[1].line, "Lunch with Ben · 13:00");
        // Phone (1) and Browser (2): Files is off the lock screen
        assert_eq!(more, 3);
        let (all, more) = card_rows(&cards, 9);
        assert_eq!(more, 0);
        assert_eq!(
            (all[2].head.as_str(), all[2].line.as_str()),
            ("Phone", "1 notification")
        );
        assert!(all.iter().all(|r| !r.line.contains("Ben\n")));
        let private = CardIn {
            private: true,
            ..c("messages", "Messages", level::SHOW, "", "secret")
        };
        assert_eq!(card_rows(&[private], 1).0[0].line, "Content hidden");
    }
}

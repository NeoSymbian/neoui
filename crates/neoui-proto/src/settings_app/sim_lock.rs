// SPDX-License-Identifier: Apache-2.0
//! D11, SIM card lock (set2-sim-lock): what the page, the Mobile row and search say about the SIM PIN, when a
//! change may be sent, and what its answer means. Pure: NEOSET's `sim_lock.rs` maps MODEMD's status (NeoModem 2.9,
//! `STATUS` with the protection) to a [`Lock`] and the card's tries; only the card supplies a try count.
//!
//! Settings never enters a PUK: a blocked SIM says so and points to the carrier. A request at one try left needs a
//! second confirmation ([`gate`]); with no known count nothing is sent.

/// The SIM PIN as far as MODEMD can say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    /// MODEMD doesn't answer
    NoModem,
    NoSim,
    /// a SIM, not ready or in error
    NotReady,
    /// a MODEMD older than NeoModem 2.9: no protection state, so no change offered
    OldModem,
    /// the card doesn't say
    Unknown,
    Off,
    /// on, and entered since the phone started
    On,
    /// on, not entered yet: the start-up prompt asks for it
    Required,
    /// the PUK is needed
    Blocked,
    PermBlocked,
}

impl Lock {
    /// The number Slint's `sim-lock` takes (the page's states).
    #[must_use]
    pub fn id(self) -> i32 {
        self as i32
    }

    /// The Mobile row's and search's value, and the page's line under Use SIM PIN when it can't be used.
    #[must_use]
    pub fn summary(self) -> &'static str {
        match self {
            Lock::NoModem => "Modem unavailable",
            Lock::NoSim => "No SIM",
            Lock::NotReady => "SIM not ready",
            Lock::OldModem => "This modem service can't say",
            Lock::Unknown => "The SIM doesn't say",
            Lock::Off => "SIM PIN off",
            Lock::On => "SIM PIN on",
            Lock::Required => "SIM PIN on, not entered yet",
            Lock::Blocked => "SIM blocked: PUK needed",
            Lock::PermBlocked => "SIM blocked for good",
        }
    }

    /// The Use SIM PIN switch can be flipped (it then asks for the PIN).
    #[must_use]
    pub fn can_toggle(self) -> bool {
        matches!(self, Lock::Off | Lock::On)
    }

    /// Change SIM PIN works only with the PIN on and entered.
    #[must_use]
    pub fn can_change(self) -> bool {
        self == Lock::On
    }

    /// What the page says beneath its rows when the PIN can't be changed here.
    #[must_use]
    pub fn note(self) -> &'static str {
        match self {
            Lock::NoModem => {
                "The modem service isn't answering, so the SIM PIN can't be read or changed now."
            }
            Lock::NoSim => "Insert a SIM to see or change its PIN.",
            Lock::NotReady => "The SIM isn't ready yet; its PIN can't be changed now.",
            Lock::OldModem => {
                "This modem service doesn't report the SIM PIN, so Settings can't change it."
            }
            Lock::Unknown => {
                "The SIM doesn't say whether its PIN is on, so Settings can't change it."
            }
            Lock::Required => "Enter the PIN at the start-up prompt first.",
            Lock::Blocked => {
                "Unlock the SIM with your carrier's PUK procedure. Settings can't enter a PUK."
            }
            Lock::PermBlocked => {
                "This SIM can't be unlocked any more. Ask your carrier for a new one."
            }
            Lock::Off | Lock::On => {
                "A wrong PIN uses an attempt. With none left the SIM is blocked and needs your carrier's PUK, which Settings can't enter."
            }
        }
    }
}

/// "PIN attempts left": the card's count; `None` (255 from MODEMD) when it doesn't say.
#[must_use]
pub fn attempts(raw: u8) -> Option<u8> {
    (raw != 255).then_some(raw)
}

/// What a press of the sheet's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Send,
    /// one try left: say so, and send only on a second press
    ConfirmLast,
    /// nothing is sent: why
    Refuse(&'static str),
}

/// May a request go to the card now? `change`: Change SIM PIN (else Use SIM PIN); `confirmed`: the one-try warning
/// was shown and pressed through.
#[must_use]
pub fn gate(lock: Lock, tries: Option<u8>, change: bool, confirmed: bool) -> Gate {
    let ok = if change {
        lock.can_change()
    } else {
        lock.can_toggle()
    };
    if !ok {
        return Gate::Refuse(lock.note());
    }
    match tries {
        None => Gate::Refuse("The SIM doesn't say how many attempts are left; nothing was sent."),
        Some(0) => Gate::Refuse(Lock::Blocked.note()),
        Some(1) if !confirmed => Gate::ConfirmLast,
        Some(_) => Gate::Send,
    }
}

/// The one-try warning the sheet shows before the second press.
pub const LAST_TRY: &str = "Only 1 attempt left. If this PIN is wrong the SIM is blocked and needs your carrier's PUK. Press again to continue.";

/// A PIN as typed: 4 to 8 digits, leading zeroes kept.
pub fn check(pin: &[u8]) -> Result<(), &'static str> {
    if !(4..=8).contains(&pin.len()) {
        Err("A SIM PIN has 4 to 8 digits")
    } else if !pin.iter().all(u8::is_ascii_digit) {
        Err("Digits only")
    } else {
        Ok(())
    }
}

/// What a finished request means (NeoModem 2.9 results; `on`: the protection asked for, `None` for a new PIN),
/// with the card's count after it.
#[must_use]
pub fn outcome(result: i32, on: Option<bool>, tries: Option<u8>) -> alloc::string::String {
    use alloc::format;
    use alloc::string::String;
    let left = |n: Option<u8>| match n {
        Some(1) => String::from("1 attempt left."),
        Some(n) => format!("{n} attempts left."),
        None => String::from("The SIM doesn't say how many attempts are left."),
    };
    match result {
        0 => String::from(match on {
            Some(true) => "SIM PIN on. The phone asks for it when it starts.",
            Some(false) => "SIM PIN off.",
            None => "SIM PIN changed.",
        }),
        -1012 => format!("Wrong PIN. {}", left(tries)),
        -1035 => String::from(
            "Wrong PIN. The SIM is blocked: unlock it with your carrier's PUK procedure.",
        ),
        -1036 => String::from("The SIM is blocked for good."),
        -33 => String::from("Result unknown; check the SIM status. Nothing was sent again."),
        -39 => String::from("The SIM or the modem changed; nothing was sent. Check and try again."),
        -14 => String::from("The modem is busy; nothing was sent. Try again in a moment."),
        -18 => String::from("The SIM isn't ready; nothing was sent."),
        -46 => String::from("Only Settings may change the SIM PIN."),
        -5 => String::from("This modem service can't change the SIM PIN."),
        e => format!("The SIM refused ({e}). {}", left(tries)),
    }
}

/// A PIN field's digits, held here and not in the UI (D11): the sheet sends each key ([`PinBuf::key`]) and shows
/// only [`PinBuf::len`] dots, so no UI string ever holds the PIN. Zeroed by [`PinBuf::clear`] and on drop.
#[derive(Default)]
pub struct PinBuf {
    b: [u8; 8],
    n: usize,
}

impl PinBuf {
    /// A key from the field: a digit is added (up to 8), Backspace (U+0008) takes the last one away. False for
    /// any other key, which the field leaves to others.
    pub fn key(&mut self, key: &str) -> bool {
        match key.as_bytes() {
            [d] if d.is_ascii_digit() => {
                if self.n < self.b.len() {
                    self.b[self.n] = *d;
                    self.n += 1;
                }
                true
            }
            [8] => {
                if self.n > 0 {
                    self.n -= 1;
                    wipe(&mut self.b[self.n..]);
                }
                true
            }
            _ => false,
        }
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.b[..self.n]
    }
    pub fn clear(&mut self) {
        wipe(&mut self.b);
        self.n = 0;
    }
}

impl Drop for PinBuf {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Zero `b` so the compiler can't drop the writes as dead (the buffer is freed or reused next).
pub fn wipe(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: a valid, aligned, exclusive reference.
        unsafe { core::ptr::write_volatile(x, 0) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_known_on_or_off_pin_with_known_tries_is_sent() {
        assert_eq!(gate(Lock::Off, Some(3), false, false), Gate::Send);
        assert_eq!(gate(Lock::On, Some(2), true, false), Gate::Send);
        // change needs it on; nothing is sent for the other states
        assert!(matches!(
            gate(Lock::Off, Some(3), true, false),
            Gate::Refuse(_)
        ));
        for l in [
            Lock::NoModem,
            Lock::NoSim,
            Lock::NotReady,
            Lock::OldModem,
            Lock::Unknown,
            Lock::Required,
            Lock::Blocked,
            Lock::PermBlocked,
        ] {
            assert!(
                matches!(gate(l, Some(3), false, true), Gate::Refuse(_)),
                "{l:?}"
            );
            assert!(
                matches!(gate(l, Some(3), true, true), Gate::Refuse(_)),
                "{l:?}"
            );
        }
        assert!(matches!(gate(Lock::On, None, false, true), Gate::Refuse(_)));
        assert!(matches!(
            gate(Lock::On, Some(0), false, true),
            Gate::Refuse(_)
        ));
        assert_eq!(attempts(255), None);
        assert_eq!(attempts(3), Some(3));
    }

    #[test]
    fn one_try_left_needs_a_second_press() {
        assert_eq!(gate(Lock::On, Some(1), false, false), Gate::ConfirmLast);
        assert_eq!(gate(Lock::On, Some(1), true, false), Gate::ConfirmLast);
        assert_eq!(gate(Lock::On, Some(1), false, true), Gate::Send);
    }

    #[test]
    fn pins_and_outcomes() {
        assert!(check(b"0000").is_ok() && check(b"01234567").is_ok());
        assert!(check(b"123").is_err() && check(b"123456789").is_err() && check(b"12 4").is_err());
        assert_eq!(
            outcome(-1012, Some(true), Some(2)),
            "Wrong PIN. 2 attempts left."
        );
        assert_eq!(outcome(-1012, None, Some(1)), "Wrong PIN. 1 attempt left.");
        assert!(outcome(-33, Some(false), None).starts_with("Result unknown"));
        assert!(outcome(-1035, None, Some(0)).contains("PUK"));
        assert_eq!(outcome(0, None, Some(3)), "SIM PIN changed.");
        assert!(Lock::Blocked.note().contains("can't enter a PUK"));
    }

    #[test]
    fn a_pin_buffer_takes_digits_and_backspace_only_and_wipes() {
        let mut p = PinBuf::default();
        for k in ["0", "1", "x", "2", "3", "\u{1b}"] {
            p.key(k);
        }
        assert_eq!(p.as_bytes(), b"0123");
        assert!(!p.key("a") && !p.key("12") && p.key("\u{8}"));
        assert_eq!(p.as_bytes(), b"012");
        assert_eq!(p.b[3], 0, "a removed digit is zeroed");
        for _ in 0..10 {
            p.key("9");
        }
        assert_eq!(p.len(), 8, "8 digits at most");
        p.clear();
        assert!(p.is_empty() && p.b == [0; 8]);
    }
}

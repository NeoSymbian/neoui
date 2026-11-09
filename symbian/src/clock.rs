// SPDX-License-Identifier: GPL-3.0-only
//! The clock across restarts and power-offs, and the way out (NeoSymbian H3a).
//!
//! At boot the server sets Symbian's clock from a reference clock plus the saved offset ([`neoui_proto::clock`]):
//! - the board RTC where there is one (the phone);
//! - otherwise the kernel's own clock as it was at boot plus the time since (Syborg: its baseport starts the
//!   kernel clock from the host's RTC, so that clock carries over a reboot even though nothing can read an RTC).
//!
//! Every [`CHECK_MS`], and before a power-off or restart, it compares the clock against what it last saw: a change
//! means someone set the time (the Clock app, the debug monitor's `date`), and it saves the new offset and zone.

use e32::fs::{File, Fs, mode};
use e32::sys::err;
use e32::{dprintln, time};
use neoui_proto::clock::{self, Saved};

/// How often the clock is checked for a change: a time set just before a reboot is kept.
const CHECK_MS: u64 = 5_000;

/// What the saved offset is measured against.
#[derive(Clone, Copy, Debug)]
enum Reference {
    /// The board RTC (raw seconds, no fixed epoch; it counts while the phone is off).
    Rtc,
    /// The kernel's UTC at boot, before anything set it, advanced by the time since.
    BootClock { utc_at_boot: i64, ms_at_boot: u64 },
}

impl Reference {
    fn detect() -> Self {
        if time::rtc_seconds().is_some() {
            Reference::Rtc
        } else {
            Reference::BootClock {
                utc_at_boot: time::utc_now().0,
                ms_at_boot: time::millis(),
            }
        }
    }

    /// The reference's seconds now; `None` when it can't be read (or falls outside `u32`).
    fn seconds(&self) -> Option<u32> {
        match *self {
            Reference::Rtc => time::rtc_seconds(),
            Reference::BootClock {
                utc_at_boot,
                ms_at_boot,
            } => {
                let elapsed =
                    i64::try_from(time::millis().saturating_sub(ms_at_boot) / 1000).ok()?;
                u32::try_from(utc_at_boot + elapsed).ok()
            }
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Reference::Rtc => "the RTC",
            Reference::BootClock { .. } => "the boot clock",
        }
    }
}

pub struct Keeper {
    reference: Reference,
    /// The clock as last seen or saved; `None` when the reference can't be read.
    last: Option<Saved>,
    /// When the clock was last checked (ms since boot).
    checked_ms: u64,
}

impl Keeper {
    /// Set Symbian's clock from the reference and the saved offset, if there is one.
    pub fn boot() -> Self {
        let reference = Reference::detect();
        let Some(now) = reference.seconds() else {
            dprintln!(
                "neoui: clock: {} can't be read: the time is not kept",
                reference.name()
            );
            return Self {
                reference,
                last: None,
                checked_ms: 0,
            };
        };
        match load().map(|s| (s, s.utc_at(now))) {
            Some((s, Some(utc))) => {
                let r = time::set_utc_and_offset(utc, s.tz);
                dprintln!(
                    "neoui: clock: from {} ({}) -> {} UTC, offset {} s ({:?})",
                    reference.name(),
                    now,
                    utc,
                    s.tz,
                    r
                );
            }
            Some((_, None)) => dprintln!(
                "neoui: clock: {} went back ({}): the time is not set",
                reference.name(),
                now
            ),
            None => dprintln!("neoui: clock: nothing saved, keeping {}", reference.name()),
        }
        let (utc, tz) = time::utc_now();
        // the baseline: nothing is saved until the time is set (so the ROM's build time never is)
        Self {
            reference,
            last: Some(Saved::of(utc, tz, now)),
            checked_ms: time::millis(),
        }
    }

    /// Checks the clock every [`CHECK_MS`] (call once per frame).
    pub fn tick(&mut self, now_ms: u64) {
        if now_ms.saturating_sub(self.checked_ms) >= CHECK_MS {
            self.checked_ms = now_ms;
            self.check();
        }
    }

    /// Save the clock if someone set it since the last look.
    pub fn check(&mut self) {
        let (Some(last), Some(now)) = (self.last, self.reference.seconds()) else {
            return;
        };
        let (utc, tz) = time::utc_now();
        if let Some(new) = last.changed(utc, tz, now) {
            self.last = Some(new);
            let r = store(&new);
            dprintln!(
                "neoui: clock: set to {} UTC, offset {} s: saved ({:?})",
                utc,
                tz,
                r
            );
        }
    }
}

fn load() -> Option<Saved> {
    let fs = Fs::connect().ok()?;
    let f = File::open(&fs, clock::PATH, mode::READ | mode::SHARE_READERS_ONLY).ok()?;
    let mut b = [0u8; clock::LEN];
    let n = f.read_at(0, &mut b).ok()?;
    Saved::decode(&b[..n])
}

fn store(s: &Saved) -> Result<(), i32> {
    let fs = Fs::connect()?;
    for dir in ["C:\\private\\", clock::DIR] {
        match fs.mkdir(dir) {
            Err(e) if e != err::KERR_ALREADY_EXISTS => return Err(e),
            _ => {}
        }
    }
    File::replace(&fs, clock::PATH, mode::SHARE_EXCLUSIVE)?.write(&s.encode())
}

/// Restart (0), restart to fastboot (1) or power off (2): keep the clock, mark the drives clean, ask the kernel.
/// Returns only when the kernel refused.
pub fn power(action: i32, keeper: &mut Keeper) -> i32 {
    dprintln!("neoui: power {}", action);
    keeper.check();
    let r = Fs::connect().and_then(|fs| fs.finalise_drives());
    dprintln!("neoui: drives finalised ({:?})", r);
    let r = e32::sys::ui::neo_power(action);
    dprintln!("neoui: power {} failed ({})", action, r);
    r
}

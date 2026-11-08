// SPDX-License-Identifier: GPL-3.0-only
//! The clock across restarts and power-offs, and the way out (NeoSymbian H3a).
//!
//! At boot the server sets Symbian's clock from the board RTC plus the saved offset ([`neoui_proto::clock`]). Once
//! a minute, and before a power-off or restart, it compares the clock against what it last saw: a change means
//! someone set the time (the Clock app, the debug monitor's `date`), and it saves the new offset.

use e32::fs::{File, Fs, mode};
use e32::sys::err;
use e32::{dprintln, time};
use neoui_proto::clock::{self, Saved};

pub struct Keeper {
    /// The clock as last seen or saved; `None` without an RTC (QEMU, Syborg).
    last: Option<Saved>,
}

impl Keeper {
    /// Set Symbian's clock from the RTC and the saved offset, if there is one.
    pub fn boot() -> Self {
        let Some(rtc) = time::rtc_seconds() else {
            dprintln!("neoui: clock: no RTC");
            return Self { last: None };
        };
        match load().map(|s| (s, s.utc_at(rtc))) {
            Some((s, Some(utc))) => {
                let r = time::set_utc_and_offset(utc, s.tz);
                dprintln!(
                    "neoui: clock: from the RTC ({}) -> {} UTC, offset {} s ({:?})",
                    rtc,
                    utc,
                    s.tz,
                    r
                );
            }
            Some((_, None)) => dprintln!(
                "neoui: clock: the RTC was reset ({}): the time is not set",
                rtc
            ),
            None => dprintln!("neoui: clock: nothing saved: the time is not set"),
        }
        let (utc, tz) = time::utc_now();
        // the baseline: nothing is saved until the time is set (so the ROM's build time never is)
        Self {
            last: Some(Saved::of(utc, tz, rtc)),
        }
    }

    /// Save the clock if someone set it since the last look.
    pub fn check(&mut self) {
        let (Some(last), Some(rtc)) = (self.last, time::rtc_seconds()) else {
            return;
        };
        let (utc, tz) = time::utc_now();
        if let Some(new) = last.changed(utc, tz, rtc) {
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

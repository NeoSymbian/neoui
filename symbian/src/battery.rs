// SPDX-License-Identifier: GPL-3.0-only
//! Five-second observations and private, bounded battery-history snapshots; the charge cycle's recorder (D5:
//! `neoui_proto::battery::cycle`, INFO 9) with its own alternating checkpoint.
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use e32::{
    dprintln,
    fs::{File, Fs, mode},
    sys::err,
    time,
};
use neoui_proto::battery::cycle::{Checkpoint, Recorder, Sample, widen};
use neoui_proto::battery::{FILE_MAX, History};
use neoui_proto::settings_app::wire::BatteryCycle;
use neoui_render::Renderer;
const PATHS: [&str; 2] = [
    "C:\\private\\e0002001\\battery-a.dat",
    "C:\\private\\e0002001\\battery-b.dat",
];
// D5
const CYCLE_PATHS: [&str; 2] = [
    "C:\\private\\e0002001\\cycle-a.dat",
    "C:\\private\\e0002001\\cycle-b.dat",
];

/// The tick counter widened past its 32-bit wrap (`time::ticks` wraps after 49.7 days at 1 ms).
pub fn ticks() -> u64 {
    static WIDE: AtomicU64 = AtomicU64::new(0);
    let t = widen(WIDE.load(Ordering::Relaxed), time::ticks());
    WIDE.store(t, Ordering::Relaxed);
    t
}

/// Milliseconds since boot from [`ticks`]: no wrap (`time::millis` wraps with the counter).
pub fn mono_ms() -> u64 {
    let us = match time::tick_period_us() {
        0 => 1000,
        us => u64::from(us),
    };
    ticks() * us / 1000
}

fn private_dir(fs: &Fs) -> Result<(), i32> {
    for dir in ["C:\\private\\", "C:\\private\\e0002001\\"] {
        match fs.mkdir(dir) {
            Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub struct Battery {
    history: History,
    next: u64,
    save_at: u64,
    storage_error: bool,
    previous: Option<(i64, u64)>,
    last_state: Option<(u32, u32, u32, u32)>,
    // D5: the charge cycle, and whether its checkpoint can't be read or saved
    cycle: Recorder,
    cycle_error: bool,
}
impl Battery {
    pub fn boot() -> Self {
        let mut slots = [None, None];
        let mut storage_error = false;
        match Fs::connect() {
            Ok(fs) => {
                for (index, path) in PATHS.iter().enumerate() {
                    let read = || -> Result<History, i32> {
                        let f = File::open(&fs, path, mode::READ | mode::SHARE_READERS_ONLY)?;
                        let size = usize::try_from(f.size()?).map_err(|_| -20)?;
                        if size > FILE_MAX {
                            return Err(-20);
                        }
                        let mut b = vec![0; size];
                        let n = f.read_at(0, &mut b)?;
                        History::decode(&b[..n]).ok_or(-20)
                    };
                    match read() {
                        Ok(history) => slots[index] = Some(history),
                        Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => {}
                        Err(_) => storage_error = true,
                    }
                }
            }
            Err(_) => storage_error = true,
        }
        let [a, b] = slots;
        let history = History::restore(a, b);
        dprintln!(
            "battery: history restored gen={} points={} storage_error={}",
            history.generation,
            history.points().len(),
            storage_error
        );
        let (saved, cycle_error) = read_cycle();
        let now = mono_ms();
        let cycle = Recorder::new(saved, now, super::display_state());
        dprintln!(
            "battery: cycle restored gen={} window={} error={}",
            cycle.generation,
            cycle.cycle.window,
            cycle_error
        );
        Self {
            history,
            next: 0,
            save_at: now + 300_000,
            storage_error,
            previous: None,
            last_state: None,
            cycle,
            cycle_error,
        }
    }
    /// Every loop; a new observation (every 5 s) returns true.
    pub fn poll(&mut self, ui: &mut Renderer, now: u64) -> bool {
        if now < self.next {
            return false;
        }
        self.next = now.saturating_add(5000);
        let sample = neoapp::hal::battery().unwrap_or_default();
        let state = (
            sample.valid,
            if sample.valid & 8 != 0 {
                sample.usb_present
            } else {
                0
            },
            if sample.valid & 16 != 0 {
                sample.usb_online
            } else {
                0
            },
            if sample.valid & 32 != 0 {
                sample.charge
            } else {
                0
            },
        );
        if self.last_state != Some(state) {
            dprintln!(
                "battery: valid={:#x} usb={}/{} charge={}",
                state.0,
                state.1,
                state.2,
                state.3
            );
            self.last_state = Some(state);
        }
        let observed = (time::utc_now().0, now);
        if self
            .previous
            .is_some_and(|previous| neoui_proto::battery::clock_discontinuity(previous, observed))
        {
            self.history.begin_segment();
        }
        self.previous = Some(observed);
        self.history.observe(observed.0, sample);
        let moved = self.cycle.sample(Sample {
            mono_ms: now,
            utc: observed.0,
            rtc: time::rtc_seconds(),
            battery: sample,
            display: super::display_state(),
        });
        if moved {
            dprintln!("battery: cycle starts (full charge, then discharge)");
            self.save_cycle();
        }
        if now >= self.save_at {
            self.save();
            self.save_at = now.saturating_add(300_000);
        }
        ui.set_battery(&sample);
        true
    }

    /// The 24-hour chart and whether the history saves, for the Settings app (`SETTINGS_INFO` `INFO_BATTERY`).
    pub fn info(&self) -> Vec<u8> {
        let now = self.history.points().back().map_or(0, |p| p.utc);
        neoui_proto::settings_app::info::Battery {
            save_error: self.storage_error,
            bars: self.history.bars(now),
        }
        .encode()
    }
    /// Battery usage for the Settings app (`SETTINGS_INFO` `INFO_BATTERY_USAGE`, SE3a).
    pub fn usage(&self) -> Vec<u8> {
        neoui_proto::settings_app::info::encode_usage(self.history.usage().as_ref())
    }
    pub fn save(&mut self) {
        self.save_cycle();
        let result = || -> Result<u64, i32> {
            let fs = Fs::connect()?;
            private_dir(&fs)?;
            let generation = self.history.generation.checked_add(1).ok_or(-22)?;
            let bytes = self.history.encode(generation);
            File::replace(&fs, PATHS[(generation & 1) as usize], mode::SHARE_EXCLUSIVE)?
                .write(&bytes)?;
            Ok(generation)
        }();
        match result {
            Ok(generation) => {
                self.history.generation = generation;
                self.storage_error = false;
                dprintln!(
                    "battery: history saved gen={} points={}",
                    generation,
                    self.history.points().len()
                );
            }
            Err(e) => {
                self.storage_error = true;
                dprintln!("battery: history save failed ({})", e);
            }
        }
    }
}

// D5
impl Battery {
    /// The screen was set on or off: the display HAL now says `state` (`None`: its read failed).
    pub fn display(&mut self, state: Option<bool>) {
        self.cycle.display(mono_ms(), state);
    }

    /// INFO 9's record (`None` before the first valid reading).
    pub fn cycle(&self) -> Option<BatteryCycle> {
        self.cycle.record(self.cycle_error)
    }

    /// The next checkpoint into the other slot: written, flushed and closed before its generation counts.
    fn save_cycle(&mut self) {
        let result = || -> Result<u64, i32> {
            let fs = Fs::connect()?;
            private_dir(&fs)?;
            let generation = self.cycle.generation.checked_add(1).ok_or(-22)?;
            let bytes = Checkpoint {
                generation,
                cycle: self.cycle.cycle,
                rtc: time::rtc_seconds(),
            }
            .encode();
            let f = File::replace(
                &fs,
                CYCLE_PATHS[(generation & 1) as usize],
                mode::SHARE_EXCLUSIVE,
            )?;
            f.write(&bytes)?;
            f.flush()?;
            Ok(generation)
        }();
        match result {
            Ok(generation) => {
                self.cycle.generation = generation;
                self.cycle_error = false;
            }
            Err(e) => {
                self.cycle_error = true;
                dprintln!("battery: cycle save failed ({})", e);
            }
        }
    }
}

/// The newest valid checkpoint, and whether a slot couldn't be read (twice). A missing or damaged slot (a torn write)
/// isn't an error: the other one is used.
fn read_cycle() -> (Option<Checkpoint>, bool) {
    let Ok(fs) = Fs::connect() else {
        return (None, true);
    };
    let mut error = false;
    let mut slot = |path: &str| {
        let read = || -> Result<Vec<u8>, i32> {
            let f = File::open(&fs, path, mode::READ | mode::SHARE_READERS_ONLY)?;
            let mut b = vec![0; Checkpoint::LEN + 1];
            let n = f.read_at(0, &mut b)?;
            b.truncate(n);
            Ok(b)
        };
        match read().or_else(|_| read()) {
            Ok(b) => Checkpoint::decode(&b),
            Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => None,
            Err(e) => {
                dprintln!("battery: {} unreadable ({})", path, e);
                error = true;
                None
            }
        }
    };
    let (a, b) = (slot(CYCLE_PATHS[0]), slot(CYCLE_PATHS[1]));
    (Checkpoint::newest(a, b), error)
}

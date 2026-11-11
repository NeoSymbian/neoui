// SPDX-License-Identifier: GPL-3.0-only
//! Five-second observations and private, bounded battery-history snapshots.
use alloc::vec;
use e32::{
    dprintln,
    fs::{File, Fs, mode},
    sys::err,
    time,
};
use neoui_proto::battery::{FILE_MAX, History};
use neoui_render::Renderer;
const PATHS: [&str; 2] = [
    "C:\\private\\e0002001\\battery-a.dat",
    "C:\\private\\e0002001\\battery-b.dat",
];
pub struct Battery {
    history: History,
    next: u64,
    save_at: u64,
    storage_error: bool,
    previous: Option<(i64, u64)>,
    last_state: Option<(u32, u32, u32, u32)>,
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
        Self {
            history,
            next: 0,
            save_at: time::millis() + 300_000,
            storage_error,
            previous: None,
            last_state: None,
        }
    }
    pub fn poll(&mut self, ui: &mut Renderer, now: u64) {
        if now < self.next {
            return;
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
        if now >= self.save_at {
            self.save();
            self.save_at = now.saturating_add(300_000);
        }
        ui.set_battery(&sample, &self.history, self.storage_error);
    }
    pub fn save(&mut self) {
        let result = || -> Result<u64, i32> {
            let fs = Fs::connect()?;
            for dir in ["C:\\private\\", "C:\\private\\e0002001\\"] {
                match fs.mkdir(dir) {
                    Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
                    Err(e) => return Err(e),
                }
            }
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

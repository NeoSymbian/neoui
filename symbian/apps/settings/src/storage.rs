// SPDX-License-Identifier: GPL-3.0-only
//! Settings → Storage (SE3b): the drives from the file server at once, and C: and E: by kind from a scan on a
//! low-priority thread of its own (`neoui_proto::settings_app::storage::scan`: it lists folders, never opens or
//! writes a file). The scan runs only while the page is open and the app shown; its last whole result is kept with
//! the time it finished and the drives' totals then, and Recount runs it again.
//!
//! Every change to the scan's state happens under one lock, and a scan only writes while its generation is the
//! current one: stopping or starting another moves the generation on, so a superseded scan can't land over a newer
//! one, and stopping clears "Counting…" at once.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::Fs;
use e32::sync::Mutex;
use neoui_proto::settings_app::storage::{self as st, KINDS, Tally};
use settings_ui::{DriveRow, PartRow, SetState, SettingsApp};
use slint::{ComponentHandle, ModelRc, VecModel};

/// `EPriorityMuchLess` (`TThreadPriority`): below the UI thread, so a long walk never slows a frame.
const PRIORITY_MUCH_LESS: i32 = -20;
/// `KCurrentThreadHandle`.
const CURRENT_THREAD: u32 = 0xFFFF_8001;

#[derive(Default)]
struct Shared {
    /// The scan allowed to write: bumped by every start and stop.
    generation: u32,
    /// A scan is running (the current generation's).
    running: bool,
    /// Its running count.
    tally: Tally,
    /// The last whole count, with C: and E:'s (used, free) when it was made (kept through a stopped or a new scan).
    last: Tally,
    last_totals: (u64, u64),
    /// When the last whole scan ended (local "HH:MM"); `None` before the first.
    at: Option<String>,
    /// Why the last scan couldn't count (a file server error), shown until the next one.
    error: Option<i32>,
}

pub struct Storage {
    shared: Option<&'static Mutex<Shared>>,
    /// C: and E: together (used, free), from the drive rows.
    totals: core::cell::Cell<(u64, u64)>,
    /// The (letter, free) pairs last logged: the free bytes go to the log when they change (a phone run's proof that
    /// nothing was written).
    logged: core::cell::RefCell<Vec<(char, i64)>>,
}

impl Storage {
    pub fn new() -> Self {
        Storage {
            shared: Mutex::new(Shared::default())
                .ok()
                .map(|m| &*Box::leak(Box::new(m))),
            totals: core::cell::Cell::new((0, 0)),
            logged: core::cell::RefCell::new(Vec::new()),
        }
    }

    /// The drive rows and the root's "N free" (E:'s free space, else C:'s); emptied when the file server can't say.
    pub fn drives(&self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let Ok(list) = Fs::connect().and_then(|fs| fs.drives().map(|l| (fs, l))) else {
            s.set_drives(ModelRc::new(VecModel::from(alloc::vec![row(
                "Drives".into(),
                "Not available".into(),
                "",
                -1.0
            )])));
            s.set_storage_free(String::new().into());
            return;
        };
        let (fs, list) = list;
        let mut rows = Vec::new();
        let mut logged = Vec::new();
        let (mut used_ce, mut free_ce, mut root_free) = (0u64, 0u64, None);
        // E: first (the board's order: the big drive), then the rest from C:
        let order = core::iter::once(4).chain((0..26).filter(|&i| i != 4));
        for i in order.filter(|&i| list[i] != 0) {
            let letter = char::from(b'A' + i as u8);
            let (name, note) = st::drive_names(letter);
            let title = format!("{letter}: {name}");
            let Ok(v) = fs.volume(i as i32) else {
                rows.push(row(title, "Not available".into(), note, -1.0));
                continue;
            };
            logged.push((letter, v.free));
            let (size, free) = (v.size.max(0) as u64, v.free.clamp(0, v.size.max(0)) as u64);
            let used = size - free;
            // the ROM: its size (nothing is free on a read-only drive); the rest: used of total
            // a composite ROM file system can report no size at all: then only "Read-only"
            if letter == 'Z' && size == 0 {
                rows.push(row(title, note.into(), "", -1.0));
                continue;
            }
            let (shown, fill) = if letter == 'Z' {
                (st::size_text(size), -1.0)
            } else {
                (
                    st::used_text(used, size),
                    if size > 0 {
                        used as f32 / size as f32
                    } else {
                        -1.0
                    },
                )
            };
            rows.push(row(title, shown, note, fill));
            if matches!(letter, 'C' | 'E') {
                used_ce += used;
                free_ce += free;
                if letter == 'E' || root_free.is_none() {
                    root_free = Some(free);
                }
            }
        }
        if *self.logged.borrow() != logged {
            for (letter, free) in &logged {
                dprintln!("settings: drive {}: free {} bytes", letter, free);
            }
            *self.logged.borrow_mut() = logged;
        }
        self.totals.set((used_ce, free_ce));
        s.set_drives(ModelRc::new(VecModel::from(rows)));
        s.set_storage_free(
            root_free
                .map_or_else(String::new, |f| format!("{} free", st::size_text(f)))
                .into(),
        );
    }

    /// The page opened, shown: count once if nothing is counted yet (a kept result isn't redone).
    pub fn ensure(&self) {
        let Some(shared) = self.shared else { return };
        let idle = {
            let g = shared.lock();
            !g.running && g.at.is_none()
        };
        if idle {
            self.start();
        }
    }

    /// Recount: stop what runs and count again.
    pub fn recount(&self) {
        self.start();
    }

    /// Leave the page or hide the app: the scan stops at its next folder, and the page stops saying "Counting…" now.
    pub fn stop(&self) {
        let Some(shared) = self.shared else { return };
        let mut g = shared.lock();
        g.generation = g.generation.wrapping_add(1);
        g.running = false;
    }

    fn start(&self) {
        let Some(shared) = self.shared else { return };
        let totals = self.totals.get();
        let mine = {
            let mut g = shared.lock();
            g.generation = g.generation.wrapping_add(1);
            g.running = true;
            g.tally = Tally::default();
            g.error = None;
            g.generation
        };
        // every write below is the current generation's only
        let finish = move |f: &mut dyn FnMut(&mut Shared)| {
            let mut g = shared.lock();
            if g.generation == mine {
                f(&mut g);
            }
        };
        // shortcut: the JoinHandle is dropped, which leaks e32's small boxed logon status per scan (e32 keeps it
        // alive by design); a Recount now and then costs a few bytes until e32 frees it on drop.
        let spawned = e32::thread::spawn(move || {
            // SAFETY: ThreadSetPriority on the current thread's pseudo-handle with a valid TThreadPriority.
            let p = unsafe {
                e32::sys::swi::exec4::<{ e32::sys::execs::slow::THREAD_SET_PRIORITY }>(
                    CURRENT_THREAD,
                    PRIORITY_MUCH_LESS as u32,
                    0,
                    0,
                )
            };
            let started = e32::time::millis();
            // C:, and E: when the file server lists it; a file server that can't answer is an error, not a count
            let fs = Fs::connect().and_then(|fs| fs.drives().map(|l| (fs, l)));
            let (fs, list) = match fs {
                Ok(v) => v,
                Err(e) => {
                    dprintln!("settings: storage scan can't reach the file server ({})", e);
                    finish(&mut |g| {
                        g.running = false;
                        g.error = Some(e);
                    });
                    return 0;
                }
            };
            let roots: Vec<&str> = if list[4] != 0 {
                alloc::vec!["C:\\", "E:\\"]
            } else {
                alloc::vec!["C:\\"]
            };
            let t = st::scan(
                &roots,
                |dir| {
                    fs.read_dir(dir).map(|v| {
                        v.into_iter()
                            .map(|e| (e.name.clone(), e.is_dir(), e.size))
                            .collect()
                    })
                },
                || shared.lock().generation != mine,
                |t| finish(&mut |g| g.tally = *t),
            );
            let ms = e32::time::millis() - started;
            dprintln!(
                "settings: storage scan {} in {} ms: {} folders, {} unreadable, {} skipped, files {} apps {} (priority {})",
                if t.done { "done" } else { "stopped" },
                ms,
                t.folders,
                t.unreadable,
                t.skipped,
                t.files_total(),
                t.apps,
                p as i32
            );
            finish(&mut |g| {
                g.running = false;
                if t.done {
                    let now = e32::time::now();
                    g.at = Some(format!("{:02}:{:02}", now.hour, now.minute));
                    g.last = t;
                    g.last_totals = totals;
                }
            });
            0
        });
        if let Err(e) = spawned {
            dprintln!("settings: storage scan didn't start ({})", e);
            let mut g = shared.lock();
            if g.generation == mine {
                g.running = false;
                g.error = Some(e);
            }
        }
    }

    /// The scan's state on the page: the bar, its parts, the kinds, "Counting…" or "Last counted HH:MM".
    pub fn show(&self, ui: &SettingsApp) {
        let Some(shared) = self.shared else { return };
        let s = ui.global::<SetState>();
        let (t, totals, running, at, error) = {
            let g = shared.lock();
            // while counting, the running totals against the drives now; else the last whole count with its own
            if g.running {
                (g.tally, self.totals.get(), true, g.at.clone(), g.error)
            } else {
                (g.last, g.last_totals, false, g.at.clone(), g.error)
            }
        };
        let (used, free) = totals;
        let other = st::other(used, &t);
        let total = (t.apps + t.files_total() + other + free).max(1) as f32;
        let mut start = 0.0;
        let mut parts = Vec::new();
        for (name, bytes) in [
            ("Apps", t.apps),
            ("Files", t.files_total()),
            ("Other (system and app data)", other),
            ("Free", free),
        ] {
            let share = bytes as f32 / total;
            parts.push(PartRow {
                name: name.into(),
                size: st::size_text(bytes).into(),
                start,
                share,
            });
            start += share;
        }
        let kinds: Vec<PartRow> = KINDS
            .iter()
            .map(|&k| PartRow {
                name: k.name().into(),
                size: st::size_text(t.files[k as usize]).into(),
                start: 0.0,
                share: 0.0,
            })
            .collect();
        s.set_storage_parts(ModelRc::new(VecModel::from(parts)));
        s.set_storage_kinds(ModelRc::new(VecModel::from(kinds)));
        s.set_storage_counting(running);
        s.set_storage_counted(at.is_some() && !running);
        s.set_storage_status(
            if running {
                format!("Counting… {} folders", t.folders)
            } else if let Some(e) = error {
                format!("Couldn't count ({e}); Recount tries again")
            } else if let Some(at) = &at {
                format!("Last counted {at}")
            } else {
                String::from("Not counted yet")
            }
            .into(),
        );
        s.set_storage_unreadable(
            match (t.unreadable, t.skipped) {
                (0, 0) => String::new(),
                (1, 0) => String::from("1 folder couldn't be read"),
                (n, 0) => format!("{n} folders couldn't be read"),
                (n, k) => format!(
                    "{n} folders couldn't be read, {k} weren't entered (too many or too deep)"
                ),
            }
            .into(),
        );
    }
}

fn row(title: String, used: String, note: &str, fill: f32) -> DriveRow {
    DriveRow {
        title: title.into(),
        used: used.into(),
        note: note.into(),
        fill,
    }
}

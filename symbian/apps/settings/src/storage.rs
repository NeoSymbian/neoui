// SPDX-License-Identifier: GPL-3.0-only
//! Settings → Storage (SE3b): the drives from the file server at once, and C: and E: by kind from a scan on a
//! low-priority thread of its own (`neoui_proto::settings_app::storage::scan`: it lists folders, never opens or
//! writes a file). The scan runs only while the page is open and the app shown; its last whole result is kept with
//! the time it finished and the drives' totals then, and Recount runs it again.
//!
//! D3: C: and E: are counted one after the other, each with its own tally, the volume's used/free and its unique ID
//! taken when its count starts; a drive remounted (or gone) by the end of its count drops the whole count as partial.
//! C:'s row draws its own Apps/Files/Other from its last count, the notes name the mounted file system, and the root
//! summary is C:'s free space.
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

/// The drives counted, by (letter, drive number): C: and E:, each with its own tally.
const COUNTED: [(char, usize); 2] = [('C', 2), ('E', 4)];

/// A counted drive's volume: zero when it isn't there.
#[derive(Clone, Copy, Default)]
struct Vol {
    used: u64,
    free: u64,
    size: u64,
}

impl Vol {
    fn of(v: &e32::fs::Volume) -> Self {
        let size = v.size.max(0) as u64;
        let free = v.free.clamp(0, v.size.max(0)) as u64;
        Vol {
            used: size - free,
            free,
            size,
        }
    }
}

#[derive(Default)]
struct Shared {
    /// The scan allowed to write: bumped by every start and stop.
    generation: u32,
    /// A scan is running (the current generation's).
    running: bool,
    /// Its running counts, by [`COUNTED`].
    tally: [Tally; 2],
    /// The last whole count of each drive, with its volume when its count started (kept through a stopped or a new
    /// scan).
    last: [Tally; 2],
    last_vols: [Vol; 2],
    /// When the last whole scan ended (local "HH:MM"); `None` before the first.
    at: Option<String>,
    /// Why the last scan couldn't count (a file server error), shown until the next one.
    error: Option<i32>,
    /// The drive remounted or gone while it was counted: the count was partial and dropped.
    changed: Option<char>,
}

pub struct Storage {
    shared: Option<&'static Mutex<Shared>>,
    /// C: and E:'s volumes now, from the drive rows.
    vols: core::cell::Cell<[Vol; 2]>,
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
            vols: core::cell::Cell::new([Vol::default(); 2]),
            logged: core::cell::RefCell::new(Vec::new()),
        }
    }

    /// The drive rows and the root's "C: N free"; emptied when the file server can't say.
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
        let mut vols = [Vol::default(); 2];
        // E: first (the board's order: the big drive), then the rest from C:
        let order = core::iter::once(4).chain((0..26).filter(|&i| i != 4));
        for i in order.filter(|&i| list[i] != 0) {
            let letter = char::from(b'A' + i as u8);
            let (name, plain) = st::drive_names(letter);
            let title = format!("{letter}: {name}");
            let Ok(v) = fs.volume(i as i32) else {
                rows.push(row(title, "Not available".into(), plain, -1.0));
                continue;
            };
            logged.push((letter, v.free));
            // the mounted file system (FAT32, or Rom/Rofs/Composite on Z:): unknown leaves the plain note
            let fs_name = fs.file_system_sub_type(i as i32).unwrap_or_default();
            let note = st::drive_note(letter, &fs_name);
            let vol = Vol::of(&v);
            let (size, used) = (vol.size, vol.used);
            // the ROM: its size (nothing is free on a read-only drive); the rest: used of total
            // a composite ROM file system can report no size at all: then only "Read-only"
            if letter == 'Z' && size == 0 {
                rows.push(row(title, plain.into(), &note, -1.0));
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
            rows.push(row(title, shown, &note, fill));
            if let Some(k) = COUNTED.iter().position(|&(_, n)| n == i) {
                vols[k] = vol;
            }
        }
        if *self.logged.borrow() != logged {
            for (letter, free) in &logged {
                dprintln!("settings: drive {}: free {} bytes", letter, free);
            }
            *self.logged.borrow_mut() = logged;
        }
        self.vols.set(vols);
        s.set_drives(ModelRc::new(VecModel::from(rows)));
        // C: explicitly (the board's "C: 3.1 GB free"): the drive the phone runs out of
        s.set_storage_free(
            if list[2] != 0 && vols[0].size > 0 {
                format!("C: {} free", st::size_text(vols[0].free))
            } else {
                String::new()
            }
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
        let mine = {
            let mut g = shared.lock();
            g.generation = g.generation.wrapping_add(1);
            g.running = true;
            g.tally = [Tally::default(); 2];
            g.error = None;
            g.changed = None;
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
            // each drive on its own: its volume first, its count, then the volume again (a remount drops it all)
            let (mut ts, mut vols) = ([Tally::default(); 2], [Vol::default(); 2]);
            let mut changed = None;
            for (k, &(letter, n)) in COUNTED.iter().enumerate() {
                if list[n] == 0 {
                    ts[k].done = true; // absent (no E:): nothing there, counted
                    continue;
                }
                let Ok(before) = fs.volume(n as i32) else {
                    changed = Some(letter);
                    break;
                };
                vols[k] = Vol::of(&before);
                let root = format!("{letter}:\\");
                ts[k] = st::scan(
                    &[root.as_str()],
                    |dir| {
                        fs.read_dir(dir).map(|v| {
                            v.into_iter()
                                .map(|e| (e.name.clone(), e.is_dir(), e.size))
                                .collect()
                        })
                    },
                    || shared.lock().generation != mine,
                    |t| finish(&mut |g| g.tally[k] = *t),
                );
                if !ts[k].done {
                    break;
                }
                if fs.volume(n as i32).map(|v| v.unique_id) != Ok(before.unique_id) {
                    dprintln!(
                        "settings: storage scan: {}: remounted while counted",
                        letter
                    );
                    ts[k].done = false;
                    changed = Some(letter);
                    break;
                }
            }
            let t = ts[0].plus(&ts[1]);
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
                g.changed = changed;
                if t.done {
                    let now = e32::time::now();
                    g.at = Some(format!("{:02}:{:02}", now.hour, now.minute));
                    g.last = ts;
                    g.last_vols = vols;
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
        let now = self.vols.get();
        let (ts, vols, running, at, error, changed, c_last) = {
            let g = shared.lock();
            // while counting, the running counts against the drives now; else the last whole count with its own
            let (ts, vols) = if g.running {
                (g.tally, now)
            } else {
                (g.last, g.last_vols)
            };
            (
                ts,
                vols,
                g.running,
                g.at.clone(),
                g.error,
                g.changed,
                g.last[0],
            )
        };
        let t = ts[0].plus(&ts[1]);
        // Other is reconciled per drive with its used bytes, then added
        let other = st::other(vols[0].used, &ts[0]) + st::other(vols[1].used, &ts[1]);
        let free = vols[0].free + vols[1].free;
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
        // C:'s row: its last whole count against its size now (none yet: the plain used fill)
        let c = now[0];
        let mut c_parts = Vec::new();
        if at.is_some() && c.size > 0 {
            let mut start = 0.0;
            for bytes in st::drive_parts(c.used, &c_last) {
                let share = bytes as f32 / c.size as f32;
                c_parts.push(PartRow {
                    name: Default::default(),
                    size: Default::default(),
                    start,
                    share,
                });
                start += share;
            }
        }
        s.set_storage_c_parts(ModelRc::new(VecModel::from(c_parts)));
        s.set_storage_kinds(ModelRc::new(VecModel::from(kinds)));
        s.set_storage_counting(running);
        s.set_storage_counted(at.is_some() && !running);
        s.set_storage_status(
            if running {
                format!("Counting… {} folders", t.folders)
            } else if let Some(e) = error {
                format!("Couldn't count ({e}); Recount tries again")
            } else if let Some(d) = changed {
                format!("{d}: changed while it was counted; Recount tries again")
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

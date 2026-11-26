// SPDX-License-Identifier: GPL-3.0-only
//! Settings → Storage (SE3b): the drives from the file server at once, and C: and E: by kind from a scan on a
//! low-priority thread of its own (`neoui_proto::settings_app::storage::scan`: it lists folders, never opens or
//! writes a file). The scan runs only while the page is open and the app shown; its last whole result is kept with
//! the time it finished and the drives' totals then, and Recount runs it again.
//!
//! D3: C: and E: are counted one after the other, each with its own tally and the volume's used/free taken when its
//! count starts, and each drive keeps its own last count: a complete one replaces it, a partial one (folders it
//! couldn't read) only stands in while the drive has none, and one that was stopped, remounted, gone or whose root
//! couldn't be read is dropped (`storage::Outcome`). A drive's mount generation guards the publication: a disk change
//! F32 reports while it is counted (ENotifyDisk: mount, dismount, media change, format), another volume ID, or the
//! drive rows seeing another volume, moves it on, and a count only lands under the generation it started with.
//! C:'s row draws its own Apps/Files/Other from its last complete count, the notes name the mounted file system, and
//! the root summary is C:'s free space.
//!
//! Every change to the scan's state happens under one lock, and a scan only writes while its generation is the
//! current one: stopping or starting another moves the generation on, so a superseded scan can't land over a newer
//! one, and stopping clears "Counting…" at once.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use e32::dprintln;
use e32::fs::{Fs, NotifyChange, notify};
use e32::sync::Mutex;
use neoui_proto::settings_app::storage::{self as st, KINDS, Outcome, Tally};
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
    /// `TVolumeInfo::iUniqueID`: another value is another medium or a format.
    id: u32,
}

impl Vol {
    fn of(v: &e32::fs::Volume) -> Self {
        let size = v.size.max(0) as u64;
        let free = v.free.clamp(0, v.size.max(0)) as u64;
        Vol {
            used: size - free,
            free,
            size,
            id: v.unique_id,
        }
    }
}

/// A drive's kept count, with its volume when the count started.
#[derive(Clone, Copy)]
struct Count {
    tally: Tally,
    vol: Vol,
    /// some folders couldn't be read ([`Outcome::Partial`])
    partial: bool,
}

#[derive(Default)]
struct Shared {
    /// The scan allowed to write: bumped by every start and stop.
    generation: u32,
    /// A scan is running (the current generation's).
    running: bool,
    /// Its running counts, by [`COUNTED`].
    tally: [Tally; 2],
    /// Each drive's kept count (kept through a stopped or a new scan); `None`: none yet, or the drive went away.
    last: [Option<Count>; 2],
    /// Each drive's mount generation (see the module's notes).
    mounts: [u32; 2],
    /// When a count was last kept (local "HH:MM"); `None` before the first.
    at: Option<String>,
    /// Why the last scan couldn't count (a file server error), shown until the next one.
    error: Option<i32>,
    /// The drives the last scan dropped, and why ("changed while it was counted").
    dropped: Vec<(char, &'static str)>,
}

pub struct Storage {
    shared: Option<&'static Mutex<Shared>>,
    /// C: and E:'s volumes now, from the drive rows.
    vols: core::cell::Cell<[Vol; 2]>,
    /// C: and E:'s volume IDs the drive rows last saw (`None` absent or unreadable); `None` before the first look.
    ids: core::cell::Cell<Option<[Option<u32>; 2]>>,
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
            ids: core::cell::Cell::new(None),
            logged: core::cell::RefCell::new(Vec::new()),
        }
    }

    /// The drive rows and the root's "C: N free"; emptied when the file server can't say.
    pub fn drives(&self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let Ok(list) = Fs::connect().and_then(|fs| fs.drives().map(|l| (fs, l))) else {
            s.set_drives(ModelRc::new(VecModel::from(alloc::vec![row(
                None,
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
        let mut ids = [None; 2];
        // E: first (the board's order: the big drive), then the rest from C:
        let order = core::iter::once(4).chain((0..26).filter(|&i| i != 4));
        for i in order.filter(|&i| list[i] != 0) {
            let letter = char::from(b'A' + i as u8);
            let (name, plain) = st::drive_names(letter);
            let title = format!("{letter}: {name}");
            let Ok(v) = fs.volume(i as i32) else {
                rows.push(row(
                    Some(letter),
                    title,
                    "Not available".into(),
                    plain,
                    -1.0,
                ));
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
                rows.push(row(Some(letter), title, plain.into(), &note, -1.0));
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
            rows.push(row(Some(letter), title, shown, &note, fill));
            if let Some(k) = COUNTED.iter().position(|&(_, n)| n == i) {
                vols[k] = vol;
                ids[k] = Some(vol.id);
            }
        }
        if *self.logged.borrow() != logged {
            for (letter, free) in &logged {
                dprintln!("settings: drive {}: free {} bytes", letter, free);
            }
            *self.logged.borrow_mut() = logged;
        }
        self.vols.set(vols);
        // another volume (or none) where the last look saw one: that drive's mount generation moves on, so a count
        // of the old one can't land
        if let (Some(old), Some(shared)) = (self.ids.replace(Some(ids)), self.shared) {
            let mut g = shared.lock();
            for k in 0..COUNTED.len() {
                if old[k] != ids[k] {
                    g.mounts[k] = g.mounts[k].wrapping_add(1);
                }
            }
        }
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
            g.dropped.clear();
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
            // each drive on its own: its mount generation and volume first, then its count, kept only if it is
            // still the same mount when it lands
            let mut t = Tally {
                done: true,
                ..Tally::default()
            };
            for (k, &(letter, n)) in COUNTED.iter().enumerate() {
                if list[n] == 0 {
                    // absent (no E:): nothing there to keep
                    finish(&mut |g| g.last[k] = None);
                    continue;
                }
                let mount = shared.lock().mounts[k];
                let Ok(before) = fs.volume(n as i32) else {
                    finish(&mut |g| g.dropped.push((letter, "couldn't be read")));
                    continue;
                };
                let root = format!("{letter}:\\");
                // F32 completes this on a mount, dismount, media change or format of the drive; armed now, polled at
                // the end and at the publication. If it can't be armed, the volume ID check below still stands.
                let mut watch = fs.notify_change(notify::DISK, &root).ok().map(Box::pin);
                // the first poll sends it: Ready there is an error, so no watch
                if watch.as_mut().is_some_and(|w| fired(w)) {
                    watch = None;
                }
                let ts = st::scan(
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
                if !ts.done {
                    t.done = false;
                    break; // stopped: nothing lands (the generation moved on)
                }
                t = t.plus(&ts);
                let remounted = fs.volume(n as i32).map(|v| v.unique_id) != Ok(before.unique_id);
                finish(&mut |g| {
                    // through the publication: a disk change seen until now moves the generation on
                    if remounted || watch.as_mut().is_some_and(|w| fired(w)) {
                        g.mounts[k] = g.mounts[k].wrapping_add(1);
                    }
                    let o = ts.outcome(g.mounts[k] != mount);
                    if st::replaces(g.last[k].map(|c| c.partial), o) {
                        g.last[k] = Some(Count {
                            tally: ts,
                            vol: Vol::of(&before),
                            partial: o == Outcome::Partial,
                        });
                        let now = e32::time::now();
                        g.at = Some(format!("{:02}:{:02}", now.hour, now.minute));
                    }
                    if o == Outcome::Dropped {
                        let why = if ts.roots_unread > 0 {
                            "couldn't be read"
                        } else {
                            "changed while it was counted"
                        };
                        dprintln!("settings: storage scan: {}: {}, not kept", letter, why);
                        g.dropped.push((letter, why));
                    }
                });
            }
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
            finish(&mut |g| g.running = false);
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
        let (ts, vols, running, at, error, dropped, kept) = {
            let g = shared.lock();
            // while counting, the running counts against the drives now; else each drive's kept count with its own
            // volume (a drive without one adds nothing: its bytes aren't put under Other)
            let (ts, vols) = if g.running {
                (g.tally, now)
            } else {
                let mut ts = [Tally::default(); 2];
                let mut vols = [Vol::default(); 2];
                for (k, c) in g.last.iter().enumerate() {
                    if let Some(c) = c {
                        (ts[k], vols[k]) = (c.tally, c.vol);
                    }
                }
                (ts, vols)
            };
            (
                ts,
                vols,
                g.running,
                g.at.clone(),
                g.error,
                g.dropped.clone(),
                g.last,
            )
        };
        let counted = kept.iter().any(Option::is_some);
        let partial: Vec<char> = COUNTED
            .iter()
            .zip(&kept)
            .filter(|(_, c)| c.is_some_and(|c| c.partial))
            .map(|(&(l, _), _)| l)
            .collect();
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
        // C:'s row: its last complete count against its size now (none, or partial: the plain used fill); the names
        // and sizes are its accessible label
        let c = now[0];
        let mut c_parts = Vec::new();
        if let Some(Count {
            tally,
            partial: false,
            ..
        }) = kept[0]
            && c.size > 0
        {
            let mut start = 0.0;
            for (name, bytes) in ["Apps", "Files", "Other"]
                .into_iter()
                .zip(st::drive_parts(c.used, &tally))
            {
                let share = bytes as f32 / c.size as f32;
                c_parts.push(PartRow {
                    name: name.into(),
                    size: st::size_text(bytes).into(),
                    start,
                    share,
                });
                start += share;
            }
        }
        s.set_storage_c_parts(ModelRc::new(VecModel::from(c_parts)));
        s.set_storage_kinds(ModelRc::new(VecModel::from(kinds)));
        s.set_storage_counting(running);
        s.set_storage_counted(counted && !running);
        s.set_storage_partial(!partial.is_empty());
        s.set_storage_status(
            if running {
                format!("Counting… {} folders", t.folders)
            } else if let Some(e) = error {
                format!("Couldn't count ({e}); Recount tries again")
            } else if !dropped.is_empty() {
                let mut text = String::new();
                for (d, why) in &dropped {
                    text += &format!("{d}: {why}; ");
                }
                text + "Recount tries again"
            } else if let (true, Some(at)) = (counted, &at) {
                let mut text = format!("Last counted {at}");
                for d in &partial {
                    text += &format!(" · {d}: partial");
                }
                text
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

/// The watch saw a disk change (or failed): polled without waiting, a no-op waker (nothing awaits it).
fn fired(w: &mut Pin<Box<NotifyChange<'_>>>) -> bool {
    matches!(
        w.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(_)
    )
}

fn row(letter: Option<char>, title: String, used: String, note: &str, fill: f32) -> DriveRow {
    DriveRow {
        letter: letter.map_or_else(String::new, String::from).into(),
        title: title.into(),
        used: used.into(),
        note: note.into(),
        fill,
    }
}

// SPDX-License-Identifier: GPL-3.0-only
//! Protocol 1.20's Settings snapshots and owner work in SYSSTART (docs neoui-shell/protocol.md "Settings snapshots,
//! owner reports and panel policy"): per-kind revisions and the Settings session's subscriptions (event 27), the paged
//! `SETTINGS_INFO` kinds 8, 9, 13 and 14, `ACTION_MEASURE_APP` and `ACTION_CLEAR`'s confirmed form with their nonces,
//! `OWNER_REPORT` over [`neoui_proto::settings_app::jobs`] (saved to [`JOBS_PATH`] before a wake and before an
//! acknowledgement), and `ENTRY_THUMB`. A child of `host`: it works on the host's apps.
//!
//! A change a request makes to the jobs table stands only once it is saved: otherwise the table goes back and the
//! request fails (no ticket, no claim, no acknowledged report). INFO 13 shows the table as last saved. A change
//! nobody asked for (an owner gone, a job expired) is kept and saved again every second until it is.
//!
//! D4 produces INFO 8 (`app_details`): per listed app its catalogue name and SID, the EXE the shell finds on C: or Z:
//! (its installed bytes), the latest measurement (an owner's report, or the shell's own for an app that isn't one:
//! `walks`: the request is answered with its ticket at once, and a worker thread counts the folder; the result stands
//! like an owner's report once the table is saved) and the clear scope an owner offers
//! (`apps::owner_scope`, once it registered with the clear capability). A version is only ever an owner's report.

use super::{App, Host, read_bytes};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use e32::server::Message;
use e32::sync::Mutex;
use e32::sys::err;
use neoui_proto::apps::{self, LEGACY, SETTINGS};
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::settings::{Caller, access, kerr};
use neoui_proto::settings_app::app_detail::with_installed;
use neoui_proto::settings_app::jobs::Jobs;
use neoui_proto::settings_app::permissions;
use neoui_proto::settings_app::wire::{
    self, AppDetail, AppRef, ClearRequest, Dedup, DriveUsage, Features, MeasureRequest,
    OwnerReport, Revisions, capability, coverage, state,
};

/// The owner-work table across restarts, and the file a save writes first (then replaces the table with it).
const JOBS_PATH: &str = "C:\\private\\e0002001\\jobs.dat";
const JOBS_STAGING: &str = "C:\\private\\e0002001\\jobs.tmp";
/// The most a saved table can be (16 unfinished + 32 receipts).
const JOBS_FILE_MAX: usize = 7 + 48 * (2 + wire::Job::MAX);

/// The host's 1.20 state.
pub struct Shelf {
    revs: Revisions,
    jobs: Jobs,
    dedup: Dedup,
    /// The Settings session and the paged kinds (bit `n`) it read since it connected: event 27 goes there.
    watch: Option<(u32, u32)>,
    /// The old apps' thumbnails by entry id, until the bridge's entries change.
    thumbs: Vec<(u32, Vec<u8>)>,
    /// The table changed and isn't saved yet: no owner is woken until it is.
    dirty: bool,
    /// The saved table was read (or there is none): until then nothing changes it, and nothing overwrites it.
    loaded: bool,
    /// INFO 13 as last saved.
    published: Vec<Vec<u8>>,
    tick_at: u64,
    /// D5: INFO 9's record as last published (`None`: no valid battery reading yet).
    cycle: Option<wire::BatteryCycle>,
    /// D4: each listed app's EXE as found (drive, bytes), read at boot and again when the shell measures the app.
    found: Vec<(&'static str, Option<(u8, u64)>)>,
    /// D4: the shell's own measurement counting now, on its worker thread.
    walk: Option<Walk>,
    /// (D8) each app's capabilities as the kernel held them when its session last bound (`caps::of_process`)
    caps: Vec<(&'static str, u64)>,
}

/// What a walk finds: the app's EXE (drive, bytes) and the bytes in its private folder.
type Found = (Option<(u8, u64)>, Result<u64, i32>);

/// One measurement on its worker: the job, the thread, and where it puts what it found.
struct Walk {
    key: &'static str,
    ticket: u32,
    thread: e32::thread::JoinHandle,
    out: Arc<Mutex<Option<Found>>>,
}

fn unix_s() -> i64 {
    e32::time::utc_now().0
}

impl Shelf {
    /// The saved table, every unfinished job in it interrupted; read again every second if that failed.
    pub fn boot() -> Self {
        let mut s = Self {
            revs: Revisions::default(),
            jobs: Jobs::default(),
            dedup: Dedup::default(),
            watch: None,
            thumbs: Vec::new(),
            dirty: false,
            loaded: false,
            published: Vec::new(),
            tick_at: 0,
            cycle: None,
            found: apps::listed().map(|a| (a.key, find_exe(a.exe))).collect(),
            walk: None,
            caps: Vec::new(),
        };
        s.load();
        s
    }

    fn load(&mut self) {
        let read = || -> Result<Vec<u8>, i32> {
            let fs = Fs::connect()?;
            let f = File::open(&fs, JOBS_PATH, mode::READ | mode::SHARE_READERS_ONLY)?;
            let size = usize::try_from(f.size()?).map_err(|_| kerr::CORRUPT)?;
            if size > JOBS_FILE_MAX {
                return Err(kerr::CORRUPT);
            }
            let mut b = vec![0; size];
            // a short read is a failed one: it is tried again, never taken as the table
            if f.read_at(0, &mut b)? != size {
                return Err(kerr::CORRUPT);
            }
            Ok(b)
        };
        let (jobs, saved) = match read() {
            Ok(b) => match Jobs::restore(&b, unix_s()) {
                Some(j) => (j, b),
                None => {
                    dprintln!("neoui: jobs file damaged, receipts lost");
                    (Jobs::default(), b)
                }
            },
            Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => {
                (Jobs::default(), Jobs::default().encode())
            }
            Err(e) => {
                if self.tick_at == 0 {
                    dprintln!("neoui: jobs file unreadable ({}), trying again", e);
                }
                return;
            }
        };
        // the interrupted jobs are saved so (and a damaged file replaced) at the next tick
        self.dirty = jobs.encode() != saved;
        self.jobs = jobs;
        self.published = self.jobs.records();
        self.loaded = true;
    }

    /// Write the table if it changed: to [`JOBS_STAGING`], flushed and closed, which then replaces [`JOBS_PATH`] in
    /// one file-server operation, so a failed save leaves the last table whole. Never before the last one was read.
    fn save(&mut self) -> Result<(), i32> {
        if !self.loaded {
            return Err(wire::KERR_NOT_READY);
        }
        if !self.dirty {
            return Ok(());
        }
        let r = || -> Result<(), i32> {
            let fs = Fs::connect()?;
            for dir in ["C:\\private\\", "C:\\private\\e0002001\\"] {
                match fs.mkdir(dir) {
                    Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
                    Err(e) => return Err(e),
                }
            }
            {
                let f = File::replace(&fs, JOBS_STAGING, mode::SHARE_EXCLUSIVE)?;
                f.write(&self.jobs.encode())?;
                f.flush()?;
            }
            fs.replace_file(JOBS_STAGING, JOBS_PATH)
        }();
        match r {
            Ok(()) => self.dirty = false,
            Err(e) => dprintln!("neoui: jobs save failed ({})", e),
        }
        r
    }

    /// A paged kind's whole snapshot; `None` for a kind this shell doesn't build.
    fn records(&self, kind: i32) -> Option<Vec<Vec<u8>>> {
        Some(match kind {
            proto::INFO_APP_DETAIL => self.app_details(),
            proto::INFO_BATTERY_CYCLE => {
                self.cycle.iter().map(wire::BatteryCycle::encode).collect()
            }
            proto::INFO_JOBS => self.published.clone(),
            proto::INFO_FEATURES => vec![Features::SHELL.encode()],
            _ => return None,
        })
    }

    /// INFO 8: one record per listed app.
    fn app_details(&self) -> Vec<Vec<u8>> {
        apps::listed()
            .map(|a| {
                let report = self.jobs.measured(a.key);
                let measuring = self.jobs.jobs().any(|j| {
                    j.state < state::COMMITTED
                        && i32::from(j.action) == proto::ACTION_MEASURE_APP
                        && j.subject == Some(AppRef::Native(a.key))
                });
                let found = self
                    .found
                    .iter()
                    .find(|(k, _)| *k == a.key)
                    .and_then(|(_, f)| *f);
                // (D8) the kernel's word once it ran; a ROM EXE has them all (the ROM build's rule); else unknown
                let (cap_source, caps) = match self.caps.iter().find(|(k, _)| *k == a.key) {
                    Some((_, c)) => (permissions::source::KERNEL, *c),
                    None if found.is_some_and(|(drive, _)| drive == 25) => {
                        (permissions::source::IMAGE, permissions::ALL)
                    }
                    None => (permissions::source::UNKNOWN, 0),
                };
                AppDetail {
                    app: AppRef::Native(a.key),
                    caption: a.name.into(),
                    exe: format!("{}.EXE", a.exe.to_ascii_uppercase()),
                    version: report.map(|r| r.version.clone()).unwrap_or_default(),
                    identity_source: if found.is_some() { 2 } else { 1 },
                    sid: apps::sid(a.key).unwrap_or(0),
                    cap_source,
                    caps,
                    coverage: if measuring {
                        coverage::MEASURING
                    } else {
                        report.map_or(coverage::UNKNOWN, |r| r.coverage)
                    },
                    measured_utc: report.map_or(0, |r| r.measured_utc),
                    clear_scope: self.jobs.offered(a.key),
                    drives: with_installed(report.map_or(&[], |r| &r.drives), found),
                }
                .encode()
            })
            .collect()
    }

    /// D4: the report of the shell's own measurement of an app (not an owner): its private folder on C: as data,
    /// partial (an app can keep data elsewhere: Clock's are DTSRV's, Phone's call log MODEMD's).
    fn walk_report(key: &str, counted: Result<u64, i32>) -> OwnerReport {
        match counted {
            Ok(bytes) => OwnerReport {
                state: state::PARTIAL,
                coverage: coverage::PARTIAL,
                measured_utc: unix_s(),
                drives: vec![DriveUsage {
                    drive: 2,
                    data: Some(bytes),
                    ..DriveUsage::default()
                }],
                ..OwnerReport::default()
            },
            Err(e) => {
                dprintln!("neoui: measuring {} failed ({})", key, e);
                failed(e)
            }
        }
    }

    /// Session `id` ended: no more hints for it.
    pub fn drop_session(&mut self, id: u32) {
        if self.watch.is_some_and(|(s, _)| s == id) {
            self.watch = None;
        }
    }
}

fn failed(e: i32) -> OwnerReport {
    OwnerReport {
        state: state::FAILED,
        result: e,
        coverage: coverage::FAILED,
        measured_utc: unix_s(),
        ..OwnerReport::default()
    }
}

/// Where the loader takes EXE `exe` from (C:, else Z:) and its bytes; `None` when it isn't in the image.
fn find_exe(exe: &str) -> Option<(u8, u64)> {
    let fs = Fs::connect().ok()?;
    [(2u8, 'C'), (25, 'Z')].into_iter().find_map(|(drive, l)| {
        fs.entry(&format!("{l}:\\sys\\bin\\{exe}.exe"))
            .ok()
            .map(|e| (drive, e.size))
    })
}

/// Entries read at most for one app's folder (a bigger one counts as what was read: the count is partial anyway).
// shortcut: counted in the shell's loop (a folder of a few files takes a few ms); a worker thread if an app's private
// folder ever grows to thousands of entries.
const FOLDER_ENTRIES_MAX: u32 = 1024;

/// The bytes in folder `dir` (`X:\...\`) and below; a folder that isn't there holds none.
fn folder_bytes(fs: &Fs, dir: &str) -> Result<u64, i32> {
    const PAGE: usize = 64;
    let (mut bytes, mut budget) = (0u64, FOLDER_ENTRIES_MAX);
    let mut stack = vec![String::from(dir)];
    while let Some(d) = stack.pop() {
        let mut offset = 0u32;
        loop {
            let page = match fs.read_dir_page(&d, offset, PAGE) {
                Ok(p) => p,
                Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) if d == dir => return Ok(0),
                Err(e) if d == dir => return Err(e),
                Err(_) => break, // a subfolder that went away
            };
            for e in &page {
                if e.is_dir() {
                    stack.push(format!("{d}{}\\", e.name));
                } else {
                    bytes = bytes.saturating_add(e.size);
                }
            }
            budget = budget.saturating_sub(page.len() as u32);
            if budget == 0 {
                return Ok(bytes);
            }
            if page.len() < PAGE {
                break;
            }
            offset += page.len() as u32;
        }
    }
    Ok(bytes)
}

/// `ACTION_CLEAR`'s request starts with [`wire::CONFIRMED`] (1.20) rather than an app key (1.17).
pub fn is_confirmed_clear(m: &Message) -> bool {
    read_bytes(m, 1, wire::REQUEST_MAX).is_ok_and(|b| b.first() == Some(&wire::CONFIRMED))
}

impl Host {
    /// `SETTINGS_INFO` of a paged kind (7 and up) from app `key` on `session`: Settings only.
    pub(super) fn paged_info(&mut self, key: &str, session: u32, m: &Message) -> i32 {
        if access::caller(key, m.client_sid()) != Caller::Settings {
            return kerr::PERMISSION_DENIED;
        }
        let kind = m.int(0);
        // TP4's channels are the host's (notifications.rs)
        let records = if kind == proto::INFO_CHANNELS {
            Some(self.channel_records())
        } else if kind == proto::INFO_TILES {
            // TP7's tiles are the host's (tiles.rs)
            Some(self.tile_records())
        } else {
            self.shelf.records(kind)
        };
        let (Some(records), Ok(k)) = (records, u8::try_from(kind)) else {
            return kerr::NOT_SUPPORTED;
        };
        let (first, expected) = (m.int(1) as u32, m.int(3) as u32);
        if first == 0 && expected == 0 {
            // subscribe before the snapshot, so a change after it is never missed
            let mask = match self.shelf.watch {
                Some((s, mask)) if s == session => mask,
                _ => 0,
            };
            self.shelf.watch = Some((session, mask | 1 << k));
        }
        let page = m
            .des_max_len(2)
            .and_then(|max| wire::page(k, self.shelf.revs.get(k), &records, first, expected, max));
        match page.and_then(|b| m.write8(2, &b, 0).map(|()| b.len() as i32)) {
            Ok(n) => n,
            Err(e) => e,
        }
    }

    /// Kind `kind` changed: its revision moves on, and Settings hears it if it read the kind.
    pub(super) fn info_changed(&mut self, kind: i32) {
        let k = kind as u8;
        let rev = self.shelf.revs.bump(k);
        if let Some((session, mask)) = self.shelf.watch
            && mask & 1 << k != 0
            && let Some(app) = self.find_session(session)
        {
            app.send(AppEvent::SettingsInfo { kind: k, rev });
        }
    }

    fn find_session(&mut self, session: u32) -> Option<&mut App> {
        self.apps
            .find_mut(|a| a.session == Some(session) && a.quit_at.is_none())
    }

    /// (D8) App `key`'s session bound from process `pid`: its capabilities as the kernel holds them; INFO 8 moves on
    /// when they changed.
    pub(super) fn saw_caps(&mut self, key: &'static str, pid: u32) {
        let Some(caps) = crate::caps::of_process(pid) else {
            return;
        };
        match self.shelf.caps.iter_mut().find(|(k, _)| *k == key) {
            Some((_, c)) if *c == caps => return,
            Some((_, c)) => *c = caps,
            None => self.shelf.caps.push((key, caps)),
        }
        dprintln!("neoui: {} runs with capabilities {:#x}", key, caps);
        self.info_changed(proto::INFO_APP_DETAIL);
    }

    /// The legacy bridge's entries changed: INFO 8's revision (the thumbnails' catalogue) moves on.
    pub(super) fn legacy_entries_changed(&mut self) {
        self.shelf.thumbs.clear();
        self.info_changed(proto::INFO_APP_DETAIL);
    }

    /// A request changed the table (it was `before`): it stands only once saved, else it goes back and the request
    /// fails. Then INFO 13 moves on and the owners due are woken.
    fn commit(&mut self, before: Jobs) -> Result<(), i32> {
        if self.shelf.jobs.encode() != before.encode() {
            let was_dirty = self.shelf.dirty;
            self.shelf.dirty = true;
            if let Err(e) = self.shelf.save() {
                self.shelf.jobs = before;
                self.shelf.dirty = was_dirty;
                return Err(e);
            }
        }
        self.saved();
        Ok(())
    }

    /// A change nobody asked for (an owner gone, a job expired): kept, and saved now or by a later tick.
    fn pending(&mut self) {
        self.shelf.dirty = true;
        if self.shelf.save().is_ok() {
            self.saved();
        }
    }

    /// After a save (or with nothing to save): INFO 13 shows the table, and the owners due are woken.
    fn saved(&mut self) {
        if self.shelf.dirty {
            return;
        }
        let records = self.shelf.jobs.records();
        if records != self.shelf.published {
            self.shelf.published = records;
            self.info_changed(proto::INFO_JOBS);
            // D4: a measurement queued or ended, a clear's size
            self.info_changed(proto::INFO_APP_DETAIL);
        }
        self.wake_owners();
    }

    fn wake_owners(&mut self) {
        for (key, session, e) in self.shelf.jobs.wakes() {
            match self.find_session(session) {
                Some(app) if app.key == key => {
                    dprintln!("neoui: owner {} woken: {:?}", key, e);
                    app.send(e);
                }
                _ => dprintln!("neoui: owner {} gone before its wake", key),
            }
        }
    }

    /// A 1.20 request from Settings (SID `sid`, its `nonce` and bytes): a retry gets its ticket again; a new one is
    /// queued for app `key`.
    fn job_request(
        &mut self,
        sid: u32,
        nonce: u64,
        request: &[u8],
        action: i32,
        app: AppRef,
        scope: u32,
    ) -> i32 {
        if !self.shelf.loaded {
            return wire::KERR_NOT_READY;
        }
        match self.shelf.dedup.check(sid, nonce, request) {
            Ok(Some(ticket)) => return ticket as i32,
            Ok(None) => {}
            Err(e) => return e,
        }
        // an old app has no owner; neither has a service nor Settings itself (Clock and Calendar own their data)
        let AppRef::Native(key) = app else {
            return kerr::NOT_SUPPORTED;
        };
        if key == SETTINGS || key == LEGACY || apps::is_service(key) {
            return kerr::NOT_SUPPORTED;
        }
        // D4: an app that isn't an owner (and isn't one registered now) is never started for a size: the shell takes
        // the measurement (saved with the ticket) and counts on its worker (`walks`)
        let here = action == proto::ACTION_MEASURE_APP
            && apps::owner_scope(key).is_none()
            && !self.shelf.jobs.registered(key);
        let before = self.shelf.jobs.clone();
        let ticket = match self
            .shelf
            .jobs
            .submit(action, key, scope, e32::time::millis())
        {
            Ok(t) => t,
            Err(e) => return e,
        };
        if here && let Err(e) = self.shelf.jobs.start_here(key, ticket) {
            self.shelf.jobs = before;
            return e;
        }
        // the ticket exists once it is saved: a failed save answers its error and remembers nothing
        if let Err(e) = self.commit(before) {
            return e;
        }
        self.shelf.dedup.record(sid, nonce, request, ticket);
        if here {
            self.walks();
        }
        dprintln!(
            "neoui: settings action {} for {}: ticket {}",
            action,
            key,
            ticket
        );
        ticket as i32
    }

    /// `ACTION_MEASURE_APP` (Settings only, checked by the caller).
    pub(super) fn measure_action(&mut self, m: &Message) -> i32 {
        let Ok(b) = read_bytes(m, 1, wire::REQUEST_MAX) else {
            return kerr::ARGUMENT;
        };
        match MeasureRequest::decode(&b) {
            Some(r) => self.job_request(
                m.client_sid(),
                r.nonce,
                &b,
                proto::ACTION_MEASURE_APP,
                r.app,
                0,
            ),
            None => kerr::ARGUMENT,
        }
    }

    /// `ACTION_CLEAR`'s confirmed form (Settings only, checked by the caller): never downgraded to the 1.17 clear.
    pub(super) fn confirmed_clear(&mut self, m: &Message) -> i32 {
        let Ok(b) = read_bytes(m, 1, wire::REQUEST_MAX) else {
            return kerr::ARGUMENT;
        };
        match ClearRequest::decode(&b) {
            Some(r) => self.job_request(
                m.client_sid(),
                r.nonce,
                &b,
                proto::ACTION_CLEAR,
                r.app,
                r.scope,
            ),
            None => kerr::ARGUMENT,
        }
    }

    /// `OWNER_REPORT` from app `key` (connected with `minor`) on `session`.
    pub(super) fn owner_report(
        &mut self,
        key: &'static str,
        minor: u8,
        session: u32,
        m: &Message,
    ) -> i32 {
        let (sid, ticket) = (m.client_sid(), m.int(1) as u32);
        let r = if self.shelf.loaded {
            match m.int(0) {
                wire::OWNER_REGISTER => self.owner_register(key, sid, session, minor, m),
                wire::OWNER_CLAIM => self.owner_claim(key, sid, session, ticket, m),
                wire::OWNER_FINISH => self.owner_finish(key, sid, session, ticket, m),
                _ => Err(kerr::ARGUMENT),
            }
        } else {
            Err(wire::KERR_NOT_READY)
        };
        if let Err(e) = r {
            dprintln!(
                "neoui: owner report {} from {} refused ({})",
                m.int(0),
                key,
                e
            );
        }
        r.unwrap_or_else(|e| e)
    }

    fn owner_register(
        &mut self,
        key: &'static str,
        sid: u32,
        session: u32,
        minor: u8,
        m: &Message,
    ) -> Result<i32, i32> {
        let b = read_bytes(m, 2, 2)?;
        let before = self.shelf.jobs.clone();
        let (now_ms, now_utc) = (e32::time::millis(), unix_s());
        self.shelf
            .jobs
            .register(key, sid, session, minor, &b, (now_ms, now_utc))?;
        // D4: what this owner clears, offered from now on (also once it quit: a clear starts it again)
        let offer = wire::decode_register(&b)
            .is_some_and(|c| c & capability::CLEAR != 0)
            .then(|| apps::owner_scope(key).unwrap_or(0))
            .filter(|&scope| scope != self.shelf.jobs.offered(key));
        if let Some(scope) = offer {
            self.shelf.jobs.set_offered(key, scope);
        }
        self.commit(before)?;
        if offer.is_some() {
            self.info_changed(proto::INFO_APP_DETAIL);
        }
        dprintln!("neoui: {} registered as an owner", key);
        Ok(0)
    }

    /// The reply is copied to the owner first; only then is the job taken (and saved).
    fn owner_claim(
        &mut self,
        key: &'static str,
        sid: u32,
        session: u32,
        ticket: u32,
        m: &Message,
    ) -> Result<i32, i32> {
        let b = self
            .shelf
            .jobs
            .claimable(key, sid, session, ticket)?
            .encode();
        if m.des_max_len(2)? < b.len() {
            return Err(kerr::OVERFLOW);
        }
        m.write8(2, &b, 0)?;
        let before = self.shelf.jobs.clone();
        self.shelf.jobs.claim(key, sid, session, ticket)?;
        // a claim that didn't save fails: the owner was given the work but hears an error, and does nothing
        self.commit(before)?;
        Ok(b.len() as i32)
    }

    /// The receipt is saved before the owner hears 0; the same report again retries a save still due.
    fn owner_finish(
        &mut self,
        key: &'static str,
        sid: u32,
        session: u32,
        ticket: u32,
        m: &Message,
    ) -> Result<i32, i32> {
        let report = OwnerReport::decode(&read_bytes(m, 2, wire::OWNER_REPORT_MAX)?)
            .ok_or(kerr::ARGUMENT)?;
        let before = self.shelf.jobs.clone();
        if self
            .shelf
            .jobs
            .finish(key, sid, session, ticket, report, unix_s())?
        {
            self.commit(before)?;
        } else {
            self.shelf.save()?;
            self.saved();
        }
        Ok(0)
    }

    /// `ENTRY_THUMB` from app `key`: Settings and Classic apps only (`apps::old_apps_client`).
    pub(super) fn entry_thumb(&mut self, key: &str, m: &Message) -> i32 {
        if !neoui_proto::apps::old_apps_client(key, m.client_sid()) {
            return kerr::PERMISSION_DENIED;
        }
        let (id, expected) = (m.int(0) as u32, m.int(2) as u32);
        if expected != 0 && expected != self.shelf.revs.get(proto::INFO_APP_DETAIL as u8) {
            return wire::KERR_NOT_READY;
        }
        let thumb = match self.shelf.thumbs.iter().find(|(i, _)| *i == id) {
            Some((_, t)) => t.clone(),
            None => {
                let Some(entry) = self.entries(LEGACY).iter().find(|e| e.id == id) else {
                    return err::KERR_NOT_FOUND;
                };
                let t = wire::thumb(&entry.icon);
                self.shelf.thumbs.push((id, t.clone()));
                t
            }
        };
        let r = m.des_max_len(1).and_then(|max| {
            if thumb.len() > max {
                return Err(kerr::OVERFLOW);
            }
            m.write8(1, &thumb, 0).map(|()| thumb.len() as i32)
        });
        r.unwrap_or_else(|e| e)
    }

    /// Session `id` ended: no more hints for it, its claimed jobs are interrupted.
    pub(super) fn settings_session_gone(&mut self, id: u32) {
        self.shelf.drop_session(id);
        if self
            .shelf
            .jobs
            .session_gone(id, e32::time::millis(), unix_s())
        {
            self.pending();
        }
    }

    /// Once a second from the main loop: the saved table is read if that failed at boot, jobs whose owner never came
    /// fail, an unsaved table is saved (then its owners are woken), and the first app of a queued job that isn't
    /// running is started hidden (one start a tick; connections are bound by SID, `Host::bind`).
    pub fn tick_jobs(&mut self, now_ms: u64) {
        // the millisecond counter wraps: a time far before the next tick is a new lap
        if now_ms < self.shelf.tick_at && self.shelf.tick_at - now_ms <= 1000 {
            return;
        }
        self.shelf.tick_at = now_ms + 1000;
        if !self.shelf.loaded {
            self.shelf.load();
            if !self.shelf.loaded {
                return;
            }
            self.saved();
        }
        if self.shelf.jobs.expire(now_ms, unix_s()) || self.shelf.dirty {
            self.pending();
        }
        self.walks();
        let start = self.shelf.jobs.waiting_owners().into_iter().find(|&key| {
            self.apps
                .find(|a| a.key == key && a.quit_at.is_none())
                .is_none()
        });
        if let Some(key) = start {
            dprintln!("neoui: starting {} for its owner work", key);
            self.launch(key);
        }
    }
}

// D4: the shell's own measurements, one at a time on a worker thread (never in an IPC handler)
impl Host {
    /// A finished walk ends its job (kept and saved like an owner's going away: `pending`); then the next taken
    /// measurement starts its walk.
    fn walks(&mut self) {
        if let Some(w) = &self.shelf.walk {
            let Some((exe, counted)) = w.out.lock().take() else {
                return; // still counting
            };
            let w = self.shelf.walk.take().expect("a walk");
            w.thread.join();
            if let Some(f) = self.shelf.found.iter_mut().find(|(k, _)| *k == w.key) {
                f.1 = exe;
            }
            let report = Shelf::walk_report(w.key, counted);
            match self
                .shelf
                .jobs
                .finish_here(w.key, w.ticket, report, unix_s())
            {
                Ok(()) => self.pending(),
                Err(e) => dprintln!("neoui: measurement {} not recorded ({})", w.ticket, e),
            }
        }
        let Some((key, ticket)) = self.shelf.jobs.taken_here().into_iter().next() else {
            return;
        };
        let (Some(a), Some(sid)) = (apps::find(key), apps::sid(key)) else {
            let _ = self
                .shelf
                .jobs
                .finish_here(key, ticket, failed(kerr::NOT_SUPPORTED), unix_s());
            self.pending();
            return;
        };
        let started = Mutex::new(None).map(Arc::new).and_then(|out| {
            let (o, exe, dir) = (out.clone(), a.exe, format!("C:\\private\\{sid:08x}\\"));
            e32::thread::spawn(move || {
                let found = (
                    find_exe(exe),
                    Fs::connect().and_then(|fs| folder_bytes(&fs, &dir)),
                );
                *o.lock() = Some(found);
                0
            })
            .map(|thread| Walk {
                key,
                ticket,
                thread,
                out,
            })
        });
        match started {
            Ok(w) => self.shelf.walk = Some(w),
            Err(e) => {
                let _ = self.shelf.jobs.finish_here(
                    key,
                    ticket,
                    Shelf::walk_report(key, Err(e)),
                    unix_s(),
                );
                self.pending();
            }
        }
    }
}

// D5
impl Host {
    /// The battery recorder's cycle (every five seconds): INFO 9 changes, and Settings hears it, when what it shows
    /// does (the minutes, the levels, the flags, the start), not at every sample.
    pub fn set_battery_cycle(&mut self, c: Option<wire::BatteryCycle>) {
        let shown = |c: Option<wire::BatteryCycle>| {
            c.map(|c| wire::BatteryCycle {
                last_utc: c.last_utc / 60,
                on_battery_s: c.on_battery_s / 60,
                screen_on_battery_s: c.screen_on_battery_s / 60,
                gap_s: c.gap_s / 60,
                ..c
            })
        };
        if shown(c) != shown(self.shelf.cycle) {
            self.shelf.cycle = c;
            self.info_changed(proto::INFO_BATTERY_CYCLE);
        }
    }
}

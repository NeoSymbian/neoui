// SPDX-License-Identifier: GPL-3.0-only
//! Protocol 1.20's Settings snapshots and owner work in SYSSTART (docs neoui-shell/protocol.md "Settings snapshots,
//! owner reports and panel policy"): per-kind revisions and the Settings session's subscriptions (event 27), the paged
//! `SETTINGS_INFO` kinds 8, 9, 13 and 14, `ACTION_MEASURE_APP` and `ACTION_CLEAR`'s confirmed form with their nonces,
//! `OWNER_REPORT` over [`neoui_proto::settings_app::jobs`] (saved to [`JOBS_PATH`] before a wake and before an
//! acknowledgement), and `ENTRY_THUMB`. A child of `host`: it works on the host's apps.

use super::{App, Host, read_bytes};
use alloc::vec;
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use e32::server::Message;
use e32::sys::err;
use neoui_proto::apps::{self, LEGACY, SETTINGS};
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::settings::{Caller, access, kerr};
use neoui_proto::settings_app::jobs::Jobs;
use neoui_proto::settings_app::wire::{
    self, AppRef, ClearRequest, Dedup, Features, MeasureRequest, OwnerReport, Revisions,
};

/// The owner-work table across restarts.
const JOBS_PATH: &str = "C:\\private\\e0002001\\jobs.dat";
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
    tick_at: u64,
    /// D5: INFO 9's record as last published (`None`: no valid battery reading yet).
    cycle: Option<wire::BatteryCycle>,
}

fn unix_s() -> i64 {
    e32::time::utc_now().0
}

impl Shelf {
    /// The saved table, every unfinished job in it interrupted.
    pub fn boot() -> Self {
        let read = || -> Result<Vec<u8>, i32> {
            let fs = Fs::connect()?;
            let f = File::open(&fs, JOBS_PATH, mode::READ | mode::SHARE_READERS_ONLY)?;
            let size = usize::try_from(f.size()?).map_err(|_| kerr::CORRUPT)?;
            if size > JOBS_FILE_MAX {
                return Err(kerr::CORRUPT);
            }
            let mut b = vec![0; size];
            let n = f.read_at(0, &mut b)?;
            b.truncate(n);
            Ok(b)
        };
        let jobs = match read() {
            Ok(b) => Jobs::restore(&b, unix_s()).unwrap_or_else(|| {
                dprintln!("neoui: jobs file damaged, receipts lost");
                Jobs::default()
            }),
            Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => Jobs::default(),
            Err(e) => {
                dprintln!("neoui: jobs file unreadable ({})", e);
                Jobs::default()
            }
        };
        Self {
            revs: Revisions::default(),
            jobs,
            dedup: Dedup::default(),
            watch: None,
            thumbs: Vec::new(),
            dirty: false,
            tick_at: 0,
            cycle: None,
        }
    }

    /// Write the table if it changed.
    // shortcut: one file replaced in place (a torn write loses the receipts, never resumes a job); A/B files like the
    // battery history if receipts ever must survive that.
    fn save(&mut self) -> Result<(), i32> {
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
            File::replace(&fs, JOBS_PATH, mode::SHARE_EXCLUSIVE)?.write(&self.jobs.encode())
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
            // their producers (D4, D5) aren't built: an empty collection, and FEATURES doesn't list them
            proto::INFO_APP_DETAIL => Vec::new(),
            proto::INFO_BATTERY_CYCLE => {
                self.cycle.iter().map(wire::BatteryCycle::encode).collect()
            }
            proto::INFO_JOBS => self.jobs.records(),
            proto::INFO_FEATURES => vec![Features::SHELL.encode()],
            _ => return None,
        })
    }

    /// Session `id` ended: no more hints for it.
    pub fn drop_session(&mut self, id: u32) {
        if self.watch.is_some_and(|(s, _)| s == id) {
            self.watch = None;
        }
    }
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
        let (Some(records), Ok(k)) = (self.shelf.records(kind), u8::try_from(kind)) else {
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
    fn info_changed(&mut self, kind: i32) {
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

    /// The legacy bridge's entries changed: INFO 8's revision (the thumbnails' catalogue) moves on.
    pub(super) fn legacy_entries_changed(&mut self) {
        self.shelf.thumbs.clear();
        self.info_changed(proto::INFO_APP_DETAIL);
    }

    /// The table changed: save it, then INFO 13 moves on and the owners due are woken (not before the save).
    fn jobs_changed(&mut self) -> Result<(), i32> {
        self.shelf.dirty = true;
        let saved = self.shelf.save();
        self.info_changed(proto::INFO_JOBS);
        if saved.is_ok() {
            self.wake_owners();
        }
        saved
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
        let ticket = match self
            .shelf
            .jobs
            .submit(action, key, scope, e32::time::millis())
        {
            Ok(t) => t,
            Err(e) => return e,
        };
        self.shelf.dedup.record(sid, nonce, request, ticket);
        dprintln!(
            "neoui: settings action {} for {}: ticket {}",
            action,
            key,
            ticket
        );
        // unsaved, it is queued but no owner is woken: the tick saves and wakes
        let _ = self.jobs_changed();
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
        let r = match m.int(0) {
            wire::OWNER_REGISTER => read_bytes(m, 2, 2)
                .and_then(|b| self.shelf.jobs.register(key, sid, session, minor, &b))
                .map(|()| {
                    dprintln!("neoui: {} registered as an owner", key);
                    if !self.shelf.dirty {
                        self.wake_owners();
                    }
                    0
                }),
            wire::OWNER_CLAIM => m
                .des_max_len(2)
                .and_then(|max| {
                    // room first: a claim that can't be answered must not take the job
                    if max < wire::OWNER_WORK_MAX {
                        return Err(kerr::OVERFLOW);
                    }
                    self.shelf.jobs.claim(key, sid, session, ticket)
                })
                .and_then(|work| {
                    let _ = self.jobs_changed();
                    let b = work.encode();
                    m.write8(2, &b, 0).map(|()| b.len() as i32)
                }),
            wire::OWNER_FINISH => read_bytes(m, 2, wire::OWNER_REPORT_MAX)
                .and_then(|b| OwnerReport::decode(&b).ok_or(kerr::ARGUMENT))
                .and_then(|report| {
                    self.shelf
                        .jobs
                        .finish(key, sid, session, ticket, report, unix_s())
                })
                // the receipt is saved before the owner hears 0 (the same report again retries the save)
                .and_then(|changed| {
                    if changed {
                        self.jobs_changed()
                    } else {
                        self.shelf.save()
                    }
                })
                .map(|()| 0),
            _ => Err(kerr::ARGUMENT),
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

    /// `ENTRY_THUMB` from app `key`: Settings only.
    pub(super) fn entry_thumb(&mut self, key: &str, m: &Message) -> i32 {
        if access::caller(key, m.client_sid()) != Caller::Settings {
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
            let _ = self.jobs_changed();
        }
    }

    /// Once a second from the main loop: jobs whose owner never came fail, an unsaved table is saved (then its
    /// owners are woken), and the owner of a queued job is started hidden, one at a time.
    pub fn tick_jobs(&mut self, now_ms: u64) {
        if now_ms < self.shelf.tick_at {
            return;
        }
        self.shelf.tick_at = now_ms + 1000;
        if self.shelf.jobs.expire(now_ms, unix_s()) {
            let _ = self.jobs_changed();
        } else if self.shelf.dirty && self.shelf.save().is_ok() {
            self.wake_owners();
        }
        let Some(key) = self.shelf.jobs.waiting_owner() else {
            return;
        };
        // `connect` binds the newest app without a session: one hidden start at a time
        if self
            .apps
            .find(|a| (a.key == key || a.session.is_none()) && a.quit_at.is_none())
            .is_none()
        {
            dprintln!("neoui: starting {} for its owner work", key);
            self.launch(key);
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

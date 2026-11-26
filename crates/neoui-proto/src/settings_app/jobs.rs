// SPDX-License-Identifier: Apache-2.0
//! The shell's acknowledged owner work (protocol 1.20, docs neoui-shell/protocol.md "Owner work"): Settings asks
//! (`ACTION_MEASURE_APP`, `ACTION_CLEAR`'s confirmed form) and gets a ticket; the app that owns the data registers
//! (`OWNER_REPORT` mode 0), is woken (`AppEvent::OwnerWork`, `AppEvent::ClearTicket`), claims the work (mode 1) and
//! reports its result (mode 2); Settings reads the outcome in INFO 13 ([`Jobs::records`]).
//!
//! Every step checks the owner's key, its kernel secure ID, its current session, its declared capability and the
//! pending ticket. A wake alone authorises nothing: an owner deletes only what it claimed. A claimed job whose owner
//! goes away, and every unfinished job across a restart ([`Jobs::restore`]), ends interrupted: a new confirmation is
//! needed before anything more is deleted. SYSSTART saves [`Jobs::encode`] before it wakes an owner and before it
//! acknowledges a report.

use super::wire::{
    self, AppRef, Job, KERR_ALREADY_EXISTS, KERR_DIED, OwnerReport, OwnerWork, capability, state,
};
use crate::apps;
use crate::ipc::{self, AppEvent};
use crate::settings::kerr;
use alloc::vec::Vec;

/// Unfinished jobs at most (`KErrOverflow` past it).
pub const UNFINISHED_MAX: usize = 16;
/// Finished jobs kept as receipts (the latest).
pub const RECEIPTS_MAX: usize = 32;
/// How long a queued job waits for its owner to register and claim it before it fails.
pub const OWNER_WAIT_MS: u64 = 20_000;
/// `KErrTimedOut`: the owner was woken but never claimed the work.
pub const KERR_TIMED_OUT: i32 = -33;
/// The first protocol minor whose apps can be owners.
pub const OWNER_MINOR: u8 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Owner {
    key: &'static str,
    sid: u32,
    session: u32,
    caps: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    job: Job,
    /// Its owner was woken for it.
    dispatched: bool,
    /// The session that claimed it.
    claimed: Option<u32>,
    /// When it was queued (or its woken owner went away), ms.
    since_ms: u64,
    report: Option<OwnerReport>,
}

impl Entry {
    fn key(&self) -> Option<&'static str> {
        match self.job.subject {
            Some(AppRef::Native(k)) => Some(k),
            _ => None,
        }
    }
    fn work(&self) -> u8 {
        if i32::from(self.job.action) == ipc::ACTION_CLEAR {
            wire::WORK_CLEAR
        } else {
            wire::WORK_MEASURE
        }
    }
    fn done(&self) -> bool {
        self.job.state >= state::COMMITTED
    }
    fn end(&mut self, st: u8, result: i32, now_utc: i64) {
        self.job.state = st;
        self.job.result = result;
        self.job.finished_utc = now_utc;
    }
}

fn needs(work: u8) -> u8 {
    if work == wire::WORK_CLEAR {
        capability::CLEAR
    } else {
        capability::MEASURE
    }
}

/// The table: owners registered now, jobs and receipts, what each owner offers to clear and its latest measurement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jobs {
    owners: Vec<Owner>,
    entries: Vec<Entry>,
    next: u32,
    offered: Vec<(&'static str, u32)>,
    measured: Vec<(&'static str, OwnerReport)>,
}

impl Default for Jobs {
    fn default() -> Self {
        Self {
            owners: Vec::new(),
            entries: Vec::new(),
            next: 1,
            offered: Vec::new(),
            measured: Vec::new(),
        }
    }
}

impl Jobs {
    /// `OWNER_REPORT` mode 0 from app `key` (by its kernel `sid`, on `session`, connected with protocol `minor`).
    ///
    /// # Errors
    /// `KErrPermissionDenied` for an app the shell can't tell apart by SID (the kit shares the legacy bridge's), a
    /// SID that isn't the app's, or a client older than 1.20; `KErrArgument` for a damaged payload.
    pub fn register(
        &mut self,
        key: &'static str,
        sid: u32,
        session: u32,
        minor: u8,
        payload: &[u8],
    ) -> Result<(), i32> {
        if minor < OWNER_MINOR || key == "kit" || key == apps::LEGACY || apps::sid(key) != Some(sid)
        {
            return Err(kerr::PERMISSION_DENIED);
        }
        let caps = wire::decode_register(payload).ok_or(kerr::ARGUMENT)?;
        self.owners.retain(|o| o.key != key);
        self.owners.push(Owner {
            key,
            sid,
            session,
            caps,
        });
        Ok(())
    }

    fn owner(&self, key: &str) -> Option<&Owner> {
        self.owners.iter().find(|o| o.key == key)
    }

    /// The owner `(key, sid, session)` as registered, or `KErrPermissionDenied`.
    fn check(&self, key: &str, sid: u32, session: u32) -> Result<Owner, i32> {
        match self.owner(key) {
            Some(o) if o.sid == sid && o.session == session => Ok(*o),
            _ => Err(kerr::PERMISSION_DENIED),
        }
    }

    /// What `key` offers to clear (INFO 8's `clear_scope`, set by its producer).
    pub fn set_offered(&mut self, key: &'static str, scope: u32) {
        self.offered.retain(|(k, _)| *k != key);
        if scope != 0 {
            self.offered.push((key, scope & wire::scope::ALL));
        }
    }

    #[must_use]
    pub fn offered(&self, key: &str) -> u32 {
        self.offered
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(0, |(_, s)| *s)
    }

    /// The latest finished measurement of `key`.
    #[must_use]
    pub fn measured(&self, key: &str) -> Option<&OwnerReport> {
        self.measured
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, r)| r)
    }

    /// Queue `action` (`ACTION_MEASURE_APP`, or `ACTION_CLEAR` with `scope`) for app `key`: its ticket. A measurement
    /// of an app already pending shares that ticket.
    ///
    /// # Errors
    /// `KErrNotSupported` for a clear of a scope the app doesn't offer, or an owner registered without the
    /// capability; `KErrOverflow` when [`UNFINISHED_MAX`] are pending; `KErrArgument` for another action.
    pub fn submit(
        &mut self,
        action: i32,
        key: &'static str,
        scope: u32,
        now_ms: u64,
    ) -> Result<u32, i32> {
        let work = match action {
            ipc::ACTION_CLEAR => wire::WORK_CLEAR,
            ipc::ACTION_MEASURE_APP => wire::WORK_MEASURE,
            _ => return Err(kerr::ARGUMENT),
        };
        if work == wire::WORK_MEASURE
            && let Some(e) = self
                .entries
                .iter()
                .find(|e| !e.done() && e.work() == work && e.key() == Some(key))
        {
            return Ok(e.job.ticket);
        }
        if work == wire::WORK_CLEAR && (scope == 0 || scope & !self.offered(key) != 0) {
            return Err(kerr::NOT_SUPPORTED);
        }
        if self.owner(key).is_some_and(|o| o.caps & needs(work) == 0) {
            return Err(kerr::NOT_SUPPORTED);
        }
        if self.entries.iter().filter(|e| !e.done()).count() >= UNFINISHED_MAX {
            return Err(kerr::OVERFLOW);
        }
        let ticket = self.ticket();
        self.entries.push(Entry {
            job: Job {
                ticket,
                action: action as u8,
                state: state::QUEUED,
                subject: Some(AppRef::Native(key)),
                requested_scope: if work == wire::WORK_CLEAR { scope } else { 0 },
                committed_scope: 0,
                result: 0,
                finished_utc: 0,
            },
            dispatched: false,
            claimed: None,
            since_ms: now_ms,
            report: None,
        });
        Ok(ticket)
    }

    fn ticket(&mut self) -> u32 {
        loop {
            let t = self.next;
            self.next = if t >= i32::MAX as u32 { 1 } else { t + 1 };
            if !self.entries.iter().any(|e| e.job.ticket == t) {
                return t;
            }
        }
    }

    /// A queued job's app that isn't registered yet: the shell starts it hidden (one at a time).
    #[must_use]
    pub fn waiting_owner(&self) -> Option<&'static str> {
        self.entries
            .iter()
            .filter(|e| e.job.state == state::QUEUED)
            .filter_map(Entry::key)
            .find(|k| self.owner(k).is_none())
    }

    /// The wakes due now, each `(key, session, event)`: queued jobs whose owner registered with the capability. They
    /// count as sent; the shell delivers each only to that session.
    pub fn wakes(&mut self) -> Vec<(&'static str, u32, AppEvent)> {
        let mut out = Vec::new();
        for e in &mut self.entries {
            let Some(key) = e.key() else { continue };
            let Some(o) = self.owners.iter().find(|o| o.key == key) else {
                continue;
            };
            if e.job.state != state::QUEUED || e.dispatched || o.caps & needs(e.work()) == 0 {
                continue;
            }
            e.dispatched = true;
            let ticket = e.job.ticket;
            let event = if e.work() == wire::WORK_CLEAR {
                AppEvent::ClearTicket { ticket }
            } else {
                AppEvent::OwnerWork { ticket }
            };
            out.push((key, o.session, event));
        }
        out
    }

    fn entry(&mut self, key: &str, ticket: u32) -> Result<&mut Entry, i32> {
        let e = self
            .entries
            .iter_mut()
            .find(|e| e.job.ticket == ticket)
            .ok_or(crate::settings::kerr::NOT_FOUND)?;
        if e.key() != Some(key) {
            return Err(kerr::PERMISSION_DENIED);
        }
        Ok(e)
    }

    /// `OWNER_REPORT` mode 1: the owner takes `ticket` (again, if it already did on this session).
    ///
    /// # Errors
    /// `KErrPermissionDenied` for a caller that isn't the registered owner with the capability, `KErrNotFound` for a
    /// ticket that isn't pending.
    pub fn claim(
        &mut self,
        key: &str,
        sid: u32,
        session: u32,
        ticket: u32,
    ) -> Result<OwnerWork, i32> {
        let o = self.check(key, sid, session)?;
        let e = self.entry(key, ticket)?;
        if o.caps & needs(e.work()) == 0 {
            return Err(kerr::PERMISSION_DENIED);
        }
        match (e.job.state, e.claimed) {
            (state::QUEUED, _) => {
                e.job.state = state::RUNNING;
                e.claimed = Some(session);
            }
            (state::RUNNING, Some(s)) if s == session => {}
            _ => return Err(kerr::NOT_FOUND),
        }
        Ok(OwnerWork {
            work: e.work(),
            subject: AppRef::Native(o.key),
            scope: e.job.requested_scope,
        })
    }

    /// `OWNER_REPORT` mode 2: the owner's result for its claimed `ticket`. `Ok(true)` when the job changed, `Ok(false)`
    /// for the same report again.
    ///
    /// # Errors
    /// As [`Self::claim`]; `KErrArgument` for a ticket not claimed on this session or a report that doesn't fit the
    /// work (more than was asked, a success with an error, a failure without one); `KErrAlreadyExists` for a
    /// different report on a finished ticket.
    pub fn finish(
        &mut self,
        key: &'static str,
        sid: u32,
        session: u32,
        ticket: u32,
        report: OwnerReport,
        now_utc: i64,
    ) -> Result<bool, i32> {
        self.check(key, sid, session)?;
        let e = self.entry(key, ticket)?;
        if e.done() {
            return if e.report.as_ref() == Some(&report) {
                Ok(false)
            } else {
                Err(KERR_ALREADY_EXISTS)
            };
        }
        if e.job.state != state::RUNNING || e.claimed != Some(session) {
            return Err(kerr::ARGUMENT);
        }
        let asked = e.job.requested_scope;
        let fits = report.committed_scope & !asked == 0
            && match report.state {
                state::COMMITTED => report.result == 0 && report.committed_scope == asked,
                state::FAILED => report.result < 0,
                _ => report.result <= 0,
            };
        if !fits {
            return Err(kerr::ARGUMENT);
        }
        e.job.committed_scope = report.committed_scope;
        e.end(report.state, report.result, now_utc);
        e.report = Some(report.clone());
        let measure = e.work() == wire::WORK_MEASURE;
        if measure {
            self.measured.retain(|(k, _)| *k != key);
            self.measured.push((key, report));
        }
        self.prune();
        Ok(true)
    }

    /// Session `session` ended: its owner registration goes, a job it claimed is interrupted, one it was only woken
    /// for waits for the next registration. True when a job changed.
    pub fn session_gone(&mut self, session: u32, now_ms: u64, now_utc: i64) -> bool {
        let gone: Vec<&'static str> = self
            .owners
            .iter()
            .filter(|o| o.session == session)
            .map(|o| o.key)
            .collect();
        self.owners.retain(|o| o.session != session);
        let mut changed = false;
        for e in &mut self.entries {
            if e.job.state == state::RUNNING && e.claimed == Some(session) {
                e.end(state::INTERRUPTED, KERR_DIED, now_utc);
                changed = true;
            } else if e.job.state == state::QUEUED
                && e.dispatched
                && e.key().is_some_and(|k| gone.contains(&k))
            {
                e.dispatched = false;
                e.since_ms = now_ms;
            }
        }
        if changed {
            self.prune();
        }
        changed
    }

    /// Queued jobs whose owner didn't claim them within [`OWNER_WAIT_MS`] fail: not supported when it never
    /// registered (an app older than 1.20, or one that isn't an owner), timed out when it was woken. True when one did.
    pub fn expire(&mut self, now_ms: u64, now_utc: i64) -> bool {
        let mut changed = false;
        for e in &mut self.entries {
            if e.job.state == state::QUEUED && now_ms.saturating_sub(e.since_ms) > OWNER_WAIT_MS {
                let r = if e.dispatched {
                    KERR_TIMED_OUT
                } else {
                    kerr::NOT_SUPPORTED
                };
                e.end(state::FAILED, r, now_utc);
                changed = true;
            }
        }
        if changed {
            self.prune();
        }
        changed
    }

    fn prune(&mut self) {
        let mut done = self.entries.iter().filter(|e| e.done()).count();
        self.entries.retain(|e| {
            if e.done() && done > RECEIPTS_MAX {
                done -= 1;
                false
            } else {
                true
            }
        });
    }

    /// The jobs, oldest first.
    pub fn jobs(&self) -> impl Iterator<Item = &Job> {
        self.entries.iter().map(|e| &e.job)
    }

    /// INFO 13's records.
    #[must_use]
    pub fn records(&self) -> Vec<Vec<u8>> {
        self.jobs().map(Job::encode).collect()
    }

    /// What SYSSTART saves: `[1][next:w][count:h]` then `count × [len:h][job]`.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![1];
        b.extend_from_slice(&self.next.to_le_bytes());
        b.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        for e in &self.entries {
            let j = e.job.encode();
            b.extend_from_slice(&(j.len() as u16).to_le_bytes());
            b.extend_from_slice(&j);
        }
        b
    }

    /// After a restart: the saved table with every unfinished job interrupted (nothing resumes on its own) and no
    /// owner registered. `None` for a damaged file.
    #[must_use]
    pub fn restore(b: &[u8], now_utc: i64) -> Option<Self> {
        let (&[1, n0, n1, n2, n3, c0, c1], mut rest) = b.split_first_chunk::<7>()? else {
            return None;
        };
        let next = u32::from_le_bytes([n0, n1, n2, n3]);
        let mut entries: Vec<Entry> = Vec::new();
        for _ in 0..u16::from_le_bytes([c0, c1]) {
            let (len, r) = rest.split_first_chunk::<2>()?;
            let len = usize::from(u16::from_le_bytes(*len));
            let mut job = Job::decode(r.get(..len)?)?;
            rest = &r[len..];
            if entries.iter().any(|e| e.job.ticket == job.ticket) {
                return None;
            }
            if job.state < state::COMMITTED {
                job.state = state::INTERRUPTED;
                job.result = KERR_DIED;
                job.finished_utc = now_utc;
            }
            entries.push(Entry {
                job,
                dispatched: false,
                claimed: None,
                since_ms: 0,
                report: None,
            });
        }
        if !rest.is_empty() || next == 0 || next > i32::MAX as u32 {
            return None;
        }
        let mut jobs = Self {
            entries,
            next,
            ..Self::default()
        };
        jobs.prune();
        Some(jobs)
    }
}

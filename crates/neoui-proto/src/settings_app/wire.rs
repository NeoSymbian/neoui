// SPDX-License-Identifier: Apache-2.0
//! Protocol 1.20's Settings snapshots and owner reports (docs neoui-shell/protocol.md "Settings snapshots, owner
//! reports and panel policy"): the paged `ipc::SETTINGS_INFO` transport for kinds 7 and up ([`page`],
//! [`parse_page`], [`Traversal`]), their records ([`AppDetail`], [`BatteryCycle`], [`Job`], [`Features`]), the
//! requests of `ipc::ACTION_CLEAR`'s confirmed form and `ipc::ACTION_MEASURE_APP`, the `ipc::OWNER_REPORT` payloads
//! and the `ipc::ENTRY_THUMB` thumbnail. Kinds 1–6 keep their 1.13–1.17 encodings ([`super::info`]); nothing here
//! wraps them.
//!
//! Integers are little-endian and packed. `sN` is a length byte and at most `N` bytes of UTF-8. A decoder takes a
//! whole record or nothing: a short field, a bad enum, a reserved bit or byte that isn't zero, or bytes left over give
//! `None`.

use crate::apps;
use crate::ipc;
use alloc::string::String;
use alloc::vec::Vec;

/// `KErrNotReady`: the snapshot changed since the page the client continues from (restart at record 0).
pub const KERR_NOT_READY: i32 = -18;
/// `KErrAlreadyExists`: a nonce reused for another request, or a second, different report for a finished ticket.
pub const KERR_ALREADY_EXISTS: i32 = -11;
/// `KErrDied`: the job's owner went away (or the phone restarted) while it worked; the job is interrupted.
pub const KERR_DIED: i32 = -13;

/// The page header's schema byte.
pub const SCHEMA: u8 = 1;
/// `schema:b, kind:b, count:h, rev:w, first:w, next:w`.
pub const HEADER_LEN: usize = 16;
/// The longest page, header included.
pub const PAGE_MAX: usize = 4096;
/// `next` of the last page.
pub const TERMINAL: u32 = u32::MAX;
/// One traversal's limits on the client: records and their bytes.
pub const TRAVERSAL_RECORDS_MAX: usize = 4096;
pub const TRAVERSAL_BYTES_MAX: usize = 1 << 20;

/// A page's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: u8,
    pub count: u16,
    pub rev: u32,
    pub first: u32,
    pub next: u32,
}

/// The server side of `SETTINGS_INFO` for a paged kind: the page of `records` (the whole snapshot at revision `rev`,
/// never 0) that starts at `first`, as many whole records as fit in `out_max` bytes (at most [`PAGE_MAX`]).
///
/// A first read sends `first = 0, expected_rev = 0`; a continuation the `next` and `rev` it was given.
///
/// # Errors
/// [`KERR_NOT_READY`] when `expected_rev` isn't 0 and isn't `rev`; `KErrArgument` for a continuation without a
/// revision or a cursor past the end; `KErrOverflow` when `out_max` can't hold the header and the next record. In each
/// case nothing is written.
pub fn page(
    kind: u8,
    rev: u32,
    records: &[Vec<u8>],
    first: u32,
    expected_rev: u32,
    out_max: usize,
) -> Result<Vec<u8>, i32> {
    use crate::settings::kerr;
    debug_assert!(rev != 0);
    if expected_rev == 0 && first != 0 {
        return Err(kerr::ARGUMENT);
    }
    if expected_rev != 0 && expected_rev != rev {
        return Err(KERR_NOT_READY);
    }
    let start = first as usize;
    if start > records.len() || (start == records.len() && start != 0) {
        return Err(kerr::ARGUMENT);
    }
    let limit = out_max.min(PAGE_MAX);
    if limit < HEADER_LEN {
        return Err(kerr::OVERFLOW);
    }
    let mut body = Vec::new();
    let mut count = 0usize;
    for r in &records[start..] {
        if HEADER_LEN + body.len() + 2 + r.len() > limit || count == usize::from(u16::MAX) {
            break;
        }
        body.extend_from_slice(&(r.len() as u16).to_le_bytes());
        body.extend_from_slice(r);
        count += 1;
    }
    let end = start + count;
    if count == 0 && end < records.len() {
        return Err(kerr::OVERFLOW);
    }
    let next = if end == records.len() {
        TERMINAL
    } else {
        end as u32
    };
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(&[SCHEMA, kind]);
    out.extend_from_slice(&(count as u16).to_le_bytes());
    for w in [rev, first, next] {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out.extend_from_slice(&body);
    Ok(out)
}

/// The client side: page `b` as the answer to a read of `kind` from `first` (`expected_rev` 0 on a first read), its
/// header and records. `None` when it breaks a rule (schema, kind, a zero or changed revision, an unechoed cursor, a
/// `next` that doesn't follow the records, a record cut short or bytes after the last).
#[must_use]
pub fn parse_page(
    b: &[u8],
    kind: u8,
    first: u32,
    expected_rev: u32,
) -> Option<(Header, Vec<&[u8]>)> {
    if b.len() > PAGE_MAX {
        return None;
    }
    let mut r = R(b);
    let h = Header {
        kind: {
            if r.b()? != SCHEMA {
                return None;
            }
            r.b()?
        },
        count: r.h()?,
        rev: r.w()?,
        first: r.w()?,
        next: r.w()?,
    };
    // every record has its 2-byte length: a count the bytes can't hold is refused before anything is reserved
    let progress = first.checked_add(u32::from(h.count));
    if h.kind != kind
        || h.rev == 0
        || (expected_rev != 0 && h.rev != expected_rev)
        || h.first != first
        || usize::from(h.count) > (b.len() - HEADER_LEN) / 2
        || progress.is_none_or(|p| p == TERMINAL)
        || (h.next != TERMINAL && (h.count == 0 || progress != Some(h.next)))
        // no records: only the whole of an empty collection, never a continuation
        || (h.count == 0 && first != 0)
    {
        return None;
    }
    let mut records = Vec::with_capacity(usize::from(h.count));
    for _ in 0..h.count {
        let n = r.h()?;
        records.push(r.take(usize::from(n))?);
    }
    r.end()?;
    Some((h, records))
}

/// One read of a paged kind, page by page: the records are staged here and handed over only when the last page came.
/// On `KErrNotReady` (or a damaged page) the caller drops it and starts again at record 0.
#[derive(Clone, Debug)]
pub struct Traversal {
    kind: u8,
    rev: u32,
    next: u32,
    bytes: usize,
    records: Vec<Vec<u8>>,
}

impl Traversal {
    #[must_use]
    pub fn new(kind: u8) -> Self {
        Self {
            kind,
            rev: 0,
            next: 0,
            bytes: 0,
            records: Vec::new(),
        }
    }

    /// The next read's `(first_record, expected_rev)`.
    #[must_use]
    pub fn request(&self) -> (u32, u32) {
        (self.next, self.rev)
    }

    /// Take the answer to [`Self::request`]: `Some(true)` when it was the last page, `Some(false)` for more, `None`
    /// when it is damaged or the traversal grew past [`TRAVERSAL_RECORDS_MAX`] / [`TRAVERSAL_BYTES_MAX`].
    pub fn feed(&mut self, b: &[u8]) -> Option<bool> {
        let (h, records) = parse_page(b, self.kind, self.next, self.rev)?;
        let bytes = self.bytes + b.len();
        if self.records.len() + records.len() > TRAVERSAL_RECORDS_MAX || bytes > TRAVERSAL_BYTES_MAX
        {
            return None;
        }
        self.bytes = bytes;
        self.rev = h.rev;
        self.records.extend(records.into_iter().map(Vec::from));
        self.next = h.next;
        Some(h.next == TERMINAL)
    }

    /// The whole snapshot: its revision and records (call after [`Self::feed`] said it was the last page).
    #[must_use]
    pub fn finish(self) -> (u32, Vec<Vec<u8>>) {
        (self.rev, self.records)
    }
}

/// Per-kind revisions on the server: they start at 1, change on every change that matters, and skip 0 when they wrap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revisions(pub [u32; 16]);

impl Default for Revisions {
    fn default() -> Self {
        Self([1; 16])
    }
}

impl Revisions {
    /// Kind `kind`'s revision now (1 for a kind out of range).
    #[must_use]
    pub fn get(&self, kind: u8) -> u32 {
        self.0.get(usize::from(kind)).copied().unwrap_or(1)
    }

    /// Kind `kind` changed: its new revision.
    pub fn bump(&mut self, kind: u8) -> u32 {
        let Some(r) = self.0.get_mut(usize::from(kind)) else {
            return 1;
        };
        *r = r.checked_add(1).unwrap_or(1);
        *r
    }
}

/// An app a record is about: a native app by its catalogue key, or an old app by the legacy bridge's entry id
/// (`0:b, key:s16` or `1:b, entry_id:w`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppRef {
    Native(&'static str),
    Legacy(u32),
}

impl AppRef {
    /// The catalogue's app for `key` (`None` for a key it doesn't have).
    #[must_use]
    pub fn native(key: &str) -> Option<Self> {
        apps::find(key).map(|a| AppRef::Native(a.key))
    }

    fn put(&self, w: &mut W) {
        match *self {
            AppRef::Native(key) => {
                w.b(0);
                w.s(key, ipc::KEY_MAX);
            }
            AppRef::Legacy(id) => {
                w.b(1);
                w.w(id);
            }
        }
    }

    fn get(r: &mut R<'_>) -> Option<Self> {
        match r.b()? {
            0 => Self::native(&r.s(ipc::KEY_MAX)?),
            1 => Some(AppRef::Legacy(r.w()?)),
            _ => None,
        }
    }
}

/// `clear_scope` bits (INFO 8, the confirmed CLEAR and the owner's work).
pub mod scope {
    pub const PREFERENCES: u32 = 1 << 0;
    pub const CACHE: u32 = 1 << 1;
    pub const LOGS: u32 = 1 << 2;
    pub const BROWSER: u32 = 1 << 3;
    pub const CALL_LOG: u32 = 1 << 4;
    pub const SMS: u32 = 1 << 5;
    pub const CLOCK: u32 = 1 << 6;
    pub const CALENDAR: u32 = 1 << 7;
    pub const FILES_INDEX: u32 = 1 << 8;
    /// Every defined bit; the others are reserved (zero).
    pub const ALL: u32 = (1 << 9) - 1;
}

/// INFO 8's coverage, also in an owner's report.
pub mod coverage {
    pub const UNKNOWN: u8 = 0;
    pub const MEASURING: u8 = 1;
    pub const PARTIAL: u8 = 2;
    pub const COMPLETE: u8 = 3;
    pub const FAILED: u8 = 4;
}

/// INFO 13's job states.
pub mod state {
    pub const QUEUED: u8 = 0;
    pub const RUNNING: u8 = 1;
    pub const COMMITTED: u8 = 2;
    pub const PARTIAL: u8 = 3;
    pub const FAILED: u8 = 4;
    pub const INTERRUPTED: u8 = 5;
}

/// One drive's bytes for an app: `drive:b, valid:b, installed:q, data:q, cache:q, logs:q`; `None` = unknown (its
/// validity bit clear, its count zero on the wire).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveUsage {
    /// 0 = A: … 25 = Z:.
    pub drive: u8,
    pub installed: Option<u64>,
    /// Data apart from cache, logs and installed files.
    pub data: Option<u64>,
    pub cache: Option<u64>,
    pub logs: Option<u64>,
}

/// The longest drive list (one per drive letter).
pub const DRIVES_MAX: usize = 26;

impl DriveUsage {
    fn put(&self, w: &mut W) {
        let counts = [self.installed, self.data, self.cache, self.logs];
        let valid = counts
            .iter()
            .enumerate()
            .fold(0u8, |v, (i, c)| v | u8::from(c.is_some()) << i);
        w.b(self.drive);
        w.b(valid);
        for c in counts {
            w.q(c.unwrap_or(0));
        }
    }

    fn get(r: &mut R<'_>) -> Option<Self> {
        let drive = r.b()?;
        let valid = r.b()?;
        if drive > 25 || valid > 0x0f {
            return None;
        }
        let mut c = [None; 4];
        for (i, slot) in c.iter_mut().enumerate() {
            let v = r.q()?;
            if valid & 1 << i != 0 {
                *slot = Some(v);
            } else if v != 0 {
                return None;
            }
        }
        Some(Self {
            drive,
            installed: c[0],
            data: c[1],
            cache: c[2],
            logs: c[3],
        })
    }

    fn put_list(list: &[Self], w: &mut W) {
        let list = &list[..list.len().min(DRIVES_MAX)];
        w.b(list.len() as u8);
        list.iter().for_each(|d| d.put(w));
    }

    fn get_list(r: &mut R<'_>) -> Option<Vec<Self>> {
        let n = usize::from(r.b()?);
        if n > DRIVES_MAX {
            return None;
        }
        let mut list: Vec<Self> = Vec::with_capacity(n);
        for _ in 0..n {
            let d = Self::get(r)?;
            if list.iter().any(|o| o.drive == d.drive) {
                return None;
            }
            list.push(d);
        }
        Some(list)
    }
}

/// INFO 8 (`ipc::INFO_APP_DETAIL`), one app: who it is, what it may do, how much it stores and what it can clear.
/// Producer: D4 (SYSSTART `symbian/src/settings_info.rs`: the catalogue, the EXE it finds and owners' reports;
/// capabilities wait for D8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppDetail {
    pub app: AppRef,
    /// s48.
    pub caption: String,
    /// s32.
    pub exe: String,
    /// s64, never inferred from a board.
    pub version: String,
    /// 0 unknown, 1 catalogue, 2 deployed image/manifest, 3 verified running component.
    pub identity_source: u8,
    /// 0 = unavailable.
    pub sid: u32,
    /// 0 unknown, 1 deployed E32 header, 2 kernel security info.
    pub cap_source: u8,
    /// `caps_lo | caps_hi << 32`.
    pub caps: u64,
    pub coverage: u8,
    /// 0 = unavailable.
    pub measured_utc: i64,
    /// [`scope`] bits the owner offers to clear.
    pub clear_scope: u32,
    pub drives: Vec<DriveUsage>,
}

impl AppDetail {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        self.app.put(&mut w);
        w.s(&self.caption, 48);
        w.s(&self.exe, 32);
        w.s(&self.version, 64);
        w.b(self.identity_source);
        w.w(self.sid);
        w.b(self.cap_source);
        w.w(self.caps as u32);
        w.w((self.caps >> 32) as u32);
        w.b(self.coverage);
        w.t(self.measured_utc);
        w.w(self.clear_scope);
        DriveUsage::put_list(&self.drives, &mut w);
        w.0
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        let d = Self {
            app: AppRef::get(&mut r)?,
            caption: r.s(48)?,
            exe: r.s(32)?,
            version: r.s(64)?,
            identity_source: r.max(3)?,
            sid: r.w()?,
            cap_source: r.max(2)?,
            caps: u64::from(r.w()?) | u64::from(r.w()?) << 32,
            coverage: r.max(coverage::FAILED)?,
            measured_utc: r.t()?,
            clear_scope: r.scope()?,
            drives: DriveUsage::get_list(&mut r)?,
        };
        r.end()?;
        Some(d)
    }
}

/// INFO 9 (`ipc::INFO_BATTERY_CYCLE`), a singleton: the discharge since the last full charge. Producer: D5's
/// recorder ([`crate::battery::cycle`]); empty before the first valid battery reading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatteryCycle {
    /// [`cycle`] bits.
    pub flags: u8,
    pub start_percent: Option<u8>,
    pub end_percent: Option<u8>,
    pub start_utc: i64,
    pub last_utc: i64,
    pub on_battery_s: u64,
    pub screen_on_battery_s: u64,
    pub gap_s: u64,
}

/// [`BatteryCycle::flags`].
pub mod cycle {
    pub const FULL_THEN_DISCHARGE: u8 = 1 << 0;
    pub const INCOMPLETE: u8 = 1 << 1;
    pub const CHECKPOINT_FAILED: u8 = 1 << 2;
    pub const SCREEN_VALID: u8 = 1 << 3;
}

impl BatteryCycle {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.b(self.flags);
        w.b(self.start_percent.map_or(255, |p| p.min(100)));
        w.b(self.end_percent.map_or(255, |p| p.min(100)));
        w.b(0);
        w.t(self.start_utc);
        w.t(self.last_utc);
        w.q(self.on_battery_s);
        w.q(self.screen_on_battery_s);
        w.q(self.gap_s);
        w.0
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        let pct = |v: u8| match v {
            0..=100 => Some(Some(v)),
            255 => Some(None),
            _ => None,
        };
        let c = Self {
            flags: r.max(0x0f)?,
            start_percent: pct(r.b()?)?,
            end_percent: pct(r.b()?)?,
            start_utc: {
                r.zero(1)?;
                r.t()?
            },
            last_utc: r.t()?,
            on_battery_s: r.q()?,
            screen_on_battery_s: r.q()?,
            gap_s: r.q()?,
        };
        r.end()?;
        Some(c)
    }
}

/// INFO 13 (`ipc::INFO_JOBS`): one acknowledged job and its outcome. Built: the shell's owner-work table
/// ([`super::jobs`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job {
    /// 1..=`i32::MAX`.
    pub ticket: u32,
    /// The `SETTINGS_ACTION` subkind that made it (`ACTION_CLEAR`, `ACTION_MEASURE_APP`).
    pub action: u8,
    /// [`state`].
    pub state: u8,
    pub subject: Option<AppRef>,
    pub requested_scope: u32,
    pub committed_scope: u32,
    /// 0 on success, a Symbian error on failure.
    pub result: i32,
    /// 0 until it finished.
    pub finished_utc: i64,
}

impl Job {
    /// The longest record.
    pub const MAX: usize = 12 + 2 + ipc::KEY_MAX + 20;

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.w(self.ticket);
        w.b(self.action);
        w.b(self.state);
        w.h(0);
        match self.subject {
            Some(a) => {
                w.b(1);
                a.put(&mut w);
            }
            None => w.b(0),
        }
        w.w(self.requested_scope);
        w.w(self.committed_scope);
        w.i(self.result);
        w.t(self.finished_utc);
        w.0
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        let ticket = r.w()?;
        let action = r.b()?;
        let state = r.max(state::INTERRUPTED)?;
        r.zero(2)?;
        let subject = match r.b()? {
            0 => None,
            1 => Some(AppRef::get(&mut r)?),
            _ => return None,
        };
        let j = Self {
            ticket,
            action,
            state,
            subject,
            requested_scope: r.scope()?,
            committed_scope: r.scope()?,
            result: r.i()?,
            finished_utc: r.t()?,
        };
        r.end()?;
        (ticket != 0 && ticket <= i32::MAX as u32 && j.consistent()).then_some(j)
    }

    /// Its state, result, scopes and time agree: unfinished with no outcome yet, committed with all that was asked
    /// and no error, failed with one, partial or interrupted with none or one; never more committed than asked.
    #[must_use]
    pub fn consistent(&self) -> bool {
        self.committed_scope & !self.requested_scope == 0
            && match self.state {
                state::QUEUED | state::RUNNING => {
                    self.result == 0 && self.committed_scope == 0 && self.finished_utc == 0
                }
                state::COMMITTED => {
                    self.result == 0 && self.committed_scope == self.requested_scope
                }
                state::FAILED => self.result < 0,
                _ => self.result <= 0,
            }
    }
}

/// INFO 14 (`ipc::INFO_FEATURES`), a singleton: what the shell implements for the caller, bit `n` = number `n`.
/// Probe it once per connection; per-app availability is in the records themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    pub major: u8,
    pub minor: u8,
    pub info: u64,
    pub actions: u64,
    pub opcodes: u64,
}

const fn bits(list: &[i32]) -> u64 {
    let mut b = 0u64;
    let mut i = 0;
    while i < list.len() {
        b |= 1 << list[i];
        i += 1;
    }
    b
}

impl Features {
    pub const LEN: usize = 28;

    /// What this build of SYSSTART answers: INFO 8 since D4, INFO 9 since D5; not the reserved kinds, actions and
    /// opcodes.
    pub const SHELL: Features = Features {
        major: ipc::VERSION.0,
        minor: ipc::VERSION.1,
        info: bits(&[
            ipc::INFO_BATTERY,
            ipc::INFO_NET,
            ipc::INFO_CLOCK,
            ipc::INFO_APPS,
            ipc::INFO_BATTERY_USAGE,
            ipc::INFO_RUNNING,
            ipc::INFO_CHANNELS,
            ipc::INFO_APP_DETAIL,
            ipc::INFO_BATTERY_CYCLE,
            ipc::INFO_TILES,
            ipc::INFO_JOBS,
            ipc::INFO_FEATURES,
        ]),
        actions: bits(&[
            ipc::ACTION_SIGN_IN,
            ipc::ACTION_OPEN_ENTRY,
            ipc::ACTION_PROFILE,
            ipc::ACTION_STOP,
            ipc::ACTION_RESET,
            ipc::ACTION_CLEAR,
            ipc::ACTION_MEASURE_APP,
            ipc::ACTION_EDIT_HOME,
            ipc::ACTION_EDIT_TILES,
            ipc::ACTION_INSTALL_WALLPAPER, // D7
            ipc::ACTION_LOCK_DIALOG,       // TP9
        ]),
        // every built opcode: 1–10, 13–30, 32, 33 (TP3), 34, 37 (TP7), 40–46 (11/12 are retired, 31, 35, 36, 38, 39
        // reserved)
        opcodes: (((1u64 << 31) - 1) & !1 & !(1 << 11) & !(1 << 12))
            | bits(&[
                ipc::NOTIFY_CLEAR,
                ipc::CHANNEL,
                ipc::SETTINGS_INFO,
                ipc::SETTINGS_ACTION,
                ipc::OWNER_REPORT,
                ipc::ENTRY_THUMB,
                ipc::CONTROL_FEEDBACK,
                ipc::REPAINT,
                ipc::MEDIA,
                ipc::TILE,
                ipc::WIDGET,
            ]),
    };

    #[must_use]
    pub fn has_info(&self, kind: i32) -> bool {
        (0..64).contains(&kind) && self.info & 1 << kind != 0
    }
    #[must_use]
    pub fn has_action(&self, kind: i32) -> bool {
        (0..64).contains(&kind) && self.actions & 1 << kind != 0
    }
    #[must_use]
    pub fn has_opcode(&self, op: i32) -> bool {
        (0..64).contains(&op) && self.opcodes & 1 << op != 0
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.b(self.major);
        w.b(self.minor);
        w.h(0);
        w.q(self.info);
        w.q(self.actions);
        w.q(self.opcodes);
        w.0
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        let major = r.b()?;
        let minor = r.b()?;
        r.zero(2)?;
        let f = Self {
            major,
            minor,
            info: r.q()?,
            actions: r.q()?,
            opcodes: r.q()?,
        };
        r.end()?;
        Some(f)
    }
}

/// A collection of `records` decoded by `f`; `None` when one is damaged or two have the same `id`.
pub fn decode_unique<T, K: PartialEq>(
    records: &[Vec<u8>],
    f: impl Fn(&[u8]) -> Option<T>,
    id: impl Fn(&T) -> K,
) -> Option<Vec<T>> {
    let mut out: Vec<T> = Vec::with_capacity(records.len());
    for r in records {
        let t = f(r)?;
        if out.iter().any(|o| id(o) == id(&t)) {
            return None;
        }
        out.push(t);
    }
    Some(out)
}

/// A singleton kind (9, 11, 14): `Some(None)` for the empty collection, `None` for damage or more than one record.
pub fn decode_singleton<T>(
    records: &[Vec<u8>],
    f: impl Fn(&[u8]) -> Option<T>,
) -> Option<Option<T>> {
    match records {
        [] => Some(None),
        [one] => f(one).map(Some),
        _ => None,
    }
}

/// INFO 8 as a list (one record per app).
#[must_use]
pub fn decode_details(records: &[Vec<u8>]) -> Option<Vec<AppDetail>> {
    decode_unique(records, AppDetail::decode, |d| d.app)
}

/// INFO 13 as a list (one record per ticket).
#[must_use]
pub fn decode_jobs(records: &[Vec<u8>]) -> Option<Vec<Job>> {
    decode_unique(records, Job::decode, |j| j.ticket)
}

/// The first byte of `ACTION_CLEAR`'s confirmed form; the 1.17 form is a bare app key, which never starts with it.
pub const CONFIRMED: u8 = 0xff;

/// `ACTION_CLEAR`'s confirmed form (1.20): `0xff:b, 1:b, nonce:q, app, scope:w`. `scope` is exactly what the
/// confirmation named, and INFO 8 must still offer it. Completes with a ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearRequest {
    /// Nonzero, kept across retries of the same request.
    pub nonce: u64,
    pub app: AppRef,
    pub scope: u32,
}

impl ClearRequest {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.b(CONFIRMED);
        w.b(1);
        w.q(self.nonce);
        self.app.put(&mut w);
        w.w(self.scope);
        w.0
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        if r.b()? != CONFIRMED || r.b()? != 1 {
            return None;
        }
        let c = Self {
            nonce: r.q()?,
            app: AppRef::get(&mut r)?,
            scope: r.scope()?,
        };
        r.end()?;
        (c.nonce != 0 && c.scope != 0).then_some(c)
    }
}

/// `ACTION_MEASURE_APP` (1.20): `1:b, nonce:q, app`. Completes with a ticket; measurements of the same app while
/// one is pending share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeasureRequest {
    pub nonce: u64,
    pub app: AppRef,
}

impl MeasureRequest {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.b(1);
        w.q(self.nonce);
        self.app.put(&mut w);
        w.0
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        if r.b()? != 1 {
            return None;
        }
        let m = Self {
            nonce: r.q()?,
            app: AppRef::get(&mut r)?,
        };
        r.end()?;
        (m.nonce != 0).then_some(m)
    }
}

/// `ACTION_EDIT_HOME` (1.20, D10): the request is empty (the editor needs nothing from Settings); `false` for any
/// other request, which the shell refuses with `KErrArgument`.
#[must_use]
pub const fn edit_home_request(b: &[u8]) -> bool {
    b.is_empty()
}

/// `ACTION_INSTALL_WALLPAPER` (1.22, D7), slot 1: `1:b, format:b = 1, width:h, height:h, stride:h, bytes:w` for the
/// tightly packed, top-down RGB888 picture in the file of slots 2 and 3 (540 × 1080 for the Pixel's cache). The shell
/// checks the file's length against `bytes` and the size against its own screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallpaperRequest {
    pub width: u16,
    pub height: u16,
}

impl WallpaperRequest {
    /// The only format: RGB888.
    pub const FORMAT_RGB888: u8 = 1;
    /// The encoded length.
    pub const LEN: usize = 12;
    /// The largest side accepted.
    pub const SIDE_MAX: u16 = 4096;

    /// `width × 3`; `None` when it doesn't fit 16 bits.
    #[must_use]
    pub fn stride(&self) -> Option<u16> {
        self.width.checked_mul(3)
    }

    /// `stride × height`; `None` when it doesn't fit 32 bits.
    #[must_use]
    pub fn bytes(&self) -> Option<u32> {
        u32::from(self.stride()?).checked_mul(u32::from(self.height))
    }

    /// The request; `None` for a side of 0 or past [`Self::SIDE_MAX`].
    #[must_use]
    pub fn encode(&self) -> Option<Vec<u8>> {
        let ok = |s: u16| (1..=Self::SIDE_MAX).contains(&s);
        if !ok(self.width) || !ok(self.height) {
            return None;
        }
        let mut w = W::default();
        w.0.extend_from_slice(&[1, Self::FORMAT_RGB888]);
        w.h(self.width);
        w.h(self.height);
        w.h(self.stride()?);
        w.w(self.bytes()?);
        Some(w.0)
    }

    /// `None` for another version or format, a side of 0 or past 4096, a stride that isn't `width × 3`, a length
    /// that isn't `stride × height`, or another length.
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        if r.b()? != 1 || r.b()? != Self::FORMAT_RGB888 {
            return None;
        }
        let req = Self {
            width: r.h()?,
            height: r.h()?,
        };
        let (stride, bytes) = (r.h()?, r.w()?);
        r.end()?;
        (req.encode().is_some() && Some(stride) == req.stride() && Some(bytes) == req.bytes())
            .then_some(req)
    }
}

/// `ACTION_LOCK_DIALOG` (1.22, TP9) modes.
pub mod lock_dialog {
    pub const ENROL: u8 = 0;
    pub const CHANGE: u8 = 1;
    pub const REMOVE: u8 = 2;
}

/// `ACTION_LOCK_DIALOG`'s request: `[1, mode]`.
#[must_use]
pub fn encode_lock_dialog(mode: u8) -> [u8; 2] {
    [1, mode]
}

/// The mode of an `ACTION_LOCK_DIALOG` request; `None` for another version, an unknown mode or another length.
#[must_use]
pub fn lock_dialog_request(b: &[u8]) -> Option<u8> {
    match *b {
        [1, mode] if mode <= lock_dialog::REMOVE => Some(mode),
        _ => None,
    }
}

/// The longest `SETTINGS_ACTION` request of 1.20's subkinds.
pub const REQUEST_MAX: usize = 2 + 8 + 2 + ipc::KEY_MAX + 4;

/// `OWNER_REPORT` modes.
pub const OWNER_REGISTER: i32 = 0;
pub const OWNER_CLAIM: i32 = 1;
pub const OWNER_FINISH: i32 = 2;

/// What an owner can do (`OWNER_REPORT` mode 0: `1:b, capabilities:b`).
pub mod capability {
    pub const MEASURE: u8 = 1 << 0;
    pub const CLEAR: u8 = 1 << 1;
}

/// Mode 0's payload.
#[must_use]
pub fn encode_register(capabilities: u8) -> Vec<u8> {
    alloc::vec![1, capabilities]
}

/// Mode 0's capabilities; `None` for another version, none of them, or a reserved bit.
#[must_use]
pub fn decode_register(b: &[u8]) -> Option<u8> {
    match *b {
        [1, c] if c != 0 && c & !(capability::MEASURE | capability::CLEAR) == 0 => Some(c),
        _ => None,
    }
}

/// The work an owner claimed (mode 1's reply: `1:b, work:b, subject:app, scope:w`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerWork {
    /// [`WORK_MEASURE`] or [`WORK_CLEAR`].
    pub work: u8,
    pub subject: AppRef,
    /// What to clear (0 for a measurement).
    pub scope: u32,
}

pub const WORK_MEASURE: u8 = 0;
pub const WORK_CLEAR: u8 = 1;
/// The longest mode 1 reply.
pub const OWNER_WORK_MAX: usize = 2 + 2 + ipc::KEY_MAX + 4;

impl OwnerWork {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.b(1);
        w.b(self.work);
        self.subject.put(&mut w);
        w.w(self.scope);
        w.0
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        if r.b()? != 1 {
            return None;
        }
        let o = Self {
            work: r.max(WORK_CLEAR)?,
            subject: AppRef::get(&mut r)?,
            scope: r.scope()?,
        };
        r.end()?;
        Some(o)
    }
}

/// An owner's result for a ticket (mode 2: `1:b, state:b, committed_scope:w, result:i, coverage:b, measured_utc:t,
/// version:s64, drive_count:b, drive*`). It can't name its own SID, capabilities or another app: the shell knows those.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnerReport {
    /// [`state::COMMITTED`] ..= [`state::INTERRUPTED`].
    pub state: u8,
    pub committed_scope: u32,
    pub result: i32,
    pub coverage: u8,
    pub measured_utc: i64,
    pub version: String,
    pub drives: Vec<DriveUsage>,
}

/// The longest mode 2 payload.
pub const OWNER_REPORT_MAX: usize = 2 + 4 + 4 + 1 + 8 + 65 + 1 + DRIVES_MAX * 34;

impl OwnerReport {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.b(1);
        w.b(self.state);
        w.w(self.committed_scope);
        w.i(self.result);
        w.b(self.coverage);
        w.t(self.measured_utc);
        w.s(&self.version, 64);
        DriveUsage::put_list(&self.drives, &mut w);
        w.0
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        if r.b()? != 1 {
            return None;
        }
        let state = r.b()?;
        if !(state::COMMITTED..=state::INTERRUPTED).contains(&state) {
            return None;
        }
        let o = Self {
            state,
            committed_scope: r.scope()?,
            result: r.i()?,
            coverage: r.max(coverage::FAILED)?,
            measured_utc: r.t()?,
            version: r.s(64)?,
            drives: DriveUsage::get_list(&mut r)?,
        };
        r.end()?;
        Some(o)
    }
}

/// The largest side of an `ENTRY_THUMB` thumbnail.
pub const THUMB_SIDE: u16 = 32;
/// The longest thumbnail: `w:h, h:h` and 32 × 32 `0xAARRGGBB` words.
pub const THUMB_MAX: usize = 4 + 32 * 32 * 4;

/// `ENTRY_THUMB`'s answer for a launcher icon (the `ipc::ENTRY` encoding): the same encoding, cut down to at most
/// [`THUMB_SIDE`] on its longer side (nearest pixel, aspect kept). Empty for an empty (generic) or malformed icon.
#[must_use]
pub fn thumb(icon: &[u8]) -> Vec<u8> {
    let Ok((w, h, px)) = ipc::decode_entry_icon(icon) else {
        return Vec::new();
    };
    if w == 0 {
        return Vec::new();
    }
    let step = w.max(h).div_ceil(THUMB_SIDE);
    let (tw, th) = (w.div_ceil(step), h.div_ceil(step));
    let mut out = Vec::with_capacity(4 + usize::from(tw) * usize::from(th) * 4);
    out.extend_from_slice(&tw.to_le_bytes());
    out.extend_from_slice(&th.to_le_bytes());
    for y in 0..th {
        for x in 0..tw {
            let i = (usize::from(y * step) * usize::from(w) + usize::from(x * step)) * 4;
            out.extend_from_slice(&px[i..i + 4]);
        }
    }
    out
}

/// A thumbnail's `(w, h, pixels)`; `(0, 0, [])` for the generic icon; `None` when larger than [`THUMB_SIDE`] or
/// malformed.
#[must_use]
pub fn decode_thumb(b: &[u8]) -> Option<(u16, u16, &[u8])> {
    let (w, h, px) = ipc::decode_entry_icon(b).ok()?;
    (w <= THUMB_SIDE && h <= THUMB_SIDE).then_some((w, h, px))
}

/// The shell's memory of 1.20 requests by `(caller SID, nonce)`: a retry of the same request gets the same ticket, the
/// same nonce with another request is refused. Holds the latest [`Dedup::MAX`].
#[derive(Clone, Debug, Default)]
pub struct Dedup(Vec<(u32, u64, Vec<u8>, u32)>);

impl Dedup {
    pub const MAX: usize = 32;

    /// `Ok(Some(ticket))` for a retry, `Ok(None)` for a new request.
    ///
    /// # Errors
    /// [`KERR_ALREADY_EXISTS`] when the nonce was used for another request.
    pub fn check(&self, sid: u32, nonce: u64, request: &[u8]) -> Result<Option<u32>, i32> {
        match self.0.iter().find(|(s, n, _, _)| *s == sid && *n == nonce) {
            Some((_, _, r, t)) if r == request => Ok(Some(*t)),
            Some(_) => Err(KERR_ALREADY_EXISTS),
            None => Ok(None),
        }
    }

    pub fn record(&mut self, sid: u32, nonce: u64, request: &[u8], ticket: u32) {
        if self.0.len() == Self::MAX {
            self.0.remove(0);
        }
        self.0.push((sid, nonce, Vec::from(request), ticket));
    }
}

/// A record's bytes, field by field.
#[derive(Default)]
pub(crate) struct W(pub(crate) Vec<u8>);

impl W {
    pub(crate) fn b(&mut self, v: u8) {
        self.0.push(v);
    }
    pub(crate) fn h(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn w(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn q(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn t(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// `sN`: cut to `max` bytes on a character boundary.
    pub(crate) fn s(&mut self, s: &str, max: usize) {
        let mut end = s.len().min(max);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.0.push(end as u8);
        self.0.extend_from_slice(&s.as_bytes()[..end]);
    }
}

/// Reads a record back; every getter is `None` past the end.
pub(crate) struct R<'a>(pub(crate) &'a [u8]);

impl<'a> R<'a> {
    pub(crate) fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.0.len() {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    pub(crate) fn arr<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
    pub(crate) fn b(&mut self) -> Option<u8> {
        Some(self.arr::<1>()?[0])
    }
    pub(crate) fn h(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.arr()?))
    }
    pub(crate) fn w(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.arr()?))
    }
    pub(crate) fn q(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.arr()?))
    }
    pub(crate) fn i(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.arr()?))
    }
    pub(crate) fn t(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.arr()?))
    }
    /// A byte of at most `max` (an enum or a bit set).
    pub(crate) fn max(&mut self, max: u8) -> Option<u8> {
        self.b().filter(|&v| v <= max)
    }
    /// A [`scope`] word with no reserved bit set.
    pub(crate) fn scope(&mut self) -> Option<u32> {
        self.w().filter(|&v| v & !scope::ALL == 0)
    }
    /// `n` reserved bytes, all zero.
    pub(crate) fn zero(&mut self, n: usize) -> Option<()> {
        self.take(n)?.iter().all(|&b| b == 0).then_some(())
    }
    pub(crate) fn s(&mut self, max: usize) -> Option<String> {
        let n = usize::from(self.b()?);
        if n > max {
            return None;
        }
        core::str::from_utf8(self.take(n)?).ok().map(String::from)
    }
    pub(crate) fn end(self) -> Option<()> {
        self.0.is_empty().then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wallpaper_request() {
        let pixel = WallpaperRequest {
            width: 540,
            height: 1080,
        };
        let b = pixel.encode().unwrap();
        assert_eq!(b, [1, 1, 0x1c, 2, 0x38, 4, 0x54, 6, 0x60, 0xb2, 0x1a, 0]);
        assert_eq!(
            (pixel.stride(), pixel.bytes()),
            (Some(1620), Some(1_749_600))
        );
        // the arithmetic is checked, whatever the sides
        let huge = WallpaperRequest {
            width: u16::MAX,
            height: u16::MAX,
        };
        assert_eq!((huge.stride(), huge.bytes()), (None, None));
        assert_eq!(huge.encode(), None);
        assert_eq!(WallpaperRequest::decode(&b), Some(pixel));
        for n in 0..b.len() {
            assert_eq!(WallpaperRequest::decode(&b[..n]), None, "{n}");
        }
        let mut long = b.clone();
        long.push(0);
        assert_eq!(WallpaperRequest::decode(&long), None);
        // version, format, a stride or length that doesn't fit the size
        for (at, v) in [(0, 2), (1, 2), (6, 0x55), (8, 0x61)] {
            let mut x = b.clone();
            x[at] = v;
            assert_eq!(WallpaperRequest::decode(&x), None, "byte {at} = {v}");
        }
        for (width, height) in [(0, 1), (1, 0), (4097, 1)] {
            assert_eq!(WallpaperRequest { width, height }.encode(), None);
        }
        let max = WallpaperRequest {
            width: 4096,
            height: 4096,
        };
        assert_eq!(WallpaperRequest::decode(&max.encode().unwrap()), Some(max));
    }

    #[test]
    fn the_lock_dialog_request() {
        for m in [lock_dialog::ENROL, lock_dialog::CHANGE, lock_dialog::REMOVE] {
            assert_eq!(lock_dialog_request(&encode_lock_dialog(m)), Some(m));
        }
        assert_eq!(lock_dialog_request(&[1, 3]), None);
        assert_eq!(lock_dialog_request(&[2, 0]), None);
        assert_eq!(lock_dialog_request(&[1]), None);
        assert_eq!(lock_dialog_request(&[1, 0, 0]), None);
    }
}

// SPDX-License-Identifier: GPL-3.0-only
//! D4: Dev tools as the owner of the logs it saved (protocol 1.20 owner work, docs neoui-shell/protocol.md "Owner
//! work"). Settings → Apps → Dev tools measures them and clears them (`wire::scope::LOGS`), but only the exports in
//! Dev tools' own list ([`exports`]) whose length and digest still match: another file in `\logs\` is never counted
//! or deleted, and makes the count partial; so does a card that isn't in the slot.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use neoapp::App;
use neoui_proto::devtools::exports::{self, Export};
use neoui_proto::settings::kerr;
use neoui_proto::settings_app::wire::{self, OwnerReport, capability, coverage, scope, state};

const KERR_NOT_FOUND: i32 = -1;
const KERR_PATH_NOT_FOUND: i32 = -12;
const KERR_ALREADY_EXISTS: i32 = -11;

/// Tell the shell Dev tools measures and clears its logs (after every connect).
pub fn register(app: &App) {
    if let Err(e) = app.register_owner(capability::MEASURE | capability::CLEAR) {
        dprintln!("devtools: not an owner ({})", e);
    }
}

/// `AppEvent::OwnerWork` / `AppEvent::ClearTicket`: take `ticket`, do it, report. Nothing is deleted before the claim.
pub fn work(app: &App, ticket: u32) {
    let w = match app.claim_owner_work(ticket) {
        Ok(w) => w,
        Err(e) => {
            dprintln!("devtools: work {} not claimed ({})", ticket, e);
            return;
        }
    };
    let report = match Fs::connect() {
        Ok(fs) if w.work == wire::WORK_CLEAR => clear(&fs, w.scope),
        Ok(fs) => survey(&fs).report(state::COMMITTED, 0, 0),
        Err(e) => failed(e),
    };
    dprintln!(
        "devtools: work {} ({}) state {} result {}",
        ticket,
        if w.work == wire::WORK_CLEAR {
            "clear"
        } else {
            "measure"
        },
        report.state,
        report.result
    );
    if let Err(e) = app.finish_owner_work(ticket, &report) {
        dprintln!("devtools: report {} refused ({})", ticket, e);
    }
}

/// Add an export just written and flushed to the list.
pub fn track(fs: &Fs, e: Export) -> Result<(), i32> {
    let mut list = read_list(fs)?.ok_or(kerr::CORRUPT)?;
    list.retain(|o| !o.path.eq_ignore_ascii_case(&e.path));
    list.push(e);
    write_list(fs, &list)
}

fn failed(e: i32) -> OwnerReport {
    OwnerReport {
        state: state::FAILED,
        result: e,
        coverage: coverage::FAILED,
        measured_utc: e32::time::utc_now().0,
        ..OwnerReport::default()
    }
}

/// The list; `Ok(None)` when it is damaged.
fn read_list(fs: &Fs) -> Result<Option<Vec<Export>>, i32> {
    let f = match File::open(
        fs,
        exports::LIST_PATH,
        mode::READ | mode::SHARE_READERS_ONLY,
    ) {
        Ok(f) => f,
        Err(KERR_NOT_FOUND | KERR_PATH_NOT_FOUND) => return Ok(Some(Vec::new())),
        Err(e) => return Err(e),
    };
    let size = usize::try_from(f.size()?).map_err(|_| kerr::CORRUPT)?;
    let mut b = alloc::vec![0u8; size];
    let n = f.read_at(0, &mut b)?;
    b.truncate(n);
    Ok(core::str::from_utf8(&b).ok().and_then(exports::decode))
}

fn write_list(fs: &Fs, list: &[Export]) -> Result<(), i32> {
    for dir in ["C:\\private\\", "C:\\private\\e0002005\\"] {
        match fs.mkdir(dir) {
            Ok(()) | Err(KERR_ALREADY_EXISTS) => {}
            Err(e) => return Err(e),
        }
    }
    let f = File::replace(fs, exports::LIST_PATH, mode::WRITE)?;
    f.write(exports::encode(list).as_bytes())?;
    f.flush()
}

/// The digest of file `path`.
fn file_digest(fs: &Fs, path: &str) -> Result<u64, i32> {
    let f = File::open(fs, path, mode::READ | mode::SHARE_READERS_ONLY)?;
    let mut buf = alloc::vec![0u8; 4096];
    let (mut at, mut h) = (0u64, exports::DIGEST_START);
    loop {
        let n = f.read_at64(at, &mut buf)?;
        if n == 0 {
            return Ok(h);
        }
        h = exports::digest(h, &buf[..n]);
        at += n as u64;
    }
}

enum Seen {
    /// Still the file Dev tools wrote.
    Ours,
    /// Deleted since (nothing to count).
    Gone,
    /// Its drive isn't mounted, or the file there isn't the one written: not counted, not deleted.
    Unknown,
}

/// What the logs are now.
struct Survey {
    /// `None`: the list is damaged.
    list: Option<Vec<(Export, Seen)>>,
    /// A `kernel-<ms>.txt` or `lastlog-<ms>.txt` that isn't in the list.
    lookalikes: bool,
    /// C:, and E: when a card is in.
    present: Vec<u8>,
}

fn survey(fs: &Fs) -> Survey {
    let present: Vec<u8> = [2u8, 4]
        .into_iter()
        .filter(|&d| fs.volume(i32::from(d)).is_ok())
        .collect();
    let list = read_list(fs).ok().flatten().map(|list| {
        list.into_iter()
            .map(|e| {
                let seen = match fs.entry(&e.path) {
                    _ if !exports::drive_of(&e.path).is_some_and(|d| present.contains(&d)) => {
                        Seen::Unknown
                    }
                    Err(KERR_NOT_FOUND | KERR_PATH_NOT_FOUND) => Seen::Gone,
                    Ok(f) if f.size == e.len && file_digest(fs, &e.path) == Ok(e.digest) => {
                        Seen::Ours
                    }
                    _ => Seen::Unknown,
                };
                (e, seen)
            })
            .collect::<Vec<_>>()
    });
    let lookalikes = present.iter().any(|&d| {
        let dir = format!("{}:\\logs\\", char::from(b'A' + d));
        fs.read_dir(&dir).ok().is_some_and(|names| {
            names.iter().any(|n| {
                let path = format!("{dir}{}", n.name);
                exports::is_export_name(&n.name)
                    && !list
                        .as_ref()
                        .is_some_and(|l| l.iter().any(|(e, _)| e.path.eq_ignore_ascii_case(&path)))
            })
        })
    });
    Survey {
        list,
        lookalikes,
        present,
    }
}

impl Survey {
    fn partial(&self) -> bool {
        self.lookalikes
            || self
                .list
                .as_ref()
                .is_none_or(|l| l.iter().any(|(_, s)| matches!(s, Seen::Unknown)))
    }

    /// The report: `whole` (the job's state when every log was counted), the scope committed and the result.
    fn report(&self, whole: u8, committed_scope: u32, result: i32) -> OwnerReport {
        let counted: Vec<(u8, u64)> = self
            .list
            .iter()
            .flatten()
            .filter(|(_, s)| matches!(s, Seen::Ours))
            .filter_map(|(e, _)| exports::drive_of(&e.path).map(|d| (d, e.len)))
            .collect();
        let partial = self.partial();
        OwnerReport {
            state: if partial { state::PARTIAL } else { whole },
            committed_scope: if partial { 0 } else { committed_scope },
            result: if partial { result.min(0) } else { 0 },
            coverage: if partial {
                coverage::PARTIAL
            } else {
                coverage::COMPLETE
            },
            measured_utc: e32::time::utc_now().0,
            version: version(),
            drives: exports::usage(&counted, &self.present),
        }
    }
}

/// This build ("neoui 57e1e14"), from neoui's build line.
fn version() -> String {
    neoui_proto::BUILD
        .split(' ')
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Delete the logs in the list that are still the files written; the others stay (and in the list, unless gone).
fn clear(fs: &Fs, asked: u32) -> OwnerReport {
    if asked != scope::LOGS {
        return failed(kerr::NOT_SUPPORTED);
    }
    let before = survey(fs);
    let Some(list) = before.list else {
        return failed(kerr::CORRUPT); // nothing deleted
    };
    let mut first_error = 0;
    let mut kept = Vec::new();
    for (e, seen) in list {
        match seen {
            Seen::Ours => match fs.delete(&e.path) {
                Ok(()) | Err(KERR_NOT_FOUND) => {}
                Err(err) => {
                    dprintln!("devtools: {} not deleted ({})", e.path, err);
                    first_error = if first_error == 0 { err } else { first_error };
                    kept.push(e);
                }
            },
            Seen::Gone => {}
            Seen::Unknown => kept.push(e),
        }
    }
    if let Err(err) = write_list(fs, &kept) {
        dprintln!("devtools: export list not saved ({})", err);
        first_error = if first_error == 0 { err } else { first_error };
    }
    let after = survey(fs);
    if first_error != 0 && !after.partial() {
        // the files went but the list didn't follow: what is left is counted, the job didn't commit
        return OwnerReport {
            state: state::PARTIAL,
            result: first_error,
            ..after.report(state::COMMITTED, 0, 0)
        };
    }
    after.report(state::COMMITTED, scope::LOGS, first_error)
}

/// A digest of what `save_logs` writes, chunk by chunk (`head`, then the log).
pub fn digest_of(parts: &[&[u8]]) -> (u64, u64) {
    parts
        .iter()
        .fold((0, exports::DIGEST_START), |(len, h), p| {
            (len + p.len() as u64, exports::digest(h, p))
        })
}

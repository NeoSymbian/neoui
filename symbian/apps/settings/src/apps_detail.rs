// SPDX-License-Identifier: GPL-3.0-only
//! D4 (docs neoui-shell/settings-data.md): Settings → Apps and an app's page from the shell's INFO 8 (protocol 1.20
//! `wire::AppDetail`): the apps found in the image, where each is and what it stores (the list's subtitles), its EXE
//! and version, Storage used, and Clear data for exactly the scope its owner offers, confirmed first and followed to
//! its outcome in INFO 13. Opening an app's page asks for one measurement; reading INFO 8 again (the tick, event 27)
//! never starts one. On a shell older than 1.20 the pages stay as they were (no sizes, the 1.17 Clear data).

use crate::{Asked, Ctx};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use e32::dprintln;
use neoui_proto::apps;
use neoui_proto::ipc as proto;
use neoui_proto::settings_app::page::Page;
use neoui_proto::settings_app::wire::{
    self, AppDetail, AppRef, ClearRequest, MeasureRequest, state,
};
use neoui_proto::settings_app::{app_detail, asks};
use settings_ui::{AppRow, SetState, SettingsApp};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

/// `KErrNotSupported`: the app's owner no longer offers that clear.
const KERR_NOT_SUPPORTED: i32 = -5;

#[derive(Default)]
pub struct D4 {
    /// INFO 8 as last read whole; `None`: this shell has none (older than 1.20)
    details: RefCell<Option<Vec<AppDetail>>>,
    /// the confirmed clear waiting for its outcome: its ticket and the app's name
    clearing: Cell<Option<(u32, &'static str)>>,
    nonce: Cell<u64>,
}

impl D4 {
    /// A new request's nonce (nonzero, never reused while NEOSET runs).
    fn nonce(&self) -> u64 {
        let n = self.nonce.get().wrapping_add(1);
        self.nonce.set(n);
        ((e32::time::millis() << 16) ^ n).max(1)
    }

    fn clear_scope(&self, key: &str) -> u32 {
        self.details
            .borrow()
            .iter()
            .flatten()
            .find(|d| is(d, key))
            .map_or(0, |d| d.clear_scope)
    }
}

fn is(d: &AppDetail, key: &str) -> bool {
    matches!(d.app, AppRef::Native(k) if k == key)
}

/// The app page opened: one measurement of its app (the shell measures it, or starts its owner hidden).
pub fn opened(ctx: &Ctx, key: &str) {
    let Some(a) = apps::find(key) else { return };
    let req = MeasureRequest {
        nonce: ctx.d4.nonce(),
        app: AppRef::Native(a.key),
    };
    let r = ctx
        .app
        .settings_job(proto::ACTION_MEASURE_APP, &req.encode());
    dprintln!("settings: measure {} ({:?})", a.key, r);
}

/// From the tick (each second; `read`: at once or every 2 s) and on event 27: a confirmed clear's outcome, and INFO 8
/// again while Apps or an app's page is open.
pub fn tick(ctx: &Ctx, ui: &SettingsApp, page: Page, read: bool) {
    if let Some((ticket, name)) = ctx.d4.clearing.get() {
        outcome(ctx, ticket, name);
    }
    if read && matches!(page, Page::Apps | Page::AppDetail) {
        match ctx.app.settings_records(proto::INFO_APP_DETAIL) {
            Ok((_, records)) => match wire::decode_details(&records) {
                Some(d) => *ctx.d4.details.borrow_mut() = Some(d),
                None => dprintln!("settings: INFO 8 damaged"),
            },
            Err(wire::KERR_NOT_READY) => {} // it changed while read: the last one stays, read again next time
            Err(_) => *ctx.d4.details.borrow_mut() = None, // a shell older than 1.20
        }
        show(ctx, ui);
    }
}

/// INFO 13's word on the clear `ticket` once it ended, as a note.
fn outcome(ctx: &Ctx, ticket: u32, name: &str) {
    let Some(job) = ctx
        .app
        .settings_records(proto::INFO_JOBS)
        .ok()
        .and_then(|(_, r)| wire::decode_jobs(&r))
        .and_then(|jobs| jobs.into_iter().find(|j| j.ticket == ticket))
    else {
        return;
    };
    if job.state < state::COMMITTED {
        return;
    }
    ctx.d4.clearing.set(None);
    dprintln!(
        "settings: clear {} of {} ended: state {} result {}",
        ticket,
        name,
        job.state,
        job.result
    );
    let text = match job.state {
        state::COMMITTED => alloc::format!("{name} data cleared"),
        state::PARTIAL => alloc::format!("Some {name} data couldn't be cleared"),
        state::INTERRUPTED => String::from("Clear interrupted; measure again before clearing"),
        _ => alloc::format!("Couldn't clear {name} data"),
    };
    let _ = ctx.app.note(&text, "");
}

/// INFO 8 into the pages: the listed apps the shell found in the image (every one when it could find none: it
/// couldn't look) with their subtitles, and the open app page's identity, Storage used and Clear data.
fn show(ctx: &Ctx, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    let details = ctx.d4.details.borrow();
    let find = |key: &str| details.iter().flatten().find(|d| is(d, key));
    let found_any = details.iter().flatten().any(|d| d.identity_source >= 2);
    let rows: Vec<AppRow> = apps::listed()
        .filter(|a| !found_any || find(a.key).is_some_and(|d| d.identity_source >= 2))
        .map(|a| AppRow {
            key: a.key.into(),
            name: a.name.into(),
            old: false,
            settings: a.settings.is_some(),
        })
        .collect();
    let subs: Vec<SharedString> = if details.is_some() {
        rows.iter()
            .map(|r| app_detail::list_subtitle(&r.key, find(&r.key)).into())
            .collect()
    } else {
        Vec::new()
    };
    // replaced only when changed (a new model drops a row's press)
    if !same(&s.get_apps(), &rows) {
        s.set_apps(ModelRc::new(VecModel::from(rows)));
    }
    if !same(&s.get_app_subs(), &subs) {
        s.set_app_subs(ModelRc::new(VecModel::from(subs)));
    }
    let current = s.get_app_current();
    if current.old {
        return;
    }
    let d = find(&current.key);
    s.set_app_size_shown(d.is_some());
    let (sub, size) = d.map(app_detail::storage_used).unwrap_or_default();
    s.set_app_size_sub(sub.into());
    s.set_app_size(size.into());
    s.set_app_identity(d.map(app_detail::identity).unwrap_or_default().into());
    // Clear data: the scope an owner offers (1.20), else the 1.17 clear
    let offered = d.map_or(0, |d| d.clear_scope);
    s.set_app_clear(
        if offered != 0 {
            asks::scope_what(offered)
        } else {
            asks::clear_what(&current.key)
        }
        .into(),
    );
}

fn same<T: PartialEq + Clone + 'static>(m: &ModelRc<T>, v: &[T]) -> bool {
    m.row_count() == v.len()
        && v.iter()
            .enumerate()
            .all(|(i, r)| m.row_data(i).as_ref() == Some(r))
}

/// "app-clear": ask before the confirmed clear of what app `key`'s owner offers. False when it offers none (the
/// 1.17 Clear data, or none).
pub fn ask_clear(ctx: &Ctx, key: &'static str, name: &str) -> bool {
    let scope = ctx.d4.clear_scope(key);
    if scope == 0 || asks::scope_what(scope).is_empty() {
        return false;
    }
    ctx.ask(
        asks::clear_scope(name, scope),
        "Delete",
        Asked::ClearScope(key, scope),
    );
    true
}

/// The question's Delete: the confirmed clear of exactly `scope`, the scope the question named; its outcome comes
/// as a note (`tick`).
pub fn clear(ctx: &Ctx, key: &'static str, scope: u32) {
    let req = ClearRequest {
        nonce: ctx.d4.nonce(),
        app: AppRef::Native(key),
        scope,
    };
    let name = apps::find(key).map_or(key, |a| a.name);
    match ctx.app.settings_job(proto::ACTION_CLEAR, &req.encode()) {
        Ok(ticket) => {
            dprintln!("settings: clear {} of {} ticket {}", scope, key, ticket);
            ctx.d4.clearing.set(Some((ticket, name)));
        }
        Err(e) => {
            dprintln!("settings: clear {} of {} refused ({})", scope, key, e);
            let _ = ctx.app.note(
                if e == KERR_NOT_SUPPORTED {
                    "Clear data isn't offered any more; nothing changed"
                } else {
                    "Couldn't clear the data"
                },
                "",
            );
        }
    }
    ctx.tick(true);
}

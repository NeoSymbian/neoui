// SPDX-License-Identifier: GPL-3.0-only
//! D1 on the device (neoui-proto `settings_app::row_model` has the contract): the row model's refreshes, epochs,
//! search tokens and Back frames.
//! - `publish` projects `SetState` (as the owners filled it) after every event and tick: the root's summaries, and
//!   while searching the results, from one snapshot.
//! - Search entry (the first letter typed) reads every owner the rows need, whether or not their pages were visited:
//!   the profiles, the passive link and saved list (never a scan, a join or a credential), the modem, the battery, the
//!   drives, the old apps, the clock, the kernel's console and slot; Storage's by-kind count starts on its worker
//!   (once, one generation, stopped when the app hides). The cheap ones again every 10 s while searching.
//! - A family's epoch moves on when its identities change (profiles by id and name, drive letters, apps, the link's
//!   BSSID) and on resume, so an old token never opens a replacement.
//! - A hit is resolved on release against the facts now; opening it keeps search (query, scroll, the hit) as a Back
//!   frame. An open from outside starts a new trail. The query is never logged or stored.
use crate::{Ctx, d2, store};
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use e32::dprintln;
use neoui_proto::settings_app::page::Page;
use neoui_proto::settings_app::row_model::{
    AppId, Destination, Epochs, Facts, Frame, Hit, InstanceId, RowSnapshot, Sample, Trail,
    project_all, resolve_hit,
};
use settings_ui::row_model as adapter;
use settings_ui::{AppRow, SearchHit, SetState, SettingsApp};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

#[derive(Default)]
pub struct Rows {
    generation: Cell<u32>,
    epochs: Cell<Epochs>,
    seen: RefCell<Seen>,
    snap: RefCell<RowSnapshot>,
    /// the query (in memory only), the results shown (replaced only when they change: a new model drops a row's
    /// press) and the hits their tokens stand for
    query: RefCell<String>,
    shown: RefCell<Vec<SearchHit>>,
    hits: RefCell<Vec<Hit>>,
    pub trail: RefCell<Trail>,
}

/// The identities each guarded family had at the last projection.
#[derive(Default, PartialEq)]
struct Seen {
    profiles: Vec<(u8, String)>,
    // shortcut: letters only (e32's Volume has no serial yet), so a card swapped for one of the same letter while
    // the app stays shown keeps its tokens; resume moves the epoch on. A serial in Volume closes this (D17).
    drives: Vec<char>,
    apps: Vec<AppId>,
    link: Option<[u8; 6]>,
}

impl Seen {
    fn of(f: &Facts) -> Self {
        Seen {
            profiles: f.profiles.iter().map(|p| (p.id, p.name.clone())).collect(),
            drives: f.drives.iter().map(|d| d.letter).collect(),
            apps: f.apps.iter().map(|a| a.id).collect(),
            link: f.link.as_ref().map(|l| l.bssid),
        }
    }
}

/// Project the screens' state now: a new generation, the epochs moved on for what changed.
fn project(ctx: &Ctx, s: &SetState<'_>) {
    let r = &ctx.rows;
    let g = r.generation.get().wrapping_add(1);
    r.generation.set(g);
    let sample = Sample {
        generation: g,
        sampled_ms: e32::time::millis(),
        utc: None,
    };
    let mut f = adapter::facts(s, sample, r.epochs.get());
    let seen = Seen::of(&f);
    let mut e = r.epochs.get();
    {
        let old = r.seen.borrow();
        let bump = |n: &mut u32, changed: bool| *n = n.wrapping_add(u32::from(changed));
        bump(&mut e.profiles, old.profiles != seen.profiles);
        bump(&mut e.drives, old.drives != seen.drives);
        bump(&mut e.apps, old.apps != seen.apps);
        bump(&mut e.wifi, old.link != seen.link);
    }
    r.epochs.set(e);
    *r.seen.borrow_mut() = seen;
    f.epochs = e;
    *r.snap.borrow_mut() = project_all(&f);
}

/// The results for the query, into the search page.
fn show(ctx: &Ctx, s: &SetState<'_>) {
    let r = &ctx.rows;
    let (shown, hits) = adapter::results(&r.snap.borrow(), &r.query.borrow());
    if *r.shown.borrow() != shown {
        s.set_results(ModelRc::new(VecModel::from(shown.clone())));
        *r.shown.borrow_mut() = shown;
    }
    *r.hits.borrow_mut() = hits;
}

/// After an event or a tick: the root's summaries, and the results while searching.
pub fn publish(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let s = ui.global::<SetState>();
    project(ctx, &s);
    adapter::publish(&s, &ctx.rows.snap.borrow());
    if ctx.page.get() == Page::Root && s.get_searching() {
        show(ctx, &s);
    }
}

/// Read what every row shows, from its owner (search entry, and every 10 s while searching).
fn refresh_sources(ctx: &Ctx, ui: &SettingsApp) {
    let ms = e32::time::millis();
    store::profiles(&ctx.app, ui);
    store::dates(&ctx.app, ui);
    {
        let mut w = ctx.wifi.borrow_mut();
        w.poll(&ctx.app, ui, ms, false, false); // not the page: no scan
        w.saved(ui);
    }
    ctx.modem.borrow_mut().poll(ui);
    ctx.battery(ui);
    ctx.clock(ui);
    ctx.storage.drives(ui);
    ctx.storage.show(ui);
    ctx.old_apps(ui);
    d2::refresh(ui, Page::Developer);
    d2::refresh(ui, Page::About);
}

/// The 1 s tick while the root searches: the count's progress, the owners every 10 s.
pub fn tick(ctx: &Ctx, ui: &SettingsApp, n: u32) {
    if ctx.page.get() != Page::Root || !ui.global::<SetState>().get_searching() {
        return;
    }
    ctx.storage.show(ui);
    if n % 10 == 9 {
        refresh_sources(ctx, ui);
    }
}

/// `act("search", query)`: each change of the query.
pub fn search(ctx: &Ctx, ui: &SettingsApp, query: &str) {
    let s = ui.global::<SetState>();
    let entered = ctx.rows.query.borrow().trim().is_empty() && !query.trim().is_empty();
    ctx.rows.query.replace(String::from(query));
    s.set_search_scroll(0.0);
    s.set_search_selected(-1);
    if entered {
        refresh_sources(ctx, ui);
        // the by-kind count, on its worker, once (a kept count isn't redone)
        ctx.storage.ensure();
    } else if query.is_empty() && !s.get_searching() {
        dprintln!("settings: search closed");
    }
    publish(ctx);
    // closed too: the next search starts empty
    show(ctx, &s);
}

/// `act("found", token)`: open the hit, if it still is what it was.
pub fn found(ctx: &Ctx, ui: &SettingsApp, token: &str) {
    let s = ui.global::<SetState>();
    let Ok(token) = token.parse::<u32>() else {
        return;
    };
    // the hits the screen showed when it was tapped, against the facts now
    let hits = ctx.rows.hits.borrow().clone();
    project(ctx, &s);
    let dest = resolve_hit(token, &hits, &ctx.rows.snap.borrow());
    let Ok(dest) = dest else {
        dprintln!("settings: result {} no longer available", token);
        let _ = ctx.app.note("No longer available", "");
        publish(ctx);
        return;
    };
    let key = hits
        .iter()
        .find(|h| h.token == token)
        .map(|h| h.key.clone());
    dprintln!(
        "settings: found {} of {}",
        key.as_ref().map(|k| k.alias()).unwrap_or_default(),
        hits.len()
    );
    ctx.rows.trail.borrow_mut().push(Frame::Search {
        query: ctx.rows.query.borrow().clone(),
        scroll: s.get_search_scroll(),
        selected: key,
    });
    // search closes without its ✕: the query stays in the frame, and a new search starts empty
    s.set_searching(false);
    s.set_query(SharedString::new());
    ctx.rows.query.borrow_mut().clear();
    show(ctx, &s);
    open(ctx, ui, &dest);
}

/// A destination: its instance first, then the page, its sheet, the row selected.
fn open(ctx: &Ctx, ui: &SettingsApp, d: &Destination) {
    let s = ui.global::<SetState>();
    match &d.context {
        Some(InstanceId::Profile(id)) => {
            if let Some(p) = s.get_profiles().iter().find(|p| p.id == i32::from(*id)) {
                s.set_profile_current(p);
            }
        }
        Some(InstanceId::App(AppId::Native(key))) => {
            app_current(&s, key);
        }
        Some(InstanceId::App(AppId::Legacy(e))) => {
            let key = alloc::format!("{e:08x}");
            if let Some(a) = s.get_old_apps().iter().find(|a| a.key.as_str() == key) {
                s.set_app_current(a);
            }
        }
        _ => {}
    }
    ctx.open(d.page);
    if let Some(sheet) = d.sheet {
        s.set_sheet(sheet.name().into());
    }
    if let Some(focus) = &d.focus {
        s.set_found(focus.alias().into());
    }
}

/// A ROM app's page is about `key` (Apps' rows and search). False: no such app.
pub fn app_current(s: &SetState<'_>, key: &str) -> bool {
    let Some(a) = neoui_proto::apps::find(key) else {
        return false;
    };
    s.set_app_current(AppRow {
        key: a.key.into(),
        name: a.name.into(),
        old: false,
        settings: a.settings.is_some(),
    });
    s.set_app_running(false);
    s.set_app_stoppable(false);
    true
}

/// A page opened from this app's own pages: Back comes back here.
pub fn go(ctx: &Ctx, page: Page) {
    let here = ctx.page.get();
    if here != page {
        ctx.rows
            .trail
            .borrow_mut()
            .push(Frame::Page(Destination::page(here)));
    }
    ctx.open(page);
}

/// Back below the sheets: the frame it came from (search as it was), else the page's parent.
pub fn back(ctx: &Ctx) {
    let Some(ui) = ctx.ui() else { return };
    let s = ui.global::<SetState>();
    let frame = ctx.rows.trail.borrow_mut().back(ctx.page.get());
    match frame {
        None => {}
        Some(Frame::Page(d)) => {
            dprintln!("settings: back");
            ctx.open(d.page);
        }
        Some(Frame::Search {
            query,
            scroll,
            selected,
        }) => {
            dprintln!("settings: back");
            ctx.open(Page::Root);
            s.set_query(query.as_str().into());
            ctx.rows.query.replace(query);
            s.set_searching(true);
            refresh_sources(ctx, &ui);
            publish(ctx);
            s.set_search_scroll(scroll);
            let hits = ctx.rows.hits.borrow();
            let token = hits.iter().find(|h| Some(&h.key) == selected.as_ref());
            s.set_search_selected(token.map_or(-1, |h| h.token as i32));
            dprintln!(
                "settings: search restored ({} results, selected {})",
                hits.len(),
                token.map(|h| h.key.alias()).unwrap_or_default()
            );
        }
    }
}

/// An open from outside: a new trail.
pub fn fresh(ctx: &Ctx) {
    ctx.rows.trail.borrow_mut().clear();
}

/// The app came back: what changed while it was hidden may have replaced an identity.
pub fn resume(ctx: &Ctx) {
    let mut e = ctx.rows.epochs.get();
    for n in [&mut e.profiles, &mut e.drives, &mut e.apps, &mut e.wifi] {
        *n = n.wrapping_add(1);
    }
    ctx.rows.epochs.set(e);
}

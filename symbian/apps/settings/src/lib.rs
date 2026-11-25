// SPDX-License-Identifier: GPL-3.0-only
//! NEOSET.EXE, the Settings app (docs neoui-shell/settings.md "The Settings app", SE0): an ordinary neoui app, so it
//! shows in Open apps. Its screens are settings-ui's; this is the device side:
//! - the settings store (`store`), written by this app's secure ID;
//! - Wi-Fi over its own NeoWifi session (`wifi`), the modem's status (`modem`), the sound test (`sound`);
//! - what only the shell knows (`App::settings_info`: battery history, the link's IP, the clock's sync) and the
//!   captive portal's sign-in (`App::settings_action`).
//!
//! Pages poll only while shown: a 1 s timer reads what the open page shows.
#![no_std]

extern crate alloc;

mod modem;
mod sound;
mod store;
mod wifi;

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::time::Duration;
use e32::dprintln;
use neoapp::{App, AppEvent, Page as AppPage, Query};
use neoui_proto::ipc as proto;
use neoui_proto::settings::Value;
use neoui_proto::settings::keys;
use neoui_proto::settings_app::choices::{KEYBOARDS, TIMEOUTS};
use neoui_proto::settings_app::info::{Battery, Clock};
use neoui_proto::settings_app::page::Page;
use neoui_proto::settings_app::rows;
use settings_ui::{KeyboardFocus, Kit, SearchHit, SetState, SettingsApp, Theme, live_value};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

e32::main!(main);

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("settings: failed {}", e);
            e
        }
    }
}

struct Ctx {
    app: Rc<App>,
    ui: slint::Weak<SettingsApp>,
    page: Cell<Page>,
    shown: RefCell<AppPage>,
    visible: Cell<bool>,
    ticks: Cell<u32>,
    wifi: RefCell<wifi::Wifi>,
    modem: RefCell<modem::Modem>,
    sound: sound::Sound,
    /// the city sheet's cities, as listed (a pick is an index into them)
    cities: RefCell<Vec<&'static str>>,
    /// "Reset remembered pads" asked its QUERY: the answer resets them
    asked_reset: Cell<bool>,
    /// Saved networks' Forget asked its QUERY for this network: the answer forgets it
    asked_forget: RefCell<Option<String>>,
    /// the root search's query (results are refreshed with it while searching), and the hits shown (the model is
    /// replaced only when they change: a new model drops a row's press, and with it the tap)
    query: RefCell<String>,
    hits: RefCell<Vec<SearchHit>>,
    /// the pages Back returns through (a page opened from another page, not from the root, returns there)
    trail: RefCell<Vec<Page>>,
    /// the keyboard as last asked of the shell (KeyboardFocus open, seq, mode)
    kb: Cell<(bool, i32, i32)>,
}

fn run() -> Result<(), i32> {
    let app = Rc::new(App::connect()?);
    let ui = SettingsApp::new().map_err(|_| -2)?;
    ui.global::<Theme>().set_dp(app.dp);
    let ctx = Rc::new(Ctx {
        app: app.clone(),
        ui: ui.as_weak(),
        page: Cell::new(Page::Root),
        shown: RefCell::new(AppPage::default()),
        visible: Cell::new(true),
        ticks: Cell::new(0),
        wifi: RefCell::default(),
        modem: RefCell::default(),
        sound: sound::Sound::new(),
        cities: RefCell::default(),
        asked_reset: Cell::new(false),
        asked_forget: RefCell::default(),
        kb: Cell::new((false, 0, 0)),
        query: RefCell::default(),
        hits: RefCell::default(),
        trail: RefCell::default(),
    });
    // watch first: a change between the first read and the watch would be missed
    if let Err(e) = app.watch_settings("") {
        dprintln!("settings: watch failed {}", e);
    }
    store::fill_fixed(&ui);
    store::load(&app, &ui);
    let c = ctx.clone();
    ui.global::<SetState>()
        .on_act(move |a, v| c.act(a.as_str(), v.as_str()));
    ctx.open(Page::Root);
    ui.show().map_err(|_| -2)?;
    let timer = Timer::default();
    let c = ctx.clone();
    timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        c.tick(false)
    });
    ctx.describe()?;
    app.draw()?;
    app.run(|e| {
        match e {
            // a kit layer first (none today: the sheets are the app's), then the sheet, then the page
            AppEvent::Back if neoapp::kit_back!(ui) => {}
            AppEvent::Back => ctx.back(),
            AppEvent::Open { page } => {
                let arg = app.arg().unwrap_or_default();
                ctx.trail.borrow_mut().clear();
                ctx.open(Page::opened(page, &arg));
            }
            AppEvent::Page { page } => {
                ctx.trail.borrow_mut().clear();
                ctx.open(Page::from_id(page).unwrap_or(Page::Root));
            }
            AppEvent::Settings { .. } => store::load(&app, &ui),
            // the shell's keyboard: its height (0 while a field keeps the focus: the user put it away)
            AppEvent::Keyboard { h } => {
                let kf = ui.global::<KeyboardFocus>();
                let dismissed = h == 0 && kf.get_open();
                let was = kf.get_dismissed();
                kf.set_dismissed(dismissed);
                kf.set_text_h(h as f32);
                // the user put it away: drop the request too, or a resume from Open apps would bring it back; a tap
                // on the field asks again (KeyboardFocus.seq)
                if dismissed && !was {
                    let _ = app.keyboard(proto::KEYBOARD_HIDDEN);
                }
            }
            AppEvent::Answer { i } if ctx.asked_reset.take() => {
                if i == 0 {
                    let epoch = app.setting(keys::LEGACY_PAD_EPOCH).unwrap_or(0);
                    let r = app.set_setting(keys::LEGACY_PAD_EPOCH, epoch.saturating_add(1));
                    dprintln!("settings: pads reset ({:?})", r);
                }
            }
            AppEvent::Answer { i } if ctx.asked_forget.borrow().is_some() => {
                let name = ctx.asked_forget.take().unwrap_or_default();
                if i == 0 {
                    let r = ctx.wifi.borrow_mut().forget(&name);
                    if r.is_err() {
                        let _ = app.note("Couldn't forget the network", "");
                    }
                }
                ctx.tick(true);
            }
            AppEvent::Visible(v) => {
                ctx.visible.set(v);
                if v {
                    ctx.tick(true);
                }
            }
            _ => {}
        }
        ctx.keyboard();
        ctx.describe()?;
        Ok(true)
    })
}

impl Ctx {
    fn ui(&self) -> Option<SettingsApp> {
        self.ui.upgrade()
    }

    /// The page as the shell sees it: Back stays in the app below the root and while a sheet is open.
    fn describe(&self) -> Result<(), i32> {
        let Some(ui) = self.ui() else { return Ok(()) };
        let page = self.page.get();
        let page = AppPage {
            title: page.title().into(),
            can_back: page != Page::Root
                || !ui.global::<SetState>().get_sheet().is_empty()
                || ui.global::<Kit>().get_layers() > 0,
            ..AppPage::default()
        };
        if page != *self.shown.borrow() {
            self.app.page(&page)?;
            *self.shown.borrow_mut() = page;
        }
        Ok(())
    }

    /// A field took the focus (the Wi-Fi password, the city search) or lost it: ask the shell for its keyboard, or
    /// hide it (the kit's KeyboardFocus, as Contacts and Files do).
    fn keyboard(&self) {
        let Some(ui) = self.ui() else { return };
        let kf = ui.global::<KeyboardFocus>();
        let want = (kf.get_open(), kf.get_seq(), kf.get_mode());
        let had = self.kb.get();
        if want.0 != had.0 || (want.0 && (want.1, want.2) != (had.1, had.2)) {
            self.kb.set(want);
            let mode = if want.0 {
                want.2.max(1)
            } else {
                proto::KEYBOARD_HIDDEN
            };
            let h = self.app.keyboard(mode).unwrap_or(0);
            dprintln!("settings: keyboard mode {} h {}", mode, h);
            kf.set_text_h(h as f32);
        }
    }

    fn open(&self, page: Page) {
        let Some(ui) = self.ui() else { return };
        let s = ui.global::<SetState>();
        self.page.set(page);
        s.set_page(i32::from(page.id()));
        s.set_title(page.title().into());
        s.set_sheet(SharedString::new());
        s.set_found(SharedString::new());
        ui.global::<KeyboardFocus>().set_open(false);
        dprintln!("settings: page {}", page.name());
        self.tick(true);
    }

    fn back(&self) {
        let Some(ui) = self.ui() else { return };
        let s = ui.global::<SetState>();
        if !s.get_sheet().is_empty() {
            s.set_sheet(SharedString::new());
            // the sheet's field went with it: let its keyboard go too (else it comes back on resume)
            ui.global::<KeyboardFocus>().set_open(false);
        } else if let Some(prev) = self.trail.borrow_mut().pop().or(self.page.get().parent()) {
            dprintln!("settings: back");
            self.open(prev);
        }
    }

    /// Read what the open page shows: Wi-Fi each second on its page (and every 5 s on the pages that name the link),
    /// the modem every 2 s, the battery and the clock every 5 s. `now`: at once (a page opened, the app came back).
    fn tick(&self, now: bool) {
        if !self.visible.get() {
            return;
        }
        // a field focused by a timer-driven frame (autofocus) asks for its keyboard here at the latest
        self.keyboard();
        let Some(ui) = self.ui() else { return };
        let n = if now { 0 } else { self.ticks.get() + 1 };
        self.ticks.set(n);
        let page = self.page.get();
        let ms = e32::time::millis();
        let every = |k: u32| n.is_multiple_of(k);
        match page {
            Page::Wifi => {
                self.wifi.borrow_mut().poll(&self.app, &ui, ms, true, false);
                if every(5) {
                    self.wifi.borrow_mut().saved(&ui);
                }
            }
            Page::WifiSaved if every(2) => {
                let mut w = self.wifi.borrow_mut();
                w.poll(&self.app, &ui, ms, false, false);
                w.saved(&ui);
            }
            Page::Root | Page::Connectivity if every(5) => self
                .wifi
                .borrow_mut()
                .poll(&self.app, &ui, ms, false, false),
            _ => {}
        }
        if matches!(
            page,
            Page::Root | Page::Mobile | Page::Connectivity | Page::Sounds
        ) && every(2)
        {
            self.modem.borrow_mut().poll(&ui);
        }
        if matches!(page, Page::Root | Page::Battery) && every(5) {
            self.battery(&ui);
        }
        if page == Page::DateTime && every(5) {
            store::dates(&self.app, &ui);
        }
        if matches!(page, Page::DateTime | Page::Root)
            && every(5)
            && let Some(c) = self
                .app
                .settings_info(proto::INFO_CLOCK)
                .ok()
                .and_then(|b| Clock::decode(&b))
        {
            let s = ui.global::<SetState>();
            s.set_zone(c.zone.into());
            s.set_synced(c.synced.into());
        }
        if page == Page::Root && ui.global::<SetState>().get_searching() && every(2) {
            self.search(&ui);
        }
        if let Some(text) = self.sound.changed() {
            ui.global::<SetState>().set_sound_feedback(text.into());
        }
    }

    /// The root's search results for the last query, values as they are now (again each tick while searching).
    fn search(&self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let query = self.query.borrow();
        let hits: Vec<SearchHit> = rows::search_with(&query, |r| live_value(&s, r.id))
            .into_iter()
            .map(|r| SearchHit {
                id: r.id.into(),
                title: r.title.into(),
                path: if r.on == Page::Root {
                    "Settings"
                } else {
                    r.on.title()
                }
                .into(),
                value: live_value(&s, r.id).into(),
                page: i32::from(r.opens.id()),
            })
            .collect();
        if *self.hits.borrow() != hits {
            s.set_results(ModelRc::new(VecModel::from(hits.clone())));
            *self.hits.borrow_mut() = hits;
        }
    }

    /// A page opened from this app's own pages: Back comes back here.
    fn go(&self, page: Page) {
        let here = self.page.get();
        if here != page {
            self.trail.borrow_mut().push(here);
        }
        self.open(page);
    }

    fn battery(&self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let now = neoapp::hal::battery().unwrap_or_default();
        s.set_battery_text(now.percent_text().into());
        s.set_battery_state(now.charge_text().into());
        s.set_battery_usb(now.usb_text().into());
        s.set_battery_voltage(now.voltage_text().into());
        if self.page.get() != Page::Battery {
            return;
        }
        let history = self
            .app
            .settings_info(proto::INFO_BATTERY)
            .ok()
            .and_then(|b| Battery::decode(&b));
        let storage = match &history {
            Some(h) if !h.save_error => {
                "History is saved every five minutes and before normal restart or shutdown"
            }
            _ => "History unavailable or could not be saved; live readings remain available",
        };
        s.set_battery_storage(storage.into());
        let bars = history.map(|h| h.bars).unwrap_or_default();
        s.set_battery_history(ModelRc::new(VecModel::from(bars)));
    }

    /// What a page asked for (`SetState.act`; the list is in settings-ui's state.slint).
    fn act(&self, action: &str, value: &str) {
        let Some(ui) = self.ui() else { return };
        let s = ui.global::<SetState>();
        let app = &self.app;
        match action {
            "open" => {
                if let Some(p) = value.parse().ok().and_then(Page::from_id) {
                    self.go(p);
                }
            }
            "back" => self.back(),
            "wifi-refresh" => {
                self.wifi
                    .borrow_mut()
                    .poll(app, &ui, e32::time::millis(), true, true)
            }
            "wifi-connect" => {
                let net = s.get_wifi_current();
                let failed = self
                    .wifi
                    .borrow_mut()
                    .connect(net.bssid.as_str(), s.get_password().as_str());
                // the sheet stays: it shows how the join goes ("Connecting…", "Wrong password", "Connected")
                if let Some(text) = failed {
                    s.set_wifi_message(text.into());
                }
            }
            "wifi-disconnect" => self.wifi.borrow_mut().leave(false),
            "wifi-forget" => self.wifi.borrow_mut().leave(true),
            // Saved networks: confirm first (QUERY), the answer forgets it
            "wifi-saved-forget" => {
                let q = Query {
                    text: alloc::format!(
                        "Forget {value}? The phone won't join it on its own any more; joining it again needs its password."
                    ),
                    buttons: alloc::vec![String::from("Forget"), String::from("Cancel")],
                    destructive: true,
                };
                if app.query(&q).is_ok() {
                    self.asked_forget.replace(Some(String::from(value)));
                }
            }
            "wifi-sign-in" => {
                if let Err(e) = app.settings_action(proto::ACTION_SIGN_IN) {
                    dprintln!("settings: sign in -> {}", e);
                    let _ = app.note("This network no longer asks to sign in", "");
                }
            }
            "sound" => self.sound.start(value),
            "timeout-pick" => {
                if let Some((secs, _)) = value.parse::<usize>().ok().and_then(|i| TIMEOUTS.get(i)) {
                    store::write(app, &ui, "display.timeout", &alloc::format!("{secs}"));
                }
            }
            "city-query" => {
                let (names, rows) = store::cities(value);
                *self.cities.borrow_mut() = names;
                let rows: Vec<SharedString> = rows.into_iter().map(Into::into).collect();
                s.set_cities(ModelRc::new(VecModel::from(rows)));
            }
            "zone-pick" => {
                let name = value
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| self.cities.borrow().get(i).copied());
                if let Some(name) = name {
                    store::write(app, &ui, "time.zone", name);
                }
            }
            "keyboard" => {
                let (on, i) = value.split_at(1.min(value.len()));
                let Some(i) = i.parse::<usize>().ok().filter(|&i| i < KEYBOARDS.len()) else {
                    return;
                };
                let mut all = store::keyboards(app);
                all[i] = on == "+";
                if all.iter().any(|&b| b) {
                    store::write(app, &ui, KEYBOARDS[i].0, if on == "+" { "1" } else { "0" });
                } else {
                    store::load(app, &ui); // the last language on stays on
                }
            }
            "keyboard-next" => {
                let on = store::keyboards(app);
                let next = store::pick(&on, s.get_keyboard() + 1);
                store::write(app, &ui, "keyboard.layout", &alloc::format!("{next}"));
            }
            "pad-reset" => {
                let q = Query {
                    text: "Forget the pad each old app was last used with? Each one gets its first pad again."
                        .into(),
                    buttons: alloc::vec![String::from("Reset"), String::from("Cancel")],
                    destructive: true,
                };
                self.asked_reset.set(app.query(&q).is_ok());
            }
            "clip-clear" => {
                let cleared = app.set_clipboard("");
                let _ = app.note(
                    if cleared {
                        "Clipboard cleared"
                    } else {
                        "Couldn't clear the clipboard"
                    },
                    "",
                );
            }
            "root-grid" => {
                let r = app.set_setting_raw(store::ROOT_GRID, &Value::Bool(value == "1"));
                dprintln!("settings: root grid {} ({:?})", value, r);
                store::load(app, &ui);
            }
            "search" => {
                self.query.replace(String::from(value));
                self.search(&ui);
            }
            // a search result: its page opens with the row selected, and search closes
            "found" => {
                if let Some(r) = rows::ROWS.iter().find(|r| r.id == value) {
                    s.set_searching(false);
                    s.set_query(SharedString::new());
                    self.query.borrow_mut().clear();
                    self.hits.borrow_mut().clear();
                    s.set_results(ModelRc::new(VecModel::from(Vec::<SearchHit>::new())));
                    self.go(r.opens);
                    if r.on != Page::Root {
                        s.set_found(r.id.into());
                    }
                }
            }
            // Dev tools on one of its pages ("console", "tools")
            "devtools" => {
                let r = app.open("devtools", 0, value);
                dprintln!("settings: open devtools {} ({:?})", value, r);
            }
            // another app's own settings (protocol 1.9 OPEN: Back comes back here)
            "app" => {
                let r = match value {
                    "phone" => app.open("phone", 3, ""),
                    "messages" => app.open("phone", 3, "messages"),
                    "devtools" => app.open("devtools", 0, ""),
                    _ => Ok(()),
                };
                dprintln!("settings: open {} ({:?})", value, r);
            }
            // a setting: its key and value
            key if key.contains('.') => {
                store::write(app, &ui, key, value);
                // the sheet that picked it closes (a tap on a choice closes the kit's sheet itself)
                if key == "nav.mode" || key == "time.date_format" || key.starts_with("legacy.pad.")
                {
                    s.set_sheet(SharedString::new());
                }
            }
            _ => dprintln!("settings: unknown act {} {}", action, value),
        }
        if self.ui().is_some() {
            self.keyboard();
            let _ = self.describe();
        }
    }
}

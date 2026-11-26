// SPDX-License-Identifier: GPL-3.0-only
//! NEOSET.EXE, the Settings app (docs neoui-shell/settings.md "The Settings app", SE0): an ordinary neoui app, so it
//! shows in Open apps. Its screens are settings-ui's; this is the device side:
//! - the settings store (`store`), written by this app's secure ID;
//! - Wi-Fi over its own NeoWifi session (`wifi`), the modem's status (`modem`), the sound test (`sound`);
//! - what only the shell knows (`App::settings_info`: battery history, the link's IP, the clock's sync) and the
//!   captive portal's sign-in (`App::settings_action`).
//!
//! Pages poll only while shown: a 1 s timer reads what the open page shows. Search reads every owner when it opens
//! (`row_model`, D1: the root, search and the pages show one projection).
#![no_std]

extern crate alloc;

mod appearance;
mod d2;
mod modem;
mod row_model;
mod sound;
mod storage;
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
use neoui_proto::settings_app::asks;
use neoui_proto::settings_app::choices::{KEYBOARDS, TIMEOUTS};
use neoui_proto::settings_app::info::OldApps;
use neoui_proto::settings_app::info::{Battery, Clock};
use neoui_proto::settings_app::page::Page;
use neoui_proto::sound_profile;
use settings_ui::{AppRow, KeyboardFocus, Kit, SetState, SettingsApp, Theme, ZoneRow};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

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

/// A QUERY waiting for its answer.
enum Asked {
    /// Delete this added sound profile
    ProfileDelete(u8),
    /// "Reset remembered pads"
    PadReset,
    /// Saved networks' Forget, for this network (raw SSID)
    Forget(Vec<u8>),
    /// Apps → an app → Stop / Clear data, for this app's key (SE4a)
    Stop(&'static str),
    Clear(&'static str),
    /// Reset → network settings or settings (`proto::RESET_*`)
    Reset(u8),
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
    storage: storage::Storage,
    /// the city sheet's cities, as listed (a pick is an index into them)
    cities: RefCell<Vec<&'static str>>,
    /// What the open QUERY asks (the shell keeps one per app, the latest replaces it): its answer does that
    asked: RefCell<Option<Asked>>,
    /// D1: the row model's projection, search and the frames Back returns through
    rows: row_model::Rows,
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
        storage: storage::Storage::new(),
        cities: RefCell::default(),
        asked: RefCell::default(),
        kb: Cell::new((false, 0, 0)),
        rows: row_model::Rows::default(),
    });
    // watch first: a change between the first read and the watch would be missed
    if let Err(e) = app.watch_settings("") {
        dprintln!("settings: watch failed {}", e);
    }
    store::fill_fixed(&ui);
    let rom: Vec<AppRow> = neoui_proto::apps::listed()
        .map(|a| AppRow {
            key: a.key.into(),
            name: a.name.into(),
            old: false,
            settings: a.settings.is_some(),
        })
        .collect();
    ui.global::<SetState>()
        .set_apps(ModelRc::new(VecModel::from(rom)));
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
                row_model::fresh(&ctx);
                ctx.open(Page::opened(page, &arg).from_outside());
            }
            AppEvent::Page { page } => {
                row_model::fresh(&ctx);
                ctx.open(Page::from_id(page).unwrap_or(Page::Root).from_outside());
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
            AppEvent::Answer { i } => match ctx.asked.take() {
                Some(Asked::PadReset) if i == 0 => {
                    let epoch = app.setting(keys::LEGACY_PAD_EPOCH).unwrap_or(0);
                    let r = app.set_setting(keys::LEGACY_PAD_EPOCH, epoch.saturating_add(1));
                    dprintln!("settings: pads reset ({:?})", r);
                }
                Some(Asked::ProfileDelete(id)) if i == 0 => {
                    if ctx.profile(sound_profile::Command::Delete(id)) {
                        ctx.back();
                    }
                }
                Some(Asked::Forget(ssid)) => {
                    if i == 0 && ctx.wifi.borrow_mut().forget(&ssid).is_err() {
                        let _ = app.note("Couldn't forget the network", "");
                    }
                    ctx.tick(true);
                }
                Some(Asked::Stop(key)) if i == 0 => {
                    let r = app.settings_action_with(proto::ACTION_STOP, key.as_bytes());
                    dprintln!("settings: stop {} ({:?})", key, r);
                    if r.is_err() {
                        let _ = app.note("Couldn't stop the app", "");
                    }
                    ctx.tick(true);
                }
                Some(Asked::Clear(key)) if i == 0 => {
                    let r = match key {
                        "messages" => ctx.modem.borrow_mut().clear(true),
                        "phone" => ctx.modem.borrow_mut().clear(false),
                        _ => app.settings_action_with(proto::ACTION_CLEAR, key.as_bytes()),
                    };
                    dprintln!("settings: clear data of {} ({:?})", key, r);
                    let _ = app.note(
                        match (&r, key) {
                            // the app clears its own after it reads the event: it logs what it did
                            (Ok(()), "clock") => "Clock is clearing its data",
                            (Ok(()), _) => "Data cleared",
                            (Err(-18), _) => "Busy; try again in a moment", // KErrNotReady
                            _ => "Couldn't clear the data",
                        },
                        "",
                    );
                    ctx.tick(true);
                }
                Some(Asked::Reset(what)) if i == 0 => {
                    let r = app.settings_action_with(proto::ACTION_RESET, &[what]);
                    dprintln!("settings: reset {} ({:?})", what, r);
                    if r.is_err() {
                        let _ = app.note("Couldn't finish the reset; the phone didn't restart", "");
                    }
                }
                _ => {}
            },
            AppEvent::Visible(v) => {
                ctx.visible.set(v);
                if v {
                    row_model::resume(&ctx);
                    if ctx.page.get() == Page::Storage {
                        ctx.storage.ensure();
                    }
                    ctx.tick(true);
                } else {
                    ctx.sound.stop(); // a tone's preview ends with the app hidden
                    ctx.storage.stop(); // hidden: no scan in the background
                }
            }
            _ => {}
        }
        ctx.keyboard();
        row_model::publish(&ctx);
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
        // a tone's preview stops when the page changes (Back included)
        self.sound.stop();
        // Storage's scan runs only on its page, shown (SE3b); a kept count isn't redone
        if page == Page::Storage && self.visible.get() {
            self.storage.ensure();
        } else {
            self.storage.stop();
        }
        if page == Page::Profile {
            // the profile a sheet chose, or (from search) the active one
            store::profiles(&self.app, &ui);
            s.set_title(s.get_profile_current().name);
        }
        if page == Page::AppDetail {
            s.set_title(s.get_app_current().name);
            s.set_app_clear(asks::clear_what(s.get_app_current().key.as_str()).into());
        }
        self.tick(true);
    }

    fn back(&self) {
        let Some(ui) = self.ui() else { return };
        let s = ui.global::<SetState>();
        if !s.get_sheet().is_empty() {
            s.set_sheet(SharedString::new());
            // the sheet's field went with it: let its keyboard go too (else it comes back on resume)
            ui.global::<KeyboardFocus>().set_open(false);
        } else {
            row_model::back(self);
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
        if now {
            d2::refresh(&ui, page);
        }
        match page {
            Page::Wifi => {
                self.wifi.borrow_mut().poll(&self.app, &ui, ms, true, false);
                if every(5) {
                    self.wifi.borrow_mut().saved(&ui);
                }
            }
            Page::WifiSaved if every(2) => {
                let mut w = self.wifi.borrow_mut();
                if every(4) {
                    w.poll(&self.app, &ui, ms, false, false); // Connected / in range
                }
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
        if matches!(page, Page::Root | Page::Storage) && every(5) {
            self.storage.drives(&ui);
        }
        if page == Page::Storage {
            self.storage.show(&ui);
        }
        if page == Page::BatteryUsage && every(5) {
            self.battery_usage(&ui);
        }
        if page == Page::Apps && every(5) {
            self.old_apps(&ui);
        }
        if page == Page::AppDetail && every(2) {
            self.running(&ui);
        }
        if page == Page::DateTime && every(5) {
            store::dates(&self.app, &ui);
        }
        if matches!(page, Page::DateTime | Page::Root) && every(5) {
            self.clock(&ui);
        }
        row_model::tick(self, &ui, n);
        if let Some(text) = self.sound.changed() {
            ui.global::<SetState>().set_sound_feedback(text.into());
        }
        row_model::publish(self);
    }

    /// The zone and when the clock last synced (the shell's `INFO_CLOCK`).
    fn clock(&self, ui: &SettingsApp) {
        if let Some(c) = self
            .app
            .settings_info(proto::INFO_CLOCK)
            .ok()
            .and_then(|b| Clock::decode(&b))
        {
            let s = ui.global::<SetState>();
            s.set_zone(c.zone.into());
            s.set_synced(c.synced.into());
        }
    }

    /// A sound profile command for the shell's reducer; a refusal shows as a note. True when it was done.
    fn profile(&self, cmd: sound_profile::Command) -> bool {
        let r = self
            .app
            .settings_action_with(proto::ACTION_PROFILE, &cmd.encode());
        dprintln!("settings: profile {:?} ({:?})", cmd, r);
        let text = match r {
            Ok(()) => None,
            Err(-9) => Some("No room for another profile"), // KErrOverflow
            Err(_) => Some("Couldn't change the profile"),
        };
        if let Some(text) = text {
            let _ = self.app.note(text, "");
        }
        if let Some(ui) = self.ui() {
            store::profiles(&self.app, &ui);
        }
        r.is_ok()
    }

    /// The old apps the legacy bridge registered (INFO_APPS), into the Apps page; a 1.13 shell can't say.
    fn old_apps(&self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let list = self
            .app
            .settings_info(proto::INFO_APPS)
            .map(|b| OldApps::decode(&b));
        s.set_old_ok(!matches!(list, Err(_) | Ok(None)));
        let list = list.ok().flatten().unwrap_or_default();
        let rows: Vec<AppRow> = list
            .apps
            .iter()
            .map(|(id, caption)| AppRow {
                key: alloc::format!("{id:08x}").into(),
                name: caption.as_str().into(),
                old: true,
                settings: false,
            })
            .collect();
        s.set_old_total(i32::from(list.total));
        // replaced only when it changed (a new model drops a row's press)
        let same = s.get_old_apps().row_count() == rows.len()
            && rows
                .iter()
                .enumerate()
                .all(|(i, r)| s.get_old_apps().row_data(i).as_ref() == Some(r));
        if !same {
            s.set_old_apps(ModelRc::new(VecModel::from(rows)));
        }
    }

    /// The app page's Stop row (SE4a): whether app-current runs now (`INFO_RUNNING`), and whether it can be stopped
    /// (never a resident, never Settings; an older shell can't say, so no row).
    fn running(&self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let key = s.get_app_current().key;
        let list = self
            .app
            .settings_info(proto::INFO_RUNNING)
            .ok()
            .and_then(|b| neoui_proto::settings_app::info::decode_running(&b));
        let me = list
            .as_ref()
            .and_then(|l| l.iter().find(|(k, _)| k.as_str() == key.as_str()));
        s.set_app_running(me.is_some());
        s.set_app_stoppable(
            list.is_some()
                && !me.is_some_and(|(_, resident)| *resident)
                && !matches!(
                    key.as_str(),
                    "phone" | "messages" | neoui_proto::apps::SETTINGS
                ),
        );
    }

    /// Ask the shell's dialog first (`confirm` in red, then Cancel); its answer does `asked`. A dialog the shell
    /// refuses shows as a note, never as nothing at all.
    fn ask(&self, text: String, confirm: &str, asked: Asked) {
        let q = Query {
            text,
            buttons: alloc::vec![String::from(confirm), String::from("Cancel")],
            destructive: true,
        };
        match self.app.query(&q) {
            Ok(()) => {
                self.asked.replace(Some(asked));
            }
            Err(e) => {
                dprintln!("settings: the question was refused ({})", e);
                let _ = self.app.note("Couldn't ask; nothing changed", "");
            }
        }
    }

    /// A page opened from this app's own pages: Back comes back here.
    fn go(&self, page: Page) {
        row_model::go(self, page);
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

    /// Battery usage (SE3a): the shell's `INFO_BATTERY_USAGE` as text; "No data yet" before the first reading or when
    /// the shell can't say (an older shell, a damaged record).
    fn battery_usage(&self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let usage = self
            .app
            .settings_info(proto::INFO_BATTERY_USAGE)
            .ok()
            .and_then(|b| neoui_proto::settings_app::info::decode_usage(&b))
            .flatten();
        s.set_usage_known(usage.is_some());
        if let Some(t) = usage.map(|u| u.text()) {
            s.set_usage_title(t.title.into());
            s.set_usage_levels(t.levels.into());
            s.set_usage_on_battery(t.on_battery.into());
            s.set_usage_gaps(t.gaps.into());
        }
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
                let Some(ssid) = value
                    .parse()
                    .ok()
                    .and_then(|i| self.wifi.borrow().saved_ssid(i))
                else {
                    return;
                };
                // the name as shown, at most 32 characters (3 bytes each lossy): within the QUERY's 160 bytes
                let name: String = String::from_utf8_lossy(&ssid).chars().take(32).collect();
                let q = Query {
                    text: alloc::format!("Forget {name}? Joining it again will need its password."),
                    buttons: alloc::vec![String::from("Forget"), String::from("Cancel")],
                    destructive: true,
                };
                match app.query(&q) {
                    Ok(()) => {
                        self.asked.replace(Some(Asked::Forget(ssid)));
                    }
                    Err(e) => dprintln!("settings: forget query -> {}", e),
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
                let (names, offsets) = store::cities(value);
                let here = store::zone(app);
                let rows: Vec<ZoneRow> = names
                    .iter()
                    .zip(offsets)
                    .map(|(n, o)| ZoneRow {
                        name: (*n).into(),
                        offset: o.into(),
                        current: *n == here,
                    })
                    .collect();
                *self.cities.borrow_mut() = names;
                s.set_cities(ModelRc::new(VecModel::from(rows)));
            }
            "zone-pick" => {
                let name = value
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| self.cities.borrow().get(i).copied());
                // a city is the phone's zone only while the automatic zone is off
                if let Some(name) = name
                    && !s.get_time_auto_zone()
                {
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
                if app.query(&q).is_ok() {
                    self.asked.replace(Some(Asked::PadReset));
                }
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
            "search" => row_model::search(self, &ui, value),
            // a search result: its page opens on its instance, with the row selected; Back returns to the results
            "found" => row_model::found(self, &ui, value),
            // Dev tools on one of its pages ("console", "tools")
            "devtools" => {
                let r = app.open("devtools", 0, value);
                dprintln!("settings: open devtools {} ({:?})", value, r);
            }
            // another app's own settings (protocol 1.9 OPEN: Back comes back here)
            "app" => {
                let r = neoui_proto::apps::find(value)
                    .and_then(|a| a.settings)
                    .map(|(key, page, arg)| app.open(key, page, arg));
                dprintln!("settings: open {} settings ({:?})", value, r);
            }
            // the active profile's ✎ (set2-sounds' card): its page
            "profile-open" => {
                let id: i32 = value.parse().unwrap_or(-1);
                if let Some(p) = s.get_profiles().iter().find(|p| p.id == id) {
                    s.set_profile_current(p);
                    self.go(Page::Profile);
                }
            }
            // sound profiles (SE2a): the shell's reducer does them (ACTION_PROFILE)
            "profile-choose" => {
                if let Ok(id) = value.parse::<u8>() {
                    let _ = self.profile(sound_profile::Command::Choose(id));
                }
            }
            "profile-set" => {
                let p = s.get_profile_current();
                let v: Vec<i32> = value.split(',').filter_map(|x| x.parse().ok()).collect();
                if let ([ring, volume, vibrate], Ok(id)) = (v.as_slice(), u8::try_from(p.id)) {
                    let _ = self.profile(sound_profile::Command::Set {
                        id,
                        ring: *ring != 0,
                        volume: (*volume).clamp(0, 10) as u8,
                        vibrate: *vibrate != 0,
                    });
                }
            }
            // (SE2b) a tone from its sheet: the profile takes it, then it plays once (at the profile's volume)
            "profile-tone" => {
                let p = s.get_profile_current();
                let (which, t) = value.split_once(',').unwrap_or(("", ""));
                if let (Ok(id), Ok(t)) = (u8::try_from(p.id), t.parse::<u8>()) {
                    let (mut ring, mut message) = (p.ringtone as u8, p.message_tone as u8);
                    if which == "r" {
                        ring = t;
                    } else {
                        message = t;
                    }
                    if self.profile(sound_profile::Command::SetTones { id, ring, message }) {
                        self.sound.preview(t, p.volume.clamp(0, 10) as u8);
                    }
                }
            }
            // a tone's ▶ on Ringtone (set2-ringtone): hear it at the profile's ring volume, without choosing it
            "tone-preview" => {
                if let Ok(t) = value.parse::<u8>() {
                    let p = s.get_profile_current();
                    self.sound.preview(t, p.volume.clamp(0, 10) as u8);
                }
            }
            "profile-add" => {
                let _ = self.profile(sound_profile::Command::Add(String::from(value.trim())));
            }
            "profile-rename" => {
                if let Ok(id) = u8::try_from(s.get_profile_current().id)
                    && self.profile(sound_profile::Command::Rename(
                        id,
                        String::from(value.trim()),
                    ))
                {
                    s.set_title(value.trim().into());
                }
            }
            "profile-delete" => {
                let p = s.get_profile_current();
                let q = Query {
                    text: alloc::format!("Delete the profile {}?", p.name),
                    buttons: alloc::vec![String::from("Delete"), String::from("Cancel")],
                    destructive: true,
                };
                if let (Ok(()), Ok(id)) = (app.query(&q), u8::try_from(p.id)) {
                    self.asked.replace(Some(Asked::ProfileDelete(id)));
                }
            }
            // an app itself (Apps page)
            "storage-recount" => self.storage.recount(),
            "console" => d2::set_console(&ui, value == "1"),
            // a ROM app's page (SE4a)
            "app-detail" => {
                if row_model::app_current(&s, value) {
                    self.go(Page::AppDetail);
                }
            }
            "app-stop" => {
                if let Some(a) = neoui_proto::apps::find(value) {
                    self.ask(asks::stop(a.name), "Stop", Asked::Stop(a.key));
                }
            }
            "app-clear" => {
                if let Some(a) = neoui_proto::apps::find(value)
                    && !asks::clear_what(a.key).is_empty()
                {
                    self.ask(asks::clear(a.name, a.key), "Delete", Asked::Clear(a.key));
                }
            }
            "reset" => {
                let what = match value {
                    "network" => proto::RESET_NETWORK,
                    "settings" => proto::RESET_SETTINGS,
                    _ => return,
                };
                self.ask(asks::reset(what).into(), "Reset", Asked::Reset(what));
            }
            "app-open" => {
                let r = app.open(value, 0, "");
                dprintln!("settings: open app {} ({:?})", value, r);
            }
            // an old app, by its id: the shell has the legacy bridge open it (INFO_APPS, ACTION_OPEN_ENTRY)
            "old-open" => {
                let r = u32::from_str_radix(value, 16).map(|id| {
                    app.settings_action_with(proto::ACTION_OPEN_ENTRY, &id.to_le_bytes())
                });
                dprintln!("settings: open old app {} ({:?})", value, r);
                if !matches!(r, Ok(Ok(()))) {
                    let _ = app.note("Couldn't open the app", "");
                }
            }
            // airplane mode on also turns Wi-Fi and Bluetooth off, once; either can be turned back on while it stays
            // on (set2-airplane's note). The Offline profile does the same through the shell's reducer.
            "radio.airplane" => {
                store::write(app, &ui, action, value);
                if value == "1" {
                    store::write(app, &ui, "radio.wifi", "0");
                    store::write(app, &ui, "radio.bt", "0");
                }
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
            row_model::publish(self);
            let _ = self.describe();
        }
    }
}

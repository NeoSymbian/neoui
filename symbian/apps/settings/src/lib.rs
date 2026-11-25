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
use neoui_proto::settings::keys;
use neoui_proto::settings_app::choices::{KEYBOARDS, TIMEOUTS};
use neoui_proto::settings_app::info::{Battery, Clock};
use neoui_proto::settings_app::page::Page;
use settings_ui::{Kit, SetState, SettingsApp, Theme};
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
    });
    store::fill_fixed(&ui);
    store::load(&app, &ui);
    if let Err(e) = app.watch_settings("") {
        dprintln!("settings: watch failed {}", e);
    }
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
                ctx.open(Page::opened(page, &arg));
            }
            AppEvent::Page { page } => ctx.open(Page::from_id(page).unwrap_or(Page::Root)),
            AppEvent::Settings { .. } => store::load(&app, &ui),
            AppEvent::Answer { i } if ctx.asked_reset.take() => {
                if i == 0 {
                    let epoch = app.setting(keys::LEGACY_PAD_EPOCH).unwrap_or(0);
                    let r = app.set_setting(keys::LEGACY_PAD_EPOCH, epoch.saturating_add(1));
                    dprintln!("settings: pads reset ({:?})", r);
                }
            }
            AppEvent::Visible(v) => {
                ctx.visible.set(v);
                if v {
                    ctx.tick(true);
                }
            }
            _ => {}
        }
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

    fn open(&self, page: Page) {
        let Some(ui) = self.ui() else { return };
        let s = ui.global::<SetState>();
        self.page.set(page);
        s.set_page(i32::from(page.id()));
        s.set_title(page.title().into());
        s.set_sheet(SharedString::new());
        dprintln!("settings: page {}", page.name());
        self.tick(true);
    }

    fn back(&self) {
        let Some(ui) = self.ui() else { return };
        let s = ui.global::<SetState>();
        if !s.get_sheet().is_empty() {
            s.set_sheet(SharedString::new());
        } else if let Some(parent) = self.page.get().parent() {
            dprintln!("settings: back");
            self.open(parent);
        }
    }

    /// Read what the open page shows: Wi-Fi each second on its page (and every 5 s on the pages that name the link),
    /// the modem every 2 s, the battery and the clock every 5 s. `now`: at once (a page opened, the app came back).
    fn tick(&self, now: bool) {
        if !self.visible.get() {
            return;
        }
        let Some(ui) = self.ui() else { return };
        let n = if now { 0 } else { self.ticks.get() + 1 };
        self.ticks.set(n);
        let page = self.page.get();
        let ms = e32::time::millis();
        let every = |k: u32| n.is_multiple_of(k);
        match page {
            Page::Wifi => self.wifi.borrow_mut().poll(&self.app, &ui, ms, true, false),
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
        if page == Page::DateTime
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
        if let Some(text) = self.sound.changed() {
            ui.global::<SetState>().set_sound_feedback(text.into());
        }
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
                    self.open(p);
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
                if let Some(text) = failed {
                    s.set_wifi_message(text.into());
                } else {
                    s.set_sheet(SharedString::new());
                    s.set_password(SharedString::new());
                }
            }
            "wifi-disconnect" => self.wifi.borrow_mut().leave(false),
            "wifi-forget" => self.wifi.borrow_mut().leave(true),
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
            let _ = self.describe();
        }
    }
}

// SPDX-License-Identifier: GPL-3.0-only
//! The Clock app's state: what every page shows, loaded from and saved to the settings store (the `clock.*` keys
//! of `neoui_proto::settings::registry`), so it survives the app closing and the phone restarting.

use alloc::vec::Vec;
use e32::dprintln;
use neoapp::App;
use neoui_proto::clock::Local;
use neoui_proto::clock::alarm::{Alarm, Alarms};
use neoui_proto::clock::stopwatch::Stopwatch;
use neoui_proto::clock::timer::Timer;
use neoui_proto::clock::world::Cities;
use neoui_proto::settings::{Bytes, Key, keys};

/// The pages, in tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Clock = 0,
    Alarms = 1,
    World = 2,
    Timer = 3,
    Stopwatch = 4,
}

impl Tab {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Tab::Alarms,
            2 => Tab::World,
            3 => Tab::Timer,
            4 => Tab::Stopwatch,
            _ => Tab::Clock,
        }
    }
}

/// What covers the tabs, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    None,
    SetTime,
    Alarm,
    Cities,
    Ringing,
}

impl View {
    /// The name `ClockState.view` uses.
    pub fn name(self) -> &'static str {
        match self {
            View::None => "",
            View::SetTime => "set-time",
            View::Alarm => "alarm",
            View::Cities => "cities",
            View::Ringing => "ringing",
        }
    }
}

/// A piece of state saved under its own key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Tab,
    Alarms,
    Cities,
    Timer,
    Stopwatch,
}

/// An alarm being edited: a copy of an existing one (`Some(id)`) or a new one.
#[derive(Clone, Debug)]
pub struct Draft {
    pub id: Option<u16>,
    pub alarm: Alarm,
}

/// Everything the pages show.
pub struct Model {
    pub tab: Tab,
    pub view: View,
    pub h24: bool,
    pub alarms: Alarms,
    pub cities: Cities,
    pub timer: Timer,
    pub stopwatch: Stopwatch,
    /// The "Set time" editor's fields.
    pub set_time: Local,
    /// The alarm editor's draft.
    pub draft: Option<Draft>,
    /// The alarm the ringing view shows.
    pub ringing: Option<u16>,
}

impl Model {
    /// Loads every saved part (a missing or unreadable one starts fresh, logged).
    pub fn load(app: &App) -> Self {
        let model = Self {
            tab: Tab::from_index(app.setting(keys::CLOCK_TAB).unwrap_or(0)),
            view: View::None,
            h24: app.setting(keys::CLOCK_24H).unwrap_or(true),
            alarms: blob(app, keys::CLOCK_ALARMS, Alarms::decode).unwrap_or_default(),
            cities: blob(app, keys::CLOCK_CITIES, Cities::decode).unwrap_or_default(),
            timer: blob(app, keys::CLOCK_TIMER, Timer::decode).unwrap_or_default(),
            stopwatch: blob(app, keys::CLOCK_STOPWATCH, Stopwatch::decode).unwrap_or_default(),
            set_time: Local {
                year: 2026,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                tz: 0,
            },
            draft: None,
            ringing: None,
        };
        dprintln!(
            "clock: state loaded (tab {}, {} alarms, {} cities, timer {:?}, stopwatch running {})",
            model.tab as i32,
            model.alarms.as_slice().len(),
            model.cities.as_slice().len(),
            model.timer.phase,
            model.stopwatch.is_running()
        );
        model
    }

    /// Reads the parts other processes may change (the UI server's settings, alarms switched off after ringing).
    pub fn reload_shared(&mut self, app: &App) {
        self.h24 = app.setting(keys::CLOCK_24H).unwrap_or(self.h24);
        if let Some(alarms) = blob(app, keys::CLOCK_ALARMS, Alarms::decode) {
            self.alarms = alarms;
        }
    }

    /// Saves one part (a failure is logged; the state stays as it is in memory).
    pub fn save(&self, app: &App, part: Part) {
        let r = match part {
            Part::Tab => app.set_setting(keys::CLOCK_TAB, self.tab as i32),
            Part::Alarms => app.set_setting(keys::CLOCK_ALARMS, Bytes(self.alarms.encode())),
            Part::Cities => app.set_setting(keys::CLOCK_CITIES, Bytes(self.cities.encode())),
            Part::Timer => app.set_setting(keys::CLOCK_TIMER, Bytes(self.timer.encode())),
            Part::Stopwatch => {
                app.set_setting(keys::CLOCK_STOPWATCH, Bytes(self.stopwatch.encode()))
            }
        };
        if let Err(e) = r {
            dprintln!("clock: saving {:?} failed ({})", part, e);
        }
    }
}

/// A saved blob decoded: `None` when unset (empty), unreadable or damaged (logged).
fn blob<T, E: core::fmt::Display>(
    app: &App,
    key: Key<Bytes>,
    decode: fn(&[u8]) -> Result<T, E>,
) -> Option<T> {
    let bytes: Vec<u8> = match app.setting(key) {
        Ok(Bytes(b)) => b,
        Err(e) => {
            dprintln!("clock: cannot read {} ({})", key.name(), e);
            return None;
        }
    };
    if bytes.is_empty() {
        return None;
    }
    decode(&bytes)
        .map_err(|e| dprintln!("clock: {} unreadable ({}), starting fresh", key.name(), e))
        .ok()
}

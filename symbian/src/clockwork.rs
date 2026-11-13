// SPDX-License-Identifier: GPL-3.0-only
//! The shell's side of the Clock: the time shown (the status bar and the home clock widget), alarms that ring
//! whether or not the Clock app runs, and the clock kept across restarts ([`clock::Keeper`]).
//!
//! Alarms live in the Clock app's setting `clock.alarms`; the server reads them straight from its settings store
//! with the same model the app uses (`neoui_proto::clock::alarm`), so "due" means the same to both.

use crate::clock;
use crate::host::Host;
use crate::settings::{self, Settings};
use alloc::format;
use alloc::string::String;
use e32::{dprintln, time};
use neoui_proto::clock::alarm::{Alarms, Minute};
use neoui_proto::clock::face::{Face, LocalTime};
use neoui_proto::clock::{datetime, world};
use neoui_proto::ipc::AppEvent;
use neoui_proto::settings::{Caller, Value, keys};
use neoui_render::Renderer;

/// The Clock app's screen key (`host::APPS`).
const CLOCK_APP: &str = "clock";

pub struct Clockwork {
    keeper: clock::Keeper,
    /// What the face shows now (the time, the second when the status bar shows it, the 24-hour setting, the
    /// settings' sequence number and the offset), so it is redrawn only when it changes.
    shown: Option<(LocalTime, u32, bool, u32, i32)>,
    /// Where the time last came from (Settings -> Date & time), set by the network clock.
    pub synced: String,
    /// The last minute alarms were checked for.
    checked: Option<Minute>,
    /// The last minute the zone's rule was applied.
    zone_checked: Option<Minute>,
    /// An alarm that rang, until the Clock app has been told.
    ringing: Option<u16>,
}

impl Clockwork {
    /// Sets the clock from what was saved ([`clock::Keeper::boot`]).
    pub fn boot() -> Self {
        Self {
            keeper: clock::Keeper::boot(),
            shown: None,
            synced: String::from("Not yet"),
            checked: None,
            zone_checked: None,
            ringing: None,
        }
    }

    /// Once per frame: keeps the clock, updates the face, rings due alarms.
    pub fn frame(&mut self, ui: &mut Renderer, host: &mut Host, now_ms: u64) {
        self.keeper.tick(now_ms);
        let auto_zone = host.settings.get(keys::TIME_AUTO_ZONE).unwrap_or(true);
        let zone = host.settings.get(keys::TIME_ZONE).unwrap_or_default();
        if ui.take_zone_next() && !auto_zone {
            let (utc, offset) = time::utc_now();
            let city = datetime::next_zone(&zone, offset, utc);
            let r = settings::set(
                &mut host.settings,
                Caller::Shell,
                keys::TIME_ZONE.name(),
                Value::Str(city.name.into()),
                now_ms,
            );
            dprintln!("neoui: clock: zone -> {} ({:?})", city.name, r);
            self.zone_checked = None; // apply it now
        }
        let (now, second) = local_now();
        // the zone's rule (summer time) while the network doesn't set the zone; once a minute
        let minute = now.minute_of_week();
        if !auto_zone && self.zone_checked != Some(minute) {
            self.zone_checked = Some(minute);
            if let Some(city) = world::find(&zone) {
                let (utc, offset) = time::utc_now();
                let want = city.offset_at(utc);
                if want != offset {
                    let r = time::set_utc_and_offset(utc, want);
                    dprintln!("neoui: clock: {} -> offset {} s ({:?})", city.name, want, r); // the keeper saves it
                }
            }
        }
        let h24 = host.settings.get(keys::CLOCK_24H).unwrap_or(true);
        let seconds = host.settings.get(keys::STATUS_SECONDS).unwrap_or(false);
        let second = if seconds { second } else { 0 };
        let seq = host.settings.store().seq();
        let offset = time::utc_now().1;
        let shown = (now, second, h24, seq, offset);
        if self.shown != Some(shown) {
            let minute_changed = self.shown.is_none_or(|s| s.0 != now || s.2 != h24);
            self.shown = Some(shown);
            let face = Face::new(&now, h24, &alarms(&host.settings));
            let mut text = status_text(&face);
            if seconds {
                text = format!("{text}:{second:02}");
            }
            if minute_changed {
                dprintln!("neoui: clock {}", text);
            }
            ui.set_clock(&text);
            ui.set_now(&face);
            let format = host.settings.get(keys::TIME_DATE_FORMAT).unwrap_or(0);
            ui.set_datetime(
                &datetime::date_text(&now, format),
                &datetime::zone_text(time::utc_now().0, offset, &zone),
                &self.synced,
            );
        }
        if self.checked != Some(minute) {
            self.checked = Some(minute);
            if let Some(alarm) = alarms(&host.settings).due(minute).next() {
                dprintln!(
                    "neoui: alarm {:02}:{:02} due (id {})",
                    alarm.hour,
                    alarm.minute,
                    alarm.id
                );
                self.ringing = Some(alarm.id);
                ui.show(CLOCK_APP, "");
            }
        }
        // the app is started when the shell shows its screen (next frame); the event waits in its queue
        if let Some(id) = self.ringing
            && host.post(CLOCK_APP, AppEvent::Alarm { id })
        {
            self.ringing = None;
        }
    }

    /// A network time arrived from `source` (Settings -> Date & time "Last set: Mobile network, 23:41").
    pub fn set_synced(&mut self, source: &str) {
        let (now, _) = local_now();
        self.synced = format!("{source}, {:02}:{:02}", now.hour, now.minute);
        self.shown = None;
    }

    /// Before a restart or power-off: keep the clock.
    pub fn power(&mut self, action: i32) {
        clock::power(action, &mut self.keeper);
    }
}

/// The local time from the kernel, and its second.
fn local_now() -> (LocalTime, u32) {
    let t = time::now();
    let local = LocalTime {
        year: t.year,
        month: t.month,
        day: t.day,
        weekday: t.weekday,
        hour: t.hour,
        minute: t.minute,
    };
    (local, t.second)
}

/// The alarms as saved by the Clock app; none when unset or unreadable (logged).
fn alarms(settings: &Settings) -> Alarms {
    let Ok(blob) = settings.get(keys::CLOCK_ALARMS) else {
        return Alarms::new();
    };
    if blob.0.is_empty() {
        return Alarms::new();
    }
    Alarms::decode(&blob.0).unwrap_or_else(|e| {
        dprintln!("neoui: clock.alarms unreadable ({}): no alarms", e);
        Alarms::new()
    })
}

/// The status bar's time: "09:41" on a 24-hour clock, "9:41" on a 12-hour one (no AM/PM: the bar is narrow).
fn status_text(face: &Face) -> String {
    if face.h24 {
        format!("{:02}:{:02}", face.hours, face.minutes)
    } else {
        format!("{}:{:02}", face.hours, face.minutes)
    }
}

// SPDX-License-Identifier: GPL-3.0-only
//! The shell's side of the Clock: the time shown (the status bar, the home clock widget and agenda card), the zone's
//! summer time, and the clock kept across restarts ([`clock::Keeper`]).
//!
//! Alarms and events are DTSRV's (NeoSymbian/datetime, protocol 1.19): it rings them and publishes the next of each
//! as the setting `dtsrv.next` (`neoui_proto::clock::next`), which the widget and the card show.

use crate::clock;
use crate::host::Host;
use crate::settings::Settings;
use alloc::format;
use alloc::string::String;
use e32::{dprintln, time};
use neoui_proto::clock::alarm::Minute;
use neoui_proto::clock::face::{Face, LocalTime};
use neoui_proto::clock::next::Next;
use neoui_proto::clock::{datetime, world};
use neoui_proto::ipc as proto;
use neoui_proto::settings::keys;
use neoui_proto::settings_app::info::Clock;
use neoui_render::Renderer;

pub struct Clockwork {
    keeper: clock::Keeper,
    /// What the face shows now (the time, the second when the status bar shows it, the 24-hour setting, the
    /// settings' sequence number and the offset), so it is redrawn only when it changes.
    shown: Option<(LocalTime, u32, bool, u32, i32)>,
    /// Where the time last came from (Settings -> Date & time), set by the network clock.
    pub synced: String,
    /// The last minute the zone's rule was applied.
    zone_checked: Option<Minute>,
    /// The zone (`time.zone`) as last seen.
    zone_seen: String,
}

impl Clockwork {
    /// Sets the clock from what was saved ([`clock::Keeper::boot`]).
    pub fn boot() -> Self {
        Self {
            keeper: clock::Keeper::boot(),
            shown: None,
            synced: String::from("Not yet"),
            zone_checked: None,
            zone_seen: String::new(),
        }
    }

    /// Once per frame: keeps the clock and updates the face.
    pub fn frame(&mut self, ui: &mut Renderer, host: &mut Host, now_ms: u64) {
        self.keeper.tick(now_ms);
        let auto_zone = host.settings.get(keys::TIME_AUTO_ZONE).unwrap_or(true);
        let zone = host.settings.get(keys::TIME_ZONE).unwrap_or_default();
        // a new zone (the Settings app's city sheet) is applied now, not at the next minute
        if zone != self.zone_seen {
            self.zone_seen.clone_from(&zone);
            self.zone_checked = None;
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
        let (utc, offset) = time::utc_now();
        let shown = (now, second, h24, seq, offset);
        if self.shown != Some(shown) {
            let minute_changed = self.shown.is_none_or(|s| s.0 != now || s.2 != h24);
            self.shown = Some(shown);
            let next = next(&host.settings);
            let face = Face::new(&now, h24, &next.alarm_text(utc, offset, h24));
            let mut text = status_text(&face);
            if seconds {
                text = format!("{text}:{second:02}");
            }
            if minute_changed {
                dprintln!("neoui: clock {}", text);
            }
            ui.set_clock(&text);
            ui.set_now(&face);
            ui.set_next_event(
                &next.event_text(utc, offset, h24),
                &next.event_title,
                next.event_colour,
            );
            let format = host.settings.get(keys::TIME_DATE_FORMAT).unwrap_or(0);
            ui.set_date_text(&datetime::date_text(&now, format));
            let info = Clock {
                zone: datetime::zone_text(utc, offset, &zone),
                synced: self.synced.clone(),
            };
            host.set_info(proto::INFO_CLOCK, info.encode());
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

/// What DTSRV says comes next; nothing when it hasn't published (not running) or the blob is unreadable (logged).
fn next(settings: &Settings) -> Next {
    let Ok(blob) = settings.get(keys::DTSRV_NEXT) else {
        return Next::default();
    };
    Next::decode(&blob.0).unwrap_or_else(|e| {
        dprintln!("neoui: dtsrv.next unreadable ({}): nothing next", e);
        Next::default()
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

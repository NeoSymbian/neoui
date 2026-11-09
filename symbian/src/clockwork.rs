// SPDX-License-Identifier: GPL-3.0-only
//! The shell's side of the Clock: the time shown (the status bar and the home clock widget), alarms that ring
//! whether or not the Clock app runs, and the clock kept across restarts ([`clock::Keeper`]).
//!
//! Alarms live in the Clock app's setting `clock.alarms`; the server reads them straight from its settings store
//! with the same model the app uses (`neoui_proto::clock::alarm`), so "due" means the same to both.

use crate::clock;
use crate::host::Host;
use crate::settings::Settings;
use alloc::format;
use alloc::string::String;
use e32::{dprintln, time};
use neoui_proto::clock::alarm::{Alarms, Minute};
use neoui_proto::clock::face::{Face, LocalTime};
use neoui_proto::ipc::AppEvent;
use neoui_proto::settings::keys;
use neoui_render::Renderer;

/// The Clock app's screen key (`host::APPS`).
const CLOCK_APP: &str = "clock";

pub struct Clockwork {
    keeper: clock::Keeper,
    /// What the face shows now, so it is redrawn only when it changes.
    shown: Option<(LocalTime, bool, u32)>,
    /// The last minute alarms were checked for.
    checked: Option<Minute>,
    /// An alarm that rang, until the Clock app has been told.
    ringing: Option<u16>,
}

impl Clockwork {
    /// Sets the clock from what was saved ([`clock::Keeper::boot`]).
    pub fn boot() -> Self {
        Self {
            keeper: clock::Keeper::boot(),
            shown: None,
            checked: None,
            ringing: None,
        }
    }

    /// Once per frame: keeps the clock, updates the face, rings due alarms.
    pub fn frame(&mut self, ui: &mut Renderer, host: &mut Host, now_ms: u64) {
        self.keeper.tick(now_ms);
        let now = local_now();
        let h24 = host.settings.get(keys::CLOCK_24H).unwrap_or(true);
        let seq = host.settings.store().seq();
        if self.shown != Some((now, h24, seq)) {
            self.shown = Some((now, h24, seq));
            let face = Face::new(&now, h24, &alarms(&host.settings));
            let text = status_text(&face);
            dprintln!("neoui: clock {}", text);
            ui.set_clock(&text);
            ui.set_now(&face);
        }
        let minute = now.minute_of_week();
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

    /// Before a restart or power-off: keep the clock.
    pub fn power(&mut self, action: i32) {
        clock::power(action, &mut self.keeper);
    }
}

/// The local time from the kernel.
fn local_now() -> LocalTime {
    let t = time::now();
    LocalTime {
        year: t.year,
        month: t.month,
        day: t.day,
        weekday: t.weekday,
        hour: t.hour,
        minute: t.minute,
    }
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

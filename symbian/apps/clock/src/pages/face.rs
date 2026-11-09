// SPDX-License-Identifier: GPL-3.0-only
//! The Clock tab: the face (the same `Face` model the home widget shows) and the way into "Set time".

use super::{Ctx, Now};
use crate::ClockState;
use crate::state::{Model, View};
use alloc::rc::Rc;
use neoui_proto::clock::Local;
use neoui_proto::clock::face::Face;

pub fn bind(ctx: &Rc<Ctx>, s: &ClockState<'_>) {
    let c = ctx.clone();
    s.on_set_time(move || {
        // the editor starts at the clock's local time and zone
        let now = Now::read();
        c.model.borrow_mut().set_time = Local {
            year: now.local.year,
            month: now.local.month,
            day: now.local.day,
            hour: now.local.hour,
            minute: now.local.minute,
            tz: now.tz,
        };
        c.open(View::SetTime);
    });
}

pub fn show(s: &ClockState<'_>, m: &Model, now: &Now) {
    let face = Face::new(&now.local, m.h24, &m.alarms);
    // at most 23 / 59 / 59
    s.set_hours(face.hours as i32);
    s.set_minutes(face.minutes as i32);
    s.set_seconds(now.second as i32);
    s.set_h24(face.h24);
    s.set_am_pm(face.am_pm.into());
    s.set_date(face.date.as_str().into());
    s.set_next_alarm(face.next_alarm.as_str().into());
}

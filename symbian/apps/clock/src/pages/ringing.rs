// SPDX-License-Identifier: GPL-3.0-only
//! A ringing alarm: the UI server's scheduler found it due and sent `AppEvent::Alarm`. Dismiss turns a one-shot
//! alarm off; Snooze adds a one-shot alarm five minutes on (`clock::alarm`). Either saves `clock.alarms`.

use super::{Ctx, Now};
use crate::ClockState;
use crate::state::{Model, Part, View};
use alloc::rc::Rc;
use e32::dprintln;

/// Shows alarm `id` ringing (from the event loop).
pub fn ring(ctx: &Ctx, id: u16) {
    let known = {
        let mut m = ctx.model.borrow_mut();
        // the server read the alarm from the same setting; this copy may be older
        m.reload_shared(&ctx.app);
        m.ringing = Some(id);
        m.alarms.get(id).is_some()
    };
    dprintln!(
        "clock: alarm {} ringing{}",
        id,
        if known { "" } else { " (unknown)" }
    );
    ctx.open(View::Ringing);
}

pub fn bind(ctx: &Rc<Ctx>, s: &ClockState<'_>) {
    let c = ctx.clone();
    s.on_dismiss(move || {
        c.update(|m, _| {
            let id = m.ringing.take()?;
            m.alarms.dismissed(id);
            dprintln!("clock: alarm {} dismissed", id);
            Some(Part::Alarms)
        });
        c.open(View::None);
    });
    let c = ctx.clone();
    s.on_snooze(move || {
        c.update(|m, now: &Now| {
            let id = m.ringing.take()?;
            match m.alarms.snooze(id, now.local.minute_of_week()) {
                Ok(new) => dprintln!("clock: alarm {} snoozed (id {})", id, new),
                Err(_) => dprintln!("clock: alarm {} snooze: list full", id),
            }
            Some(Part::Alarms)
        });
        c.open(View::None);
    });
}

pub fn show(s: &ClockState<'_>, m: &Model) {
    let Some(alarm) = m.ringing.and_then(|id| m.alarms.get(id)) else {
        return;
    };
    s.set_ringing_time(alarm.time_text(m.h24).as_str().into());
    s.set_ringing_label(
        if alarm.label.is_empty() {
            "Alarm"
        } else {
            alarm.label.as_str()
        }
        .into(),
    );
}

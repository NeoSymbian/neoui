// SPDX-License-Identifier: GPL-3.0-only
//! The Alarms tab and the alarm editor: the list (switch on/off, tap to edit), "+ Add alarm", and the editor's
//! time steppers, day chips and label presets. Every change is saved in `clock.alarms` at once; the UI server's
//! scheduler rings them from there.

use super::{Ctx, rows};
use crate::state::{Draft, Model, Part, View};
use crate::{AlarmRow, ClockState};
use alloc::rc::Rc;
use alloc::vec::Vec;
use e32::dprintln;
use neoui_proto::clock::alarm::{ALARMS_MAX, Alarm, Weekdays};

// shortcut: preset labels instead of typing one; use the system keyboard when the editor gets a text field.
/// The labels the editor cycles through.
const LABELS: [&str; 6] = ["Alarm", "Wake up", "Work", "Gym", "Medicine", "Meeting"];

pub fn bind(ctx: &Rc<Ctx>, s: &ClockState<'_>) {
    let c = ctx.clone();
    s.on_add_alarm(move || {
        {
            let mut m = c.model.borrow_mut();
            // a new alarm starts on the next whole hour
            let now = super::Now::read();
            let hour = (now.local.hour + 1) % 24;
            m.draft = Some(Draft {
                id: None,
                alarm: Alarm {
                    id: 0,
                    hour: hour as u8,
                    minute: 0,
                    days: Weekdays::ONCE,
                    label: LABELS[0].into(),
                    on: true,
                },
            });
        }
        c.open(View::Alarm);
    });
    let c = ctx.clone();
    s.on_edit_alarm(move |id| {
        let found = {
            let mut m = c.model.borrow_mut();
            let alarm = u16::try_from(id)
                .ok()
                .and_then(|id| m.alarms.get(id).cloned());
            m.draft = alarm.map(|a| Draft {
                id: Some(a.id),
                alarm: a,
            });
            m.draft.is_some()
        };
        if found {
            c.open(View::Alarm);
        }
    });
    let c = ctx.clone();
    s.on_alarm_switched(move |id, on| {
        c.update(|m, _| {
            let id = u16::try_from(id).ok()?;
            m.alarms.set_on(id, on).then(|| {
                dprintln!("clock: alarm {} {}", id, if on { "on" } else { "off" });
                Part::Alarms
            })
        });
    });
    let c = ctx.clone();
    s.on_edit_step(move |field, delta| {
        c.update(|m, _| {
            let d = m.draft.as_mut()?;
            let a = &mut d.alarm;
            match field {
                0 => a.hour = wrap(a.hour, delta, 24),
                _ => a.minute = wrap(a.minute, delta, 60),
            }
            None
        });
    });
    let c = ctx.clone();
    s.on_edit_day(move |day| {
        c.update(|m, _| {
            let d = m.draft.as_mut()?;
            d.alarm.days = d.alarm.days.toggled(u8::try_from(day).ok()?);
            None
        });
    });
    let c = ctx.clone();
    s.on_edit_label_next(move || {
        c.update(|m, _| {
            let d = m.draft.as_mut()?;
            let at = LABELS
                .iter()
                .position(|l| *l == d.alarm.label)
                .map_or(0, |i| i + 1);
            d.alarm.label = LABELS[at % LABELS.len()].into();
            None
        });
    });
    let c = ctx.clone();
    s.on_save_alarm(move || {
        c.update(|m, _| {
            let d = m.draft.take()?;
            let a = d.alarm;
            match d.id {
                Some(_) => {
                    m.alarms.update(a.clone());
                    dprintln!("clock: alarm {} saved {:02}:{:02}", a.id, a.hour, a.minute);
                }
                None => match m.alarms.add(a.hour, a.minute, a.days, &a.label) {
                    Ok(id) => dprintln!(
                        "clock: alarm added {:02}:{:02} (id {})",
                        a.hour,
                        a.minute,
                        id
                    ),
                    Err(_) => dprintln!("clock: alarm list full"),
                },
            }
            Some(Part::Alarms)
        });
        c.open(View::None);
    });
    let c = ctx.clone();
    s.on_delete_alarm(move || {
        c.update(|m, _| {
            let id = m.draft.take()?.id?;
            m.alarms.remove(id).then(|| {
                dprintln!("clock: alarm {} deleted", id);
                Part::Alarms
            })
        });
        c.open(View::None);
    });
}

pub fn show(s: &ClockState<'_>, m: &Model) {
    s.set_alarms(rows(m.alarms.as_slice().iter().map(|a| AlarmRow {
        id: i32::from(a.id),
        time: a.time_text(m.h24).as_str().into(),
        days: a.days.describe().as_str().into(),
        label: if a.label.is_empty() {
            "Alarm".into()
        } else {
            a.label.as_str().into()
        },
        on: a.on,
    })));
    s.set_alarms_full(m.alarms.as_slice().len() >= ALARMS_MAX);
    if let Some(d) = &m.draft {
        let a = &d.alarm;
        s.set_edit_new(d.id.is_none());
        let (hour, am_pm) = if m.h24 {
            (a.hour, "")
        } else {
            (
                (a.hour + 11) % 12 + 1,
                if a.hour < 12 { "AM" } else { "PM" },
            )
        };
        s.set_edit_hour(i32::from(hour));
        s.set_edit_minute(i32::from(a.minute));
        s.set_edit_am_pm(am_pm.into());
        s.set_edit_days(rows(
            (0..7u8).map(|d| a.days.contains(d)).collect::<Vec<_>>(),
        ));
        s.set_edit_label(a.label.as_str().into());
        s.set_edit_repeat(a.days.describe().as_str().into());
    }
}

/// `v + delta` wrapped into `0..n`.
fn wrap(v: u8, delta: i32, n: i32) -> u8 {
    // the result is in 0..n, n <= 60
    (i32::from(v) + delta).rem_euclid(n) as u8
}

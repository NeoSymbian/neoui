// SPDX-License-Identifier: GPL-3.0-only
//! Clock settings (the toolbar's Options): the face (digital or analog), the seconds, the date, 24-hour time. Each
//! change is saved at once under its own key.

use super::Ctx;
use crate::ClockState;
use crate::state::{Model, Part};
use alloc::rc::Rc;

pub fn bind(ctx: &Rc<Ctx>, s: &ClockState<'_>) {
    let c = ctx.clone();
    s.on_set_analog(move |on| c.update(|m, _| set(&mut m.analog, on, Part::Face)));
    let c = ctx.clone();
    s.on_set_seconds(move |on| c.update(|m, _| set(&mut m.seconds, on, Part::Seconds)));
    let c = ctx.clone();
    s.on_set_date(move |on| c.update(|m, _| set(&mut m.date, on, Part::Date)));
    let c = ctx.clone();
    s.on_set_h24(move |on| c.update(|m, _| set(&mut m.h24, on, Part::H24)));
}

fn set(field: &mut bool, on: bool, part: Part) -> Option<Part> {
    (*field != on).then(|| {
        *field = on;
        part
    })
}

pub fn show(s: &ClockState<'_>, m: &Model) {
    s.set_analog(m.analog);
    s.set_show_seconds(m.seconds);
    s.set_show_date(m.date);
}

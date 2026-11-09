// SPDX-License-Identifier: GPL-3.0-only
//! "Set time": steppers for the date, the time and the zone (`neoui_proto::clock::Local`); Save sets Symbian's clock
//! (needs WriteDeviceData, which ROM EXEs have) and the UI server keeps it across restarts within seconds.

use super::{Ctx, strings};
use crate::ClockState;
use crate::state::{Model, View};
use alloc::rc::Rc;
use e32::{dprintln, time};
use neoui_proto::clock::FIELDS;

pub fn bind(ctx: &Rc<Ctx>, s: &ClockState<'_>) {
    let c = ctx.clone();
    s.on_adjust(move |field, delta| {
        c.update(|m, _| {
            // field indexes FIELDS (0..6), delta is ±1
            m.set_time
                .adjust(usize::try_from(field).unwrap_or(usize::MAX), delta);
            None
        });
    });
    let c = ctx.clone();
    s.on_save_time(move || {
        let l = c.model.borrow().set_time;
        match time::set_utc_and_offset(l.to_unix(), l.tz) {
            Ok(()) => dprintln!("clock: set {} UTC, offset {} s", l.to_unix(), l.tz),
            Err(e) => dprintln!("clock: set failed ({})", e),
        }
        c.open(View::None);
    });
}

pub fn show(s: &ClockState<'_>, m: &Model) {
    s.set_fields(strings(FIELDS));
    s.set_values(strings((0..FIELDS.len()).map(|i| m.set_time.show(i))));
}

// SPDX-License-Identifier: GPL-3.0-only
//! The Stopwatch tab: Start/Stop, Lap, Reset and the laps (`clock::stopwatch`, wall-clock based, saved in
//! `clock.stopwatch` on every change so it keeps running while the app is closed).

use super::{Ctx, Now, rows};
use crate::state::{Model, Part};
use crate::{ClockState, LapRow};
use alloc::rc::Rc;
use e32::dprintln;
use neoui_proto::clock::stopwatch::format_elapsed;
use slint::Model as _;

pub fn bind(ctx: &Rc<Ctx>, s: &ClockState<'_>) {
    let c = ctx.clone();
    s.on_stopwatch_toggle(move || {
        c.update(|m, now| {
            if m.stopwatch.is_running() {
                m.stopwatch.stop(now.wall_ms);
                dprintln!(
                    "clock: stopwatch stop {} ms",
                    m.stopwatch.elapsed_ms(now.wall_ms)
                );
            } else {
                m.stopwatch.start(now.wall_ms);
                dprintln!("clock: stopwatch start");
            }
            Some(Part::Stopwatch)
        });
    });
    let c = ctx.clone();
    s.on_stopwatch_lap(move || {
        c.update(|m, now| match m.stopwatch.lap(now.wall_ms) {
            Ok(()) => {
                dprintln!(
                    "clock: stopwatch lap {} ms",
                    m.stopwatch.elapsed_ms(now.wall_ms)
                );
                Some(Part::Stopwatch)
            }
            Err(_) => None,
        });
    });
    let c = ctx.clone();
    s.on_stopwatch_reset(move || {
        c.update(|m, _| {
            m.stopwatch.reset();
            dprintln!("clock: stopwatch reset");
            Some(Part::Stopwatch)
        });
    });
}

pub fn show(s: &ClockState<'_>, m: &Model, now: &Now) {
    let w = &m.stopwatch;
    let elapsed = w.elapsed_ms(now.wall_ms);
    s.set_stopwatch_text(format_elapsed(elapsed).as_str().into());
    s.set_stopwatch_running(w.is_running());
    s.set_stopwatch_started(elapsed > 0);
    // the laps change only on a lap or reset; rebuilding them each tick would redraw the list
    if s.get_laps().row_count() != w.laps().count() {
        s.set_laps(rows(w.laps().map(|(n, lap, total)| LapRow {
            // at most LAPS_MAX
            number: n as i32,
            lap: format_elapsed(lap).as_str().into(),
            total: format_elapsed(total).as_str().into(),
        })));
    }
}

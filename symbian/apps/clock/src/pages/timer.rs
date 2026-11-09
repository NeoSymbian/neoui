// SPDX-License-Identifier: GPL-3.0-only
//! The Timer tab: presets and steppers while idle, Start/Pause/Resume/Reset (`clock::timer`, wall-clock based, saved
//! in `clock.timer` on every change so a running timer survives the app closing and a reboot).

use super::{Ctx, Now, strings};
use crate::ClockState;
use crate::state::{Model, Part};
use alloc::format;
use alloc::rc::Rc;
use e32::dprintln;
use neoui_proto::clock::timer::{PRESETS_MS, Phase, format_left};

pub fn bind(ctx: &Rc<Ctx>, s: &ClockState<'_>) {
    let c = ctx.clone();
    s.on_timer_preset(move |i| {
        c.update(|m, _| {
            let ms = *PRESETS_MS.get(usize::try_from(i).ok()?)?;
            m.timer.set(i64::from(ms));
            Some(Part::Timer)
        });
    });
    let c = ctx.clone();
    s.on_timer_step(move |seconds| {
        c.update(|m, _| {
            m.timer.adjust(i64::from(seconds) * 1000);
            Some(Part::Timer)
        });
    });
    let c = ctx.clone();
    s.on_timer_start(move || {
        c.update(|m, now| {
            m.timer.start(now.wall_ms);
            dprintln!(
                "clock: timer start {} s",
                m.timer.left_ms(now.wall_ms) / 1000
            );
            Some(Part::Timer)
        });
    });
    let c = ctx.clone();
    s.on_timer_pause(move || {
        c.update(|m, now| {
            m.timer.pause(now.wall_ms);
            dprintln!("clock: timer paused");
            Some(Part::Timer)
        });
    });
    let c = ctx.clone();
    s.on_timer_reset(move || {
        c.update(|m, _| {
            m.timer.reset();
            dprintln!("clock: timer reset");
            Some(Part::Timer)
        });
    });
}

pub fn show(s: &ClockState<'_>, m: &Model, now: &Now) {
    let t = &m.timer;
    let left = t.left_ms(now.wall_ms);
    s.set_timer_text(format_left(left).as_str().into());
    s.set_timer_phase(match t.phase {
        Phase::Idle => 0,
        Phase::Running { .. } => 1,
        Phase::Paused { .. } => 2,
        Phase::Done => 3,
    });
    // the share still to go: full while idle, empty when done
    s.set_timer_progress(if t.set_ms == 0 {
        0.0
    } else {
        left as f32 / t.set_ms as f32
    });
    s.set_timer_presets(strings(
        PRESETS_MS.iter().map(|ms| format!("{} min", ms / 60_000)),
    ));
}

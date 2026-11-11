// SPDX-License-Identifier: GPL-3.0-only
//! The Clock tab: the face (the same `Face` model the home widget shows) and the way into "Set time".

use super::{Ctx, Now};
use crate::ClockState;
use crate::state::{Model, View};
use alloc::rc::Rc;
use neoui_proto::clock::Local;
use neoui_proto::clock::analog::{self, Rgba};
use neoui_proto::clock::face::Face;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

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

/// The analog face's dial for one size and ink, drawn once (the hands change every second, the dial doesn't).
#[derive(Default)]
pub struct Analog {
    dial: alloc::vec::Vec<Rgba>,
    size: usize,
    ink: [u8; 3],
}

/// The face at `(h, m, s)`: a copy of the dial with the hands over it.
pub fn draw_analog(
    a: &mut Analog,
    size: usize,
    t: (u32, u32, u32),
    seconds: bool,
    ink: [u8; 3],
    accent: [u8; 3],
) -> Image {
    if a.size != size || a.ink != ink {
        a.dial = alloc::vec![[0; 4]; size * size];
        analog::dial(&mut a.dial, size, ink);
        (a.size, a.ink) = (size, ink);
    }
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(size as u32, size as u32);
    let (px, _) = buf.make_mut_bytes().as_chunks_mut::<4>();
    px.copy_from_slice(&a.dial);
    analog::hands(px, size, t, seconds, ink, accent);
    Image::from_rgba8(buf)
}

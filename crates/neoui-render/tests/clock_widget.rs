// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::clock::alarm::{Alarms, Weekdays};
use neoui_proto::clock::face::{Face, LocalTime};

/// The home clock widget is live: `set_now` changes what it draws (time, date, next alarm), and a tap on it opens
/// the Clock app.
#[test]
fn clock_widget() {
    let (w, h) = (540, 1080);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let mon = LocalTime {
        year: 2026,
        month: 11,
        day: 9,
        weekday: 0,
        hour: 20,
        minute: 30,
    };
    let mut alarms = Alarms::new();
    alarms.add(7, 15, Weekdays::WORKDAYS, "").expect("room");

    r.set_now(&Face::new(&mon, true, &alarms));
    common::frame(&mut r, &mut buf, w, "home");
    let before = buf.clone();
    common::write_png("home-clock-widget-540x1080.png", &buf, w, h);

    r.set_now(&Face::new(&LocalTime { minute: 31, ..mon }, false, &alarms));
    common::frame(&mut r, &mut buf, w, "home");
    common::write_png("home-clock-widget-12h-540x1080.png", &buf, w, h);
    let d = common::dp(w, h);
    // the widget sits in the top part of page 0, below the 25 dp status bar
    let top = (25.0 * d) as usize * w as usize;
    let bottom = (25.0 * d + 180.0 * d) as usize * w as usize;
    let changed = buf[top..bottom]
        .iter()
        .zip(&before[top..bottom])
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 500,
        "the widget did not redraw: {changed} px changed"
    );

    // a tap on the time opens the Clock app
    common::tap(&mut r, (40.0 * d) as i32, (25.0 * d + 50.0 * d) as i32);
    common::settle();
    r.render(&mut buf, w as usize);
    assert_eq!(r.screen(), "clock");
}

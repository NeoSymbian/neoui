// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::activity::{Activity, Trail, key, kind};
use neoui_proto::island::{self, Island};
use neoui_proto::media::{Session, caps, state};
use neoui_proto::page::{NOTIFY_ONGOING, action, style};
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::{Notification, Renderer};

const NOW: u64 = 1_795_691_635_000;

fn call() -> Activity {
    Activity {
        id: 0,
        kind: kind::CALL,
        title: "Ava Lindqvist".into(),
        text: "Mobile".into(),
        trail: Trail::Since(NOW - 252_000),
        ..Activity::default()
    }
}

fn timer() -> Activity {
    Activity {
        id: 1,
        kind: kind::TIMER,
        title: "Pasta".into(),
        text: "Timer".into(),
        trail: Trail::Until(NOW + 192_000),
        buttons: vec!["Pause".into(), "Cancel".into()],
        ..Activity::default()
    }
}

fn song() -> Session {
    Session {
        state: state::PLAYING,
        caps: caps::ALL,
        modes: 0,
        title: "Northern Lights".into(),
        artist: "The Harbour".into(),
        album: String::new(),
        art: 0,
        duration_ms: 245_000,
        position_ms: 102_000,
        at: NOW,
    }
}

/// What the UI server does each second: the island's model in its order, the two it shows.
fn show(r: &mut Renderer, isl: &Island<&'static str>, media: Option<&Session>, full: bool) {
    let m = media.and_then(|s| island::from_media(s).map(|a| ("files", a)));
    let all = isl.ordered(m);
    let shown = island::shown(&all, full);
    let idx: Vec<usize> = shown
        .iter()
        .filter_map(|(k, a)| all.iter().position(|(k2, b)| k2 == k && b.id == a.id))
        .collect();
    let live: Vec<(&str, &Activity)> = all.iter().map(|(k, a)| (*k, a)).collect();
    r.set_island(&live, &idx, NOW);
}

/// A press held 600 ms; `buf` is the renderer's frame (only damage is drawn into it).
fn hold(r: &mut Renderer, buf: &mut [u32], x: i32, y: i32) {
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y,
        phase: TouchPhase::Down,
    });
    r.render(buf, 1080);
    common::advance(600);
    r.render(buf, 1080);
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y,
        phase: TouchPhase::Up,
    });
}

fn swipe(r: &mut Renderer, x: i32, y: i32, dx: i32) {
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y,
        phase: TouchPhase::Down,
    });
    for k in 1..=6 {
        r.dispatch(InputEvent::Touch {
            id: 0,
            x: x + dx * k / 6,
            y,
            phase: TouchPhase::Move,
        });
    }
    r.dispatch(InputEvent::Touch {
        id: 0,
        x: x + dx,
        y,
        phase: TouchPhase::Up,
    });
}

/// TP6 in the shell (di-states, di-call, di-home, nt-shade): the island in the status bar with one activity, two (a
/// pill and a circle; a sideways swipe swaps them), a 2 s alert in its place; held, a call opens in place with the
/// shell's Mute · Keypad · Speaker · End (End reported as 0x13) and lies over the app (`island_rect`); media held
/// open is the player; the activities are cards at the top of the shade's Live section. Over a full-screen app only
/// a call floats. Snapshots `tp6-island-*.png`, `di-call.png`, `di-home.png`, `nt-shade.png`.
#[test]
fn the_island() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let at = |x: f32, y: f32| ((x * d) as i32, (y * d) as i32);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");
    let quiet = buf.clone();
    let bar = (25.0 * d) as usize * w as usize;

    // one: the call, green, its time ticking
    let mut isl: Island<&'static str> = Island::default();
    isl.post("phone", call(), false).unwrap();
    show(&mut r, &isl, None, false);
    common::frame(&mut r, &mut buf, w, "one");
    common::write_png("tp6-island-call.png", &buf, w, h);
    assert_ne!(
        buf[..bar],
        quiet[..bar],
        "the island draws in the status bar"
    );
    assert_eq!(buf[bar..], quiet[bar..], "and nowhere else");
    assert_eq!(
        r.island_rect(),
        None,
        "it lies in the status bar, not over the app"
    );
    let (x, y) = at(170.0, 12.5);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "tapped");
    assert_eq!(r.take_island(), vec![("phone".into(), 0, 0)]);

    // two: the call's pill, the timer's circle; a sideways swipe brings the timer forward
    isl.post("clock", timer(), false).unwrap();
    show(&mut r, &isl, None, false);
    common::frame(&mut r, &mut buf, w, "two");
    common::write_png("tp6-island-two.png", &buf, w, h);
    swipe(&mut r, x, y, (60.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "swapped");
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "tapped the timer");
    assert_eq!(r.take_island(), vec![("clock".into(), 1, 0)]);
    swipe(&mut r, x, y, -(60.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "back");

    // held: the call opens in place over the app (di-call)
    hold(&mut r, &mut buf, x, y);
    common::frame(&mut r, &mut buf, w, "expanded");
    assert!(r.island_open());
    common::write_png("di-call.png", &buf, w, h);
    let o = r.island_rect().expect("over the app");
    assert!(o.h as f32 > 100.0 * d, "{o:?}");
    assert!(r.take_island().is_empty(), "a hold is no tap");
    let (ex, ey) = at(297.0, 102.0);
    common::tap(&mut r, ex, ey);
    common::frame(&mut r, &mut buf, w, "End");
    assert_eq!(r.take_island(), vec![("phone".into(), 0, key::END)]);
    // a touch on the app folds it
    r.fold_island();
    common::frame(&mut r, &mut buf, w, "folded");
    assert!(!r.island_open() && r.island_rect().is_none());

    // the shade: the call and the timer at the top of Live, then the cards (nt-shade)
    let copy = Notification {
        id: 3,
        title: "kernel.img".into(),
        text: "768 MB of 1.2 GB".into(),
        style: style::PROGRESS,
        flags: NOTIFY_ONGOING,
        progress: Some(64),
        actions: vec!["Pause".into(), "Cancel".into()],
        when: NOW,
        ..Notification::default()
    };
    let ava = Notification {
        id: 2,
        title: "Ava Lindqvist".into(),
        text: "Perfect. I'll book a table.\nRunning late, see you at 7?".into(),
        style: style::CONVERSATION,
        initials: "AL".into(),
        tint: 0x22b46a,
        actions: vec!["Reply".into(), "Mark read".into(), "Call".into()],
        action_kinds: vec![action::REPLY, action::IN_PLACE],
        when: NOW - 120_000,
        ..Notification::default()
    };
    let list: Vec<(&str, &Notification)> = vec![("browser", &copy), ("messages", &ava)];
    r.set_notes(&list, NOW, 0, true);
    common::tap(&mut r, (40.0 * d) as i32, y);
    common::frame(&mut r, &mut buf, w, "shade");
    common::write_png("nt-shade.png", &buf, w, h);
    // a tap on the status bar closes it
    common::tap(&mut r, (40.0 * d) as i32, y);
    common::frame(&mut r, &mut buf, w, "shade closed");
    r.set_notes(&[], NOW, 0, true);

    // media and the timer: the timer's pill, media's circle; swiped, then held: the player (di-home)
    isl.end_app("phone");
    show(&mut r, &isl, Some(&song()), false);
    r.set_media(Some(("files", &song(), true)), true, NOW);
    common::frame(&mut r, &mut buf, w, "timer and media");
    common::write_png("tp6-island-media.png", &buf, w, h);
    swipe(&mut r, x, y, (60.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "media forward");
    hold(&mut r, &mut buf, x, y);
    common::frame(&mut r, &mut buf, w, "player");
    common::write_png("di-home.png", &buf, w, h);
    assert!(r.island_open());
    r.fold_island();

    // a 2 s alert in the island's place
    r.set_island_alert(Some((1, "Silent")));
    common::frame(&mut r, &mut buf, w, "alert");
    common::write_png("tp6-island-alert.png", &buf, w, h);
    r.set_island_alert(None);

    // full screen: only a call floats over the app
    show(&mut r, &isl, Some(&song()), true);
    assert!(
        r.island_rows().is_empty(),
        "a timer and media hide over a full-screen app"
    );
    isl.post("phone", call(), false).unwrap();
    show(&mut r, &isl, Some(&song()), true);
    assert_eq!(r.island_rows(), vec![("phone".into(), 0)]);
}

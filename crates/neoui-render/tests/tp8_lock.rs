// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

use neoui_proto::clock::face::{Face, LocalTime};
use neoui_proto::lock::{CardIn, card_rows};
use neoui_proto::media::{Session, caps, state};
use neoui_proto::notification_policy::lock as level;
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;
use neoui_render::lock::{DialogText, LockEvent};

const NOW: u64 = 1_795_691_635_000;

fn swipe(r: &mut Renderer, from: (i32, i32), to: (i32, i32)) {
    let steps = 8;
    r.dispatch(InputEvent::Touch {
        id: 0,
        x: from.0,
        y: from.1,
        phase: TouchPhase::Down,
    });
    for i in 1..=steps {
        common::advance(16);
        r.dispatch(InputEvent::Touch {
            id: 0,
            x: from.0 + (to.0 - from.0) * i / steps,
            y: from.1 + (to.1 - from.1) * i / steps,
            phase: TouchPhase::Move,
        });
    }
    r.dispatch(InputEvent::Touch {
        id: 0,
        x: to.0,
        y: to.1,
        phase: TouchPhase::Up,
    });
}

/// TP8 and TP9 in the shell (nt-lock, nt-lock-pin, set2-lock-pin): the lock screen with the time, the activity, the
/// media player, the cards by their lock level, "+3 more" and the operator in the bar; a swipe up and a tap on a card
/// unlock; the PIN pad's digits come back one by one (Slint gets only their count), Delete, Cancel and its emergency
/// dialer; SYSSTART's PIN dialog for Settings with its Done and ✕. Snapshots `nt-lock.png`, `nt-lock-pin.png`,
/// `set2-lock-pin.png`.
#[test]
fn lock_screen_pin_pad_and_pin_dialog() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let at = |x: f32, y: f32| ((x * d) as i32, (y * d) as i32);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    r.set_network(3, "Ucom", "LTE");
    let mon = LocalTime {
        year: 2026,
        month: 11,
        day: 16,
        weekday: 0,
        hour: 3,
        minute: 12,
    };
    r.set_now(&Face::new(&mon, true, ""));
    let song = Session {
        state: state::PLAYING,
        caps: caps::ALL,
        modes: 0,
        title: "Northern Lights".into(),
        artist: "The Harbour".into(),
        album: String::new(),
        art: 0,
        duration_ms: 200_000,
        position_ms: 10_000,
        at: NOW,
    };
    r.set_media(Some(("files", &song, true)), true, NOW);
    let card = |key, app, lv, title, text, time| CardIn {
        key,
        app,
        level: lv,
        private: false,
        title,
        text,
        time,
    };
    let cards = [
        card(
            "messages",
            "Messages",
            level::HIDE_CONTENT,
            "Ava Lindqvist",
            "On my way",
            "now",
        ),
        card(
            "messages",
            "Messages",
            level::HIDE_CONTENT,
            "Ben",
            "OK",
            "now",
        ),
        card(
            "calendar",
            "Calendar",
            level::SHOW,
            "Lunch with Ben",
            "13:00",
            "12:45",
        ),
        card("files", "Files", level::OFF, "Copied", "", "1 min"),
        card(
            "phone",
            "Phone",
            level::COUNTS,
            "Missed call",
            "Ben",
            "12:01",
        ),
        card(
            "browser",
            "Browser",
            level::SHOW,
            "Downloaded",
            "a.pdf",
            "11:40",
        ),
        card(
            "browser",
            "Browser",
            level::SHOW,
            "Downloaded",
            "b.pdf",
            "11:39",
        ),
    ];
    let (rows, more) = card_rows(&cards, 2);
    r.set_lock_cards(&rows, more);
    r.set_lock_activity(Some(("Clock · Timer · Pasta", "03:12", 0xf5a524)));
    assert!(!r.overlay_open());
    r.set_lock(true, false);
    assert!(r.lock_shown() && r.overlay_open() && !r.number_pad_up());
    common::frame(&mut r, &mut buf, w, "lock");
    common::write_png("nt-lock.png", &buf, w, h);

    // a swipe sideways in Buttons-less modes only from an edge; up from anywhere
    swipe(&mut r, at(180.0, 400.0), at(260.0, 400.0));
    assert_eq!(r.take_lock_events(), [], "sideways in the middle");
    swipe(&mut r, at(180.0, 600.0), at(180.0, 420.0));
    assert_eq!(r.take_lock_events(), [LockEvent::Swiped]);
    swipe(&mut r, at(3.0, 500.0), at(150.0, 500.0));
    assert_eq!(
        r.take_lock_events(),
        [LockEvent::Swiped],
        "in from the left edge"
    );
    // a card: unlock (the pad with a PIN)
    common::frame(&mut r, &mut buf, w, "lock");
    let (cx, cy) = at(180.0, 380.0);
    common::tap(&mut r, cx, cy);
    assert_eq!(r.take_lock_events(), [LockEvent::Swiped], "the first card");

    // the PIN pad (nt-lock-pin): two digits typed
    r.set_lock(true, true);
    r.set_lock_entry(2, "");
    assert!(r.number_pad_up(), "no tap logs or touch rings over the pad");
    common::frame(&mut r, &mut buf, w, "pin pad");
    common::write_png("nt-lock-pin.png", &buf, w, h);
    // the keys: 1 2 3 / 4 5 6 / 7 8 9 / 0 Delete, 100 dp apart, from 213 dp under the status bar
    let key = |c: i32, row: i32| {
        at(
            80.0 + 100.0 * c as f32,
            25.0 + 213.0 + 26.0 + 70.0 * row as f32,
        )
    };
    for (c, row, want) in [
        (1, 1, LockEvent::Digit(5)),
        (0, 0, LockEvent::Digit(1)),
        (1, 3, LockEvent::Digit(0)),
    ] {
        let (x, y) = key(c, row);
        common::tap(&mut r, x, y);
        assert_eq!(r.take_lock_events(), [want]);
    }
    let (x, y) = key(2, 3);
    common::tap(&mut r, x, y);
    assert_eq!(r.take_lock_events(), [LockEvent::Delete]);
    let bottom = 25.0 + 213.0 + 4.0 * 70.0 + 10.0;
    let (x, y) = at(300.0, bottom);
    common::tap(&mut r, x, y);
    assert_eq!(r.take_lock_events(), [LockEvent::Cancel]);
    // Emergency call: the dialer on the same keys, its Call with the number
    let (x, y) = at(92.0, bottom);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "emergency");
    for (c, row) in [(0, 0), (0, 0), (1, 0)] {
        let (x, y) = key(c, row);
        common::tap(&mut r, x, y);
    }
    assert_eq!(r.take_lock_events(), [], "the dialer's digits aren't a PIN");
    let (x, y) = key(2, 3);
    common::tap(&mut r, x, y);
    assert_eq!(
        r.take_lock_events(),
        [LockEvent::EmergencyCall("112".into())]
    );
    r.set_lock(false, false);
    assert!(!r.overlay_open() && !r.number_pad_up());

    // SYSSTART's PIN dialog for Settings (set2-lock-pin), four digits in
    r.show("settings", "");
    r.set_lock_dialog(Some(&DialogText {
        title: "Choose a PIN",
        text: "4 to 8 digits. You enter it when the phone wakes.",
        step: "Step 1 of 2 · next you enter it again",
    }));
    r.set_lock_entry(4, "");
    assert!(r.overlay_open() && r.number_pad_up());
    common::frame(&mut r, &mut buf, w, "dialog");
    common::write_png("set2-lock-pin.png", &buf, w, h);
    // Done (bottom right), 7 (third row), ✕ and ✓ in the header
    let (x, y) = at(300.0, 720.0 - 30.0);
    common::tap(&mut r, x, y);
    assert_eq!(r.take_lock_events(), [LockEvent::Submit]);
    let (x, y) = at(60.0, 720.0 - 30.0 - 48.0);
    common::tap(&mut r, x, y);
    assert_eq!(r.take_lock_events(), [LockEvent::Digit(7)]);
    let (x, y) = at(28.0, 25.0 + 24.0);
    common::tap(&mut r, x, y);
    assert_eq!(r.take_lock_events(), [LockEvent::Cancel]);
    let (x, y) = at(332.0, 25.0 + 24.0);
    common::tap(&mut r, x, y);
    assert_eq!(r.take_lock_events(), [LockEvent::Submit]);
    r.set_lock_dialog(None);
    assert!(!r.overlay_open());
}

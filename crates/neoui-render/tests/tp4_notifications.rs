// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::page::{action, style};
use neoui_render::Notification;

fn x_mid(w: u32) -> i32 {
    (w / 2) as i32
}

/// TP4 in the shell (nt-headsup, nt-shade): an Urgent conversation's banner over home with Do not disturb's moon off,
/// a tap on its Mark read reported as the shade's action (and the banner gone), a swipe that only hides it, and a Min
/// channel's card folded under Silent at the bottom of the list. Snapshots `tp4-*-pixel.png`; their visual
/// comparison with nt-headsup is pending (no board in the export).
#[test]
fn banner_silent_fold_and_dnd() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    let now = 1_795_691_635_000u64;
    let ava = Notification {
        id: 2,
        title: "Ava Lindqvist".into(),
        text: "Running late, see you at 7?".into(),
        style: style::CONVERSATION,
        channel: 1,
        initials: "AL".into(),
        tint: 0x22b46a,
        actions: vec!["Reply".into(), "Mark read".into(), "Call".into()],
        action_kinds: vec![action::REPLY, action::IN_PLACE],
        when: now,
        ..Notification::default()
    };
    let tip = Notification {
        id: 9,
        title: "Tip: hold a file to share it".into(),
        text: "Files".into(),
        when: now - 600_000,
        ..Notification::default()
    };
    let list: Vec<(&str, &Notification)> = vec![("messages", &ava), ("files", &tip)];
    r.show("home", "");
    r.set_silent_notes(&[("files", 9)]);
    r.set_notes(&list, now, 14_400, true);
    r.set_status_apps(&["messages"]);
    common::frame(&mut r, &mut buf, w, "home");
    let quiet = buf.clone();

    // the banner slides in under the status bar
    r.set_banner(Some(("messages", 2)));
    assert!(r.banner_shown());
    common::frame(&mut r, &mut buf, w, "banner");
    common::write_png("tp4-headsup-pixel.png", &buf, w, h);
    assert_ne!(buf, quiet, "the banner draws");
    // where the server composites it over an app and keeps its touches from the app
    let b = r.banner_rect().expect("the banner's bounds");
    let (x, y) = (x_mid(w), (60.0 * d) as i32);
    assert!(b.y >= (25.0 * d) as i32);
    assert!(x >= b.x && y >= b.y && x < b.x + b.w as i32 && y < b.y + b.h as i32);
    assert!(b.h as f32 > 60.0 * d && (b.h as f32) < 200.0 * d, "{b:?}");

    // a swipe up hides it; the card stays (nothing reported)
    let (x, y) = ((w / 2) as i32, (80.0 * d) as i32);
    r.dispatch(neoui_proto::InputEvent::Touch {
        id: 0,
        x,
        y,
        phase: neoui_proto::TouchPhase::Down,
    });
    for step in 1..=6 {
        r.dispatch(neoui_proto::InputEvent::Touch {
            id: 0,
            x,
            y: y - step * (8.0 * d) as i32,
            phase: neoui_proto::TouchPhase::Move,
        });
    }
    r.dispatch(neoui_proto::InputEvent::Touch {
        id: 0,
        x,
        y: y - (48.0 * d) as i32,
        phase: neoui_proto::TouchPhase::Up,
    });
    common::frame(&mut r, &mut buf, w, "swiped up");
    assert!(!r.banner_shown(), "a swipe up hides the banner");
    assert!(r.take_notes_opened().is_empty());

    // shown again: a tap on its body opens the card as the shade's would
    r.set_banner(Some(("messages", 2)));
    common::frame(&mut r, &mut buf, w, "banner again");
    common::tap(&mut r, (w / 2) as i32, (60.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "tapped");
    assert_eq!(r.take_notes_opened(), vec![("messages".to_string(), 2, 0)]);
    assert!(!r.banner_shown());

    // the card left the shade: the banner can't show it
    r.set_banner(Some(("messages", 7)));
    assert!(!r.banner_shown());
    common::frame(&mut r, &mut buf, w, "no banner");
    assert_eq!(r.banner_rect(), None, "nothing to keep from the app");

    // Do not disturb: the moon in the status bar
    r.set_dnd(true);
    common::frame(&mut r, &mut buf, w, "dnd");
    let bar = (25.0 * d) as usize * w as usize;
    assert_ne!(buf[..bar], quiet[..bar], "the moon draws");
    common::write_png("tp4-dnd-pixel.png", &buf, w, h);

    // the shade's list: the Min card under SILENT, last
    common::tap(&mut r, (w / 2) as i32, (12.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "shade");
    common::write_png("tp4-silent-pixel.png", &buf, w, h);
}

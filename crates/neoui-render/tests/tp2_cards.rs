// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::page::{NOTIFY_ONGOING, action, style};
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Notification;

/// TP2a's shade (nt-shade): the first stop with its first 2 cards and See all, the list by section (Live,
/// Conversations, Alerts) with a group's count, the group open, a swipe that dismisses a card but not an ongoing one,
/// and Clear all. Snapshots `tp2-shade-*-pixel.png`.
#[test]
fn cards_by_section_group_swipe_and_clear() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    let now = 1_795_691_635_000u64;
    let card = |id: u32, title: &str, text: &str| Notification {
        id,
        title: title.into(),
        text: text.into(),
        when: now - 120_000,
        ..Notification::default()
    };
    let call = Notification {
        flags: NOTIFY_ONGOING,
        style: style::CALL,
        initials: "AL".into(),
        tint: 0x22b46a,
        actions: vec!["End".into()],
        action_kinds: vec![action::DANGER],
        when: now,
        ..card(1, "Call in progress · 04:12", "Ava Lindqvist")
    };
    let chat = |id, who: &str, initials: &str, tint, text: &str| Notification {
        style: style::CONVERSATION,
        group: "sms".into(),
        initials: initials.into(),
        tint,
        actions: vec!["Reply".into(), "Mark read".into(), "Call".into()],
        action_kinds: vec![action::REPLY, action::IN_PLACE],
        ..card(id, who, text)
    };
    let ava = chat(
        2,
        "Ava Lindqvist",
        "AL",
        0x22b46a,
        "Perfect. I'll book a table.\nRunning late, see you at 7?",
    );
    let ben = chat(
        3,
        "Ben Okafor",
        "BO",
        0x3d8bfd,
        "The build passed on the phone!",
    );
    let grace = chat(4, "Grace Nakamura", "GN", 0xe5484d, "Lunch tomorrow?");
    let copy = Notification {
        style: style::PROGRESS,
        progress: Some(64),
        actions: vec!["Cancel".into()],
        ..card(5, "Copying 48 photos to SD card", "768 MB of 1.2 GB")
    };
    let lunch = Notification {
        when: now - 3 * 3_600_000,
        ..card(6, "Lunch with Ben in 15 min", "13:00 · Harbour Café")
    };
    // the shell's list, newest first
    let list: Vec<(&str, &Notification)> = vec![
        ("files", &copy),
        ("messages", &ava),
        ("phone", &call),
        ("messages", &ben),
        ("clock", &lunch),
        ("messages", &grace),
    ];
    r.show("home", "");
    r.set_notes(&list, now, 14_400, true);
    common::frame(&mut r, &mut buf, w, "home");
    common::tap(&mut r, (w / 2) as i32, (12.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "first stop");
    common::write_png("tp2-shade-first-pixel.png", &buf, w, h);
    let first = buf.clone();

    // Mark read (in place) on the Messages card: reported, and the shade stays open
    common::tap(&mut r, (MARK_READ_X * d) as i32, (MARK_READ_Y * d) as i32);
    common::frame(&mut r, &mut buf, w, "mark read");
    assert_eq!(r.take_notes_opened(), vec![("messages".to_string(), 2, 2)]);
    assert!(r.overlay_open(), "an in-place action leaves the shade open");

    // See all (right of NOTIFICATIONS)
    common::tap(&mut r, (w as f32 - 40.0 * d) as i32, (SEE_ALL_Y * d) as i32);
    common::frame(&mut r, &mut buf, w, "all");
    common::write_png("tp2-shade-all-pixel.png", &buf, w, h);
    assert_ne!(buf, first, "See all opens the list");

    // the group's ▾ (Messages, 3) opens it
    common::tap(&mut r, (w as f32 - 40.0 * d) as i32, (GROUP_Y * d) as i32);
    common::frame(&mut r, &mut buf, w, "group open");
    common::write_png("tp2-shade-group-pixel.png", &buf, w, h);

    let swipe = |r: &mut neoui_render::Renderer, y: f32| {
        let y = (y * d) as i32;
        let x0 = (60.0 * d) as i32;
        r.dispatch(InputEvent::Touch {
            id: 0,
            x: x0,
            y,
            phase: TouchPhase::Down,
        });
        for k in 1..=10 {
            let x = x0 + k * (w as i32 - x0 - 10) / 10;
            r.dispatch(InputEvent::Touch {
                id: 0,
                x,
                y,
                phase: TouchPhase::Move,
            });
        }
        r.dispatch(InputEvent::Touch {
            id: 0,
            x: w as i32 - 10,
            y,
            phase: TouchPhase::Up,
        });
    };
    // the ongoing call doesn't go; the Files card (Alerts) does
    swipe(&mut r, LIVE_Y);
    common::frame(&mut r, &mut buf, w, "swiped the call");
    assert_eq!(
        r.take_notes_dismissed(),
        Vec::<(String, u32)>::new(),
        "an ongoing card stays"
    );
    common::tap(&mut r, (w as f32 - 40.0 * d) as i32, (GROUP_Y * d) as i32); // close the group again
    common::frame(&mut r, &mut buf, w, "group closed");
    swipe(&mut r, ALERT_Y);
    common::frame(&mut r, &mut buf, w, "swiped an alert");
    assert_eq!(
        r.take_notes_dismissed(),
        vec![("files".to_string(), 5)],
        "the Files card, first in the shell's list"
    );

    // a drag up on a card scrolls the list (the last card comes into view) and dismisses nothing
    let before = buf.clone();
    let x = (100.0 * d) as i32;
    let (y0, y1) = ((ALERT_Y * d) as i32, (LIVE_Y * d) as i32);
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y: y0,
        phase: TouchPhase::Down,
    });
    for k in 1..=10 {
        let y = y0 + k * (y1 - y0) / 10;
        r.dispatch(InputEvent::Touch {
            id: 0,
            x,
            y,
            phase: TouchPhase::Move,
        });
    }
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y: y1,
        phase: TouchPhase::Up,
    });
    common::frame(&mut r, &mut buf, w, "scrolled");
    common::write_png("tp2-shade-scrolled-pixel.png", &buf, w, h);
    assert_ne!(buf, before, "the list scrolled");
    assert_eq!(r.take_notes_dismissed(), Vec::<(String, u32)>::new());
    assert_eq!(
        r.take_notes_opened(),
        Vec::<(String, u32, u16)>::new(),
        "a scroll opens nothing"
    );

    // a drag that starts in a gap between cards scrolls back down
    let before = buf.clone();
    let y0 = (GAP_Y * d) as i32;
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y: y0,
        phase: TouchPhase::Down,
    });
    for k in 1..=10 {
        r.dispatch(InputEvent::Touch {
            id: 0,
            x,
            y: y0 + k * (300.0 * d) as i32 / 10,
            phase: TouchPhase::Move,
        });
    }
    r.dispatch(InputEvent::Touch {
        id: 0,
        x,
        y: y0 + (300.0 * d) as i32,
        phase: TouchPhase::Up,
    });
    common::frame(&mut r, &mut buf, w, "scrolled from a gap");
    assert_ne!(buf, before, "a drag in a gap scrolls the list");

    // Clear all, at the list's end (left of TP4's Manage)
    common::tap(&mut r, (CLEAR_X * d) as i32, (CLEAR_Y * d) as i32);
    common::frame(&mut r, &mut buf, w, "cleared");
    assert!(r.take_notes_cleared(), "Clear all reported");
}

// where things are in dp (measured from the snapshots): See all's row, the group's ▾, a Live and an Alerts card, Clear all
const SEE_ALL_Y: f32 = 226.0;
// the 8 dp gap under the scrolled list's Conversations card (tp2-shade-scrolled-pixel.png)
const GAP_Y: f32 = 357.0;
const MARK_READ_X: f32 = 115.0;
const MARK_READ_Y: f32 = 505.0;
const GROUP_Y: f32 = 291.0;
const LIVE_Y: f32 = 162.0;
const ALERT_Y: f32 = 515.0;
const CLEAR_Y: f32 = 666.0;
const CLEAR_X: f32 = 125.0;

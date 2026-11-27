// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::page::{CALL_ANSWER, CALL_REJECT, NOTIFY_ONGOING, NOTIFY_STICKY, action, style};
use neoui_render::Notification;

const NOW: u64 = 1_795_691_635_000;

/// NEOPHONE's ringing card (TP5, O1): Urgent, Call style, sticky; its own actions are Silence and Reply with message.
pub fn ringing() -> Notification {
    Notification {
        id: 1,
        title: "Ava Lindqvist".into(),
        text: "+1 555 0142 · Mobile".into(),
        style: style::CALL,
        flags: NOTIFY_STICKY | NOTIFY_ONGOING,
        channel: 1,
        initials: "AL".into(),
        tint: 0x22b46a,
        actions: vec!["Silence".into(), "Reply with message".into()],
        action_kinds: vec![action::IN_PLACE, action::REPLY],
        when: NOW,
        ..Notification::default()
    }
}

/// TP5 in the shell (nt-call-banner): a ringing call while the phone is in use is the heads-up call banner with the
/// shell's round Reject and Answer (reported as 0xA1 / 0xA0, O1) and the card's Silence · Reply with message; it has
/// no bar (it doesn't time out); a tap on it opens the card (the call screen). Snapshot `nt-call-banner.png`.
#[test]
fn the_call_banner() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    let call = ringing();
    let list: Vec<(&str, &Notification)> = vec![("phone", &call)];
    r.show("home", "");
    r.set_notes(&list, NOW, 0, true);
    r.set_banner(Some(("phone", 1)));
    r.set_banner_seconds(0);
    common::frame(&mut r, &mut buf, w, "call banner");
    common::write_png("nt-call-banner.png", &buf, w, h);
    let b = r.banner_rect().expect("the banner shows");
    assert!(b.h as f32 > 100.0 * d && (b.h as f32) < 170.0 * d, "{b:?}");
    // the round keys at the right of the body row: Reject, then Answer at the edge
    let at = |x: f32, y: f32| ((x * d) as i32, (y * d) as i32);
    let (x, y) = at(314.0, 93.0);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "answered");
    assert_eq!(
        r.take_notes_opened(),
        vec![("phone".into(), 1, CALL_ANSWER)]
    );
    r.set_banner(Some(("phone", 1)));
    common::frame(&mut r, &mut buf, w, "again");
    let (x, y) = at(254.0, 93.0);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "rejected");
    assert_eq!(
        r.take_notes_opened(),
        vec![("phone".into(), 1, CALL_REJECT)]
    );
    // the body: the call screen
    r.set_banner(Some(("phone", 1)));
    common::frame(&mut r, &mut buf, w, "again");
    let (x, y) = at(130.0, 93.0);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "opened");
    assert_eq!(r.take_notes_opened(), vec![("phone".into(), 1, 0)]);
}

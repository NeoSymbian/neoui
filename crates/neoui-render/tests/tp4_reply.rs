// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::page::{action, style};
use neoui_render::Notification;

const NOW: u64 = 1_795_691_635_000;

fn ava() -> Notification {
    Notification {
        id: 2,
        title: "Ava Lindqvist".into(),
        text: "Yes. 7 at the usual place?\nRunning late, see you at 7?".into(),
        style: style::CONVERSATION,
        channel: 1,
        initials: "AL".into(),
        tint: 0x22b46a,
        actions: vec!["Reply".into(), "Mark read".into(), "Call".into()],
        action_kinds: vec![action::REPLY, action::IN_PLACE],
        when: NOW,
        ..Notification::default()
    }
}

/// TP4's banner (nt-headsup, nt-headsup-buttons) and inline reply (nt-reply): an Urgent conversation's banner with
/// Reply (blue) · Mark read · Call as pills and its time bar; Reply on it (Messages reads REPLY_TEXT) opens the field
/// in the banner with the shell's keyboard, the quick replies and the SMS counter, and covers the app; a quick reply
/// and Send hand the text over (`take_replies`) and close it. For an app that doesn't read REPLY_TEXT, Reply opens
/// the app as before. Snapshots `nt-headsup.png`, `nt-headsup-buttons.png`, `nt-reply.png`.
#[test]
fn banner_and_inline_reply() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let at = |x: f32, y: f32| ((x * d) as i32, (y * d) as i32);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    let ava = ava();
    let list: Vec<(&str, &Notification)> = vec![("messages", &ava)];
    r.show("home", "");
    r.set_notes(&list, NOW, 0, true);
    r.set_banner(Some(("messages", 2)));
    common::frame(&mut r, &mut buf, w, "banner");
    common::write_png("nt-headsup.png", &buf, w, h);

    // without the app reading REPLY_TEXT, Reply opens the app (action 1)
    let (x, y) = at(73.0, 133.0);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "reply opens the app");
    assert_eq!(r.take_notes_opened(), vec![("messages".into(), 2, 1)]);
    assert!(!r.replying());

    // Messages reads it: the field opens in the banner
    r.set_reply_apps(&["messages"]);
    r.set_notes(&list, NOW, 0, true);
    r.set_banner(Some(("messages", 2)));
    common::frame(&mut r, &mut buf, w, "banner again");
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "reply field");
    assert!(r.take_notes_opened().is_empty());
    assert!(r.replying(), "the field is open");
    assert!(r.overlay_open(), "it covers the app");
    assert!(r.keyboard_h() > 0, "with the shell's keyboard");
    // the quick reply "On my way" (its row under the field), then Send (right of the field)
    let (x, y) = at(120.0, 270.0);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "quick reply");
    common::write_png("nt-reply.png", &buf, w, h);
    let (x, y) = at(316.0, 222.0);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "sent");
    assert_eq!(
        r.take_replies(),
        vec![("messages".into(), 2, "On my way".into())]
    );
    assert!(!r.replying() && !r.banner_shown());
    assert_eq!(r.keyboard_h(), 0, "the keyboard went with it");

    // Reply on the shade's card: the banner takes it with the field open
    common::tap(&mut r, (w / 2) as i32, (12.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "shade");
    common::write_png("tp4-reply-shade.png", &buf, w, h);
    let (x, y) = at(45.0, 368.0);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "reply from the shade");
    assert!(r.replying() && r.banner_shown());
    assert!(r.take_notes_opened().is_empty());
    common::write_png("tp4-reply-from-shade.png", &buf, w, h);
    // put away: a swipe sideways closes the field and its keyboard
    r.set_banner(None);
    common::frame(&mut r, &mut buf, w, "closed");
    assert!(!r.replying() && !r.overlay_open());
    assert_eq!(
        r.keyboard_h(),
        0,
        "the field's keyboard went with the banner"
    );

    // Buttons mode (the key row under it)
    common::keys(&mut r, w, h);
    r.show("home", "");
    r.set_notes(&list, NOW, 0, true);
    r.set_banner(Some(("messages", 2)));
    common::frame(&mut r, &mut buf, w, "banner, buttons");
    common::write_png("nt-headsup-buttons.png", &buf, w, h);
}

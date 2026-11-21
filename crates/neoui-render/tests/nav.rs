// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

// where the first shade entry's actions sit (dp): the row under its text, Call after Reply
const NOTE_ACTIONS_Y: f32 = 332.0;
const NOTE_CALL_X: f32 = 194.0;

use neoui_proto::settings::Value;
use neoui_render::{Nav, Note, Notification, Page, Query, Renderer};

fn press(r: &mut Renderer, buf: &mut [u32], x: i32, y: i32) {
    common::tap(r, x, y);
    common::settle();
    r.render(buf, 539);
}

fn s(t: &str) -> String {
    t.into()
}

/// Navigation v2 (docs neoui-shell/navigation.md, design canvas Navigation): the bottom is Buttons' key row (← ⌂ ▢, 56 dp) or
/// the gesture band (16 dp), the same for the shell's screens and apps, so an app has 639 dp or 679 dp. Back goes
/// into the app while its page says so, else out of it; a dialog takes Back first; the toast's action and the
/// shade's app entries come back to the server.
#[test]
fn the_v2_chrome() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let d = common::dp(w, h);
    let at = common::keys(&mut r, w, h);

    // the body: 25 dp status bar, then 56 dp of keys (Buttons) or 16 dp of band (Gestures)
    let buttons_h = r.app_body().h as f32 / d;
    assert!(
        (buttons_h - (h as f32 / d - 25.0 - 56.0)).abs() < 1.5,
        "{buttons_h}"
    );
    for mode in [1, 2] {
        assert!(r.apply_setting("nav.mode", &Value::Int(mode)));
        let gestures_h = r.app_body().h as f32 / d;
        assert!((gestures_h - buttons_h - 40.0).abs() < 1.5, "{gestures_h}");
        assert_eq!(r.body(), r.app_body(), "shell screens and apps match");
    }
    let at2 = common::keys(&mut r, w, h);
    assert_eq!(at2.y, at.y);

    r.show("files", "");
    press(&mut r, &mut buf, w as i32 / 2, h as i32 / 2); // a settled first frame
    r.set_app_front(true); // the server says so whenever an app process is in front
    r.set_app_page(&Page {
        title: s("Files"),
        can_back: true,
        ..Page::default()
    });
    press(&mut r, &mut buf, at.back, at.y);
    assert_eq!(r.take_nav(), [Nav::Back]);
    assert_eq!(r.screen(), "files", "Back inside the app must not navigate");

    // a dialog: Back answers its last button
    r.set_app_query(Some(&Query {
        text: s("Delete 2 files?"),
        buttons: vec![s("Delete"), s("Cancel")],
        destructive: true,
    }));
    assert!(r.overlay_open());
    common::frame(&mut r, &mut buf, w, "query");
    common::write_png("keys-query-539x1079.png", &buf, w, h);
    press(&mut r, &mut buf, at.back, at.y);
    assert_eq!(r.take_nav(), [Nav::Answer(1)]);
    assert!(!r.overlay_open(), "answering closes it");

    // the toast, 12 dp above the key row; its action goes to the app
    r.set_note(Some(&Note {
        text: s("2 files deleted"),
        action: s("Undo"),
    }));
    common::frame(&mut r, &mut buf, w, "toast");
    common::write_png("keys-toast-539x1079.png", &buf, w, h);
    let undo = (
        (w as f32 - 12.0 * d - 30.0 * d) as i32,
        (h as f32 - 56.0 * d - 12.0 * d - 24.0 * d) as i32,
    );
    press(&mut r, &mut buf, undo.0, undo.1);
    assert_eq!(r.take_nav(), [Nav::NoteAction]);
    r.set_note(None);

    // nowhere to go back to inside the app: ← leaves it
    r.set_app_page(&Page::default());
    press(&mut r, &mut buf, at.back, at.y);
    assert!(r.take_nav().is_empty());
    assert_ne!(
        r.screen(),
        "files",
        "Back at the app's first page leaves it"
    );
    r.set_app_front(false);

    // the legacy bridge (no kit): the shell's header over it, ← while it keeps Back, ⋮ for its Options key
    r.show("legacy", "");
    r.set_app_front(true);
    r.set_app_page(&Page {
        can_back: true,
        ..Page::default()
    });
    r.set_app_legacy(true, true);
    common::frame(&mut r, &mut buf, w, "legacy header");
    common::write_png("keys-legacy-header-539x1079.png", &buf, w, h);
    let header_y = ((25.0 + 24.0) * d) as i32;
    press(&mut r, &mut buf, (w as f32 - 28.0 * d) as i32, header_y);
    assert_eq!(r.take_nav(), [Nav::Options]);
    press(&mut r, &mut buf, (24.0 * d) as i32, header_y);
    assert_eq!(r.take_nav(), [Nav::Back]);
    r.set_app_legacy(false, false);
    r.set_app_front(false);

    // a full-screen page hides the status bar and the bottom; a tap (the server) shows the key row for a moment
    r.show("files", "");
    r.set_app_front(true);
    r.set_app_page(&Page {
        full: true,
        ..Page::default()
    });
    common::settle();
    r.render(&mut buf, w as usize);
    r.set_key_row_peek(true);
    common::settle();
    r.render(&mut buf, w as usize);
    common::write_png("keys-full-peek-539x1079.png", &buf, w, h);
    r.set_key_row_peek(false);
    r.set_app_page(&Page::default());
    r.set_app_front(false);

    // the shell's screens: the kit's header with the screen's name; ← in a sub-view goes up
    r.show("settings", "display");
    common::frame(&mut r, &mut buf, w, "settings display");
    common::write_png("keys-settings-display-539x1079.png", &buf, w, h);
    press(
        &mut r,
        &mut buf,
        (24.0 * d) as i32,
        ((25.0 + 24.0) * d) as i32,
    );
    assert_eq!((r.screen().as_str(), r.view().as_str()), ("settings", ""));

    // ⌂ and ▢
    press(&mut r, &mut buf, at.home, at.y);
    assert_eq!(r.screen(), "home");
    common::frame(&mut r, &mut buf, w, "home buttons");
    common::write_png("home-buttons-539x1079.png", &buf, w, h);
    press(&mut r, &mut buf, at.apps, at.y);
    assert_eq!(r.screen(), "switcher");

    // an app's entry in the shade: a tap reports its index and closes the shade
    r.show("home", "");
    let ava = Notification {
        title: "Ava Lindqvist".into(),
        text: "Perfect. I'll book a table.\nRunning late, see you at 7?".into(),
        actions: vec!["Reply".into(), "Call".into()],
        initials: "AL".into(),
        tint: 0x22b46a,
        time: "10:05".into(),
        ..Notification::default()
    };
    let timer = Notification {
        title: "Timer done · 05:00".into(),
        text: "Clock · Time's up".into(),
        ..Notification::default()
    };
    r.set_notes(&[&ava, &timer]);
    common::frame(&mut r, &mut buf, w, "notes");
    press(&mut r, &mut buf, w as i32 / 2, (12.0 * d) as i32);
    assert!(r.overlay_open(), "the shade is open");
    common::frame(&mut r, &mut buf, w, "shade");
    common::write_png("shade-notes-539x1079.png", &buf, w, h);
    // its second action (Call) under the first entry: reported as (entry 0, action 2), and the shade closes
    press(
        &mut r,
        &mut buf,
        (NOTE_CALL_X * d) as i32,
        (NOTE_ACTIONS_Y * d) as i32,
    );
    assert_eq!(r.take_notes_opened(), [(0, 2)]);
    assert!(!r.overlay_open(), "an action closes the shade");

    // badges on the Apps menu (and the dock): Phone 2, Messages 3 (MODEMD's count while the app posts none), Clock 1
    r.set_unread_messages(3, "Ava: on my way");
    r.set_badges(&[("phone", 2), ("clock", 1)]);
    r.show("menu", "");
    common::frame(&mut r, &mut buf, w, "menu badges");
    common::write_png("menu-badges-539x1079.png", &buf, w, h);

    // Gestures: the band and the dock's Apps
    assert!(r.apply_setting("nav.mode", &Value::Int(1)));
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home gestures");
    common::write_png("home-gestures-539x1079.png", &buf, w, h);
    r.show("switcher", "");
    r.set_running(&["files", "clock"]);
    common::frame(&mut r, &mut buf, w, "switcher gestures");
    common::write_png("switcher-gestures-539x1079.png", &buf, w, h);
}

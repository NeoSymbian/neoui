// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::settings::Value;
use neoui_render::{Action, Nav, Page, Query, Renderer};

fn press(r: &mut Renderer, buf: &mut [u32], x: i32, y: i32) {
    common::tap(r, x, y);
    common::settle();
    r.render(buf, 539);
}

fn s(t: &str) -> String {
    t.into()
}

/// Five places (docs gestures.md): the shell draws the strip from the app's page and reports taps on it; Back
/// closes the sheet, then the dialog, then goes up the app's page, else leaves the app. ⋮ shows only with options.
#[test]
fn the_strip_follows_the_page() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let at = common::strip(w, h);
    let d = common::dp(w, h);

    r.show("files", "");
    press(&mut r, &mut buf, w as i32 / 2, h as i32 / 2); // a settled first frame
    r.set_app_front(true); // the server says so whenever an app process is in front
    let page = Page {
        title: s("Files"),
        can_back: true,
        options: vec![s("New folder"), s("Delete")],
        ..Page::default()
    };
    r.set_app_page(&page, false);
    press(&mut r, &mut buf, at.back, at.y);
    assert_eq!(r.take_nav(), [Nav::Back]);
    assert_eq!(r.screen(), "files", "Back inside the app must not navigate");

    // ⋮ opens the shell's sheet with the page's items; the first item is 98 dp above the body's bottom
    press(&mut r, &mut buf, at.options, at.y);
    assert!(r.overlay_open(), "the sheet covers the app");
    assert!(r.take_nav().is_empty());
    common::frame(&mut r, &mut buf, w, "app options");
    common::write_png("strip-options-539x1079.png", &buf, w, h);
    press(
        &mut r,
        &mut buf,
        w as i32 / 2,
        (h as f32 - 56.0 * d - 98.0 * d) as i32,
    );
    assert_eq!(r.take_nav(), [Nav::Option(0)]);
    assert!(!r.overlay_open());
    // Back closes an open sheet before anything else
    press(&mut r, &mut buf, at.options, at.y);
    press(&mut r, &mut buf, at.back, at.y);
    assert!(r.take_nav().is_empty());
    assert!(!r.overlay_open());

    // a dialog: Back answers its last button
    r.set_app_query(Some(&Query {
        text: s("Delete 2 files?"),
        buttons: vec![s("Delete"), s("Cancel")],
        destructive: true,
    }));
    assert!(r.overlay_open());
    common::frame(&mut r, &mut buf, w, "query");
    common::write_png("strip-query-539x1079.png", &buf, w, h);
    // the dialog covers the strip; Back is the edge (or hardware) one
    r.gesture(neoui_proto::gesture::Gesture::Back);
    assert_eq!(r.take_nav(), [Nav::Answer(1)]);
    assert!(!r.overlay_open(), "answering closes it");

    // the primary pill (no ⋮: it ends the strip) and a form's Cancel (← ⌂ ▢ on the right, the page's group left)
    r.set_app_page(
        &Page {
            title: s("New contact"),
            can_back: true,
            primary: Some(Action {
                icon: 5,
                label: s("Save"),
            }),
            ..Page::default()
        },
        false,
    );
    press(&mut r, &mut buf, at.last, at.y);
    assert_eq!(r.take_nav(), [Nav::Primary]);
    assert!(r.apply_setting("nav.strip", &Value::Int(1)));
    r.set_app_page(
        &Page {
            title: s("New contact"),
            can_back: true,
            cancel: true,
            primary: Some(Action {
                icon: 5,
                label: s("Save"),
            }),
            ..Page::default()
        },
        false,
    );
    common::frame(&mut r, &mut buf, w, "form, system right");
    common::write_png("strip-form-right-539x1079.png", &buf, w, h);
    press(&mut r, &mut buf, at.first, at.y);
    assert_eq!(r.take_nav(), [Nav::Cancel]);
    assert!(r.apply_setting("nav.strip", &Value::Int(0)));

    // an older app that draws its own Options: ⋮ is its event
    r.set_app_page(&Page::default(), true);
    press(&mut r, &mut buf, at.options, at.y);
    assert_eq!(r.take_nav(), [Nav::Options]);
    assert!(!r.overlay_open());

    // nowhere to go back to inside the app: ← (dimmed) leaves it
    r.set_app_page(&Page::default(), false);
    press(&mut r, &mut buf, at.back, at.y);
    assert!(r.take_nav().is_empty());
    assert_ne!(
        r.screen(),
        "files",
        "Back at the app's first page leaves it"
    );
    r.set_app_front(false);

    // ⌂ and ▢
    r.show("settings", "display");
    press(&mut r, &mut buf, at.home, at.y);
    assert_eq!(r.screen(), "home");
    press(&mut r, &mut buf, at.apps, at.y);
    assert_eq!(r.screen(), "switcher");
    strip_snapshots(&mut r, &mut buf, w, h);
}

/// The strip in every arrangement and both modes, for the eye (snapshots); Classic keeps it on home (Apps), Gestures
/// home has none and the body grows; an app's full-screen page hides the status bar and the strip.
fn strip_snapshots(r: &mut Renderer, buf: &mut [u32], w: u32, h: u32) {
    r.show("files", "");
    common::frame(r, buf, w, "files");
    r.set_app_front(true);
    let page = Page {
        title: s("Contacts"),
        primary: Some(Action {
            icon: 3,
            label: s("New"),
        }),
        options: vec![s("Mark several"), s("Import vCards")],
        dots: Some((2, 1)),
        ..Page::default()
    };
    r.set_app_page(&page, false);
    for (mode, strip, name) in [
        (0, 0, "left"),
        (0, 1, "right"),
        (0, 2, "split"),
        (1, 0, "gestures"),
    ] {
        assert!(r.apply_setting("nav.mode", &Value::Int(mode)));
        assert!(r.apply_setting("nav.strip", &Value::Int(strip)));
        common::frame(r, buf, w, name);
        common::write_png(&format!("strip-{name}-539x1079.png"), buf, w, h);
    }
    let form = Page {
        title: s("New contact"),
        can_back: true,
        cancel: true,
        primary: Some(Action {
            icon: 5,
            label: s("Save"),
        }),
        ..Page::default()
    };
    r.set_app_page(&form, false);
    common::frame(r, buf, w, "form gestures");
    common::write_png("strip-form-gestures-539x1079.png", buf, w, h);
    assert!(r.apply_setting("nav.mode", &Value::Int(0)));
    common::frame(r, buf, w, "form classic");
    common::write_png("strip-form-left-539x1079.png", buf, w, h);

    let body = r.body();
    r.set_app_page(
        &Page {
            full: true,
            ..Page::default()
        },
        false,
    );
    common::settle();
    r.render(buf, w as usize); // all black: the server draws the app over it
    assert!(!r.strip_shown(), "a full-screen page hides the strip");
    r.set_app_page(&Page::default(), false);
    assert!(r.strip_shown());
    assert_eq!(r.body(), body, "and gives it back");
    r.set_app_front(false);

    r.show("home", "");
    common::frame(r, buf, w, "home classic");
    assert!(r.strip_shown(), "Classic keeps the strip on home");
    common::write_png("home-classic-539x1079.png", buf, w, h);
    assert!(r.apply_setting("nav.mode", &Value::Int(1)));
    common::frame(r, buf, w, "home gestures");
    assert!(!r.strip_shown(), "Gestures home has none");
    assert!(r.body().h > body.h);
    for (screen, view) in [("menu", ""), ("settings", ""), ("settings", "display")] {
        r.show(screen, view);
        common::frame(r, buf, w, screen);
        assert!(!r.strip_shown(), "Gestures: no empty strip on {screen} {view}");
    }
    r.show("switcher", "");
    common::frame(r, buf, w, "switcher");
    assert!(r.strip_shown(), "Open apps has Home");
    assert!(r.apply_setting("nav.mode", &Value::Int(0)));
}

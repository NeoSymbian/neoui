// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

use neoui_proto::gesture::Gesture;
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;

fn touch(r: &mut Renderer, (x, y): (i32, i32), phase: TouchPhase) {
    r.dispatch(InputEvent::Touch { id: 0, x, y, phase });
}

/// Home's page slide as the UI server runs it on the Pixel: a press on the pages, then the server takes the touch over
/// at its first move (`cancel_touch`), and Slint never sees the release. Then a swipe up opens the Apps menu.
fn slide_then_menu(r: &mut Renderer, buf: &mut [u32], w: u32, at: (i32, i32)) {
    r.show("home", "");
    r.set_home_page(0);
    common::frame(r, buf, w, "home");
    touch(r, at, TouchPhase::Down);
    r.cancel_touch();
    r.set_home_page(1);
    common::frame(r, buf, w, "slid");
    r.gesture(Gesture::Home);
    common::frame(r, buf, w, "menu");
    assert_eq!(r.screen(), "menu");
}

/// The swipe of a home page used to leave Slint's press pending (the pages' swipe handler holds a press back 100 ms
/// before its children see it, and a cancel in that time was dropped), so the press reached home's widget later and
/// grabbed the pointer: the next touches, on the Apps menu, went to home (its long-press menu, the Music card).
#[test]
fn the_menu_gets_the_touches_after_a_home_slide() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.set_server_transitions(true);

    // a tap on Settings (the 4th row's middle) opens Settings, after a slide that started on the agenda card
    slide_then_menu(&mut r, &mut buf, w, (950, 900));
    let settings = (540, 1615);
    touch(&mut r, settings, TouchPhase::Down);
    common::advance(80);
    r.render(&mut buf, w as usize);
    touch(&mut r, settings, TouchPhase::Up);
    common::frame(&mut r, &mut buf, w, "tapped");
    assert_eq!(r.screen(), "settings");

    // a long press on Clock gives its app menu (Add to home), after a slide that started on the music card
    slide_then_menu(&mut r, &mut buf, w, (540, 1615));
    let _ = r.take_setting_edits();
    let clock = (900, 1210);
    touch(&mut r, clock, TouchPhase::Down);
    for _ in 0..40 {
        common::advance(17);
        r.render(&mut buf, w as usize);
    }
    touch(&mut r, clock, TouchPhase::Up);
    common::frame(&mut r, &mut buf, w, "held");
    assert!(
        !r.home_editing() && r.screen() == "menu",
        "{} {}",
        r.home_editing(),
        r.screen()
    );
    // its one item, under the finger
    common::tap(&mut r, 700, 1300);
    common::frame(&mut r, &mut buf, w, "added");
    let edits = r.take_setting_edits();
    assert!(
        edits.iter().any(|(k, _)| *k == "home.layout"),
        "Add to home: {edits:?}"
    );
}

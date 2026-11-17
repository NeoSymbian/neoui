// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;

/// A sideways drag across the home pages, in 8 moves, `dx` px.
fn drag(r: &mut Renderer, buf: &mut [u32], w: u32, y: i32, dx: i32) {
    let x0 = w as i32 / 2 - dx / 2;
    let mut send = |r: &mut Renderer, x, phase| {
        r.dispatch(InputEvent::Touch { id: 0, x, y, phase });
        common::advance(17);
        r.render(buf, w as usize);
    };
    send(r, x0, TouchPhase::Down);
    for k in 1..=8 {
        send(r, x0 + dx * k / 8, TouchPhase::Move);
    }
    send(r, x0 + dx, TouchPhase::Up);
    common::settle();
    r.render(buf, w as usize);
}

/// With the shade open over home, a sideways drag stays in the shade: the home page doesn't change under it and the
/// shade stays open. (The UI server's own page slide checks `overlay_open()` before it takes a press.)
#[test]
fn the_shade_keeps_sideways_drags_from_the_home_pages() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let d = common::dp(w, h);
    let y = (400.0 * d) as i32; // on the home pages, below the status bar

    r.show("home", "");
    common::settle();
    r.render(&mut buf, w as usize);
    // the drag itself works: without the shade it turns the page
    drag(&mut r, &mut buf, w, y, -(200.0 * d) as i32);
    assert_eq!(r.home_page(), 1);

    common::tap(&mut r, w as i32 / 2, (12.0 * d) as i32); // the status bar opens the shade
    common::settle();
    r.render(&mut buf, w as usize);
    assert!(r.overlay_open());
    // on the panel (it is as tall as its rows: the time, the toggles, the slider)
    let on_panel = (190.0 * d) as i32;
    drag(&mut r, &mut buf, w, on_panel, -(200.0 * d) as i32);
    drag(&mut r, &mut buf, w, on_panel, (200.0 * d) as i32);
    assert_eq!(
        r.home_page(),
        1,
        "a drag on the shade must not turn the home page"
    );
    assert!(r.overlay_open(), "the shade stays open");
}

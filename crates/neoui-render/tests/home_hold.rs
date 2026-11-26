// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::{InputEvent, TouchPhase};

/// Home's long press (D20, Edit home · Wallpaper) opens on a held finger, but not on one the UI server took over: the
/// pull down to the shade (top-panel TP1, `stop_home_hold`) or its page slide (`cancel_touch`). Both used to leave
/// Slint's press held, and the menu opened half a second later (over the shade, on the Pixel).
#[test]
fn long_press_only_when_held_still() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");
    let calm = buf.clone();
    // the empty part of page 0, between the music card and the page dots
    let (x, y) = (540, 1680);
    let touch = |r: &mut neoui_render::Renderer, y: i32, phase| {
        r.dispatch(InputEvent::Touch { id: 0, x, y, phase });
    };

    let differ = |a: &[u32], b: &[u32]| a.iter().zip(b).filter(|(p, q)| p != q).count();

    // a pull down the server recognised, held: no menu
    touch(&mut r, y, TouchPhase::Down);
    touch(&mut r, y + 60, TouchPhase::Move);
    r.stop_home_hold();
    common::frame(&mut r, &mut buf, w, "pulled");
    assert_eq!(differ(&buf, &calm), 0, "a pull opens nothing");
    r.cancel_touch();
    touch(&mut r, y + 60, TouchPhase::Up);
    common::frame(&mut r, &mut buf, w, "pulled, released");

    // taken over by the UI server and held: no menu
    touch(&mut r, y, TouchPhase::Down);
    r.cancel_touch();
    common::frame(&mut r, &mut buf, w, "taken over");
    assert_eq!(
        differ(&buf, &calm),
        0,
        "a press the server took over opens nothing"
    );
    touch(&mut r, y, TouchPhase::Up);
    common::frame(&mut r, &mut buf, w, "taken over, released");

    // held still: the menu
    touch(&mut r, y, TouchPhase::Down);
    common::frame(&mut r, &mut buf, w, "held");
    assert!(
        differ(&buf, &calm) > 0,
        "a held press opens Edit home · Wallpaper"
    );
    touch(&mut r, y, TouchPhase::Up);
}

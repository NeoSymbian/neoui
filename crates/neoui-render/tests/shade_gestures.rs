// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

use neoui_proto::gesture::{Edge, Gesture, live_edges};

/// The shade is transient UI over the screen: the edges still work while it is open (Back closes its Wi-Fi sheet,
/// then the shade; Home closes it), and the screen going off closes it, sheet and all.
#[test]
fn edge_gestures_and_screen_off_close_the_shade() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let px = |v: f32| (v * d) as i32;
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];

    // the edges the UI server gives gestures: home keeps its sides for its pages, but not under the shade
    assert_eq!(live_edges(1, true, false, false), [Edge::Bottom]);
    assert!(live_edges(1, true, true, false).contains(&Edge::Left));
    assert!(live_edges(2, true, true, false).contains(&Edge::Left));
    assert!(
        !live_edges(1, false, true, true).contains(&Edge::Top),
        "the status bar's drag down is the shade's own while it is open"
    );
    assert!(live_edges(0, false, true, false).is_empty(), "Buttons");

    // the control centre with its Wi-Fi sheet (the Wi-Fi tile's ›)
    r.show("home", "");
    r.open_control();
    common::frame(&mut r, &mut buf, w, "control");
    common::tap(&mut r, px(12.0 + 164.0 - 18.0), px(85.0 + 32.0));
    common::frame(&mut r, &mut buf, w, "sheet");
    assert!(r.wifi_sheet_open());

    // Back: the sheet, then the shade
    r.gesture(Gesture::Back);
    common::frame(&mut r, &mut buf, w, "back 1");
    assert!(!r.wifi_sheet_open() && r.control_open());
    r.gesture(Gesture::Back);
    common::frame(&mut r, &mut buf, w, "back 2");
    assert!(!r.overlay_open());
    assert_eq!(r.screen(), "home");

    // Home (a swipe up) closes the shade and stays where it was: not the Apps menu from home
    r.open_shade();
    common::frame(&mut r, &mut buf, w, "shade");
    r.gesture(Gesture::Home);
    common::frame(&mut r, &mut buf, w, "home");
    assert!(!r.overlay_open());
    assert_eq!(r.screen(), "home");
    r.show("clock", "");
    r.open_shade();
    r.gesture(Gesture::Home);
    common::frame(&mut r, &mut buf, w, "home from clock");
    assert!(!r.overlay_open());

    // the screen goes off: the shade closes, and opens again at its first stop without the sheet
    r.show("home", "");
    r.open_control();
    common::frame(&mut r, &mut buf, w, "control again");
    common::tap(&mut r, px(12.0 + 164.0 - 18.0), px(85.0 + 32.0));
    common::frame(&mut r, &mut buf, w, "sheet again");
    assert!(r.wifi_sheet_open());
    r.close_shade();
    common::frame(&mut r, &mut buf, w, "screen off");
    assert!(!r.overlay_open());
    r.open_shade();
    common::frame(&mut r, &mut buf, w, "woken");
    assert!(r.overlay_open() && !r.control_open() && !r.wifi_sheet_open());
}

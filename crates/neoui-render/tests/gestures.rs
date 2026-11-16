// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::gesture::Gesture;
use neoui_proto::settings::Value;

/// Edge gestures (docs gestures.md G1) do what the toolbar and the menus do, and gesture mode hides the Back arrow
/// but leaves its place empty.
#[test]
fn gestures_take_the_shells_paths() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];

    // Back: a claimed Back goes to the app, else the shell goes back
    r.show("files", "");
    r.set_app_front(true);
    r.set_app_nav(true, false);
    r.gesture(Gesture::Back);
    assert_eq!(r.take_nav(), [1]);
    assert_eq!(r.screen(), "files", "a claimed Back must not navigate");
    r.set_app_nav(false, false);
    r.gesture(Gesture::Back);
    assert!(r.take_nav().is_empty());
    assert_eq!(r.screen(), "home");
    r.set_app_front(false);

    // a sub-view goes back to its root first
    r.show("settings", "display");
    r.gesture(Gesture::Back);
    assert_eq!((r.screen().as_str(), r.view().as_str()), ("settings", ""));

    // Open apps from anywhere, and Back returns where it was opened from
    r.show("menu", "");
    r.gesture(Gesture::OpenApps);
    assert_eq!(r.screen(), "switcher");
    r.gesture(Gesture::OpenApps);
    assert_eq!(r.screen(), "switcher");
    r.gesture(Gesture::Back);
    assert_eq!(r.screen(), "menu");

    // the bottom edge: Home, from home the Apps menu, and the long swipe the Apps menu
    r.show("clock", "");
    r.gesture(Gesture::Home);
    assert_eq!(r.screen(), "home");
    r.gesture(Gesture::Home);
    assert_eq!(r.screen(), "menu");
    r.gesture(Gesture::Home);
    assert_eq!(r.screen(), "home");
    r.show("clock", "");
    r.gesture(Gesture::AppsMenu);
    assert_eq!(r.screen(), "menu");

    // gesture mode: the toolbar's Back arrow is gone (a tap there does nothing), Options stays
    assert!(r.apply_setting("nav.mode", &Value::Int(1)));
    common::snapshot(&mut r, &mut buf, "settings", "navigation", "pixel", w, h);
    let d = common::dp(w, h);
    let y = (h as f32 - 30.5 * d) as i32;
    common::tap(&mut r, (58.0 * d) as i32, y);
    common::settle();
    r.render(&mut buf, w as usize);
    assert_eq!(
        (r.screen().as_str(), r.view().as_str()),
        ("settings", "navigation")
    );
    assert!(r.apply_setting("nav.mode", &Value::Int(0)));
    common::snapshot(
        &mut r,
        &mut buf,
        "settings",
        "navigation",
        "classic-pixel",
        w,
        h,
    );
}

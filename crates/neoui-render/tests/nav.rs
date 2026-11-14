// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

/// An app that claims the toolbar's Back and Options gets them through `take_nav`; the shell doesn't act on them.
/// Unclaimed, Back leaves the screen as before (Options does nothing: an app in front owns it).
#[test]
fn claimed_toolbar_buttons_go_to_the_app() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let d = common::dp(w, h);
    // ui/components/frame.slint: the toolbar's 116 dp buttons at both ends, the bar 61 dp high
    let y = (h as f32 - 30.5 * d) as i32;
    let (back, options) = ((58.0 * d) as i32, (w as f32 - 58.0 * d) as i32);
    let mut press = |r: &mut neoui_render::Renderer, x| {
        common::tap(r, x, y);
        common::settle();
        r.render(&mut buf, w as usize);
    };

    r.show("files", "");
    press(&mut r, w as i32 / 2); // a settled first frame
    r.set_app_front(true); // the server says so whenever an app process is in front
    r.set_app_nav(true, true);
    press(&mut r, back);
    press(&mut r, options);
    assert_eq!(r.take_nav(), [1, 2]);
    assert_eq!(r.screen(), "files", "a claimed Back must not navigate");

    r.set_app_nav(false, false);
    press(&mut r, back);
    assert!(r.take_nav().is_empty());
    assert_ne!(r.screen(), "files", "an unclaimed Back leaves the screen");
}

// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

/// An app that claims the toolbar's Back and Options gets them through `take_nav`; the shell doesn't act on them.
/// Unclaimed, Back leaves the screen as before.
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
    r.set_app_nav(true, true);
    press(&mut r, back);
    press(&mut r, options);
    assert_eq!(r.take_nav(), [1, 2]);
    assert_eq!(r.screen(), "files", "a claimed Back must not navigate");

    r.set_app_nav(false, false);
    press(&mut r, back);
    assert!(r.take_nav().is_empty());
    assert_ne!(r.screen(), "files", "an unclaimed Back leaves the screen");

    // C7: an old close timer cannot dismiss a replacement/remaining call. Once every call ends it closes normally.
    r.set_call("Connected", "+37400000000", false);
    r.set_call_audio(false, false, 8);
    r.set_calls(&[neoui_render::CallView {
        id: 2,
        number: "+37400000000",
        state: "Connected",
        incoming: false,
        held: false,
        waiting: false,
    }]);
    r.show_call();
    common::frame(&mut r, &mut buf, w, "active call");
    let active = buf.clone();
    r.close_call();
    common::frame(&mut r, &mut buf, w, "active call retained");
    assert_eq!(
        buf, active,
        "close must preserve controls while a call remains"
    );

    r.set_calls(&[]);
    r.close_call();
    common::frame(&mut r, &mut buf, w, "call ended");
    assert_ne!(buf, active, "the last call ending returns to the dialer");

    // A delayed call close must never dismiss another screen's sub-view.
    r.show("settings", "connectivity");
    common::frame(&mut r, &mut buf, w, "settings connectivity");
    let settings = buf.clone();
    r.close_call();
    common::frame(&mut r, &mut buf, w, "settings retained");
    assert_eq!(buf, settings);
}

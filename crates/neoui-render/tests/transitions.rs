// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

/// smooth-ui S2b: with server transitions on, Slint switches screens at once, so one render after a switch leaves
/// the whole new screen in the buffer (the UI server then fades from its snapshot of the old one); S2c: home pages
/// too.
#[test]
fn server_transitions_switch_screens_in_one_render() {
    let (w, h) = (540, 1080);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    common::frame(&mut r, &mut buf, w, "home");

    r.show("menu", "");
    r.render(&mut buf, w as usize);
    assert!(
        r.has_active_animations(),
        "without the flag, Slint fades screens itself"
    );
    common::frame(&mut r, &mut buf, w, "menu");

    r.set_server_transitions(true);
    r.show("home", "");
    let damage = r.render(&mut buf, w as usize);
    assert!(
        !r.has_active_animations(),
        "the switch must not animate in Slint"
    );
    let body = r.body();
    assert_eq!(
        damage.bounds().intersection(&body),
        body,
        "the first render redraws the whole body"
    );
    let once = buf.clone();
    common::settle();
    r.render(&mut buf, w as usize);
    assert!(buf == once, "nothing is left to draw after that one render");

    // smooth-ui S2c: the UI server turns the home page itself; with server transitions on, the pages jump in one
    // render (no Slint slide), inside the reported damage
    common::frame(&mut r, &mut buf, w, "home");
    let strip = r.home_strip();
    assert_eq!((strip.x, strip.y, strip.w), (0, r.body().y, w));
    assert!(strip.h > h / 2 && strip.h < r.body().h, "{strip:?}");
    assert_eq!(r.home_page(), 0);
    let page0 = buf.clone();
    r.set_home_page(1);
    let damage = r.render(&mut buf, w as usize);
    assert!(
        !r.has_active_animations(),
        "the page must not slide in Slint"
    );
    assert_eq!(r.home_page(), 1);
    assert!(buf != page0);
    let b = damage.bounds();
    for i in (0..buf.len()).filter(|&i| buf[i] != page0[i]) {
        let (x, y) = ((i % w as usize) as i32, (i / w as usize) as i32);
        assert!(
            x >= b.x && y >= b.y && x < b.x + b.w as i32 && y < b.y + b.h as i32,
            "pixel {x},{y} changed outside the damage"
        );
    }
    r.set_home_page(9);
    assert_eq!(r.home_page(), neoui_render::HOME_PAGES - 1, "clamped");
}

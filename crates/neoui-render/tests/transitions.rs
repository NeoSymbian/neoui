// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

/// smooth-ui S2b: with server transitions on, Slint switches screens at once, so one render after a switch leaves
/// the whole new screen in the buffer (the UI server then fades from its snapshot of the old one).
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
}

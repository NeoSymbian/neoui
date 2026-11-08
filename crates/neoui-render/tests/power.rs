// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

/// The Power menu: its buttons are 60 dp apart around the screen's centre (ui/components/powermenu.slint), and each
/// tap reports its action; a tap outside the card dismisses it.
#[test]
fn power_menu() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    r.set_power_menu(true);
    common::frame(&mut r, &mut buf, w, "power menu");
    common::write_png("power-539x1079.png", &buf, w, h);
    let d = common::dp(w, h);
    let (cx, cy) = (w as i32 / 2, h as i32 / 2);
    let mut tap = |r: &mut neoui_render::Renderer, x: i32, y: i32| {
        common::tap(r, x, y);
        common::settle();
        r.render(&mut buf, w as usize);
        r.take_power()
    };
    assert_eq!(tap(&mut r, cx, cy - (60.0 * d) as i32), [2]); // Power off
    assert_eq!(tap(&mut r, cx, cy), [0]); // Restart
    assert_eq!(tap(&mut r, cx, cy + (60.0 * d) as i32), [-1]); // Cancel
    assert_eq!(tap(&mut r, cx, 40), [-1]); // outside the card
    assert_eq!(tap(&mut r, cx, cy + (100.0 * d) as i32), [] as [i32; 0]); // inside the card, between buttons: nothing

    r.set_power_menu(false);
    assert_eq!(tap(&mut r, cx, cy), [] as [i32; 0]);
}

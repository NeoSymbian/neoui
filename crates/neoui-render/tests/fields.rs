// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_render::Renderer;

const W: u32 = 539;
const H: u32 = 1079;

/// Centre of the one-unit key at `at` (of 10 per row) in keyboard row `k`, counted up from the bottom row (0); as
/// tests/keyboard.rs.
fn key(r: &Renderer, k: usize, at: f32, units: f32) -> (i32, i32) {
    let d = common::dp(W, H);
    let body = r.body();
    let bottom = (body.y + body.h as i32) as f32;
    (
        ((at + units / 2.0) * W as f32 / 10.0) as i32,
        (bottom - (27.0 + k as f32 * 48.0) * d) as i32,
    )
}

/// The shell's own text fields bring up the system keyboard and take what it types; leaving them hides it.
#[test]
fn shell_fields_use_the_system_keyboard() {
    let mut r = common::renderer(W, H);
    let mut buf = vec![0u32; (W * H) as usize];
    let d = common::dp(W, H);
    let tap = |r: &mut Renderer, buf: &mut Vec<u32>, (x, y): (i32, i32)| {
        common::tap(r, x, y);
        common::settle();
        r.render(buf, W as usize);
    };

    let press = |r: &mut Renderer, buf: &mut Vec<u32>, k, at, units| {
        let at = key(r, k, at, units);
        tap(r, buf, at);
    };

    // Settings -> Wi-Fi with two secured networks; tapping the second opens its sheet, the password field focused
    let mut scan = vec![1u8, 2, 2, 1];
    for (last, ch, dbm, ssid) in [(1u8, 44u8, -46i8, "Home"), (2, 6, -70, "Cafe Guest")] {
        let mut e = vec![2, 0, 0, 0, 0, last, ch, dbm as u8, 1, ssid.len() as u8];
        e.extend_from_slice(ssid.as_bytes());
        e.resize(42, 0);
        scan.extend(e);
    }
    r.set_wifi(&neoui_proto::wifi::Scan::decode(&scan).unwrap());
    r.show("settings", "wifi");
    common::frame(&mut r, &mut buf, W, "wifi list");
    assert_eq!(r.keyboard_h(), 0);
    tap(&mut r, &mut buf, ((W / 2) as i32, (H as f32 * 0.33) as i32));
    common::frame(&mut r, &mut buf, W, "wifi sheet");
    let kb = r.keyboard_h();
    assert!(kb > 0, "the password field brings the keyboard up");

    // a, b, 1, backspace: they go into the field, nothing is left for an app
    press(&mut r, &mut buf, 2, 0.5, 1.0);
    press(&mut r, &mut buf, 1, 5.5, 1.0);
    press(&mut r, &mut buf, 4, 0.0, 1.0);
    press(&mut r, &mut buf, 4, 1.0, 1.0);
    press(&mut r, &mut buf, 1, 8.5, 1.5);
    assert!(
        r.take_typed().is_empty(),
        "shell field keys are not an app's"
    );
    common::frame(&mut r, &mut buf, W, "wifi password");
    common::write_png("wifi-password-539x1079.png", &buf, W, H);

    // Cancel: the left of the two buttons at the card's bottom, just above the keyboard
    let body = r.body();
    let card_bottom = (body.y + body.h as i32 - kb as i32) as f32;
    tap(
        &mut r,
        &mut buf,
        ((W / 4) as i32, (card_bottom - 42.0 * d) as i32),
    );
    common::frame(&mut r, &mut buf, W, "wifi closed");
    assert_eq!(r.keyboard_h(), 0, "closing the sheet hides the keyboard");

    // Leaving the screen hides the keyboard a field opened: the sheet again, then home
    tap(&mut r, &mut buf, ((W / 2) as i32, (H as f32 * 0.33) as i32));
    common::frame(&mut r, &mut buf, W, "wifi sheet again");
    assert!(
        r.keyboard_h() > 0,
        "the password field brings the keyboard up"
    );
    r.show("home", "");
    common::frame(&mut r, &mut buf, W, "home");
    assert_eq!(r.keyboard_h(), 0, "leaving the screen hides the keyboard");
}

// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_render::Renderer;

const W: u32 = 539;
const H: u32 = 1079;

/// Centre of key `i` of the `n` sharing pad row `k`, counted up from the bottom row (0); rows as the letter
/// keyboard's (ui/components/keyboard/keyboard.slint: centred 27 + 48 k dp above the body's bottom).
fn key(r: &Renderer, k: usize, i: usize, n: usize) -> (i32, i32) {
    let d = common::dp(W, H);
    let body = r.body();
    let bottom = (body.y + body.h as i32) as f32;
    (
        ((i as f32 + 0.5) * W as f32 / n as f32) as i32,
        (bottom - (27.0 + k as f32 * 48.0) * d) as i32,
    )
}

fn tap(r: &mut Renderer, buf: &mut [u32], (x, y): (i32, i32)) {
    common::tap(r, x, y);
    common::settle();
    r.render(buf, W as usize);
}

fn h_dp(r: &Renderer) -> u32 {
    (r.keyboard_h() as f32 / common::dp(W, H)).round() as u32
}

/// The number and phone pads (proto::KEYBOARD 3 and 4) type like the letter keyboard; the SIM prompt's PIN field
/// brings up the number pad; the toolbar's keyboard button puts an app's keyboard away.
#[test]
fn pads() {
    let mut r = common::renderer(W, H);
    let mut buf = vec![0u32; (W * H) as usize];
    let d = common::dp(W, H);
    r.show("terminal", "");

    // the number pad: 1 2 3 / 4 5 6 / 7 8 9 / Backspace 0 Done
    r.set_keyboard(3);
    common::frame(&mut r, &mut buf, W, "number pad");
    common::write_png("keyboard-number-539x1079.png", &buf, W, H);
    assert_eq!(h_dp(&r), 198);
    assert_eq!(r.keyboard_h(), r.keyboard_heights()[3]);
    for (k, i) in [(3, 0), (3, 1), (2, 2), (1, 1), (0, 1), (0, 0), (0, 2)] {
        let at = key(&r, k, i, 3);
        tap(&mut r, &mut buf, at);
    }
    assert_eq!(r.take_typed(), [0x31, 0x32, 0x36, 0x38, 0x30, 8, 13]);

    // the phone pad: the digits with their letters, * 0 #, then + p w Backspace Done
    r.set_keyboard(4);
    common::frame(&mut r, &mut buf, W, "phone pad");
    common::write_png("keyboard-phone-539x1079.png", &buf, W, H);
    assert_eq!(h_dp(&r), 246);
    assert_eq!(r.keyboard_h(), r.keyboard_heights()[4]);
    for (k, i, n) in [
        (0, 0, 5),
        (1, 0, 3),
        (1, 2, 3),
        (0, 1, 5),
        (0, 2, 5),
        (3, 1, 3),
        (4, 0, 3),
        (0, 3, 5),
        (0, 4, 5),
    ] {
        let at = key(&r, k, i, n);
        tap(&mut r, &mut buf, at);
    }
    let typed: String = r
        .take_typed()
        .into_iter()
        .map(|c| char::from_u32(c).unwrap())
        .collect();
    assert_eq!(typed, "+*#pw51\u{8}\r");

    // an unknown mode is the text keyboard
    r.set_keyboard(9);
    assert_eq!(r.keyboard_h(), r.keyboard_heights()[1]);
    r.set_keyboard(0);
    assert_eq!(r.keyboard_h(), 0);

    // The SIM prompt: its PIN field (keys: 3) brings up the number pad over the prompt; Done unlocks
    r.set_sim(true, false, Some(3));
    common::frame(&mut r, &mut buf, W, "sim prompt");
    common::write_png("sim-pin-539x1079.png", &buf, W, H);
    assert_eq!(r.keyboard_h(), r.keyboard_heights()[3], "the number pad");
    for (k, i) in [(3, 0), (3, 1), (3, 2), (2, 0), (0, 2)] {
        let at = key(&r, k, i, 3);
        tap(&mut r, &mut buf, at);
    }
    assert!(
        r.take_typed().is_empty(),
        "the PIN is the shell's, not an app's"
    );
    assert_eq!(r.take_phone(), [("pin".into(), "1234".into())]);
    r.set_sim(false, false, None);
    common::frame(&mut r, &mut buf, W, "sim unlocked");
    assert_eq!(r.keyboard_h(), 0, "the pad goes with the prompt");

    // An app editor in front (Cancel | keyboard | Save | Options, 90 dp each) with the text keyboard
    r.set_app_front(true);
    r.set_app_chrome("New contact", 5, 2, "Save");
    r.set_keyboard(1);
    common::frame(&mut r, &mut buf, W, "app editor");
    common::write_png("toolbar-keyboard-539x1079.png", &buf, W, H);
    let kb = r.keyboard_h();
    assert!(kb > 0);
    let kb_button = ((135.0 * d) as i32, (H as f32 - 30.0 * d) as i32);
    tap(&mut r, &mut buf, kb_button);
    assert!(r.keyboard_dismissed());
    assert_eq!(r.keyboard_h(), 0, "put away: the app gets its body back");
    assert_eq!(r.take_nav(), [], "the keyboard button is the shell's");
    tap(&mut r, &mut buf, kb_button);
    assert_eq!(r.keyboard_h(), kb, "and back");
    tap(&mut r, &mut buf, kb_button);
    r.clear_keyboard_dismissed(); // the app asked for the keyboard again
    assert_eq!(r.keyboard_h(), kb);
    // Save is still the action: the third button
    tap(
        &mut r,
        &mut buf,
        ((225.0 * d) as i32, (H as f32 - 30.0 * d) as i32),
    );
    assert_eq!(r.take_nav(), [4]);
}

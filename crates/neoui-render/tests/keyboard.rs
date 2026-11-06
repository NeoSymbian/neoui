// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_render::Renderer;

/// Centre of the key `units_before`..+`units` along `row` of the open keyboard; mirrors ui/components/keyboard.slint
/// (rows 48 dp apart from 6 dp below its top, keys 42 dp high, each row 10 units wide). `symbian/taps.sh` uses the
/// same numbers.
fn key(r: &Renderer, w: u32, h: u32, row: usize, units_before: f32, units: f32) -> (i32, i32) {
    let d = common::dp(w, h);
    let body = r.body();
    let top = (body.y + body.h as i32 - r.keyboard_h() as i32) as f32;
    let unit = w as f32 / 10.0;
    (
        ((units_before + units / 2.0) * unit) as i32,
        (top + (6.0 + row as f32 * 48.0 + 21.0) * d) as i32,
    )
}

#[test]
fn keyboard() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    r.show("terminal", "");
    r.set_keyboard(true);
    common::frame(&mut r, &mut buf, w, "keyboard");
    common::write_png("keyboard-539x1079.png", &buf, w, h);
    assert_eq!(r.keyboard_h(), (198.0 * common::dp(w, h)) as u32);

    let mut tap = |row, before, units| {
        let (x, y) = key(&r, w, h, row, before, units);
        common::tap(&mut r, x, y);
        common::settle();
        r.render(&mut buf, w as usize);
    };
    tap(1, 0.5 + 2.0, 1.0); // d
    tap(0, 7.0, 1.0); // i
    tap(0, 3.0, 1.0); // r
    tap(3, 8.5, 1.5); // enter
    tap(2, 0.0, 1.5); // shift
    tap(1, 0.5, 1.0); // A
    tap(1, 0.5, 1.0); // a (shift is one-shot)
    tap(2, 8.5, 1.5); // backspace
    tap(3, 2.5, 5.0); // space
    tap(3, 0.0, 1.5); // ?123
    tap(1, 0.5 + 2.0, 1.0); // backslash
    tap(0, 0.0, 1.0); // 1
    tap(3, 0.0, 1.5); // ABC
    tap(0, 0.0, 1.0); // q
    let typed: String = r
        .take_typed()
        .into_iter()
        .map(|c| char::from_u32(c).unwrap())
        .collect();
    assert_eq!(typed, "dir\rAa\u{8} \\1q");
    assert!(r.take_typed().is_empty());

    r.set_keyboard(false);
    let (x, y) = key(&r, w, h, 0, 0.0, 1.0);
    common::tap(&mut r, x, y);
    assert!(r.take_typed().is_empty(), "a closed keyboard types nothing");
}

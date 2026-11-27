// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

/// Theme.sheet, the Add sheet's background.
const SHEET: u32 = 0xFF1B_1E23;

/// Home Part 3 in landscape (360 dp of height): the editor's Add sheet fits, its Widgets / Apps tabs on screen.
#[test]
fn add_sheet_fits_in_landscape() {
    let (w, h) = (2160, 1080);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let d = common::dp(w, h);
    let px = |v: f32| (v * d) as i32;
    r.show("home", "");
    r.edit_home();
    common::frame(&mut r, &mut buf, w, "editing");
    let body_y = r.body().y;
    // + at the bar's left (8 dp padding, a 40 dp button)
    common::tap(&mut r, px(28.0), body_y + px(24.0));
    common::frame(&mut r, &mut buf, w, "add sheet");
    common::write_png("home-edit-add-landscape.png", &buf, w, h);
    // the sheet's top edge down the middle column (its corners are round, the middle isn't)
    let x = w / 2;
    let top = (0..h)
        .find(|&y| buf[(y * w + x) as usize] == SHEET)
        .expect("the sheet shows") as i32;
    assert!(
        top > body_y,
        "the sheet starts at {top}, above home's top {body_y}"
    );
    // Apps: the right tab (20 dp side padding, two tabs 8 dp apart), under the handle (18 dp) and the title (34 dp)
    let before = buf.clone();
    common::tap(&mut r, (w * 3 / 4) as i32, top + px(18.0 + 34.0 + 18.0));
    common::frame(&mut r, &mut buf, w, "apps tab");
    assert!(
        buf.iter().zip(&before).any(|(a, b)| a != b),
        "the Apps tab answered"
    );
}

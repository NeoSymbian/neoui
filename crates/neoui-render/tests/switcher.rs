// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_render::Renderer;

/// Centre of card `i` and of its ×; mirrors ui/screens/switcher.slint (12 dp margins and gaps, two columns, 160 dp
/// cards, × in a 44 dp square at the top-right).
fn card(r: &Renderer, w: u32, h: u32, i: usize) -> ((i32, i32), (i32, i32)) {
    let d = common::dp(w, h);
    let gap = 12.0 * d;
    let card_w = (w as f32 - 3.0 * gap) / 2.0;
    let card_h = 160.0 * d;
    let x = gap + (i % 2) as f32 * (card_w + gap);
    let y = r.body().y as f32 + gap + (i / 2) as f32 * (card_h + gap);
    (
        ((x + card_w / 2.0) as i32, (y + card_h / 2.0) as i32),
        ((x + card_w - 22.0 * d) as i32, (y + 22.0 * d) as i32),
    )
}

#[test]
fn switcher() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let d = common::dp(w, h);
    let tap = |r: &mut Renderer, buf: &mut [u32], (x, y): (i32, i32)| {
        common::tap(r, x, y);
        common::settle();
        r.render(buf, w as usize);
    };

    // the Apps menu's toolbar button (middle) opens the switcher
    r.show("menu", "");
    tap(
        &mut r,
        &mut buf,
        (w as i32 / 2, (h as f32 - 30.5 * d) as i32),
    );
    assert_eq!(r.screen(), "switcher");

    // empty: no cards to close or open
    let empty = card(&r, w, h, 0);
    tap(&mut r, &mut buf, empty.1);
    tap(&mut r, &mut buf, empty.0);
    assert!(r.take_closed().is_empty());
    assert_eq!(r.screen(), "switcher");

    r.set_running(&["files", "clock", "terminal"]);
    r.show("switcher", "");
    common::frame(&mut r, &mut buf, w, "switcher");
    common::write_png("switcher-539x1079.png", &buf, w, h);
    let (_, close0) = card(&r, w, h, 0);
    tap(&mut r, &mut buf, close0);
    assert_eq!(r.take_closed(), ["files"]);
    assert_eq!(r.screen(), "switcher", "× must not also open the card");
    let (open1, _) = card(&r, w, h, 1);
    tap(&mut r, &mut buf, open1);
    assert_eq!(r.screen(), "clock");
}

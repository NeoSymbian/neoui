// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_render::Renderer;

/// Centre of card `i` and of its ×; mirrors ui/screens/switcher.slint (12 dp margins and gaps, two columns, 160 dp
/// cards, × in a 44 dp square at the top-right).
fn card(r: &Renderer, w: u32, h: u32, i: usize) -> ((i32, i32), (i32, i32)) {
    card_in(r, w, h, i, 2)
}

/// The same with `cols` columns (nav.switcher_cols, G3): (width - (cols + 1) gaps) / cols wide.
fn card_in(r: &Renderer, w: u32, h: u32, i: usize, cols: usize) -> ((i32, i32), (i32, i32)) {
    let d = common::dp(w, h);
    let gap = 12.0 * d;
    let card_w = (w as f32 - (cols + 1) as f32 * gap) / cols as f32;
    let card_h = 160.0 * d;
    let x = gap + (i % cols) as f32 * (card_w + gap);
    // below the 48 dp header
    let y = r.body().y as f32 + 48.0 * d + gap + (i / cols) as f32 * (card_h + gap);
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
    let tap = |r: &mut Renderer, buf: &mut [u32], (x, y): (i32, i32)| {
        common::tap(r, x, y);
        common::settle();
        r.render(buf, w as usize);
    };

    // the strip's ▢ opens the switcher
    r.show("menu", "");
    let at = common::keys(&mut r, w, h);
    tap(&mut r, &mut buf, (at.apps, at.y));
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

    // G3: thumbnails (two apps, two colours: each card shows its own), three columns
    use neoui_proto::settings::Value;
    r.set_running(&["files", "clock", "terminal"]);
    r.set_thumb("files", 40, 60, &[0xffd0_2020; 40 * 60]);
    r.set_thumb("clock", 40, 60, &[0xff20_d020; 40 * 60]);
    r.show("switcher", "");
    assert!(r.apply_setting("nav.switcher_cols", &Value::Int(3)));
    common::frame(&mut r, &mut buf, w, "switcher 3");
    common::write_png("switcher-3-539x1079.png", &buf, w, h);
    let at = |buf: &[u32], (x, y): (i32, i32)| buf[y as usize * w as usize + x as usize] & 0xffffff;
    let (c0, c1, c2) = (
        card_in(&r, w, h, 0, 3).0,
        card_in(&r, w, h, 1, 3).0,
        card_in(&r, w, h, 2, 3).0,
    );
    assert_eq!(at(&buf, c0), 0xd02020, "files' thumbnail");
    assert_eq!(at(&buf, c1), 0x20d020, "clock's thumbnail");
    assert!(
        ![0xd02020, 0x20d020].contains(&at(&buf, c2)),
        "terminal: its icon card"
    );
    assert!(c2.1 == c0.1 && c2.0 > c1.0, "three to a row");
    // a thumbnail goes with its app
    r.set_running(&["clock"]);
    common::frame(&mut r, &mut buf, w, "one left");
    assert_eq!(at(&buf, card_in(&r, w, h, 0, 3).0), 0x20d020);

    // the legacy bridge isn't in the Apps menu, but its card (an old app running) keeps its name band: "Classic apps"
    // and the legacy icon over the thumbnail, as every app's
    r.set_running(&["legacy", "clock"]);
    r.set_thumb("legacy", 40, 60, &[0xffd0_2020; 40 * 60]);
    common::frame(&mut r, &mut buf, w, "legacy card");
    common::write_png("switcher-legacy-539x1079.png", &buf, w, h);
    let d = common::dp(w, h);
    let card_w = (w as f32 - 4.0 * 12.0 * d) / 3.0;
    let (cx, cy) = card_in(&r, w, h, 0, 3).0;
    let band = (
        (cx as f32 - card_w / 2.0 + 4.0 * d) as i32,
        (cy as f32 + 80.0 * d - 4.0 * d) as i32,
    );
    assert_eq!(
        at(&buf, (cx, cy - (40.0 * d) as i32)),
        0xd02020,
        "its thumbnail"
    );
    assert_ne!(
        at(&buf, band),
        0xd02020,
        "the bridge's card has its name band"
    );
    let band_px = (band.1 - (28.0 * d) as i32..band.1)
        .flat_map(|y| (band.0..band.0 + (card_w * 0.6) as i32).map(move |x| (x, y)))
        .filter(|&p| at(&buf, p) != at(&buf, band))
        .count();
    assert!(
        band_px > 50,
        "the band shows the icon and name ({band_px} px)"
    );

    // close mode (Gestures): no × until a card is held 500 ms; a tap off the cards leaves
    assert!(r.apply_setting("nav.switcher_cols", &Value::Int(2)));
    assert!(r.apply_setting("nav.mode", &Value::Int(1)));
    r.set_running(&["files", "clock"]);
    r.show("switcher", "");
    common::frame(&mut r, &mut buf, w, "gestures");
    let (open0, close0) = card(&r, w, h, 0);
    let touch = |r: &mut Renderer, (x, y): (i32, i32), phase| {
        r.dispatch(neoui_proto::InputEvent::Touch { id: 0, x, y, phase });
    };
    touch(&mut r, open0, neoui_proto::TouchPhase::Down);
    common::advance(600);
    common::frame(&mut r, &mut buf, w, "held");
    touch(&mut r, open0, neoui_proto::TouchPhase::Up);
    common::frame(&mut r, &mut buf, w, "close mode");
    assert_eq!(r.screen(), "switcher", "the hold doesn't open the card");
    common::write_png("switcher-close-mode-539x1079.png", &buf, w, h);
    tap(&mut r, &mut buf, close0);
    assert_eq!(r.take_closed(), ["files"]);
    // off the cards: close mode ends, a × spot then opens the card under it
    tap(&mut r, &mut buf, (w as i32 / 2, h as i32 - 200));
    tap(&mut r, &mut buf, close0);
    assert!(r.take_closed().is_empty());
    assert_eq!(r.screen(), "files");
}

// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

use neoui_proto::settings::Value;
use neoui_proto::tile_table::Slots;
use neoui_proto::tiles::{self, Offer, builtin, kind};
use neoui_proto::wifi::{LinkView, Network};
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;

/// A drag from (x0, y0) to (x1, y1) in 10 moves, px.
fn drag(r: &mut Renderer, (x0, y0): (i32, i32), (x1, y1): (i32, i32)) {
    r.dispatch(InputEvent::Touch {
        id: 0,
        x: x0,
        y: y0,
        phase: TouchPhase::Down,
    });
    for k in 1..=10 {
        common::advance(17);
        r.dispatch(InputEvent::Touch {
            id: 0,
            x: x0 + (x1 - x0) * k / 10,
            y: y0 + (y1 - y0) * k / 10,
            phase: TouchPhase::Move,
        });
    }
    r.dispatch(InputEvent::Touch {
        id: 0,
        x: x1,
        y: y1,
        phase: TouchPhase::Up,
    });
}

fn net(ssid: &str, dbm: i8, channel: u8, secured: bool) -> Network {
    Network {
        bssid: [2, 0, 0, 0, 0, dbm as u8],
        channel,
        dbm,
        secured,
        ssid: ssid.into(),
    }
}

/// TP7 (tp-shade's first stop, tp-control, tp-control-buttons, tp-edit, tp-wifi): the first stop's four tiles, a pull
/// on to the control centre with its wide and small tiles (only backed ones), a tile that switches its setting, an
/// app's tile that reports its tap, the editor (drag, hold for wide, remove, Done saves `shell.tiles` once) and the
/// Wi-Fi sheet. Snapshots `tp-*.png`.
#[test]
fn the_control_centre_its_editor_and_the_wifi_sheet() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let px = |v: f32| (v * d) as i32;
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    r.set_network(4, "Ucom", "LTE");
    r.set_wifi_link(&LinkView {
        shown: true,
        connected: true,
        name: "Home-5G".into(),
        bars: 4,
        band: "5 GHz".into(),
        ..LinkView::default()
    });
    // the Clock's Stopwatch is offered (not placed); a placed tile of a slot nobody has stays out
    let mut slots = Slots::default();
    slots.offer(
        "clock",
        Offer {
            local_id: 1,
            kind: kind::TOGGLE,
            glyph: 1,
            label: "Stopwatch".into(),
            ..Offer::default()
        },
    );
    r.set_tile_slots(&slots, &[]);
    let stored = tiles::encode_order(&[
        (builtin::WIFI, true),
        (builtin::MOBILE, true),
        (builtin::SILENT, false),
        (builtin::DND, false),
        (builtin::AIRPLANE, false),
        (builtin::BATTERY_SAVER, false),
        (builtin::DARK, false),
        (264, false),
        (builtin::FLASHLIGHT, false),
    ])
    .unwrap();
    assert!(r.apply_setting("shell.tiles", &Value::Bytes(stored.clone())));
    let _ = r.take_setting_edits();

    // the first stop: Wi-Fi, Mobile data, Silent, Do not disturb
    common::tap(&mut r, px(180.0), px(12.0));
    common::frame(&mut r, &mut buf, w, "first stop");
    common::write_png("tp-shade.png", &buf, w, h);
    assert!(r.overlay_open() && !r.control_open());
    // Do not disturb (the 4th) switches notify.dnd
    let first_y = px(25.0 + 12.0 + 36.0 + 12.0 + 37.0);
    common::tap(&mut r, px(12.0 + 3.0 * 86.0 + 39.0), first_y);
    common::frame(&mut r, &mut buf, w, "dnd");
    assert_eq!(
        r.take_setting_edits(),
        vec![("notify.dnd", Value::Bool(true))]
    );

    // a pull down on the tiles: the control centre
    drag(&mut r, (px(60.0), first_y), (px(60.0), first_y + px(120.0)));
    common::frame(&mut r, &mut buf, w, "control");
    assert!(
        r.control_open(),
        "a pull past the tiles opens the second stop"
    );
    assert!(r.take_setting_edits().is_empty(), "a pull switches nothing");
    common::write_png("tp-control.png", &buf, w, h);
    // grid: 4 cells of 78 dp, 8 dp apart, from 12 dp; rows of 64 dp, 8 apart, from 85 dp
    let cell = |col: f32, row: f32| (px(12.0 + col * 86.0 + 39.0), px(85.0 + row * 72.0 + 32.0));

    // the Wi-Fi tile's › : the sheet
    common::tap(&mut r, px(12.0 + 164.0 - 18.0), cell(0.0, 0.0).1);
    common::frame(&mut r, &mut buf, w, "wifi sheet");
    assert!(r.wifi_sheet_open());
    let joined = net("Home-5G", -46, 36, true);
    let (home2, guest, cafe, printer) = (
        net("Home-2G", -58, 6, true),
        net("Ucom-Guest", -63, 11, false),
        net("Harbour Cafe", -75, 1, true),
        net("DIRECT-printer", -80, 6, true),
    );
    r.set_wifi_networks(
        Some(&joined),
        &[
            (&home2, true),
            (&guest, false),
            (&cafe, true),
            (&printer, false),
        ],
        true,
    );
    common::frame(&mut r, &mut buf, w, "wifi list");
    common::write_png("tp-wifi.png", &buf, w, h);
    // the sheet is opaque (tp-wifi): under the list, nothing of the control centre shows through
    let band = (h as usize * 65 / 100)..(h as usize * 90 / 100);
    let ghost = band
        .flat_map(|y| buf[y * w as usize..][..w as usize].iter().copied())
        .find(|&p| p != 0xFF0F_1115);
    assert_eq!(ghost, None, "the Wi-Fi sheet shows the control centre");
    r.cancel_touch();
    // Done (the right half of the bottom row)
    common::tap(
        &mut r,
        px(270.0),
        (h as f32 - 16.0 * d - px(12.0 + 22.0) as f32) as i32,
    );
    common::frame(&mut r, &mut buf, w, "sheet done");
    assert!(!r.wifi_sheet_open() && r.control_open());

    // ✎: the editor
    common::tap(
        &mut r,
        px(360.0 - 12.0 - 36.0 * 2.5 - 20.0),
        px(25.0 + 12.0 + 18.0),
    );
    common::frame(&mut r, &mut buf, w, "editor");
    assert!(r.tiles_editing());
    common::write_png("tp-edit.png", &buf, w, h);
    // the editor's grid starts lower (title 36, hint 2 lines, 12 dp gaps; rows 12 dp apart)
    let ecell = |col: f32, row: f32| {
        (
            px(12.0 + col * 86.0 + 39.0),
            px(25.0 + 12.0 + 36.0 + 12.0 + 34.0 + 12.0 + row * 76.0 + 32.0),
        )
    };
    // hold Silent (row 1, col 0): wide
    let (sx, sy) = ecell(0.0, 1.0);
    r.dispatch(InputEvent::Touch {
        id: 0,
        x: sx,
        y: sy,
        phase: TouchPhase::Down,
    });
    common::advance(600);
    r.render(&mut buf, w as usize);
    r.dispatch(InputEvent::Touch {
        id: 0,
        x: sx,
        y: sy,
        phase: TouchPhase::Up,
    });
    common::frame(&mut r, &mut buf, w, "silent wide");
    // drag Dark (row 1 now has Silent wide, DND, Airplane; Battery saver and Dark on row 2) onto Wi-Fi's cell
    drag(&mut r, ecell(1.0, 2.0), ecell(0.0, 0.0));
    common::frame(&mut r, &mut buf, w, "dark first");
    // ⊕ on the Stopwatch adds it
    common::frame(&mut r, &mut buf, w, "before add");
    // Done
    common::tap(&mut r, px(360.0 - 12.0 - 30.0), px(25.0 + 12.0 + 18.0));
    common::frame(&mut r, &mut buf, w, "done");
    assert!(!r.tiles_editing() && r.control_open());
    let edits = r.take_setting_edits();
    assert_eq!(edits.len(), 1, "{edits:?}");
    let Value::Bytes(b) = &edits[0].1 else {
        panic!("{edits:?}")
    };
    let order = tiles::decode_order(b).unwrap();
    assert_eq!(edits[0].0, "shell.tiles");
    assert_eq!(order[0], (builtin::DARK, false), "{order:?}");
    assert!(order.contains(&(builtin::SILENT, true)), "{order:?}");
    assert!(
        !order
            .iter()
            .any(|t| t.0 == 264 || t.0 == builtin::FLASHLIGHT),
        "what isn't drawn leaves the value: {order:?}"
    );
    assert!(r.take_setting_edits().is_empty(), "saved once");

    // the app's tile: placed by the editor, a tap goes to the UI server for its app
    let mut placed = order.clone();
    placed.insert(0, (257, false));
    assert!(r.apply_setting(
        "shell.tiles",
        &Value::Bytes(tiles::encode_order(&placed).unwrap())
    ));
    common::frame(&mut r, &mut buf, w, "stopwatch placed");
    let (x, y) = cell(0.0, 0.0);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "stopwatch tapped");
    assert_eq!(r.take_tile_taps(), vec![(257, false)]);
    assert!(
        r.take_setting_edits().is_empty(),
        "an app's tile switches no setting"
    );

    // Buttons mode (tp-control-buttons): the key row under the control centre; the stored default order again
    assert!(r.apply_setting("shell.tiles", &Value::Bytes(Vec::new())));
    let keys = common::keys(&mut r, w, h);
    r.show("home", "");
    r.open_control();
    common::frame(&mut r, &mut buf, w, "buttons");
    common::write_png("tp-control-buttons.png", &buf, w, h);
    // Back closes it
    common::tap(&mut r, keys.back, keys.y);
    common::frame(&mut r, &mut buf, w, "closed");
    assert!(!r.overlay_open());
    // shell.tap 1: a tap on the status bar opens the control centre at once
    assert!(r.apply_setting("shell.tap", &Value::Int(1)));
    common::tap(&mut r, px(180.0), px(12.0));
    common::frame(&mut r, &mut buf, w, "tap to control");
    assert!(r.control_open());
}

// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

use std::time::Instant;

use neoui_proto::wifi::Network;
use neoui_render::Renderer;

/// Renders `n` frames 17 ms apart: (Slint's time in µs, pixels drawn) summed.
fn frames(r: &mut Renderer, buf: &mut [u32], w: u32, n: usize) -> (u128, u64, usize) {
    let (mut us, mut px, mut drawn) = (0, 0, 0);
    for _ in 0..n {
        common::advance(17);
        let t = Instant::now();
        let d = r.render(buf, w as usize);
        us += t.elapsed().as_micros();
        let p: u64 = d
            .rects()
            .iter()
            .map(|r| u64::from(r.w) * u64::from(r.h))
            .sum();
        px += p;
        drawn += usize::from(p > 0);
    }
    (us, px, drawn)
}

fn net(ssid: &str, dbm: i8) -> Network {
    Network {
        bssid: [2, 0, 0, 0, 0, dbm as u8],
        channel: 6,
        dbm,
        secured: true,
        ssid: ssid.into(),
    }
}

/// smooth-ui's budget with the shade (W1 Pixel run: 35–49 ms a frame while it opened, 16 ms is the budget): with the
/// server's transitions Slint draws the opening shade once, not every frame of its fade (the UI server blends
/// snapshots), and a static shade draws nothing. Prints the host's numbers.
#[test]
fn the_shade_opens_with_one_render_and_then_draws_nothing() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    r.set_server_transitions(true);
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");

    // a screen change, the same: the Apps menu is drawn once
    r.show("menu", "");
    let (us, px, drawn) = frames(&mut r, &mut buf, w, 18);
    eprintln!("apps menu: {drawn} frames drawn, {px} px, {us} us");
    assert_eq!(drawn, 1, "one render of the Apps menu");
    r.show("home", "");
    frames(&mut r, &mut buf, w, 3);

    r.open_shade();
    let (us, px, drawn) = frames(&mut r, &mut buf, w, 18);
    eprintln!("opening: {drawn} frames drawn, {px} px, {us} us");
    assert_eq!(drawn, 1, "one render of the open shade");
    assert!(!r.has_active_animations());
    let (_, px, _) = frames(&mut r, &mut buf, w, 30);
    assert_eq!(px, 0, "a static shade draws nothing");

    // the Wi-Fi sheet's list changing (a scan): only the sheet is drawn, not the control centre under it
    r.open_control();
    frames(&mut r, &mut buf, w, 2);
    common::tap(
        &mut r,
        ((12.0 + 164.0 - 18.0) * d) as i32,
        ((85.0 + 32.0) * d) as i32,
    );
    frames(&mut r, &mut buf, w, 2);
    assert!(r.wifi_sheet_open());
    let (a, b) = (net("Home-2G", -58), net("Cafe", -75));
    let (mut us, mut px) = (0, 0);
    for i in 0..10 {
        let list: Vec<(&Network, bool)> = if i % 2 == 0 {
            vec![(&a, true), (&b, false)]
        } else {
            vec![(&b, false)]
        };
        r.set_wifi_networks(None, &list, true);
        let (u, p, _) = frames(&mut r, &mut buf, w, 1);
        us += u;
        px += p;
    }
    eprintln!("wifi list updates: 10 frames, {px} px, {us} us");
}

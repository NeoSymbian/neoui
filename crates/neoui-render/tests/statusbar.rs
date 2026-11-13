// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::battery::{History, Snapshot};
use neoui_proto::wifi::{Link, LinkState, LinkView, Network};

/// The status bar with a modem, Wi-Fi and a battery: the radio cluster sits at the right with the other icons, and a
/// short operator name leaves the middle of the bar empty (the bars once took half the spare width, mid-bar).
#[test]
fn modem_cluster_stays_right() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    r.set_network(3, "Ucom AM", "LTE");
    r.set_clock("23:41");
    let home = Network {
        bssid: [2, 0, 0, 0, 0, 1],
        channel: 44,
        dbm: -50,
        secured: true,
        ssid: "Home".into(),
    };
    let link = Link {
        state: LinkState::Connected,
        bssid: home.bssid,
        ..Link::default()
    };
    r.set_wifi_link(&LinkView::new(&link, Some(&home)));
    let battery = Snapshot {
        valid: 63,
        present: 1,
        percent: 97,
        voltage_uv: 4_200_000,
        ..Snapshot::default()
    };
    r.set_battery(&battery, &History::default(), false);
    common::frame(&mut r, &mut buf, w, "status bar with a modem");
    common::write_png("statusbar-modem-pixel.png", &buf, w, h);

    let bar_h = (25.0 * common::dp(w, h)) as usize;
    let bg = buf[0];
    let lit =
        |x0: usize, x1: usize| (0..bar_h).any(|y| (x0..x1).any(|x| buf[y * w as usize + x] != bg));
    let (quarter, half) = (w as usize / 4, w as usize / 2);
    assert!(lit(0, quarter), "the operator name is drawn at the left");
    assert!(
        !lit(quarter, half + 20),
        "the gap after a short operator name stays empty: the bars sit with the other icons"
    );
    assert!(
        lit(half + 20, w as usize),
        "LTE, bars, Wi-Fi, battery and time at the right"
    );
}

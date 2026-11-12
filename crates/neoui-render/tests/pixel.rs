// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
mod common;

/// Every screen and sub-view, as (screen, view).
const SCREENS: &[(&str, &str)] = &[
    ("home", ""),
    ("menu", ""),
    ("phone", ""),
    ("phone", "log"),
    ("phone", "call"),
    ("messages", ""),
    ("messages", "thread"),
    ("contacts", ""),
    ("contacts", "card"),
    ("browser", ""),
    ("browser", "page"),
    ("camera", ""),
    ("photos", ""),
    ("photos", "photo"),
    ("music", ""),
    ("music", "playing"),
    ("calendar", ""),
    ("calendar", "day"),
    ("clock", ""),
    ("files", ""),
    ("files", "folder"),
    ("settings", ""),
    ("settings", "connectivity"),
    ("settings", "wifi"),
    ("settings", "display"),
    ("settings", "keyboard"),
    ("settings", "about"),
    ("terminal", ""),
];

#[test]
fn pixel() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    for (screen, view) in SCREENS {
        common::snapshot(&mut r, &mut buf, screen, view, "pixel", w, h);
    }
    common::menu_tap(&mut r, &mut buf, "pixel", w, h);

    // Settings -> Wi-Fi with a scan (sample networks) and the sheet a tapped network opens
    let mut scan = vec![1u8, 2, 3, 1];
    for (last, ch, dbm, secured, ssid) in [
        (1u8, 44u8, -46i8, 1u8, "Home"),
        (2, 6, -70, 1, "Cafe Guest"),
        (3, 36, -80, 0, ""),
    ] {
        let mut e = vec![
            2,
            0,
            0,
            0,
            0,
            last,
            ch,
            dbm as u8,
            secured,
            ssid.len() as u8,
        ];
        e.extend_from_slice(ssid.as_bytes());
        e.resize(42, 0);
        scan.extend(e);
    }
    r.set_wifi(&neoui_proto::wifi::Scan::decode(&scan).unwrap());
    r.show("settings", "wifi");
    common::frame(&mut r, &mut buf, w, "wifi list");
    common::write_png("settings-wifi-list-pixel.png", &buf, w, h);
    common::tap(&mut r, (w / 2) as i32, (h as f32 * 0.33) as i32); // the second network, "Cafe Guest"
    common::frame(&mut r, &mut buf, w, "wifi sheet");
    common::write_png("settings-wifi-sheet-pixel.png", &buf, w, h);

    // Overlays, opened by real taps: Options (toolbar, right button) and the notification shade (status bar).
    let d = common::dp(w, h);
    r.show("messages", "");
    common::frame(&mut r, &mut buf, w, "messages");
    common::tap(
        &mut r,
        (w as f32 - 58.0 * d) as i32,
        (h as f32 - 30.0 * d) as i32,
    );
    common::frame(&mut r, &mut buf, w, "options");
    common::write_png("options-pixel.png", &buf, w, h);
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");
    common::tap(&mut r, (w / 2) as i32, (12.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "shade");
    common::write_png("shade-pixel.png", &buf, w, h);

    // Wi-Fi connected (W4): "Current network" above the list, the status bar's icon, then its details sheet
    let mut link = alloc_link();
    link.state = neoui_proto::wifi::LinkState::Connected;
    let home = neoui_proto::wifi::Network {
        bssid: [2, 0, 0, 0, 0, 1],
        channel: 44,
        dbm: -50,
        secured: true,
        ssid: "Home".into(),
    };
    link.bssid = home.bssid;
    r.set_wifi_link(&neoui_proto::wifi::LinkView::new(&link, Some(&home)));
    r.show("settings", "wifi");
    common::frame(&mut r, &mut buf, w, "wifi page");
    common::tap(&mut r, (w / 2) as i32, (h as f32 * 0.1) as i32); // above the open sheet's card: closes it
    common::frame(&mut r, &mut buf, w, "wifi connected");
    common::write_png("settings-wifi-connected-pixel.png", &buf, w, h);
    common::tap(&mut r, (w / 2) as i32, (h as f32 * 0.232) as i32); // the current network's row
    common::frame(&mut r, &mut buf, w, "wifi details");
    common::write_png("settings-wifi-details-pixel.png", &buf, w, h);
}

// a connected link as the kernel reports it (HAL WlanLink status)
fn alloc_link() -> neoui_proto::wifi::Link {
    let mut b = vec![
        1u8, 3, 4, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1, 2, 0x4e, 0xc1, 0x47, 0x28, 0xa4,
    ];
    b.extend_from_slice(&120u32.to_le_bytes());
    b.extend_from_slice(&96u32.to_le_bytes());
    let rsn = [
        0x30u8, 20, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 2, 0, 0,
    ];
    b.push(rsn.len() as u8);
    b.extend_from_slice(&rsn);
    neoui_proto::wifi::Link::decode(&b).unwrap()
}

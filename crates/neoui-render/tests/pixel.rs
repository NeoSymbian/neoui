// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
mod common;

/// Every screen and sub-view, as (screen, view).
const SCREENS: &[(&str, &str)] = &[
    ("home", ""),
    ("menu", ""),
    ("phone", ""),
    ("phone", "log"),
    ("phone", "logdetail"),
    ("phone", "speed"),
    ("phone", "call"),
    ("messages", ""),
    ("messages", "thread"),
    ("contacts", ""),
    ("contacts", "card"),
    ("contacts", "edit"),
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
    ("settings", "phone"),
    ("settings", "about"),
    ("terminal", ""),
];

#[test]
fn pixel() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    // C8 data for the Phone and Contacts views (invented names; 555-01xx is the fictional range)
    r.set_people(
        &[
            neoui_render::PersonItem {
                id: 1,
                name: "Ava Lindqvist",
                detail: "+1 555 0142",
                favourite: true,
            },
            neoui_render::PersonItem {
                id: 2,
                name: "Անի Պետրոսյան",
                detail: "+1 555 0118",
                favourite: false,
            },
        ],
        2,
        true,
    );
    r.set_card(&neoui_render::CardItem {
        id: 1,
        name: "Ava Lindqvist",
        company: "Example Co",
        numbers: &[
            ("Mobile", "+1 555 0142"),
            ("Telephone (work)", "+1 555 0177"),
        ],
        email: "ava@example.org",
        note: "",
        favourite: true,
    });
    r.set_log(&[
        neoui_render::LogItem {
            id: 3,
            title: "Ava Lindqvist",
            subtitle: "Mobile",
            when: "21:03",
            kind: 3,
            count: 2,
        },
        neoui_render::LogItem {
            id: 1,
            title: "+1 555 0123",
            subtitle: "",
            when: "Yesterday",
            kind: 3,
            count: 1,
        },
    ]);
    r.set_log_detail(&neoui_render::LogDetailItem {
        id: 3,
        title: "Ava Lindqvist",
        number: "+1 555 0142",
        direction: "Missed",
        when: "Today 21:03",
        duration: "",
        known: true,
    });
    r.set_speed(&[
        ("Ava Lindqvist", "+1 555 0142"),
        ("", ""),
        ("", ""),
        ("", ""),
        ("", ""),
        ("", ""),
        ("", ""),
        ("", ""),
    ]);
    r.set_dial("26", &[(1, "Anna Smith", "+1 555 0150")]);
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
    let ip = neoui_proto::wifi::Ip {
        address: Some(([192, 168, 3, 57], 24)),
        gateway: [192, 168, 3, 1],
        dns: [192, 168, 3, 1],
        internet: neoui_proto::wifi::Internet::Ok,
    };
    r.set_wifi_link(&neoui_proto::wifi::LinkView::new(&link, Some(&home)).with_ip(&ip));
    r.show("settings", "wifi");
    common::frame(&mut r, &mut buf, w, "wifi page");
    common::tap(&mut r, (w / 2) as i32, (h as f32 * 0.1) as i32); // above the open sheet's card: closes it
    common::frame(&mut r, &mut buf, w, "wifi connected");
    common::write_png("settings-wifi-connected-pixel.png", &buf, w, h);
    common::tap(&mut r, (w / 2) as i32, (h as f32 * 0.232) as i32); // the current network's row
    common::frame(&mut r, &mut buf, w, "wifi details");
    common::write_png("settings-wifi-details-pixel.png", &buf, w, h);

    // Settings -> Date & time
    r.set_datetime(
        "Friday, 13 November",
        "UTC+04:00 · Yerevan",
        "Mobile network, 23:41",
    );
    r.show("settings", "datetime");
    common::frame(&mut r, &mut buf, w, "date and time");
    common::write_png("settings-datetime-pixel.png", &buf, w, h);
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

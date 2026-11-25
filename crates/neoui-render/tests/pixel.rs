// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
mod common;

/// Every screen and sub-view, as (screen, view).
const SCREENS: &[(&str, &str)] = &[
    ("home", ""),
    ("menu", ""),
    ("camera", ""),
    ("photos", ""),
    ("photos", "photo"),
    ("music", ""),
    ("music", "playing"),
    ("calendar", ""),
    ("calendar", "day"),
    ("clock", ""),
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

    // Overlays: the app's toast (NOTE) above the band, and the notification shade (status bar)
    let d = common::dp(w, h);
    r.show("files", "");
    common::frame(&mut r, &mut buf, w, "files");
    r.set_app_front(true);
    r.set_app_page(&neoui_render::Page {
        title: "Files".into(),
        ..Default::default()
    });
    r.set_note(Some(&neoui_render::Note {
        text: "notes.txt deleted".into(),
        action: "Undo".into(),
    }));
    common::frame(&mut r, &mut buf, w, "toast");
    common::write_png("toast-pixel.png", &buf, w, h);
    r.set_note(None);
    r.set_app_front(false);
    r.show("home", "");
    // two unread messages (C9): the status bar's envelope, the shade's row
    r.set_unread_messages(2, "+374 91 234567: Running late, see you at 7?");
    // an app's entry (NOTIFY) above the shell's own rows
    let timer = neoui_render::Notification {
        title: "Timer done · 05:00".into(),
        text: "Clock · Time's up".into(),
        ..neoui_render::Notification::default()
    };
    r.set_notes(&[&timer]);
    common::frame(&mut r, &mut buf, w, "home");
    common::tap(&mut r, (w / 2) as i32, (12.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "shade");
    common::write_png("shade-pixel.png", &buf, w, h);
    // a copy in Files (1.11): its entry with a progress bar and a plain Cancel pill (files2-progress)
    let copying = neoui_render::Notification {
        id: 1,
        title: "Files · Copying file 2 of 2".into(),
        text: "live-take.flac · 12.6 of 28 MB · 45 %".into(),
        actions: vec!["Cancel".into()],
        progress: Some(45),
        ..neoui_render::Notification::default()
    };
    r.set_notes(&[&copying, &timer]);
    common::frame(&mut r, &mut buf, w, "shade progress");
    common::write_png("shade-progress-pixel.png", &buf, w, h);

    // Wi-Fi connected (W4): the status bar's icon (the Settings app draws the network itself)
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
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "wifi connected");
    common::write_png("home-wifi-pixel.png", &buf, w, h);
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

// SPDX-License-Identifier: GPL-3.0-only
//! Every Settings page with sample data, drawn to `snapshots/set2-*.png` for review against the canvas's set2-*
//! boards, and what taps on them ask the app for (`SetState.act`). One test: Slint's platform is per process.
mod common;

use common::{BUTTONS, Shot, model, strings};
use settings_ui::{AppRow, SearchHit, SetState, WifiDetail, WifiLink, WifiNet, WifiSaved};
use slint::{ComponentHandle, Model, SharedString};

/// The sample state the boards show (set2-root-list's values).
fn fill(sh: &Shot) {
    let s = sh.ui.global::<SetState>();
    s.set_wifi(true);
    s.set_mobile_data(true);
    s.set_vibrate(true);
    s.set_ring_volume(7);
    s.set_media_volume(7);
    s.set_dark(true);
    s.set_brightness(160.0 / 255.0);
    s.set_timeout(30);
    s.set_timeout_text("30 seconds".into());
    s.set_timeouts(strings(&[
        "15 seconds",
        "30 seconds",
        "1 minute",
        "2 minutes",
        "5 minutes",
        "10 minutes",
        "Never",
    ]));
    s.set_timeout_index(1);
    s.set_nav_mode(1);
    s.set_nav_modes(strings(&["Buttons", "Gestures", "N9 gestures"]));
    s.set_nav_hints(strings(&[
        "Keys at the bottom: Back, Home, Open apps. No edge gestures.",
        "Either side edge: Back. Up from the bottom: Home; pause: Open apps; along it: switch apps.",
        "Left edge: Back. Right edge: Open apps; flick: last app. Up: Home; keep going: Apps menu.",
    ]));
    s.set_media_routes(strings(&["Both speakers", "Earpiece", "Bottom speaker"]));
    s.set_mics(strings(&["Main", "Second"]));
    s.set_time_auto(true);
    s.set_time_auto_zone(true);
    s.set_clock_24h(true);
    s.set_shade_date(true);
    s.set_date_text("Tuesday, 17 November".into());
    s.set_date_formats(strings(&[
        "Tuesday, 17 November",
        "17.11.2026",
        "2026-11-17",
        "11/17/2026",
    ]));
    s.set_zone("UTC+04:00 · Yerevan".into());
    s.set_synced("Mobile network, 10:02".into());
    s.set_cities(strings(&[
        "Moscow · UTC+03:00",
        "Tehran · UTC+03:30",
        "Dubai · UTC+04:00",
        "Yerevan · UTC+04:00",
        "Tbilisi · UTC+04:00",
    ]));
    s.set_keyboards(model(vec![true, true, true]));
    s.set_keyboard_names(strings(&["English", "Armenian", "Russian"]));
    s.set_keyboard_hints(strings(&["QWERTY", "Phonetic", "ЙЦУКЕН"]));
    s.set_same_height(true);
    s.set_pad_vibrate(true);
    s.set_pad_strength(1);
    s.set_pad_strengths(strings(&["Light", "Medium", "Strong"]));
    s.set_pad_defaults(strings(&["Auto", "Navigator", "Gamepad", "Number"]));
    s.set_pad_dirs_names(strings(&["Arrows", "Number keys"]));
    s.set_pad_a_names(strings(&["OK", "5"]));
    s.set_pad_b_names(strings(&["Right softkey", "0", "#"]));
    s.set_operator("Ucom AM".into());
    s.set_sim("SIM ready".into());
    s.set_rat("3G".into());
    s.set_radio_online(true);
    s.set_battery_text("72%".into());
    s.set_battery_state("Discharging".into());
    s.set_battery_usb("Disconnected".into());
    s.set_battery_voltage("3.874 V".into());
    s.set_battery_history(model(
        (0..1440)
            .map(|i| {
                if i < 300 {
                    -1.0
                } else {
                    1.0 - i as f32 / 5000.0
                }
            })
            .collect(),
    ));
    s.set_battery_storage(
        "History is saved every five minutes and before normal restart or shutdown".into(),
    );
    s.set_build(strings(&[
        "kernel 1885e2c 2026-11-16T09:46Z bonito",
        "rom 2026-11-16T09:59Z build 269cf8d",
        "neoui 6fec461 2026-11-16T10:05Z, e32-rs 3e443ca",
    ]));
    let net = |name: &str,
               detail: &str,
               bars,
               secured,
               band: &str,
               signal: &str,
               channel,
               bssid: &str| {
        WifiNet {
            name: name.into(),
            detail: detail.into(),
            bars,
            secured,
            band: band.into(),
            signal: signal.into(),
            channel,
            bssid: bssid.into(),
        }
    };
    s.set_wifi_networks(model(vec![
        net(
            "Cafe Guest",
            "Secured · 2.4 GHz · -70 dBm",
            2,
            true,
            "2.4 GHz",
            "Fair",
            6,
            "02:00:00:00:00:02",
        ),
        net(
            "Library Free",
            "Open · 2.4 GHz · -82 dBm",
            1,
            false,
            "2.4 GHz",
            "Weak",
            1,
            "02:00:00:00:00:03",
        ),
        net(
            "TP-Link_4F2A",
            "Secured · 2.4 GHz · -91 dBm",
            0,
            true,
            "2.4 GHz",
            "Weak",
            11,
            "02:00:00:00:00:04",
        ),
    ]));
    s.set_wifi_hidden(1);
    s.set_wifi_saved_ok(true);
    s.set_apps(model(
        neoui_proto::apps::listed()
            .map(|a| AppRow {
                key: a.key.into(),
                name: a.name.into(),
                old: false,
                settings: a.settings.is_some(),
            })
            .collect(),
    ));
    s.set_old_apps(model(vec![
        AppRow {
            key: "20009aa3".into(),
            name: "X-plore".into(),
            old: true,
            settings: false,
        },
        AppRow {
            key: "2001a7b1".into(),
            name: "Frozen Bubble".into(),
            old: true,
            settings: false,
        },
    ]));
    s.set_old_total(2);
    s.set_wifi_saved(model(vec![
        WifiSaved {
            name: "Home-5G".into(),
            detail: "Connected · Secured".into(),
        },
        WifiSaved {
            name: "Cafe Guest".into(),
            detail: "Secured".into(),
        },
        WifiSaved {
            name: "Ararat-Office".into(),
            detail: "Secured · not in range".into(),
        },
    ]));
    s.set_wifi_status("3 networks".into());
    let rows = |v: &[(&str, &str)]| {
        model(
            v.iter()
                .map(|(l, d)| WifiDetail {
                    label: (*l).into(),
                    value: (*d).into(),
                })
                .collect(),
        )
    };
    s.set_wifi_link(WifiLink {
        shown: true,
        connected: true,
        name: "Home-5G".into(),
        subtitle: "Connected · 5 GHz · Excellent".into(),
        bars: 4,
        band: "5".into(),
        details: rows(&[
            ("Status", "Connected"),
            ("Signal", "Excellent (-46 dBm)"),
            ("Band", "5 GHz · channel 44"),
            ("Security", "WPA2-Personal (AES-CCMP)"),
            ("IP address", "192.168.3.57/24"),
            ("Internet", "Connected"),
        ]),
        sign_in: false,
    });
}

fn page(sh: &Shot, id: i32, title: &str) {
    let s = sh.ui.global::<SetState>();
    s.set_page(id);
    s.set_title(title.into());
    s.set_sheet(SharedString::new());
}

fn sheet(sh: &Shot, name: &str) {
    sh.ui.global::<SetState>().set_sheet(name.into());
}

#[test]
fn every_page() {
    let sh = common::app();
    fill(&sh);
    let s = sh.ui.global::<SetState>();

    // the root, in both navigation modes
    sh.shoot("set2-root-list");
    sh.shoot_at("set2-root-buttons", BUTTONS);

    // the Nokia grid (S01, ⋮ → View as grid) and search (set2-search) with what it found for "time"
    s.set_root_grid(true);
    sh.shoot("set2-root-grid");
    s.set_root_grid(false);
    let hit = |title: &str, path: &str, value: &str, page| SearchHit {
        id: "root.datetime".into(),
        title: title.into(),
        path: path.into(),
        value: value.into(),
        page,
    };
    s.set_results(model(vec![
        hit("Date & time", "Settings", "", 9),
        hit("Automatic time", "Date & time", "On", 9),
        hit("Automatic time zone", "Date & time", "On", 9),
        hit("Time zone", "Date & time", "UTC+04:00 · Yerevan", 9),
        hit("24-hour time", "Date & time", "On", 9),
        hit("Screen timeout", "Display", "30 seconds", 5),
    ]));
    s.set_searching(true);
    sh.shoot("set2-search");
    sh.tap(100.0, 48.0 + 36.0);
    assert!(
        sh.take_acts()
            .contains(&("found".to_string(), "root.datetime".to_string()))
    );
    s.set_searching(false);

    for (id, title, name) in [
        (1, "Wi-Fi", "set2-wifi"),
        (2, "Mobile network", "set2-mobile"),
        (3, "Bluetooth", "set2-bluetooth"),
        (4, "Connectivity", "set2-airplane"),
        (5, "Display", "set2-display"),
        (6, "Home & navigation", "set2-navigation"),
        (7, "Sounds", "set2-sounds"),
        (8, "Battery", "set2-battery"),
        (9, "Date & time", "set2-datetime"),
        (10, "Language & keyboard", "set2-language"),
        (11, "Developer options", "set2-developer"),
        (12, "About phone", "set2-about"),
        (13, "Apps", "set2-apps"),
        (14, "Old apps", "pad-settings"),
        (15, "Privacy", "set2-privacy"),
        (16, "Licences", "set2-licences"),
        (17, "Saved networks", "set2-wifi-saved"),
    ] {
        page(&sh, id, title);
        sh.shoot(name);
    }

    // the sheets
    page(&sh, 1, "Wi-Fi");
    s.set_wifi_current(s.get_wifi_networks().row_data(0).unwrap());
    sheet(&sh, "wifi-join");
    sh.shoot("set2-wifi-join");
    // with the shell's keyboard up (276 dp of text keyboard + digits): the card's field and buttons stay above it
    let kf = sh.ui.global::<settings_ui::KeyboardFocus>();
    kf.set_open(true);
    kf.set_text_h(276.0 * 3.0);
    sh.shoot("set2-wifi-join-keyboard");
    kf.set_open(false);
    sheet(&sh, "wifi-details");
    sh.shoot("set2-wifi-details");
    s.set_wifi(false);
    sheet(&sh, "");
    sh.shoot("set2-wifi-off");
    s.set_wifi(true);
    page(&sh, 5, "Display");
    sheet(&sh, "timeout");
    sh.shoot("set2-timeout");
    page(&sh, 9, "Date & time");
    s.set_time_auto_zone(false);
    sheet(&sh, "zone");
    sh.shoot("set2-timezone");
    sheet(&sh, "date-format");
    sh.shoot("set2-dateformat");
    sh.take_acts();

    // what taps ask for: a root row opens its page, its switch writes the setting (S02)
    let acts = |v: &[(&str, &str)]| {
        v.iter()
            .map(|(a, b)| (String::from(*a), String::from(*b)))
            .collect::<Vec<_>>()
    };
    page(&sh, 0, "Settings");
    sh.draw();
    sh.tap(100.0, 110.0);
    assert_eq!(sh.take_acts(), acts(&[("open", "1")]));
    sh.tap(315.0, 110.0);
    assert_eq!(sh.take_acts(), acts(&[("radio.wifi", "0")]));
    s.set_wifi(true);
    sh.tap(100.0, 165.0);
    assert_eq!(sh.take_acts(), acts(&[("open", "2")]));

    // Home & navigation: a mode is one tap
    page(&sh, 6, "Home & navigation");
    sh.draw();
    let first = sh.take_acts();
    sh.tap(100.0, 105.0);
    assert_eq!(
        sh.take_acts(),
        acts(&[("nav.mode", "0")]),
        "after {first:?}"
    );

    // Apps: a row opens its sheet; Open and App settings ask for the app (SE1d)
    page(&sh, 13, "Apps");
    sh.draw();
    sh.take_acts();
    s.set_app_current(s.get_apps().row_data(5).unwrap()); // Phone: it has settings of its own
    sheet(&sh, "app");
    sh.shoot("set2-app-sheet");
    sheet(&sh, "");
    // the old apps (a short ROM list so they are in view): two, more than the grid keeps, and none yet
    let rom = s.get_apps();
    s.set_apps(model(vec![
        rom.row_data(0).unwrap(),
        rom.row_data(5).unwrap(),
    ]));
    sh.shoot("set2-apps");
    s.set_old_total(70);
    sh.shoot("set2-apps-many");
    let old = s.get_old_apps();
    s.set_old_apps(model(Vec::new()));
    s.set_old_total(0);
    sh.shoot("set2-apps-no-old");
    s.set_old_apps(old);
    s.set_old_total(2);
    s.set_apps(rom);
    // a row opens its sheet; Open asks for that app
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 118.0);
    assert_eq!(s.get_sheet().as_str(), "app");
    assert_eq!(s.get_app_current().key.as_str(), "clock");
    sh.shoot("set2-app-sheet-clock");
    sh.tap(180.0, 588.0);
    assert_eq!(sh.take_acts(), acts(&[("app-open", "clock")]));
    sheet(&sh, "");

    // Saved networks: a row asks to forget that network (the app confirms first)
    page(&sh, 17, "Saved networks");
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 231.0);
    assert_eq!(sh.take_acts(), acts(&[("wifi-saved-forget", "1")]));

    // the pad sheets (pad-settings: the B button the board leaves out, D5)
    page(&sh, 14, "Old apps");
    sheet(&sh, "pad-b");
    sh.shoot("pad-settings-b");

    // every row that shows a value has one for search (settings_ui::live_value), from the sample state above
    let s = sh.ui.global::<SetState>();
    for r in neoui_proto::settings_app::rows::ROWS {
        if !settings_ui::VALUELESS.contains(&r.id) {
            assert!(
                !settings_ui::live_value(&s, r.id).is_empty(),
                "{} shows a value but search has none",
                r.id
            );
        }
    }
}

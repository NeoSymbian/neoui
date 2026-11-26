// SPDX-License-Identifier: GPL-3.0-only
//! Every Settings page with sample data, drawn to `snapshots/set2-*.png` for review against the canvas's set2-*
//! boards, and what taps on them ask the app for (`SetState.act`). One test: Slint's platform is per process.
mod common;

use common::{BUTTONS, Shot, model, strings};
use settings_ui::{
    AppRow, DriveRow, PartRow, ProfileRow, SearchHit, SetState, WifiDetail, WifiLink, WifiNet,
    WifiSaved, ZoneRow,
};
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
    let city = |name: &str, offset: &str| ZoneRow {
        name: name.into(),
        offset: offset.into(),
        current: name == "Yerevan",
    };
    s.set_cities(model(vec![
        city("Moscow", "UTC+03:00"),
        city("Nairobi", "UTC+03:00"),
        city("Riyadh", "UTC+03:00"),
        city("Baghdad", "UTC+03:00"),
        city("Tehran", "UTC+03:30"),
        city("Dubai", "UTC+04:00"),
        city("Yerevan", "UTC+04:00"),
        city("Tbilisi", "UTC+04:00"),
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
    // Storage (set2-storage, SE3b): the board's drives and parts
    let drive = |title: &str, used: &str, note: &str, fill: f32| DriveRow {
        title: title.into(),
        used: used.into(),
        note: note.into(),
        fill,
    };
    s.set_drives(model(vec![
        drive(
            "E: Mass memory",
            "12.4 of 41.8 GB",
            "Media and documents",
            0.3,
        ),
        drive(
            "C: Phone",
            "2.1 of 8 GB",
            "Settings, installed apps, app data",
            0.26,
        ),
        drive("D: RAM", "18 MB", "Temporary files", -1.0),
        drive("Z: System", "1.2 GB", "Read-only", -1.0),
    ]));
    let bar = |name: &str, size: &str, start: f32, share: f32| PartRow {
        name: name.into(),
        size: size.into(),
        start,
        share,
    };
    let part = |name: &str, size: &str| bar(name, size, 0.0, 0.0);
    s.set_storage_parts(model(vec![
        bar("Apps", "1.4 GB", 0.0, 0.028),
        bar("Files", "11.8 GB", 0.028, 0.237),
        bar("Other (system and app data)", "1.3 GB", 0.265, 0.026),
        bar("Free", "35.3 GB", 0.291, 0.709),
    ]));
    s.set_storage_kinds(model(vec![
        part("Images", "6.2 GB"),
        part("Video", "3.9 GB"),
        part("Audio", "1.1 GB"),
        part("Documents", "210 MB"),
        part("Other files", "400 MB"),
    ]));
    s.set_storage_counted(true);
    s.set_storage_status("Last counted 06:50".into());
    s.set_storage_free("35.3 GB free".into());
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
    let mut profiles = neoui_proto::sound_profile::builtins();
    profiles[0].volume = 7;
    profiles.push(neoui_proto::sound_profile::Profile {
        id: 5,
        name: "Gym".into(),
        ring: true,
        volume: 10,
        vibrate: false,
        ringtone: neoui_proto::sound_profile::tone::TWO_BEEPS,
        message_tone: neoui_proto::sound_profile::tone::NONE,
    });
    s.set_profiles(model(
        profiles
            .iter()
            .map(|p| ProfileRow {
                id: i32::from(p.id),
                name: p.name.as_str().into(),
                summary: p.summary().into(),
                ring: p.ring,
                volume: i32::from(p.volume),
                vibrate: p.vibrate,
                ringtone: i32::from(p.ringtone),
                message_tone: i32::from(p.message_tone),
            })
            .collect(),
    ));
    use neoui_proto::sound_profile::tone;
    s.set_ring_tones(strings(&tone::ALL.map(tone::ring_name)));
    s.set_message_tones(strings(&tone::ALL.map(tone::message_name)));
    s.set_active_profile(0);
    s.set_active_profile_name("General".into());
    s.set_active_profile_summary(s.get_profiles().row_data(0).unwrap().summary);
    s.set_profile_current(s.get_profiles().row_data(0).unwrap());
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
            connected: true,
        },
        WifiSaved {
            name: "Cafe Guest".into(),
            detail: "Secured".into(),
            connected: false,
        },
        WifiSaved {
            name: "Ararat-Office".into(),
            detail: "Secured · not in range".into(),
            connected: false,
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

/// The centres of a plain list's first three rows (dp, the header included): for taps.
const ROW1: f32 = 84.0;
const ROW2: f32 = 156.0;
const ROW3: f32 = 228.0;

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
    let hit = |title: &str, path: &str, value: &str, page| {
        let m = settings_ui::split_match(title, "time");
        SearchHit {
            id: "root.datetime".into(),
            title: title.into(),
            pre: m.pre.into(),
            mark: m.mark.into(),
            post: m.post.into(),
            gap_before: m.gap_before,
            gap_after: m.gap_after,
            path: path.into(),
            value: value.into(),
            page,
        }
    };
    s.set_results(model(vec![
        hit("Date & time", "Settings", "", 9),
        hit(
            "Same height for all time zones and languages",
            "Language & keyboard",
            "On",
            10,
        ),
        hit("Automatic time", "Date & time", "On", 9),
        hit("Automatic time zone", "Date & time", "On", 9),
        hit("Time zone", "Date & time", "UTC+04:00 · Yerevan", 9),
        hit("24-hour time", "Date & time", "On", 9),
        hit("Screen timeout", "Display", "30 seconds", 5),
    ]));
    s.set_query("time".into());
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
        (7, "Sounds & profiles", "set2-sounds"),
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
        (18, "General", "set2-profile"),
    ] {
        page(&sh, id, title);
        sh.shoot(name);
    }

    // Storage (SE3b): counted, counting, and a phone without E: whose scan couldn't read two folders
    page(&sh, 20, "Storage");
    sh.shoot("set2-storage");
    s.set_storage_counting(true);
    s.set_storage_status("Counting… 120 folders".into());
    sh.shoot("set2-storage-counting");
    s.set_storage_counting(false);
    s.set_storage_status("Last counted 06:50".into());
    let (drives, parts) = (s.get_drives(), s.get_storage_parts());
    let bar = |name: &str, size: &str, start: f32, share: f32| PartRow {
        name: name.into(),
        size: size.into(),
        start,
        share,
    };
    s.set_drives(model(drives.iter().skip(1).collect()));
    s.set_storage_parts(model(vec![
        bar("Apps", "320 MB", 0.0, 0.04),
        bar("Files", "1.2 GB", 0.04, 0.15),
        bar("Other (system and app data)", "580 MB", 0.19, 0.07),
        bar("Free", "5.9 GB", 0.26, 0.74),
    ]));
    s.set_storage_unreadable("2 folders couldn't be read".into());
    sh.shoot("set2-storage-no-e");
    s.set_drives(drives);
    s.set_storage_parts(parts);
    s.set_storage_unreadable("".into());

    // Battery usage (SE3a): what the history holds, then before its first reading
    s.set_usage_known(true);
    s.set_usage_title("Since full charge".into());
    s.set_usage_levels("From 100% to 72% over 9 h 40 min".into());
    s.set_usage_on_battery("On battery 3 h 34 min".into());
    s.set_usage_gaps("Off or not recorded for 6 h, not counted".into());
    page(&sh, 19, "Battery usage");
    sh.shoot("set2-battery-usage");
    s.set_usage_known(false);
    sh.shoot("set2-battery-usage-empty");

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
    page(&sh, 24, "Time zone");
    s.set_time_auto_zone(false);
    sh.shoot("set2-timezone");
    // a city's tap picks it and goes back to Date & time
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 160.0 + 28.0);
    assert_eq!(
        sh.take_acts(),
        vec![
            ("zone-pick".to_string(), "0".to_string()),
            ("back".to_string(), String::new())
        ]
    );
    page(&sh, 9, "Date & time");
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

    // Sounds & profiles (SE2a): a row's sheet activates it or opens its page; the page asks the reducer
    page(&sh, 7, "Sounds & profiles");
    s.set_profile_current(s.get_profiles().row_data(2).unwrap()); // Meeting
    sheet(&sh, "profile");
    sh.shoot("set2-profile-sheet");
    sheet(&sh, "profile-add");
    sh.shoot("set2-profile-add");
    sheet(&sh, "");
    for (i, name) in [(4, "set2-profile-offline"), (5, "set2-profile-added")] {
        s.set_profile_current(s.get_profiles().row_data(i).unwrap());
        page(&sh, 18, s.get_profile_current().name.as_str());
        sh.shoot(name);
    }
    // a ringtone's sheet (set2-ringtone): a tap asks for the tone (the app sets and previews it)
    s.set_profile_current(s.get_profiles().row_data(0).unwrap());
    page(&sh, 18, "General");
    page(&sh, 23, "Ringtone");
    sh.shoot("set2-ringtone");
    // a row chooses the tone, its ▶ only plays it
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 82.0 + 64.0 + 32.0);
    sh.tap(320.0, 82.0 + 64.0 + 32.0);
    assert_eq!(
        sh.take_acts(),
        acts(&[("profile-tone", "r,1"), ("tone-preview", "1")])
    );
    sheet(&sh, "");
    page(&sh, 7, "Sounds & profiles");
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 413.0); // Meeting's row (the third, under the active profile's card)
    assert_eq!(s.get_sheet().as_str(), "profile");
    assert_eq!(s.get_profile_current().id, 2);
    sheet(&sh, "");
    sh.take_acts();
    sh.tap(320.0, 140.0); // the card's ✎: the active profile's page
    assert_eq!(sh.take_acts(), acts(&[("profile-open", "0")]));

    // Apps: a row opens its sheet; Open and App settings ask for the app (SE1d)
    page(&sh, 13, "Apps");
    sh.draw();
    sh.take_acts();
    s.set_app_current(s.get_old_apps().row_data(0).unwrap()); // an old app keeps the sheet
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
    // a ROM app's row opens its page (SE4a)
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 118.0);
    assert_eq!(sh.take_acts(), acts(&[("app-detail", "clock")]));
    // the app's page: Open, Stop while it runs, Clear data; each asks the app (which confirms first)
    s.set_app_current(s.get_apps().row_data(0).unwrap());
    s.set_app_running(true);
    s.set_app_stoppable(true);
    s.set_app_clear("Alarms, timers, stopwatch, world clocks, its settings".into());
    page(&sh, 21, "Clock");
    sh.shoot("set2-app-detail");
    for (y, act) in [(ROW1, "app-open"), (ROW2, "app-stop"), (ROW3, "app-clear")] {
        sh.tap(100.0, y);
        assert_eq!(sh.take_acts(), acts(&[(act, "clock")]), "{act}");
    }
    // not running: Stop does nothing; a resident (Phone) has no Stop, an app without an owner path no Clear data
    s.set_app_running(false);
    sh.draw();
    sh.tap(100.0, ROW2);
    assert!(sh.take_acts().is_empty());
    s.set_app_current(s.get_apps().row_data(5).unwrap());
    s.set_app_stoppable(false);
    s.set_app_clear("The call history (contacts stay)".into());
    page(&sh, 21, "Phone");
    sh.shoot("set2-app-detail-phone");

    // Reset (SE4a): each row asks the app, which confirms first
    page(&sh, 22, "Reset");
    sh.shoot("set2-reset");
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, ROW1);
    assert_eq!(sh.take_acts(), acts(&[("reset", "network")]));
    sh.tap(100.0, ROW2);
    assert_eq!(sh.take_acts(), acts(&[("reset", "settings")]));

    // Saved networks: a row asks to forget that network (the app confirms first)
    page(&sh, 17, "Saved networks");
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 231.0);
    assert_eq!(sh.take_acts(), acts(&[("wifi-saved-forget", "1")]));

    // SE3a: Show touches and Keypress vibration write their keys (off by default)
    page(&sh, 11, "Developer options");
    sh.draw();
    sh.take_acts();
    sh.tap(321.0, 402.0);
    assert_eq!(sh.take_acts(), acts(&[("developer.show_touches", "1")]));
    page(&sh, 10, "Language & keyboard");
    sh.draw();
    sh.take_acts();
    sh.tap(321.0, 496.0);
    assert_eq!(sh.take_acts(), acts(&[("keyboard.vibrate", "1")]));

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

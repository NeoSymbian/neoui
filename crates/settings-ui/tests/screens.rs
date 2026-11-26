// SPDX-License-Identifier: GPL-3.0-only
//! Every Settings page with sample data, drawn to `snapshots/set2-*.png` for review against the canvas's set2-*
//! boards, and what taps on them ask the app for (`SetState.act`). One test: Slint's platform is per process.
mod common;

use common::{BUTTONS, Shot, model, strings};
use neoui_proto::settings_app::page::Page;
use neoui_proto::settings_app::row_model::{
    Destination, Epochs, InstanceId, Reading, RowKey, RowSnapshot, Sample, resolve_hit,
};
use neoui_proto::settings_app::rows::{ROOT_LIST, Search};
use settings_ui::row_model as rm;
use settings_ui::{
    AppRow, DriveRow, PartRow, ProfileRow, SetState, WifiDetail, WifiLink, WifiNet, WifiSaved,
    ZoneRow,
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
    s.set_keyboard_hints(strings(&[
        "QWERTY",
        "Հայերեն · Phonetic",
        "Русский · ЙЦУКЕН",
    ]));
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
    s.set_screen_on("2 h 14 min since charged".into()); // D5
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
        letter: title[..1].into(),
        title: title.into(),
        used: used.into(),
        note: note.into(),
        fill,
    };
    s.set_drives(model(vec![
        drive(
            "E: Mass memory",
            "12.4 of 41.8 GB",
            "Media and documents · FAT32",
            0.297,
        ),
        drive(
            "C: Phone",
            "2.1 of 8 GB",
            "Settings, installed apps, app data · FAT32",
            0.262,
        ),
        drive("D: RAM", "18 MB", "Temporary files", -1.0),
        drive(
            "Z: System",
            "Read-only",
            "ROFS + core ROM · read-only",
            -1.0,
        ),
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
    s.set_storage_free("C: 3.1 GB free".into());
    // D3: C:'s own Apps, Files and Other inside its bar (the board's 11.2 %, 6.2 %, 8.8 %)
    s.set_storage_c_parts(model(vec![
        bar("", "", 0.0, 0.112),
        bar("", "", 0.112, 0.062),
        bar("", "", 0.174, 0.088),
    ]));
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
    let profiles = profiles();
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
                keypad: p.keypad,
                touch: p.touch,
                tone_name: neoui_proto::sound_profile::tone::ring_name(p.ringtone).into(),
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
    // D6: Messages' "tone and vibrate", as MODEMD keeps it
    s.set_sms_alert(0);
    s.set_message_summary(profiles[0].message_summary(Some(0)).into());
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
        bssid: "02:00:00:00:00:01".into(),
        name: "Home-5G".into(),
        subtitle: "Connected · 5 GHz · Excellent".into(),
        bars: 4,
        band: "5".into(),
        details: rows(&[
            ("Status", "Connected"),
            ("Signal", "Excellent (-46 dBm)"),
            ("Band", "5 GHz · channel 44"),
            ("Security", "WPA2-Personal (AES-CCMP)"),
            // D2: WIFID's link record
            ("Access point", "02:00:00:00:00:01"),
            ("Phone MAC", "02:4e:c1:47:28:a4"),
            ("Association ID", "1"),
            ("Received", "120 frames (96 ARP/IP)"),
            ("IP address", "192.168.3.57/24"),
            ("Gateway", "192.168.3.1"),
            ("DNS", "192.168.3.1"),
            ("Internet", "Connected"),
        ]),
        sign_in: false,
    });
    // D2: the kernel console (off) and the booted slot
    s.set_console_ok(true);
    s.set_slot("a".into());
    // D7: Medium text, the Night wallpaper; D8: the clipboard toast on
    s.set_font_sizes(strings(&["Small", "Medium", "Large", "Largest"]));
    s.set_font_scales(strings(&["87.5 %", "100 %", "112.5 %", "125 %"]));
    s.set_font_size(1);
    s.set_wallpaper("night".into());
    s.set_wallpaper_name("Night".into());
    s.set_clip_notice(true);
}

/// The app row with this key (the catalogue's order changes as apps come and go).
fn by_key(apps: &slint::ModelRc<AppRow>, key: &str) -> AppRow {
    apps.iter().find(|a| a.key == key).unwrap()
}

/// The production projection of the state now (D1), with the root's summaries published from it.
fn project(sh: &Shot, epochs: Epochs) -> RowSnapshot {
    let s = sh.ui.global::<SetState>();
    let snap = rm::project(&s, Sample::default(), epochs);
    rm::publish(&s, &snap);
    snap
}

/// Search `query` through the projector into the screen; the hits its tokens stand for.
fn searched(
    sh: &Shot,
    snap: &RowSnapshot,
    query: &str,
) -> Vec<neoui_proto::settings_app::row_model::Hit> {
    let s = sh.ui.global::<SetState>();
    let (shown, hits) = rm::results(snap, query);
    s.set_results(model(shown));
    s.set_query(query.into());
    s.set_searching(true);
    hits
}

/// The profiles the pages show: the built-ins (General at volume 7) and an added Gym.
fn profiles() -> Vec<neoui_proto::sound_profile::Profile> {
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
        keypad: false,
        touch: true,
    });
    profiles[0].keypad = true; // a fresh General (the board)
    profiles
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

    // the root, in both navigation modes, its values from the projection (D1)
    let snap = project(&sh, Epochs::default());
    sh.shoot("set2-root-list");
    sh.shoot_at("set2-root-buttons", BUTTONS);
    // each root value is its row's search value (the page binds summaries[i], tests/rows.rs)
    let summaries: Vec<String> = s.get_summaries().iter().map(|v| v.to_string()).collect();
    for (i, id) in ROOT_LIST.iter().enumerate() {
        let row = snap.get(&RowKey::of(id)).unwrap();
        assert_eq!(summaries[i], row.value_text, "{id}");
        if row.search == Search::PublicValue {
            let hits = neoui_proto::settings_app::row_model::search(&snap, &row.title);
            let hit = hits.iter().find(|h| h.key == row.key).expect(id);
            assert_eq!(snap.rows[hit.row].value_text, summaries[i], "{id}");
        }
    }
    assert_eq!(summaries[8], "C: 3.1 GB free");

    // the Nokia grid (S01, ⋮ → View as grid)
    s.set_root_grid(true);
    sh.shoot("set2-root-grid");
    s.set_root_grid(false);

    // search (set2-search) for "time" through the production projector: the actual results, their real tokens
    let hits = searched(&sh, &snap, "time");
    assert_eq!(s.get_results().row_count(), hits.len());
    let first = &snap.rows[hits[0].row];
    assert_eq!(
        (first.title.as_str(), first.path.as_str()),
        ("Automatic time", "Date & time")
    );
    sh.shoot("set2-search");
    sh.tap(100.0, 48.0 + 36.0);
    assert_eq!(sh.take_acts(), vec![("found".to_string(), "0".to_string())]);
    let open = resolve_hit(0, &hits, &snap).unwrap();
    assert_eq!(
        open,
        Destination {
            page: Page::DateTime,
            context: None,
            sheet: None,
            focus: Some(RowKey::of("time.auto"))
        }
    );
    // the second result is the second token
    sh.tap(100.0, 48.0 + 36.0 + 72.0);
    assert_eq!(sh.take_acts(), vec![("found".to_string(), "1".to_string())]);

    // a value changed while its page is closed: the hit shows it, and its old token still opens the same row
    s.set_timeout_text("1 minute".into());
    let later = project(&sh, Epochs::default());
    let timeout = hits
        .iter()
        .find(|h| h.key == RowKey::of("display.timeout"))
        .unwrap();
    assert_eq!(
        resolve_hit(timeout.token, &hits, &later).map(|d| d.page),
        Ok(Page::Display)
    );
    let now = searched(&sh, &later, "1 min");
    assert_eq!(later.rows[now[0].row].title, "Screen timeout");
    s.set_timeout_text("30 seconds".into());

    // airplane mode over the last technology: Off on the root and in search alike
    s.set_airplane(true);
    let snap = project(&sh, Epochs::default());
    assert_eq!(s.get_summaries().row_data(1).unwrap(), "Off");
    let hits = searched(&sh, &snap, "mobile network");
    assert_eq!(snap.rows[hits[0].row].value_text, "Off");
    s.set_airplane(false);

    // an inactive profile's row opens its own page, and two profiles' Ring volume are two rows
    let snap = project(&sh, Epochs::default());
    let hits = searched(&sh, &snap, "ring volume");
    let meeting = RowKey::with("profile.volume", InstanceId::Profile(2));
    let hit = hits.iter().find(|h| h.key == meeting).unwrap();
    assert_eq!(snap.rows[hit.row].path, "Meeting");
    assert_eq!(
        resolve_hit(hit.token, &hits, &snap).unwrap().context,
        Some(InstanceId::Profile(2))
    );
    assert!(hits.len() >= 3);
    // a profile deleted (its id reusable): the profiles epoch moves on and the old token opens nothing
    let gone = project(
        &sh,
        Epochs {
            profiles: 1,
            ..Epochs::default()
        },
    );
    assert!(resolve_hit(hit.token, &hits, &gone).is_err());

    // a producer that isn't there: no console row, and C: unread reads so on the root
    s.set_console_ok(false);
    s.set_storage_free("".into());
    let snap = project(&sh, Epochs::default());
    assert!(searched(&sh, &snap, "console overlay").is_empty());
    assert_eq!(s.get_summaries().row_data(8).unwrap(), "C: can't be read");
    s.set_console_ok(true);
    s.set_storage_free("C: 3.1 GB free".into());
    let snap = project(&sh, Epochs::default());
    // Back to search: the query and the result stay, shown selected
    let hits = searched(&sh, &snap, "time");
    s.set_search_selected(hits[1].token as i32);
    sh.shoot("set2-search-back");
    s.set_search_selected(-1);
    s.set_searching(false);
    s.set_query("".into());

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

    // Battery usage (D5): the charge cycle's texts (battery::cycle::text) since a full charge (the board's numbers),
    // before one, with a gap, from an older shell (INFO 5, no screen time), and before the first reading
    use neoui_proto::battery::cycle::{from_usage, text};
    use neoui_proto::settings_app::wire::{BatteryCycle, cycle as bits};
    let show = |t: &neoui_proto::battery::cycle::CycleText| {
        s.set_usage_known(true);
        s.set_usage_title(t.title.as_str().into());
        s.set_usage_from(t.from.as_str().into());
        s.set_usage_to(t.to.as_str().into());
        s.set_usage_span(t.span.as_str().into());
        s.set_usage_on_battery(t.on_battery.as_str().into());
        s.set_usage_gaps(t.gaps.as_str().into());
        s.set_screen_on(t.screen.as_str().into());
    };
    let cycle = BatteryCycle {
        flags: bits::FULL_THEN_DISCHARGE | bits::SCREEN_VALID,
        start_percent: Some(100),
        end_percent: Some(72),
        start_utc: 1_790_000_000,
        last_utc: 1_790_000_000 + 9 * 3600 + 40 * 60,
        on_battery_s: 9 * 3600 + 40 * 60,
        screen_on_battery_s: 2 * 3600 + 14 * 60,
        gap_s: 0,
    };
    page(&sh, 19, "Battery usage");
    show(&text(
        &BatteryCycle {
            flags: bits::SCREEN_VALID,
            start_percent: Some(85),
            ..cycle
        },
        true,
    ));
    sh.shoot("set2-battery-usage-observed");
    show(&text(
        &BatteryCycle {
            flags: cycle.flags | bits::INCOMPLETE,
            on_battery_s: 3 * 3600 + 40 * 60,
            gap_s: 6 * 3600,
            ..cycle
        },
        true,
    ));
    sh.shoot("set2-battery-usage-partial");
    show(&text(
        &from_usage(&neoui_proto::battery::Usage {
            since_full: true,
            start_utc: cycle.start_utc,
            end_utc: cycle.last_utc,
            start_percent: Some(100),
            end_percent: Some(72),
            on_battery_s: 3 * 3600 + 40 * 60,
            gap_s: 6 * 3600,
        }),
        false,
    ));
    sh.shoot("set2-battery-usage-old-shell");
    s.set_usage_known(false);
    s.set_screen_on("".into());
    sh.shoot("set2-battery-usage-empty");
    show(&text(&cycle, true));
    assert_eq!(s.get_screen_on(), "2 h 14 min since charged");
    sh.shoot("set2-battery-usage");
    page(&sh, 8, "Battery");
    sh.shoot("set2-battery");

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
    let profiles = profiles();
    for (i, name) in [(4, "set2-profile-offline"), (5, "set2-profile-added")] {
        s.set_profile_current(s.get_profiles().row_data(i).unwrap());
        s.set_message_summary(profiles[i].message_summary(Some(0)).into());
        page(&sh, 18, s.get_profile_current().name.as_str());
        sh.shoot(name);
    }
    s.set_message_summary(profiles[0].message_summary(Some(0)).into());
    // a ringtone's sheet (set2-ringtone): a tap asks for the tone (the app sets and previews it)
    s.set_profile_current(s.get_profiles().row_data(0).unwrap());
    page(&sh, 18, "General");
    page(&sh, 23, "Ringtone");
    // D6: the built-in ringtone's preview, a second in (one period is 2 s, so never the board's 0:03)
    s.set_preview_tone(0);
    s.set_preview_text(neoui_proto::sound_profile::preview::label(false, 1_000).into());
    sh.shoot("set2-ringtone");
    // its ⏸ pauses it (the app asks AUDIOD); paused, the line freezes and ▶ plays on
    sh.draw();
    sh.take_acts();
    sh.tap(320.0, 82.0 + 32.0);
    assert_eq!(sh.take_acts(), acts(&[("tone-pause", "")]));
    s.set_preview_paused(true);
    s.set_preview_text(neoui_proto::sound_profile::preview::label(true, 1_000).into());
    sh.shoot("set2-ringtone-paused");
    s.set_preview_tone(-1);
    s.set_preview_paused(false);
    // D6 (SE2c-1): two sounds of the user's, one chosen, and a refused import; a long press opens its sheet
    s.set_your_tones(model(vec![
        settings_ui::YourTone {
            id: 4,
            name: "Morning bell".into(),
        },
        settings_ui::YourTone {
            id: 9,
            name: "Gong".into(),
        },
    ]));
    let mut general = s.get_profile_current();
    general.ringtone = 4;
    s.set_profile_current(general);
    s.set_import_note("Use a 16-bit PCM WAV".into()); // neoaudio::wav::WavError::Format
    sh.shoot("set2-ringtone-yours");
    s.set_your_current(s.get_your_tones().row_data(0).unwrap());
    sheet(&sh, "your-tone");
    sh.shoot("set2-ringtone-sheet");
    sheet(&sh, "");
    s.set_your_tones(model(vec![]));
    s.set_import_note("".into());
    s.set_profile_current(s.get_profiles().row_data(0).unwrap());
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
    sh.shoot("set2-apps"); // every ROM app, as the board lists them
    // the old apps (a short ROM list so they are in view): two, more than the grid keeps, and none yet
    let rom = s.get_apps();
    s.set_apps(model(vec![rom.row_data(0).unwrap(), by_key(&rom, "phone")]));
    sh.shoot("set2-apps-old");
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
    // the header, Storage (Clear data), App (Open, Stop app)
    for (y, act) in [
        (180.0, "app-clear"),
        (278.0, "app-open"),
        (343.0, "app-stop"),
    ] {
        sh.tap(100.0, y);
        assert_eq!(sh.take_acts(), acts(&[(act, "clock")]), "{act}");
    }
    // not running: Stop does nothing; a resident (Phone) has no Stop, an app without an owner path no Clear data
    s.set_app_running(false);
    sh.draw();
    sh.tap(100.0, 343.0);
    assert!(sh.take_acts().is_empty());
    s.set_app_current(by_key(&s.get_apps(), "phone"));
    s.set_app_stoppable(false);
    s.set_app_clear("The call history (contacts stay)".into());
    page(&sh, 21, "Phone");
    sh.shoot("set2-app-detail-phone");

    // Reset (SE4a): each row asks the app, which confirms first
    page(&sh, 22, "Reset");
    sh.shoot("set2-reset");
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 175.0); // under the note and the Reset header
    assert_eq!(sh.take_acts(), acts(&[("reset", "network")]));
    sh.tap(100.0, 246.0);
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
    sh.tap(321.0, 474.0);
    assert_eq!(sh.take_acts(), acts(&[("developer.show_touches", "1")]));
    // D2: the console switch asks the app (which writes the HAL and reads it back)
    sh.tap(321.0, 368.0);
    assert_eq!(sh.take_acts(), acts(&[("console", "1")]));
    // a kernel that can't say: no row, Show touches back in its place
    s.set_console_ok(false);
    s.set_console_shown(false);
    sh.draw();
    sh.tap(321.0, 402.0);
    assert_eq!(sh.take_acts(), acts(&[("developer.show_touches", "0")]));
    s.set_console_ok(true);
    page(&sh, 10, "Language & keyboard");
    sh.draw();
    sh.take_acts();
    sh.tap(321.0, 496.0);
    assert_eq!(sh.take_acts(), acts(&[("keyboard.vibrate", "1")]));

    // D7: Wallpaper and Font size (the board shows Large chosen, the page itself drawn at Medium)
    page(&sh, 25, "Wallpaper");
    sh.shoot("set2-wallpaper");
    sh.take_acts();
    sh.tap(150.0, 160.0); // Black, the second swatch
    assert_eq!(sh.take_acts(), acts(&[("display.wallpaper", "black")]));
    s.set_font_size(2);
    page(&sh, 26, "Font size");
    sh.shoot("set2-fontsize");
    sh.take_acts();
    sh.tap(100.0, 506.0); // Largest, the last of the sizes
    assert_eq!(sh.take_acts(), acts(&[("display.font_size", "3")]));
    // the whole app at Largest: rows keep their height, the text grows
    sh.ui.global::<settings_ui::Theme>().set_font_size(3);
    s.set_font_size(3);
    page(&sh, 5, "Display");
    sh.shoot("set2-display-largest");
    sh.ui.global::<settings_ui::Theme>().set_font_size(1);
    s.set_font_size(1);
    // the Display rows open their pages
    sh.draw();
    sh.take_acts();
    sh.tap(100.0, 238.0);
    sh.tap(100.0, 350.0);
    assert_eq!(sh.take_acts(), acts(&[("open", "26"), ("open", "25")]));
    // D8: the clipboard toast's switch, and the local log in Dev tools
    page(&sh, 15, "Privacy");
    sh.draw();
    sh.take_acts();
    sh.tap(321.0, 186.0);
    sh.tap(100.0, 292.0);
    assert_eq!(
        sh.take_acts(),
        acts(&[("privacy.clip_notice", "0"), ("devtools", "console")])
    );
    s.set_clip_notice(true);

    // the pad sheets (pad-settings: the B button the board leaves out, D5)
    page(&sh, 14, "Old apps");
    sheet(&sh, "pad-b");
    sh.shoot("pad-settings-b");

    // every row that shows a value has one for search, from the sample state above (D1's projection)
    let snap = project(&sh, Epochs::default());
    for r in &snap.rows {
        if r.search == Search::PublicValue {
            assert!(
                matches!(&r.reading, Reading::Value(v) if !v.is_empty()),
                "{:?} shows a value but search has {:?}",
                r.key,
                r.reading
            );
        }
    }
}

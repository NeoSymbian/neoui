// SPDX-License-Identifier: GPL-3.0-only
//! The row catalogue (`neoui_proto::settings_app::rows`, D1's row model) and the pages can't drift:
//! - every (page, row title) the `.slint` pages draw, literal or from a model (`choices`), is in the catalogue, every
//!   catalogue row is on its page, and every family is drawn by a repeater over the model its provider reads;
//! - the root list shows `SetState.summaries[i]` for `ROOT_LIST[i]`, the grid's tiles are `ROOT_GRID`'s;
//! - the manifest below: for each board's instanced row, its text, provider, destination and the gate without which
//!   it isn't offered.
//!
//! Licences (not settings), sheets and the search results themselves are left out.
use neoui_proto::settings_app::choices::{KEYBOARDS, NAV_MODES};
use neoui_proto::settings_app::page::Page;
use neoui_proto::settings_app::row_model::{
    AppFact, AppId, Availability, ChannelFact, Console, DriveFact, Facts, KindsState, LinkFact,
    NotifyFact, Reading, Scalar, WifiField, project_all, search,
};
use neoui_proto::settings_app::rows::{ProviderId, ROOT_GRID, ROOT_LIST, ROWS};
use neoui_proto::settings_app::storage::Kind;
use std::collections::BTreeSet;
use std::path::Path;

/// The page component each page is drawn by.
const COMPONENTS: &[(&str, Page)] = &[
    ("RootPage", Page::Root),
    ("WifiPage", Page::Wifi),
    ("ConnectivityPage", Page::Connectivity),
    ("MobilePage", Page::Mobile),
    ("BluetoothPage", Page::Bluetooth),
    ("DisplayPage", Page::Display),
    ("NavigationPage", Page::Navigation),
    ("SoundsPage", Page::Sounds),
    ("BatteryPage", Page::Battery),
    ("DateTimePage", Page::DateTime),
    ("KeyboardPage", Page::Keyboard),
    ("PrivacyPage", Page::Privacy),
    ("AppsPage", Page::Apps),
    ("OldAppsPage", Page::OldApps),
    ("DeveloperPage", Page::Developer),
    ("AboutPage", Page::About),
    ("WifiSavedPage", Page::WifiSaved),
    ("ProfilePage", Page::Profile),
    ("StoragePage", Page::Storage),
    ("ResetPage", Page::Reset),
    ("BatteryUsagePage", Page::BatteryUsage),
    ("PanelPage", Page::Panel),
    ("NotificationsPage", Page::Notifications),
    ("NotifyAppPage", Page::NotifyApp),
    ("SimLockPage", Page::SimLock),
];

fn page_file(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ui/pages")
            .join(name),
    )
    .unwrap()
}

/// (page, title) of every row the pages draw.
fn page_rows() -> BTreeSet<(&'static str, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/pages");
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap() == "licences.slint" {
            continue;
        }
        let mut page = None;
        for line in std::fs::read_to_string(&path).unwrap().lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("export component ") {
                let name = rest.split_whitespace().next().unwrap();
                page = COMPONENTS
                    .iter()
                    .find(|(c, _)| *c == name)
                    .map(|(_, p)| p.name());
            } else if let (Some(p), Some(rest)) = (page, line.strip_prefix("title: \"")) {
                out.insert((p, rest.split('"').next().unwrap().to_string()));
            }
        }
    }
    // rows drawn from a model: the repeaters must be there, bound to the models NEOSET fills from `choices`
    assert!(page_file("device.slint").contains("for name[i] in SetState.nav-modes: ModeCard"));
    assert!(
        page_file("personal.slint").contains("for name[i] in SetState.keyboard-names: ListRow")
    );
    for (name, _) in NAV_MODES {
        out.insert((Page::Navigation.name(), name.to_string()));
    }
    for (_, name, _) in KEYBOARDS {
        out.insert((Page::Keyboard.name(), name.to_string()));
    }
    // Storage's by-kind section is a search target though not a list row; About's slot is a key/value row (D2)
    assert!(page_file("storage.slint").contains("SectionHeader { text: \"C: and E: by kind\"; }"));
    out.insert((Page::Storage.name(), "C: and E: by kind".to_string()));
    assert!(page_file("system.slint").contains("key: \"Active slot\";"));
    out.insert((Page::About.name(), "Active slot".to_string()));
    out
}

#[test]
fn the_catalogue_matches_the_pages() {
    let pages = page_rows();
    let table: BTreeSet<(&'static str, String)> = ROWS
        .iter()
        .filter(|r| !r.title.is_empty())
        .map(|r| (r.on.name(), r.title.to_string()))
        .collect();
    let missing_in_table: Vec<_> = pages.difference(&table).collect();
    let missing_on_pages: Vec<_> = table.difference(&pages).collect();
    assert!(
        missing_in_table.is_empty(),
        "(page, row) on the pages but not in settings_app::rows: {missing_in_table:?}"
    );
    assert!(
        missing_on_pages.is_empty(),
        "(page, row) in settings_app::rows but not on that page: {missing_on_pages:?}"
    );
}

/// Each family's repeater: the file, the line that draws one instance per model row.
const REPEATERS: &[(ProviderId, &str, &str)] = &[
    (
        ProviderId::Profiles,
        "device.slint",
        "for p in SetState.profiles: ListRow",
    ),
    (
        ProviderId::Apps,
        "system.slint",
        "for a[i] in SetState.apps: ListRow",
    ),
    (
        ProviderId::Apps,
        "system.slint",
        "for a in SetState.old-apps: ListRow",
    ),
    (
        ProviderId::Drives,
        "storage.slint",
        "for d[i] in SetState.drives: DriveItem",
    ),
    (
        ProviderId::OverviewKinds,
        "storage.slint",
        "for k[i] in SetState.storage-kinds: ListRow",
    ),
    (
        ProviderId::WifiFields,
        "wifi.slint",
        "for d in SetState.wifi-link.details: HorizontalLayout",
    ),
    (
        ProviderId::NotifyApps,
        "notifications.slint",
        "for a in SetState.notify-apps: ListRow",
    ),
    (
        ProviderId::Channels,
        "notifications.slint",
        "for c[i] in SetState.channels: VerticalLayout",
    ),
];

#[test]
fn every_family_is_drawn_by_its_repeater() {
    for r in ROWS.iter().filter(|r| r.title.is_empty()) {
        let mine: Vec<_> = REPEATERS.iter().filter(|x| x.0 == r.provider).collect();
        assert!(!mine.is_empty(), "{} has no repeater", r.id);
        for (_, file, line) in mine {
            assert!(page_file(file).contains(line), "{file}: {line}");
        }
    }
}

/// RootPage's rows top to bottom: (title, the value it binds, if any).
fn root_list() -> Vec<(String, Option<String>)> {
    let root = page_file("root.slint");
    let page = &root[root.find("export component RootPage").unwrap()..];
    let page = &page[..page.find("\n}\n").unwrap()];
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for line in page.lines().map(str::trim) {
        if let Some(t) = line.strip_prefix("title: \"") {
            out.push((t.split('"').next().unwrap().into(), None));
        } else if let Some(v) = line.strip_prefix("value: ") {
            out.last_mut().unwrap().1 = Some(v.trim_end_matches(';').into());
        }
    }
    out
}

#[test]
fn the_root_list_shows_the_projection() {
    let drawn = root_list();
    let ids: Vec<_> = ROOT_LIST
        .iter()
        .map(|id| ROWS.iter().find(|r| r.id == *id).unwrap())
        .collect();
    assert_eq!(
        drawn.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
        ids.iter().map(|r| r.title).collect::<Vec<_>>()
    );
    for (i, ((title, value), row)) in drawn.iter().zip(&ids).enumerate() {
        // a row with a value shows its summary, never a formatter of its own; Airplane, Privacy and About show none
        let want = (!matches!(row.id, "root.airplane" | "root.privacy" | "root.about"))
            .then(|| format!("SetState.summaries[{i}]"));
        assert_eq!(value, &want, "{title}");
    }
}

#[test]
fn the_grid_is_the_placements() {
    let root = page_file("root.slint");
    let tiles: Vec<(String, u16, i32)> = root
        .lines()
        .filter_map(|l| {
            let l = l.trim().trim_start_matches('[').trim_start_matches('{');
            let label = l.strip_prefix(" l: \"")?.split('"').next()?;
            let p = l
                .split("p: ")
                .nth(1)?
                .split(',')
                .next()?
                .trim()
                .parse()
                .ok()?;
            let s = l.split("s: ").nth(1)?.split(' ').next()?.parse().ok()?;
            Some((label.to_string(), p, s))
        })
        .collect();
    let want: Vec<(String, u16, i32)> = ROOT_GRID
        .iter()
        .map(|(l, p, s)| {
            let i = s.map_or(-1, |id| {
                ROOT_LIST.iter().position(|x| *x == id).unwrap() as i32
            });
            (l.to_string(), p.id(), i)
        })
        .collect();
    assert_eq!(tiles, want);
}

/// Facts with one of everything the manifest names.
fn facts() -> Facts {
    let mut f = Facts {
        profiles: neoui_proto::sound_profile::builtins(),
        drives: vec![DriveFact {
            letter: 'C',
            title: "C: Phone".into(),
            used: "2.1 of 8 GB".into(),
        }],
        kinds: vec![(Kind::Images, "6.2 GB".into())],
        kinds_state: KindsState::Counted("Last counted 06:50".into()),
        apps: vec![
            AppFact {
                id: AppId::Native("clock"),
                name: "Clock".into(),
                size: String::new(),
            },
            AppFact {
                id: AppId::Legacy(0x2000_9aa3),
                name: "X-plore".into(),
                size: String::new(),
            },
        ],
        link: Some(LinkFact {
            bssid: [2, 0, 0, 0, 0, 1],
            fields: vec![(WifiField::AccessPoint, "02:00:00:00:00:01".into())],
        }),
        ..Facts::default()
    };
    f.console = Console::Shown(false);
    f.notify_apps = vec![NotifyFact {
        owner: "messages",
        name: "Messages".into(),
        summary: "Banners · sound".into(),
    }];
    f.channels = vec![ChannelFact {
        owner: "messages",
        id: 1,
        name: "Conversations".into(),
        value: "Urgent".into(),
    }];
    f.scalars.insert(
        "root.notifications",
        Scalar::Text("8 of 8 apps allowed".into()),
    );
    f.scalars.insert("about.slot", Scalar::Text("a".into()));
    f.scalars.insert("radio.wifi", Scalar::On(true));
    f.scalars
        .insert("storage.free", Scalar::Text("C: 5.9 GB free".into()));
    f.scalars.insert(
        "battery.screen_on",
        Scalar::Text("2 h 14 min since charged".into()),
    );
    f.scalars.insert("sim.attempts", Scalar::Text("3".into()));
    f
}

/// (board, the row's text there, row id, provider, the page its hit opens, the gate: without it the row isn't
/// offered, or reads unavailable)
type Gate = fn(&mut Facts);
const MANIFEST: &[(&str, &str, &str, ProviderId, Page, Gate)] = &[
    (
        "set2-sounds",
        "Meeting",
        "sounds.profile",
        ProviderId::Profiles,
        Page::Sounds,
        |f| f.profiles.retain(|p| p.name != "Meeting"),
    ),
    (
        "set2-profile",
        "Ring volume",
        "profile.volume",
        ProviderId::ProfileFields,
        Page::Profile,
        |f| f.profiles.iter_mut().for_each(|p| p.ring = false),
    ),
    (
        "set2-profile",
        "Keypad tones",
        "profile.keypad",
        ProviderId::ProfileFields,
        Page::Profile,
        |f| f.profiles.retain(|p| p.id == 4), // only Offline: no such row
    ),
    (
        "set2-profile",
        "Vibrate on touch",
        "profile.touch",
        ProviderId::ProfileFields,
        Page::Profile,
        |f| f.profiles.retain(|p| p.id == 4),
    ),
    (
        "set2-storage",
        "C: Phone",
        "storage.drive",
        ProviderId::Drives,
        Page::Storage,
        |f| f.drives.clear(),
    ),
    (
        "set2-storage",
        "Images",
        "storage.kind",
        ProviderId::OverviewKinds,
        Page::Storage,
        |f| f.kinds_state = KindsState::NotCounted,
    ),
    (
        "set2-apps",
        "Clock",
        "apps.app",
        ProviderId::Apps,
        Page::AppDetail,
        |f| f.apps.retain(|a| a.name != "Clock"),
    ),
    (
        "set2-apps",
        "X-plore",
        "apps.app",
        ProviderId::Apps,
        Page::Apps,
        |f| f.apps.retain(|a| a.name != "X-plore"),
    ),
    (
        "set2-wifi-details",
        "Access point",
        "wifi.field",
        ProviderId::WifiFields,
        Page::Wifi,
        |f| f.link = None,
    ),
    (
        "set2-developer",
        "Console overlay",
        "dev.console",
        ProviderId::Builtin,
        Page::Developer,
        |f| f.console = Console::Unsupported,
    ),
    (
        "set2-about",
        "Active slot",
        "about.slot",
        ProviderId::Builtin,
        Page::About,
        |f| {
            f.scalars.remove("about.slot");
        },
    ),
    (
        "set2-notifications",
        "Messages",
        "notify.app",
        ProviderId::NotifyApps,
        Page::NotifyApp,
        |f| f.notify_apps.clear(),
    ),
    (
        "nt-settings",
        "Conversations",
        "notify.channel",
        ProviderId::Channels,
        Page::NotifyApp,
        |f| f.channels.clear(),
    ),
    (
        "set2-root-list",
        "Notifications",
        "root.notifications",
        ProviderId::Builtin,
        Page::Panel,
        |f| {
            f.scalars.remove("root.notifications");
        },
    ),
    (
        "set2-navigation",
        "Open apps columns",
        "nav.switcher_cols",
        ProviderId::Builtin,
        Page::Navigation,
        |f| {
            f.scalars.remove("nav.switcher_cols");
        },
    ),
    (
        "set2-root-list",
        "Storage",
        "root.storage",
        ProviderId::Builtin,
        Page::Storage,
        |f| {
            f.scalars.remove("storage.free");
        },
    ),
    // D5: no cycle with a display measurement yet
    (
        "set2-search",
        "Screen-on time",
        "battery.screen_on",
        ProviderId::Builtin,
        Page::BatteryUsage,
        |f| {
            f.scalars.insert(
                "battery.screen_on",
                Scalar::Text(neoui_proto::battery::cycle::SCREEN_NOT_YET.into()),
            );
        },
    ),
    // D11: the card's count, or none (MODEMD's 255); never a default
    (
        "set2-sim-lock",
        "PIN attempts left",
        "sim.attempts",
        ProviderId::Builtin,
        Page::SimLock,
        |f| {
            f.scalars.remove("sim.attempts");
        },
    ),
];

#[test]
fn the_manifest_resolves_and_its_gates_hold() {
    for (board, text, id, provider, page, gate) in MANIFEST {
        let row = ROWS.iter().find(|r| r.id == *id).unwrap();
        assert_eq!(row.provider, *provider, "{board} {text}");
        let snap = project_all(&facts());
        let hit = search(&snap, text)
            .into_iter()
            .find(|h| h.key.id.0 == *id && snap.rows[h.row].title == *text)
            .unwrap_or_else(|| panic!("{board}: {text} not found"));
        let r = &snap.rows[hit.row];
        assert_eq!(r.destination.page, *page, "{board} {text}");
        assert_eq!(r.availability, Availability::Readable, "{board} {text}");
        // without its producer: not offered, or honestly unavailable
        let mut f = facts();
        gate(&mut f);
        let snap = project_all(&f);
        let offered = snap.get(&hit.key).filter(|r| {
            r.availability != Availability::Hidden && !matches!(r.reading, Reading::Unavailable(_))
        });
        assert!(
            offered.is_none(),
            "{board} {text} without its producer: {offered:?}"
        );
    }
}

/// The boards' rows whose stage hasn't landed (g6-big-specs D12–D19): (board, the row's text, row id, the page its hit
/// opens, why). Each is drawn, does nothing, and search finds it disabled with the reason.
const NOT_YET: &[(&str, &str, &str, Page, &str)] = &[
    (
        "set2-wifi",
        "Add hidden network",
        "wifi.hidden",
        Page::Wifi,
        "Not yet (D12)",
    ),
    (
        "set2-mobile",
        "Network mode",
        "mobile.mode",
        Page::Mobile,
        "Not yet (D13)",
    ),
    (
        "set2-mobile",
        "Roaming",
        "mobile.roaming",
        Page::Mobile,
        "Not yet (D13)",
    ),
    (
        "set2-mobile",
        "Access point names",
        "mobile.apn",
        Page::Mobile,
        "Not yet (D13)",
    ),
    (
        "set2-mobile",
        "Data usage",
        "mobile.usage",
        Page::Mobile,
        "Not yet (D13)",
    ),
    (
        "set2-bluetooth",
        "Visible to nearby devices",
        "bt.visible",
        Page::Bluetooth,
        "Not yet (D14)",
    ),
    (
        "set2-bluetooth",
        "Paired devices",
        "bt.paired",
        Page::Bluetooth,
        "Not yet (D14)",
    ),
    (
        "set2-root-list",
        "Lock screen & security",
        "root.lock",
        Page::Root,
        "Not yet (D15)",
    ),
    (
        "set2-notifications",
        "Show on lock screen",
        "notify.lock",
        Page::Notifications,
        "Not yet (D15)",
    ),
    (
        "set2-storage",
        "Share E: over USB",
        "storage.usb",
        Page::Storage,
        "Not yet (FS6)",
    ),
    (
        "set2-reset",
        "Erase C:",
        "reset.erase",
        Page::Reset,
        "Not yet (D19)",
    ),
];

#[test]
fn rows_not_yet_are_found_disabled() {
    let snap = project_all(&facts());
    for (board, text, id, page, why) in NOT_YET {
        let hit = search(&snap, text)
            .into_iter()
            .find(|h| h.key.id.0 == *id)
            .unwrap_or_else(|| panic!("{board}: {text} not found"));
        let r = &snap.rows[hit.row];
        assert_eq!(r.title, *text, "{board}");
        assert_eq!(r.destination.page, *page, "{board} {text}");
        assert_eq!(
            r.availability,
            Availability::Disabled(why),
            "{board} {text}"
        );
        assert_eq!(r.value_text, *why, "{board} {text}");
    }
    // D18: About's "Check for updates" is the slot as read and no update service, whatever the slot
    let updates = |f: &Facts| {
        let snap = project_all(f);
        let hit = search(&snap, "Check for updates")
            .into_iter()
            .find(|h| h.key.id.0 == "about.updates")
            .expect("set2-about: Check for updates");
        snap.rows[hit.row].value_text.clone()
    };
    assert_eq!(updates(&facts()), "Slot a · Update service unavailable");
    let mut f = facts();
    f.scalars.remove("about.slot");
    assert_eq!(updates(&f), "Slot unavailable · Update service unavailable");
}

#[test]
fn a_result_title_splits_around_the_query() {
    use settings_ui::{Split, split_match};
    let s = |pre: &str, mark: &str, post: &str, gap_before, gap_after| Split {
        pre: pre.into(),
        mark: mark.into(),
        post: post.into(),
        gap_before,
        gap_after,
    };
    assert_eq!(
        split_match("Automatic time", "TIME"),
        s("Automatic", "time", "", true, false)
    );
    assert_eq!(
        split_match("Screen timeout", " time "),
        s("Screen", "time", "out", true, false)
    );
    assert_eq!(
        split_match("Date & time", "date"),
        s("", "Date", "& time", false, true)
    );
    assert_eq!(
        split_match("Automatic time zone", "time"),
        s("Automatic", "time", "zone", true, true)
    );
    // no match in the title (an alias found it): nothing highlighted
    assert_eq!(
        split_match("Wi-Fi", "wlan"),
        s("Wi-Fi", "", "", false, false)
    );
    assert_eq!(split_match("Wi-Fi", ""), s("Wi-Fi", "", "", false, false));
    // a partial match that fails later doesn't swallow the title
    assert_eq!(split_match("aab", "ab"), s("a", "ab", "", false, false));
    assert_eq!(split_match("Ünits", "ün"), s("", "Ün", "its", false, false));
    // any whitespace the trim removes is a gap
    assert_eq!(
        split_match("Wi\u{a0}Fi", "fi"),
        s("Wi", "Fi", "", true, false)
    );
}

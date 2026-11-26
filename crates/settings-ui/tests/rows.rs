// SPDX-License-Identifier: GPL-3.0-only
//! The search table (`neoui_proto::settings_app::rows::ROWS`) and the pages can't drift: every (page, row title) the
//! `.slint` pages draw, literal or from a model (`choices`), is in the table, and every table row is on its page.
//! Licences (not settings), sheets and the search results themselves are left out.
use neoui_proto::settings_app::choices::{KEYBOARDS, NAV_MODES};
use neoui_proto::settings_app::page::Page;
use neoui_proto::settings_app::rows::ROWS;
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
];

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
    let pages = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/pages");
    let device = std::fs::read_to_string(pages.join("device.slint")).unwrap();
    assert!(device.contains("for name[i] in SetState.nav-modes: ChoiceRow"));
    let personal = std::fs::read_to_string(pages.join("personal.slint")).unwrap();
    assert!(personal.contains("for name[i] in SetState.keyboard-names: ListRow"));
    for (name, _) in NAV_MODES {
        out.insert((Page::Navigation.name(), name.to_string()));
    }
    for (_, name, _) in KEYBOARDS {
        out.insert((Page::Keyboard.name(), name.to_string()));
    }
    let system = std::fs::read_to_string(pages.join("system.slint")).unwrap();
    assert!(system.contains("for a in SetState.apps: ListRow"));
    let device = std::fs::read_to_string(pages.join("device.slint")).unwrap();
    assert!(device.contains("for p in SetState.profiles: ListRow"));
    for p in neoui_proto::sound_profile::builtins() {
        out.insert((Page::Sounds.name(), p.name));
    }
    for a in neoui_proto::apps::listed() {
        out.insert((Page::Apps.name(), a.name.to_string()));
    }
    // Storage's search targets that aren't list rows: its by-kind section and its Recount button
    let storage = std::fs::read_to_string(pages.join("storage.slint")).unwrap();
    for (found, title) in [
        (
            "SectionHeader { text: \"C: and E: by kind\"; }",
            "C: and E: by kind",
        ),
        ("\"Counting…\" : \"Recount\"", "Recount"),
    ] {
        assert!(storage.contains(found), "storage.slint lost {title}");
        out.insert((Page::Storage.name(), title.to_string()));
    }
    out
}

#[test]
fn the_search_table_matches_the_pages() {
    let pages = page_rows();
    let table: BTreeSet<(&'static str, String)> = ROWS
        .iter()
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

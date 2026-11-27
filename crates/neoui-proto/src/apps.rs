// SPDX-License-Identifier: Apache-2.0
//! The apps in the ROM: what the shell starts for each screen key (its EXE and command line), what they are called,
//! and which have settings of their own (docs neoui-shell/settings.md SE1d). The shell's process table and the
//! Settings app's Apps page both come from here.
//!
//! shortcut: a fixed table, like [`crate::open`]; the store's app registry (docs store.md ST) replaces it with what
//! installed apps declare.

/// One app: its screen key, EXE (without `.exe`), command line, name, and where its own settings are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct App {
    pub key: &'static str,
    pub exe: &'static str,
    pub args: &'static str,
    pub name: &'static str,
    /// `OPEN(key, page, arg)` of the app's own settings page, if it has one.
    pub settings: Option<(&'static str, u16, &'static str)>,
    /// 1.19: a service, an app with no screen (DTSRV): started at boot and kept alive, never the screen, never
    /// listed; its card taps arrive in place and its `OPEN` flags count (docs neoui-shell/protocol.md "Services").
    pub service: bool,
}

const fn app(key: &'static str, exe: &'static str, name: &'static str) -> App {
    App {
        key,
        exe,
        args: "",
        name,
        settings: None,
        service: false,
    }
}

/// The Settings app's key (it doesn't list itself) and the old apps' bridge (listed as its old apps instead).
pub const SETTINGS: &str = "settings";
pub const LEGACY: &str = "legacy";
/// Classic apps (CLASSIC.EXE, NeoSymbian/legacy classic/): the list of the bridge's old apps, in the Apps menu.
pub const CLASSIC: &str = "classic";

pub const APPS: &[App] = &[
    // the alarms, timers, events and to-dos, rung whether or not Clock or Calendar runs (NeoSymbian/datetime)
    App {
        service: true,
        ..app("dtsrv", "dtsrv", "Clock & Calendar")
    },
    // Clock and Calendar (NeoSymbian/datetime)
    app("clock", "neoclock", "Clock"),
    app("calendar", "neocal", "Calendar"),
    // the Terminal (NeoSymbian/neoterm)
    app("terminal", "neoterm", "Terminal"),
    app("files", "neofiles", "Files"),
    App {
        settings: Some(("devtools", 0, "settings")),
        ..app("devtools", "neodev", "Dev tools")
    },
    // the kit gallery and the template for new apps (symbian/apps/kit)
    app("kit", "neokit", "UI kit"),
    // S60 apps through the guest window server (NeoSymbian/legacy host/legacy.cpp): not in the Apps menu, Classic
    // apps lists them (NeoSymbian/legacy classic/)
    app(LEGACY, "legacy", "Classic apps"),
    app(CLASSIC, "classic", "Classic apps"),
    // Phone, Contacts and Messages (NeoSymbian/phone): three apps that open each other (ipc OPEN); Phone's page 3 is
    // the three apps' settings
    App {
        settings: Some(("phone", 3, "")),
        ..app("phone", "neophone", "Phone")
    },
    App {
        settings: Some(("phone", 3, "contacts")),
        ..app("contacts", "neocont", "Contacts")
    },
    App {
        settings: Some(("phone", 3, "messages")),
        ..app("messages", "neomsg", "Messages")
    },
    // the browser (NeoSymbian/browser, docs browser/ui.md): its own chrome over a page engine
    app("browser", "neoweb", "Browser"),
    // Settings (NEOSET, docs neoui-shell/settings.md "The Settings app"): writes shell settings by its secure ID
    app(SETTINGS, "neoset", "Settings"),
];

/// The apps the Settings app's Apps page lists: all but itself, the services and the old apps' bridge (its apps are
/// listed instead).
pub fn listed() -> impl Iterator<Item = &'static App> {
    APPS.iter()
        .filter(|a| a.key != SETTINGS && a.key != LEGACY && !a.service)
}

/// Is `key` a service (1.19, [`App::service`])?
#[must_use]
pub fn is_service(key: &str) -> bool {
    find(key).is_some_and(|a| a.service)
}

/// The apps that clear their own data on `ipc::AppEvent::ClearData` (Settings → Apps → an app → Clear data, 1.17).
/// Phone's call history and Messages' SMS live in MODEMD, which Settings asks itself; the others get the row when they
/// handle the event.
#[must_use]
pub fn clears_data(key: &str) -> bool {
    matches!(key, "clock" | "calendar")
}

/// D4: the apps of this image that answer owner work (protocol 1.20 `OWNER_REPORT`), so the shell may start one
/// hidden for it, and the `clear_scope` each offers once it registered with the clear capability
/// (`settings_app::wire::scope`). Any other app is measured by the shell itself (its EXE and private folder) and is
/// never started for a size: a 1.19 app's normal startup (Files' index, Contacts' migration) would change what it
/// measures. Clock, Calendar, Files, Browser, Phone and Messages join when their own repos land their owners.
#[must_use]
pub fn owner_scope(key: &str) -> Option<u32> {
    use crate::settings_app::wire::scope;
    match key {
        // its tracked log exports (symbian/apps/devtools/src/app_data.rs)
        "devtools" => Some(scope::LOGS),
        // nothing kept: the screen and the ESHELL session go with the process
        "terminal" => Some(0),
        _ => None,
    }
}

/// The secure ID of app `key`'s EXE (its UID3 in the `exes.txt` that builds it), or `None` for one the shell doesn't
/// know by SID (the old apps' bridge). Since 1.18 the shell takes cards and badges (`ipc::NOTIFY`,
/// `ipc::NOTIFY_CLEAR`, `ipc::BADGE`) for an app only from this process (top-panel TP2a "Step 0").
#[must_use]
pub fn sid(key: &str) -> Option<u32> {
    Some(match key {
        "clock" => 0xE000_8001,
        "calendar" => 0xE000_8002,
        "dtsrv" => 0xE000_8003,
        "terminal" => 0xE000_2003,
        "devtools" => 0xE000_2005,
        "kit" => 0xE000_2006,
        CLASSIC => 0xE000_2008,
        SETTINGS => crate::settings::access::SETTINGS_SID,
        "phone" => 0xE000_5001,
        "contacts" => 0xE000_5002,
        "messages" => 0xE000_5003,
        "files" => 0xE000_6001,
        "browser" => 0xE000_7001,
        _ => return None,
    })
}

/// The legacy bridge's secure ID: LEGACY.EXE's UID3, which the kit's EXE shares (docs neoui-shell/top-panel.md).
pub const LEGACY_SID: u32 = 0xE000_2006;

/// Can a process whose kernel secure ID is `sid` be app `key`'s? Its EXE's SID ([`sid`]), or the shared one for the
/// legacy bridge. The shell binds a new connection to a started app only when it fits and its process is the one the
/// shell started (the SID alone can't tell the kit from the bridge).
#[must_use]
pub fn sid_fits(key: &str, sid: u32) -> bool {
    match self::sid(key) {
        Some(s) => s == sid,
        None => key == LEGACY && sid == LEGACY_SID,
    }
}

/// The app with screen key `key`.
#[must_use]
pub fn find(key: &str) -> Option<&'static App> {
    APPS.iter().find(|a| a.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_fits_only_its_own_exe() {
        assert!(sid_fits("files", 0xE000_6001));
        assert!(!sid_fits("files", 0xE000_7001), "the browser isn't Files");
        assert!(!sid_fits("files", LEGACY_SID));
        assert!(sid_fits(LEGACY, LEGACY_SID));
        assert!(!sid_fits(LEGACY, 0xE000_6001));
        assert_eq!(
            sid("kit"),
            Some(LEGACY_SID),
            "the shared SID the kit and the bridge can't be told apart by"
        );
        assert!(!sid_fits("nosuchapp", 0));
    }

    #[test]
    fn every_app_but_the_bridge_has_its_sid() {
        for a in APPS {
            assert_eq!(sid(a.key).is_some(), a.key != LEGACY, "{}", a.key);
        }
        let mut sids: alloc::vec::Vec<u32> = APPS.iter().filter_map(|a| sid(a.key)).collect();
        sids.sort_unstable();
        sids.dedup();
        assert_eq!(sids.len(), APPS.len() - 1, "one SID per app");
    }

    #[test]
    fn keys_are_unique_and_short() {
        for (i, a) in APPS.iter().enumerate() {
            assert!(a.key.len() <= crate::ipc::KEY_MAX, "{}", a.key);
            assert!(APPS[i + 1..].iter().all(|b| b.key != a.key), "{}", a.key);
            if let Some((k, _, _)) = a.settings {
                assert!(find(k).is_some(), "{} settings in {k}", a.key);
            }
        }
    }

    #[test]
    fn services_are_not_listed() {
        assert!(is_service("dtsrv"));
        assert!(!is_service("clock") && !is_service("nope"));
        assert!(listed().all(|a| !a.service));
        assert!(listed().any(|a| a.key == "calendar"));
        assert!(clears_data("calendar") && !clears_data("dtsrv"));
    }

    #[test]
    fn owners_are_listed_apps_with_their_own_sid() {
        for a in APPS.iter().filter(|a| owner_scope(a.key).is_some()) {
            assert!(listed().any(|l| l.key == a.key), "{}", a.key);
            assert!(a.key != "kit", "the kit shares the bridge's SID");
        }
        assert_eq!(
            owner_scope("files"),
            None,
            "a 1.19 app is never started for a size"
        );
    }

    /// Every app the shell's Apps menu shows that is a process here has the same name there (ui/screens/menu.slint);
    /// a service isn't in the menu, nor the bridge (Classic apps opens its apps).
    #[test]
    fn the_apps_menu_names_match() {
        extern crate std;
        let menu = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../ui/screens/menu.slint"
        ))
        .unwrap();
        let mut seen = 0;
        for line in menu.lines() {
            let (Some(n), Some(k)) = (line.find("name: \""), line.find("key: \"")) else {
                continue;
            };
            let name = line[n + 7..].split('"').next().unwrap();
            let key = line[k + 6..].split('"').next().unwrap();
            if let Some(a) = find(key) {
                assert_ne!(key, LEGACY, "the bridge isn't in the Apps menu");
                assert_eq!(a.name, name, "{key}");
                seen += 1;
            }
        }
        assert_eq!(
            seen,
            APPS.iter()
                .filter(|a| !a.service && a.key != LEGACY)
                .count(),
            "an app here is missing from the Apps menu"
        );
    }
}

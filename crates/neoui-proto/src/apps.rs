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
}

const fn app(key: &'static str, exe: &'static str, name: &'static str) -> App {
    App {
        key,
        exe,
        args: "",
        name,
        settings: None,
    }
}

/// The Settings app's key (it doesn't list itself) and the old apps' bridge (listed as its old apps instead).
pub const SETTINGS: &str = "settings";
pub const LEGACY: &str = "legacy";

pub const APPS: &[App] = &[
    app("clock", "neoclock", "Clock"),
    app("terminal", "neoterm", "Terminal"),
    app("files", "neofiles", "Files"),
    App {
        settings: Some(("devtools", 0, "settings")),
        ..app("devtools", "neodev", "Dev tools")
    },
    // the kit gallery and the template for new apps (symbian/apps/kit)
    app("kit", "neokit", "UI kit"),
    // S60 apps through the guest window server (NeoSymbian/legacy host/legacy.cpp)
    app(LEGACY, "legacy", "Old apps"),
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

/// The apps the Settings app's Apps page lists: all but itself and the old apps' bridge (its apps are listed instead).
pub fn listed() -> impl Iterator<Item = &'static App> {
    APPS.iter().filter(|a| a.key != SETTINGS && a.key != LEGACY)
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
    fn keys_are_unique_and_short() {
        for (i, a) in APPS.iter().enumerate() {
            assert!(a.key.len() <= crate::ipc::KEY_MAX, "{}", a.key);
            assert!(APPS[i + 1..].iter().all(|b| b.key != a.key), "{}", a.key);
            if let Some((k, _, _)) = a.settings {
                assert!(find(k).is_some(), "{} settings in {k}", a.key);
            }
        }
    }

    /// Every app the shell's Apps menu shows that is a process here has the same name there (ui/screens/menu.slint).
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
                assert_eq!(a.name, name, "{key}");
                seen += 1;
            }
        }
        assert_eq!(
            seen,
            APPS.len(),
            "an app here is missing from the Apps menu"
        );
    }
}

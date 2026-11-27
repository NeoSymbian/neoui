// SPDX-License-Identifier: Apache-2.0
//! Every row the Settings app's pages draw, for search (`set2-search`): a stable id, the page it is on, the page a
//! tap on it opens, its title as the page shows it, and other words people search for. settings-ui's tests fail when
//! a title here is missing from the `.slint` pages, or a page's row is missing here, so the two can't drift.

use super::page::Page;

/// Where a row's value comes from (`row_model::Provider`): one provider per family, shared by the page, search and
/// the root's summary. A later stream adds its variant, its provider module and its rows here (row_model's contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderId {
    /// a setting, a choice or a reading the device already formats (and the root summaries made of them)
    Builtin,
    /// a row that opens a page or acts: found by name only
    Action,
    /// one row per sound profile (its summary)
    Profiles,
    /// a profile page's rows, one per profile that shows them
    ProfileFields,
    /// Storage's drives, one per mounted letter
    Drives,
    /// Storage's "C: and E: by kind" kinds
    OverviewKinds,
    /// the ROM apps and the old apps
    Apps,
    /// the Wi-Fi link's detail fields, while there is a link
    WifiFields,
    /// (TP4) Notifications' apps, one per policy owner (INFO 7), with its summary
    NotifyApps,
    /// (TP4) every app's declared channels (INFO 7), with its effective importance
    Channels,
    /// (D1) the networks Wi-Fi sees now, one per access point
    WifiAps,
    /// (D1) the networks WIFID remembers, one per SSID
    SavedNets,
}

/// What search may match a row by. Credentials, PINs, clipboard contents, message bodies and draft text are never
/// rows' values at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Search {
    /// title, other names, page and the value as shown
    PublicValue,
    /// title, other names and page only (an action, or a value that isn't public)
    TitleOnly,
    /// not searchable
    Omit,
}

/// One row definition (D1's `RowSpec`): a family row (`provider` other than Builtin/Action) stands for every instance
/// its provider enumerates, and its `title` is "" (each instance has its own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// Stable, unique, never derived from the title.
    pub id: &'static str,
    /// The page the row is on.
    pub on: Page,
    /// The page a search result opens (the row's own page, or for a root row the page it leads to).
    pub opens: Page,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
    pub provider: ProviderId,
    pub search: Search,
}

pub type RowSpec = Row;

const fn row(
    id: &'static str,
    on: Page,
    opens: Page,
    title: &'static str,
    aliases: &'static [&'static str],
) -> Row {
    Row {
        id,
        on,
        opens,
        title,
        aliases,
        provider: ProviderId::Builtin,
        search: Search::PublicValue,
    }
}

/// A row with no value: it opens or acts.
const fn action(
    id: &'static str,
    on: Page,
    opens: Page,
    title: &'static str,
    aliases: &'static [&'static str],
) -> Row {
    Row {
        provider: ProviderId::Action,
        search: Search::TitleOnly,
        ..row(id, on, opens, title, aliases)
    }
}

/// A family: one row per instance, titled by its provider.
const fn family(
    id: &'static str,
    on: Page,
    opens: Page,
    provider: ProviderId,
    aliases: &'static [&'static str],
) -> Row {
    Row {
        provider,
        ..row(id, on, opens, "", aliases)
    }
}

/// A profile page's row, once per profile that shows it.
const fn field(id: &'static str, title: &'static str, aliases: &'static [&'static str]) -> Row {
    Row {
        provider: ProviderId::ProfileFields,
        ..row(id, Page::Profile, Page::Profile, title, aliases)
    }
}

use Page::*;

pub const ROWS: &[Row] = &[
    // the root (set2-root-list)
    row(
        "root.wifi",
        Root,
        Wifi,
        "Wi-Fi",
        &["wlan", "wireless", "internet"],
    ),
    row(
        "root.mobile",
        Root,
        Mobile,
        "Mobile network",
        &["cellular", "sim", "lte", "3g", "operator"],
    ),
    row("root.bt", Root, Bluetooth, "Bluetooth", &[]),
    row(
        "root.airplane",
        Root,
        Connectivity,
        "Airplane mode",
        &["flight", "offline", "radio"],
    ),
    row("root.display", Root, Display, "Display", &["screen"]),
    row(
        "root.navigation",
        Root,
        Navigation,
        "Home & navigation",
        &["gestures", "buttons", "n9"],
    ),
    row(
        "root.sounds",
        Root,
        Sounds,
        "Sounds & profiles",
        &["volume", "ringtone", "silent"],
    ),
    row(
        "root.battery",
        Root,
        Battery,
        "Battery",
        &["power", "charge"],
    ),
    row(
        "root.storage",
        Root,
        Storage,
        "Storage",
        &["space", "memory", "disk", "free"],
    ),
    // D15: the lock screen and its PIN have no page yet (set2-lock); the root says so
    action(
        "root.lock",
        Root,
        Root,
        "Lock screen & security",
        &["security", "pin", "lock"],
    ),
    row(
        "root.datetime",
        Root,
        DateTime,
        "Date & time",
        &["clock", "time"],
    ),
    row(
        "root.keyboard",
        Root,
        Keyboard,
        "Language & keyboard",
        &["input", "typing"],
    ),
    action("root.privacy", Root, Privacy, "Privacy", &[]),
    row("root.apps", Root, Apps, "Apps", &["applications"]),
    row(
        "root.developer",
        Root,
        Developer,
        "Developer options",
        &["debug", "usb"],
    ),
    action(
        "root.about",
        Root,
        About,
        "About phone",
        &["version", "build", "model"],
    ),
    // Wi-Fi
    row("wifi.switch", Wifi, Wifi, "Wi-Fi", &["networks"]),
    row(
        "wifi.saved",
        Wifi,
        WifiSaved,
        "Saved networks",
        &["forget", "remembered", "known networks"],
    ),
    // the current link's details sheet (set2-wifi-details), one row per field it shows
    family(
        "wifi.field",
        Wifi,
        Wifi,
        ProviderId::WifiFields,
        &["link", "network details"],
    ),
    // (D1) every network in range (set2-wifi's list) and every saved one (set2-wifi-saved): each its own row
    family(
        "wifi.ap",
        Wifi,
        Wifi,
        ProviderId::WifiAps,
        &["network", "ssid", "access point"],
    ),
    family(
        "wifi.saved_net",
        WifiSaved,
        WifiSaved,
        ProviderId::SavedNets,
        &["saved network", "known network"],
    ),
    // D12: a hidden network needs a directed scan the driver doesn't do yet (set2-wifi-hidden)
    action(
        "wifi.hidden",
        Wifi,
        Wifi,
        "Add hidden network",
        &["hidden ssid", "hidden network", "add network"],
    ),
    // Connectivity
    row("conn.wifi", Connectivity, Connectivity, "Wi-Fi", &[]),
    row("conn.bt", Connectivity, Connectivity, "Bluetooth", &[]),
    row("conn.data", Connectivity, Connectivity, "Mobile data", &[]),
    row(
        "conn.airplane",
        Connectivity,
        Connectivity,
        "Airplane mode",
        &["flight"],
    ),
    // Mobile network
    row(
        "mobile.sim",
        Mobile,
        Mobile,
        "SIM card & operator",
        &["operator", "carrier"],
    ),
    row("mobile.data", Mobile, Mobile, "Mobile data", &["internet"]),
    // D13: they need the data bearer (C10)
    action(
        "mobile.mode",
        Mobile,
        Mobile,
        "Network mode",
        &["lte", "3g", "2g", "4g", "preferred network"],
    ),
    action(
        "mobile.roaming",
        Mobile,
        Mobile,
        "Roaming",
        &["data roaming", "abroad"],
    ),
    action(
        "mobile.apn",
        Mobile,
        Mobile,
        "Access point names",
        &["apn", "internet settings"],
    ),
    action(
        "mobile.usage",
        Mobile,
        Mobile,
        "Data usage",
        &["this month", "traffic", "megabytes"],
    ),
    // D11: SIM card lock (set2-mobile's row, set2-sim-lock); the PIN itself is never a value
    row(
        "mobile.sim_lock",
        Mobile,
        SimLock,
        "SIM card lock",
        &["sim pin", "pin lock"],
    ),
    row(
        "sim.pin",
        SimLock,
        SimLock,
        "Use SIM PIN",
        &["sim pin", "pin lock"],
    ),
    action(
        "sim.change",
        SimLock,
        SimLock,
        "Change SIM PIN",
        &["pin code", "new pin"],
    ),
    row(
        "sim.attempts",
        SimLock,
        SimLock,
        "PIN attempts left",
        &["tries", "puk"],
    ),
    // Bluetooth
    row("bt.switch", Bluetooth, Bluetooth, "Bluetooth", &[]),
    // D14: no driver yet
    action(
        "bt.visible",
        Bluetooth,
        Bluetooth,
        "Visible to nearby devices",
        &["discoverable", "visibility"],
    ),
    action(
        "bt.paired",
        Bluetooth,
        Bluetooth,
        "Paired devices",
        &["pair", "headphones", "devices"],
    ),
    // Display
    row(
        "display.dark",
        Display,
        Display,
        "Dark theme",
        &["night", "theme"],
    ),
    row(
        "display.timeout",
        Display,
        Display,
        "Screen timeout",
        &["sleep", "lock"],
    ),
    // Home & navigation: the three modes (choices::NAV_MODES, drawn from a model)
    row(
        "nav.0",
        Navigation,
        Navigation,
        "Buttons",
        &["keys", "navigation"],
    ),
    row(
        "nav.1",
        Navigation,
        Navigation,
        "Gestures",
        &["thumb", "navigation"],
    ),
    row(
        "nav.2",
        Navigation,
        Navigation,
        "N9 gestures",
        &["navigation"],
    ),
    // Sounds
    // the profiles, one row each (row_model::Profiles: built-in and added, by id; `sounds.p<id>` is the old id)
    family(
        "sounds.profile",
        Sounds,
        Sounds,
        ProviderId::Profiles,
        &["profile"],
    ),
    action(
        "sounds.add",
        Sounds,
        Sounds,
        "Add profile",
        &["new profile"],
    ),
    // a profile's page: one row per profile that shows it (row_model::ProfileFields)
    field("profile.tone", "Ringtone", &["ring tone"]),
    field("profile.ring", "Ring", &["silent", "ringer"]),
    field(
        "profile.volume",
        "Ring volume",
        &["ringer volume", "loudness"],
    ),
    field("profile.vibrate", "Vibrate", &["vibration", "buzz"]),
    field(
        "profile.message",
        "Message tone",
        &["text tone", "notification sound"],
    ),
    // D6: the profile's touch feedback
    field(
        "profile.keypad",
        "Keypad tones",
        &["dial tones", "key sounds", "dtmf"],
    ),
    field(
        "profile.touch",
        "Vibrate on touch",
        &["haptic feedback", "touch vibration"],
    ),
    row(
        "sounds.media",
        Sounds,
        Sounds,
        "Media volume",
        &["music", "loud"],
    ),
    row(
        "sounds.route",
        Sounds,
        Sounds,
        "Play media on",
        &["speaker", "earpiece"],
    ),
    row("sounds.mic", Sounds, Sounds, "Microphone", &["record"]),
    // Date & time
    row(
        "time.auto",
        DateTime,
        DateTime,
        "Automatic time",
        &["network time", "nitz"],
    ),
    row(
        "time.auto_zone",
        DateTime,
        DateTime,
        "Automatic time zone",
        &[],
    ),
    row(
        "time.zone",
        DateTime,
        DateTime,
        "Time zone",
        &["city", "utc"],
    ),
    row("time.24h", DateTime, DateTime, "24-hour time", &["clock"]),
    row(
        "time.seconds",
        DateTime,
        DateTime,
        "Seconds in the status bar",
        &[],
    ),
    row(
        "time.shade_date",
        DateTime,
        DateTime,
        "Date in the shade",
        &[],
    ),
    row("time.format", DateTime, DateTime, "Date format", &[]),
    // Language & keyboard
    // the keyboard's languages (choices::KEYBOARDS, drawn from a model)
    row(
        "kb.0",
        Keyboard,
        Keyboard,
        "English",
        &["qwerty", "keyboard"],
    ),
    row(
        "kb.1",
        Keyboard,
        Keyboard,
        "Armenian",
        &["hayeren", "հայերեն", "keyboard"],
    ),
    row(
        "kb.2",
        Keyboard,
        Keyboard,
        "Russian",
        &["cyrillic", "русский", "keyboard"],
    ),
    row(
        "kb.3",
        Keyboard,
        Keyboard,
        "Western Armenian",
        &["arevmtahayeren", "արեւմտահայերէն", "keyboard"],
    ),
    row(
        "kb.current",
        Keyboard,
        Keyboard,
        "Current language",
        &["english", "armenian", "russian"],
    ),
    row(
        "kb.same_height",
        Keyboard,
        Keyboard,
        "Same height for all languages",
        &[],
    ),
    row(
        "kb.vibrate",
        Keyboard,
        Keyboard,
        "Keypress vibration",
        &["haptic", "vibrate", "keyboard"],
    ),
    row(
        "kb.suggest",
        Keyboard,
        Keyboard,
        "Word suggestions",
        &["predict", "autocomplete", "keyboard"],
    ),
    // Privacy
    action(
        "privacy.clip",
        Privacy,
        Privacy,
        "Clear clipboard",
        &["copy", "paste"],
    ),
    // Apps
    // every app, the ROM's and the old apps the legacy bridge registered (row_model::Apps; `apps.<key>` is the old id)
    Row {
        search: Search::TitleOnly,
        ..family("apps.app", Apps, AppDetail, ProviderId::Apps, &["app"])
    },
    action(
        "apps.oldapps",
        Apps,
        OldApps,
        "Key pads for classic apps",
        &["s60", "classic apps", "old apps", "legacy"],
    ),
    // Old apps' key pads
    row("pads.vibrate", OldApps, OldApps, "Key vibration", &[]),
    row("pads.strength", OldApps, OldApps, "Vibration strength", &[]),
    row(
        "pads.default",
        OldApps,
        OldApps,
        "Default pad for new apps",
        &["gamepad", "navigator"],
    ),
    row("pads.dirs", OldApps, OldApps, "Gamepad directions", &[]),
    row("pads.a", OldApps, OldApps, "Gamepad A button", &[]),
    row("pads.b", OldApps, OldApps, "Gamepad B button", &[]),
    row("pads.lefty", OldApps, OldApps, "Left-handed gamepad", &[]),
    row(
        "pads.console",
        OldApps,
        OldApps,
        "Open games in the console",
        &["landscape"],
    ),
    // Battery (SE3a)
    row(
        "battery.saver",
        Battery,
        Battery,
        "Battery saver",
        &["dim", "power saving", "brightness"],
    ),
    row(
        "battery.percent",
        Battery,
        Battery,
        "Show percentage",
        &["status bar", "percent"],
    ),
    action(
        "battery.usage",
        Battery,
        BatteryUsage,
        "Battery usage",
        &["since full charge", "power"],
    ),
    // Storage (SE3b)
    row(
        "storage.kinds",
        Storage,
        Storage,
        "C: and E: by kind",
        &["images", "video", "audio", "documents", "apps"],
    ),
    action(
        "storage.files",
        Storage,
        Storage,
        "Open Files",
        &["file manager"],
    ),
    action(
        "storage.usb",
        Storage,
        Storage,
        "Share E: over USB",
        &["mass storage", "usb"],
    ),
    // D1: each drive, and C: and E: together by kind
    family(
        "storage.drive",
        Storage,
        Storage,
        ProviderId::Drives,
        &["drive"],
    ),
    family(
        "storage.kind",
        Storage,
        Storage,
        ProviderId::OverviewKinds,
        &["by kind"],
    ),
    // Developer options
    row(
        "dev.usb",
        Developer,
        Developer,
        "USB debugging",
        &["adb", "shell"],
    ),
    action(
        "dev.log",
        Developer,
        Developer,
        "Kernel log",
        &["console", "logs"],
    ),
    action("dev.save", Developer, Developer, "Save logs", &["logs"]),
    // D2
    row(
        "dev.console",
        Developer,
        Developer,
        "Console overlay",
        &["kernel console", "overlay", "screen log"],
    ),
    row(
        "dev.touches",
        Developer,
        Developer,
        "Show touches",
        &["pointer", "finger", "tap"],
    ),
    action(
        "dev.tools",
        Developer,
        Developer,
        "Open Dev tools",
        &["devices", "processes"],
    ),
    // About phone
    action(
        "about.licences",
        About,
        Licences,
        "Licences",
        &["licenses", "open source", "gpl", "apache"],
    ),
    action(
        "about.reset",
        About,
        Reset,
        "Reset",
        &["factory", "defaults", "restore", "erase"],
    ),
    // D2: the booted A/B slot, when the kernel read it
    row(
        "about.slot",
        About,
        About,
        "Active slot",
        &["slot", "a/b", "boot slot"],
    ),
    // D18: the slot and that no update service is configured (it never checks)
    row(
        "about.updates",
        About,
        About,
        "Check for updates",
        &["update", "software update", "upgrade", "ota"],
    ),
    // Reset (SE4a)
    action(
        "reset.network",
        Reset,
        Reset,
        "Reset network settings",
        &["forget networks", "wifi", "wlan"],
    ),
    action(
        "reset.settings",
        Reset,
        Reset,
        "Reset settings",
        &["defaults", "factory"],
    ),
    // D19: erasing C: needs the recovery step
    action(
        "reset.erase",
        Reset,
        Reset,
        "Erase C:",
        &["factory reset", "wipe", "erase"],
    ),
    // D7: Display's text size and wallpaper (their pages), Language & keyboard's display language
    row(
        "display.font_size",
        Display,
        FontSize,
        "Font size",
        &["text size", "bigger text", "smaller text", "zoom"],
    ),
    row(
        "display.wallpaper",
        Display,
        Wallpaper,
        "Wallpaper",
        &["background", "home screen", "colour", "color"],
    ),
    // (D7) a picture from Files (set2-wallpaper's Pictures): an action, it opens Files' picker only when tapped
    action(
        "display.wallpaper_files",
        Wallpaper,
        Wallpaper,
        "From Files",
        &["picture", "photo", "image", "wallpaper"],
    ),
    action(
        "display.wallpaper_photos",
        Wallpaper,
        Wallpaper,
        "From Photos",
        &["picture", "gallery"],
    ),
    row(
        "kb.display_language",
        Keyboard,
        Keyboard,
        "Display language",
        &["language", "translation", "menus"],
    ),
    // D8: the clipboard toast, the local diagnostics (Dev tools' log)
    // TP4 / D9 (notifications.rs): Panel & notifications (tp-settings), Notifications (set2-notifications), one app
    // (set2-notify-app, nt-settings); an app's own switches go by its page only (they're about the app on it)
    row(
        "root.notifications",
        Root,
        Panel,
        "Notifications",
        &["alerts", "do not disturb", "banners", "shade"],
    ),
    // TP7: the control centre's editor (the shell's), what the status bar's tap opens
    action(
        "panel.tiles",
        Panel,
        Panel,
        "Tiles",
        &["control centre", "quick settings", "edit tiles"],
    ),
    row(
        "panel.tap",
        Panel,
        Panel,
        "Tap on the status bar",
        &["control centre", "status bar", "shade"],
    ),
    row(
        "panel.swipehome",
        Panel,
        Panel,
        "Swipe down on home opens the shade",
        &["shade", "gesture", "pull down"],
    ),
    action(
        "panel.apps",
        Panel,
        Notifications,
        "App notifications",
        &["per app", "channels"],
    ),
    row(
        "panel.dnd",
        Panel,
        Panel,
        "Do not disturb",
        &["dnd", "quiet", "silence"],
    ),
    row(
        "notify.bannert",
        Panel,
        Panel,
        "Pop-up time",
        &["banner", "heads-up", "seconds"],
    ),
    // D15: no lock screen yet
    action(
        "notify.lock",
        Notifications,
        Notifications,
        "Show on lock screen",
        &["lock screen", "hide content", "private"],
    ),
    row(
        "notify.dnd",
        Notifications,
        Notifications,
        "Do not disturb",
        &["dnd", "quiet", "silence"],
    ),
    family(
        "notify.app",
        Notifications,
        NotifyApp,
        ProviderId::NotifyApps,
        &["notifications"],
    ),
    Row {
        search: Search::Omit,
        ..action(
            "notify.allow",
            NotifyApp,
            NotifyApp,
            "Allow notifications",
            &[],
        )
    },
    Row {
        search: Search::Omit,
        ..action("notify.banners", NotifyApp, NotifyApp, "Banners", &[])
    },
    Row {
        search: Search::Omit,
        ..action("notify.sound", NotifyApp, NotifyApp, "Sound", &[])
    },
    Row {
        search: Search::Omit,
        ..action(
            "notify.badge",
            NotifyApp,
            NotifyApp,
            "Badge on the icon",
            &[],
        )
    },
    family(
        "notify.channel",
        NotifyApp,
        NotifyApp,
        ProviderId::Channels,
        &["channel", "importance"],
    ),
    row(
        "privacy.clip_notice",
        Privacy,
        Privacy,
        "Show when an app reads it",
        &["clipboard", "paste", "toast"],
    ),
    // (SE4/D8) the capabilities view: each app's page
    action(
        "privacy.permissions",
        Privacy,
        Apps,
        "App permissions",
        &["capabilities", "permissions", "platsec"],
    ),
    action(
        "privacy.diagnostics",
        Privacy,
        Privacy,
        "Usage & diagnostics",
        &["log", "telemetry", "crash"],
    ), // D10: Home & navigation's gesture rows and the home editor (SETTINGS_ACTION 9)
    row(
        "nav.swipe_close",
        Navigation,
        Navigation,
        "Swipe down closes apps",
        &["close", "top edge", "status bar", "gesture"],
    ),
    row(
        "nav.switcher_cols",
        Navigation,
        Navigation,
        "Open apps columns",
        &["switcher", "recent apps", "multitasking", "grid"],
    ),
    row(
        "home.style",
        Navigation,
        Navigation,
        "Home style",
        &["lanes", "pages", "home screen", "n9"],
    ),
    action(
        "home.edit",
        Navigation,
        Navigation,
        "Edit home",
        &["widgets", "dock", "shortcuts", "home screen"],
    ),
    // D5: the charge cycle's screen-on time (INFO 9)
    row(
        "battery.screen_on",
        BatteryUsage,
        BatteryUsage,
        "Screen-on time",
        &["screen time", "since charged", "display on", "full charge"],
    ),
];

/// The root list's rows, top to bottom (set2-root-list, root.slint's `RootPage`): row `i` shows
/// `SetState.summaries[i]`, the projection of `ROOT_LIST[i]` (tests/rows.rs checks the page against this).
pub const ROOT_LIST: [&str; 17] = [
    "root.wifi",
    "root.mobile",
    "root.bt",
    "root.airplane",
    "root.display",
    "root.navigation",
    "root.sounds",
    "root.notifications",
    "root.battery",
    "root.storage",
    "root.lock",
    "root.datetime",
    "root.keyboard",
    "root.privacy",
    "root.apps",
    "root.developer",
    "root.about",
];

/// The root grid's tiles in order (set2-root-grid): label, the page it opens and the root row whose summary it reads
/// out (accessibility only; the tiles stay label-only). Security waits for its page.
pub const ROOT_GRID: [(&str, Page, Option<&str>); 13] = [
    ("Connectivity", Connectivity, None),
    ("Display", Display, Some("root.display")),
    ("Sounds", Sounds, Some("root.sounds")),
    ("Navigation", Navigation, Some("root.navigation")),
    ("Notifications", Panel, Some("root.notifications")),
    ("Battery", Battery, Some("root.battery")),
    ("Storage", Storage, Some("root.storage")),
    ("Privacy", Privacy, None),
    ("Time", DateTime, Some("root.datetime")),
    ("Language", Keyboard, Some("root.keyboard")),
    ("Apps", Apps, Some("root.apps")),
    ("Developer", Developer, Some("root.developer")),
    ("About", About, None),
];

/// The definition with id `id`.
#[must_use]
pub fn find(id: &str) -> Option<&'static Row> {
    ROWS.iter().find(|r| r.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        for (i, a) in ROWS.iter().enumerate() {
            assert!(ROWS[i + 1..].iter().all(|b| b.id != a.id), "{}", a.id);
        }
    }

    #[test]
    fn families_have_no_title_and_rows_have_one() {
        for r in ROWS {
            let fixed = matches!(
                r.provider,
                ProviderId::Builtin | ProviderId::Action | ProviderId::ProfileFields
            );
            assert_eq!(r.title.is_empty(), !fixed, "{}", r.id);
            // an action has no value to find; a value row is found by it (the apps have none)
            let value = !matches!(r.provider, ProviderId::Action | ProviderId::Apps);
            assert_eq!(value, r.search == Search::PublicValue, "{}", r.id);
        }
    }

    #[test]
    fn placements_name_root_rows() {
        for id in ROOT_LIST {
            assert_eq!(find(id).map(|r| r.on), Some(Page::Root), "{id}");
        }
        for (label, page, summary) in ROOT_GRID {
            if let Some(id) = summary {
                assert_eq!(find(id).map(|r| r.opens), Some(page), "{label}");
            }
        }
    }
}

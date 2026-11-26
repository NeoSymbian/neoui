// SPDX-License-Identifier: Apache-2.0
//! Every row the Settings app's pages draw, for search (`set2-search`): a stable id, the page it is on, the page a
//! tap on it opens, its title as the page shows it, and other words people search for. settings-ui's tests fail when
//! a title here is missing from the `.slint` pages, or a page's row is missing here, so the two can't drift.

use super::page::Page;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// Stable, unique: the app shows a live value for some (`neoset` `search`).
    pub id: &'static str,
    /// The page the row is on.
    pub on: Page,
    /// The page a search result opens (the row's own page, or for a root row the page it leads to).
    pub opens: Page,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
}

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
    row("root.privacy", Root, Privacy, "Privacy", &[]),
    row("root.apps", Root, Apps, "Apps", &["applications"]),
    row(
        "root.developer",
        Root,
        Developer,
        "Developer options",
        &["debug", "usb"],
    ),
    row(
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
    // Bluetooth
    row("bt.switch", Bluetooth, Bluetooth, "Bluetooth", &[]),
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
    // the profiles (sound_profile::builtins; tests/rows.rs keeps them in step) and Add profile
    row(
        "sounds.p0",
        Sounds,
        Sounds,
        "General",
        &["profile", "normal"],
    ),
    row(
        "sounds.p1",
        Sounds,
        Sounds,
        "Silent",
        &["profile", "mute", "quiet"],
    ),
    row("sounds.p2", Sounds, Sounds, "Meeting", &["profile", "beep"]),
    row("sounds.p3", Sounds, Sounds, "Outdoor", &["profile", "loud"]),
    row(
        "sounds.p4",
        Sounds,
        Sounds,
        "Offline",
        &["profile", "airplane", "flight"],
    ),
    row(
        "sounds.add",
        Sounds,
        Sounds,
        "Add profile",
        &["new profile"],
    ),
    // a profile's page (the active one when search opens it)
    row("profile.tone", Profile, Profile, "Ringtone", &["ring tone"]),
    row(
        "profile.ring",
        Profile,
        Profile,
        "Ring",
        &["silent", "ringer"],
    ),
    row(
        "profile.volume",
        Profile,
        Profile,
        "Ring volume",
        &["ringer volume", "loudness"],
    ),
    row(
        "profile.vibrate",
        Profile,
        Profile,
        "Vibrate",
        &["vibration", "buzz"],
    ),
    row(
        "profile.message",
        Profile,
        Profile,
        "Message tone",
        &["text tone", "notification sound"],
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
    // Privacy
    row(
        "privacy.clip",
        Privacy,
        Privacy,
        "Clear clipboard",
        &["copy", "paste"],
    ),
    // Apps
    // the ROM apps (crate::apps, the ones Settings lists; tests/rows.rs keeps them in step)
    row("apps.clock", Apps, Apps, "Clock", &["alarms", "timer"]),
    row("apps.calendar", Apps, Apps, "Calendar", &["events"]),
    row("apps.terminal", Apps, Apps, "Terminal", &["shell"]),
    row("apps.files", Apps, Apps, "Files", &[]),
    row("apps.devtools", Apps, Apps, "Dev tools", &["logs"]),
    row("apps.kit", Apps, Apps, "UI kit", &["gallery"]),
    row("apps.phone", Apps, Apps, "Phone", &["calls"]),
    row("apps.contacts", Apps, Apps, "Contacts", &[]),
    row("apps.messages", Apps, Apps, "Messages", &["sms"]),
    row("apps.browser", Apps, Apps, "Browser", &["web"]),
    row(
        "apps.oldapps",
        Apps,
        OldApps,
        "Key pads for old apps",
        &["s60", "old apps", "legacy"],
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
    row(
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
    row(
        "storage.files",
        Storage,
        Storage,
        "Open Files",
        &["file manager"],
    ),
    row(
        "storage.usb",
        Storage,
        Storage,
        "Share E: over USB",
        &["mass storage", "usb"],
    ),
    // Developer options
    row(
        "dev.usb",
        Developer,
        Developer,
        "USB debugging",
        &["adb", "shell"],
    ),
    row(
        "dev.log",
        Developer,
        Developer,
        "Kernel log",
        &["console", "logs"],
    ),
    row("dev.save", Developer, Developer, "Save logs", &["logs"]),
    row(
        "dev.touches",
        Developer,
        Developer,
        "Show touches",
        &["pointer", "finger", "tap"],
    ),
    row(
        "dev.tools",
        Developer,
        Developer,
        "Open Dev tools",
        &["devices", "processes"],
    ),
    // About phone
    row(
        "about.licences",
        About,
        Licences,
        "Licences",
        &["licenses", "open source", "gpl", "apache"],
    ),
    row(
        "about.reset",
        About,
        Reset,
        "Reset",
        &["factory", "defaults", "restore", "erase"],
    ),
    // Reset (SE4a)
    row(
        "reset.network",
        Reset,
        Reset,
        "Reset network settings",
        &["forget networks", "wifi", "wlan"],
    ),
    row(
        "reset.settings",
        Reset,
        Reset,
        "Reset settings",
        &["defaults", "factory"],
    ),
];

/// The rows matching `query` (case-insensitive, anywhere in the title, an alias, the page's title or the row's live
/// value as `value` gives it), best first: title matches, then aliases, then the page's title, then the value. An empty
/// query finds nothing.
#[must_use]
pub fn search_with(query: &str, value: impl Fn(&Row) -> String) -> Vec<&'static Row> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let rank = |r: &Row| {
        if r.title.to_lowercase().contains(&q) {
            Some(0)
        } else if r.aliases.iter().any(|a| a.contains(&q)) {
            Some(1)
        } else if r.on != Page::Root && r.on.title().to_lowercase().contains(&q) {
            Some(2)
        } else if value(r).to_lowercase().contains(&q) {
            Some(3)
        } else {
            None
        }
    };
    let mut hits: Vec<(u8, usize)> = ROWS
        .iter()
        .enumerate()
        .filter_map(|(i, r)| rank(r).map(|k| (k, i)))
        .collect();
    hits.sort_unstable();
    hits.into_iter().map(|(_, i)| &ROWS[i]).collect()
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

    /// Search without live values.
    fn search(query: &str) -> Vec<&'static Row> {
        search_with(query, |_| String::new())
    }

    #[test]
    fn live_values_are_found_last() {
        let hits = search_with("30 sec", |r| {
            if r.id == "display.timeout" {
                "30 seconds".into()
            } else {
                String::new()
            }
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "display.timeout");
    }

    #[test]
    fn search_ranks_titles_first() {
        let hits = search("time");
        assert!(!hits.is_empty());
        assert!(hits[0].title.to_lowercase().contains("time"));
        assert!(hits.iter().any(|r| r.id == "time.zone"));
        assert!(search("  ").is_empty());
        assert_eq!(search("SMS")[0].id, "apps.messages");
        // a root row opens its page
        assert_eq!(search("about")[0].opens, Page::About);
        // the languages' own names find their keyboards (the rows show them since V2 batch 4)
        assert_eq!(search("Русский")[0].id, "kb.2");
        assert_eq!(search("հայերեն")[0].id, "kb.1");
    }
}

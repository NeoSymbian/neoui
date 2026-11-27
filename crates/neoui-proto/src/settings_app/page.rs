// SPDX-License-Identifier: Apache-2.0
//! The Settings app's pages, by a stable number: the shell opens one with `AppEvent::Open { page }` or
//! `AppEvent::Page { page }` (`host::PAGES`), and an app names one in `OPEN("settings", page, arg)`.

/// Every page of SE0, numbered for good: later pages append.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Page {
    Root = 0,
    Wifi = 1,
    Mobile = 2,
    Bluetooth = 3,
    Connectivity = 4,
    Display = 5,
    Navigation = 6,
    Sounds = 7,
    Battery = 8,
    DateTime = 9,
    Keyboard = 10,
    Developer = 11,
    About = 12,
    Apps = 13,
    OldApps = 14,
    Privacy = 15,
    Licences = 16,
    WifiSaved = 17,
    Profile = 18,
    BatteryUsage = 19,
    Storage = 20,
    /// One app (SE4a): Open, its settings, Stop, Clear data; the app is NEOSET's current one.
    AppDetail = 21,
    Reset = 22,
    /// A profile's ringtone (set2-ringtone): the tones, each with a preview.
    Ringtone = 23,
    /// The city whose time the phone keeps (set2-timezone), when the time zone isn't automatic.
    TimeZone = 24,
    /// (D7) Home's wallpaper (set2-wallpaper): the colours.
    Wallpaper = 25,
    /// (D7) The text size (set2-fontsize): a preview and the four sizes.
    FontSize = 26,
    /// (TP4) Panel & notifications (tp-settings): the shade's swipe, App notifications, Do not disturb, pop-up time.
    Panel = 27,
    /// (TP4) Notifications (set2-notifications): Do not disturb and every app with its policy.
    Notifications = 28,
    /// (TP4) One app's notifications (set2-notify-app, nt-settings): its switches and channels; the app is NEOSET's
    /// current one.
    NotifyApp = 29,
    /// (D11) SIM card lock (set2-sim-lock): Use SIM PIN, Change SIM PIN, the attempts left.
    SimLock = 30,
    /// (D3) One drive (set2-storage-drive): its kinds, file system and size; the drive is NEOSET's current one.
    StorageDrive = 31,
    /// (D14, not yet) A Bluetooth device (set2-bt-device); the device is NEOSET's current one.
    BtDevice = 32,
    /// (D12, not yet) Add a hidden Wi-Fi network (set2-wifi-hidden).
    WifiHidden = 33,
}

pub const ALL: [Page; 34] = [
    Page::Root,
    Page::Wifi,
    Page::Mobile,
    Page::Bluetooth,
    Page::Connectivity,
    Page::Display,
    Page::Navigation,
    Page::Sounds,
    Page::Battery,
    Page::DateTime,
    Page::Keyboard,
    Page::Developer,
    Page::About,
    Page::Apps,
    Page::OldApps,
    Page::Privacy,
    Page::Licences,
    Page::WifiSaved,
    Page::Profile,
    Page::BatteryUsage,
    Page::Storage,
    Page::AppDetail,
    Page::Reset,
    Page::Ringtone,
    Page::TimeZone,
    Page::Wallpaper,
    Page::FontSize,
    Page::Panel,
    Page::Notifications,
    Page::NotifyApp,
    Page::SimLock,
    Page::StorageDrive,
    Page::BtDevice,
    Page::WifiHidden,
];

impl Page {
    #[must_use]
    pub fn from_id(id: u16) -> Option<Self> {
        ALL.get(usize::from(id)).copied()
    }

    #[must_use]
    pub fn id(self) -> u16 {
        self as u16
    }

    /// The name logs, the shell's views and `OPEN`'s argument use ("" for the root).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Page::Root => "",
            Page::Wifi => "wifi",
            Page::Mobile => "mobile",
            Page::Bluetooth => "bluetooth",
            Page::Connectivity => "connectivity",
            Page::Display => "display",
            Page::Navigation => "navigation",
            Page::Sounds => "sounds",
            Page::Battery => "battery",
            Page::DateTime => "datetime",
            Page::Keyboard => "keyboard",
            Page::Developer => "developer",
            Page::About => "about",
            Page::Apps => "apps",
            Page::OldApps => "oldapps",
            Page::Privacy => "privacy",
            Page::Licences => "licences",
            Page::WifiSaved => "wifisaved",
            Page::Profile => "profile",
            Page::BatteryUsage => "batteryusage",
            Page::Storage => "storage",
            Page::AppDetail => "app",
            Page::Reset => "reset",
            Page::Ringtone => "ringtone",
            Page::TimeZone => "timezone",
            Page::Wallpaper => "wallpaper",
            Page::FontSize => "fontsize",
            Page::Panel => "panel",
            Page::Notifications => "notifications",
            Page::NotifyApp => "notifyapp",
            Page::SimLock => "simlock",
            Page::StorageDrive => "drive",
            Page::BtDevice => "btdevice",
            Page::WifiHidden => "wifihidden",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        ALL.iter().copied().find(|p| p.name() == name)
    }

    /// The page an `OPEN("settings", page, arg)` asks for: a named page in `arg` wins (the browser's
    /// `OPEN("settings", 0, "wifi")`, UI5), else the number.
    #[must_use]
    pub fn opened(page: u16, arg: &str) -> Self {
        Self::from_name(arg)
            .filter(|_| !arg.is_empty())
            .or_else(|| Self::from_id(page))
            .unwrap_or(Page::Root)
    }

    /// The page an open from outside the app shows: an app's page needs the app chosen on Apps, so Apps; a
    /// ringtone needs its profile chosen, so Sounds & profiles; a city needs Date & time's automatic zone off, so
    /// Date & time.
    #[must_use]
    pub fn from_outside(self) -> Self {
        match self {
            Page::AppDetail => Page::Apps,
            Page::Ringtone => Page::Sounds,
            Page::TimeZone => Page::DateTime,
            Page::NotifyApp => Page::Notifications,
            Page::StorageDrive => Page::Storage,
            Page::BtDevice => Page::Bluetooth,
            p => p,
        }
    }

    /// The header's title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Page::Root => "Settings",
            Page::Wifi => "Wi-Fi",
            Page::Mobile => "Mobile network",
            Page::Bluetooth => "Bluetooth",
            Page::Connectivity => "Connectivity",
            Page::Display => "Display",
            Page::Navigation => "Home & navigation",
            Page::Sounds => "Sounds & profiles",
            Page::Battery => "Battery",
            Page::DateTime => "Date & time",
            Page::Keyboard => "Language & keyboard",
            Page::Developer => "Developer options",
            Page::About => "About phone",
            Page::Apps => "Apps",
            Page::OldApps => "Classic apps",
            Page::Privacy => "Privacy",
            Page::Licences => "Licences",
            Page::WifiSaved => "Saved networks",
            Page::Profile => "Profile",
            Page::BatteryUsage => "Battery usage",
            Page::Storage => "Storage",
            Page::AppDetail => "App",
            Page::Reset => "Reset",
            Page::Ringtone => "Ringtone",
            Page::TimeZone => "Time zone",
            Page::Wallpaper => "Wallpaper",
            Page::FontSize => "Font size",
            Page::Panel => "Panel & notifications",
            Page::Notifications | Page::NotifyApp => "Notifications",
            Page::SimLock => "SIM card lock",
            Page::StorageDrive => "Drive",
            Page::BtDevice => "Bluetooth device",
            Page::WifiHidden => "Add network",
        }
    }

    /// Where Back goes from this page (`None` from the root: Back leaves the app).
    #[must_use]
    pub fn parent(self) -> Option<Self> {
        match self {
            Page::Root => None,
            Page::OldApps => Some(Page::Apps),
            Page::Licences => Some(Page::About),
            Page::WifiSaved => Some(Page::Wifi),
            Page::Profile => Some(Page::Sounds),
            Page::BatteryUsage => Some(Page::Battery),
            Page::AppDetail => Some(Page::Apps),
            Page::Reset => Some(Page::About),
            Page::Ringtone => Some(Page::Profile),
            Page::TimeZone => Some(Page::DateTime),
            Page::Wallpaper | Page::FontSize => Some(Page::Display),
            Page::Notifications => Some(Page::Panel),
            Page::NotifyApp => Some(Page::Notifications),
            Page::SimLock => Some(Page::Mobile),
            Page::StorageDrive => Some(Page::Storage),
            Page::BtDevice => Some(Page::Bluetooth),
            Page::WifiHidden => Some(Page::Wifi),
            _ => Some(Page::Root),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_names_round_trip() {
        for (i, p) in ALL.iter().enumerate() {
            assert_eq!(usize::from(p.id()), i);
            assert_eq!(Page::from_id(p.id()), Some(*p));
            assert_eq!(Page::from_name(p.name()), Some(*p));
        }
        assert_eq!(Page::from_id(34), None);
        assert_eq!(Page::opened(31, "").from_outside(), Page::Storage);
        assert_eq!(Page::opened(0, "btdevice").from_outside(), Page::Bluetooth);
        assert_eq!(Page::WifiHidden.parent(), Some(Page::Wifi));
        assert_eq!(Page::StorageDrive.parent(), Some(Page::Storage));
        assert_eq!(Page::SimLock.parent(), Some(Page::Mobile));
        assert_eq!(Page::opened(0, "simlock"), Page::SimLock);
        assert_eq!(Page::opened(29, "").from_outside(), Page::Notifications);
        assert_eq!(Page::NotifyApp.parent(), Some(Page::Notifications));
        assert_eq!(Page::Notifications.parent(), Some(Page::Panel));
        assert_eq!(Page::opened(0, "wallpaper"), Page::Wallpaper);
        assert_eq!(Page::FontSize.parent(), Some(Page::Display));
        assert_eq!(Page::Ringtone.parent(), Some(Page::Profile));
        assert_eq!(Page::TimeZone.parent(), Some(Page::DateTime));
        assert_eq!(Page::opened(23, "").from_outside(), Page::Sounds);
        assert_eq!(Page::opened(0, "timezone").from_outside(), Page::DateTime);
        assert_eq!(Page::AppDetail.parent(), Some(Page::Apps));
        assert_eq!(Page::Reset.parent(), Some(Page::About));
        assert_eq!(Page::opened(21, "").from_outside(), Page::Apps);
        assert_eq!(Page::opened(0, "app").from_outside(), Page::Apps);
        assert_eq!(Page::Reset.from_outside(), Page::Reset);
        assert_eq!(Page::BatteryUsage.parent(), Some(Page::Battery));
    }

    #[test]
    fn open_takes_the_name_then_the_number() {
        assert_eq!(Page::opened(0, "wifi"), Page::Wifi); // the browser's Check Wi-Fi
        assert_eq!(Page::opened(5, ""), Page::Display);
        assert_eq!(Page::opened(99, "nonsense"), Page::Root);
        assert_eq!(Page::OldApps.parent(), Some(Page::Apps));
        assert_eq!(Page::WifiSaved.parent(), Some(Page::Wifi));
        assert_eq!(Page::Root.parent(), None);
    }
}

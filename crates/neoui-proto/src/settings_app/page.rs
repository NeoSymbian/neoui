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
}

pub const ALL: [Page; 23] = [
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

    /// The page an open from outside the app shows: an app's page needs the app chosen on Apps, so Apps.
    #[must_use]
    pub fn from_outside(self) -> Self {
        if self == Page::AppDetail {
            Page::Apps
        } else {
            self
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
            Page::Sounds => "Sounds",
            Page::Battery => "Battery",
            Page::DateTime => "Date & time",
            Page::Keyboard => "Language & keyboard",
            Page::Developer => "Developer options",
            Page::About => "About phone",
            Page::Apps => "Apps",
            Page::OldApps => "Old apps",
            Page::Privacy => "Privacy",
            Page::Licences => "Licences",
            Page::WifiSaved => "Saved networks",
            Page::Profile => "Profile",
            Page::BatteryUsage => "Battery usage",
            Page::Storage => "Storage",
            Page::AppDetail => "App",
            Page::Reset => "Reset",
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
        assert_eq!(Page::from_id(23), None);
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

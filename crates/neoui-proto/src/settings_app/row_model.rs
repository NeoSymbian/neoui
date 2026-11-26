// SPDX-License-Identifier: Apache-2.0
//! D1, the Settings row model (docs neoui-shell/settings-data.md D1): one catalogue ([`rows::ROWS`]), one
//! [`Facts`] reading per refresh, and one projection of it ([`project`] → [`RowSnapshot`]) that the page's row, its
//! search hit and the root summary all read, so a value is formatted once, by its [`Provider`].
//!
//! - A row is a [`RowKey`]: its stable definition id ([`RowId`], never the title) and, for a family, an
//!   [`InstanceId`] (a profile by id, a drive by letter, an app by key, a Wi-Fi field by BSSID and field code). The
//!   old ids `sounds.p2` and `apps.clock` are aliases of the typed instance ([`RowKey::alias`], [`RowKey::from_alias`]).
//! - A row's [`Reading`] says what is known: a value, counting, partial, stale, unavailable (with why) or none (an
//!   action). Missing measurements are never zero. Its [`Availability`] hides or disables it.
//! - A [`Destination`] is the page, the instance it is about (installed before the page shows), a typed sheet and the
//!   row to show selected. Opening a hit never toggles, clears, connects, forgets or submits anything.
//! - [`search`] ranks title (an exact title first, then whole words, page rows before root rows), other names, the
//!   page or path, then a public value; it returns [`Hit`]s whose opaque token [`resolve_hit`] checks against the
//!   current snapshot: an instance that is gone, or whose identity list changed since (a family's epoch in
//!   [`Epochs`]), is "No longer available"; a changed value on the same instance opens normally.
//!
//! Pure: no Slint, E32, IPC or files. settings-ui's `row_model` builds [`Facts`] from the screens' state; NEOSET's
//! `row_model` refreshes the owners, bumps the epochs, keeps the tokens and the Back frames.
//!
//! # Binding contract for later streams
//!
//! To add a row: put its definition in [`rows::ROWS`] (a plain row: `row`/`action`; a family: `family` with a new
//! [`rows::ProviderId`]), give its owner's typed facts a field in [`Facts`] (filled by settings-ui's adapter or the
//! device), and read it in a [`Provider`] (`enumerate`/`read`/`destination` on the same facts). A new instance kind
//! adds an [`InstanceId`] variant and its alias. A row whose producer isn't there yet stays out, or reads
//! `Unavailable("Not yet (<stage>)")`. The first binding of each stream (its module under
//! `symbian/apps/settings/src/`):
//!
//! | Stream / module | First row → instance / producer |
//! |---|---|
//! | D2 `d2.rs` | `wifi.field` "Access point" → `WifiField { bssid, AccessPoint }` from `LinkView`; `dev.console` from HAL readback (Hidden when the kernel can't say) |
//! | D3 `drive_detail.rs` | set2-storage-drive "Images & video" → `DriveKind { drive: E, kind }`; before Storage is visited a hit reads Pending or a timestamped count, and opens E: |
//! | D4 `app_detail.rs` | set2-app-browser "Storage used" → `App(Native("browser"))`, INFO 8; partial coverage is `Partial` |
//! | D5 `battery_cycle.rs` | set2-search "Screen-on time" → INFO 9; "since charged" only after an observed full→discharge boundary |
//! | D6 `profile_feedback.rs` | set2-profile "Keypad tones" → `Profile(id)`; an inactive profile's change updates its own hit |
//! | D7 `appearance.rs` | set2-display "Font size" → watched `display.font_size`; set2-wallpaper "From Files" is an action |
//! | D8 `privacy.rs` | set2-privacy "App permissions" → `App(id)`; a clipboard read never supplies a value |
//! | D9/TP4 `notifications.rs` | nt-settings "Conversations" → `Channel { owner: Native("messages"), id: 1 }`; page, summary and root share one policy snapshot |
//! | D10 `home_navigation.rs` | set2-navigation "Open apps columns" → the watched, applied layout; "Edit home" focuses the editor action |
//! | D11 `sim_lock.rs` | set2-sim-lock "PIN attempts left" → native attempts, 255 = `Unavailable` |
//! | D12 `hidden_wifi.rs` | set2-wifi-hidden "Add network" → an action; saved networks are `SavedSsid(raw)`; no password or draft name is indexed |
//! | D13 `mobile_data.rs` | set2-mobile "Data usage" → bearer + billing period with coverage (`Partial`) |
//! | D14 `bluetooth.rs` | set2-bt-device "Audio" → a bonded peer (new `InstanceId` variant) + profile |
//! | D15 `lock_policy.rs` | set2-lock "Screen lock PIN" → the derived enrolled status; never the PIN |
//! | D16 `energy.rs` | set2-battery-usage "Screen" → the INFO 9 cycle's estimate, until calibrated the observed discharge |
//! | D17 `volume.rs` | set2-storage-drive "Eject for USB" → E:'s mount generation (the drives epoch) + INFO 10 |
//! | D18 `updates.rs` | set2-about "Check for updates" → slot + build lines + INFO 11, else "Update service unavailable" |
//! | D19 `reset_jobs.rs` | set2-reset "Erase C:" → an action, gated by recovery |
//! | TP2b–TP8 `panel.rs` | tp-settings rows → their panel owners; tiles are `AppTile { owner, local_id }` |
//!
//! A stream's device module refreshes its owner where `row_model::refresh_sources` does the others (cheap reads on
//! search entry and resume; an expensive one on a bounded worker with one generation), and bumps its family's epoch
//! when an identity it can't prove the same (a remount, a deleted and reused id) may have been replaced.

use super::page::Page;
use super::rows::{self, ProviderId, ROWS, Row, Search};
use super::storage::{KINDS, Kind};
use crate::sound_profile::{self, Profile, tone};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A row definition's stable id (`rows::Row::id`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowId(pub &'static str);

/// An app: a ROM app by its screen key, or an old app by its legacy entry id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AppId {
    Native(&'static str),
    Legacy(u32),
}

/// The Wi-Fi details' fields (`wifi::LinkView` rows), by a code that never changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum WifiField {
    Status = 0,
    Signal = 1,
    Band = 2,
    Security = 3,
    AccessPoint = 4,
    PhoneMac = 5,
    AssociationId = 6,
    Received = 7,
    IpAddress = 8,
    Gateway = 9,
    Dns = 10,
    Internet = 11,
}

impl WifiField {
    pub const ALL: [WifiField; 12] = [
        WifiField::Status,
        WifiField::Signal,
        WifiField::Band,
        WifiField::Security,
        WifiField::AccessPoint,
        WifiField::PhoneMac,
        WifiField::AssociationId,
        WifiField::Received,
        WifiField::IpAddress,
        WifiField::Gateway,
        WifiField::Dns,
        WifiField::Internet,
    ];

    /// The label `LinkView::new`/`with_ip` gives the field.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            WifiField::Status => "Status",
            WifiField::Signal => "Signal",
            WifiField::Band => "Band",
            WifiField::Security => "Security",
            WifiField::AccessPoint => "Access point",
            WifiField::PhoneMac => "Phone MAC",
            WifiField::AssociationId => "Association ID",
            WifiField::Received => "Received",
            WifiField::IpAddress => "IP address",
            WifiField::Gateway => "Gateway",
            WifiField::Dns => "DNS",
            WifiField::Internet => "Internet",
        }
    }

    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.label() == label)
    }
}

/// Which one of a family's rows. Extend with a typed peer, period or energy category when its owner lands.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InstanceId {
    /// a sound profile by id (an added profile's id can be reused after a delete: the profiles epoch guards it)
    Profile(u8),
    /// a drive by number (0 = A: … 25 = Z:); a letter is not a medium: the drives epoch guards a remount
    Drive(u8),
    DriveKind {
        drive: u8,
        kind: Kind,
    },
    /// C: and E: together, by kind
    OverviewKind(Kind),
    App(AppId),
    Channel {
        owner: AppId,
        id: u8,
    },
    WifiAp([u8; 6]),
    WifiField {
        bssid: [u8; 6],
        field: WifiField,
    },
    /// a saved network by its raw SSID (two can show the same name)
    SavedSsid(Vec<u8>),
    Tone {
        profile: u8,
        tone: u8,
    },
    AppTile {
        owner: AppId,
        local_id: u16,
    },
}

/// A row: its definition and, for a family, which instance. Equality and order include the whole instance.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RowKey {
    pub id: RowId,
    pub instance: Option<InstanceId>,
}

impl RowKey {
    #[must_use]
    pub fn of(id: &'static str) -> Self {
        RowKey {
            id: RowId(id),
            instance: None,
        }
    }

    #[must_use]
    pub fn with(id: &'static str, instance: InstanceId) -> Self {
        RowKey {
            id: RowId(id),
            instance: Some(instance),
        }
    }

    /// The text a page compares `SetState.found` with: the row id, or for an instance the old id (`sounds.p2`,
    /// `apps.clock`, `apps.20009aa3`) or `<id>.<instance>` (`storage.drive.E`, `storage.kind.0`, `wifi.field.4`).
    #[must_use]
    pub fn alias(&self) -> String {
        let id = self.id.0;
        match &self.instance {
            Some(InstanceId::Profile(p)) if id == "sounds.profile" => format!("sounds.p{p}"),
            Some(InstanceId::App(AppId::Native(k))) => format!("apps.{k}"),
            Some(InstanceId::App(AppId::Legacy(e))) => format!("apps.{e:08x}"),
            Some(InstanceId::Drive(d)) => format!("{id}.{}", char::from(b'A' + d)),
            Some(InstanceId::OverviewKind(k)) => format!("{id}.{}", *k as u8),
            Some(InstanceId::WifiField { field, .. }) => format!("{id}.{}", *field as u8),
            _ => String::from(id),
        }
    }

    /// The row an old id names: `sounds.p<id>` is that profile, `apps.<key>` that ROM app, `apps.<8 hex>` that old
    /// app; any other row id is itself.
    #[must_use]
    pub fn from_alias(alias: &str) -> Option<Self> {
        if let Some(p) = alias.strip_prefix("sounds.p").and_then(|p| p.parse().ok()) {
            return Some(RowKey::with("sounds.profile", InstanceId::Profile(p)));
        }
        if let Some(k) = alias.strip_prefix("apps.") {
            if let Some(a) = crate::apps::find(k) {
                return Some(RowKey::with(
                    "apps.app",
                    InstanceId::App(AppId::Native(a.key)),
                ));
            }
            if k.len() == 8
                && let Ok(e) = u32::from_str_radix(k, 16)
            {
                return Some(RowKey::with("apps.app", InstanceId::App(AppId::Legacy(e))));
            }
        }
        rows::find(alias)
            .filter(|r| matches!(r.provider, ProviderId::Builtin | ProviderId::Action))
            .map(|r| RowKey::of(r.id))
    }
}

/// A sheet a destination opens over its page (state.slint's `sheet` names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    /// an old app's sheet on Apps
    App,
    /// the current link's details on Wi-Fi
    WifiDetails,
}

impl Sheet {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Sheet::App => "app",
            Sheet::WifiDetails => "wifi-details",
        }
    }
}

/// Where a row is: the page, the instance the page is about (a profile, an app), a sheet over it, the row selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub page: Page,
    pub context: Option<InstanceId>,
    pub sheet: Option<Sheet>,
    pub focus: Option<RowKey>,
}

impl Destination {
    #[must_use]
    pub fn page(page: Page) -> Self {
        Destination {
            page,
            context: None,
            sheet: None,
            focus: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// not on its page now (the kernel can't say, the profile doesn't show it): not found either
    Hidden,
    Readable,
    /// on its page but not usable yet, and why
    Disabled(&'static str),
}

/// What is known about a row's value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reading {
    Value(String),
    /// being measured ("Counting…")
    Pending,
    /// some of it (the text says how much)
    Partial(String),
    /// an older measurement, the text names when
    Stale(String),
    /// no value, and why ("C: can't be read")
    Unavailable(&'static str),
    /// an action or a page: no value at all
    NoValue,
}

impl Reading {
    /// The reading as the page, search and the root show it.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Reading::Value(s) | Reading::Partial(s) | Reading::Stale(s) => s.clone(),
            Reading::Pending => String::from("Counting…"),
            Reading::Unavailable(why) => String::from(*why),
            Reading::NoValue => String::new(),
        }
    }
}

/// When a projection was made: the facts' generation, the monotonic time they were read at, and (when an owner
/// says) the UTC time it observed them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    pub generation: u32,
    pub sampled_ms: u64,
    pub utc: Option<i64>,
}

/// One counter per family whose identities can be replaced: moved on when its list of identities changed in a way
/// that may hide a replacement (a delete then a reused id, a remount, a link break), and on resume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Epochs {
    pub profiles: u32,
    pub drives: u32,
    pub apps: u32,
    pub wifi: u32,
}

/// A simple fact, by row id (or a store key a root summary reads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scalar {
    On(bool),
    Text(String),
    /// a choice: its index in the choice table and its name
    Choice(usize, String),
}

/// A drive as the file server listed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveFact {
    pub letter: char,
    /// "E: Mass memory"
    pub title: String,
    /// "12.4 of 41.8 GB", "Not available"
    pub used: String,
}

/// Storage's scan, for the by-kind rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum KindsState {
    #[default]
    NotCounted,
    Counting,
    /// counted: its status ("Last counted 06:50")
    Counted(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppFact {
    pub id: AppId,
    pub name: String,
}

/// The current link (Wi-Fi's "Current network"): its BSSID and the fields its details show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkFact {
    pub bssid: [u8; 6],
    pub fields: Vec<(WifiField, String)>,
}

/// Everything the providers read, at one generation.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub sample: Sample,
    pub epochs: Epochs,
    /// settings, choices and readings the device formats, by row id; and for the root's summaries `radio.wifi`,
    /// `radio.bt`, `radio.data`, `radio.airplane`, `wifi.network` (the joined network), `rat`, `nav.mode`,
    /// `storage.free` (the root's "C: N free", D3), `kb.display_language`
    pub scalars: BTreeMap<&'static str, Scalar>,
    pub profiles: Vec<Profile>,
    pub active_profile: u8,
    pub drives: Vec<DriveFact>,
    pub kinds: Vec<(Kind, String)>,
    pub kinds_state: KindsState,
    pub apps: Vec<AppFact>,
    pub link: Option<LinkFact>,
}

impl Facts {
    fn on(&self, key: &str) -> Option<bool> {
        match self.scalars.get(key) {
            Some(Scalar::On(b)) => Some(*b),
            _ => None,
        }
    }

    fn text(&self, key: &str) -> Option<&str> {
        match self.scalars.get(key) {
            Some(Scalar::Text(t) | Scalar::Choice(_, t)) => Some(t),
            _ => None,
        }
    }

    fn profile(&self, key: &RowKey) -> Option<&Profile> {
        match key.instance {
            Some(InstanceId::Profile(id)) => self.profiles.iter().find(|p| p.id == id),
            _ => None,
        }
    }
}

/// A row's title, where it is ("Date & time", a profile's name) and its instance's other names.
pub struct Label {
    pub title: String,
    pub path: String,
    pub aliases: &'static [&'static str],
}

/// A family's values. All methods read the same facts generation.
pub trait Provider {
    /// The instances of `row` the facts hold (a plain row: itself, once).
    fn enumerate(&self, _f: &Facts, row: &'static Row, out: &mut Vec<RowKey>) {
        out.push(RowKey::of(row.id));
    }

    fn read(&self, f: &Facts, key: &RowKey) -> Reading;

    fn destination(&self, _f: &Facts, key: &RowKey) -> Option<Destination> {
        let row = rows::find(key.id.0)?;
        Some(Destination {
            page: row.opens,
            context: key.instance.clone(),
            sheet: None,
            focus: (row.on != Page::Root).then(|| key.clone()),
        })
    }

    fn label(&self, _f: &Facts, key: &RowKey) -> Label {
        let row = rows::find(key.id.0);
        Label {
            title: String::from(row.map_or("", |r| r.title)),
            path: String::from(row.map_or("", |r| path_of(r.on))),
            aliases: &[],
        }
    }

    fn availability(&self, _f: &Facts, _key: &RowKey) -> Availability {
        Availability::Readable
    }

    /// What the value comes with: when it was measured ("Last counted 06:50").
    fn detail(&self, _f: &Facts, _key: &RowKey) -> String {
        String::new()
    }

    /// The epoch that guards this family's identities (0: a row that is always the same one).
    fn guard(&self, _f: &Facts) -> u32 {
        0
    }
}

fn path_of(page: Page) -> &'static str {
    if page == Page::Root {
        "Settings"
    } else {
        page.title()
    }
}

fn on_off(b: bool) -> Reading {
    Reading::Value(String::from(if b { "On" } else { "Off" }))
}

fn known(t: Option<&str>) -> Reading {
    match t {
        Some(t) if !t.is_empty() => Reading::Value(String::from(t)),
        _ => Reading::Unavailable("No reading"),
    }
}

struct Builtin;
impl Provider for Builtin {
    fn read(&self, f: &Facts, key: &RowKey) -> Reading {
        let id = key.id.0;
        let pick = |yes: &str, no: &str, b: Option<bool>| {
            b.map_or(Reading::Unavailable("No reading"), |b| {
                Reading::Value(String::from(if b { yes } else { no }))
            })
        };
        match id {
            "root.wifi" | "wifi.switch" | "conn.wifi" => match f.on("radio.wifi") {
                Some(false) => Reading::Value(String::from("Off")),
                Some(true) => Reading::Value(String::from(
                    f.text("wifi.network")
                        .filter(|n| !n.is_empty())
                        .unwrap_or("Not connected"),
                )),
                None => Reading::Unavailable("No reading"),
            },
            "root.bt" | "bt.switch" | "conn.bt" => f.on("radio.bt").map_or(known(None), on_off),
            "conn.data" | "mobile.data" => f.on("radio.data").map_or(known(None), on_off),
            "root.airplane" | "conn.airplane" => f.on("radio.airplane").map_or(known(None), on_off),
            // airplane mode turns the modem off, whatever technology it had last
            "root.mobile" if f.on("radio.airplane") == Some(true) => {
                Reading::Value(String::from("Off"))
            }
            "root.mobile" => Reading::Value(String::from(
                f.text("rat")
                    .filter(|r| !r.is_empty())
                    .unwrap_or("No service"),
            )),
            "root.display" => pick("Dark", "Light", f.on("display.dark")),
            "root.navigation" => known(f.text("nav.mode")),
            "nav.0" | "nav.1" | "nav.2" => match f.scalars.get("nav.mode") {
                Some(Scalar::Choice(i, _)) => on_off(id[4..].parse() == Ok(*i)),
                _ => known(None),
            },
            "root.sounds" => known(
                f.profiles
                    .iter()
                    .find(|p| p.id == f.active_profile)
                    .map(|p| p.name.as_str()),
            ),
            // C: explicitly (the board's reading, "C: 3.1 GB free" as Storage makes it, D3)
            "root.storage" => match f.text("storage.free") {
                Some(free) if !free.is_empty() => Reading::Value(String::from(free)),
                _ => Reading::Unavailable("C: can't be read"),
            },
            "root.datetime" => pick("Automatic", "Manual", f.on("time.auto")),
            "root.keyboard" => known(f.text("kb.current")),
            "root.apps" => Reading::Value(String::from("Phone, old apps")),
            "root.developer" => Reading::Value(String::from(if f.on("dev.usb") == Some(true) {
                "USB debugging"
            } else if f.on("dev.touches") == Some(true) {
                "Show touches"
            } else {
                "Off"
            })),
            _ => match f.scalars.get(id) {
                Some(Scalar::On(b)) => on_off(*b),
                Some(Scalar::Text(t) | Scalar::Choice(_, t)) => known(Some(t)),
                None => known(None),
            },
        }
    }

    fn availability(&self, f: &Facts, key: &RowKey) -> Availability {
        // D2: only drawn when the kernel answered
        match key.id.0 {
            "dev.console" | "about.slot" if !f.scalars.contains_key(key.id.0) => {
                Availability::Hidden
            }
            _ => Availability::Readable,
        }
    }
}

struct Action;
impl Provider for Action {
    fn read(&self, _f: &Facts, _key: &RowKey) -> Reading {
        Reading::NoValue
    }

    fn availability(&self, _f: &Facts, key: &RowKey) -> Availability {
        match key.id.0 {
            "storage.usb" => Availability::Disabled("Not yet (FS6)"),
            _ => Availability::Readable,
        }
    }
}

/// The built-in profiles' other names (by id).
const PROFILE_ALIASES: [&[&str]; 5] = [
    &["normal"],
    &["mute", "quiet"],
    &["beep"],
    &["loud"],
    &["airplane", "flight"],
];

struct Profiles;
impl Provider for Profiles {
    fn enumerate(&self, f: &Facts, row: &'static Row, out: &mut Vec<RowKey>) {
        for p in &f.profiles {
            out.push(RowKey::with(row.id, InstanceId::Profile(p.id)));
        }
    }

    fn read(&self, f: &Facts, key: &RowKey) -> Reading {
        f.profile(key)
            .map_or(Reading::Unavailable("No longer available"), |p| {
                Reading::Value(p.summary())
            })
    }

    fn label(&self, f: &Facts, key: &RowKey) -> Label {
        let p = f.profile(key);
        Label {
            title: p.map_or_else(String::new, |p| p.name.clone()),
            path: String::from(Page::Sounds.title()),
            aliases: p
                .and_then(|p| PROFILE_ALIASES.get(usize::from(p.id)))
                .copied()
                .unwrap_or(&[]),
        }
    }

    fn guard(&self, f: &Facts) -> u32 {
        f.epochs.profiles
    }
}

struct ProfileFields;
impl ProfileFields {
    /// Whether profile `p`'s page draws row `id` (device.slint's `ProfilePage`).
    fn shows(p: &Profile, id: &str) -> bool {
        p.id != sound_profile::OFFLINE
            && match id {
                "profile.tone" | "profile.volume" => p.ring,
                "profile.ring" => p.id != sound_profile::SILENT,
                _ => true,
            }
    }
}
impl Provider for ProfileFields {
    fn enumerate(&self, f: &Facts, row: &'static Row, out: &mut Vec<RowKey>) {
        for p in f.profiles.iter().filter(|p| Self::shows(p, row.id)) {
            out.push(RowKey::with(row.id, InstanceId::Profile(p.id)));
        }
    }

    fn read(&self, f: &Facts, key: &RowKey) -> Reading {
        let Some(p) = f.profile(key) else {
            return Reading::Unavailable("No longer available");
        };
        match key.id.0 {
            "profile.tone" => Reading::Value(String::from(tone::ring_name(p.ringtone))),
            "profile.ring" => on_off(p.ring),
            "profile.volume" => Reading::Value(format!("{}", p.volume)),
            "profile.vibrate" => on_off(p.vibrate),
            "profile.keypad" => on_off(p.keypad),
            "profile.touch" => on_off(p.touch),
            _ => Reading::Value(String::from(tone::message_name(p.message_tone))),
        }
    }

    fn label(&self, f: &Facts, key: &RowKey) -> Label {
        Label {
            title: String::from(rows::find(key.id.0).map_or("", |r| r.title)),
            // the profile page's title is the profile's name
            path: f.profile(key).map_or_else(String::new, |p| p.name.clone()),
            aliases: &[],
        }
    }

    fn guard(&self, f: &Facts) -> u32 {
        f.epochs.profiles
    }
}

struct Drives;
impl Provider for Drives {
    fn enumerate(&self, f: &Facts, row: &'static Row, out: &mut Vec<RowKey>) {
        for d in f.drives.iter().filter(|d| d.letter.is_ascii_uppercase()) {
            out.push(RowKey::with(
                row.id,
                InstanceId::Drive(d.letter as u8 - b'A'),
            ));
        }
    }

    fn read(&self, f: &Facts, key: &RowKey) -> Reading {
        drive(f, key).map_or(Reading::Unavailable("No longer available"), |d| {
            known(Some(&d.used))
        })
    }

    fn label(&self, f: &Facts, key: &RowKey) -> Label {
        Label {
            title: drive(f, key).map_or_else(String::new, |d| d.title.clone()),
            path: String::from(Page::Storage.title()),
            aliases: &[],
        }
    }

    fn guard(&self, f: &Facts) -> u32 {
        f.epochs.drives
    }
}

fn drive<'a>(f: &'a Facts, key: &RowKey) -> Option<&'a DriveFact> {
    match key.instance {
        Some(InstanceId::Drive(n)) => f
            .drives
            .iter()
            .find(|d| d.letter as u32 == u32::from(b'A' + n)),
        _ => None,
    }
}

struct OverviewKinds;
impl Provider for OverviewKinds {
    fn enumerate(&self, _f: &Facts, row: &'static Row, out: &mut Vec<RowKey>) {
        for k in KINDS {
            out.push(RowKey::with(row.id, InstanceId::OverviewKind(k)));
        }
    }

    fn read(&self, f: &Facts, key: &RowKey) -> Reading {
        let Some(InstanceId::OverviewKind(k)) = key.instance else {
            return Reading::Unavailable("No reading");
        };
        let size = f.kinds.iter().find(|(x, _)| *x == k).map(|(_, s)| s);
        match (&f.kinds_state, size) {
            (KindsState::Counting, _) => Reading::Pending,
            (KindsState::Counted(_), Some(size)) => Reading::Value(size.clone()),
            _ => Reading::Unavailable("Not counted yet"),
        }
    }

    // a count is a measurement of then: it names when
    fn detail(&self, f: &Facts, _key: &RowKey) -> String {
        match &f.kinds_state {
            KindsState::Counted(at) => at.clone(),
            _ => String::new(),
        }
    }

    fn label(&self, _f: &Facts, key: &RowKey) -> Label {
        Label {
            title: String::from(match key.instance {
                Some(InstanceId::OverviewKind(k)) => k.name(),
                _ => "",
            }),
            path: String::from(Page::Storage.title()),
            aliases: &[],
        }
    }

    fn guard(&self, f: &Facts) -> u32 {
        f.epochs.drives
    }
}

/// The ROM apps' other names (by key).
const APP_ALIASES: &[(&str, &[&str])] = &[
    ("clock", &["alarms", "timer"]),
    ("calendar", &["events"]),
    ("terminal", &["shell"]),
    ("devtools", &["logs"]),
    ("kit", &["gallery"]),
    ("phone", &["calls"]),
    ("messages", &["sms"]),
    ("browser", &["web"]),
];

struct Apps;
impl Apps {
    fn app<'a>(f: &'a Facts, key: &RowKey) -> Option<&'a AppFact> {
        match &key.instance {
            Some(InstanceId::App(id)) => f.apps.iter().find(|a| a.id == *id),
            _ => None,
        }
    }
}
impl Provider for Apps {
    fn enumerate(&self, f: &Facts, row: &'static Row, out: &mut Vec<RowKey>) {
        for a in &f.apps {
            out.push(RowKey::with(row.id, InstanceId::App(a.id)));
        }
    }

    fn read(&self, _f: &Facts, _key: &RowKey) -> Reading {
        Reading::NoValue
    }

    fn label(&self, f: &Facts, key: &RowKey) -> Label {
        let a = Self::app(f, key);
        Label {
            title: a.map_or_else(String::new, |a| a.name.clone()),
            path: String::from(Page::Apps.title()),
            aliases: match a.map(|a| a.id) {
                Some(AppId::Native(k)) => APP_ALIASES
                    .iter()
                    .find(|(x, _)| *x == k)
                    .map_or(&[], |(_, a)| a),
                _ => &[],
            },
        }
    }

    fn destination(&self, f: &Facts, key: &RowKey) -> Option<Destination> {
        let a = Self::app(f, key)?;
        Some(match a.id {
            // a ROM app has its page; an old app its sheet on Apps
            AppId::Native(_) => Destination {
                page: Page::AppDetail,
                context: key.instance.clone(),
                sheet: None,
                focus: None,
            },
            AppId::Legacy(_) => Destination {
                page: Page::Apps,
                context: key.instance.clone(),
                sheet: Some(Sheet::App),
                focus: Some(key.clone()),
            },
        })
    }

    fn guard(&self, f: &Facts) -> u32 {
        f.epochs.apps
    }
}

struct WifiFields;
impl Provider for WifiFields {
    fn enumerate(&self, f: &Facts, row: &'static Row, out: &mut Vec<RowKey>) {
        if let Some(l) = &f.link {
            for (field, _) in &l.fields {
                out.push(RowKey::with(
                    row.id,
                    InstanceId::WifiField {
                        bssid: l.bssid,
                        field: *field,
                    },
                ));
            }
        }
    }

    fn read(&self, f: &Facts, key: &RowKey) -> Reading {
        let (Some(l), Some(InstanceId::WifiField { bssid, field })) = (&f.link, &key.instance)
        else {
            return Reading::Unavailable("No longer available");
        };
        l.fields
            .iter()
            .find(|(x, _)| x == field && l.bssid == *bssid)
            .map_or(Reading::Unavailable("No longer available"), |(_, v)| {
                Reading::Value(v.clone())
            })
    }

    fn label(&self, _f: &Facts, key: &RowKey) -> Label {
        Label {
            title: String::from(match key.instance {
                Some(InstanceId::WifiField { field, .. }) => field.label(),
                _ => "",
            }),
            path: String::from(Page::Wifi.title()),
            aliases: &[],
        }
    }

    fn destination(&self, _f: &Facts, key: &RowKey) -> Option<Destination> {
        Some(Destination {
            page: Page::Wifi,
            context: key.instance.clone(),
            sheet: Some(Sheet::WifiDetails),
            focus: Some(key.clone()),
        })
    }

    fn guard(&self, f: &Facts) -> u32 {
        f.epochs.wifi
    }
}

impl ProviderId {
    /// The provider that serves this family.
    #[must_use]
    pub fn get(self) -> &'static dyn Provider {
        match self {
            ProviderId::Builtin => &Builtin,
            ProviderId::Action => &Action,
            ProviderId::Profiles => &Profiles,
            ProviderId::ProfileFields => &ProfileFields,
            ProviderId::Drives => &Drives,
            ProviderId::OverviewKinds => &OverviewKinds,
            ProviderId::Apps => &Apps,
            ProviderId::WifiFields => &WifiFields,
        }
    }
}

/// One row as everything shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayRow {
    pub key: RowKey,
    pub title: String,
    pub path: String,
    pub aliases: Vec<&'static str>,
    pub reading: Reading,
    /// the page's value or switch state ("On", "30 seconds")
    pub primary: String,
    /// a subtitle the value comes with ("Last counted 06:50"), or a disabled row's reason
    pub detail: String,
    /// what search lists and matches: the reading's text, or a disabled row's reason
    pub value_text: String,
    pub availability: Availability,
    pub destination: Destination,
    pub search: Search,
    /// on the root (its path is "Settings")
    pub on_root: bool,
    pub sample: Sample,
    /// the family's epoch when projected
    pub guard: u32,
}

/// One projection of the catalogue.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowSnapshot {
    pub sample: Sample,
    pub rows: Vec<DisplayRow>,
}

impl RowSnapshot {
    #[must_use]
    pub fn get(&self, key: &RowKey) -> Option<&DisplayRow> {
        self.rows.iter().find(|r| r.key == *key)
    }

    /// The root list's summaries, `rows::ROOT_LIST` order (`SetState.summaries`).
    #[must_use]
    pub fn summaries(&self) -> Vec<String> {
        rows::ROOT_LIST
            .iter()
            .map(|id| {
                self.get(&RowKey::of(id))
                    .map_or_else(String::new, |r| r.value_text.clone())
            })
            .collect()
    }
}

/// Every row of `catalogue` the facts hold, as the pages, search and the root show it.
// shortcut: formatted again on every projection (≈100 short rows); cache by (key, generation) if it ever shows up
#[must_use]
pub fn project(catalogue: &'static [Row], f: &Facts) -> RowSnapshot {
    let mut rows = Vec::new();
    let mut keys = Vec::new();
    for spec in catalogue {
        let p = spec.provider.get();
        keys.clear();
        p.enumerate(f, spec, &mut keys);
        for key in keys.drain(..) {
            let Some(destination) = p.destination(f, &key) else {
                continue;
            };
            let label = p.label(f, &key);
            let reading = p.read(f, &key);
            let availability = p.availability(f, &key);
            let primary = reading.text();
            let (detail, value_text) = match availability {
                Availability::Disabled(why) => (String::from(why), String::from(why)),
                _ => {
                    let detail = p.detail(f, &key);
                    let text = if detail.is_empty() {
                        primary.clone()
                    } else {
                        format!("{primary} · {detail}")
                    };
                    (detail, text)
                }
            };
            let mut aliases: Vec<&'static str> = spec.aliases.to_vec();
            aliases.extend_from_slice(label.aliases);
            rows.push(DisplayRow {
                key,
                title: label.title,
                path: label.path,
                aliases,
                reading,
                primary,
                detail,
                value_text,
                availability,
                destination,
                search: spec.search,
                on_root: spec.on == Page::Root,
                sample: f.sample,
                guard: p.guard(f),
            });
        }
    }
    RowSnapshot {
        sample: f.sample,
        rows,
    }
}

/// [`project`] over the app's catalogue.
#[must_use]
pub fn project_all(f: &Facts) -> RowSnapshot {
    project(ROWS, f)
}

/// A search result: its token (its place in the results, what the screen sends back), the row in the snapshot,
/// and the identity it stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub token: u32,
    pub row: usize,
    pub key: RowKey,
    pub guard: u32,
}

/// Unicode lower case (Ü → ü, Д → д).
fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// `q` occurs in `s` as whole words.
fn word_match(s: &str, q: &str) -> bool {
    s.match_indices(q).any(|(i, _)| {
        let before = s[..i].chars().next_back();
        let after = s[i + q.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The rows matching `query` (case-insensitive, anywhere), best first: the title (an exact one first, then a whole
/// word, page rows before root rows), other names, the page or path (not the root's "Settings"), then a public value;
/// catalogue order within each. An empty query finds nothing; a hidden or unsearchable row is never found.
#[must_use]
pub fn search(snap: &RowSnapshot, query: &str) -> Vec<Hit> {
    let q = fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<((u8, bool, bool, bool), usize)> = snap
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.availability != Availability::Hidden && r.search != Search::Omit)
        .filter_map(|(i, r)| {
            let title = fold(&r.title);
            let tier = if title.contains(&q) {
                0
            } else if r.aliases.iter().any(|a| fold(a).contains(&q)) {
                1
            } else if !r.on_root && fold(&r.path).contains(&q) {
                2
            } else if r.search == Search::PublicValue
                && matches!(
                    r.reading,
                    Reading::Value(_) | Reading::Partial(_) | Reading::Stale(_)
                )
                && fold(&r.value_text).contains(&q)
            {
                3
            } else {
                return None;
            };
            let exact = tier == 0 && title == q;
            let word = tier == 0 && word_match(&title, &q);
            Some(((tier, !exact, r.on_root, !word), i))
        })
        .collect();
    ranked.sort_unstable();
    ranked
        .into_iter()
        .enumerate()
        .map(|(token, (_, row))| Hit {
            token: token as u32,
            row,
            key: snap.rows[row].key.clone(),
            guard: snap.rows[row].guard,
        })
        .collect()
}

/// The hit's instance is gone, or may have been replaced since the results were made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gone;

/// Where hit `token` of `hits` leads, checked against the current snapshot.
pub fn resolve_hit(token: u32, hits: &[Hit], current: &RowSnapshot) -> Result<Destination, Gone> {
    let hit = hits.iter().find(|h| h.token == token).ok_or(Gone)?;
    let row = current
        .get(&hit.key)
        .filter(|r| r.availability != Availability::Hidden && r.guard == hit.guard)
        .ok_or(Gone)?;
    Ok(row.destination.clone())
}

/// One step Back returns through: a page (opened from another page of the app), or search as it was when a hit was
/// opened from it.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    Page(Destination),
    Search {
        query: String,
        /// the results' scroll (logical px, `SetState.search-scroll`)
        // shortcut: the offset, not the nearest surviving row; the list clamps it when the results got shorter
        scroll: f32,
        selected: Option<RowKey>,
    },
}

/// The way Back goes in the app: the frames, else the page's parent. An open from outside starts a new one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trail(Vec<Frame>);

impl Trail {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn push(&mut self, f: Frame) {
        self.0.push(f);
    }

    /// Back from `here`: the frame it came from, else its parent page; `None` at the root (Back leaves the app).
    pub fn back(&mut self, here: Page) -> Option<Frame> {
        self.0
            .pop()
            .or_else(|| here.parent().map(|p| Frame::Page(Destination::page(p))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn facts() -> Facts {
        let mut f = Facts::default();
        let s = &mut f.scalars;
        s.insert("radio.wifi", Scalar::On(true));
        s.insert("wifi.network", Scalar::Text("Home-5G".into()));
        s.insert("radio.airplane", Scalar::On(false));
        s.insert("rat", Scalar::Text("3G".into()));
        s.insert("time.auto", Scalar::On(true));
        s.insert("time.auto_zone", Scalar::On(true));
        s.insert("time.24h", Scalar::On(true));
        s.insert("time.zone", Scalar::Text("UTC+04:00 · Yerevan".into()));
        s.insert("display.timeout", Scalar::Text("30 seconds".into()));
        s.insert("storage.free", Scalar::Text("C: 5.9 GB free".into()));
        s.insert("nav.mode", Scalar::Choice(1, "Gestures".into()));
        f.profiles = sound_profile::builtins();
        f.drives = vec![
            DriveFact {
                letter: 'E',
                title: "E: Mass memory".into(),
                used: "12.4 of 41.8 GB".into(),
            },
            DriveFact {
                letter: 'C',
                title: "C: Phone".into(),
                used: "2.1 of 8 GB".into(),
            },
        ];
        f.apps = crate::apps::listed()
            .map(|a| AppFact {
                id: AppId::Native(a.key),
                name: a.name.into(),
            })
            .collect();
        f.link = Some(LinkFact {
            bssid: [2, 0, 0, 0, 0, 1],
            fields: vec![
                (WifiField::Status, "Connected".into()),
                (WifiField::AccessPoint, "02:00:00:00:00:01".into()),
            ],
        });
        f
    }

    fn titles(snap: &RowSnapshot, q: &str) -> Vec<String> {
        search(snap, q)
            .iter()
            .map(|h| snap.rows[h.row].title.clone())
            .collect()
    }

    #[test]
    fn keys_are_unique_and_routes_resolve() {
        let mut f = facts();
        // two profiles with one name are two rows
        f.profiles.push(Profile {
            id: 5,
            name: "General".into(),
            ..f.profiles[0].clone()
        });
        let snap = project_all(&f);
        for (i, a) in snap.rows.iter().enumerate() {
            assert!(snap.rows[i + 1..].iter().all(|b| b.key != a.key), "{a:?}");
            let d = &a.destination;
            // a page that is about one instance gets it
            match d.page {
                Page::Profile => assert!(matches!(d.context, Some(InstanceId::Profile(_)))),
                Page::AppDetail => assert!(matches!(d.context, Some(InstanceId::App(_)))),
                _ => {}
            }
            if let Some(focus) = &d.focus {
                assert!(snap.get(focus).is_some(), "{focus:?}");
            }
        }
        let general: Vec<_> = search(&snap, "general")
            .into_iter()
            .filter(|h| h.key.id == RowId("sounds.profile"))
            .collect();
        assert_eq!(general.len(), 2);
        assert_ne!(general[0].key, general[1].key);
    }

    #[test]
    fn old_ids_are_aliases_of_their_instance() {
        for (alias, key) in [
            (
                "sounds.p2",
                RowKey::with("sounds.profile", InstanceId::Profile(2)),
            ),
            (
                "apps.clock",
                RowKey::with("apps.app", InstanceId::App(AppId::Native("clock"))),
            ),
            (
                "apps.20009aa3",
                RowKey::with("apps.app", InstanceId::App(AppId::Legacy(0x2000_9aa3))),
            ),
            ("display.timeout", RowKey::of("display.timeout")),
        ] {
            assert_eq!(RowKey::from_alias(alias).as_ref(), Some(&key), "{alias}");
            assert_eq!(key.alias(), alias);
        }
        assert_eq!(RowKey::from_alias("sounds.profile"), None);
        assert_eq!(RowKey::from_alias("nonsense"), None);
        let e = RowKey::with("storage.drive", InstanceId::Drive(4));
        assert_eq!(e.alias(), "storage.drive.E");
    }

    #[test]
    fn search_ranks_like_the_board() {
        let snap = project_all(&facts());
        // set2-search: the Date & time rows, Screen timeout, then the root's Date & time, then the page's other rows
        let t = titles(&snap, "time");
        assert_eq!(
            t[..6],
            [
                "Automatic time",
                "Automatic time zone",
                "Time zone",
                "24-hour time",
                "Screen timeout",
                "Date & time"
            ]
        );
        assert!(t.contains(&String::from("Date format")));
        // an exact title first (virt-taps' "apps" and "offline")
        assert_eq!(titles(&snap, "apps")[0], "Apps");
        assert_eq!(titles(&snap, "Offline")[0], "Offline");
        assert_eq!(titles(&snap, "SMS")[0], "Messages");
        // the languages' own names find their keyboards
        assert_eq!(titles(&snap, "Русский")[0], "Russian");
        assert_eq!(titles(&snap, "հայերեն")[0], "Armenian");
        assert!(search(&snap, "  ").is_empty());
        assert!(search(&snap, "").is_empty());
        // a value is found last; the root's Storage reads C:
        let v = search(&snap, "30 sec");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].key, RowKey::of("display.timeout"));
        assert_eq!(
            snap.get(&RowKey::of("root.storage")).unwrap().value_text,
            "C: 5.9 GB free"
        );
    }

    #[test]
    fn case_folds_unicode() {
        let mut f = facts();
        f.profiles.push(Profile {
            id: 5,
            name: "Über Дом".into(),
            ..f.profiles[0].clone()
        });
        let snap = project_all(&f);
        assert_eq!(titles(&snap, "üBER")[0], "Über Дом");
        assert_eq!(titles(&snap, "ДОМ")[0], "Über Дом");
    }

    #[test]
    fn actions_and_private_values_are_found_by_name_only() {
        let snap = project_all(&facts());
        let clip = snap.get(&RowKey::of("privacy.clip")).unwrap();
        assert_eq!(clip.reading, Reading::NoValue);
        // D8's toast switch is found by its names too, never by what was pasted
        assert_eq!(
            titles(&snap, "paste"),
            ["Clear clipboard", "Show when an app reads it"]
        );
        // an app row's caption is its title; it has no value
        let clock = RowKey::with("apps.app", InstanceId::App(AppId::Native("clock")));
        assert_eq!(snap.get(&clock).unwrap().value_text, "");
        // a disabled row says why
        let usb = snap.get(&RowKey::of("storage.usb")).unwrap();
        assert_eq!(usb.availability, Availability::Disabled("Not yet (FS6)"));
        // a credential or a typed draft is never a fact, so never found: the search covers the catalogue only
        assert!(search(&snap, "password").is_empty());
    }

    #[test]
    fn readings_are_honest() {
        let mut f = facts();
        // airplane mode over a previous technology
        f.scalars.insert("radio.airplane", Scalar::On(true));
        f.profiles[0].volume = 0;
        f.scalars.remove("storage.free");
        let snap = project_all(&f);
        let v = |k: RowKey| snap.get(&k).unwrap().reading.clone();
        assert_eq!(v(RowKey::of("root.mobile")), Reading::Value("Off".into()));
        // zero is a value, unknown is not
        assert_eq!(
            v(RowKey::with("profile.volume", InstanceId::Profile(0))),
            Reading::Value("0".into())
        );
        assert_eq!(
            v(RowKey::of("root.storage")),
            Reading::Unavailable("C: can't be read")
        );
        let images = RowKey::with("storage.kind", InstanceId::OverviewKind(Kind::Images));
        assert_eq!(v(images.clone()), Reading::Unavailable("Not counted yet"));
        f.kinds_state = KindsState::Counting;
        assert_eq!(
            project_all(&f).get(&images).unwrap().reading,
            Reading::Pending
        );
        f.kinds_state = KindsState::Counted("Last counted 06:50".into());
        f.kinds = vec![(Kind::Images, "6.2 GB".into())];
        let snap = project_all(&f);
        let row = snap.get(&images).unwrap();
        assert_eq!(
            (row.primary.as_str(), row.detail.as_str()),
            ("6.2 GB", "Last counted 06:50")
        );
        // the console row only when the kernel answered
        assert_eq!(
            snap.get(&RowKey::of("dev.console")).unwrap().availability,
            Availability::Hidden
        );
        assert!(search(&snap, "console overlay").is_empty());
    }

    #[test]
    fn profile_rows_are_per_profile() {
        let mut f = facts();
        f.profiles[2].volume = 3; // Meeting
        let snap = project_all(&f);
        let hits = search(&snap, "ring volume");
        // every profile that rings has its own (Silent and Offline don't show it)
        assert_eq!(hits.len(), 3);
        let meeting = RowKey::with("profile.volume", InstanceId::Profile(2));
        let row = snap.get(&meeting).unwrap();
        assert_eq!(
            (row.path.as_str(), row.value_text.as_str()),
            ("Meeting", "3")
        );
        let d = &row.destination;
        assert_eq!(
            (d.page, d.context.clone()),
            (Page::Profile, Some(InstanceId::Profile(2)))
        );
        assert_eq!(
            d.focus.as_ref().map(RowKey::alias).as_deref(),
            Some("profile.volume")
        );
    }

    #[test]
    fn a_hit_resolves_against_the_current_facts() {
        let mut f = facts();
        f.profiles.push(Profile {
            id: 5,
            name: "Gym".into(),
            ..f.profiles[0].clone()
        });
        let snap = project_all(&f);
        let hits = search(&snap, "gym");
        assert_eq!(
            resolve_hit(0, &hits, &snap).map(|d| d.page),
            Ok(Page::Sounds)
        );
        // the value changed on the same profile: it still opens
        f.profiles[5].volume = 1;
        assert!(resolve_hit(0, &hits, &project_all(&f)).is_ok());
        // deleted, and its id reused at once: the profiles epoch moved, so never the replacement
        f.profiles[5].name = "Run".into();
        f.epochs.profiles += 1;
        assert_eq!(resolve_hit(0, &hits, &project_all(&f)), Err(Gone));
        f.profiles.pop();
        assert_eq!(resolve_hit(0, &hits, &project_all(&f)), Err(Gone));
        assert_eq!(resolve_hit(9, &hits, &snap), Err(Gone));
    }

    #[test]
    fn back_returns_to_the_search_it_came_from() {
        let mut t = Trail::default();
        let key = RowKey::with("profile.volume", InstanceId::Profile(2));
        t.push(Frame::Search {
            query: "ring".into(),
            scroll: -120.0,
            selected: Some(key.clone()),
        });
        // the hit's page, then a page from it
        t.push(Frame::Page(Destination::page(Page::Profile)));
        assert_eq!(
            t.back(Page::Ringtone),
            Some(Frame::Page(Destination::page(Page::Profile)))
        );
        assert_eq!(
            t.back(Page::Profile),
            Some(Frame::Search {
                query: "ring".into(),
                scroll: -120.0,
                selected: Some(key)
            })
        );
        // nothing left: the parent, then out
        assert_eq!(
            t.back(Page::Ringtone),
            Some(Frame::Page(Destination::page(Page::Profile)))
        );
        assert_eq!(t.back(Page::Root), None);
        t.push(Frame::Page(Destination::page(Page::Apps)));
        t.clear();
        assert_eq!(
            t.back(Page::Wifi),
            Some(Frame::Page(Destination::page(Page::Root)))
        );
    }

    #[test]
    fn wifi_fields_follow_the_link() {
        let mut f = facts();
        let snap = project_all(&f);
        let hits = search(&snap, "access point");
        assert_eq!(hits.len(), 1);
        let d = resolve_hit(0, &hits, &snap).unwrap();
        assert_eq!((d.page, d.sheet), (Page::Wifi, Some(Sheet::WifiDetails)));
        // a disconnect: no fields, and the old token is gone even if the same BSSID comes back
        f.link = None;
        f.epochs.wifi += 1;
        assert!(search(&project_all(&f), "access point").is_empty());
        f.link = facts().link;
        assert_eq!(resolve_hit(0, &hits, &project_all(&f)), Err(Gone));
    }

    #[test]
    fn drives_and_apps_keep_their_identity() {
        let mut f = facts();
        // an old app captioned like a ROM app is another row
        f.apps.push(AppFact {
            id: AppId::Legacy(0x2000_9aa3),
            name: "Clock".into(),
        });
        let snap = project_all(&f);
        let clocks = search(&snap, "clock");
        let apps: Vec<_> = clocks
            .iter()
            .filter(|h| h.key.id == RowId("apps.app"))
            .map(|h| snap.rows[h.row].destination.clone())
            .collect();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].page, Page::AppDetail);
        assert_eq!(
            (apps[1].page, apps[1].sheet),
            (Page::Apps, Some(Sheet::App))
        );
        // C: and E: are two rows with their own values
        let e = snap
            .get(&RowKey::with("storage.drive", InstanceId::Drive(4)))
            .unwrap();
        let c = snap
            .get(&RowKey::with("storage.drive", InstanceId::Drive(2)))
            .unwrap();
        assert_ne!(e.value_text, c.value_text);
        assert_eq!(c.title, "C: Phone");
    }
}

// SPDX-License-Identifier: Apache-2.0
//! The registry: every system setting with its type, default, limit and owner, declared once in the
//! [`registry!`] table below. The table generates both the typed [`keys`] and [`REGISTRY`], and checks at compile
//! time that each default has the key's type, so the two cannot drift apart.
//!
//! **Adding a system setting:** add one row to the table. If the shell shows it, bind it in `neoui-render`'s
//! settings binding table too. Keys an app keeps under its own namespace (`clock.*`) need no row: they are allowed,
//! limited only by the value size limits, and only that app may write them.

use super::key::{Bytes, Key, Setting};
use super::{Value, ValueType};

/// Who may write a registered key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The UI server only (system settings shown in Settings and the shade).
    Shell,
    /// The app with this key (`"clock"`), and the UI server.
    App(&'static str),
    /// Any of these apps, and the UI server: a key apps share (Phone and Contacts: speed dial, the name order).
    Apps(&'static [&'static str]),
    /// Every app: a system-wide choice any app may make for the user (`open.always`).
    Any,
}

/// What values a registered key accepts beyond its type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    None,
    /// An `Int` within `min..=max`.
    Range {
        min: i32,
        max: i32,
    },
    /// A `Str` or `Bytes` of at most this many bytes (tighter than the type-wide limit).
    MaxLen(usize),
    /// (TP4) An `n.<app>` notification policy (`notification_policy::Policy`): one validator for SET, load and reset.
    Policy,
    /// (TP7) A tile order (`tiles::decode_order`): at most 32 valid ids, none twice.
    Tiles,
    /// (TP6) A list of app keys (`activity::decode_deny`: `s16` entries, each 1–16 bytes of UTF-8), at most this many
    /// bytes.
    AppKeys(usize),
}

/// A default value, in a form that can be written in a `const` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultValue {
    Bool(bool),
    Int(i32),
    Str(&'static str),
    Bytes(&'static [u8]),
}

impl DefaultValue {
    #[must_use]
    pub const fn ty(&self) -> ValueType {
        match self {
            DefaultValue::Bool(_) => ValueType::Bool,
            DefaultValue::Int(_) => ValueType::Int,
            DefaultValue::Str(_) => ValueType::Str,
            DefaultValue::Bytes(_) => ValueType::Bytes,
        }
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        match *self {
            DefaultValue::Bool(v) => Value::Bool(v),
            DefaultValue::Int(v) => Value::Int(v),
            DefaultValue::Str(s) => Value::Str(s.into()),
            DefaultValue::Bytes(b) => Value::Bytes(b.to_vec()),
        }
    }

    /// Whether `v` equals this default (without allocating).
    #[must_use]
    pub fn matches(&self, v: &Value) -> bool {
        match (self, v) {
            (DefaultValue::Bool(a), Value::Bool(b)) => a == b,
            (DefaultValue::Int(a), Value::Int(b)) => a == b,
            (DefaultValue::Str(a), Value::Str(b)) => *a == b.as_str(),
            (DefaultValue::Bytes(a), Value::Bytes(b)) => *a == b.as_slice(),
            _ => false,
        }
    }
}

/// A registered setting.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub name: &'static str,
    pub default: DefaultValue,
    pub limit: Limit,
    pub owner: Owner,
}

impl Spec {
    #[must_use]
    pub const fn ty(&self) -> ValueType {
        self.default.ty()
    }

    /// Whether `v` fits this key's type and limit.
    ///
    /// # Errors
    /// [`super::Error::WrongType`], [`super::Error::OutOfRange`] or [`super::Error::TooLong`].
    pub fn check(&self, v: &Value) -> Result<(), super::Error> {
        if v.ty() != self.ty() {
            return Err(super::Error::WrongType);
        }
        match (self.limit, v) {
            (Limit::Range { min, max }, Value::Int(i)) if !(min..=max).contains(i) => {
                Err(super::Error::OutOfRange)
            }
            (Limit::MaxLen(n), Value::Str(s)) if s.len() > n => Err(super::Error::TooLong),
            (Limit::MaxLen(n), Value::Bytes(b)) if b.len() > n => Err(super::Error::TooLong),
            (Limit::Policy, Value::Bytes(b))
                if crate::notification_policy::Policy::decode(b).is_none() =>
            {
                Err(super::Error::OutOfRange)
            }
            (Limit::Tiles, Value::Bytes(b)) if crate::tiles::decode_order(b).is_none() => {
                Err(super::Error::OutOfRange)
            }
            (Limit::AppKeys(n), Value::Bytes(b)) if b.len() > n => Err(super::Error::TooLong),
            (Limit::AppKeys(_), Value::Bytes(b)) if crate::activity::decode_deny(b).is_none() => {
                Err(super::Error::OutOfRange)
            }
            _ => Ok(()),
        }
    }
}

/// Shell keys that Settings → Reset settings (`ipc::RESET_SETTINGS`, 1.17) keeps:
/// - the sound profiles with their live values: the profiles are the user's own, and resetting a mirror alone would
///   be folded back into the active profile;
/// - the radio switches (airplane mode, Wi-Fi, Bluetooth, data), which aren't preferences;
/// - the old apps' pad epoch, a counter;
/// - the apps chosen to open each file type (`open.always`), which belong with the apps;
/// - home's pages and dock (`home.layout`, D10) and the control centre's tiles (`shell.tiles`, TP7), the user's own
///   arrangements;
/// - whether a lock PIN is set (`lock.pin_set`, TP9): the PIN has its own flow, and its verifier stays.
pub const RESET_KEEPS: &[&str] = &[
    "sound.profile",
    "sound.profiles",
    "sound.tones",
    "sound.feedback",
    "sound.your_tones",
    "sound.tones_yours",
    "sound.keypad_tones",
    "sound.touch_vibrate",
    "sound.silent",
    "sound.vibrate",
    "sound.ring_volume",
    "sound.ringtone",
    "sound.message_tone",
    "radio.wifi",
    "radio.bt",
    "radio.data",
    "radio.airplane",
    "legacy.pad.epoch",
    "open.always",
    "home.layout",
    "shell.tiles",
    "lock.pin_set",
];

/// The keys Reset settings puts back to their defaults: every key no app owns, but [`RESET_KEEPS`]. Apps' keys
/// (Clock's alarms, Phone's speed dial, `contacts.migrated`) are never in it.
pub fn reset_by_default() -> impl Iterator<Item = &'static Spec> {
    REGISTRY
        .iter()
        .filter(|s| matches!(s.owner, Owner::Shell | Owner::Any) && !RESET_KEEPS.contains(&s.name))
}

/// What an app's Clear data resets (`ipc::AppEvent::ClearData`, 1.17): every key that app alone owns, but the ones the
/// Settings app shows as the phone's (`access::SETTINGS_ALSO`: Clock's 24-hour time is Date & time's).
pub fn cleared_with(app: &str) -> impl Iterator<Item = &'static Spec> + '_ {
    REGISTRY.iter().filter(move |s| {
        matches!(s.owner, Owner::App(owner) if owner == app)
            && !super::access::SETTINGS_ALSO.contains(&s.name)
    })
}

/// The registered spec for `name`, if any.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static Spec> {
    REGISTRY.iter().find(|s| s.name == name)
}

/// The spec of a typed key, if registered.
#[must_use]
pub fn spec_of<T: Setting>(key: Key<T>) -> Option<&'static Spec> {
    lookup(key.name())
}

macro_rules! registry {
    ($(
        $(#[$doc:meta])*
        $konst:ident: $ty:ty = $name:literal, default $default:expr, limit $limit:expr, owner $owner:expr;
    )*) => {
        /// Typed handles to every registered setting.
        pub mod keys {
            #[allow(unused_imports)]
            use super::super::key::{Bytes, Key};
            $(
                $(#[$doc])*
                pub const $konst: Key<$ty> = Key::new($name);
            )*
        }

        /// Every registered setting.
        pub static REGISTRY: &[Spec] = &[$(
            Spec { name: $name, default: $default, limit: $limit, owner: $owner },
        )*];

        // Each default has its key's type.
        $(const _: () = assert!(
            $default.ty() as u8 == <$ty as Setting>::TYPE as u8,
            concat!("default of ", $name, " does not match its type"),
        );)*
    };
}

registry! {
    /// Wi-Fi on: off, SYSSTART neither scans nor rejoins on its own (the kernel keeps the radio up for now).
    RADIO_WIFI: bool = "radio.wifi", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// Bluetooth radio on. UI only until there is a driver.
    RADIO_BT: bool = "radio.bt", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// Mobile data on.
    RADIO_DATA: bool = "radio.data", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// Airplane mode: every radio off.
    RADIO_AIRPLANE: bool = "radio.airplane", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// Silent: no ringer or notification sounds.
    SOUND_SILENT: bool = "sound.silent", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// Vibrate for calls and notifications.
    SOUND_VIBRATE: bool = "sound.vibrate", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// C7 built-in ringtone volume, matching NeoModem's 0..=10 steps.
    SOUND_RING_VOLUME: i32 = "sound.ring_volume", default DefaultValue::Int(8),
        limit Limit::Range { min: 0, max: 10 }, owner Owner::Shell;
    /// (SE2b) The active profile's ringtone and message tone (`sound_profile::tone`: 0 built-in, 1 beep, 2 two beeps,
    /// 3 none; D6: 4..=15 the user's own sounds, `sound.your_tones`): live values the shell sends MODEMD, written by the
    /// profiles' reducer.
    SOUND_RINGTONE: i32 = "sound.ringtone", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 15 }, owner Owner::Shell;
    SOUND_MESSAGE_TONE: i32 = "sound.message_tone", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 15 }, owner Owner::Shell;
    /// (D6, SE2c-1) The user's own sounds (`sound_profile::Profiles::encode_yours`): `[id][len][name]`, ids 4..=15;
    /// their PCM is NEOSET's `tones\<id>.pcm`. Written only by the shell's reducer.
    /// (D6) The profiles' tones that are the user's own sounds (`sound_profile::Profiles::encode_tones_yours`), read
    /// over `sound.tones` (which keeps their built-in fallback for an older shell). Written only by the reducer.
    SOUND_TONES_YOURS: Bytes = "sound.tones_yours", default DefaultValue::Bytes(&[]),
        limit Limit::MaxLen(32), owner Owner::Shell;
    SOUND_YOUR_TONES: Bytes = "sound.your_tones", default DefaultValue::Bytes(&[]),
        limit Limit::MaxLen(512), owner Owner::Shell;
    /// (SE2b) The profiles' tones (`sound_profile::Profiles::encode_tones`): a key of its own beside
    /// `sound.profiles`, so a shell from before SE2b keeps the profiles. Written only by the shell's reducer.
    SOUND_TONES: Bytes = "sound.tones", default DefaultValue::Bytes(&[]),
        limit Limit::MaxLen(32), owner Owner::Shell;
    /// (D6) The profiles' keypad tones and vibrate on touch (`sound_profile::Profiles::encode_feedback`), a sidecar
    /// beside `sound.profiles` (which stays version 1). Written only by the shell's reducer.
    SOUND_FEEDBACK: Bytes = "sound.feedback", default DefaultValue::Bytes(&[]),
        limit Limit::MaxLen(34), owner Owner::Shell;
    /// (D6) The active profile's keypad tones: an accepted dial key out of a call plays its tone. A live value the
    /// reducer writes; the system keyboard's pulse is `keyboard.vibrate`, in-call DTMF doesn't depend on it.
    SOUND_KEYPAD_TONES: bool = "sound.keypad_tones", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// (D6) The active profile's vibrate on touch: one short pulse for each control a touch works. Written by the
    /// reducer.
    SOUND_TOUCH_VIBRATE: bool = "sound.touch_vibrate", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// The active sound profile (`sound_profile`: 0 General, 1 Silent, 2 Meeting, 3 Outdoor, 4 Offline, 5.. added).
    /// Written only by the shell's reducer; `sound.silent/vibrate/ring_volume` are its live values.
    SOUND_PROFILE: i32 = "sound.profile", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 254 }, owner Owner::Shell;
    /// The profiles (`sound_profile::Profiles::encode`): built-in overrides, added profiles, what Silent and Offline
    /// remember. Written only by the shell's reducer.
    SOUND_PROFILES: Bytes = "sound.profiles", default DefaultValue::Bytes(&[]),
        limit Limit::MaxLen(1024), owner Owner::Shell;
    /// Media volume (NeoAudio's amp volume, docs/hardware/audio.md): 0..=10; the volume keys outside a call.
    SOUND_MEDIA_VOLUME: i32 = "sound.media_volume", default DefaultValue::Int(7),
        limit Limit::Range { min: 0, max: 10 }, owner Owner::Shell;
    /// Where media plays: 0 both speakers, 1 the earpiece, 2 the bottom speaker (NeoAudio's route).
    SOUND_MEDIA_ROUTE: i32 = "sound.media_route", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 2 }, owner Owner::Shell;
    /// The microphone recordings use: 0 the main one (calls use it), 1 the second (NeoAudio's MIC).
    SOUND_MIC: i32 = "sound.mic", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 1 }, owner Owner::Shell;
    /// Dark theme.
    DISPLAY_DARK: bool = "display.dark", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// Screen brightness, 0 (dimmest) to 255; the UI server sends it to the panel (display HAL, 0..1023).
    DISPLAY_BRIGHTNESS: i32 = "display.brightness", default DefaultValue::Int(160),
        limit Limit::Range { min: 0, max: 255 }, owner Owner::Shell;
    /// Seconds without a touch before the screen turns off; 0 = never.
    DISPLAY_TIMEOUT: i32 = "display.timeout", default DefaultValue::Int(60),
        limit Limit::Range { min: 0, max: 600 }, owner Owner::Shell;
    /// (D7) Home's wallpaper: a colour of `settings_app::choices::WALLPAPERS` by its name ("night", "black", …);
    /// the shell draws an unknown name as Night.
    DISPLAY_WALLPAPER: alloc::string::String = "display.wallpaper", default DefaultValue::Str("night"),
        limit Limit::MaxLen(16), owner Owner::Shell;
    /// (D7) The text size: 0 Small 87.5 %, 1 Medium, 2 Large 112.5 %, 3 Largest 125 % (`choices::FONT_SIZES`); the
    /// kit's text tokens scale with it (ui/theme.slint), the status bar, clock digits and key faces don't.
    DISPLAY_FONT_SIZE: i32 = "display.font_size", default DefaultValue::Int(1),
        limit Limit::Range { min: 0, max: 3 }, owner Owner::Shell;
    /// (D8) A toast names the app each time one reads the clipboard (`ipc::CLIP_GET`, nonempty).
    PRIVACY_CLIP_NOTICE: bool = "privacy.clip_notice", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::Shell;
    /// Settings -> Battery (SE3a): the status bar's battery shows its percent (off: the level's fill only).
    BATTERY_SHOW_PERCENT: bool = "battery.show_percent", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::Shell;
    /// Settings -> Battery (SE3a): Battery saver caps the panel's brightness (`effects::SAVER_CAP_PERCENT`) without
    /// moving the brightness setting (`effects::panel_level`). Switched by hand only.
    BATTERY_SAVER: bool = "battery.saver", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// Settings -> Home screen & navigation (Navigation v2, D12 and D19): 0 Buttons (the key row ← ⌂ ▢, no edge
    /// gestures), 1 Gestures (the Thumb map: Back from either side, up for Home, up and pause for Open apps, along the
    /// band for the previous / next app), 2 N9 gestures (left Back, right Open apps, a right flick for the last app).
    /// A stored 1 (v1's Gestures) becomes the Thumb map.
    NAV_MODE: i32 = "nav.mode", default DefaultValue::Int(1), limit Limit::Range { min: 0, max: 2 }, owner Owner::Shell;
    /// (D10, G3) Open apps' columns: 2 or 3 cards to a row (their thumbnails scale with them).
    NAV_SWITCHER_COLS: i32 = "nav.switcher_cols", default DefaultValue::Int(2),
        limit Limit::Range { min: 2, max: 3 }, owner Owner::Shell;
    /// (D10, G4) In Gestures and N9 gestures, a drag down from the status bar over an app closes it past half the
    /// screen (a resident app goes home and keeps running); the shade then opens by a tap on the bar.
    NAV_SWIPE_CLOSE: bool = "nav.swipe_close", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// (D10, G5) Home: 0 Pages (Belle's widget pages), 1 Lanes (N9: Events, Home, Open apps).
    HOME_STYLE: i32 = "home.style", default DefaultValue::Int(0), limit Limit::Range { min: 0, max: 1 },
        owner Owner::Shell;
    /// (D10) Home's pages and dock (`home::Layout`, version byte first), written by the shell's home editor only;
    /// empty or damaged = the default layout.
    HOME_LAYOUT: Bytes = "home.layout", default DefaultValue::Bytes(&[]), limit Limit::MaxLen(2048),
        owner Owner::Shell;
    /// Settings -> Date & time: the network (the modem's NITZ, else an HTTP Date) sets the clock.
    TIME_AUTO: bool = "time.auto", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// The mobile network's time zone (NITZ) sets the offset; off, the Time zone row picks it.
    TIME_AUTO_ZONE: bool = "time.auto_zone", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// The time zone, a city of `clock::world::CATALOGUE`: its rule moves the clock in and out of summer time while
    /// "Automatic time zone" is off. Empty: a fixed offset (as `clock.dat` keeps it).
    TIME_ZONE: alloc::string::String = "time.zone", default DefaultValue::Str(""), limit Limit::MaxLen(32), owner Owner::Shell;
    /// The shade's date format: an index into `clock::datetime::DATE_FORMATS`.
    TIME_DATE_FORMAT: i32 = "time.date_format", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 3 }, owner Owner::Shell;
    /// The status bar's time shows the seconds.
    STATUS_SECONDS: bool = "status.seconds", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// The shade shows the date beside the time.
    SHADE_DATE: bool = "shade.date", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    // a swipe down on home opens the shade (top-panel.md TP1; Settings → Panel & notifications, TP4)
    SHELL_SWIPEHOME: bool = "shell.swipehome", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// (TP7) What a tap on the status bar opens: 0 the shade, 1 the control centre.
    SHELL_TAP: i32 = "shell.tap", default DefaultValue::Int(0), limit Limit::Range { min: 0, max: 1 },
        owner Owner::Shell;
    /// (TP3) The media player in the shade.
    SHELL_MEDIA: bool = "shell.media", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// (TP7) The control centre's placed tiles in order (`tiles::encode_order`: LE u16 ids, high bit wide; empty = the
    /// default order, so the editor keeps at least one tile). Only the shell's editor writes it (`access::SHELL_ONLY`; Settings opens it with
    /// `ipc::ACTION_EDIT_TILES`).
    SHELL_TILES: Bytes = "shell.tiles", default DefaultValue::Bytes(&[]), limit Limit::Tiles, owner Owner::Shell;
    /// (TP6) Show live activities in the island.
    ISLAND_ON: bool = "island.on", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// (TP6) The apps turned off for the island (`activity::encode_deny`: `s16` app keys).
    ISLAND_DENY: Bytes = "island.deny", default DefaultValue::Bytes(&[]), limit Limit::AppKeys(16 * 17),
        owner Owner::Shell;
    /// (TP8) Show the lock screen when the screen comes on.
    LOCK_ON: bool = "lock.on", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// (TP9) A lock PIN is set. Only SYSSTART writes it (`access::SHELL_ONLY`), after its own PIN dialog; the salted
    /// verifier is in `C:\private\e0002001\`, never in the store.
    LOCK_PIN_SET: bool = "lock.pin_set", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// (SE5, O2) The display language: `en`, `hy` or `ru` (another value shows English), independent of the keyboard.
    UI_LANGUAGE: alloc::string::String = "ui.language", default DefaultValue::Str("en"), limit Limit::MaxLen(8),
        owner Owner::Shell;
    /// (O5) The Power menu: 0 the full-screen sheet (v2), 1 the short list.
    POWER_STYLE: i32 = "power.style", default DefaultValue::Int(0), limit Limit::Range { min: 0, max: 1 },
        owner Owner::Shell;
    /// (TP4) Do not disturb: no banners, sounds or vibrations from the shell; cards still reach the shade. Calls and
    /// SMS tones are MODEMD's and still ring (NeoModem's alert policy is later).
    NOTIFY_DND: bool = "notify.dnd", default DefaultValue::Bool(false), limit Limit::None, owner Owner::Shell;
    /// (TP4) When Do not disturb ends: empty (or 0) until turned off, else 8 bytes, a LE i64 UTC second; the shell
    /// turns `notify.dnd` off once it passes.
    NOTIFY_DNDTO: Bytes = "notify.dndto", default DefaultValue::Bytes(&[]), limit Limit::MaxLen(8), owner Owner::Shell;
    /// (TP4) Calls from favourites and repeated callers ring through Do not disturb (needs NeoModem's alert policy).
    NOTIFY_DNDFAV: bool = "notify.dndfav", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// (TP4) How long a banner stays, in seconds.
    NOTIFY_BANNERT: i32 = "notify.bannert", default DefaultValue::Int(5),
        limit Limit::Range { min: 1, max: 30 }, owner Owner::Shell;
    /// (TP8) Cards on the lock screen: 0 show, 1 private, 2 off.
    NOTIFY_LOCK: i32 = "notify.lock", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 2 }, owner Owner::Shell;
    /// (TP4) Each publisher's notification policy (`notification_policy::Policy`; empty = the defaults). Shell-owned:
    /// the app can't change what the user chose; Settings writes them.
    N_PHONE: Bytes = "n.phone", default DefaultValue::Bytes(&[]), limit Limit::Policy, owner Owner::Shell;
    N_MESSAGES: Bytes = "n.messages", default DefaultValue::Bytes(&[]), limit Limit::Policy, owner Owner::Shell;
    N_DTSRV: Bytes = "n.dtsrv", default DefaultValue::Bytes(&[]), limit Limit::Policy, owner Owner::Shell;
    N_FILES: Bytes = "n.files", default DefaultValue::Bytes(&[]), limit Limit::Policy, owner Owner::Shell;
    N_BROWSER: Bytes = "n.browser", default DefaultValue::Bytes(&[]), limit Limit::Policy, owner Owner::Shell;
    N_DEVTOOLS: Bytes = "n.devtools", default DefaultValue::Bytes(&[]), limit Limit::Policy, owner Owner::Shell;
    N_TERMINAL: Bytes = "n.terminal", default DefaultValue::Bytes(&[]), limit Limit::Policy, owner Owner::Shell;
    N_KIT: Bytes = "n.kit", default DefaultValue::Bytes(&[]), limit Limit::Policy, owner Owner::Shell;
    /// 24-hour clock (else 12-hour).
    CLOCK_24H: bool = "clock.24h", default DefaultValue::Bool(true), limit Limit::None, owner Owner::App("clock");
    /// USB debugging (Developer options): fshell's USB serial port and the shell on it (`phone.py sh`), like
    /// Android's adb. The UI server turns the port on (kernel HAL 0x4e04) and supervises fshell.
    DEVELOPER_USB_DEBUGGING: bool = "developer.usb_debugging", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// Developer options -> Show touches (SE3a): the shell draws a ring under each finger, over everything; drawing
    /// only, input goes where it went.
    DEVELOPER_SHOW_TOUCHES: bool = "developer.show_touches", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// The system keyboard has English (Settings -> Language & keyboard; the globe key cycles the languages on).
    KEYBOARD_EN: bool = "keyboard.en", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// The system keyboard has Armenian (phonetic).
    KEYBOARD_HY: bool = "keyboard.hy", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// The system keyboard has Russian (ЙЦУКЕН).
    KEYBOARD_RU: bool = "keyboard.ru", default DefaultValue::Bool(true), limit Limit::None, owner Owner::Shell;
    /// The system keyboard's language: an index into ui/components/keyboard/layouts.slint's list (append only).
    KEYBOARD_LAYOUT: i32 = "keyboard.layout", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 15 }, owner Owner::Shell;
    /// Every keyboard language as tall as the tallest one on (shorter ones get a digit row); else each its own height.
    KEYBOARD_SAME_HEIGHT: bool = "keyboard.same_height", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::Shell;
    /// Keypress vibration (SE3a): a short pulse on each key of the system keyboard (`effects::key_pulse`), as the old
    /// apps' pads give.
    KEYBOARD_VIBRATE: bool = "keyboard.vibrate", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// Settings -> Old apps (the key pads LEGACY draws under an old app; NeoSymbian/legacy host/pad.cpp): a short
    /// vibration on each key.
    LEGACY_PAD_VIBRATE: bool = "legacy.pad.vibrate", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::Shell;
    /// The key vibration's strength: 0 light, 1 medium, 2 strong.
    LEGACY_PAD_STRENGTH: i32 = "legacy.pad.strength", default DefaultValue::Int(1),
        limit Limit::Range { min: 0, max: 2 }, owner Owner::Shell;
    /// The pad an app gets on its first open: 0 auto (games the Gamepad, others the Navigator), 1 Navigator,
    /// 2 Gamepad, 3 Number. Each app then remembers its own (`legacy.<uid>.pad`, written by LEGACY).
    LEGACY_PAD_DEFAULT: i32 = "legacy.pad.default", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 3 }, owner Owner::Shell;
    /// The Gamepad's d-pad sends 0 the arrows (a diagonal holds two), 1 the number keys 1-9.
    LEGACY_PAD_DIRS: i32 = "legacy.pad.dirs", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 1 }, owner Owner::Shell;
    /// The Gamepad's A button sends 0 OK (select), 1 the 5 key.
    LEGACY_PAD_A: i32 = "legacy.pad.a", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 1 }, owner Owner::Shell;
    /// The Gamepad's B button sends 0 the right softkey, 1 the 0 key, 2 the # key.
    LEGACY_PAD_B: i32 = "legacy.pad.b", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 2 }, owner Owner::Shell;
    /// Left-handed Gamepad: the d-pad on the right, A and B on the left.
    LEGACY_PAD_LEFTY: bool = "legacy.pad.lefty", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// A game's first open is in the landscape console (the Gamepad beside the app).
    LEGACY_PAD_CONSOLE: bool = "legacy.pad.console", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Shell;
    /// "Reset remembered pads" adds one: LEGACY ignores the per-app pads saved under an older value.
    LEGACY_PAD_EPOCH: i32 = "legacy.pad.epoch", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: i32::MAX }, owner Owner::Shell;
    /// Settings -> Phone (telephony C8): caller ID on our calls: 0 set by the network, 1 hidden, 2 shown (MODEMD
    /// prefixes `#31#`/`*31#`).
    PHONE_CLIR: i32 = "phone.clir", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 2 }, owner Owner::App("phone");
    /// The call screen counts the connected time.
    PHONE_SHOW_DURATION: bool = "phone.show_duration", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::App("phone");
    /// A note with the call's duration when it ends.
    PHONE_SUMMARY: bool = "phone.summary", default DefaultValue::Bool(true), limit Limit::None, owner Owner::App("phone");
    /// How many days the call log keeps a call: 0 (no log), 1, 10 or 30.
    PHONE_LOG_DAYS: i32 = "phone.log_days", default DefaultValue::Int(30),
        limit Limit::Range { min: 0, max: 30 }, owner Owner::App("phone");
    /// The dialer offers contacts whose names the typed digits spell ("Contact search").
    PHONE_SMART_DIAL: bool = "phone.smart_dial", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::App("phone");
    /// The voice mailbox number (key 1 held in the dialer); typed by hand until it is read from the SIM.
    PHONE_VOICEMAIL: alloc::string::String = "phone.voicemail", default DefaultValue::Str(""),
        limit Limit::MaxLen(48), owner Owner::App("phone");
    /// Speed dial: the numbers of keys 2..=9, '\n'-separated (empty = unassigned).
    PHONE_SPEED: Bytes = "phone.speed", default DefaultValue::Bytes(&[]), limit Limit::MaxLen(400),
        owner Owner::Apps(&["phone", "contacts"]);
    /// The newest call-log entry the user has seen in Recents: missed calls after it count as new.
    PHONE_MISSED_SEEN: i32 = "phone.missed_seen", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: i32::MAX }, owner Owner::App("phone");
    /// The Phone app (NEOPHONE, its own process) owns `phone.*` and `contacts.*`; the shell still reads them.
    /// Contacts show names "First Last" (0) or "Last First" (1).
    CONTACTS_NAME_ORDER: i32 = "contacts.name_order", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 1 }, owner Owner::Apps(&["phone", "contacts"]);
    /// The number type a new contact's first number gets (phone_core's label list: 0 Mobile).
    CONTACTS_DEFAULT_LABEL: i32 = "contacts.default_label", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 15 }, owner Owner::Apps(&["phone", "contacts"]);
    /// The Phone app has copied the shell's contacts file (C:\private\e0002001\contacts.dat) into its own.
    CONTACTS_MIGRATED: bool = "contacts.migrated", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::Apps(&["phone", "contacts"]);
    /// The Clock app's face: 0 digital, 1 analog (Clock settings).
    CLOCK_FACE: i32 = "clock.face", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 1 }, owner Owner::App("clock");
    /// The Clock app shows the seconds (the seconds hand on the analog face).
    CLOCK_SECONDS: bool = "clock.seconds", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::App("clock");
    /// The Clock app shows the date under the face.
    CLOCK_DATE: bool = "clock.date", default DefaultValue::Bool(true), limit Limit::None, owner Owner::App("clock");
    /// The Clock app's open page: 0 Clock, 1 Alarms, 2 World, 3 Timer, 4 Stopwatch.
    CLOCK_TAB: i32 = "clock.tab", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 4 }, owner Owner::App("clock");
    /// The alarms as the Clock app kept them before DTSRV (NeoSymbian/datetime `datetime-core::alarm::Alarms` blob;
    /// empty = none): DTSRV imports them into its own store once, then deletes them.
    CLOCK_ALARMS: Bytes = "clock.alarms", default DefaultValue::Bytes(&[]), limit Limit::None,
        owner Owner::Apps(&["clock", "dtsrv"]);
    /// The world clocks (`clock::world::Cities` blob; empty = the default cities).
    CLOCK_CITIES: Bytes = "clock.cities", default DefaultValue::Bytes(&[]), limit Limit::None, owner Owner::App("clock");
    /// The countdown timer as the Clock app kept it before DTSRV (`datetime-core::timer::Timer` blob; empty = none):
    /// imported and deleted like `clock.alarms`.
    CLOCK_TIMER: Bytes = "clock.timer", default DefaultValue::Bytes(&[]), limit Limit::None,
        owner Owner::Apps(&["clock", "dtsrv"]);
    /// The stopwatch (`datetime-core::stopwatch::Stopwatch` blob; empty = reset).
    CLOCK_STOPWATCH: Bytes = "clock.stopwatch", default DefaultValue::Bytes(&[]), limit Limit::None,
        owner Owner::App("clock");
    /// Snooze puts a ringing alarm off this many minutes (Clock settings; DTSRV reads it).
    CLOCK_SNOOZE: i32 = "clock.snooze", default DefaultValue::Int(5), limit Limit::Range { min: 1, max: 60 },
        owner Owner::App("clock");
    /// How many times an alarm can be snoozed before it stops ringing.
    CLOCK_SNOOZE_COUNT: i32 = "clock.snooze_count", default DefaultValue::Int(3),
        limit Limit::Range { min: 0, max: 10 }, owner Owner::App("clock");
    /// The alarm sound's name (DTSRV's built-in tones).
    CLOCK_SOUND: alloc::string::String = "clock.sound", default DefaultValue::Str("Dawn"), limit Limit::MaxLen(32),
        owner Owner::App("clock");
    /// Alarms start quietly and get louder.
    CLOCK_RISE: bool = "clock.rise", default DefaultValue::Bool(false), limit Limit::None, owner Owner::App("clock");
    /// The shade shows the next alarm ahead of time.
    CLOCK_SHADE_NEXT: bool = "clock.shade_next", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::App("clock");
    /// The Calendar app's view: 0 Month, 1 Week, 2 Day, 3 To-do.
    CALENDAR_VIEW: i32 = "calendar.view", default DefaultValue::Int(0), limit Limit::Range { min: 0, max: 3 },
        owner Owner::App("calendar");
    /// The first day of the week: 0 Monday .. 6 Sunday.
    CALENDAR_WEEK_START: i32 = "calendar.week_start", default DefaultValue::Int(0),
        limit Limit::Range { min: 0, max: 6 }, owner Owner::App("calendar");
    /// Week numbers in the month and week views.
    CALENDAR_WEEK_NUMBERS: bool = "calendar.week_numbers", default DefaultValue::Bool(false), limit Limit::None,
        owner Owner::App("calendar");
    /// A new event's reminder, in minutes before it starts.
    CALENDAR_REMINDER: i32 = "calendar.reminder", default DefaultValue::Int(15),
        limit Limit::Range { min: 0, max: 65_535 }, owner Owner::App("calendar");
    /// A new event's length, in minutes.
    CALENDAR_LENGTH: i32 = "calendar.length", default DefaultValue::Int(60),
        limit Limit::Range { min: 0, max: 65_535 }, owner Owner::App("calendar");
    /// Reminders ring like an alarm (else a shade entry only); DTSRV reads it.
    CALENDAR_RING: bool = "calendar.ring", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::App("calendar");
    /// Contacts' birthdays show in the calendar; DTSRV reads it.
    CALENDAR_BIRTHDAYS: bool = "calendar.birthdays", default DefaultValue::Bool(true), limit Limit::None,
        owner Owner::App("calendar");
    /// The calendars hidden from the views (their ids; empty = all shown).
    CALENDAR_HIDDEN: Bytes = "calendar.hidden", default DefaultValue::Bytes(&[]), limit Limit::None,
        owner Owner::App("calendar");
    /// The next alarm and the next event, published by DTSRV for the home clock widget and the agenda card
    /// (`clock::next::Next` blob; empty = nothing next, or DTSRV not running).
    DTSRV_NEXT: Bytes = "dtsrv.next", default DefaultValue::Bytes(&[]), limit Limit::None, owner Owner::App("dtsrv");
    /// The app that always opens a file type, chosen in an Open with sheet's "Always" (`crate::open::always`:
    /// `ext=key;` entries).
    OPEN_ALWAYS: Bytes = "open.always", default DefaultValue::Bytes(&[]), limit Limit::None, owner Owner::Any;
}

#[cfg(test)]
mod tests {
    #[test]
    fn clocks_clear_data_is_its_own_keys_but_24_hour_time() {
        let names: alloc::vec::Vec<&str> = cleared_with("clock").map(|s| s.name).collect();
        for k in [
            "clock.stopwatch",
            "clock.cities",
            "clock.face",
            "clock.snooze",
            "clock.shade_next",
        ] {
            assert!(names.contains(&k), "{k}");
        }
        assert!(!names.contains(&"clock.24h"));
        assert!(names.iter().all(|n| n.starts_with("clock.")));
        // the alarms and the timer are DTSRV's now (Clock's Clear data asks it), and so is what it publishes
        assert!(!names.contains(&"clock.alarms") && !names.contains(&"clock.timer"));
        assert!(!names.contains(&"dtsrv.next"));
        let calendar: alloc::vec::Vec<&str> = cleared_with("calendar").map(|s| s.name).collect();
        assert_eq!(calendar.len(), 8);
        assert!(calendar.iter().all(|n| n.starts_with("calendar.")));
        assert_eq!(
            cleared_with("dtsrv")
                .map(|s| s.name)
                .collect::<alloc::vec::Vec<_>>(),
            ["dtsrv.next"]
        );
        // shared keys are no one app's to clear
        assert!(cleared_with("contacts").next().is_none());
    }

    use super::*;
    use crate::settings::key::is_valid_name;

    #[test]
    fn names_are_valid_and_unique() {
        for (i, s) in REGISTRY.iter().enumerate() {
            assert!(is_valid_name(s.name), "{}", s.name);
            assert!(
                REGISTRY[i + 1..].iter().all(|t| t.name != s.name),
                "duplicate {}",
                s.name
            );
        }
    }

    #[test]
    fn defaults_satisfy_their_own_limits() {
        for s in REGISTRY {
            assert_eq!(s.check(&s.default.to_value()), Ok(()), "{}", s.name);
            assert!(s.default.matches(&s.default.to_value()), "{}", s.name);
        }
    }

    #[test]
    fn the_1_22_keys() {
        for k in [
            "shell.tap",
            "shell.media",
            "shell.tiles",
            "island.on",
            "island.deny",
            "lock.on",
            "lock.pin_set",
            "ui.language",
            "power.style",
        ] {
            assert_eq!(lookup(k).map(|s| s.owner), Some(Owner::Shell), "{k}");
        }
        let tiles = spec_of(keys::SHELL_TILES).unwrap();
        assert_eq!(
            tiles.check(&Value::Bytes(
                crate::tiles::encode_order(&[(1, true), (3, false)]).unwrap()
            )),
            Ok(())
        );
        assert_eq!(
            tiles.check(&Value::Bytes(alloc::vec![1, 0, 1, 0])),
            Err(super::super::Error::OutOfRange)
        );
        assert_eq!(
            spec_of(keys::SHELL_TAP).unwrap().check(&Value::Int(2)),
            Err(super::super::Error::OutOfRange)
        );
        assert!(RESET_KEEPS.contains(&"lock.pin_set") && RESET_KEEPS.contains(&"shell.tiles"));
    }

    #[test]
    fn typed_keys_resolve_to_their_spec() {
        let spec = spec_of(keys::DISPLAY_BRIGHTNESS).expect("registered");
        assert_eq!(spec.ty(), ValueType::Int);
        assert_eq!(
            spec.check(&Value::Int(256)),
            Err(super::super::Error::OutOfRange)
        );
        assert_eq!(
            spec.check(&Value::Bool(true)),
            Err(super::super::Error::WrongType)
        );
        assert!(lookup("radio.nothing").is_none());
    }
}

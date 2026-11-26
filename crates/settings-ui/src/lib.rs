// SPDX-License-Identifier: GPL-3.0-only
//! The Settings app's screens (`NEOSET.EXE`, docs neoui-shell/settings.md "The Settings app"): `SettingsApp` and
//! its `SetState` global. The device crate (`symbian/apps/settings`) fills `SetState` and answers its `act`
//! callback; the tests here draw every page into `snapshots/set2-*.png`.
#![no_std]

slint::include_modules!();

extern crate alloc;

use alloc::format;
use alloc::string::String;
use slint::Model;

/// The rows that show no value of their own (they open a page or act): search finds them by name only.
pub const VALUELESS: &[&str] = &[
    "sounds.add",
    "root.privacy",
    "root.about",
    "privacy.clip",
    "apps.clock",
    "apps.terminal",
    "apps.files",
    "apps.devtools",
    "apps.kit",
    "apps.phone",
    "apps.contacts",
    "apps.messages",
    "apps.browser",
    "apps.oldapps",
    "dev.log",
    "dev.save",
    "dev.tools",
    "about.licences",
];

/// What a row (`neoui_proto::settings_app::rows` id) shows now, as search matches and lists it: a switch as On/Off, a
/// choice by its name, a reading as the page shows it. Every row not in [`VALUELESS`] has one (tests/rows.rs).
#[must_use]
pub fn live_value(s: &SetState<'_>, id: &str) -> String {
    let on = |b: bool| String::from(if b { "On" } else { "Off" });
    let pick = |names: slint::ModelRc<slint::SharedString>, i: i32| {
        usize::try_from(i)
            .ok()
            .and_then(|i| names.row_data(i))
            .map_or_else(String::new, |n| String::from(n.as_str()))
    };
    match id {
        "root.wifi" | "wifi.switch" | "conn.wifi" => {
            let l = s.get_wifi_link();
            if !s.get_wifi() {
                "Off".into()
            } else if l.connected {
                l.name.into()
            } else {
                "Not connected".into()
            }
        }
        "wifi.saved" => format!("{}", s.get_wifi_saved().row_count()),
        "root.bt" | "bt.switch" | "conn.bt" => on(s.get_bluetooth()),
        "conn.data" | "mobile.data" => on(s.get_mobile_data()),
        "root.airplane" | "conn.airplane" => on(s.get_airplane()),
        "root.mobile" => {
            let rat = s.get_rat();
            if rat.is_empty() {
                "No service".into()
            } else {
                rat.into()
            }
        }
        "mobile.sim" => format!("{} · {}", s.get_operator(), s.get_sim()),
        "root.display" => String::from(if s.get_dark() { "Dark" } else { "Light" }),
        "display.dark" => on(s.get_dark()),
        "display.timeout" => s.get_timeout_text().into(),
        "root.navigation" => pick(s.get_nav_modes(), s.get_nav_mode()),
        "nav.0" | "nav.1" | "nav.2" => on(id.ends_with(&format!("{}", s.get_nav_mode()))),
        "root.sounds" => s.get_active_profile_name().into(),
        // a profile's row: its summary; a profile page's rows: the active profile's values
        p if p.starts_with("sounds.p") => p[8..]
            .parse::<i32>()
            .ok()
            .and_then(|id| s.get_profiles().iter().find(|r| r.id == id))
            .map_or_else(String::new, |r| String::from(r.summary.as_str())),
        row @ ("profile.tone" | "profile.ring" | "profile.volume" | "profile.vibrate"
        | "profile.message") => {
            let active = s.get_active_profile();
            let Some(r) = s.get_profiles().iter().find(|r| r.id == active) else {
                return String::new();
            };
            match row {
                "profile.tone" => String::from(if r.ring { "Built-in ringtone" } else { "None" }),
                "profile.ring" => on(r.ring),
                "profile.volume" => format!("{}", r.volume),
                "profile.vibrate" => on(r.vibrate),
                _ => String::from(if r.ring {
                    "Two buzzes + chime"
                } else {
                    "Two buzzes"
                }),
            }
        }
        "sounds.media" => format!("{}", s.get_media_volume()),
        "sounds.route" => pick(s.get_media_routes(), s.get_media_route()),
        "sounds.mic" => pick(s.get_mics(), s.get_mic()),
        "root.battery" => s.get_battery_text().into(),
        "root.datetime" => String::from(if s.get_time_auto() {
            "Automatic"
        } else {
            "Manual"
        }),
        "time.auto" => on(s.get_time_auto()),
        "time.auto_zone" => on(s.get_time_auto_zone()),
        "time.zone" => s.get_zone().into(),
        "time.24h" => on(s.get_clock_24h()),
        "time.seconds" => on(s.get_status_seconds()),
        "time.shade_date" => on(s.get_shade_date()),
        "time.format" => s.get_date_text().into(),
        "root.keyboard" | "kb.current" => pick(s.get_keyboard_names(), s.get_keyboard()),
        "kb.0" | "kb.1" | "kb.2" => {
            let i = id[3..].parse().unwrap_or(0);
            on(s.get_keyboards().row_data(i).unwrap_or(false))
        }
        "kb.same_height" => on(s.get_same_height()),
        "root.apps" => "Phone, old apps".into(),
        "root.developer" => String::from(if s.get_usb_debugging() {
            "USB debugging"
        } else {
            "Off"
        }),
        "dev.usb" => on(s.get_usb_debugging()),
        "pads.vibrate" => on(s.get_pad_vibrate()),
        "pads.strength" => pick(s.get_pad_strengths(), s.get_pad_strength()),
        "pads.default" => pick(s.get_pad_defaults(), s.get_pad_default()),
        "pads.dirs" => pick(s.get_pad_dirs_names(), s.get_pad_dirs()),
        "pads.a" => pick(s.get_pad_a_names(), s.get_pad_a()),
        "pads.b" => pick(s.get_pad_b_names(), s.get_pad_b()),
        "pads.lefty" => on(s.get_pad_lefty()),
        "pads.console" => on(s.get_pad_console()),
        _ => String::new(),
    }
}

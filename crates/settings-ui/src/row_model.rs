// SPDX-License-Identifier: GPL-3.0-only
//! D1's Slint side (neoui-proto `settings_app::row_model`): the [`Facts`] the providers read, taken from `SetState`
//! as the device filled it; the root's summaries ([`publish`]) and the search results ([`results`]) from one
//! projection, so the root, search and the page show one value. A later stream that adds facts reads them here
//! (or the device passes them in), never formats a row's value here.
use crate::{SearchHit, SetState, split_match};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use neoui_proto::settings_app::row_model::{
    AppFact, AppId, DriveFact, Epochs, Facts, Hit, KindsState, LinkFact, RowSnapshot, Sample,
    Scalar, WifiField, project_all, search,
};
use neoui_proto::settings_app::storage::KINDS;
use neoui_proto::sound_profile::Profile;
use slint::{Model, ModelRc, SharedString, VecModel};

/// "02:00:00:00:00:01" as bytes (zeros when it isn't one).
fn mac(text: &str) -> [u8; 6] {
    let mut b = [0u8; 6];
    for (o, h) in b.iter_mut().zip(text.split(':')) {
        *o = u8::from_str_radix(h, 16).unwrap_or(0);
    }
    b
}

/// Everything the rows read, from the screens' state at `sample`, the identities guarded by `epochs`.
#[must_use]
pub fn facts(s: &SetState<'_>, sample: Sample, epochs: Epochs) -> Facts {
    let mut f = Facts {
        sample,
        epochs,
        ..Facts::default()
    };
    let pick = |names: ModelRc<SharedString>, i: i32| {
        let name = usize::try_from(i).ok().and_then(|i| names.row_data(i));
        name.map(|n| Scalar::Choice(i as usize, String::from(n.as_str())))
    };
    let text = |t: SharedString| Some(Scalar::Text(String::from(t.as_str())));
    let on = |b: bool| Some(Scalar::On(b));
    let link = s.get_wifi_link();
    let keyboards = s.get_keyboards();
    let kb = |i: usize| keyboards.row_data(i).map(Scalar::On);
    for (key, v) in [
        ("radio.wifi", on(s.get_wifi())),
        (
            "wifi.network",
            link.connected
                .then(|| Scalar::Text(String::from(link.name.as_str()))),
        ),
        ("radio.bt", on(s.get_bluetooth())),
        ("radio.data", on(s.get_mobile_data())),
        ("radio.airplane", on(s.get_airplane())),
        ("rat", text(s.get_rat())),
        (
            "wifi.saved",
            s.get_wifi_saved_ok()
                .then(|| Scalar::Text(format!("{}", s.get_wifi_saved().row_count()))),
        ),
        (
            "mobile.sim",
            Some(Scalar::Text(format!(
                "{} · {}",
                s.get_operator(),
                s.get_sim()
            ))),
        ),
        ("display.dark", on(s.get_dark())),
        ("display.timeout", text(s.get_timeout_text())),
        ("nav.mode", pick(s.get_nav_modes(), s.get_nav_mode())),
        (
            "sounds.media",
            Some(Scalar::Text(format!("{}", s.get_media_volume()))),
        ),
        (
            "sounds.route",
            pick(s.get_media_routes(), s.get_media_route()),
        ),
        ("sounds.mic", pick(s.get_mics(), s.get_mic())),
        ("root.battery", text(s.get_battery_text())),
        ("storage.free", text(s.get_storage_free())),
        ("storage.kinds", text(s.get_storage_status())),
        ("time.auto", on(s.get_time_auto())),
        ("time.auto_zone", on(s.get_time_auto_zone())),
        ("time.zone", text(s.get_zone())),
        ("time.24h", on(s.get_clock_24h())),
        ("time.seconds", on(s.get_status_seconds())),
        ("time.shade_date", on(s.get_shade_date())),
        ("time.format", text(s.get_date_text())),
        ("kb.current", pick(s.get_keyboard_names(), s.get_keyboard())),
        ("kb.0", kb(0)),
        ("kb.1", kb(1)),
        ("kb.2", kb(2)),
        ("kb.same_height", on(s.get_same_height())),
        ("kb.vibrate", on(s.get_key_vibrate())),
        ("dev.usb", on(s.get_usb_debugging())),
        ("dev.touches", on(s.get_show_touches())),
        // D2: only what the kernel answered
        (
            "dev.console",
            s.get_console_ok()
                .then(|| Scalar::On(s.get_console_shown())),
        ),
        (
            "about.slot",
            (!s.get_slot().is_empty()).then(|| Scalar::Text(String::from(s.get_slot().as_str()))),
        ),
        // D7: Display's text size and wallpaper; the one display language until translations exist
        (
            "display.font_size",
            pick(s.get_font_sizes(), s.get_font_size()),
        ),
        ("display.wallpaper", text(s.get_wallpaper_name())),
        ("kb.display_language", Some(Scalar::Text("English".into()))),
        // D8: the clipboard toast
        ("privacy.clip_notice", on(s.get_clip_notice())),
        ("battery.saver", on(s.get_battery_saver())),
        ("battery.percent", on(s.get_show_percent())),
        ("pads.vibrate", on(s.get_pad_vibrate())),
        (
            "pads.strength",
            pick(s.get_pad_strengths(), s.get_pad_strength()),
        ),
        (
            "pads.default",
            pick(s.get_pad_defaults(), s.get_pad_default()),
        ),
        ("pads.dirs", pick(s.get_pad_dirs_names(), s.get_pad_dirs())),
        ("pads.a", pick(s.get_pad_a_names(), s.get_pad_a())),
        ("pads.b", pick(s.get_pad_b_names(), s.get_pad_b())),
        ("pads.lefty", on(s.get_pad_lefty())),
        ("pads.console", on(s.get_pad_console())),
        // D5
        ("battery.screen_on", text(s.get_screen_on())),
    ] {
        if let Some(v) = v {
            f.scalars.insert(key, v);
        }
    }
    f.profiles = s
        .get_profiles()
        .iter()
        .map(|p| Profile {
            id: p.id as u8,
            name: String::from(p.name.as_str()),
            ring: p.ring,
            volume: p.volume.clamp(0, 10) as u8,
            vibrate: p.vibrate,
            ringtone: p.ringtone as u8,
            message_tone: p.message_tone as u8,
            keypad: p.keypad,
            touch: p.touch,
        })
        .collect();
    f.active_profile = s.get_active_profile() as u8;
    f.drives = s
        .get_drives()
        .iter()
        .filter_map(|d| {
            Some(DriveFact {
                letter: d.letter.chars().next()?,
                title: String::from(d.title.as_str()),
                used: String::from(d.used.as_str()),
            })
        })
        .collect();
    f.kinds = s
        .get_storage_kinds()
        .iter()
        .filter_map(|k| {
            let kind = KINDS.into_iter().find(|x| x.name() == k.name.as_str())?;
            Some((kind, String::from(k.size.as_str())))
        })
        .collect();
    f.kinds_state = if s.get_storage_counting() {
        KindsState::Counting
    } else if s.get_storage_counted() {
        KindsState::Counted(String::from(s.get_storage_status().as_str()))
    } else {
        KindsState::NotCounted
    };
    let (rom, old) = (s.get_apps(), s.get_old_apps());
    let native = rom.iter().filter_map(|a| {
        let app = neoui_proto::apps::find(a.key.as_str())?;
        Some(AppFact {
            id: AppId::Native(app.key),
            name: String::from(a.name.as_str()),
        })
    });
    let old = old.iter().filter_map(|a| {
        Some(AppFact {
            id: AppId::Legacy(u32::from_str_radix(a.key.as_str(), 16).ok()?),
            name: String::from(a.name.as_str()),
        })
    });
    f.apps = native.chain(old.filter(|_| s.get_old_ok())).collect();
    f.link = link.shown.then(|| LinkFact {
        bssid: mac(link.bssid.as_str()),
        fields: link
            .details
            .iter()
            .filter_map(|d| {
                Some((
                    WifiField::from_label(d.label.as_str())?,
                    String::from(d.value.as_str()),
                ))
            })
            .collect(),
    });
    f
}

/// The projection of the screens' state now.
#[must_use]
pub fn project(s: &SetState<'_>, sample: Sample, epochs: Epochs) -> RowSnapshot {
    project_all(&facts(s, sample, epochs))
}

/// The root's values from `snap` (replaced only when one changed).
pub fn publish(s: &SetState<'_>, snap: &RowSnapshot) {
    let now = snap.summaries();
    let had = s.get_summaries();
    if had.row_count() != now.len() || had.iter().zip(&now).any(|(a, b)| a.as_str() != b) {
        let model: Vec<SharedString> = now.iter().map(|v| v.as_str().into()).collect();
        s.set_summaries(ModelRc::new(VecModel::from(model)));
    }
}

/// The results for `query`: as the screen lists them, and the hits their tokens stand for.
#[must_use]
pub fn results(snap: &RowSnapshot, query: &str) -> (Vec<SearchHit>, Vec<Hit>) {
    let hits = search(snap, query);
    let shown = hits
        .iter()
        .map(|h| {
            let r = &snap.rows[h.row];
            let m = split_match(&r.title, query);
            SearchHit {
                token: h.token as i32,
                title: r.title.as_str().into(),
                pre: m.pre.into(),
                mark: m.mark.into(),
                post: m.post.into(),
                gap_before: m.gap_before,
                gap_after: m.gap_after,
                path: r.path.as_str().into(),
                value: r.value_text.as_str().into(),
                page: i32::from(r.destination.page.id()),
            }
        })
        .collect();
    (shown, hits)
}

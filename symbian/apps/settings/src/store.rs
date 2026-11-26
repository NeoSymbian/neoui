// SPDX-License-Identifier: GPL-3.0-only
//! The settings the pages show, read from the store into `SetState` (again on every `AppEvent::Settings`), and the
//! pages' writes back to it. The shell lets this app write shell-owned keys because its messages carry NEOSET's secure
//! ID (`Caller::Settings`).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::dprintln;
use neoapp::App;
use neoui_proto::clock::datetime::date_text;
use neoui_proto::clock::face::LocalTime;
use neoui_proto::clock::world::CATALOGUE;
use neoui_proto::settings::registry::{self, DefaultValue, keys};
use neoui_proto::settings::{Key, Setting, Value};
use neoui_proto::settings_app::choices::{self, KEYBOARDS, TIMEOUTS};
use neoui_proto::sound_profile::{self, Mirrors, Profiles};
use settings_ui::{ProfileRow, SetState, SettingsApp};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

fn strings(v: impl IntoIterator<Item = impl Into<SharedString>>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        v.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
}

/// The fixed choice names, once.
pub fn fill_fixed(ui: &SettingsApp) {
    {
        use neoui_proto::sound_profile::tone;
        let s = ui.global::<SetState>();
        s.set_ring_tones(strings(tone::ALL.iter().map(|t| tone::ring_name(*t))));
        s.set_message_tones(strings(tone::ALL.iter().map(|t| tone::message_name(*t))));
    }
    let s = ui.global::<SetState>();
    s.set_timeouts(strings(TIMEOUTS.iter().map(|(_, t)| *t)));
    s.set_nav_modes(strings(choices::NAV_MODES.iter().map(|(n, _)| *n)));
    s.set_nav_hints(strings(choices::NAV_MODES.iter().map(|(_, h)| *h)));
    s.set_media_routes(strings(choices::MEDIA_ROUTES));
    s.set_mics(strings(choices::MICS));
    s.set_keyboard_names(strings(KEYBOARDS.iter().map(|(_, n, _)| *n)));
    s.set_keyboard_hints(strings(KEYBOARDS.iter().map(|(_, _, h)| *h)));
    s.set_pad_strengths(strings(choices::PAD_STRENGTHS));
    s.set_pad_defaults(strings(choices::PAD_DEFAULTS));
    s.set_pad_dirs_names(strings(choices::PAD_DIRS));
    s.set_pad_a_names(strings(choices::PAD_A));
    s.set_pad_b_names(strings(choices::PAD_B));
    s.set_build(strings(neoapp::build::lines()));
    // D7: Font size's choices
    s.set_font_sizes(strings(choices::FONT_SIZES.iter().map(|(n, _)| *n)));
    s.set_font_scales(strings(choices::FONT_SIZES.iter().map(|(_, p)| *p)));
}

/// The shell's value, else the registry's default (a shell older than the key refuses it).
fn get<T: Setting + Default>(app: &App, key: Key<T>) -> T {
    app.setting(key)
        .or_else(|_| neoui_proto::settings::Store::new().get(key))
        .unwrap_or_default()
}

/// Which keyboard languages are on (`keyboard.en/hy/ru`).
pub fn keyboards(app: &App) -> Vec<bool> {
    KEYBOARDS
        .iter()
        .map(|(k, _, _)| matches!(app.setting_raw(k), Ok(Value::Bool(true))))
        .collect()
}

/// The first language turned on at or after `from`, wrapping round (the shell's keyboard picks the same way).
pub fn pick(on: &[bool], from: i32) -> i32 {
    let n = on.len() as i32;
    (0..n)
        .map(|d| (from + d).rem_euclid(n.max(1)))
        .find(|&i| on.get(i as usize) == Some(&true))
        .unwrap_or(0)
}

fn local_now() -> LocalTime {
    let t = e32::time::now();
    LocalTime {
        year: t.year,
        month: t.month,
        day: t.day,
        weekday: t.weekday,
        hour: t.hour,
        minute: t.minute,
    }
}

/// Date format: today's date in the chosen format and in each of the four (again each time the page reads it, so a
/// page left open over midnight catches up).
pub fn dates(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    let format = get(app, keys::TIME_DATE_FORMAT);
    let today = local_now();
    s.set_date_format(format);
    s.set_date_text(date_text(&today, format).into());
    s.set_date_formats(strings((0..4).map(|f| date_text(&today, f))));
}

/// The root as the Nokia grid: the app's own key (`settings.*`, its namespace in `settings::access`; no registry row,
/// so a missing or retyped value is the list).
pub const ROOT_GRID: &str = "settings.root_grid";

/// The sound profiles as the shell's reducer keeps them (read the same way, `sound_profile::read`).
/// The keys come one IPC each: a commit between them (the power menu, the shade) shows as the active id changing,
/// and they are read again.
pub fn read_profiles(app: &App) -> Profiles {
    let mut tries = 0;
    loop {
        let active = get(app, keys::SOUND_PROFILE);
        let bytes = |k| match app.setting_raw(k) {
            Ok(Value::Bytes(b)) => b,
            _ => Vec::new(),
        };
        let (blob, tones) = (bytes("sound.profiles"), bytes("sound.tones"));
        let feedback = bytes("sound.feedback");
        let yours = bytes("sound.your_tones");
        let mirrors = Mirrors {
            silent: get(app, keys::SOUND_SILENT),
            vibrate: get(app, keys::SOUND_VIBRATE),
            volume: get(app, keys::SOUND_RING_VOLUME).clamp(0, 10) as u8,
        };
        tries += 1;
        if get(app, keys::SOUND_PROFILE) == active || tries == 3 {
            return sound_profile::read(active, &blob, &tones, &feedback, &yours, mirrors);
        }
    }
}

/// The profiles into Sounds & profiles; the profile page's own follows its id.
pub fn profiles(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    let p = read_profiles(app);
    let rows: Vec<ProfileRow> = p
        .list
        .iter()
        .map(|x| ProfileRow {
            id: i32::from(x.id),
            name: x.name.as_str().into(),
            summary: p.summary(x).into(),
            tone_name: p.ring_name(x.ringtone).into(),
            ring: x.ring,
            volume: i32::from(x.volume),
            vibrate: x.vibrate,
            ringtone: i32::from(x.ringtone),
            message_tone: i32::from(x.message_tone),
            keypad: x.keypad,
            touch: x.touch,
        })
        .collect();
    s.set_active_profile(i32::from(p.active));
    s.set_active_profile_name(
        p.get(p.active)
            .map_or("General", |x| x.name.as_str())
            .into(),
    );
    s.set_active_profile_summary(
        p.get(p.active)
            .map(|x| p.summary(x))
            .unwrap_or_default()
            .into(),
    );
    let current = s.get_profile_current().id;
    let pick = rows
        .iter()
        .find(|r| r.id == current)
        .or_else(|| rows.iter().find(|r| r.id == i32::from(p.active)))
        .cloned();
    if let Some(r) = pick {
        s.set_profile_current(r);
    }
    s.set_profiles(ModelRc::new(VecModel::from(rows)));
    crate::profile_audio::summary(ui);
    crate::tone_import::show(ui, &p);
}

/// Every setting a page shows.
pub fn load(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    s.set_root_grid(matches!(app.setting_raw(ROOT_GRID), Ok(Value::Bool(true))));
    s.set_wifi(get(app, keys::RADIO_WIFI));
    s.set_bluetooth(get(app, keys::RADIO_BT));
    s.set_mobile_data(get(app, keys::RADIO_DATA));
    s.set_airplane(get(app, keys::RADIO_AIRPLANE));
    s.set_silent(get(app, keys::SOUND_SILENT));
    s.set_vibrate(get(app, keys::SOUND_VIBRATE));
    s.set_ring_volume(get(app, keys::SOUND_RING_VOLUME));
    profiles(app, ui);
    s.set_media_volume(get(app, keys::SOUND_MEDIA_VOLUME));
    s.set_media_route(get(app, keys::SOUND_MEDIA_ROUTE));
    s.set_mic(get(app, keys::SOUND_MIC));
    s.set_dark(get(app, keys::DISPLAY_DARK));
    s.set_brightness(get(app, keys::DISPLAY_BRIGHTNESS) as f32 / 255.0);
    let timeout = get(app, keys::DISPLAY_TIMEOUT);
    s.set_timeout(timeout);
    s.set_timeout_text(choices::timeout_text(timeout).into());
    s.set_timeout_index(choices::timeout_index(timeout));
    s.set_nav_mode(get(app, keys::NAV_MODE));
    s.set_time_auto(get(app, keys::TIME_AUTO));
    s.set_time_auto_zone(get(app, keys::TIME_AUTO_ZONE));
    s.set_clock_24h(get(app, keys::CLOCK_24H));
    s.set_status_seconds(get(app, keys::STATUS_SECONDS));
    s.set_shade_date(get(app, keys::SHADE_DATE));
    dates(app, ui);
    s.set_usb_debugging(get(app, keys::DEVELOPER_USB_DEBUGGING));
    s.set_show_touches(get(app, keys::DEVELOPER_SHOW_TOUCHES));
    s.set_key_vibrate(get(app, keys::KEYBOARD_VIBRATE));
    s.set_battery_saver(get(app, keys::BATTERY_SAVER));
    s.set_show_percent(get(app, keys::BATTERY_SHOW_PERCENT));
    let on = keyboards(app);
    s.set_keyboard(pick(&on, get(app, keys::KEYBOARD_LAYOUT)));
    s.set_keyboards(ModelRc::new(VecModel::from(on)));
    s.set_same_height(get(app, keys::KEYBOARD_SAME_HEIGHT));
    s.set_pad_vibrate(get(app, keys::LEGACY_PAD_VIBRATE));
    s.set_pad_strength(get(app, keys::LEGACY_PAD_STRENGTH));
    s.set_pad_default(get(app, keys::LEGACY_PAD_DEFAULT));
    s.set_pad_dirs(get(app, keys::LEGACY_PAD_DIRS));
    s.set_pad_a(get(app, keys::LEGACY_PAD_A));
    s.set_pad_b(get(app, keys::LEGACY_PAD_B));
    s.set_pad_lefty(get(app, keys::LEGACY_PAD_LEFTY));
    s.set_pad_console(get(app, keys::LEGACY_PAD_CONSOLE));
    crate::appearance::load(app, ui);
    crate::notifications::refresh(app, ui);
    crate::navigation::load(app, ui);
    s.set_clip_notice(get(app, keys::PRIVACY_CLIP_NOTICE));
}

/// A page's write: `value` as the pages send it ("1"/"0", a number, text), typed by the key's registry row. A refused
/// write is logged and the pages show the stored value again.
pub fn write(app: &App, ui: &SettingsApp, key: &str, value: &str) {
    let Some(spec) = registry::lookup(key) else {
        dprintln!("settings: no setting {}", key);
        return;
    };
    let v = match spec.default {
        DefaultValue::Bool(_) => Some(Value::Bool(value == "1")),
        DefaultValue::Int(_) => value.parse().ok().map(Value::Int),
        DefaultValue::Str(_) => Some(Value::Str(String::from(value))),
        DefaultValue::Bytes(_) => None,
    };
    let r = match v {
        Some(v) => app.set_setting_raw(key, &v),
        None => Err(-6),
    };
    dprintln!("settings: set {} = {} ({:?})", key, value, r);
    if r.is_err() {
        load(app, ui);
    }
}

/// The city sheet's rows for `query` (case-insensitive, anywhere in the name): their names, and "Name · UTC+04:00".
pub fn cities(query: &str) -> (Vec<&'static str>, Vec<String>) {
    // the names, and each one's offset now ("UTC+04:00")
    let q = query.trim().to_lowercase();
    let utc = e32::time::utc_now().0;
    CATALOGUE
        .iter()
        .filter(|c| q.is_empty() || c.name.to_lowercase().contains(&q))
        .map(|c| {
            let o = c.offset_at(utc);
            let m = o.unsigned_abs() / 60;
            let sign = if o < 0 { '-' } else { '+' };
            (c.name, format!("UTC{sign}{:02}:{:02}", m / 60, m % 60))
        })
        .unzip()
}

/// The city the phone keeps the time of (`time.zone`), "" before one was picked.
pub fn zone(app: &App) -> String {
    match app.setting_raw("time.zone") {
        Ok(Value::Str(s)) => s,
        _ => String::new(),
    }
}

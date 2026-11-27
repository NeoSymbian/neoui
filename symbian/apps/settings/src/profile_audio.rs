// SPDX-License-Identifier: GPL-3.0-only
//! D6 (docs neoui-shell/settings-data.md): what a profile's sounds really do.
//! - The message tone's description: MODEMD alerts a new message with two buzzes and, with Messages' "tone and
//!   vibrate", the first 0.8 s of the tone (`sound_profile::Profile::message_summary`). Messages' setting is read from
//!   MODEMD (`SMS_SETTINGS`) when the profile page opens and every few seconds on it, so a change in Messages shows.
//! - A ringtone's preview: pause and play on, and how far it played (NeoAudio INFO's frames, `sound`).
//! - Keypad tones and vibrate on touch, per profile: a reducer command (`ACTION_PROFILE` tag 6); the shell plays and
//!   pulses them (its `feedback` module).
use crate::modem::Modem;
use neoui_proto::sound_profile::Command;
use neoui_proto::sound_profile::Profile;
use settings_ui::{ProfileRow, SetState, SettingsApp};
use slint::ComponentHandle;

fn profile(r: &ProfileRow) -> Profile {
    let byte = |v: i32| v.clamp(0, 255) as u8;
    Profile {
        id: byte(r.id),
        name: alloc::string::String::from(r.name.as_str()),
        ring: r.ring,
        volume: byte(r.volume),
        vibrate: r.vibrate,
        ringtone: byte(r.ringtone),
        message_tone: byte(r.message_tone),
        keypad: r.keypad,
        touch: r.touch,
    }
}

/// The profile page's message tone row, for the profile it shows now.
pub fn summary(ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    let alert = u8::try_from(s.get_sms_alert()).ok();
    let text = profile(&s.get_profile_current()).message_summary(alert);
    s.set_message_summary(text.into());
}

/// "profile-feedback" + "keypad,touch" (0/1): the profile page's two switches, as a reducer command.
pub fn feedback_command(ui: &SettingsApp, value: &str) -> Option<Command> {
    let id = u8::try_from(ui.global::<SetState>().get_profile_current().id).ok()?;
    let (keypad, touch) = value.split_once(',')?;
    Some(Command::SetFeedback {
        id,
        keypad: keypad == "1",
        touch: touch == "1",
    })
}

/// (D6) The Ringtone page's preview line: the tone that plays, "Playing · 0:01" / "Paused · 0:01".
pub fn show_preview(sound: &crate::sound::Sound, ui: &SettingsApp) {
    let (tone, paused, ms) = sound.preview_state();
    let s = ui.global::<SetState>();
    s.set_preview_tone(tone);
    s.set_preview_paused(paused);
    s.set_preview_text(neoui_proto::sound_profile::preview::label(paused, ms).into());
}

/// (D6) ⏸ / ▶ on the playing tone: shown at once, AUDIOD follows between the worker's writes.
pub fn toggle_pause(sound: &crate::sound::Sound, ui: &SettingsApp) {
    sound.toggle_pause();
    show_preview(sound, ui);
}

/// Read Messages' alert setting from MODEMD again (an unreadable one shows as such).
pub fn refresh(modem: &mut Modem, ui: &SettingsApp) {
    let alert = modem.sms_alert().map_or(-1, i32::from);
    ui.global::<SetState>().set_sms_alert(alert);
    summary(ui);
}

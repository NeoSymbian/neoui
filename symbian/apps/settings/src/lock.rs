// SPDX-License-Identifier: GPL-3.0-only
//! TP8/TP9, Lock screen & security (set2-lock): `lock.on`, `notify.lock` and the shell's `lock.pin_set` for the
//! page, its preview's time, and the screen-lock PIN, which only SYSSTART's own dialog sets (`ACTION_LOCK_DIALOG`):
//! NEOSET asks for the dialog and never sees a digit.

use crate::store::{date_text_now, get};
use e32::dprintln;
use neoapp::App;
use neoui_proto::ipc as proto;
use neoui_proto::settings::keys;
use neoui_proto::settings_app::wire::encode_lock_dialog;
use settings_ui::{SetState, SettingsApp};
use slint::ComponentHandle;

/// The page's values, from the store.
pub fn load(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    s.set_lock_on(get(app, keys::LOCK_ON));
    s.set_lock_pin_set(get(app, keys::LOCK_PIN_SET));
    s.set_notify_lock(get(app, keys::NOTIFY_LOCK).clamp(0, 2));
}

/// The preview's time and date (each tick on the page).
pub fn show(ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    let t = e32::time::now();
    s.set_lock_time(alloc::format!("{:02}:{:02}", t.hour, t.minute).into());
    s.set_lock_date(date_text_now().into());
}

/// "lock-pin" + "0" | "1" | "2": the shell's PIN dialog (enrol, change, remove) over NEOSET.
pub fn pin(app: &App, mode: &str) {
    let Ok(m) = mode.parse::<u8>() else { return };
    let r = app.settings_action_with(proto::ACTION_LOCK_DIALOG, &encode_lock_dialog(m));
    dprintln!("settings: lock PIN dialog {} ({:?})", m, r);
    if r.is_err() {
        let _ = app.note("This phone's shell can't set a screen lock PIN yet", "");
    }
}

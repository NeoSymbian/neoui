// SPDX-License-Identifier: GPL-3.0-only
//! (D10) Home & navigation's gesture rows as stored (`nav.swipe_close`, `nav.switcher_cols`, `home.style`), read again
//! with every other setting (`store::load`), and Edit home: the shell's home editor (`ipc::ACTION_EDIT_HOME`), which
//! saves through its own Done and Back.
use alloc::vec::Vec;
use e32::dprintln;
use neoapp::App;
use neoui_proto::ipc as proto;
use neoui_proto::settings::keys;
use neoui_proto::settings_app::choices::{HOME_STYLES, SWITCHER_COLS};
use settings_ui::{SetState, SettingsApp};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

fn strings(v: impl IntoIterator<Item = &'static str>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        v.into_iter().map(SharedString::from).collect::<Vec<_>>(),
    ))
}

pub fn load(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    // an older shell refuses the keys: the defaults (off, 2 columns, Pages)
    s.set_swipe_close(app.setting(keys::NAV_SWIPE_CLOSE).unwrap_or(false));
    s.set_switcher_cols(
        app.setting(keys::NAV_SWITCHER_COLS)
            .unwrap_or(2)
            .clamp(2, 3),
    );
    s.set_home_style(app.setting(keys::HOME_STYLE).unwrap_or(0).clamp(0, 1));
    s.set_switcher_col_names(strings(SWITCHER_COLS));
    s.set_home_styles(strings(HOME_STYLES.iter().map(|(n, _)| *n)));
    s.set_home_style_hints(strings(HOME_STYLES.iter().map(|(_, h)| *h)));
}

/// Edit home: home comes up in the shell's editor (this app goes to the back).
pub fn edit_home(app: &App) {
    let r = app.settings_action(proto::ACTION_EDIT_HOME);
    dprintln!("settings: edit home ({:?})", r);
    if r.is_err() {
        let _ = app.note("Couldn't open the home editor", "");
    }
}

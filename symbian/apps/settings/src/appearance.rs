// SPDX-License-Identifier: GPL-3.0-only
//! (D7) Display's text size and wallpaper as stored (`display.font_size`, `display.wallpaper`), read again with every
//! other setting (`store::load`). The size also scales this app's own text (the kit's `Theme.font-size`); the shell
//! scales its own and draws the wallpaper. Other apps follow the size when they read it too (their repins).
use neoapp::App;
use neoui_proto::settings::keys;
use neoui_proto::settings_app::choices::wallpaper_name;
use settings_ui::{SetState, SettingsApp, Theme};
use slint::ComponentHandle;

pub fn load(app: &App, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    // an older shell refuses the keys: the defaults (Medium, Night)
    let size = app
        .setting(keys::DISPLAY_FONT_SIZE)
        .unwrap_or(1)
        .clamp(0, 3);
    s.set_font_size(size);
    ui.global::<Theme>().set_font_size(size);
    let wallpaper = app
        .setting(keys::DISPLAY_WALLPAPER)
        .unwrap_or_else(|_| "night".into());
    s.set_wallpaper_name(wallpaper_name(&wallpaper).into());
    s.set_wallpaper(wallpaper.as_str().into());
}

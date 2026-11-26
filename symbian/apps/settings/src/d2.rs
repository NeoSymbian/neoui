// SPDX-License-Identifier: GPL-3.0-only
//! D2 (docs neoui-shell/settings-data.md): what the kernel already says, read straight from its HAL as Dev tools does:
//! - Developer's Console overlay (HAL GET 0x4e03 / SET 0x4e02), read back after a write and whenever the page opens or
//!   the app comes back (Dev tools can change it too); no row when the kernel can't say;
//! - About's active A/B slot (HAL 0x4e09); no row without slots or on a GPT read error.
//!
//! The Wi-Fi details' BSSID, phone MAC, association ID and received frames are WIFID's link record (`wifi`).
use neoapp::hal;
use neoui_proto::settings_app::page::Page;
use settings_ui::{SetState, SettingsApp};
use slint::ComponentHandle;

/// The page opened or the app came back: read what it shows.
pub fn refresh(ui: &SettingsApp, page: Page) {
    match page {
        Page::Developer => console(ui),
        Page::About => {
            let slot = hal::current_slot().map(|c| alloc::format!("{c}"));
            ui.global::<SetState>()
                .set_slot(slot.unwrap_or_default().into());
        }
        _ => {}
    }
}

fn console(ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    let shown = hal::console_shown();
    s.set_console_ok(shown.is_ok());
    s.set_console_shown(shown.unwrap_or(false));
}

/// The switch: write, then show what the kernel has now.
pub fn set_console(ui: &SettingsApp, on: bool) {
    let r = hal::set_console(on);
    e32::dprintln!("settings: console {} -> {:?}", on, r);
    console(ui);
}

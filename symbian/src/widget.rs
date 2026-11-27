// SPDX-License-Identifier: Apache-2.0
//! (home.md Part 4, protocol 1.22) App widgets: `ipc::WIDGET` from an app's own EXE (`contract::widget` checks the
//! caller, then the record against `widget::CATALOGUE`), kept under `widget.<app>.<name>` by the shell, so home shows
//! it after a reboot without the app running. The settings mirror shows a changed key on home (`show_changed`), and
//! every catalogue widget once at boot (`show_all`). A child of `host` (it reads the message there).

use super::{Host, read_bytes};
use alloc::vec::Vec;
use e32::dprintln;
use e32::server::Message;
use neoui_proto::contract;
use neoui_proto::settings::{Caller, Value};
use neoui_proto::widget::{self, Content};
use neoui_render::Renderer;

use crate::settings::{self, Settings};

impl Host {
    /// `WIDGET` (46) from app `key`: 0 once kept, else the contract's refusal or the store's.
    pub(super) fn widget(&mut self, key: &str, m: &Message) -> i32 {
        let c = match contract::widget(key, m.client_sid(), |max| read_bytes(m, 0, max)) {
            Ok(c) => c,
            Err(e) => {
                dprintln!("neoui: widget refused for {} ({})", key, e);
                return e;
            }
        };
        // `fits` found it in the catalogue, so the key is one
        let Some(name) = widget::setting_key(key, &c.name) else {
            return neoui_proto::settings::kerr::ARGUMENT;
        };
        let r = settings::set(
            &mut self.settings,
            Caller::Shell,
            &name,
            Value::Bytes(c.encode()),
            e32::time::millis(),
        );
        match r {
            Ok(_) => {
                dprintln!(
                    "neoui: widget {} from {} ({:?})",
                    name,
                    key,
                    c.body.template()
                );
                0
            }
            Err(e) => e.to_symbian(),
        }
    }
}

/// What the store keeps for `(app, name)`, if anything readable.
fn stored(settings: &Settings, app: &str, name: &str) -> Option<Content> {
    let key = widget::setting_key(app, name)?;
    match settings.get_raw(&key) {
        Ok(Value::Bytes(b)) => Content::decode(&b).filter(|c| c.fits(app).is_ok()),
        _ => None,
    }
}

/// Every catalogue widget's kept content on home (at boot, or when the store can't say what changed).
pub(crate) fn show_all(ui: &mut Renderer, settings: &Settings) {
    for (app, name, ..) in widget::CATALOGUE {
        ui.set_app_widget(app, name, stored(settings, app, name));
    }
}

/// The widgets among the store's changed `keys` on home.
pub(crate) fn show_changed(ui: &mut Renderer, settings: &Settings, keys: &[&str]) {
    let changed: Vec<_> = widget::CATALOGUE
        .iter()
        .filter(|(app, name, ..)| {
            widget::setting_key(app, name).is_some_and(|k| keys.contains(&k.as_str()))
        })
        .collect();
    for (app, name, ..) in changed {
        ui.set_app_widget(app, name, stored(settings, app, name));
    }
}

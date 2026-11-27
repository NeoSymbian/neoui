// SPDX-License-Identifier: Apache-2.0
//! (TP3) The media session in SYSSTART (docs `neoui-shell/top-panel.md` "Media session"), a child of `host` (it
//! works on the apps there): `MEDIA` from a player, the one session system-wide (`neoui_proto::media::Current`: the
//! last app that played owns it, the old owner gets PAUSE, the owner's exit or an empty record ends it), the
//! commands the shade's and home's buttons send back as `AppEvent::Media`, the volume HUD's 2 s, and `Visible` to the
//! front app when the screen goes off and on.

use super::{Host, read_bytes, unix_ms};
use e32::dprintln;
use e32::server::Message;
use neoui_proto::contract;
use neoui_proto::ipc::AppEvent;
use neoui_proto::media::{Current, cmd, valid_command};
use neoui_proto::settings::keys;
use neoui_render::Renderer;

/// How long the volume HUD shows after the last volume key.
const HUD_MS: u64 = 2_000;

/// The session and what the main loop last drew of it.
#[derive(Default)]
pub struct Media {
    pub current: Current<&'static str>,
    changed: bool,
    /// the second (unix) the playing bar was last drawn at, and `shell.media` as drawn
    drawn: Option<(Option<u64>, bool)>,
    /// when the HUD goes (ms), 0 none
    hud_until: u64,
}

impl Host {
    /// `MEDIA` from app `key`: its session, or the empty record that ends it.
    pub(super) fn media_message(&mut self, key: &'static str, m: &Message) -> i32 {
        let s = match contract::media(key, m.client_sid(), |max| read_bytes(m, 0, max)) {
            Ok(s) => s,
            Err(e) => {
                dprintln!("neoui: media from {} refused ({})", key, e);
                return e;
            }
        };
        match &s {
            Some(s) => dprintln!("neoui: media {} state {}", key, s.state),
            None => dprintln!("neoui: media {} ends", key),
        }
        if let Some(old) = self.media.current.post(key, s) {
            dprintln!("neoui: media {} takes over, {} pauses", key, old);
            self.post(
                old,
                AppEvent::Media {
                    cmd: cmd::PAUSE,
                    arg: 0,
                },
            );
        }
        self.media.changed = true;
        0
    }

    /// Each pass of the main loop: the owner's exit ends the session, the buttons' commands go to the owner, the
    /// player is drawn again when it changed (and each second while it plays), and the HUD goes after its 2 s.
    pub fn media_frame(&mut self, ui: &mut Renderer, ms: u64) {
        if let Some(k) = self.media.current.owner()
            && self
                .apps
                .find(|a| a.key == k && a.quit_at.is_none())
                .is_none()
        {
            dprintln!("neoui: media {} ends (gone)", k);
            self.media.current.end(k);
            self.media.changed = true;
        }
        for (c, arg) in ui.take_media_commands() {
            if let Some(k) = self.media.current.owner()
                && valid_command(c, arg)
            {
                dprintln!("neoui: media command {} {} to {}", c, arg, k);
                self.post(k, AppEvent::Media { cmd: c, arg });
            }
        }
        let now = unix_ms();
        let shade = self.settings.get(keys::SHELL_MEDIA).unwrap_or(true);
        let tick = self.media.current.playing().map(|_| now / 1000);
        let want = Some((tick, shade));
        if core::mem::take(&mut self.media.changed) || self.media.drawn != want {
            ui.set_media(self.media.current.shown(), shade, now);
            self.media.drawn = want;
        }
        if self.media.hud_until != 0 && ms >= self.media.hud_until {
            self.media.hud_until = 0;
            ui.show_volume(None, false);
        }
    }

    /// A volume key stepped the call's volume (`call`) or the media volume: the HUD shows the level for 2 s.
    pub fn volume_hud(&mut self, ui: &mut Renderer, call: Option<u8>, ms: u64) {
        let level = call.unwrap_or_else(|| {
            self.settings
                .get(keys::SOUND_MEDIA_VOLUME)
                .unwrap_or(7)
                .clamp(0, 10) as u8
        });
        ui.show_volume(Some(level), call.is_some());
        self.media.hud_until = ms + HUD_MS;
    }

    /// The screen went off or on: the front app is told it is hidden or shown (a player keeps playing; a page stops
    /// drawing), and on wake its surface is composed again whole.
    pub fn screen_power(&mut self, on: bool) {
        if let Some(key) = self.front
            && let Some(app) = self.apps.find_mut(|a| a.key == key && a.quit_at.is_none())
        {
            app.send(AppEvent::Visible(on));
        }
        if on {
            self.refresh = true;
        }
    }
}

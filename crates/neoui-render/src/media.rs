// SPDX-License-Identifier: Apache-2.0
//! (TP3) The media session in the shell (docs `neoui-shell/top-panel.md` "Media session"): the `Media` global
//! (ui/media.slint) that the shade's player, home's Media card and the volume HUD draw, and the commands their
//! buttons send (`media::cmd`, for the session's owner as `AppEvent::Media`).

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use neoui_proto::Rect;
use neoui_proto::apps::APPS;
use neoui_proto::media::{Session, caps, clock_text, modes, state};
use slint::ComponentHandle;

use crate::{Media, Renderer, Shell};

/// The buttons' commands not yet taken: (cmd, arg).
pub(crate) type Commands = Rc<RefCell<Vec<(u8, u32)>>>;

pub(crate) fn wire(ui: &Shell) -> Commands {
    let sink = Commands::default();
    let out = sink.clone();
    ui.global::<Media>().on_command(move |c, arg| {
        if let (Ok(c), Ok(arg)) = (u8::try_from(c), u32::try_from(arg)) {
            out.borrow_mut().push((c, arg));
        }
    });
    sink
}

impl Renderer {
    /// What the media session shows: app `key`'s `session` (`live`), or the last track (not live, home only); `None`
    /// for nothing playing. `shade`: the shade draws the player (`shell.media`). `now` (unix ms) places the bar of a
    /// playing session; call again each second while it plays.
    pub fn set_media(&mut self, s: Option<(&str, &Session, bool)>, shade: bool, now: u64) {
        let g = self.ui.global::<Media>();
        let Some((key, s, live)) = s else {
            g.set_on(false);
            g.set_live(false);
            g.set_in_shade(false);
            g.set_playing(false);
            return;
        };
        let name = APPS.iter().find(|a| a.key == key).map_or(key, |a| a.name);
        let has = |c: u8| live && s.caps & c != 0;
        let pos = s.position_at(now);
        g.set_on(true);
        g.set_live(live);
        g.set_in_shade(live && shade);
        g.set_playing(live && matches!(s.state, state::PLAYING | state::BUFFERING));
        g.set_title(s.title.as_str().into());
        g.set_artist(s.artist.as_str().into());
        g.set_app(name.into());
        g.set_app_key(key.into());
        g.set_can_play(!live || has(caps::PLAY | caps::PAUSE));
        g.set_can_prev(has(caps::PREV));
        g.set_can_next(has(caps::NEXT));
        g.set_can_seek(has(caps::SEEK) && s.duration_ms > 0);
        g.set_can_shuffle(has(caps::SHUFFLE));
        g.set_can_repeat(has(caps::REPEAT));
        g.set_shuffle(s.modes & modes::SHUFFLE != 0);
        g.set_repeat(s.modes & modes::REPEAT != 0);
        g.set_duration_ms(i32::try_from(s.duration_ms).unwrap_or(i32::MAX));
        if s.duration_ms == 0 {
            g.set_progress(-1.0);
        } else {
            g.set_progress(pos as f32 / s.duration_ms as f32);
            g.set_elapsed(clock_text(pos).into());
            g.set_total(clock_text(s.duration_ms).into());
        }
    }

    /// The shade's and home's media buttons since the last call: (`media::cmd`, arg).
    pub fn take_media_commands(&mut self) -> Vec<(u8, u32)> {
        core::mem::take(&mut *self.media.borrow_mut())
    }

    /// The volume HUD: `Some(level)` 0..=10 shows it (`call`: the call's volume), `None` hides it. The UI server
    /// hides it 2 s after the last key.
    pub fn show_volume(&mut self, level: Option<u8>, call: bool) {
        let g = self.ui.global::<Media>();
        g.set_hud(level.is_some());
        g.set_hud_call(call);
        if let Some(l) = level {
            g.set_hud_level(i32::from(l));
        }
    }

    /// The volume HUD while it shows, in physical pixels: it lies over the app's body, so the UI server copies it
    /// from the frame again after composing the app's surface.
    pub fn hud_rect(&self) -> Option<Rect> {
        self.ui.get_hud_on().then(|| {
            Rect::new(
                self.ui.get_hud_x() as i32,
                self.ui.get_hud_y() as i32,
                // a pixel more each way: its edges may fall between pixels
                self.ui.get_hud_w() as u32 + 1,
                self.ui.get_hud_h() as u32 + 1,
            )
        })
    }
}

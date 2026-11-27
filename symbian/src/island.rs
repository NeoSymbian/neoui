// SPDX-License-Identifier: Apache-2.0
//! (TP6) The live island in SYSSTART (docs `neoui-shell/top-panel.md` "Live island (TP6)"), a child of `host`: the
//! apps' `ACTIVITY` (checked by `contract::activity`; `island.deny` refuses an app), the media session's own activity,
//! the order and the two the island shows (`neoui_proto::island`), the shell's ticks of Since/Until (each second) with
//! an app's updates drawn at most once a second, the 2 s system alerts (the ringer to silent, the charger in), and the
//! taps that come back: the activity's app comes forward on its page, or hears `ActivityTap` where it is. Chrono
//! cards in the shade tick with it.

use super::{Host, read_bytes, unix_ms};
use alloc::vec::Vec;
use e32::dprintln;
use e32::server::Message;
use neoui_proto::activity::{self, Activity};
use neoui_proto::contract;
use neoui_proto::ipc::AppEvent;
use neoui_proto::island::{self as model, ALERT_MS, Change, Island, MEDIA_ID, UPDATE_MS};
use neoui_proto::page::style;
use neoui_proto::settings::{Value, kerr, keys};
use neoui_render::Renderer;

/// The island as last drawn, and its alerts.
#[derive(Default)]
pub struct Live {
    list: Island<&'static str>,
    /// what was drawn (every activity in order, the shown ones), when (ms) and at which unix second
    drawn: Vec<(&'static str, Activity)>,
    shown: Vec<usize>,
    drawn_at: u64,
    second: u64,
    on: Option<bool>,
    /// the alert's end (ms, 0 none); the ringer and the charger as last seen (none before the first frame)
    alert_until: u64,
    seen: Option<(bool, bool)>,
    /// the unix second the shade's Chrono cards were last ticked at
    chrono: u64,
}

impl Host {
    /// The apps turned off for the island.
    fn island_denied(&self, key: &str) -> bool {
        match self.settings.get_raw(keys::ISLAND_DENY.name()) {
            Ok(Value::Bytes(b)) => {
                activity::decode_deny(&b).is_some_and(|d| d.iter().any(|k| k == key))
            }
            _ => false,
        }
    }

    /// `ACTIVITY` from app `key`.
    pub(super) fn activity_message(&mut self, key: &'static str, m: &Message) -> i32 {
        let a = match contract::activity(key, m.client_sid(), |max| read_bytes(m, 0, max)) {
            Ok(a) => a,
            Err(e) => {
                dprintln!("neoui: activity from {} refused ({})", key, e);
                return e;
            }
        };
        if !a.is_end() && self.island_denied(key) {
            dprintln!("neoui: activity from {} refused (island.deny)", key);
            return kerr::PERMISSION_DENIED;
        }
        let (id, kind, end) = (a.id, a.kind, a.is_end());
        match self.live.list.post(key, a, self.front == Some(key)) {
            Ok(Change::None) => {}
            Ok(_) if end => dprintln!("neoui: activity {} {} ends", key, id),
            Ok(Change::Shape) => dprintln!("neoui: activity {} {} kind {}", key, id, kind),
            Ok(_) => {}
            Err(e) => {
                dprintln!("neoui: activity from {} refused (not in front)", key);
                return e;
            }
        }
        0
    }

    /// Each pass of the main loop: the island's owners that exited or were turned off lose their activities, the
    /// taps go to their apps, the alerts come and go, and the island is drawn again when its shape changed, when an
    /// update is a second old, or each second while a time ticks.
    pub fn island_frame(&mut self, ui: &mut Renderer, ms: u64) {
        let gone: Vec<&'static str> = self
            .live
            .list
            .keys()
            .filter(|k| {
                self.apps
                    .find(|a| a.key == *k && a.quit_at.is_none())
                    .is_none()
                    || self.island_denied(k)
            })
            .collect();
        for k in gone {
            if self.live.list.end_app(k) {
                dprintln!("neoui: activities of {} end", k);
            }
        }
        for (key, id, i) in ui.take_island() {
            self.island_tap(&key, id, i, ms);
        }
        // the 2 s alerts
        let ringer_off = self.settings.get(keys::SOUND_SILENT).unwrap_or(false)
            || self.settings.get(keys::SOUND_RING_VOLUME).unwrap_or(8) == 0;
        let (charger, percent) = ui.charger();
        if let Some((was_off, was_in)) = self.live.seen {
            let alert = if ringer_off && !was_off {
                if self.settings.get(keys::SOUND_VIBRATE).unwrap_or(true) {
                    Some((2, alloc::string::String::from("Vibrate")))
                } else {
                    Some((1, "Silent".into()))
                }
            } else if charger && !was_in {
                Some((
                    3,
                    percent.map_or("Charging".into(), |p| alloc::format!("Charging · {p}%")),
                ))
            } else {
                None
            };
            if let Some((kind, text)) = alert {
                dprintln!("neoui: island alert {}", text);
                ui.set_island_alert(Some((kind, &text)));
                self.live.alert_until = ms + ALERT_MS;
            }
        }
        self.live.seen = Some((ringer_off, charger));
        if self.live.alert_until != 0 && ms >= self.live.alert_until {
            self.live.alert_until = 0;
            ui.set_island_alert(None);
        }
        let on = self.settings.get(keys::ISLAND_ON).unwrap_or(true);
        if self.live.on != Some(on) {
            ui.set_island_on(on);
            self.live.on = Some(on);
        }
        // what to draw: the apps' activities and the media session's, in order; the two shown
        let media = self
            .media
            .current
            .shown()
            .filter(|(_, _, live)| *live)
            .and_then(|(k, s, _)| model::from_media(s).map(|a| (k, a)));
        let all = self.live.list.ordered(media);
        let shown: Vec<usize> = model::shown(&all, self.front_full())
            .iter()
            .filter_map(|(k, a)| all.iter().position(|(k2, b)| k2 == k && b.id == a.id))
            .collect();
        let now = unix_ms();
        let shape = |v: &[(&'static str, Activity)]| -> Vec<(&'static str, u8, u8)> {
            v.iter().map(|(k, a)| (*k, a.id, a.kind)).collect()
        };
        let ticks = all.iter().any(|(_, a)| model::ticks(&a.trail));
        let redraw = shape(&all) != shape(&self.live.drawn)
            || shown != self.live.shown
            || (all != self.live.drawn && ms.saturating_sub(self.live.drawn_at) >= UPDATE_MS)
            || (ticks && now / 1000 != self.live.second);
        if redraw {
            let live: Vec<(&str, &Activity)> = all.iter().map(|(k, a)| (*k, a)).collect();
            ui.set_island(&live, &shown, now);
            self.live.drawn = all;
            self.live.shown = shown;
            self.live.drawn_at = ms;
            self.live.second = now / 1000;
        }
        // the shade's Chrono cards (a timer, a stopwatch) tick each second
        if now / 1000 != self.live.chrono
            && self
                .notifications
                .iter()
                .any(|(_, n)| n.style == style::CHRONO && n.when != 0)
        {
            self.live.chrono = now / 1000;
            self.notes_changed = true;
        }
    }

    /// How long (µs) until the island's next tick or alert's end, for the main loop's wait.
    pub fn island_wake_us(&self, ms: u64) -> Option<u64> {
        let tick = self
            .live
            .drawn
            .iter()
            .any(|(_, a)| model::ticks(&a.trail))
            .then(|| (1000 - unix_ms() % 1000) * 1000);
        let alert =
            (self.live.alert_until != 0).then(|| self.live.alert_until.saturating_sub(ms) * 1000);
        tick.into_iter().chain(alert).min()
    }

    /// A tap on activity `id` of app `key` (`i` 0), or one of its keys: the activity itself brings its app forward on
    /// its page; a key goes to the app where it is. The media session's opens its player.
    fn island_tap(&mut self, key: &str, id: u8, i: u8, now_ms: u64) {
        let Some(key) = self.live.drawn.iter().map(|(k, _)| *k).find(|k| *k == key) else {
            return;
        };
        dprintln!("neoui: island {} {} tap {}", key, id, i);
        let page = self
            .live
            .drawn
            .iter()
            .find(|(k, a)| *k == key && a.id == id)
            .map_or(0, |(_, a)| a.page);
        if i == 0 {
            self.limit_running(key, now_ms);
            self.start(key);
            if id != MEDIA_ID {
                self.post(key, AppEvent::Page { page });
            }
            self.requested_screen = Some(key);
        }
        if id != MEDIA_ID {
            self.post(key, AppEvent::ActivityTap { id, i });
        }
    }
}

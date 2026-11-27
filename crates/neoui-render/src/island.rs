// SPDX-License-Identifier: Apache-2.0
//! (TP6) The live island in the shell (docs `neoui-shell/top-panel.md` "Live island (TP6)"): the `Island` global
//! (ui/island.slint) that the status bar's pill and circle, the island held open and the shade's Live cards draw, and
//! what their taps and keys send back (the activity's app and id, and `i`: 0 the activity, 1–2 its app's buttons,
//! `activity::key` a call's keys).

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use neoui_proto::Rect;
use neoui_proto::activity::{Activity, flag, kind};
use neoui_proto::apps;
use neoui_proto::island::{self as model, trail_text};
use slint::{ComponentHandle, Model};

use crate::{Island, IslandItem, Renderer, Shell};

/// The activities as drawn (app key, id), by their index in `Island.live`, and the taps not yet taken.
#[derive(Default)]
pub(crate) struct State {
    rows: Rc<RefCell<Vec<(String, u8)>>>,
    acts: Rc<RefCell<Vec<(String, u8, u8)>>>,
}

pub(crate) fn wire(ui: &Shell) -> State {
    let s = State::default();
    let (rows, acts) = (s.rows.clone(), s.acts.clone());
    ui.global::<Island>().on_act(move |i, k| {
        let row = usize::try_from(i)
            .ok()
            .and_then(|i| rows.borrow().get(i).cloned());
        if let (Some((key, id)), Ok(k)) = (row, u8::try_from(k)) {
            acts.borrow_mut().push((key, id, k));
        }
    });
    s
}

fn item(key: &str, a: &Activity, now: u64) -> IslandItem {
    let tint = model::tint(a.tint, a.kind);
    IslandItem {
        kind: i32::from(a.kind),
        title: a.title.as_str().into(),
        text: a.text.as_str().into(),
        app: apps::find(key).map_or(key, |x| x.name).into(),
        app_key: key.into(),
        trail: trail_text(&a.trail, now).into(),
        ring: model::ring(&a.trail),
        tint: slint::Color::from_argb_encoded(0xff00_0000 | tint),
        initials: if a.kind == kind::CALL {
            model::initials(&a.title).into()
        } else {
            "".into()
        },
        muted: a.flags & flag::MUTED != 0,
        speaker: a.flags & flag::SPEAKER != 0,
        buttons: slint::ModelRc::from(Rc::new(slint::VecModel::from(
            a.buttons
                .iter()
                .map(|b| b.as_str().into())
                .collect::<Vec<slint::SharedString>>(),
        ))),
    }
}

impl Renderer {
    /// The activities, every one in the island's order (`neoui_proto::island::Island::ordered`, the shade's Live
    /// cards), `shown` (indexes into `live`) in the island; their times as at `now` (unix ms: call again each second
    /// while one ticks). The activity held open stays open while it lasts.
    pub fn set_island(&mut self, live: &[(&str, &Activity)], shown: &[usize], now: u64) {
        let g = self.ui.global::<Island>();
        let open = usize::try_from(g.get_expanded())
            .ok()
            .and_then(|i| self.island.rows.borrow().get(i).cloned());
        let rows: Vec<(String, u8)> = live.iter().map(|(k, a)| ((*k).into(), a.id)).collect();
        let open = open.and_then(|o| rows.iter().position(|r| *r == o));
        *self.island.rows.borrow_mut() = rows;
        let items: Vec<IslandItem> = live.iter().map(|(k, a)| item(k, a, now)).collect();
        g.set_cards(
            i32::try_from(live.iter().filter(|(_, a)| a.kind != kind::MEDIA).count()).unwrap_or(0),
        );
        g.set_live(slint::ModelRc::from(Rc::new(slint::VecModel::from(items))));
        g.set_shown(slint::ModelRc::from(Rc::new(slint::VecModel::from(
            shown
                .iter()
                .filter(|i| **i < live.len())
                .map(|i| *i as i32)
                .collect::<Vec<i32>>(),
        ))));
        g.set_expanded(open.map_or(-1, |i| i as i32));
        if shown.len() < 2 {
            g.set_swapped(false);
        }
    }

    /// `island.on`: off, the status bar draws no island (the shade's Live cards stay).
    pub fn set_island_on(&mut self, on: bool) {
        self.ui.global::<Island>().set_on(on);
    }

    /// A transient system alert in the island's place (`kind` 1 ringer silent, 2 vibrate, 3 charger in), or none.
    pub fn set_island_alert(&mut self, alert: Option<(i32, &str)>) {
        let g = self.ui.global::<Island>();
        let (k, t) = alert.unwrap_or((0, ""));
        g.set_alert_kind(k);
        g.set_alert(t.into());
    }

    /// The island's and the Live cards' taps since the last call: (app key, activity id, `i`).
    pub fn take_island(&mut self) -> Vec<(String, u8, u8)> {
        core::mem::take(&mut *self.island.acts.borrow_mut())
    }

    /// Whether the island is held open.
    #[must_use]
    pub fn island_open(&self) -> bool {
        self.ui.global::<Island>().get_expanded() >= 0
    }

    /// A touch went to the app: the island held open folds (a tap outside it).
    pub fn fold_island(&mut self) {
        self.ui.global::<Island>().set_expanded(-1);
    }

    /// The island while it lies over the app (held open, or floating over a full-screen call), in physical pixels:
    /// the UI server copies it from the frame again after composing the app's surface, and keeps touches in it from
    /// the app.
    pub fn island_rect(&self) -> Option<Rect> {
        self.ui.get_island_over().then(|| {
            Rect::new(
                self.ui.get_island_x() as i32,
                self.ui.get_island_y() as i32,
                // a pixel more each way: its edges may fall between pixels
                self.ui.get_island_w() as u32 + 1,
                self.ui.get_island_h() as u32 + 1,
            )
        })
    }

    /// The charger as the status bar shows it (plugged in), and the battery's percent: the island's "Charging" alert.
    #[must_use]
    pub fn charger(&self) -> (bool, Option<u8>) {
        let p = self.ui.get_battery_percent();
        (
            self.ui.get_battery_connected() || self.ui.get_battery_charging(),
            u8::try_from(p).ok(),
        )
    }

    /// The activities as the island has them (app key, id), for tests.
    #[must_use]
    pub fn island_rows(&self) -> Vec<(String, u8)> {
        let g = self.ui.global::<Island>();
        let rows = self.island.rows.borrow();
        g.get_shown()
            .iter()
            .filter_map(|i| rows.get(usize::try_from(i).ok()?).cloned())
            .collect()
    }
}

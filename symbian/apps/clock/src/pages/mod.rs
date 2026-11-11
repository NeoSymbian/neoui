// SPDX-License-Identifier: GPL-3.0-only
//! The pages: each module binds its `ClockState` callbacks ([`bind`]) and fills its properties from the [`Model`]
//! ([`show`] after a change, [`tick`] as time passes). [`Ctx`] is what every callback shares: the connection to the
//! UI server (to save settings and claim Back), the window and the model.

mod alarms;
mod face;
mod ringing;
mod set_time;
mod stopwatch;
mod timer;
mod world;

pub use ringing::ring;

use crate::state::{Model, Part, View};
use crate::{ClockApp, ClockState};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use e32::{dprintln, time};
use neoapp::App;
use neoui_proto::clock::face::LocalTime;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

/// The time a refresh shows.
#[derive(Clone, Copy, Debug)]
pub struct Now {
    pub local: LocalTime,
    /// UTC, Unix seconds.
    pub utc: i64,
    /// The local zone, seconds east of UTC.
    pub tz: i32,
    /// UTC, Unix milliseconds (the timer and stopwatch).
    pub wall_ms: i64,
    pub second: u32,
}

impl Now {
    pub fn read() -> Self {
        let t = time::now();
        let (utc, tz) = time::utc_now();
        Self {
            local: LocalTime {
                year: t.year,
                month: t.month,
                day: t.day,
                weekday: t.weekday,
                hour: t.hour,
                minute: t.minute,
            },
            utc,
            tz,
            wall_ms: neoapp::wall_ms(),
            second: t.second,
        }
    }
}

/// What the callbacks share.
pub struct Ctx {
    pub app: App,
    pub ui: slint::Weak<ClockApp>,
    pub model: RefCell<Model>,
}

impl Ctx {
    /// Render the change; a scrolled list moves its rows instead of being redrawn (smooth-ui S2d, as Files).
    pub fn draw(&self) -> Result<(), i32> {
        let Some(ui) = self.ui.upgrade() else {
            return self.app.draw();
        };
        self.app.draw_scrolled(|| {
            let sc = ui.global::<crate::Scroll>();
            let moved = (sc.get_dy(), sc.get_lists());
            sc.set_dy(0);
            sc.set_lists(0);
            let list = neoui_proto::Rect::new(
                sc.get_x() as i32,
                sc.get_y() as i32,
                sc.get_width() as u32,
                sc.get_height() as u32,
            );
            (moved, list)
        })
    }

    /// Changes the model, saves the part it names, and shows the result.
    pub fn update(&self, f: impl FnOnce(&mut Model, &Now) -> Option<Part>) {
        let now = Now::read();
        let part = f(&mut self.model.borrow_mut(), &now);
        if let Some(part) = part {
            self.model.borrow().save(&self.app, part);
        }
        self.show(&now);
    }

    /// Opens a sub-view over the tabs (or closes it with [`View::None`]); while one is open, Back closes it.
    pub fn open(&self, view: View) {
        self.model.borrow_mut().view = view;
        if let Err(e) = self.app.claim_nav(view != View::None, false) {
            dprintln!("clock: cannot claim Back ({})", e);
        }
        dprintln!("clock: view {}", view.name());
        self.show(&Now::read());
    }

    /// Fills every page from the model.
    pub fn show(&self, now: &Now) {
        let Some(ui) = self.ui.upgrade() else { return };
        let m = self.model.borrow();
        let s = ui.global::<ClockState>();
        s.set_tab(m.tab as i32);
        s.set_view(m.view.name().into());
        face::show(&s, &m, now);
        set_time::show(&s, &m);
        alarms::show(&s, &m);
        world::show(&s, &m, now);
        timer::show(&s, &m, now);
        stopwatch::show(&s, &m, now);
        ringing::show(&s, &m);
    }

    /// As time passes: the parts that move by themselves. Returns true while something moves faster than once a
    /// second (the stopwatch on screen), so the caller ticks quicker.
    pub fn tick(&self, now: &Now) -> bool {
        let Some(ui) = self.ui.upgrade() else {
            return false;
        };
        let s = ui.global::<ClockState>();
        let finished = self.model.borrow_mut().timer.tick(now.wall_ms);
        if finished {
            dprintln!("clock: timer done");
            self.model.borrow().save(&self.app, Part::Timer);
        }
        let m = self.model.borrow();
        face::show(&s, &m, now);
        timer::show(&s, &m, now);
        stopwatch::show(&s, &m, now);
        if now.second == 0 {
            world::show(&s, &m, now);
        }
        m.view == View::None && m.tab == crate::state::Tab::Stopwatch && m.stopwatch.is_running()
    }
}

/// Binds every page's callbacks.
pub fn bind(ctx: &Rc<Ctx>, ui: &ClockApp) {
    let s = ui.global::<ClockState>();
    let c = ctx.clone();
    s.on_tab_chosen(move |i| {
        c.update(|m, _| {
            m.tab = crate::state::Tab::from_index(i);
            dprintln!("clock: tab {}", i);
            Some(Part::Tab)
        });
    });
    let c = ctx.clone();
    s.on_close(move || c.open(View::None));
    face::bind(ctx, &s);
    set_time::bind(ctx, &s);
    alarms::bind(ctx, &s);
    world::bind(ctx, &s);
    timer::bind(ctx, &s);
    stopwatch::bind(ctx, &s);
    ringing::bind(ctx, &s);
}

/// A Slint list model from strings.
fn strings<S: Into<SharedString>>(items: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
}

/// A Slint list model from rows.
fn rows<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

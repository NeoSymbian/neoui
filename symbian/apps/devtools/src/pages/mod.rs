// SPDX-License-Identifier: GPL-3.0-only
//! The pages: each module binds its `DevState` callbacks ([`bind`]) and fills its properties from the [`Model`]
//! (`show`). [`Ctx`] is what every callback shares: the connection to the UI server, the window and the model; it
//! also refreshes the model from the system ([`Ctx::refresh_log`], [`Ctx::refresh_stats`], [`Ctx::refresh_devices`])
//! and shows only what is on screen.

mod console;
mod device;
mod devices;
pub mod home;
mod more;
mod procs;
mod tests;

pub use console::follow_new_lines;

use crate::probe;
use crate::state::{Model, Registry, Tab, View};
use crate::{DevApp, DevState};
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use e32::{dprintln, time};
use neoapp::{App, hal, sys};
use neoui_proto::devtools::device::{State, component, merge};
use neoui_proto::devtools::stats::{CpuSample, cpu_load};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

/// What the callbacks share.
pub struct Ctx {
    pub app: App,
    pub ui: slint::Weak<DevApp>,
    pub model: RefCell<Model>,
    /// The Console's rows: a fixed-size model whose rows change in place.
    pub console_rows: Rc<VecModel<SharedString>>,
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

    /// Changes the model, then shows the result.
    pub fn update(&self, f: impl FnOnce(&mut Model)) {
        f(&mut self.model.borrow_mut());
        self.show();
    }

    /// Opens a sub-view over the tabs (or closes it with [`View::None`]); while one is open, Back closes it.
    pub fn open(&self, view: View) {
        self.model.borrow_mut().view = view;
        if let Err(e) = self.app.claim_nav(view != View::None, false) {
            dprintln!("devtools: cannot claim Back ({})", e);
        }
        dprintln!("devtools: view {}", view.name());
        self.show();
    }

    /// Opens a page by name: a tab, or a sub-view (Home's list, More's list).
    pub fn open_page(&self, name: &str) {
        let tab = match name {
            "devices" => Some(Tab::Devices),
            "console" => Some(Tab::Console),
            "procs" => Some(Tab::Procs),
            _ => None,
        };
        match (tab, name) {
            (Some(t), _) => self.choose_tab(t),
            (None, "settings") => self.open(View::Settings),
            (None, "tools") => self.open(View::Tools),
            (None, "about") => self.open(View::About),
            _ => {}
        }
    }

    pub fn choose_tab(&self, tab: Tab) {
        self.model.borrow_mut().tab = tab;
        dprintln!("devtools: tab {}", tab as i32);
        // what a tab shows is refreshed on the way in, not a tick later
        match tab {
            Tab::Devices | Tab::Home => self.refresh_devices(),
            Tab::Procs => self.refresh_stats(),
            _ => {}
        }
        self.show();
    }

    /// Fills the tab or sub-view on screen from the model.
    pub fn show(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let m = self.model.borrow();
        let s = ui.global::<DevState>();
        s.set_tab(m.tab as i32);
        s.set_view(m.view.name().into());
        match m.view {
            View::None => match m.tab {
                Tab::Home => home::show(&s, &m),
                Tab::Devices => devices::show(&s, &m),
                Tab::Console => console::show(&s, &m, &self.console_rows),
                Tab::Procs => procs::show(&s, &m),
                Tab::More => more::show_more(&s),
            },
            View::Device => device::show(&s, &m),
            View::Proc => procs::show_proc(&s, &m),
            View::Settings => more::show_settings(&s, &self.app),
            View::Tools => more::show_tools(&s, &m),
            View::About => more::show_about(&s, &m),
            View::TouchTest => tests::show_touch(&s, &m),
            View::DisplayTest => tests::show_display(&s, &m),
        }
    }

    /// Reads the kernel log written since the last read; true if there was any.
    pub fn refresh_log(&self) -> bool {
        let mut m = self.model.borrow_mut();
        let mut bytes = Vec::new();
        let mut cursor = m.cursor;
        match hal::log_read(&mut cursor, &mut bytes) {
            Ok(()) => {
                m.cursor = cursor;
                m.log_error = None;
                m.live.push(&bytes);
                !bytes.is_empty()
            }
            Err(e) => {
                m.log_error = Some(e);
                false
            }
        }
    }

    /// Memory, processes, threads and the CPU load (from the threads' CPU time, when the kernel measures it).
    pub fn refresh_stats(&self) {
        let memory = sys::memory().ok();
        let threads = sys::threads();
        let procs = sys::processes();
        let mut m = self.model.borrow_mut();
        if let Some(mem) = memory
            && mem.total_ram > 0
        {
            m.mem_history
                .push(1.0 - mem.free_ram as f32 / mem.total_ram as f32);
        }
        m.memory = memory;
        // every thread's time but the idle ones'; None as soon as one is not measured
        let busy: Option<u64> = threads
            .iter()
            .filter(|t| !t.name.starts_with("Null"))
            .map(|t| t.cpu_us)
            .sum();
        let sample = busy.map(|busy_us| CpuSample {
            busy_us,
            wall_us: time::millis() * 1000,
        });
        m.cpu_load = match (m.cpu_prev, sample) {
            (Some(prev), Some(now)) => cpu_load(prev, now, m.cpus),
            _ => None,
        };
        if let Some(l) = m.cpu_load {
            m.cpu_history.push(l);
        }
        m.cpu_prev = sample;
        m.threads = threads;
        m.procs = procs;
        if m.proc >= m.procs.len() {
            m.proc = 0;
        }
    }

    /// The probes' readings, the kernel's device registry, and the merged device list.
    pub fn refresh_devices(&self) {
        let readings = probe::read();
        let kernel = probe::kernel_list();
        let mut m = self.model.borrow_mut();
        let probes = probe::probes(&readings, m.cpus, m.cpu_khz);
        let (list, registry) = match kernel {
            Ok((list, bad)) => {
                let devices = list.len();
                (list, Registry::Listed { devices, bad })
            }
            Err(e) => (Vec::new(), Registry::Missing(e)),
        };
        if m.registry != registry {
            dprintln!("devtools: kernel registry {:?}", registry);
        }
        let live = &m.live;
        let mut devices = merge(&list, &probes, |tag| live.has_tag(tag));
        if matches!(registry, Registry::Missing(_)) {
            // this kernel cannot say: a part some board's driver reports may well be driven here
            for d in devices.iter_mut().filter(|d| d.state == State::NoDriver) {
                if component(&d.key).is_some_and(|c| !c.kernel.is_empty()) {
                    d.state = State::Unknown;
                    d.detail = "this kernel reports no device status".into();
                }
            }
        }
        m.registry = registry;
        m.devices = devices;
        m.readings = readings;
        if m.device >= m.devices.len() {
            m.device = 0;
        }
    }

    /// Runs every device's check (Devices → Run checks).
    pub fn run_checks(&self) {
        let mut m = self.model.borrow_mut();
        let checks: Vec<_> = m
            .devices
            .iter()
            .map(|d| {
                let c = probe::check(d, &m.readings);
                dprintln!("devtools: check {} {} {}", d.key, c.word(), c.text());
                (d.key.clone(), c)
            })
            .collect();
        let (pass, fail) = (
            checks.iter().filter(|(_, c)| c.word() == "pass").count(),
            checks.iter().filter(|(_, c)| c.word() == "fail").count(),
        );
        dprintln!(
            "devtools: checks {} pass, {} fail, {} skipped",
            pass,
            fail,
            checks.len() - pass - fail
        );
        m.checks = checks;
    }
}

/// Binds every page's callbacks.
pub fn bind(ctx: &Rc<Ctx>, ui: &DevApp) {
    let s = ui.global::<DevState>();
    let c = ctx.clone();
    s.on_tab_chosen(move |i| c.choose_tab(Tab::from_index(i)));
    let c = ctx.clone();
    s.on_open(move |name| c.open_page(name.as_str()));
    devices::bind(ctx, &s);
    device::bind(ctx, &s);
    console::bind(ctx, &s);
    procs::bind(ctx, &s);
    more::bind(ctx, &s);
    tests::bind(ctx, &s);
}

/// A Slint list model from items.
fn rows<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

/// A Slint list model from strings.
fn strings<S: Into<SharedString>>(items: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    rows(items.into_iter().map(Into::into))
}

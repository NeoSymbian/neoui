// SPDX-License-Identifier: GPL-3.0-only
//! The Dev tools app (`NEODEV.EXE`): the phone as a developer sees it. Five tabs (Home, Devices, Console, Procs,
//! More) and their sub-views, drawn with Slint into the surface `NeoUi` hands over ([`neoapp`]).
//!
//! - Devices: every component, its driver and live state, from the kernel's device registry (HAL 0x4e19), the
//!   app's own probes and the kernel log ([`neoui_proto::devtools::device`]), with checks and tests per device.
//! - Console: the kernel log, live or from the previous boot, filtered by tag.
//! - Procs, Settings, Tools, About: processes and threads, every setting, the essential tools, the builds.
//!
//! It is a ROM EXE, so it holds the capabilities the kernel's calls need (ReadDeviceData, WriteDeviceData, CommDD).
//! Boards without a call (Syborg's 2010 kernel) answer -5, and the pages say so.
//!
//! - [`state`]: the model
//! - [`probe`]: reading and testing the hardware
//! - [`pages`]: one module per page (callbacks in, properties out)
//! - this file: connecting, and the event loop (input, Back, refreshing as time passes)
#![no_std]

extern crate alloc;

mod pages;
mod probe;
mod state;

use alloc::rc::Rc;
use core::cell::RefCell;
use e32::rt;
use e32::{dprintln, time};
use neoapp::{App, Ticker};
use neoui_proto::ipc::AppEvent;
use neoui_proto::{InputEvent, Rect, TouchPhase};
use pages::Ctx;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use state::{Model, Tab as DevTab, View};

slint::include_modules!();

e32::main!(main);

/// How often the loop wakes: the log is read each time.
const TICK_US: u32 = 500_000;
/// Memory, processes and the CPU load.
const STATS_MS: u64 = 1_000;
/// The probes and the kernel's device registry.
const DEVICES_MS: u64 = 2_000;
/// The Console's text size and the space above its rows (chips, tags, status) and below (the tab bar), in dp.
const MONO_DP: f32 = 11.5;
const CONSOLE_CHROME_DP: f32 = 112.0 + 46.0;

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("devtools: failed {}", e);
            e
        }
    }
}

fn round(v: f32) -> f32 {
    (v + 0.5) as i32 as f32
}

/// Rows of log text that fit the Console in a window `h` px high.
fn console_rows(h: usize, dp: f32, cell: f32) -> usize {
    ((h as f32 - CONSOLE_CHROME_DP * dp) / cell).max(1.0) as usize
}

fn run() -> Result<(), i32> {
    let app = App::connect()?;
    let ui = DevApp::new().map_err(|_| -2)?;
    let dp = app.dp;
    ui.global::<Theme>().set_dp(dp);
    let mono = round(MONO_DP * dp);
    let cell = round(mono * 1.25);
    let rows_fit = console_rows(app.height(), dp, cell);
    let state = ui.global::<DevState>();
    state.set_mono_px(mono);
    state.set_cell_h(cell);
    let rows = Rc::new(VecModel::from(
        (0..rows_fit)
            .map(|_| SharedString::new())
            .collect::<alloc::vec::Vec<_>>(),
    ));
    state.set_console_rows(ModelRc::from(rows.clone()));
    ui.show().map_err(|_| -2)?;
    if let Err(e) = app.watch_settings("") {
        dprintln!("devtools: cannot watch settings ({})", e);
    }
    // neoapp's own frames (animations, Slint timers) move scrolled rows too, as Ctx::draw does
    let weak = ui.as_weak();
    app.set_scrolled(move || {
        weak.upgrade()
            .map_or(((0, 0), Rect::default()), |ui| pages::scrolled(&ui))
    });
    let mut model = Model::new(rows_fit, (app.width, app.height()));
    model.cpus = neoapp::sys::cpus();
    model.cpu_khz = neoapp::sys::cpu_max_khz();
    let ctx = Rc::new(Ctx {
        app,
        ui: ui.as_weak(),
        model: RefCell::new(model),
        console_rows: rows,
        gnss: RefCell::new(None),
    });
    pages::bind(&ctx, &ui);
    ctx.refresh_log();
    ctx.refresh_stats();
    ctx.refresh_devices();
    {
        let m = ctx.model.borrow();
        let count = |src: &str| m.devices.iter().filter(|d| d.source.label() == src).count();
        dprintln!(
            "devtools: devices {} (kernel {}, probe {}, log {}); procs {} threads {}; {} console rows; {}",
            m.devices.len(),
            count("kernel"),
            count("probe"),
            count("log"),
            m.procs.len(),
            m.threads.len(),
            rows_fit,
            m.build.first().map_or("", alloc::string::String::as_str)
        );
    }
    ctx.open(View::None); // shows the tabs and describes the page
    ctx.draw()?; // nothing until Visible(true): neoapp asks for the surface when shown

    // In the background (R10) the app neither refreshes nor draws; the log is still followed (cheap, and the
    // kernel's ring would drop what it is not asked for).
    let mut visible = false;
    // a finger is down: the periodic refresh waits, since replacing the list models under a press loses the drag
    // (the Flickable takes it only after 8 px) and the list wouldn't scroll
    let mut touching = false;
    let (mut stats_at, mut devices_at) = (time::millis(), time::millis());
    rt::block_on(async {
        loop {
            // one NEXT_EVENT stays outstanding while the log is followed (neoapp's Ticker); Slint's timers and
            // animations get their frames inside next_event
            let mut ticker = || -> Result<(), i32> {
                // nor during a fling (Slint animates the list)
                let refresh = visible && !touching && !ctx.app.window.has_active_animations();
                if tick(&ctx, refresh, &mut stats_at, &mut devices_at) {
                    ctx.draw()?;
                }
                Ok(())
            };
            let e = ctx
                .app
                .next_event(Some(Ticker {
                    every_us: TICK_US,
                    f: &mut ticker,
                }))
                .await?;
            match e {
                AppEvent::Quit => {
                    dprintln!("devtools: quit");
                    return Ok(());
                }
                AppEvent::Visible(v) => {
                    dprintln!("devtools: visible {}", v);
                    visible = v;
                    if v {
                        // what the page shows, fresh, at once
                        ctx.refresh_stats();
                        ctx.refresh_devices();
                        (stats_at, devices_at) = (time::millis(), time::millis());
                        ctx.show();
                    }
                }
                AppEvent::Area { h } if h > 0 => {
                    let n = console_rows(h as usize, dp, cell);
                    dprintln!("devtools: area {} px, {} console rows", h, n);
                    ctx.resize_console(n, (ctx.app.width, h as usize));
                }
                AppEvent::Settings { .. } => {
                    if ctx.model.borrow().view == View::Settings {
                        ctx.show();
                    }
                }
                // the kit's layers first (a menu, a sheet), then a sub-view (kept only while one is open)
                AppEvent::Back if neoapp::kit_back!(ui) => {}
                AppEvent::Back => ctx.back(),
                // App::handle already gave the key to Slint (key-down only)
                AppEvent::Input(InputEvent::Key { code, down }) => {
                    ctx.model.borrow_mut().last_key = Some((code, down));
                    dprintln!(
                        "devtools: key {:#x} {}",
                        code,
                        if down { "down" } else { "up" }
                    );
                    ctx.show();
                }
                AppEvent::Input(InputEvent::Touch { phase, .. }) => {
                    touching = phase != TouchPhase::Up;
                }
                _ => {}
            }
            ctx.draw()?;
        }
    })
}

/// As time passes: follow the log; while `visible` (and not touched), refresh what the screen shows. True when something to draw changed.
fn tick(ctx: &Ctx, visible: bool, stats_at: &mut u64, devices_at: &mut u64) -> bool {
    let grew = ctx.refresh_log();
    if grew {
        pages::follow_new_lines(&mut ctx.model.borrow_mut());
    }
    if !visible {
        return false;
    }
    let now = time::millis();
    let (tab, view) = {
        let m = ctx.model.borrow();
        (m.tab, m.view)
    };
    let mut changed = grew
        && matches!(
            (tab, view),
            (DevTab::Console, View::None) | (_, View::Device)
        );
    // Home shows everything; the other pages what they list
    let wants_stats =
        view == View::None && matches!(tab, DevTab::Home | DevTab::Procs) || view == View::Proc;
    let wants_devices =
        view == View::None && matches!(tab, DevTab::Home | DevTab::Devices) || view == View::Device;
    if wants_stats && now.saturating_sub(*stats_at) >= STATS_MS {
        *stats_at = now;
        ctx.refresh_stats();
        changed = true;
    }
    if wants_devices && now.saturating_sub(*devices_at) >= DEVICES_MS {
        *devices_at = now;
        ctx.refresh_devices();
        changed = true;
    }
    if changed {
        ctx.show();
    }
    changed
}

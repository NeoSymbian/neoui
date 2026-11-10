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
use core::pin::pin;
use e32::ipc::TIpcArgs;
use e32::rt::{self, Either};
use e32::sys::TPtr8;
use e32::{dprintln, time};
use neoapp::App;
use neoui_proto::InputEvent;
use neoui_proto::ipc::{self as proto, AppEvent};
use pages::Ctx;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use state::{Model, Tab, View};

slint::include_modules!();

e32::main!(main);

/// How often the loop wakes: the log is read each time.
const TICK_US: i32 = 500_000;
/// While an animation runs.
const FRAME_US: i32 = 16_000;
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

fn run() -> Result<(), i32> {
    let app = App::connect()?;
    let ui = DevApp::new().map_err(|_| -2)?;
    let dp = app.dp;
    ui.global::<Theme>().set_dp(dp);
    let mono = round(MONO_DP * dp);
    let cell = round(mono * 1.25);
    let console_rows = ((app.height as f32 - CONSOLE_CHROME_DP * dp) / cell).max(1.0) as usize;
    let state = ui.global::<DevState>();
    state.set_mono_px(mono);
    state.set_cell_h(cell);
    let rows = Rc::new(VecModel::from(
        (0..console_rows)
            .map(|_| SharedString::new())
            .collect::<alloc::vec::Vec<_>>(),
    ));
    state.set_console_rows(ModelRc::from(rows.clone()));
    ui.show().map_err(|_| -2)?;
    if let Err(e) = app.watch_settings("") {
        dprintln!("devtools: cannot watch settings ({})", e);
    }
    let mut model = Model::new(console_rows, (app.width, app.height));
    model.cpus = neoapp::sys::cpus();
    model.cpu_khz = neoapp::sys::cpu_max_khz();
    let ctx = Rc::new(Ctx {
        app,
        ui: ui.as_weak(),
        model: RefCell::new(model),
        console_rows: rows,
    });
    pages::bind(&ctx, &ui);
    ctx.refresh_log();
    ctx.refresh_stats();
    ctx.refresh_devices();
    {
        let m = ctx.model.borrow();
        let kernel = m
            .devices
            .iter()
            .filter(|d| d.source.label() == "kernel")
            .count();
        let probed = m
            .devices
            .iter()
            .filter(|d| d.source.label() == "probe")
            .count();
        let logged = m
            .devices
            .iter()
            .filter(|d| d.source.label() == "log")
            .count();
        dprintln!(
            "devtools: devices {} (kernel {}, probe {}, log {}); procs {} threads {}; {} console rows; {}",
            m.devices.len(),
            kernel,
            probed,
            logged,
            m.procs.len(),
            m.threads.len(),
            console_rows,
            m.build.first().map_or("", alloc::string::String::as_str)
        );
    }
    ctx.show();
    ctx.app.draw()?;

    // In the background (R10) the app neither refreshes nor draws; the log is still followed (cheap, and the
    // kernel's ring would drop what it is not asked for).
    let mut visible = true;
    let (mut stats_at, mut devices_at) = (time::millis(), time::millis());
    rt::block_on(async {
        loop {
            let mut ev = [0u8; proto::EVENT_LEN];
            let got = {
                let mut des = TPtr8::new(&mut ev);
                let mut args = TIpcArgs::new();
                args.des8(0, &mut des);
                let mut next = pin!(ctx.app.server.send(proto::NEXT_EVENT, &args));
                loop {
                    // a running Slint animation (the tab underline) needs frames until it ends
                    let animating = ctx.app.window.has_active_animations();
                    let wait = if animating { FRAME_US } else { TICK_US };
                    match rt::select(next.as_mut(), time::delay(wait)).await {
                        Either::Left(r) => break r,
                        Either::Right(()) => {
                            if tick(&ctx, visible, &mut stats_at, &mut devices_at) || animating {
                                ctx.app.draw()?;
                            }
                        }
                    }
                }
            };
            got?;
            match AppEvent::decode(&ev) {
                Some(AppEvent::Quit) => {
                    dprintln!("devtools: quit");
                    return Ok(());
                }
                Some(AppEvent::Visible(v)) => {
                    dprintln!("devtools: visible {}", v);
                    visible = v;
                    if v {
                        (stats_at, devices_at) = (0, 0); // refresh at once
                    }
                }
                Some(AppEvent::Settings { .. }) => {
                    if ctx.model.borrow().view == View::Settings {
                        ctx.show();
                        ctx.app.draw()?;
                    }
                }
                Some(AppEvent::Back) => {
                    // claimed only while a sub-view is open; a test returns to its device
                    let back_to = match ctx.model.borrow().view {
                        View::TouchTest | View::DisplayTest => View::Device,
                        _ => View::None,
                    };
                    ctx.open(back_to);
                    ctx.app.draw()?;
                }
                Some(AppEvent::Input(InputEvent::Key { code, down })) => {
                    ctx.model.borrow_mut().last_key = Some((code, down));
                    dprintln!(
                        "devtools: key {:#x} {}",
                        code,
                        if down { "down" } else { "up" }
                    );
                    ctx.app.dispatch_key(code);
                    ctx.show();
                    ctx.app.draw()?;
                }
                Some(AppEvent::Input(input)) => {
                    ctx.app.dispatch_touch(input);
                    ctx.app.draw()?;
                }
                _ => {}
            }
        }
    })
}

/// As time passes: follow the log; while visible, refresh what the screen shows. True when something to draw changed.
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
    let mut changed = grew && matches!((tab, view), (Tab::Console, View::None) | (_, View::Device));
    // Home shows everything; the other pages what they list
    let wants_stats =
        view == View::None && matches!(tab, Tab::Home | Tab::Procs) || view == View::Proc;
    let wants_devices =
        view == View::None && matches!(tab, Tab::Home | Tab::Devices) || view == View::Device;
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

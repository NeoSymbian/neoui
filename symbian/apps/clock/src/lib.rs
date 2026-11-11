// SPDX-License-Identifier: GPL-3.0-only
//! The Clock app (`NEOCLOCK.EXE`, NeoSymbian R6): a separate Symbian process the UI server starts. Five pages
//! (Clock, Alarms, World, Timer, Stopwatch) and their sub-views, drawn with Slint into the surface `NeoUi` hands
//! over ([`neoapp`]). The state of every page is kept in the settings store ([`state`]), so it survives the app
//! closing and the phone restarting; alarms ring from the UI server even while the app is closed.
//!
//! - [`state`]: the model and its persistence
//! - [`pages`]: one module per page (callbacks in, properties out)
//! - this file: connecting, and the event loop (input, Back, settings changes, alarms, ticking)
#![no_std]

extern crate alloc;

mod pages;
mod state;

use alloc::rc::Rc;
use core::cell::RefCell;
use core::pin::pin;
use e32::ipc::TIpcArgs;
use e32::rt::{self, Either};
use e32::sys::TPtr8;
use e32::{dprintln, time};
use neoapp::App;
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::{InputEvent, TouchPhase};
use pages::{Ctx, Now};
use slint::ComponentHandle;
use state::{Model, View};

slint::include_modules!();

e32::main!(main);

/// How often the face is redrawn: on each new second.
const TICK_US: i32 = 250_000;
/// While the stopwatch runs on screen: hundredths move.
const FAST_TICK_US: i32 = 50_000;

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("clock: failed {}", e);
            e
        }
    }
}

fn run() -> Result<(), i32> {
    let app = App::connect()?;
    dprintln!("clock: surface {}x{} mapped", app.width, app.height);
    let ui = ClockApp::new().map_err(|_| -2)?;
    ui.global::<Theme>().set_dp(app.dp);
    ui.show().map_err(|_| -2)?;
    // the 24-hour setting and the alarms can change outside the app (Settings, the UI server after an alarm)
    if let Err(e) = app.watch_settings("clock.") {
        dprintln!("clock: cannot watch settings ({})", e);
    }
    let model = Model::load(&app);
    let ctx = Rc::new(Ctx {
        app,
        ui: ui.as_weak(),
        model: RefCell::new(model),
    });
    pages::bind(&ctx, &ui);
    ctx.show(&Now::read());
    ctx.draw()?;

    // In the background (R10) the clock neither ticks nor draws.
    let mut visible = true;
    let mut fast = false;
    let mut shown_second = u32::MAX;
    rt::block_on(async {
        loop {
            let mut ev = [0u8; proto::EVENT_LEN];
            let got = {
                let mut des = TPtr8::new(&mut ev);
                let mut args = TIpcArgs::new();
                args.des8(0, &mut des);
                // One NEXT_EVENT stays outstanding while the clock ticks.
                let mut next = pin!(ctx.app.server.send(proto::NEXT_EVENT, &args));
                loop {
                    if visible {
                        let now = Now::read();
                        if fast || now.second != shown_second {
                            if now.second != shown_second {
                                dprintln!(
                                    "clock: tick {:02}:{:02}:{:02}",
                                    now.local.hour,
                                    now.local.minute,
                                    now.second
                                );
                            }
                            shown_second = now.second;
                            fast = ctx.tick(&now);
                            ctx.draw()?;
                        }
                    }
                    let wait = if fast { FAST_TICK_US } else { TICK_US };
                    match rt::select(next.as_mut(), time::delay(wait)).await {
                        Either::Left(r) => break r,
                        Either::Right(()) => {}
                    }
                }
            };
            got?;
            match AppEvent::decode(&ev) {
                Some(AppEvent::Quit) => {
                    dprintln!("clock: quit");
                    return Ok(());
                }
                Some(AppEvent::Visible(v)) => {
                    dprintln!("clock: visible {}", v);
                    visible = v;
                    shown_second = u32::MAX; // redraw at once when shown again
                }
                Some(AppEvent::Settings { seq }) => {
                    ctx.model.borrow_mut().reload_shared(&ctx.app);
                    dprintln!(
                        "clock: settings changed ({}): 24-hour {}",
                        seq,
                        ctx.model.borrow().h24
                    );
                    ctx.show(&Now::read());
                    ctx.draw()?;
                }
                Some(AppEvent::Alarm { id }) => {
                    pages::ring(&ctx, id);
                    ctx.draw()?;
                }
                Some(AppEvent::Back) => {
                    // claimed only while a sub-view is open: Back closes it, as Cancel does (Dismiss when ringing)
                    if ctx.model.borrow().view == View::Ringing {
                        ui.global::<ClockState>().invoke_dismiss();
                    } else {
                        ctx.open(View::None);
                    }
                    ctx.draw()?;
                }
                Some(AppEvent::Input(input)) => {
                    if let InputEvent::Touch {
                        x,
                        y,
                        phase: TouchPhase::Up,
                        ..
                    } = input
                    {
                        dprintln!("clock: touch {},{}", x, y);
                    }
                    ctx.app.dispatch_touch(input);
                    ctx.draw()?;
                }
                _ => {}
            }
        }
    })
}

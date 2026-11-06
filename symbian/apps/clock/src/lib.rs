// SPDX-License-Identifier: GPL-3.0-only
//! The Clock app (`NEOCLOCK.EXE`, NeoSymbian R6): a separate Symbian process started by the UI server. It connects
//! to `NeoUi` ([`neoapp`]), renders `ui/clock.slint` into its surface, ticks once a second from Symbian's clock, and
//! exits on `Quit`.
#![no_std]

extern crate alloc;

use alloc::format;
use core::pin::pin;
use e32::ipc::TIpcArgs;
use e32::rt::{self, Either};
use e32::sys::TPtr8;
use e32::{dprintln, time};
use neoapp::App;
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::{InputEvent, TouchPhase};
use slint::ComponentHandle;

slint::include_modules!();

e32::main!(main);

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

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

    let mut shown_second = u32::MAX;
    rt::block_on(async {
        loop {
            let mut ev = [0u8; proto::EVENT_LEN];
            let got = {
                let mut des = TPtr8::new(&mut ev);
                let mut args = TIpcArgs::new();
                args.des8(0, &mut des);
                // One NEXT_EVENT stays outstanding while the clock ticks.
                let mut next = pin!(app.server.send(proto::NEXT_EVENT, &args));
                loop {
                    let now = time::now();
                    if now.second != shown_second {
                        shown_second = now.second;
                        set_time(&ui, &now);
                        dprintln!(
                            "clock: tick {:02}:{:02}:{:02}",
                            now.hour,
                            now.minute,
                            now.second
                        );
                        app.draw()?;
                    }
                    match rt::select(next.as_mut(), time::delay(250_000)).await {
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
                    app.dispatch_touch(input);
                    app.draw()?;
                }
                _ => {}
            }
        }
    })
}

fn set_time(ui: &ClockApp, t: &time::DateTime) {
    ui.set_hours(t.hour as i32);
    ui.set_minutes(t.minute as i32);
    ui.set_seconds(t.second as i32);
    ui.set_date(
        format!(
            "{}, {} {} {}",
            DAYS[t.weekday as usize % 7],
            t.day,
            MONTHS[(t.month as usize + 11) % 12],
            t.year
        )
        .into(),
    );
}

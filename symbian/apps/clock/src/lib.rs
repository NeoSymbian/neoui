// SPDX-License-Identifier: GPL-3.0-only
//! The Clock app (`NEOCLOCK.EXE`, NeoSymbian R6): a separate Symbian process started by the UI server. It connects
//! to `NeoUi` ([`neoapp`]), renders `ui/clock.slint` into its surface, ticks once a second from Symbian's clock, and
//! exits on `Quit`. "Set time" sets Symbian's clock and time zone by hand; the UI server keeps them across restarts.
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::pin::pin;
use e32::ipc::TIpcArgs;
use e32::rt::{self, Either};
use e32::sys::TPtr8;
use e32::{dprintln, time};
use neoapp::App;
use neoui_proto::clock::{self, Local};
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::settings::keys;
use neoui_proto::{InputEvent, TouchPhase};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

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
    set_up_editing(&ui);
    // the 24-hour setting (docs/neoui-shell.md, Part 1): read now and again whenever it changes
    if let Err(e) = app.watch_settings("clock.") {
        dprintln!("clock: cannot watch settings ({})", e);
    }
    let mut h24 = clock_24h(&app);

    let mut shown_second = u32::MAX;
    // In the background (R10) the clock neither ticks nor draws.
    let mut visible = true;
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
                    if visible && now.second != shown_second {
                        shown_second = now.second;
                        set_time(&ui, &now, h24);
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
                Some(AppEvent::Settings { seq }) => {
                    h24 = clock_24h(&app);
                    dprintln!("clock: settings changed ({}): 24-hour {}", seq, h24);
                    shown_second = u32::MAX; // redraw with the new format
                }
                Some(AppEvent::Visible(v)) => {
                    dprintln!("clock: visible {}", v);
                    visible = v;
                    shown_second = u32::MAX; // redraw at once when shown again
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

/// Whether the clock shows 24-hour time; the default when the setting can't be read (an older UI server).
fn clock_24h(app: &App) -> bool {
    app.setting(keys::CLOCK_24H).unwrap_or_else(|e| {
        dprintln!("clock: cannot read clock.24h ({})", e);
        true
    })
}

/// Shows `t`; on a 12-hour clock the hours run 12, 1, ..., 11.
fn set_time(ui: &ClockApp, t: &time::DateTime, h24: bool) {
    let hours = if h24 { t.hour } else { (t.hour + 11) % 12 + 1 };
    ui.set_hours(hours as i32);
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

fn strings(items: impl Iterator<Item = SharedString>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.collect::<Vec<_>>()))
}

fn show_fields(ui: &ClockApp, l: &Local) {
    ui.set_values(strings((0..clock::FIELDS.len()).map(|i| l.show(i).into())));
}

/// "Set time": steppers start at the clock's local time and zone; Save sets both (needs WriteDeviceData, which ROM
/// EXEs have).
fn set_up_editing(ui: &ClockApp) {
    ui.set_labels(strings(
        clock::FIELDS.iter().map(|f| SharedString::from(*f)),
    ));
    let edit = Rc::new(RefCell::new(Local {
        year: 2026,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        tz: 0,
    }));
    let (weak, e) = (ui.as_weak(), edit.clone());
    ui.on_edit(move || {
        let now = time::now();
        let (_, tz) = time::utc_now();
        let l = Local {
            year: now.year,
            month: now.month,
            day: now.day,
            hour: now.hour,
            minute: now.minute,
            tz,
        };
        *e.borrow_mut() = l;
        if let Some(ui) = weak.upgrade() {
            show_fields(&ui, &l);
            ui.set_editing(true);
        }
    });
    let (weak, e) = (ui.as_weak(), edit.clone());
    ui.on_adjust(move |field, delta| {
        e.borrow_mut().adjust(field as usize, delta);
        if let Some(ui) = weak.upgrade() {
            show_fields(&ui, &e.borrow());
        }
    });
    let (weak, e) = (ui.as_weak(), edit);
    ui.on_save(move || {
        let l = *e.borrow();
        match time::set_utc_and_offset(l.to_unix(), l.tz) {
            Ok(()) => dprintln!("clock: set {} UTC, offset {} s", l.to_unix(), l.tz),
            Err(err) => dprintln!("clock: set failed ({})", err),
        }
        if let Some(ui) = weak.upgrade() {
            ui.set_editing(false);
        }
    });
    let weak = ui.as_weak();
    ui.on_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_editing(false);
        }
    });
}

// SPDX-License-Identifier: GPL-3.0-only
//! The Clock app (`NEOCLOCK.EXE`, NeoSymbian R6): a separate Symbian process started by the UI server. It connects
//! to `NeoUi`, maps the surface chunk the server gives it, renders `ui/clock.slint` into it with Slint's software
//! renderer, reports the damaged rectangles, ticks once a second from Symbian's clock, and exits on `Quit`.
#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use core::pin::pin;
use core::time::Duration;
use e32::chunk::Chunk;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::rt::{self, Either};
use e32::sys::TPtr8;
use e32::{dprintln, time};
use neoui_proto::ipc::{self as proto, AppEvent};
use neoui_proto::{InputEvent, TouchPhase};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, PlatformError};

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

/// XRGB8888 as the frame buffer wants it (same as neoui-render's pixel type).
#[derive(Clone, Copy)]
#[repr(transparent)]
struct Xrgb8888(u32);

impl TargetPixel for Xrgb8888 {
    fn blend(&mut self, c: PremultipliedRgbaColor) {
        let inv = 255 - c.alpha as u32;
        let ch = |shift: u32, src: u8| ((self.0 >> shift & 0xFF) * inv / 255 + src as u32).min(255);
        self.0 = 0xFF00_0000 | ch(16, c.red) << 16 | ch(8, c.green) << 8 | ch(0, c.blue);
    }
    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32)
    }
}

struct AppPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for AppPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        Duration::from_millis(time::millis())
    }
}

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
    let (major, minor) = proto::VERSION;
    let server = Session::connect(
        proto::SERVER,
        TVersion {
            major,
            minor,
            build: 0,
        },
    )?;

    let mut geo = [0u8; proto::GEOMETRY_LEN];
    {
        let mut des = TPtr8::new(&mut geo);
        let mut args = TIpcArgs::new();
        args.des8(0, &mut des);
        server.send_receive(proto::GEOMETRY, &args)?;
    }
    let word =
        |i: usize| u32::from_le_bytes([geo[i * 4], geo[i * 4 + 1], geo[i * 4 + 2], geo[i * 4 + 3]]);
    let (w, h, dp) = (word(0) as usize, word(1) as usize, word(2) as f32 / 256.0);

    let surface = Chunk::from_handle(server.send_receive(proto::SURFACE, &TIpcArgs::new())?);
    // SAFETY: the server made the chunk `w * h * 4` bytes, committed, and only reads it; it outlives `px`.
    let px = unsafe { core::slice::from_raw_parts_mut(surface.base().cast::<Xrgb8888>(), w * h) };
    dprintln!("clock: surface {}x{} mapped", w, h);

    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(AppPlatform(window.clone()))).map_err(|_| -2)?;
    let ui = ClockApp::new().map_err(|_| -2)?;
    ui.global::<Theme>().set_dp(dp);
    window.set_size(PhysicalSize::new(w as u32, h as u32));
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
                let mut next = pin!(server.send(proto::NEXT_EVENT, &args));
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
                        draw(&window, px, w, &server)?;
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
                    dispatch(&window, input);
                    draw(&window, px, w, &server)?;
                }
                None => {}
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

/// Render what changed into the surface and tell the server which rectangles to show.
fn draw(
    window: &MinimalSoftwareWindow,
    px: &mut [Xrgb8888],
    stride: usize,
    server: &Session,
) -> Result<(), i32> {
    slint::platform::update_timers_and_animations();
    let mut result = Ok(());
    window.draw_if_needed(|r| {
        for (o, s) in r.render(px, stride).iter() {
            let mut args = TIpcArgs::new();
            args.int(0, o.x)
                .int(1, o.y)
                .int(2, s.width as i32)
                .int(3, s.height as i32);
            if let Err(e) = server.send_receive(proto::DAMAGE, &args) {
                result = Err(e);
            }
        }
    });
    result
}

/// Input from the server (surface coordinates) to Slint, as neoui-render does for the shell.
fn dispatch(window: &MinimalSoftwareWindow, input: InputEvent) {
    if let InputEvent::Touch { x, y, phase, .. } = input {
        let position = LogicalPosition::new(x as f32, y as f32);
        let button = PointerEventButton::Left;
        let event = match phase {
            TouchPhase::Down => WindowEvent::PointerPressed { position, button },
            TouchPhase::Move => WindowEvent::PointerMoved { position },
            TouchPhase::Up => WindowEvent::PointerReleased { position, button },
        };
        window.dispatch_event(event);
        if phase == TouchPhase::Up {
            window.dispatch_event(WindowEvent::PointerExited);
        }
    }
}

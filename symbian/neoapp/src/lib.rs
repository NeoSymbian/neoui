// SPDX-License-Identifier: GPL-3.0-only
//! The app side of neoui's app model (NeoSymbian R6): connect to the `NeoUi` server, take the geometry and the
//! surface chunk it hands over, render Slint into the surface and report damage, and turn input from the server into
//! Slint events. Each app keeps one `NEXT_EVENT` outstanding in its own loop (see the Clock and Terminal apps).
#![no_std]

extern crate alloc;

pub mod build;

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::time::Duration;
use e32::chunk::Chunk;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sys::TPtr8;
use e32::time;
use neoui_proto::ipc as proto;
use neoui_proto::{InputEvent, Rect, ScrollPlan, Scrolling, TouchPhase};
use slint::platform::software_renderer::{
    LineBufferProvider, MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
    TargetPixel,
};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize, PlatformError, SharedString};

/// XRGB8888 as the frame buffer wants it (same as neoui-render's pixel type).
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Xrgb8888(pub u32);

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

/// Slint's lines for a scrolled list's frame (smooth-ui S2d, as neoui-render's): the rows of `list` outside `draw`
/// were moved into place already, so Slint skips them.
struct ScrollLines<'a> {
    px: &'a mut [Xrgb8888],
    stride: usize,
    list: Rect,
    draw: core::ops::Range<i32>,
}

impl LineBufferProvider for ScrollLines<'_> {
    type TargetPixel = Xrgb8888;

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Xrgb8888]),
    ) {
        let (l, y) = (&self.list, line as i32);
        let moved = y >= l.y
            && y < l.y + l.h as i32
            && !self.draw.contains(&y)
            && range.start >= l.x as usize
            && range.end <= l.x as usize + l.w as usize;
        if !moved {
            render_fn(&mut self.px[line * self.stride..][range]);
        }
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

/// A connected app: its session with `NeoUi`, its surface and the Slint window that draws into it.
pub struct App {
    pub server: Session,
    pub width: usize,
    pub height: usize,
    /// The shell's design unit in pixels (set it on the app's `Theme.dp`).
    pub dp: f32,
    pub window: Rc<MinimalSoftwareWindow>,
    surface: Chunk,
    scrolling: RefCell<Scrolling>,
}

impl App {
    /// Connect, ask for the geometry and the surface, and install the Slint platform (create the UI after this).
    pub fn connect() -> Result<Self, i32> {
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
        let word = |i: usize| {
            u32::from_le_bytes([geo[i * 4], geo[i * 4 + 1], geo[i * 4 + 2], geo[i * 4 + 3]])
        };
        let (width, height, dp) = (word(0) as usize, word(1) as usize, word(2) as f32 / 256.0);
        let surface = Chunk::from_handle(server.send_receive(proto::SURFACE, &TIpcArgs::new())?);
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(AppPlatform(window.clone()))).map_err(|_| -2)?;
        window.set_size(PhysicalSize::new(width as u32, height as u32));
        Ok(Self {
            server,
            width,
            height,
            dp,
            window,
            surface,
            scrolling: RefCell::default(),
        })
    }

    /// Render what changed into the surface and tell the server which rectangles to show.
    pub fn draw(&self) -> Result<(), i32> {
        self.draw_scrolled(|| ((0, 0), Rect::default()))
    }

    /// [`Self::draw`] for an app with `NeoList`s (`ui/components/scroll.slint`, smooth-ui S2d): `scrolled` reads and
    /// clears its `Scroll` global (px moved, lists moved, the list's place) once the change handlers have run.
    pub fn draw_scrolled(&self, scrolled: impl FnOnce() -> ((i32, i32), Rect)) -> Result<(), i32> {
        slint::platform::update_timers_and_animations();
        let (moved, list) = scrolled();
        // SAFETY: the server made the chunk `width * height * 4` bytes, committed; only this thread writes it and the
        // server only reads it; the slice lives for this call.
        let px = unsafe {
            core::slice::from_raw_parts_mut(
                self.surface.base().cast::<Xrgb8888>(),
                self.width * self.height,
            )
        };
        let screen = Rect::new(0, 0, self.width as u32, self.height as u32);
        let plan = self
            .scrolling
            .borrow_mut()
            .plan(px, self.width, screen, moved, list);
        if plan == ScrollPlan::Full {
            self.window.request_redraw();
        }
        let mut result = Ok(());
        self.window.draw_if_needed(|r| {
            let region = match plan {
                ScrollPlan::Shift { list, draw } => r.render_by_line(ScrollLines {
                    px,
                    stride: self.width,
                    list,
                    draw,
                }),
                ScrollPlan::Full => {
                    // a change of buffer type drops Slint's cache, so this render draws the whole surface
                    r.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
                    r.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
                    r.render(px, self.width)
                }
                ScrollPlan::Normal => r.render(px, self.width),
            };
            for (o, s) in region.iter() {
                let mut args = TIpcArgs::new();
                args.int(0, o.x)
                    .int(1, o.y)
                    .int(2, s.width as i32)
                    .int(3, s.height as i32);
                if let Err(e) = self.server.send_receive(proto::DAMAGE, &args) {
                    result = Err(e);
                }
            }
        });
        result
    }

    /// Show or hide the system keyboard; returns how many pixels of the surface's bottom it now covers.
    pub fn keyboard(&self, show: bool) -> Result<u32, i32> {
        let mut args = TIpcArgs::new();
        args.int(0, show as i32);
        self.server
            .send_receive(proto::KEYBOARD, &args)
            .map(|h| h as u32)
    }

    /// Take over the toolbar's Back and/or Options: the server then sends `AppEvent::Back` / `AppEvent::Options`.
    pub fn claim_nav(&self, back: bool, options: bool) -> Result<(), i32> {
        let mut args = TIpcArgs::new();
        let claims =
            if back { proto::NAV_BACK } else { 0 } | if options { proto::NAV_OPTIONS } else { 0 };
        args.int(0, claims);
        self.server.send_receive(proto::NAV, &args).map(drop)
    }

    /// A key from the server (a hardware key or the system keyboard: Unicode, Enter 13, Backspace 8, Escape 27) as a
    /// Slint key press and release, for text fields.
    pub fn dispatch_key(&self, code: u32) {
        let text: SharedString = match code {
            13 | 10 => Key::Return.into(),
            8 => Key::Backspace.into(),
            27 => Key::Escape.into(),
            c => match char::from_u32(c) {
                Some(ch) => ch.into(),
                None => return,
            },
        };
        self.window
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.window
            .dispatch_event(WindowEvent::KeyReleased { text });
    }

    /// A touch from the server (surface coordinates) as Slint pointer events; keys are the app's business.
    pub fn dispatch_touch(&self, input: InputEvent) {
        if let InputEvent::Touch { x, y, phase, .. } = input {
            let position = LogicalPosition::new(x as f32, y as f32);
            let button = PointerEventButton::Left;
            let event = match phase {
                TouchPhase::Down => WindowEvent::PointerPressed { position, button },
                TouchPhase::Move => WindowEvent::PointerMoved { position },
                TouchPhase::Up => WindowEvent::PointerReleased { position, button },
            };
            self.window.dispatch_event(event);
            if phase == TouchPhase::Up {
                self.window.dispatch_event(WindowEvent::PointerExited);
            }
        }
    }
}

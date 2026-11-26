// SPDX-License-Identifier: GPL-3.0-only
//! The neoui app API (protocol 1.8; docs `neoui-shell/kit.md`): everything an app process does with the shell.
//!
//! - [`App::connect`]: the `NeoUi` session, the geometry, the surface chunk, and Slint's platform drawing into it.
//! - [`App::next_event`]: the event loop's one outstanding `NEXT_EVENT`. While it waits it runs Slint's timers and
//!   animations (the kit's long press, spinners, sheets) and an optional [`Ticker`]; when an event comes it dispatches
//!   input to Slint, resizes the window for [`AppEvent::Area`], and draws what changed, then returns the event.
//!   [`App::run`] is the whole loop for an app with nothing else to wait for.
//! - The page ([`App::page`]: its name, Back, full screen), dialogs ([`App::query`]), toasts ([`App::note`]), the
//!   shade ([`App::notify`]), launcher entries and badges ([`App::entry`], [`App::badge`]), the keyboard
//!   ([`App::keyboard`]), settings ([`App::setting`]) and the clipboard (Slint's text fields use it).
//!
//! The app draws its header, footer and content with the kit (`@neoui/kit.slint`, built with the `neoui-kit` crate);
//! [`kit_back!`] runs Back through the kit's layers (menus, sheets, search, selection) first. A new app starts as a
//! copy of `symbian/apps/kit` (NEOKIT.EXE).
#![no_std]

extern crate alloc;

pub mod build;
pub mod hal;
pub mod settings_info;
pub mod sys;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use core::cell::{Cell, RefCell};
use core::time::Duration;
use e32::chunk::Chunk;
use e32::fs::{File, SharedFile};
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::rt::{self, Either};
use e32::sys::{TPtr8, TPtrC8};
use e32::time;
use neoui_proto::ipc as proto;
pub use neoui_proto::ipc::AppEvent;
/// Which apps open which files, and the pages of Files other apps open (protocol 1.12).
pub use neoui_proto::open;
pub use neoui_proto::page::{Note, Notification, Page, Query};

/// What a [`Notification`] card carries since 1.18 (top-panel TP2a): flags, styles, action kinds, and why the shade
/// removed one ([`AppEvent::NotifyGone`]).
pub mod card {
    pub use neoui_proto::ipc::{GONE_CLEARED, GONE_OFF, GONE_SWIPED};
    pub use neoui_proto::page::{
        NOTIFY_ONGOING, NOTIFY_PRIVATE, NOTIFY_SILENT_UPDATE, action, style,
    };
}
use neoui_proto::settings::value::ENCODED_MAX;
use neoui_proto::settings::{Key as SettingKey, Setting, Value, kerr};
use neoui_proto::{InputEvent, Rect, ScrollPlan, Scrolling, TouchPhase};
use slint::platform::software_renderer::{
    LineBufferProvider, MinimalSoftwareWindow, PremultipliedRgbaColor, RenderingRotation,
    RepaintBufferType, TargetPixel,
};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize, PlatformError, SharedString};

/// Days from 0000-01-01 (Symbian's `TTime` zero) to 1970-01-01.
const TTIME_UNIX_DAYS: i64 = 719_540;

/// Wall-clock UTC now, in Unix milliseconds (for timers that must keep counting while an app is closed or across a
/// reboot; `e32::time::millis` restarts at every boot).
pub fn wall_ms() -> i64 {
    let (us, _) = e32::sys::time_now();
    us.div_euclid(1000) - TTIME_UNIX_DAYS * 86_400_000
}

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

struct AppPlatform {
    window: Rc<MinimalSoftwareWindow>,
    server: Rc<Session>,
}

impl Platform for AppPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        Duration::from_millis(time::millis())
    }
    // the clipboard is the server's, shared by every app (CLIP_SET, CLIP_GET)
    fn set_clipboard_text(&self, text: &str, clipboard: slint::platform::Clipboard) {
        if clipboard == slint::platform::Clipboard::DefaultClipboard {
            clip_set(&self.server, text);
        }
    }
    fn clipboard_text(
        &self,
        clipboard: slint::platform::Clipboard,
    ) -> Option<alloc::string::String> {
        if clipboard != slint::platform::Clipboard::DefaultClipboard {
            return None;
        }
        clip_get(&self.server)
    }
}

/// The server's clipboard text (`proto::CLIP_GET`); `None` when empty.
fn clip_get(server: &Session) -> Option<alloc::string::String> {
    let mut buf = vec![0u8; proto::CLIP_MAX];
    let len = {
        let mut out = TPtr8::new(&mut buf);
        let mut args = TIpcArgs::new();
        args.des8(0, &mut out);
        server.send_receive(proto::CLIP_GET, &args).ok()?;
        out.len()
    };
    buf.truncate(len);
    alloc::string::String::from_utf8(buf)
        .ok()
        .filter(|t| !t.is_empty())
}

/// Put `text` on the server's clipboard (`proto::CLIP_SET`); longer than `proto::CLIP_MAX` is refused.
fn clip_set(server: &Session, text: &str) -> bool {
    if text.len() > proto::CLIP_MAX {
        return false;
    }
    let t = TPtrC8::new(text.as_bytes());
    let mut args = TIpcArgs::new();
    args.desc8(0, &t);
    server.send_receive(proto::CLIP_SET, &args).is_ok()
}

/// Something an app does on its own clock while it waits for events (the Clock's seconds): `f` runs every
/// `every_us` microseconds, between events, and may draw.
pub struct Ticker<'a> {
    pub every_us: u32,
    pub f: &'a mut dyn FnMut() -> Result<(), i32>,
}

/// The read side of an app's `Scroll` global for [`App::draw_scrolled`]: px moved and the list's place.
/// A rectangle of the app's surface that any thread of the app may write ([`App::raw_target`]): the surface chunk is
/// mapped for the whole process.
pub struct RawTarget {
    px: *mut u32,
    len: usize,
    /// The row pitch, in pixels.
    pub stride: usize,
    /// The rectangle, clipped to the surface: what [`App::damage`] shows.
    pub rect: Rect,
}

// SAFETY: the pointer is into a process-wide chunk that outlives the session; one owner writes it at a time (`&mut`).
unsafe impl Send for RawTarget {}

impl RawTarget {
    /// The surface from the rectangle's top-left corner: `rect.h` rows of `rect.w` pixels, `stride` apart (XRGB8888).
    pub fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: `px..px + len` lies inside the surface chunk (App::raw_target); `&mut self` keeps it unaliased.
        unsafe { core::slice::from_raw_parts_mut(self.px, self.len) }
    }
}

type Scrolled = dyn Fn() -> ((i32, i32), Rect);

/// A connected app: its session with `NeoUi`, its surface and the Slint window that draws into it.
pub struct App {
    pub server: Rc<Session>,
    pub width: usize,
    /// The surface's rows in use: the body, or the whole screen for a full-screen page ([`App::page`]).
    height: Cell<usize>,
    /// The shell's design unit in pixels (set it on the app's `Theme.dp`).
    pub dp: f32,
    pub window: Rc<MinimalSoftwareWindow>,
    /// Asked for on the first frame drawn while the app is on screen: a resident app started hidden (Phone,
    /// Messages) costs no surface (~7 MB on the phone) until it is first shown.
    surface: RefCell<Option<Chunk>>,
    /// The server's last `Visible`.
    visible: Cell<bool>,
    scrolling: RefCell<Scrolling>,
    /// How [`App::next_event`] reads the app's `Scroll` global when it draws ([`App::set_scrolled`]).
    scrolled: RefCell<Option<Box<Scrolled>>>,
    /// The keyboard's edit bar sent Control: the next key is a chord.
    ctrl: Cell<bool>,
    /// Landscape (FA12's video): Slint draws the window turned a quarter clockwise into the surface.
    rotation: Cell<RenderingRotation>,
    /// The next frame draws the whole surface (the rotation changed).
    full: Cell<bool>,
}

impl App {
    /// Connect, ask for the geometry and the surface, and install the Slint platform (create the UI after this).
    pub fn connect() -> Result<Self, i32> {
        let (major, minor) = proto::VERSION;
        let server = Rc::new(Session::connect(
            proto::SERVER,
            TVersion {
                major,
                minor,
                build: 0,
            },
        )?);
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
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(AppPlatform {
            window: window.clone(),
            server: server.clone(),
        }))
        .map_err(|_| -2)?;
        window.set_size(PhysicalSize::new(width as u32, height as u32));
        Ok(Self {
            server,
            width,
            height: Cell::new(height),
            dp,
            window,
            surface: RefCell::new(None),
            visible: Cell::new(false),
            scrolling: RefCell::default(),
            scrolled: RefCell::default(),
            ctrl: Cell::new(false),
            rotation: Cell::new(RenderingRotation::NoRotation),
            full: Cell::new(false),
        })
    }

    /// Tell [`App::next_event`] how to read the app's `Scroll` global (its kit lists), so the frames it draws move
    /// scrolled pixels instead of redrawing them: `|| scrolled(&ui)` as Files does.
    pub fn set_scrolled(&self, f: impl Fn() -> ((i32, i32), Rect) + 'static) {
        *self.scrolled.borrow_mut() = Some(Box::new(f));
    }

    /// Draw what changed, through the reader given to [`App::set_scrolled`] if any.
    pub fn redraw(&self) -> Result<(), i32> {
        match self.scrolled.borrow().as_ref() {
            Some(f) => self.draw_scrolled(f),
            None => self.draw(),
        }
    }

    /// Wait for the next event, keeping one `NEXT_EVENT` outstanding. Meanwhile Slint's timers and animations run and
    /// their frames are drawn, and `tick` runs on its period. The event is handled as far as the shell's part goes
    /// ([`App::handle`]) and what changed is drawn before it is returned; after acting on it, call
    /// [`App::redraw`].
    pub async fn next_event(&self, mut tick: Option<Ticker<'_>>) -> Result<AppEvent, i32> {
        let mut ev = [0u8; proto::EVENT_LEN];
        let got = {
            let mut des = TPtr8::new(&mut ev);
            let mut args = TIpcArgs::new();
            args.des8(0, &mut des);
            let mut next = core::pin::pin!(self.server.send(proto::NEXT_EVENT, &args));
            let mut tick_at = time::millis();
            loop {
                let now = time::millis();
                if let Some(t) = tick.as_mut()
                    && now >= tick_at
                {
                    (t.f)()?;
                    tick_at = now + u64::from(t.every_us / 1000).max(1);
                }
                let animating = self.window.has_active_animations();
                // the soonest of: a frame (16 ms) while animating, a Slint timer, the ticker
                let mut wait_us: u64 = if animating { 16_000 } else { 1_000_000 };
                if let Some(d) = slint::platform::duration_until_next_timer_update() {
                    wait_us = wait_us.min(d.as_micros() as u64);
                }
                if tick.is_some() {
                    wait_us = wait_us.min(tick_at.saturating_sub(now) * 1000);
                }
                let idle = !animating
                    && tick.is_none()
                    && slint::platform::duration_until_next_timer_update().is_none();
                let r = if idle {
                    next.as_mut().await
                } else {
                    match rt::select(next.as_mut(), time::delay(wait_us.max(1000) as i32)).await {
                        Either::Left(r) => r,
                        Either::Right(()) => {
                            self.redraw()?;
                            continue;
                        }
                    }
                };
                break r;
            }
        };
        got?;
        let e = AppEvent::decode(&ev).ok_or(kerr::ARGUMENT)?;
        self.handle(&e);
        self.redraw()?;
        Ok(e)
    }

    /// How long an app's own loop may sleep before Slint needs it: the next Slint timer (a long-press, a toast's
    /// Undo), or a frame (16 ms) while an animation runs (a switch flipping); `None` when Slint waits for nothing. A
    /// loop that waits on more than `NEXT_EVENT` (MODEMD's status) takes the shorter of this and its own wake, then
    /// calls [`App::redraw`], as [`App::next_event`] does.
    pub fn wake_us(&self) -> Option<u64> {
        let timer =
            slint::platform::duration_until_next_timer_update().map(|d| d.as_micros() as u64);
        if self.window.has_active_animations() {
            return Some(timer.map_or(16_000, |t| t.min(16_000)));
        }
        timer
    }

    /// The shell's part of an event: input goes to Slint, a new area height resizes the window. The app handles the
    /// rest (Back, Answer, Quit...).
    pub fn handle(&self, e: &AppEvent) {
        match *e {
            AppEvent::Input(InputEvent::Key { code, down: true }) => self.dispatch_key(code),
            AppEvent::Input(t @ InputEvent::Touch { .. }) => self.dispatch_touch(t),
            AppEvent::Area { h } if h > 0 => self.resize(h as usize),
            AppEvent::Visible(v) => self.visible.set(v),
            _ => {}
        }
    }

    /// The whole event loop: `on_event` gets every event (after [`App::handle`]) and returns false to leave; Quit
    /// leaves too. The app draws after each one.
    pub fn run(&self, mut on_event: impl FnMut(AppEvent) -> Result<bool, i32>) -> Result<(), i32> {
        rt::block_on(async {
            loop {
                let e = self.next_event(None).await?;
                if e == AppEvent::Quit || !on_event(e)? {
                    return Ok(());
                }
                self.redraw()?;
            }
        })
    }

    // the server sized the surface for the whole screen, so any height it gives fits
    fn resize(&self, h: usize) {
        if h != self.height.get() {
            self.height.set(h);
            self.size_window();
        }
    }

    /// The window's size: the surface's, or its turn while landscape.
    fn size_window(&self) {
        let (w, h) = (self.width as u32, self.height.get() as u32);
        let size = if self.landscape() {
            PhysicalSize::new(h, w)
        } else {
            PhysicalSize::new(w, h)
        };
        self.window.set_size(size);
        self.window.request_redraw();
    }

    fn landscape(&self) -> bool {
        self.rotation.get() != RenderingRotation::NoRotation
    }

    /// Landscape on or off (FA12's video): the whole UI is drawn turned a quarter clockwise, its window as wide as
    /// the surface is high; touches are turned back. Use with a full-screen [`Page`] so the shell's bars are gone.
    pub fn set_landscape(&self, on: bool) {
        let r = if on {
            RenderingRotation::Rotate90
        } else {
            RenderingRotation::NoRotation
        };
        if r != self.rotation.get() {
            self.rotation.set(r);
            self.full.set(true);
            self.size_window();
        }
    }

    /// A rectangle of the (possibly turned) window on the surface.
    pub fn to_surface(&self, r: Rect) -> Rect {
        if !self.landscape() {
            return r;
        }
        // Rotate90: window (x, y) is surface (H - 1 - y, x), H the window's height (the surface's width)
        let h = self.width as i32;
        Rect::new(h - r.y - r.h as i32, r.x, r.h, r.w)
    }

    /// Pixels the app writes itself into the surface (a video frame), shown with one DAMAGE: `draw` gets the
    /// surface from `r`'s top-left corner and the row pitch, and writes `r.w` x `r.h` pixels (XRGB8888, `0x00RRGGBB`).
    /// `r` is in surface pixels ([`App::to_surface`] when landscape). Slint must draw nothing there meanwhile, or the
    /// two overwrite each other: keep its items over that rectangle unchanged (or hidden) while frames come.
    /// Like [`App::draw`], it returns once the server has shown the frame.
    pub fn raw(&self, r: Rect, draw: impl FnOnce(&mut [u32], usize)) -> Result<(), i32> {
        let Some(mut t) = self.raw_target(r) else {
            return Ok(());
        };
        let (stride, rect) = (t.stride, t.rect);
        draw(t.pixels(), stride);
        self.damage(rect)
    }

    /// [`App::raw`] in two halves, so another thread of the app can write the pixels (FA12: the video converter) while
    /// this one keeps the session: the surface inside `r` (clipped to it), `None` while the app was never visible or
    /// `r` misses the surface. The same rule holds: Slint draws nothing over the rectangle while the target is in use.
    /// Nothing shows until [`App::damage`] for [`RawTarget::rect`].
    pub fn raw_target(&self, r: Rect) -> Option<RawTarget> {
        let base = self.surface()?;
        let (w, h) = (self.width as i32, self.height.get() as i32);
        let x0 = r.x.clamp(0, w);
        let y0 = r.y.clamp(0, h);
        let x1 = (r.x + r.w as i32).clamp(x0, w);
        let y1 = (r.y + r.h as i32).clamp(y0, h);
        if x1 == x0 || y1 == y0 {
            return None;
        }
        let stride = self.width;
        let start = y0 as usize * stride + x0 as usize;
        let end = (y1 as usize - 1) * stride + x1 as usize;
        Some(RawTarget {
            // SAFETY: the server made the chunk `width * screen height * 4` bytes, committed, and never frees it while
            // the session lives; `start..end` lies inside it (clipped above).
            px: unsafe { base.add(start).cast::<u32>() },
            len: end - start,
            stride,
            rect: Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32),
        })
    }

    /// Shows a rectangle of the surface the app wrote itself ([`App::raw_target`]); returns once the server has
    /// copied it, so the pixels may be written again from then on.
    pub fn damage(&self, r: Rect) -> Result<(), i32> {
        let mut args = TIpcArgs::new();
        args.int(0, r.x)
            .int(1, r.y)
            .int(2, r.w as i32)
            .int(3, r.h as i32);
        self.server.send_receive(proto::DAMAGE, &args).map(|_| ())
    }

    /// The surface's first pixel (`width * screen height` of them): asked of the server on the first frame shown;
    /// `None` while the app was never visible.
    fn surface(&self) -> Option<*mut Xrgb8888> {
        if self.surface.borrow().is_none() {
            if !self.visible.get() {
                return None;
            }
            let h = self
                .server
                .send_receive(proto::SURFACE, &TIpcArgs::new())
                .ok()?;
            *self.surface.borrow_mut() = Some(Chunk::from_handle(h));
        }
        self.surface
            .borrow()
            .as_ref()
            .map(|c| c.base().cast::<Xrgb8888>())
    }

    /// The window's height in pixels: the body, or the screen while a full-screen page shows.
    pub fn height(&self) -> usize {
        self.height.get()
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
        // nothing to show yet: what changed stays dirty until the app is shown
        let Some(base) = self.surface() else {
            return Ok(());
        };
        // SAFETY: the server made the chunk `width * screen height * 4` bytes, committed, and never frees it while
        // the session lives; only this thread writes it and the server only reads it; the slice lives for this call.
        let px = unsafe { core::slice::from_raw_parts_mut(base, self.width * self.height.get()) };
        let screen = Rect::new(0, 0, self.width as u32, self.height.get() as u32);
        // turned, list scrolls aren't shifted rows of the surface: drawn as usual
        let (moved, list) = if self.landscape() {
            ((0, 0), Rect::default())
        } else {
            (moved, list)
        };
        let mut plan = self
            .scrolling
            .borrow_mut()
            .plan(px, self.width, screen, moved, list);
        if self.full.replace(false) {
            plan = ScrollPlan::Full;
        }
        if plan == ScrollPlan::Full {
            self.window.request_redraw();
        }
        let mut result = Ok(());
        self.window.draw_if_needed(|r| {
            r.set_rendering_rotation(self.rotation.get());
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
            // one DAMAGE per frame, the union: the server answers it once the frame is on screen, so the next render
            // never writes pixels the server hasn't copied yet
            let damage = region.iter().fold(None, |acc: Option<Rect>, (o, s)| {
                let r = Rect::new(o.x, o.y, s.width, s.height);
                Some(acc.map_or(r, |a| a.union(&r)))
            });
            if let Some(d) = damage {
                let mut args = TIpcArgs::new();
                args.int(0, d.x)
                    .int(1, d.y)
                    .int(2, d.w as i32)
                    .int(3, d.h as i32);
                if let Err(e) = self.server.send_receive(proto::DAMAGE, &args) {
                    result = Err(e);
                }
            }
        });
        result
    }

    /// Show the system keyboard in `mode` (`proto::KEYBOARD_TEXT`, `KEYBOARD_TERMINAL`) or hide it
    /// (`KEYBOARD_HIDDEN`); returns how many pixels of the surface's bottom it now covers. When that changes later
    /// (the user switched language) the app gets `AppEvent::Keyboard`.
    pub fn keyboard(&self, mode: i32) -> Result<u32, i32> {
        let mut args = TIpcArgs::new();
        args.int(0, mode);
        self.server
            .send_receive(proto::KEYBOARD, &args)
            .map(|h| h as u32)
    }

    /// A setting's effective value: stored, else its registered default (`proto::SETTING_GET`).
    ///
    /// # Errors
    /// The server's: `KErrNotFound` for a key with no value and no default, `KErrArgument` for an invalid key or a
    /// stored value of another type than `T`.
    pub fn setting<T: Setting>(&self, key: SettingKey<T>) -> Result<T, i32> {
        T::from_value(self.setting_raw(key.name())?).ok_or(kerr::ARGUMENT)
    }

    /// A setting by name, untyped (for keys only known at run time).
    ///
    /// # Errors
    /// As [`App::setting`].
    pub fn setting_raw(&self, name: &str) -> Result<Value, i32> {
        let name = TPtrC8::new(name.as_bytes());
        let mut buf = vec![0u8; ENCODED_MAX];
        let len = {
            let mut out = TPtr8::new(&mut buf);
            let mut args = TIpcArgs::new();
            args.desc8(0, &name).des8(1, &mut out);
            self.server.send_receive(proto::SETTING_GET, &args)?;
            out.len()
        };
        Value::decode(&buf[..len]).map_err(|e| e.to_symbian())
    }

    /// Writes a setting (`proto::SETTING_SET`). An app may write keys in its own namespace (`clock.*`) only.
    ///
    /// # Errors
    /// The server's: `KErrPermissionDenied` for a key the app may not write, `KErrArgument`/`KErrTooBig` for an
    /// invalid key or value.
    pub fn set_setting<T: Setting>(&self, key: SettingKey<T>, value: T) -> Result<(), i32> {
        self.set_setting_raw(key.name(), &value.into_value())
    }

    /// Writes a setting by name, untyped.
    ///
    /// # Errors
    /// As [`App::set_setting`].
    pub fn set_setting_raw(&self, name: &str, value: &Value) -> Result<(), i32> {
        let encoded = value.to_bytes();
        let (name, value) = (TPtrC8::new(name.as_bytes()), TPtrC8::new(&encoded));
        let mut args = TIpcArgs::new();
        args.desc8(0, &name).desc8(1, &value);
        self.server
            .send_receive(proto::SETTING_SET, &args)
            .map(drop)
    }

    /// Asks for `AppEvent::Settings` whenever a key starting with `prefix` changes (`""`: any key;
    /// `proto::SETTING_WATCH`).
    ///
    /// # Errors
    /// The server's: `KErrArgument` for an invalid prefix, `KErrOverflow` past the per-app limit.
    pub fn watch_settings(&self, prefix: &str) -> Result<(), i32> {
        let prefix = TPtrC8::new(prefix.as_bytes());
        let mut args = TIpcArgs::new();
        args.desc8(0, &prefix);
        self.server
            .send_receive(proto::SETTING_WATCH, &args)
            .map(drop)
    }

    /// Describe the page on show (protocol 1.8): its name (Open apps), whether Back stays inside the app (a kit layer
    /// open, a page to go up to: the app gets `AppEvent::Back`), full screen. The app draws its header and footer
    /// itself (the kit). Send it again when any of it changes. A full-screen page makes the window as tall as the
    /// screen (and the body again when it ends); redraw after a change of height.
    pub fn page(&self, page: &Page) -> Result<(), i32> {
        let b = page.encode().map_err(|_| kerr::ARGUMENT)?;
        let d = TPtrC8::new(&b);
        let mut args = TIpcArgs::new();
        args.desc8(0, &d);
        let h = self.server.send_receive(proto::PAGE, &args)? as usize;
        if h > 0 {
            self.resize(h);
        }
        Ok(())
    }

    /// Ask with a shell dialog; the answer comes as `AppEvent::Answer` with the button's index (Back answers the last).
    pub fn query(&self, query: &Query) -> Result<(), i32> {
        let b = query.encode().map_err(|_| kerr::ARGUMENT)?;
        self.send_query(&b)
    }

    /// Take the open dialog away unanswered.
    pub fn close_query(&self) -> Result<(), i32> {
        self.send_query(&[])
    }

    fn send_query(&self, b: &[u8]) -> Result<(), i32> {
        let d = TPtrC8::new(b);
        let mut args = TIpcArgs::new();
        args.desc8(0, &d);
        self.server.send_receive(proto::QUERY, &args).map(drop)
    }

    /// A toast above the key row or band for 3 s (`proto::NOTE`); a tap on `action` ("Undo"; "" = none) sends
    /// `AppEvent::NoteAction`.
    pub fn note(&self, text: &str, action: &str) -> Result<(), i32> {
        let n = Note {
            text: text.into(),
            action: action.into(),
        };
        self.send_bytes(proto::NOTE, &n.encode().map_err(|_| kerr::ARGUMENT)?)
    }

    /// Take this app's toast away.
    pub fn close_note(&self) -> Result<(), i32> {
        self.note("", "")
    }

    /// Add or replace an entry in the shade (`proto::NOTIFY`); a tap brings the app to the front with
    /// `AppEvent::Page { page }` then `AppEvent::Notified { id, action: 0 }`, a tap on action `i` with `Notified {
    /// id, action: i + 1 }`.
    pub fn notify(&self, n: &Notification) -> Result<(), i32> {
        self.send_bytes(proto::NOTIFY, &n.encode().map_err(|_| kerr::ARGUMENT)?)
    }

    /// Remove every card of this app from the shade, ongoing ones too (1.18).
    pub fn clear_notify(&self) -> Result<(), i32> {
        self.server
            .send_receive(proto::NOTIFY_CLEAR, &TIpcArgs::new())
            .map(drop)
    }

    /// Remove the shade's entry `id`.
    pub fn cancel_notify(&self, id: u32) -> Result<(), i32> {
        self.notify(&Notification {
            id,
            ..Notification::default()
        })
    }

    /// The badge counter `id` on the app's launcher icon (`proto::BADGE`; the counters are summed); 0 removes it.
    pub fn badge(&self, id: u32, count: u32) -> Result<(), i32> {
        let mut args = TIpcArgs::new();
        args.int(0, id as i32).int(1, count as i32);
        self.server.send_receive(proto::BADGE, &args).map(drop)
    }

    /// Add or replace launcher entry `id` (`proto::ENTRY`): a caption (empty removes it) and an icon (empty: a
    /// generic tile; else `w: u16, h: u16` LE then `w * h` ARGB u32 LE).
    pub fn entry(&self, id: u32, caption: &str, icon: &[u8]) -> Result<(), i32> {
        let (c, i) = (TPtrC8::new(caption.as_bytes()), TPtrC8::new(icon));
        let mut args = TIpcArgs::new();
        args.int(0, id as i32).desc8(1, &c).desc8(2, &i);
        self.server.send_receive(proto::ENTRY, &args).map(drop)
    }

    /// Open app `key` (`"phone"`, `"contacts"`, `"messages"`...) on its page `page` with `arg` (a number; at most
    /// `proto::ARG_MAX` bytes) (`proto::OPEN`): it gets `AppEvent::Open { page }` and reads `arg` with
    /// [`App::arg`]; Back at its root comes back here.
    pub fn open(&self, key: &str, page: u16, arg: &str) -> Result<(), i32> {
        self.open_with(key, page, arg, 0)
    }

    /// [`App::open`] with `flags` (1.19, `proto::OPEN_WAKE`): they count only from a service.
    pub fn open_with(&self, key: &str, page: u16, arg: &str, flags: i32) -> Result<(), i32> {
        let (k, a) = (TPtrC8::new(key.as_bytes()), TPtrC8::new(arg.as_bytes()));
        let mut args = TIpcArgs::new();
        args.desc8(0, &k)
            .int(1, i32::from(page))
            .desc8(2, &a)
            .int(3, flags);
        self.server.send_receive(proto::OPEN, &args).map(drop)
    }

    /// What only the shell knows, for the Settings app (`proto::SETTINGS_INFO`): kind `proto::INFO_BATTERY`,
    /// `INFO_NET` or `INFO_CLOCK`, encoded as `neoui_proto::settings_app::info` decodes it.
    pub fn settings_info(&self, kind: i32) -> Result<alloc::vec::Vec<u8>, i32> {
        use neoui_proto::settings_app::info::{BATTERY_MAX, INFO_MAX};
        let mut buf =
            alloc::vec![0u8; if kind == proto::INFO_APPS { INFO_MAX } else { BATTERY_MAX }];
        let n = {
            let mut des = TPtr8::new(&mut buf);
            let mut args = TIpcArgs::new();
            args.int(0, kind).int(1, 0).des8(2, &mut des);
            self.server.send_receive(proto::SETTINGS_INFO, &args)?;
            des.len()
        };
        buf.truncate(n);
        Ok(buf)
    }

    /// Ask the shell to act for the Settings app (`proto::SETTINGS_ACTION`, e.g. `proto::ACTION_SIGN_IN`).
    pub fn settings_action(&self, kind: i32) -> Result<(), i32> {
        self.settings_action_with(kind, &[])
    }

    /// [`Self::settings_action`] with its request (`proto::ACTION_OPEN_ENTRY`: the old app's id, u32 LE).
    pub fn settings_action_with(&self, kind: i32, request: &[u8]) -> Result<(), i32> {
        let des = TPtrC8::new(request);
        let mut args = TIpcArgs::new();
        args.int(0, kind).desc8(1, &des);
        self.server
            .send_receive(proto::SETTINGS_ACTION, &args)
            .map(drop)
    }

    /// (D6) A control accepted a touch (`proto::CONTROL_FEEDBACK`): the shell pulses as the profile says (`key`:
    /// 0). The kit's controls
    /// report through [`kit_feedback!`]; an app's own touch control can call this from its click.
    pub fn control_feedback(&self, key: u8) -> Result<(), i32> {
        self.feedback(proto::FEEDBACK_CONTROL, key)
    }

    /// (D6) [`Self::control_feedback`] for a dial pad key (`key`: its ASCII digit, `*` or `#`).
    pub fn dial_key_feedback(&self, key: u8) -> Result<(), i32> {
        self.feedback(proto::FEEDBACK_DIAL_KEY, key)
    }

    fn feedback(&self, kind: i32, key: u8) -> Result<(), i32> {
        let mut args = TIpcArgs::new();
        args.int(0, kind).int(1, i32::from(key));
        self.server
            .send_receive(proto::CONTROL_FEEDBACK, &args)
            .map(drop)
    }

    /// The argument of the `OPEN` that sent the last `AppEvent::Open` (empty: none); reading takes it.
    pub fn arg(&self) -> Result<alloc::string::String, i32> {
        let mut buf = alloc::vec![0u8; proto::ARG_MAX];
        let n = {
            let mut des = TPtr8::new(&mut buf);
            let mut args = TIpcArgs::new();
            args.des8(0, &mut des);
            self.server.send_receive(proto::ARG, &args)?
        };
        buf.truncate(n.max(0) as usize);
        alloc::string::String::from_utf8(buf).map_err(|_| kerr::ARGUMENT)
    }

    /// The task the app was opened for is finished (a call ended): the shell shows where the user came from
    /// (`proto::DONE`). The app keeps running.
    pub fn done(&self) -> Result<(), i32> {
        self.server
            .send_receive(proto::DONE, &TIpcArgs::new())
            .map(drop)
    }

    /// The clipboard's text, shared by every app and the shell (Copy text, Paste); `None` when empty.
    pub fn clipboard(&self) -> Option<alloc::string::String> {
        clip_get(&self.server)
    }

    /// What the clipboard holds: `proto::CLIP_TEXT`, or files (`CLIP_PATHS_COPY`, `CLIP_PATHS_MOVE`) whose full
    /// paths are its text, one per line (`proto::CLIP_KIND`).
    pub fn clipboard_kind(&self) -> i32 {
        self.server
            .send_receive(proto::CLIP_KIND, &TIpcArgs::new())
            .unwrap_or(proto::CLIP_TEXT)
    }

    /// Put files on the clipboard to copy or move (`kind`: `proto::CLIP_PATHS_COPY` or `CLIP_PATHS_MOVE`); false when
    /// the paths are over `proto::CLIP_MAX` bytes together or the server refused.
    pub fn set_clipboard_paths(&self, kind: i32, paths: &[&str]) -> bool {
        let text = paths.join("\n");
        if text.len() > proto::CLIP_MAX {
            return false;
        }
        let t = TPtrC8::new(text.as_bytes());
        let mut args = TIpcArgs::new();
        args.desc8(0, &t).int(1, kind);
        self.server.send_receive(proto::CLIP_SET, &args).is_ok()
    }

    /// Open app `key` on its page `page` with an open file (`proto::OPEN_FILE`), e.g. Open with: it takes the file
    /// with [`App::arg_file`]. `file`'s session must be shared (`Fs::share_protected`); `file` stays open here.
    pub fn open_file(&self, key: &str, page: u16, file: &File<'_>) -> Result<(), i32> {
        let k = TPtrC8::new(key.as_bytes());
        let mut args = TIpcArgs::new();
        args.desc8(0, &k).int(1, i32::from(page));
        file.transfer_to_server(&mut args, 2, 3)?;
        self.server.send_receive(proto::OPEN_FILE, &args).map(drop)
    }

    /// The file the last `OPEN_FILE` of this app carried, if it hasn't been taken yet (`proto::ARG_FILE`).
    pub fn arg_file(&self) -> Option<SharedFile> {
        self.take_file(proto::ARG_FILE)
    }

    /// The file an app this one opened answered with (a file picked or saved in Files), once this app is shown
    /// again (`AppEvent::Visible(true)`); `None` when it answered without one (`proto::RESULT_FILE`).
    pub fn result_file(&self) -> Option<SharedFile> {
        self.take_file(proto::RESULT_FILE)
    }

    /// Finish the task with a file for the app that opened this one, which takes it with [`App::result_file`]
    /// (`proto::DONE_FILE`); the shell shows that app again, as [`App::done`].
    pub fn done_file(&self, file: &File<'_>) -> Result<(), i32> {
        let mut args = TIpcArgs::new();
        file.transfer_to_server(&mut args, 0, 1)?;
        self.server.send_receive(proto::DONE_FILE, &args).map(drop)
    }

    fn take_file(&self, function: i32) -> Option<SharedFile> {
        let mut handle = [0u8; 4];
        let fs = {
            let mut des = TPtr8::new(&mut handle);
            let mut args = TIpcArgs::new();
            args.des8(0, &mut des);
            self.server.send_receive(function, &args).ok()?
        };
        SharedFile::adopt_from_server(fs, i32::from_le_bytes(handle)).ok()
    }

    /// Put `text` on the clipboard; false when it is over `proto::CLIP_MAX` bytes or the server refused.
    pub fn set_clipboard(&self, text: &str) -> bool {
        clip_set(&self.server, text)
    }

    /// Show the app's surface (true) or the shell's screen for its key (false) (`proto::FRONT`).
    pub fn front(&self, front: bool) -> Result<(), i32> {
        let mut args = TIpcArgs::new();
        args.int(0, front as i32);
        self.server.send_receive(proto::FRONT, &args).map(drop)
    }

    fn send_bytes(&self, op: i32, b: &[u8]) -> Result<(), i32> {
        let d = TPtrC8::new(b);
        let mut args = TIpcArgs::new();
        args.desc8(0, &d);
        self.server.send_receive(op, &args).map(drop)
    }

    /// A key from the server (a hardware key or the system keyboard: Unicode, Enter 13, Backspace 8, Escape 27, Tab 9,
    /// the arrows 0xF807-0xF80A) as a Slint key press and release, for text fields.
    pub fn dispatch_key(&self, code: u32) {
        // Control (0x11, the keyboard's edit bar) holds for the next key: Ctrl+A, Ctrl+C, Ctrl+V
        if code == 0x11 {
            self.ctrl.set(true);
            return;
        }
        let ctrl = self.ctrl.replace(false);
        let control: SharedString = Key::Control.into();
        if ctrl {
            self.window.dispatch_event(WindowEvent::KeyPressed {
                text: control.clone(),
            });
        }
        let text: SharedString = match code {
            13 | 10 => Key::Return.into(),
            8 => Key::Backspace.into(),
            27 => Key::Escape.into(),
            9 => Key::Tab.into(),
            // Symbian's arrow key codes (the system keyboard's terminal row)
            0xF807 => Key::LeftArrow.into(),
            0xF808 => Key::RightArrow.into(),
            0xF809 => Key::UpArrow.into(),
            0xF80A => Key::DownArrow.into(),
            c => match char::from_u32(c) {
                Some(ch) => ch.into(),
                None => {
                    if ctrl {
                        self.window
                            .dispatch_event(WindowEvent::KeyReleased { text: control });
                    }
                    return;
                }
            },
        };
        self.window
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.window
            .dispatch_event(WindowEvent::KeyReleased { text });
        if ctrl {
            self.window
                .dispatch_event(WindowEvent::KeyReleased { text: control });
        }
    }

    /// A touch from the server (surface coordinates) as Slint pointer events; keys are the app's business.
    pub fn dispatch_touch(&self, input: InputEvent) {
        if let InputEvent::Touch { x, y, phase, .. } = input {
            // landscape: surface (x, y) is window (y, W - 1 - x), W the surface's width
            let (x, y) = if self.landscape() {
                (y, self.width as i32 - 1 - x)
            } else {
                (x, y)
            };
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

/// (D6) Tell the shell about the kit's controls that acted since the last call (`Kit.accepted`, `Kit.dialled`), one
/// `CONTROL_FEEDBACK` each kind: the shell answers one press once. Call it after each event, in the app's crate where
/// its generated `Kit` global is: `neoapp::kit_feedback!(ui, app);`. Errors (an older shell) are dropped.
#[macro_export]
macro_rules! kit_feedback {
    ($ui:expr, $app:expr) => {{
        let ui = &$ui;
        let kit = ui.global::<Kit>();
        if kit.get_dialled() != kit.get_dialled_sent() {
            kit.set_dialled_sent(kit.get_dialled());
            kit.set_accepted_sent(kit.get_accepted());
            let key = kit.get_dial_key().as_bytes().first().copied().unwrap_or(0);
            let _ = $app.dial_key_feedback(key);
        } else if kit.get_accepted() != kit.get_accepted_sent() {
            kit.set_accepted_sent(kit.get_accepted());
            let _ = $app.control_feedback(0);
        }
    }};
}

/// Back through the kit's layers first (Navigation v2 rule 10): with a menu, sheet, search or selection open, the top
/// one closes and this is true; otherwise false (the app goes up a page, or its page has `can_back` false and Back
/// never came). A layer whose component went away while open is forgotten. Use in the app's crate, where its
/// generated `Kit` global is: `Some(AppEvent::Back) if neoapp::kit_back!(ui) => {}`.
#[macro_export]
macro_rules! kit_back {
    ($ui:expr) => {{
        // bound first: `$ui` may be a temporary (`ctx.ui()`)
        let ui = &$ui;
        let kit = ui.global::<Kit>();
        if kit.get_layers() > 0 {
            let seq = kit.get_back_seq() + 1;
            kit.set_back_seq(seq);
            // the layers see back-seq change in their change handlers
            slint::platform::update_timers_and_animations();
            if kit.get_answered() == seq {
                true
            } else {
                kit.set_layers(0);
                false
            }
        } else {
            false
        }
    }};
}

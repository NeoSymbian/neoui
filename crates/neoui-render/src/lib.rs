// SPDX-License-Identifier: Apache-2.0
//! Renders the neoui launcher with Slint's software renderer into a raw
//! XRGB8888 buffer. Platform-agnostic: the caller owns the buffer, the clock
//! and the input source.
#![no_std]

extern crate alloc;

pub mod layer;

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::time::Duration;

use neoui_proto::{Damage, InputEvent, Rect, ScrollPlan, Scrolling, TouchPhase};
use slint::platform::software_renderer::{
    LineBufferProvider, MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
    TargetPixel,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, PlatformError};

/// The home screen's page count (`ui/screens/home.slint`).
pub const HOME_PAGES: i32 = 3;

slint::include_modules!();

/// One XRGB8888 pixel, `0xFFRRGGBB` (alpha byte always 0xFF).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

struct NeoPlatform {
    window: Rc<MinimalSoftwareWindow>,
    now_ms: fn() -> u64,
}

impl Platform for NeoPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        Duration::from_millis((self.now_ms)())
    }
}

/// Owns the Slint platform and the launcher UI.
///
/// Slint's platform is a process-wide singleton (and with
/// `unsafe-single-threaded` it must only be used from one thread), so only one
/// `Renderer` can exist per process; a second `new` returns an error.
pub struct Renderer {
    window: Rc<MinimalSoftwareWindow>,
    ui: Shell,
    typed: Rc<RefCell<Vec<u32>>>,
    nav: Rc<RefCell<Vec<i32>>>,
    closed: Rc<RefCell<Vec<alloc::string::String>>>,
    power: Rc<RefCell<Vec<i32>>>,
    /// lists scrolled by shifting pixels (smooth-ui S2d), and how many frames did so
    scrolling: Scrolling,
    shifts: u32,
}

/// Slint's lines for a scrolled list's frame (smooth-ui S2d): the rows of `list` outside `draw` were moved into place
/// already ([`neoui_proto::shift_rows`]), so Slint skips them; everything else it draws as usual.
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

impl Renderer {
    /// `now_ms` is a monotonic millisecond clock used for animations and timers.
    pub fn new(width: u32, height: u32, now_ms: fn() -> u64) -> Result<Self, PlatformError> {
        // The caller keeps one buffer and we redraw only what changed in it.
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(NeoPlatform {
            window: window.clone(),
            now_ms,
        }))
        .map_err(PlatformError::SetPlatformError)?;
        let ui = Shell::new()?;
        window.set_size(PhysicalSize::new(width, height));
        ui.show()?;
        let typed: Rc<RefCell<Vec<u32>>> = Rc::default();
        let sink = typed.clone();
        ui.on_typed(move |s| sink.borrow_mut().extend(s.chars().map(u32::from)));
        let nav: Rc<RefCell<Vec<i32>>> = Rc::default();
        let sink = nav.clone();
        ui.on_app_nav(move |b| sink.borrow_mut().push(b));
        let closed: Rc<RefCell<Vec<alloc::string::String>>> = Rc::default();
        let sink = closed.clone();
        ui.on_close_app(move |k| sink.borrow_mut().push(k.as_str().into()));
        let power: Rc<RefCell<Vec<i32>>> = Rc::default();
        let sink = power.clone();
        ui.on_power(move |a| sink.borrow_mut().push(a));
        Ok(Self {
            window,
            ui,
            typed,
            nav,
            closed,
            power,
            scrolling: Scrolling::default(),
            shifts: 0,
        })
    }

    /// Renders what changed since the last call into `buf` (XRGB8888, `stride_px`
    /// pixels per row, at least `stride_px * height` long) and returns the dirty region.
    /// `buf` must hold the previous frame (`RepaintBufferType::ReusedBuffer`).
    /// Jump straight to `screen` (an app key, "home" or "menu") and its sub-`view` ("" = the main view), closing
    /// any overlay. For snapshots and the host's `--screen`.
    pub fn show(&mut self, screen: &str, view: &str) {
        self.ui.set_screen(screen.into());
        self.ui.set_view(view.into());
        self.ui.set_options_open(false);
        self.ui.set_shade_open(false);
    }

    /// The current screen key: "home", "menu" or an app key such as "clock".
    pub fn screen(&self) -> alloc::string::String {
        self.ui.get_screen().as_str().into()
    }

    /// Where an app's content goes: the full-width band between the status bar and the toolbar (physical pixels).
    pub fn body(&self) -> Rect {
        let size = self.window.size();
        Rect::new(
            0,
            self.ui.get_body_y() as i32,
            size.width,
            self.ui.get_body_h() as u32,
        )
    }

    /// The home screen's pages band, where its swipe works (physical pixels; smooth-ui S2c).
    pub fn home_strip(&self) -> Rect {
        Rect::new(
            0,
            self.ui.get_body_y() as i32,
            self.window.size().width,
            self.ui.get_home_strip_h() as u32,
        )
    }

    /// The home page on show (0..`HOME_PAGES`).
    pub fn home_page(&self) -> i32 {
        self.ui.get_home_page()
    }

    /// Show home page `n` (at once with server transitions on).
    pub fn set_home_page(&mut self, n: i32) {
        self.ui.set_home_page(n.clamp(0, HOME_PAGES - 1));
    }

    /// Show or hide the system keyboard over the bottom of the body.
    /// Which build is running, one line per part (kernel, user-side ROM, neoui): Settings -> About phone.
    pub fn set_build_info(&mut self, lines: &[String]) {
        let model: Vec<slint::SharedString> = lines.iter().map(|l| l.as_str().into()).collect();
        self.ui
            .set_build_info(Rc::new(slint::VecModel::from(model)).into());
    }

    /// The UI server animates screen changes itself, from snapshots (smooth-ui S2): Slint switches screens at once.
    pub fn set_server_transitions(&mut self, on: bool) {
        self.ui.set_server_transitions(on);
    }

    pub fn set_keyboard(&mut self, open: bool) {
        self.ui.set_keyboard_open(open);
    }

    /// The keyboard's height (physical pixels) when it is open.
    pub fn keyboard_h(&self) -> u32 {
        self.ui.get_keyboard_h() as u32
    }

    /// What the keyboard typed since the last call, as Unicode code points (Backspace 8, Enter 13).
    pub fn take_typed(&mut self) -> Vec<u32> {
        core::mem::take(&mut *self.typed.borrow_mut())
    }

    /// Whether the foreground app handles the toolbar's Back and Options itself.
    pub fn set_app_nav(&mut self, back: bool, options: bool) {
        self.ui.set_app_back(back);
        self.ui.set_app_options(options);
    }

    /// Claimed toolbar buttons pressed since the last call: 1 = Back, 2 = Options (`neoui_proto::ipc::NAV_*`).
    pub fn take_nav(&mut self) -> Vec<i32> {
        core::mem::take(&mut *self.nav.borrow_mut())
    }

    /// The running apps' keys, most recent first, for the switcher (screen `switcher`).
    pub fn set_running(&mut self, keys: &[&str]) {
        let model: Vec<slint::SharedString> = keys.iter().map(|k| (*k).into()).collect();
        self.ui
            .set_running(slint::ModelRc::from(Rc::new(slint::VecModel::from(model))));
    }

    /// Apps the switcher's × asked to close since the last call.
    pub fn take_closed(&mut self) -> Vec<alloc::string::String> {
        core::mem::take(&mut *self.closed.borrow_mut())
    }

    /// Show or hide the Power menu (the UI server opens it when Power is held).
    pub fn set_power_menu(&mut self, open: bool) {
        self.ui.set_power_open(open);
    }

    /// What the Power menu chose since the last call: 2 = power off, 0 = restart, -1 = dismissed.
    pub fn take_power(&mut self) -> Vec<i32> {
        core::mem::take(&mut *self.power.borrow_mut())
    }

    /// The status bar's time text, e.g. "09:41".
    pub fn set_clock(&mut self, text: &str) {
        self.ui.set_clock(text.into());
    }

    pub fn render(&mut self, buf: &mut [u32], stride_px: usize) -> Damage {
        slint::platform::update_timers_and_animations();
        // smooth-ui S2d: one list scrolled (`ui/components/scroll.slint`, its change handler ran above): move what it
        // showed, and Slint draws only the rows that came into view. Once the scrolling stops, one full redraw puts
        // right whatever the skipped rows missed (a highlight, a sub-pixel position).
        let sc = self.ui.global::<Scroll>();
        let (dy, lists) = (sc.get_dy(), sc.get_lists());
        sc.set_dy(0);
        sc.set_lists(0);
        let size = self.window.size();
        let list = Rect::new(
            sc.get_x() as i32,
            sc.get_y() as i32,
            sc.get_width() as u32,
            sc.get_height() as u32,
        );
        let screen = Rect::new(0, 0, size.width, size.height);
        let plan = self
            .scrolling
            .plan(buf, stride_px, screen, (dy, lists), list);
        let full = plan == ScrollPlan::Full;
        if full {
            self.window.request_redraw();
        } else if plan != ScrollPlan::Normal {
            self.shifts += 1;
        }
        // SAFETY: Xrgb8888 is repr(transparent) over u32, same size/alignment.
        let px = unsafe {
            core::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<Xrgb8888>(), buf.len())
        };
        let mut damage = Damage::new();
        self.window.draw_if_needed(|r| {
            if full {
                // a change of buffer type drops Slint's cache, so this render draws the whole window
                r.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
                r.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            }
            let region = match plan {
                ScrollPlan::Shift { list, draw } => r.render_by_line(ScrollLines {
                    px,
                    stride: stride_px,
                    list,
                    draw,
                }),
                _ => r.render(px, stride_px),
            };
            for (o, s) in region.iter() {
                damage.add(Rect::new(o.x, o.y, s.width, s.height));
            }
        });
        damage
    }

    /// Feeds one input event to the UI. Touch positions are physical pixels
    /// (scale factor 1, so equal to Slint logical pixels).
    pub fn dispatch(&mut self, ev: InputEvent) {
        // shortcut: single pointer, all touch ids drive it; real multi-touch when Slint grows it.
        let event = match ev {
            InputEvent::Touch { x, y, phase, .. } => {
                let position = LogicalPosition::new(x as f32, y as f32);
                let button = PointerEventButton::Left;
                match phase {
                    TouchPhase::Down => WindowEvent::PointerPressed { position, button },
                    TouchPhase::Move => WindowEvent::PointerMoved { position },
                    TouchPhase::Up => {
                        self.window
                            .dispatch_event(WindowEvent::PointerReleased { position, button });
                        // A lifted finger does not hover.
                        WindowEvent::PointerExited
                    }
                }
            }
            InputEvent::Key { code, down } => {
                let Some(ch) = char::from_u32(code) else {
                    return;
                };
                let text = ch.into();
                if down {
                    WindowEvent::KeyPressed { text }
                } else {
                    WindowEvent::KeyReleased { text }
                }
            }
        };
        self.window.dispatch_event(event);
    }

    /// Frames that scrolled a list by shifting its pixels since the last call (smooth-ui S2d).
    pub fn take_shifts(&mut self) -> u32 {
        core::mem::take(&mut self.shifts)
    }

    /// True while an animation runs, so the caller should keep rendering.
    pub fn has_active_animations(&self) -> bool {
        self.window.has_active_animations()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_blend() {
        assert_eq!(Xrgb8888::from_rgb(0x12, 0x34, 0x56).0, 0xFF12_3456);
        let mut p = Xrgb8888(0xFF00_00FF);
        // 50% premultiplied red over blue.
        p.blend(PremultipliedRgbaColor {
            red: 128,
            green: 0,
            blue: 0,
            alpha: 128,
        });
        assert_eq!(p.0, 0xFF80_007F);
        let mut q = Xrgb8888(0xFF00_00FF);
        q.blend(PremultipliedRgbaColor {
            red: 1,
            green: 2,
            blue: 3,
            alpha: 255,
        });
        assert_eq!(q.0, 0xFF01_0203);
    }
}

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

use neoui_proto::{Damage, InputEvent, Rect, TouchPhase};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, PlatformError};

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
        // SAFETY: Xrgb8888 is repr(transparent) over u32, same size/alignment.
        let px = unsafe {
            core::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<Xrgb8888>(), buf.len())
        };
        let mut damage = Damage::new();
        self.window.draw_if_needed(|r| {
            for (o, s) in r.render(px, stride_px).iter() {
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

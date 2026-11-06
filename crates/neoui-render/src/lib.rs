// SPDX-License-Identifier: Apache-2.0
//! Renders the neoui launcher with Slint's software renderer into a raw
//! XRGB8888 buffer. Platform-agnostic: the caller owns the buffer, the clock
//! and the input source.
#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::rc::Rc;
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
    _ui: Launcher,
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
        let ui = Launcher::new()?;
        window.set_size(PhysicalSize::new(width, height));
        ui.show()?;
        Ok(Self { window, _ui: ui })
    }

    /// Renders what changed since the last call into `buf` (XRGB8888, `stride_px`
    /// pixels per row, at least `stride_px * height` long) and returns the dirty region.
    /// `buf` must hold the previous frame (`RepaintBufferType::ReusedBuffer`).
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

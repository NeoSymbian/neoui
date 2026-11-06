// SPDX-License-Identifier: Apache-2.0
//! Desktop host: renders the launcher into a fixed-size XRGB8888 framebuffer
//! (as the Symbian framebuffer will be) and shows it scaled in a window.
//! Mouse button + motion become touch events.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Instant;

use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

struct App {
    renderer: Renderer,
    fb: Vec<u32>,
    fb_w: usize,
    fb_h: usize,
    window_size: LogicalSize<f64>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    cursor: PhysicalPosition<f64>,
    pressed: bool,
}

impl App {
    /// Maps a window position to framebuffer pixels (the window shows the fb scaled).
    fn touch(&mut self, phase: TouchPhase) {
        let Some(window) = self.surface.as_ref().map(|s| s.window()) else {
            return;
        };
        let size = window.inner_size();
        let x = (self.cursor.x * self.fb_w as f64 / size.width.max(1) as f64) as i32;
        let y = (self.cursor.y * self.fb_h as f64 / size.height.max(1) as f64) as i32;
        self.renderer
            .dispatch(InputEvent::Touch { id: 0, x, y, phase });
        window.request_redraw();
    }

    fn redraw(&mut self) {
        self.renderer.render(&mut self.fb, self.fb_w);
        let Some(surface) = &mut self.surface else {
            return;
        };
        let size = surface.window().inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        surface.resize(w, h).expect("resize surface");
        let mut buf = surface.buffer_mut().expect("surface buffer");
        let (sw, sh) = (size.width as usize, size.height as usize);
        // Nearest-neighbour scale; softbuffer wants the top byte zero.
        for y in 0..sh {
            let src = &self.fb[y * self.fb_h / sh * self.fb_w..][..self.fb_w];
            let dst = &mut buf[y * sw..][..sw];
            for (x, d) in dst.iter_mut().enumerate() {
                *d = src[x * self.fb_w / sw] & 0x00FF_FFFF;
            }
        }
        buf.present().expect("present");
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("neoui")
            .with_inner_size(self.window_size)
            .with_resizable(false);
        let window = Rc::new(event_loop.create_window(attrs).expect("create window"));
        let context = Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&context, window.clone()).expect("softbuffer surface");
        window.request_redraw();
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) => {
                if let Some(s) = &self.surface {
                    s.window().request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if self.pressed {
                    self.touch(TouchPhase::Move);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.pressed = state == ElementState::Pressed;
                self.touch(if self.pressed {
                    TouchPhase::Down
                } else {
                    TouchPhase::Up
                });
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.has_active_animations()
            && let Some(s) = &self.surface
        {
            s.window().request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let size = match args.as_slice() {
        [] => "pixel",
        [flag, s] if flag == "--size" => s.as_str(),
        _ => return Err("usage: neoui-host [--size pixel|syborg]".into()),
    };
    // (framebuffer px, window logical size)
    let ((fb_w, fb_h), window_size) = match size {
        "pixel" => ((1080, 2160), LogicalSize::new(540.0, 1080.0)),
        "syborg" => ((768, 640), LogicalSize::new(768.0, 640.0)),
        other => return Err(format!("unknown size {other:?}, expected pixel or syborg").into()),
    };
    let renderer =
        Renderer::new(fb_w as u32, fb_h as u32, now_ms).map_err(|e| format!("slint: {e}"))?;
    let mut app = App {
        renderer,
        fb: vec![0; fb_w * fb_h],
        fb_w,
        fb_h,
        window_size,
        surface: None,
        cursor: PhysicalPosition::new(0.0, 0.0),
        pressed: false,
    };
    EventLoop::new()?.run_app(&mut app)?;
    Ok(())
}

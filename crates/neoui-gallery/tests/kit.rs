// SPDX-License-Identifier: Apache-2.0
//! The kit gallery rendered at the Pixel's Gestures app surface (360 x 679 dp at 3 px/dp: the screen less the status
//! bar and the 16 dp band) into `snapshots/kit-*.png`, for side-by-side review with the canvas's kit sheets; and Back
//! closing the kit's layers one at a time, the top one first.
use neoui_gallery::{Gallery, GalleryState, Kit, Theme};
use slint::ComponentHandle;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, WindowAdapter};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

const W: u32 = 1080;
const H: u32 = 2037;

thread_local! {
    static NOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

struct Host(Rc<MinimalSoftwareWindow>);

impl Platform for Host {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        NOW.get()
    }
}

#[derive(Clone, Copy, Default)]
struct Px(u32);

impl TargetPixel for Px {
    fn blend(&mut self, c: PremultipliedRgbaColor) {
        let a = 255 - u32::from(c.alpha);
        let ch = |s: u32, v: u8| (u32::from(v) + ((self.0 >> s) & 0xff) * a / 255).min(255);
        self.0 = ch(16, c.red) << 16 | ch(8, c.green) << 8 | ch(0, c.blue);
    }
    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Px(u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
    }
}

fn shoot(window: &MinimalSoftwareWindow, name: &str) -> Vec<u32> {
    NOW.set(NOW.get() + Duration::from_secs(1));
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    let mut px = vec![Px(0); (W * H) as usize];
    window.draw_if_needed(|r| {
        r.render(&mut px, W as usize);
    });
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../snapshots");
    std::fs::create_dir_all(&root).unwrap();
    let file = std::fs::File::create(root.join(format!("{name}.png"))).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    let rgb: Vec<u8> = px
        .iter()
        .flat_map(|p| [(p.0 >> 16) as u8, (p.0 >> 8) as u8, p.0 as u8])
        .collect();
    w.write_image_data(&rgb).unwrap();
    px.into_iter().map(|p| p.0).collect()
}

fn at(px: &[u32], x_dp: u32, y_dp: u32) -> u32 {
    px[(y_dp * 3 * W + x_dp * 3) as usize]
}

/// One test: Slint's platform is per process.
#[test]
fn the_kit() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Host(window.clone()))).unwrap();
    let ui = Gallery::new().unwrap();
    ui.global::<Theme>().set_dp(3.0);
    window.set_size(slint::PhysicalSize::new(W, H));
    ui.show().unwrap();
    let g = ui.global::<GalleryState>();

    // Back closes the layers one at a time, top first: the Menus page opened its ⋮ menu, then the sheet
    g.set_page(3);
    shoot(&window, "kit-scratch");
    g.set_sheet(true);
    shoot(&window, "kit-scratch");
    let kit = ui.global::<Kit>();
    assert_eq!(kit.get_layers(), 2, "the menu and the sheet are layers");
    kit.set_back_seq(kit.get_back_seq() + 1);
    shoot(&window, "kit-scratch");
    assert!(!g.get_sheet(), "Back closed the sheet first");
    assert_eq!(kit.get_layers(), 1);
    kit.set_back_seq(kit.get_back_seq() + 1);
    shoot(&window, "kit-scratch");
    assert_eq!(kit.get_layers(), 0, "then the menu");
    std::fs::remove_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../snapshots/kit-scratch.png"),
    )
    .unwrap();

    let names = [
        "headers", "footer", "lists", "menus", "date", "time", "controls", "feedback",
    ];
    for (i, name) in names.iter().enumerate() {
        g.set_page(i as i32);
        if *name == "menus" {
            g.set_sheet(true);
        }
        let px = shoot(&window, &format!("kit-{name}"));
        match *name {
            // the root header: black, the title white, the hairline under it
            "headers" => assert_eq!(at(&px, 300, 30), 0x000000),
            // footer app: the active tab's pill is accent-tinted (Alarms, tab 1 of 5, 72 dp each, pill centred)
            "footer" => {
                let pill = at(&px, 72 + 36 - 22, 679 - 24);
                assert_ne!(pill, 0x000000, "the active tab sits on its pill");
            }
            _ => {}
        }
    }
}

// SPDX-License-Identifier: Apache-2.0
//! Shared snapshot helpers: a test clock that drives Slint's animations, PNG output to `snapshots/`,
//! screen snapshots, and the menu tap test.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;

static CLOCK_MS: AtomicU64 = AtomicU64::new(0);

fn now() -> u64 {
    CLOCK_MS.load(Ordering::Relaxed)
}

/// Moves the test clock past every running transition (the longest, a page switch, is 350 ms).
pub fn settle() {
    CLOCK_MS.fetch_add(1000, Ordering::Relaxed);
}

pub fn renderer(w: u32, h: u32) -> Renderer {
    Renderer::new(w, h, now).unwrap()
}

pub fn write_png(name: &str, buf: &[u32], w: u32, h: u32) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../snapshots");
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join(name)).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = buf
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
}

/// Renders a settled frame and checks it looks like a real screen (many colours, opaque).
pub fn frame(r: &mut Renderer, buf: &mut [u32], w: u32, what: &str) {
    settle();
    r.render(buf, w as usize);
    settle();
    r.render(buf, w as usize);
    let colours: BTreeSet<u32> = buf.iter().copied().collect();
    assert!(
        colours.len() > 100,
        "{what}: only {} distinct colours",
        colours.len()
    );
    assert!(
        buf.iter().all(|p| p >> 24 == 0xFF),
        "{what}: alpha byte must be 0xFF"
    );
}

/// Shows `screen`/`view` and writes `<screen>[-<view>]-<size>.png`.
/// `buf` is the renderer's framebuffer: it is reused across frames (only damaged regions are redrawn).
pub fn snapshot(
    r: &mut Renderer,
    buf: &mut [u32],
    screen: &str,
    view: &str,
    size: &str,
    w: u32,
    h: u32,
) {
    r.show(screen, view);
    let name = if view.is_empty() {
        format!("{screen}-{size}")
    } else {
        format!("{screen}-{view}-{size}")
    };
    frame(r, buf, w, &name);
    write_png(&format!("{name}.png"), buf, w, h);
}

pub fn tap(r: &mut Renderer, x: i32, y: i32) {
    for phase in [TouchPhase::Down, TouchPhase::Up] {
        r.dispatch(InputEvent::Touch { id: 0, x, y, phase });
    }
}

/// 1 dp at this window size, as ui/shell.slint computes it.
pub fn dp(w: u32, h: u32) -> f32 {
    w.min(h) as f32 / 360.0
}

/// Centre of menu tile `i`'s icon; mirrors ui/screens/menu.slint (grid 12 dp below the 25 dp status bar,
/// 135 dp rows, 72 dp icons 12 dp below the cell top).
fn tile_center(i: usize, w: u32, h: u32) -> (i32, i32) {
    let d = dp(w, h);
    let cols = if w > h { 4 } else { 3 };
    let cell_w = w as f32 / cols as f32;
    (
        ((i % cols) as f32 * cell_w + cell_w / 2.0) as i32,
        ((25.0 + 12.0) * d + (i / cols) as f32 * 135.0 * d + (12.0 + 36.0) * d) as i32,
    )
}

/// The menu at this size: writes `menu-<name>.png`; a tap on tile 1 opens Messages (`menu-<name>-tapped.png`).
/// The tap must change a large part of the frame, and the reported damage must cover every changed pixel.
pub fn menu_tap(r: &mut Renderer, buf: &mut [u32], name: &str, w: u32, h: u32) {
    r.show("menu", "");
    frame(r, buf, w, name);
    write_png(&format!("menu-{name}.png"), buf, w, h);

    let before = buf.to_vec();
    let (x, y) = tile_center(1, w, h);
    tap(r, x, y);
    let mut damage = r.render(buf, w as usize);
    for _ in 0..3 {
        settle();
        let d = r.render(buf, w as usize);
        damage.add(d.bounds());
    }
    assert!(!damage.is_empty(), "{name}: tap produced no damage");
    let changed = before
        .iter()
        .zip(buf.iter())
        .filter(|(a, c)| a != c)
        .count();
    assert!(
        changed > (w * h / 10) as usize,
        "{name}: tapping Messages changed only {changed} pixels"
    );
    let b = damage.bounds();
    for (i, (a, c)) in before.iter().zip(buf.iter()).enumerate() {
        if a != c {
            let (px, py) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
            assert!(
                px >= b.x && py >= b.y && px < b.x + b.w as i32 && py < b.y + b.h as i32,
                "{name}: changed pixel ({px},{py}) outside damage {b:?}"
            );
        }
    }
    write_png(&format!("menu-{name}-tapped.png"), buf, w, h);
}

// SPDX-License-Identifier: Apache-2.0
//! Shared snapshot helper: renders the launcher into a buffer, writes PNGs to
//! `snapshots/`, and checks that a tap changes the frame.

use std::collections::BTreeSet;
use std::path::PathBuf;

use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;

fn write_png(name: &str, buf: &[u32], w: u32, h: u32) {
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

/// 1 dp at this window size, as ui/launcher.slint computes it.
fn dp(w: u32, h: u32) -> f32 {
    w.min(h) as f32 / 360.0
}

/// Top-left of tile `i`'s cell; mirrors ui/launcher.slint (grid below the 25 dp status bar + 12 dp, 135 dp rows).
fn tile_origin(i: usize, w: u32, h: u32) -> (f32, f32) {
    let d = dp(w, h);
    let cols = if w > h { 4 } else { 3 };
    let cell_w = w as f32 / cols as f32;
    (
        (i % cols) as f32 * cell_w,
        (25.0 + 12.0) * d + (i / cols) as f32 * 135.0 * d,
    )
}

/// Centre of tile `i`'s icon (72 dp, 12 dp below the cell top).
fn tile_center(i: usize, w: u32, h: u32) -> (i32, i32) {
    let d = dp(w, h);
    let cols = if w > h { 4 } else { 3 };
    let (x, y) = tile_origin(i, w, h);
    (
        (x + w as f32 / cols as f32 / 2.0) as i32,
        (y + (12.0 + 36.0) * d) as i32,
    )
}

/// Renders one size, writes `launcher-<name>.png` and `launcher-<name>-tapped.png`,
/// and checks the tap on tile 1 (Messages) showed its selection plate and was reported as damage.
pub fn snapshot_and_tap(name: &str, w: u32, h: u32) {
    let mut r = Renderer::new(w, h, || 0).unwrap();
    let mut buf = vec![0u32; (w * h) as usize];
    let damage = r.render(&mut buf, w as usize);
    assert!(!damage.is_empty(), "{name}: first frame has no damage");

    let colours: BTreeSet<u32> = buf.iter().copied().collect();
    assert!(
        colours.len() > 100,
        "{name}: only {} distinct colours",
        colours.len()
    );
    assert!(
        buf.iter().all(|p| p >> 24 == 0xFF),
        "{name}: alpha byte must be 0xFF"
    );
    write_png(&format!("launcher-{name}.png"), &buf, w, h);

    let before = buf.clone();
    let (x, y) = tile_center(1, w, h);
    for phase in [TouchPhase::Down, TouchPhase::Up] {
        r.dispatch(InputEvent::Touch { id: 0, x, y, phase });
    }
    let damage = r.render(&mut buf, w as usize);
    assert!(!damage.is_empty(), "{name}: tap produced no damage");

    // The tapped tile gets its selection plate: a pixel inside the plate, above the icon, lights up.
    let (cx, _) = tile_center(1, w, h);
    let (_, top) = tile_origin(1, w, h);
    let plate = (top + 8.0 * dp(w, h)) as usize * w as usize + cx as usize;
    assert_ne!(
        before[plate], buf[plate],
        "{name}: selection plate did not appear"
    );

    // The dirty region must cover every changed pixel.
    let b = damage.bounds();
    for (i, (a, c)) in before.iter().zip(&buf).enumerate() {
        if a != c {
            let (px, py) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
            assert!(
                px >= b.x && py >= b.y && px < b.x + b.w as i32 && py < b.y + b.h as i32,
                "{name}: changed pixel ({px},{py}) outside damage {b:?}"
            );
        }
    }
    write_png(&format!("launcher-{name}-tapped.png"), &buf, w, h);
}

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

/// Centre of tile `i`'s icon; mirrors the layout formulas in ui/launcher.slint.
fn tile_center(i: usize, w: u32, h: u32) -> (i32, i32) {
    let (w, h) = (w as f32, h as f32);
    let unit = w.min(h) / 40.0;
    let cols = if w > h { 4 } else { 3 };
    let rows = 8usize.div_ceil(cols) as f32;
    let avail_y = 2.4 * unit + 4.0 * unit + unit;
    let avail_h = h - avail_y - unit;
    let cell_w = w / cols as f32;
    let cell_h = (cell_w * 1.25).min(avail_h / rows);
    let grid_y = avail_y + (avail_h - rows * cell_h) / 2.0;
    let icon = cell_w.min(cell_h) * 0.55;
    let x = (i % cols) as f32 * cell_w + cell_w / 2.0;
    let y = grid_y + (i / cols) as f32 * cell_h + unit + icon / 2.0;
    (x as i32, y as i32)
}

/// Renders one size, writes `launcher-<name>.png` and `launcher-<name>-tapped.png`,
/// and checks the tap on tile 1 (Messages) changed the banner and was reported as damage.
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

    // Banner background (left part, clear of the text) takes the app colour.
    let unit = w.min(h) as f32 / 40.0;
    let banner = (4.9 * unit) as usize * w as usize + w as usize / 10;
    assert_ne!(before[banner], buf[banner], "{name}: banner did not change");

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

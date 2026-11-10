// SPDX-License-Identifier: GPL-3.0-only
//! The full-screen tests a device's view opens: the touch grid and the display colours. Back ends them.

use super::{Ctx, rows};
use crate::DevState;
use crate::state::{Model, View};
use alloc::format;
use alloc::rc::Rc;
use e32::dprintln;
use slint::Color;

pub const COLS: usize = 6;
pub const ROWS: usize = 12;
pub const CELLS: usize = COLS * ROWS;
/// The display test's colours, in order; after the last the test closes.
const FILLS: [(u32, &str); 6] = [
    (0xff0000, "Red"),
    (0x00ff00, "Green"),
    (0x0000ff, "Blue"),
    (0xffffff, "White"),
    (0x000000, "Black"),
    (0x808080, "Grey"),
];

pub fn bind(ctx: &Rc<Ctx>, s: &DevState<'_>) {
    let c = ctx.clone();
    s.on_touched(move |x, y, w, h| {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        // fractions of the area, so the grid needs no pixel sizes
        let col = ((x / w * COLS as f32) as usize).min(COLS - 1);
        let row = ((y / h * ROWS as f32) as usize).min(ROWS - 1);
        c.update(|m| {
            if let Some(cell) = m.touch.get_mut(row * COLS + col)
                && !*cell
            {
                *cell = true;
                let lit = m.touch.iter().filter(|c| **c).count();
                dprintln!(
                    "devtools: touch test cell {},{} ({} of {})",
                    col,
                    row,
                    lit,
                    CELLS
                );
            }
            m.touch_last = Some((x as i32, y as i32));
        });
    });
    let c = ctx.clone();
    s.on_fill_next(move || {
        let done = {
            let mut m = c.model.borrow_mut();
            m.fill += 1;
            m.fill >= FILLS.len()
        };
        if done {
            dprintln!("devtools: display test done");
            c.open(View::Device);
        } else {
            c.show();
        }
    });
}

pub fn show_touch(s: &DevState<'_>, m: &Model) {
    s.set_touch_cols(COLS as i32);
    s.set_touch_rows(ROWS as i32);
    s.set_touch_cells(rows(m.touch.iter().copied()));
    let lit = m.touch.iter().filter(|c| **c).count();
    let at = m
        .touch_last
        .map_or(alloc::string::String::from("touch everywhere"), |(x, y)| {
            format!("{x}, {y}")
        });
    s.set_touch_text(format!("Touch test: {lit} of {CELLS} cells · {at} · Back to finish").into());
}

pub fn show_display(s: &DevState<'_>, m: &Model) {
    let (rgb, name) = FILLS[m.fill.min(FILLS.len() - 1)];
    s.set_fill(Color::from_rgb_u8(
        (rgb >> 16) as u8,
        (rgb >> 8) as u8,
        rgb as u8,
    ));
    s.set_fill_name(format!("{name} ({} of {})", m.fill + 1, FILLS.len()).into());
}

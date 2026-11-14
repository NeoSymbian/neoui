// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::{InputEvent, TouchPhase};

/// smooth-ui S2d: a list dragged on the Pixel's screen is drawn by shifting the pixels it showed and letting Slint
/// draw only the uncovered rows; the frames must match what a full redraw draws.
#[test]
fn a_scrolled_list_shifts_and_matches_a_full_redraw() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    r.set_server_transitions(true);
    let names: Vec<String> = (0..40).map(|i| format!("Contact {i:02}")).collect();
    let rows: Vec<neoui_render::PersonItem> = names
        .iter()
        .enumerate()
        .map(|(i, n)| neoui_render::PersonItem {
            id: i as u32 + 1,
            name: n,
            detail: "+37491000000",
            favourite: false,
        })
        .collect();
    r.set_people(&rows, rows.len(), false);
    r.show("contacts", "");
    common::frame(&mut r, &mut buf, w, "contacts");

    let x = 540;
    let touch = |r: &mut neoui_render::Renderer, y, phase| {
        r.dispatch(InputEvent::Touch { id: 0, x, y, phase })
    };
    touch(&mut r, 1700, TouchPhase::Down);
    let mut shifted = 0;
    for i in 1..=12 {
        common::advance(17);
        touch(&mut r, 1700 - i * 40, TouchPhase::Move);
        r.render(&mut buf, w as usize);
        shifted += r.take_shifts();
    }
    assert!(
        shifted >= 8,
        "the drag went through the shift path ({shifted} of 12 frames)"
    );
    let by_shift = buf.clone();

    // the finger rests: nothing scrolls, so the renderer redraws everything once (into a cleared buffer, so a missing
    // redraw can't pass)
    buf.fill(0);
    common::advance(17);
    touch(&mut r, 1700 - 12 * 40, TouchPhase::Move);
    r.render(&mut buf, w as usize);
    assert_eq!(r.take_shifts(), 0);
    let wrong = (0..buf.len()).filter(|&i| buf[i] != by_shift[i]).count();
    assert_eq!(wrong, 0, "{wrong} px differ from a full redraw");
    touch(&mut r, 1220, TouchPhase::Up);
}

// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_render::{Editor, Renderer};

const W: u32 = 539;
const H: u32 = 1079;

/// What the contact editor's fields take from the system keyboard reaches `Renderer::editor` (telephony C8).
#[test]
fn typed_names_reach_the_editor() {
    let mut r = common::renderer(W, H);
    let mut buf = vec![0u32; (W * H) as usize];
    let d = common::dp(W, H);
    let tap = |r: &mut Renderer, buf: &mut Vec<u32>, (x, y): (i32, i32)| {
        common::tap(r, x, y);
        common::settle();
        r.render(buf, W as usize);
    };
    r.set_editor(&Editor {
        numbers: vec![("Mobile".into(), "077453993".into())],
        ..Default::default()
    });
    r.show("contacts", "edit");
    common::frame(&mut r, &mut buf, W, "editor");
    let body = r.body();
    // the first field: 16 dp margin, 48 dp tall (Theme.field-h)
    tap(
        &mut r,
        &mut buf,
        ((W / 2) as i32, body.y + (16.0 * d + 20.0 * d) as i32),
    );
    assert!(r.keyboard_h() > 0, "the field brings the keyboard up");
    // 'a' on the home row (as tests/fields.rs)
    let bottom = (body.y + body.h as i32) as f32;
    let a = ((0.5 + 0.5) * W as f32 / 10.0) as i32;
    tap(
        &mut r,
        &mut buf,
        (a, (bottom - (27.0 + 2.0 * 48.0) * d) as i32),
    );
    common::frame(&mut r, &mut buf, W, "typed");
    // Save leaves the view before the UI server reads the fields
    r.show("contacts", "");
    common::frame(&mut r, &mut buf, W, "left");
    let e = r.editor();
    assert_eq!(e.numbers, vec![("Mobile".into(), "077453993".into())]);
    assert!(!e.first.is_empty(), "typed first name lost: {e:?}");
}

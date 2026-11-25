// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
mod common;

#[test]
fn syborg() {
    let (w, h) = (768, 640);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    common::snapshot(&mut r, &mut buf, "home", "", "syborg", w, h);
    common::menu_tap(&mut r, &mut buf, "syborg", w, h);
}

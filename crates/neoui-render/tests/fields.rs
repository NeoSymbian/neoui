// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

const W: u32 = 539;
const H: u32 = 1079;

/// The shell's own text field (the SIM PIN prompt's; Settings' Wi-Fi password moved into the Settings app) brings up
/// the system keyboard; the prompt going away hides it.
#[test]
fn shell_fields_use_the_system_keyboard() {
    let mut r = common::renderer(W, H);
    let mut buf = vec![0u32; (W * H) as usize];
    common::frame(&mut r, &mut buf, W, "home");
    assert_eq!(r.keyboard_h(), 0);
    r.set_sim(true, false, Some(3));
    common::frame(&mut r, &mut buf, W, "sim prompt");
    assert!(r.keyboard_h() > 0, "the PIN field brings the number pad up");
    assert!(r.take_typed().is_empty(), "nothing went to an app");
    r.set_sim(false, false, None);
    common::frame(&mut r, &mut buf, W, "sim unlocked");
    assert_eq!(
        r.keyboard_h(),
        0,
        "the prompt going away hides the keyboard"
    );
}

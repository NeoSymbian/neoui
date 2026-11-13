// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::settings::{Value, registry};
use neoui_render::Renderer;

/// The shell's view of the system settings (docs/neoui-shell.md, Part 1): every shown setting is a registered key
/// whose UI value has the registry's type and default; values from the store show in the UI; edits in the UI (the
/// shade's toggles) come back as `(key, value)` for the store.
#[test]
fn settings_binding() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    common::frame(&mut r, &mut buf, w, "home");

    for key in Renderer::shown_settings() {
        let spec = registry::lookup(key).unwrap_or_else(|| panic!("{key} is not registered"));
        let shown = r.setting(key).expect("bound");
        assert_eq!(shown.ty(), spec.ty(), "{key}: UI type");
        assert!(
            spec.default.matches(&shown),
            "{key}: UI default {shown} differs from the registry"
        );
    }
    assert_eq!(r.take_setting_edits(), [], "nothing edited yet");

    // the store's values show in the UI; wrong types and unknown keys are refused
    assert!(r.apply_setting("radio.wifi", &Value::Bool(false)));
    assert!(r.apply_setting("display.brightness", &Value::Int(42)));
    assert!(!r.apply_setting("radio.wifi", &Value::Int(1)));
    assert!(!r.apply_setting("radio.nothing", &Value::Bool(true)));
    common::frame(&mut r, &mut buf, w, "home");
    assert_eq!(r.setting("radio.wifi"), Some(Value::Bool(false)));
    assert_eq!(r.setting("display.brightness"), Some(Value::Int(42)));
    let _ = r.take_setting_edits(); // the applied values echo back; storing them again changes nothing

    // the shade's Wi-Fi tile: the first of four across the panel, below the status bar
    let d = common::dp(w, h);
    common::tap(&mut r, w as i32 / 2, (10.0 * d) as i32); // the status bar opens the shade
    common::frame(&mut r, &mut buf, w, "shade");
    common::write_png("shade-settings-539x1079.png", &buf, w, h);
    let (margin, gap) = (12.0 * d, 8.0 * d);
    let tile_w = (w as f32 - 2.0 * margin - 3.0 * gap) / 4.0;
    let header = 47.0 * d; // the shade's time-and-date row and the gap under it
    let (x, y) = (margin + tile_w / 2.0, 25.0 * d + margin + header + 37.0 * d);
    common::tap(&mut r, x as i32, y as i32);
    common::frame(&mut r, &mut buf, w, "shade");
    assert_eq!(r.take_setting_edits(), [("radio.wifi", Value::Bool(true))]);
    assert_eq!(r.setting("radio.wifi"), Some(Value::Bool(true)));
}

// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::ipc;
use neoui_proto::settings_app::asks;
use neoui_render::{Page, Query};

/// set2-reset-query: Settings → Reset asks first, in the shell's dialog (QUERY), with Reset in red.
#[test]
fn the_reset_question() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    r.show("settings", "");
    r.set_app_front(true);
    r.set_app_page(&Page {
        title: "Reset".into(),
        can_back: true,
        ..Page::default()
    });
    let text = asks::reset(ipc::RESET_SETTINGS);
    assert!(
        text.starts_with("Reset settings? "),
        "the board's question first"
    );
    assert!(asks::reset(ipc::RESET_NETWORK).starts_with("Reset network settings? "));
    r.set_app_query(Some(&Query {
        text: text.into(),
        buttons: vec!["Reset".into(), "Cancel".into()],
        destructive: true,
    }));
    assert!(r.overlay_open());
    common::frame(&mut r, &mut buf, w, "reset query");
    // the destructive button is the theme's red
    let red = buf
        .iter()
        .filter(|&&p| {
            let (r8, g8) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF);
            r8 > 0xB0 && g8 < 0x70
        })
        .count();
    assert!(red > 500, "{red} red px");
    common::write_png("set2-reset-query.png", &buf, w, h);
}

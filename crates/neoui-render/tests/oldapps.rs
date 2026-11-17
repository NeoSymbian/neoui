// SPDX-License-Identifier: Apache-2.0
// Own test process: Slint's platform is a singleton.
#[allow(dead_code)]
mod common;

use neoui_render::LauncherEntry;

#[test]
fn old_apps_grid_and_empty_state() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let d = common::dp(w, h);
    let mut icon = vec![32, 0, 32, 0];
    // An original sample note icon; include transparent and translucent pixels to exercise straight alpha.
    for y in 0..32 {
        for x in 0..32 {
            let argb: u32 = if !(2..30).contains(&x) || !(2..30).contains(&y) {
                0x00ff_0000
            } else if x == 2 || x == 29 || y == 2 || y == 29 {
                0x80f0_9020
            } else if (8..24).contains(&x) && [10, 11, 16, 17, 22, 23].contains(&y) {
                0xffff_ffff
            } else {
                0xfff0_9020
            };
            icon.extend_from_slice(&argb.to_le_bytes());
        }
    }
    r.set_old_apps(&[
        LauncherEntry {
            id: 0xe000_1234,
            caption: "Notes".into(),
            icon,
        },
        LauncherEntry {
            id: 0x1020_5678,
            caption: "Calculator".into(),
            icon: vec![],
        },
    ]);
    r.show("legacy", "");
    common::frame(&mut r, &mut buf, w, "old apps");
    common::write_png("oldapps-pixel.png", &buf, w, h);
    let body = r.body();
    let header_end = body.y as usize * w as usize;
    let toolbar_start = (body.y as usize + body.h as usize) * w as usize;
    let header = buf[..header_end].to_vec();
    let toolbar = buf[toolbar_start..].to_vec();
    assert!(
        buf.contains(&0xfff0_9020),
        "ARGB must keep the red and blue channels in order"
    );
    let y = ((85.0 + 48.0) * d) as i32; // below the 48 dp header
    common::tap(&mut r, (w / 6) as i32, y);
    assert_eq!(r.take_entries(), [0xe000_1234]);
    assert_eq!(
        r.screen(),
        "legacy",
        "a tap waits for the app's FRONT request"
    );
    common::tap(&mut r, (w / 2) as i32, y);
    assert_eq!(r.take_entries(), [0x1020_5678]);

    r.set_app_front(true);
    common::frame(&mut r, &mut buf, w, "legacy surface");
    common::tap(&mut r, (w / 6) as i32, y);
    assert!(
        r.take_entries().is_empty(),
        "a hidden grid must not receive taps"
    );
    r.set_app_front(false);
    r.set_old_apps(&[]);
    common::frame(&mut r, &mut buf, w, "no old apps");
    assert_eq!(
        buf[..header_end],
        header,
        "the empty grid keeps the Old apps title and status bar"
    );
    assert_eq!(
        buf[toolbar_start..],
        toolbar,
        "the empty grid keeps the shell strip"
    );
    common::write_png("oldapps-empty-pixel.png", &buf, w, h);
    common::tap(&mut r, (w / 6) as i32, y);
    assert!(r.take_entries().is_empty());
    let at = common::keys(&mut r, w, h);
    common::tap(&mut r, at.back, at.y);
    assert_ne!(
        r.screen(),
        "legacy",
        "Back leaves the grid like other shell screens"
    );
}

// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::settings::Value;
use neoui_proto::settings_app::wallpaper;

/// (D7) Home draws the installed picture while `display.wallpaper` is `picture`, covering the screen; a colour's
/// name, or no picture (the file was never installed), draws the colour.
#[test]
fn home_draws_the_installed_picture() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0u32; (w * h) as usize];
    let picked = |buf: &[u32]| buf.iter().filter(|&&p| p & 0xFF_FFFF == 0xC0_3070).count();
    // a raster as Settings sends it: one colour above, a darker one below (so a cover fit is visible)
    let (pw, ph) = (u32::from(wallpaper::W), u32::from(wallpaper::H));
    let mut px = Vec::with_capacity(wallpaper::BYTES);
    for y in 0..ph {
        let c: [u8; 3] = if y < ph / 2 {
            [0xC0, 0x30, 0x70]
        } else {
            [0x20, 0x10, 0x40]
        };
        for _ in 0..pw {
            px.extend_from_slice(&c);
        }
    }
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");
    assert_eq!(picked(&buf), 0);

    // installed, but the key still names a colour: the colour
    r.set_wallpaper_picture(Some((&px, pw, ph)));
    assert!(r.has_wallpaper_picture());
    common::frame(&mut r, &mut buf, w, "home");
    assert_eq!(picked(&buf), 0, "the key decides");

    // display.wallpaper = picture: most of the top half is the picture's colour
    assert!(r.apply_setting("display.wallpaper", &Value::Str(wallpaper::KEY.into())));
    common::frame(&mut r, &mut buf, w, "home picture");
    let shown = picked(&buf);
    assert!(shown > (w * h / 4) as usize, "{shown} px of the picture");
    common::write_png("home-wallpaper-picture-539x1079.png", &buf, w, h);

    // a wrong-length raster takes it away: the colour behind (Night for "picture") again
    r.set_wallpaper_picture(Some((&px[..10], pw, ph)));
    assert!(!r.has_wallpaper_picture());
    common::frame(&mut r, &mut buf, w, "home");
    assert_eq!(picked(&buf), 0);
}

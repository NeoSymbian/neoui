// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

use neoui_proto::battery::Snapshot;
use neoui_proto::power_menu::act;
use neoui_proto::settings::Value;
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;

fn touch(r: &mut Renderer, x: i32, y: i32, phase: TouchPhase) {
    r.dispatch(InputEvent::Touch { id: 0, x, y, phase });
}

/// Renders `ms` of a hold in frames of 50 ms (Slint's timers and the ring advance in `render`).
fn hold_for(r: &mut Renderer, buf: &mut [u32], w: u32, ms: u64) {
    for _ in 0..ms / 50 {
        common::advance(50);
        r.render(buf, w as usize);
    }
}

/// Where the sheet's controls are, in px (ui/components/power-sheet.slint, from the status bar's bottom).
struct At {
    d: f32,
    w: u32,
}

impl At {
    fn y(&self, dp: f32) -> i32 {
        ((25.0 + dp) * self.d) as i32
    }
    fn x(&self, dp: f32) -> i32 {
        (self.w as f32 / 2.0 + dp * self.d) as i32
    }
    /// Power off (-74 dp from the centre) or Restart (+74), the big buttons' centres
    fn big(&self, dx: f32, dev: bool) -> (i32, i32) {
        (self.x(dx), self.y(if dev { 119.0 } else { 129.0 } + 48.0))
    }
}

/// Power menu v2 (POWER; boards sh-power-menu-v2, -hold, -dev): the sheet over the darkened home screen, Vibrate on,
/// then Power off held for 68 % of its ring ("Release to cancel", "Keep holding…"), then with Developer options'
/// USB debugging on (Fastboot). Snapshots `sh-power-menu-v2*.png` at the Pixel's size. The screenshot writer gets the
/// home screen too: it decodes, and its size is printed.
#[test]
fn the_sheet_its_hold_and_the_developer_row() {
    let (w, h) = (1080, 2160);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    r.set_clock("10:24");
    r.set_network(3, "Ucom AM", "3G");
    r.set_network_state("", false);
    r.set_battery(&Snapshot {
        valid: 63,
        present: 1,
        percent: 72,
        voltage_uv: 3_900_000,
        usb_present: 0,
        usb_online: 0,
        charge: 1,
    });
    for (k, v) in [
        ("sound.silent", true),
        ("sound.vibrate", true),
        ("radio.airplane", false),
    ] {
        assert!(r.apply_setting(k, &Value::Bool(v)));
    }
    common::frame(&mut r, &mut buf, w, "home");
    let home = buf.clone();
    let png = neoui_proto::png_write::encode(w as usize, h as usize, |y, out| {
        out.copy_from_slice(&home[y * w as usize..][..w as usize]);
    });
    let mut dec = png::Decoder::new(std::io::Cursor::new(&png[..]))
        .read_info()
        .unwrap();
    let mut rgb = vec![0; dec.output_buffer_size().unwrap()];
    dec.next_frame(&mut rgb).unwrap();
    assert!(
        home.iter()
            .zip(rgb.as_chunks::<3>().0)
            .all(|(p, c)| *c == [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
    );
    println!(
        "screenshot of home at {w}x{h}: {} bytes (raw {})",
        png.len(),
        h * (1 + 3 * w)
    );

    r.set_power_backdrop(w as usize, h as usize, |y, out| {
        out.copy_from_slice(&home[y * w as usize..][..w as usize]);
    });
    r.set_power_profiles(&[(1, "Silent"), (0, "General")], 1);
    r.set_power_menu(true);
    common::frame(&mut r, &mut buf, w, "power sheet");
    common::write_png("sh-power-menu-v2.png", &buf, w, h);

    let at = At {
        d: common::dp(w, h),
        w,
    };
    let (x, y) = at.big(-74.0, false);
    touch(&mut r, x, y, TouchPhase::Down);
    r.render(&mut buf, w as usize);
    hold_for(&mut r, &mut buf, w, 800);
    common::advance(16);
    r.render(&mut buf, w as usize);
    r.render(&mut buf, w as usize);
    common::write_png("sh-power-menu-v2-hold.png", &buf, w, h);
    assert!(r.take_power().is_empty(), "not yet");
    touch(&mut r, x, y, TouchPhase::Up);
    assert_eq!(r.take_power(), [act::CANCELLED]);

    assert!(r.apply_setting("developer.usb_debugging", &Value::Bool(true)));
    common::frame(&mut r, &mut buf, w, "power sheet, developer");
    common::write_png("sh-power-menu-v2-dev.png", &buf, w, h);
}

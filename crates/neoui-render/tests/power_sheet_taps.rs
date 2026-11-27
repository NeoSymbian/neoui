// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)]
mod common;

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

/// A press at `(x, y)` held `ms`, then let go; what the menu reported.
fn hold(r: &mut Renderer, buf: &mut [u32], (x, y): (i32, i32), ms: u64) -> Vec<i32> {
    let w = r.body().w;
    touch(r, x, y, TouchPhase::Down);
    r.render(buf, w as usize);
    hold_for(r, buf, w, ms);
    touch(r, x, y, TouchPhase::Up);
    common::settle();
    r.render(buf, w as usize);
    r.take_power()
}

/// A tap at `(x, y)`; what the menu reported.
fn tap(r: &mut Renderer, buf: &mut [u32], (x, y): (i32, i32)) -> Vec<i32> {
    let w = r.body().w;
    common::tap(r, x, y);
    common::settle();
    r.render(buf, w as usize);
    r.take_power()
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
    /// tile `i` (0 Lock now .. 3 the ring tile): 77 dp columns 8 dp apart from 14 dp
    fn tile(&self, i: usize, dev: bool) -> (i32, i32) {
        let top = if dev { 119.0 + 150.0 } else { 129.0 + 160.0 };
        (
            ((14.0 + 38.5 + i as f32 * 85.0) * self.d) as i32,
            self.y(top + 30.0),
        )
    }
}

/// What each control reports, at virt's size (symbian/taps.sh's step uses the same places): the shade's ⏻ asks for
/// the menu; a hold of Restart let go at 0.5 s is cancelled, one of 1.3 s restarts; Power off; the four tiles (the
/// airplane tile writes `radio.airplane` itself); Emergency call; Fastboot only with USB debugging on; a swipe down
/// closes the sheet; `power.style` 1 brings back the short list.
#[test]
fn the_controls_report_their_actions() {
    let (w, h) = (539, 1079);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    assert!(r.apply_setting("sound.silent", &Value::Bool(false)));
    common::frame(&mut r, &mut buf, w, "home");
    r.open_shade();
    common::frame(&mut r, &mut buf, w, "shade");
    common::tap(&mut r, 494, 82);
    assert!(r.take_power_asked(), "the shade's ⏻ at (494, 82)");

    r.set_power_menu(true);
    common::frame(&mut r, &mut buf, w, "sheet");
    let _ = r.take_setting_edits();
    let at = At {
        d: common::dp(w, h),
        w,
    };
    assert_eq!(
        hold(&mut r, &mut buf, at.big(74.0, false), 500),
        [act::CANCELLED]
    );
    assert_eq!(
        hold(&mut r, &mut buf, at.big(74.0, false), 1300),
        [act::RESTART]
    );
    assert_eq!(
        hold(&mut r, &mut buf, at.big(-74.0, false), 1300),
        [act::OFF]
    );

    assert_eq!(tap(&mut r, &mut buf, at.tile(0, false)), [act::LOCK]);
    assert_eq!(tap(&mut r, &mut buf, at.tile(1, false)), [act::SCREENSHOT]);
    assert_eq!(tap(&mut r, &mut buf, at.tile(2, false)), [] as [i32; 0]);
    assert_eq!(
        r.take_setting_edits(),
        vec![("radio.airplane", Value::Bool(true))]
    );
    assert_eq!(tap(&mut r, &mut buf, at.tile(3, false)), [act::RING]);
    // Emergency call: the pill 105 dp above the bottom, 48 dp tall
    let sos = (at.x(0.0), ((h as f32 / at.d - 105.0 + 24.0) * at.d) as i32);
    assert_eq!(tap(&mut r, &mut buf, sos), [act::EMERGENCY]);
    // Fastboot, the Developer row's button (tiles + 120 + 22 dp), only with USB debugging
    let fastboot = (at.x(0.0), at.y(119.0 + 150.0 + 120.0 + 22.0 + 24.0));
    assert_eq!(tap(&mut r, &mut buf, fastboot), [] as [i32; 0]);
    assert!(r.apply_setting("developer.usb_debugging", &Value::Bool(true)));
    assert_eq!(tap(&mut r, &mut buf, fastboot), [act::FASTBOOT]);
    assert_eq!(tap(&mut r, &mut buf, at.tile(0, true)), [act::LOCK]);
    assert_eq!(
        hold(&mut r, &mut buf, at.big(74.0, true), 1300),
        [act::RESTART]
    );

    // a swipe down from the title closes it; a short one doesn't
    let swipe = |r: &mut Renderer, dy: i32| {
        let (x, y) = (at.x(0.0), at.y(50.0));
        touch(r, x, y, TouchPhase::Down);
        for k in 1..=8 {
            common::advance(17);
            touch(r, x, y + dy * k / 8, TouchPhase::Move);
        }
        touch(r, x, y + dy, TouchPhase::Up);
        r.take_power()
    };
    assert_eq!(swipe(&mut r, (60.0 * at.d) as i32), [] as [i32; 0]);
    assert_eq!(swipe(&mut r, (140.0 * at.d) as i32), [act::DISMISSED]);

    // the short list (power.style 1): its Restart on the centre line
    assert!(r.apply_setting("power.style", &Value::Int(1)));
    common::frame(&mut r, &mut buf, w, "short list");
    assert_eq!(
        tap(&mut r, &mut buf, (w as i32 / 2, h as i32 / 2)),
        [act::RESTART]
    );
    r.set_power_menu(false);
    assert_eq!(tap(&mut r, &mut buf, at.tile(0, false)), [] as [i32; 0]);
}

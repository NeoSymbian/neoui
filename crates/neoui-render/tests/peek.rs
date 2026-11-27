// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::gesture::Gesture;
use neoui_proto::settings::Value;
use neoui_render::Renderer;
use neoui_render::peek::{Kind, Neighbour, Peek};

const W: u32 = 539;
const H: u32 = 1079;

/// A peek's frame as the UI server puts it on the screen; written below the status bar (the boards' crop) as
/// `snapshots/<name>.png`.
fn shot(
    r: &Renderer,
    p: &Peek,
    now: u64,
    cur: &[u32],
    target: &[u32],
    n: Neighbour<'_>,
    name: &str,
) -> Vec<u32> {
    let w = W as usize;
    let mut out = vec![0u32; w * H as usize];
    let (mut a, mut b) = (vec![0u32; w], vec![0u32; w]);
    p.compose(now, cur, target, n, (&mut a, &mut b), |y, row| {
        out[y * w..][..w].copy_from_slice(row)
    });
    let top = r.body().y as usize;
    common::write_png(&format!("{name}.png"), &out[top * w..], W, H - top as u32);
    out
}

fn rows(r: &Renderer) -> (usize, usize) {
    let b = r.app_body();
    (b.y as usize, (b.y + b.h as i32) as usize)
}

/// G2: each map's peeks over the real shell (the Music demo stands in for an app), with the boards' snapshots.
#[test]
fn peeks_follow_the_finger_over_their_targets() {
    let (w, h) = (W as usize, H as usize);
    let mut r = common::renderer(W, H);
    r.set_server_transitions(true);
    let mut buf = vec![0u32; w * h];
    assert!(r.apply_setting("nav.mode", &Value::Int(1)));
    let d = common::dp(W, H);
    let dp256 = (w.min(h) * 256 / 360) as u32;
    r.set_running(&["music", "photos", "camera"]);
    r.show("photos", "");
    common::frame(&mut r, &mut buf, W, "photos");
    let photos = buf.clone();
    r.show("menu", "");
    r.show_from("music", "menu");
    common::frame(&mut r, &mut buf, W, "music");
    let cur = buf.clone();

    // a preview draws the target, and putting it back leaves Back's origin (the Apps menu) as it was
    let saved = r.peek_save();
    r.peek_show(Gesture::Home);
    common::frame(&mut r, &mut buf, W, "home preview");
    assert_eq!(r.screen(), "home");
    let home = buf.clone();
    r.peek_restore(&saved);
    assert_eq!(r.peek_save(), saved);
    r.gesture(Gesture::Back);
    assert_eq!(r.screen(), "menu", "the origin survived the preview");
    r.show_from("music", "menu");
    common::frame(&mut r, &mut buf, W, "music again");

    // Thumb, up and hold (nav-map-pause): a card over home, then Open apps' cards at its sides
    let body = rows(&r);
    let mut p = Peek::start(
        Kind::Card,
        (w, h),
        body,
        (w as i32 / 2, h as i32 - 4),
        -1,
        dp256,
    );
    let finger = h as i32 - (132.0 * d) as i32;
    p.drag(w as i32 / 2, finger);
    shot(&r, &p, 0, &cur, &home, Neighbour::None, "nav-map-thumb");
    p.pause(0, true);
    let f = shot(&r, &p, 1000, &cur, &home, Neighbour::None, "nav-map-pause");
    let mid = (finger as usize - 100) * w;
    assert_ne!(f[mid + w / 2], home[mid + w / 2], "the card");
    assert_eq!(f[mid + 4], 0xff16_181c, "a side card");

    // N9: up from the bottom (Lift) over home; a drag from the right (Side) over Open apps
    r.peek_show(Gesture::OpenApps);
    common::frame(&mut r, &mut buf, W, "open apps preview");
    assert_eq!(r.screen(), "switcher");
    let open_apps = buf.clone();
    r.peek_restore(&saved);
    let mut p = Peek::start(
        Kind::Lift,
        (w, h),
        body,
        (w as i32 / 2, h as i32 - 4),
        -1,
        dp256,
    );
    p.drag(w as i32 / 2, h as i32 - (200.0 * d) as i32);
    let f = shot(&r, &p, 0, &cur, &home, Neighbour::None, "nav-map-n9");
    assert_eq!(
        f[(h - (100.0 * d) as usize) * w + 9],
        home[(h - (100.0 * d) as usize) * w + 9]
    );
    let mut p = Peek::start(
        Kind::Side,
        (w, h),
        body,
        (w as i32 - 4, h as i32 / 2),
        -1,
        dp256,
    );
    p.drag(w as i32 - (150.0 * d) as i32, h as i32 / 2);
    let f = shot(
        &r,
        &p,
        0,
        &cur,
        &open_apps,
        Neighbour::None,
        "nav-map-n9-flick",
    );
    let y = h / 2 * w;
    assert_eq!(
        f[y + w - 2],
        open_apps[y + w - 2],
        "Open apps behind, in place"
    );
    assert_eq!(
        f[y],
        cur[y + (150.0 * d) as usize - 4],
        "the page, moved left"
    );

    // Back from the right edge (nav-map-back-arrow): the page slides a quarter of the way, the bubble at 72 dp
    let mut p = Peek::start(Kind::Back, (w, h), body, (w as i32 - 4, 0), -1, dp256);
    p.drag(w as i32 - 4 - (72.0 * d) as i32, 0);
    let mut f = shot(&r, &p, 0, &cur, &cur, Neighbour::None, "nav-map-back-arrow");
    let bd = (36.0 * d) as i32;
    let (bx, by) = (w as i32 - (48.0 * d) as i32 - bd, h as i32 / 2);
    for y in 0..bd {
        for x in 0..bd {
            if let Some(px) = neoui_render::back_bubble_pixel(x, y, bd, true, true) {
                f[(by + y) as usize * w + (bx + x) as usize] = px;
            }
        }
    }
    let top = r.body().y as usize;
    common::write_png("nav-map-back-arrow.png", &f[top * w..], W, H - top as u32);
    assert_eq!(
        f[y + w - 1],
        neoui_render::peek::BG,
        "nothing behind the page yet"
    );

    // along the band (nav-map-along): rightwards, the previous app's last frame comes in from the left
    assert_eq!(r.app_for(Gesture::PrevApp).as_deref(), Some("photos"));
    assert_eq!(r.app_for(Gesture::NextApp), None, "Music is the newest");
    let mut p = Peek::start(Kind::Along, (w, h), body, (60, h as i32 - 4), 1, dp256);
    p.drag(60 + w as i32 * 3 / 5, h as i32 - 4);
    let kept = &photos[body.0 * w..body.1 * w];
    let n = Neighbour::Frame {
        px: kept,
        w,
        y0: body.0,
    };
    let f = shot(&r, &p, 0, &cur, &cur, n, "nav-map-along");
    let at = (body.0 + 300) * w;
    assert_eq!(f[at + 10], photos[at + 10 + w * 2 / 5 + 1]);
    assert!(
        p.release(0, Some(Gesture::PrevApp)),
        "it settles onto Photos"
    );
    assert!(!p.step(1000));
    assert_eq!(p.committed(), Some(Gesture::PrevApp));
    lanes_home_returns_to_the_last_lane(r);
}

/// G5: Home comes back to the lane used last (the N9's behaviour); a preview of Open apps leaves it alone.
fn lanes_home_returns_to_the_last_lane(mut r: Renderer) {
    let mut buf = vec![0u32; (W * H) as usize];
    assert!(r.apply_setting("home.style", &Value::Int(1)));
    r.show("home", "");
    common::frame(&mut r, &mut buf, W, "lanes");
    r.set_home_page(0);
    r.show("clock", "");
    common::frame(&mut r, &mut buf, W, "clock");
    let saved = r.peek_save();
    r.peek_show(Gesture::OpenApps);
    assert_eq!((r.screen().as_str(), r.home_page()), ("home", 2));
    r.peek_restore(&saved);
    r.gesture(Gesture::Home);
    common::frame(&mut r, &mut buf, W, "lanes again");
    assert_eq!(r.home_page(), 0, "Events, the lane used last");
}

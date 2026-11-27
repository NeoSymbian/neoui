// SPDX-License-Identifier: GPL-3.0-only
//! (G2) Peek, the UI server's side (docs `neoui-shell/navigation.md` G2; the pixels are `neoui_render::peek`'s): when
//! an edge gesture locks, the screen as it is becomes a layer (the S2b snapshot, `old`) and Slint draws the gesture's
//! target once into the frame; each frame then moves the layer with the finger over it. While it runs, the shell's
//! screen change, fade and app lifecycle are held back (`lib.rs` skips them), so a short swipe puts everything back
//! as it was; one that settles onto the target applies the gesture once, with no fade, and a haptic pulse.
//!
//! Also (G3) the pinch on Open apps: 2 ↔ 3 columns from the first two fingers (phone only: QEMU has one pointer).

use super::{Hint, haptic, snapshot};
use crate::host::Host;
use crate::settings;
use alloc::string::String;
use alloc::vec::Vec;
use e32::display::Display;
use e32::dprintln;
use neoui_proto::gesture::{Edge, Gesture, Map};
use neoui_proto::pinch::Pinch;
use neoui_proto::settings::{Caller, Value, keys};
use neoui_proto::{InputEvent, effects};
use neoui_render::Renderer;
use neoui_render::peek::{Kind, Neighbour, Peek, Shown};

pub struct Peeking {
    p: Peek,
    /// what Slint showed before the preview, and what it shows now
    shown: Shown,
    showing: String,
    /// the gesture whose target Slint draws now (None: Back and Along, which keep the screen)
    preview: Option<Gesture>,
    /// Along: the app that slides in beside the page
    beside: Option<String>,
}

impl Peeking {
    /// The screen Slint is expected to show while this peek runs (something else means it was changed under it).
    pub fn showing(&self) -> &str {
        &self.showing
    }
}

/// A gesture is under way (`Step::Track`, or the Thumb's pause from `Recognizer::tick` with no finger position):
/// start a peek on its first step, then follow the finger. With no peek (on home, without the memory for the
/// snapshot) G1's hint is all there is, as before.
#[allow(clippy::too_many_arguments)]
pub fn track(
    peek: &mut Option<Peeking>,
    ui: &mut Renderer,
    host: &Host,
    (frame, old, (w, h)): (&[u32], &mut Vec<u32>, (usize, usize)),
    (map, along): (Map, Option<i32>),
    (edge, progress): (Edge, u32),
    at: Option<(i32, i32)>,
    dp256: u32,
    now: u64,
) {
    if let Some(p) = peek.as_mut() {
        if let Some((x, y)) = at {
            p.p.drag(x, y);
        }
        p.p.pause(now, progress >= 512);
        let want = p.p.kind.preview(progress);
        if want != p.preview
            && let Some(g) = want
        {
            ui.peek_show(g); // N9: past half the screen the Apps menu, and back to home below it
            p.preview = want;
            p.showing = ui.screen();
        }
        return;
    }
    let Some((x, y)) = at else { return };
    let screen = ui.screen();
    let Some(kind) = Kind::of(map, edge, along.is_some()) else {
        return;
    };
    // home keeps its own bottom (the Apps menu) and right edge; Open apps doesn't peek at itself; over the shade a
    // gesture only closes it, so there is nothing to preview (the hint only)
    if screen == "home" || ui.shade_open() || (kind == Kind::Side && screen == "switcher") {
        return;
    }
    let dir = match (kind, along) {
        (Kind::Along, Some(dx)) => dx.signum(),
        (Kind::Back, _) if edge == Edge::Left => 1,
        _ => -1,
    };
    let beside = match kind {
        Kind::Along if dir > 0 => ui.app_for(Gesture::PrevApp),
        Kind::Along => ui.app_for(Gesture::NextApp),
        _ => None,
    };
    if (kind == Kind::Along && (dir == 0 || beside.is_none()))
        || !(old.len() == w * h || old.try_reserve_exact(w * h).is_ok())
    {
        return; // nothing to slide in, or no memory: the hint only
    }
    old.resize(w * h, 0);
    snapshot(host, frame, old, w);
    let body = ui.app_body();
    let rows = (
        body.y.max(0) as usize,
        (body.y.max(0) as u32 + body.h) as usize,
    );
    let p = Peek::start(kind, (w, h), rows, (x, y), dir, dp256);
    let shown = ui.peek_save();
    let preview = kind.preview(progress);
    if let Some(g) = preview {
        ui.peek_show(g);
    }
    dprintln!("neoui: peek {:?}", kind);
    *peek = Some(Peeking {
        p,
        showing: ui.screen(),
        shown,
        preview,
        beside,
    });
}

/// The finger lifted with gesture `g` (None: short of the commit point). True while the peek settles: the gesture
/// waits for [`finish`]. False when there was no peek or it is over at once (Slint put back, `repaint` set): apply the
/// gesture now, as without a peek.
pub fn release(
    peek: &mut Option<Peeking>,
    ui: &mut Renderer,
    now: u64,
    g: Option<Gesture>,
    repaint: &mut bool,
) -> bool {
    let Some(p) = peek.as_mut() else {
        return false;
    };
    if p.p.release(now, g) {
        match g {
            Some(g) if p.p.committed().is_some() => dprintln!("neoui: peek commit {}", g.name()),
            _ => dprintln!("neoui: peek cancel"),
        }
        return true;
    }
    if let Some(p) = peek.take() {
        ui.peek_restore(&p.shown);
    }
    *repaint = true;
    false
}

/// Something else changed the screen under the peek (a call, an app's OPEN), or a modal came: drop it, putting back
/// only what is still the preview's. The caller repaints.
pub fn abort(peek: &mut Option<Peeking>, ui: &mut Renderer) {
    if let Some(p) = peek.take() {
        if ui.screen() == p.showing {
            ui.peek_restore(&p.shown);
        } else {
            ui.set_app_front(p.shown.front);
        }
        dprintln!("neoui: peek dropped");
    }
}

/// One frame of the peek, straight to the screen (the hint, the Back bubble or the band's bar, over it). False once
/// it settled: call [`finish`].
#[allow(clippy::too_many_arguments)]
pub fn frame(
    p: &mut Peeking,
    now: u64,
    (old, frame): (&[u32], &[u32]),
    host: &Host,
    hint: Option<Hint>,
    (scratch, out): &mut (Vec<u32>, Vec<u32>),
    display: &Display,
) -> bool {
    let live = p.p.step(now);
    let w = out.len();
    let n = neighbour(host, p.beside.as_deref(), w);
    p.p.compose(now, old, frame, n, (scratch, out), |y, row| {
        if let Some(h) = hint {
            overlay(&h, y, row);
        }
        display.blit_to(row, w, (0, 0, w, 1), (0, y));
    });
    live
}

/// The page settled: Slint as it was before the preview, and the gesture to apply now if it settled onto the target
/// (no fade: the slide was the animation).
pub fn finish(p: Peeking, ui: &mut Renderer) -> Option<Gesture> {
    ui.peek_restore(&p.shown);
    p.p.committed()
}

/// A gesture took effect: one short pulse with the profile's vibrate on touch (`sound.touch_vibrate`).
pub fn pulse(host: &Host) {
    if let Some((ms, amp)) = effects::key_pulse(
        host.settings
            .get(keys::SOUND_TOUCH_VIBRATE)
            .unwrap_or(false),
    ) {
        let _ = haptic(ms, amp);
    }
}

/// Along's neighbour as kept when it went to the back: its last full frame, else its Open apps thumbnail.
fn neighbour<'a>(host: &'a Host, key: Option<&str>, w: usize) -> Neighbour<'a> {
    let Some(key) = key else {
        return Neighbour::None;
    };
    match &host.d10.frame {
        Some((k, y0, px)) if *k == key => Neighbour::Frame { px, w, y0: *y0 },
        _ => match host.d10.kept.iter().find(|t| t.0 == key) {
            Some((_, tw, th, px)) => Neighbour::Thumb {
                px,
                w: *tw,
                h: *th,
                y0: host.body.y.max(0) as usize,
            },
            None => Neighbour::None,
        },
    }
}

/// The hint's pixels on screen row `y`.
fn overlay(h: &Hint, y: usize, row: &mut [u32]) {
    let r = h.rect;
    let y = y as i32;
    if y < r.y || y >= r.y + r.h as i32 {
        return;
    }
    let x0 = r.x.max(0) as usize;
    let x1 = (x0 + r.w as usize).min(row.len());
    match h.bubble {
        Some((committed, left)) => {
            for (i, p) in row[x0..x1].iter_mut().enumerate() {
                let bubble = neoui_render::back_bubble_pixel;
                if let Some(px) = bubble(i as i32, y - r.y, r.w as i32, committed, left) {
                    *p = px;
                }
            }
        }
        None => row[x0..x1].fill(h.colour),
    }
}

/// (G3) A pinch on Open apps (the switcher, or Lanes' third lane) switches 2 ↔ 3 columns (`nav.switcher_cols`).
/// True when the touch is the pinch's and goes no further: the second finger, and the first while both are down
/// (Slint loses its press then, so no card opens). None while the first finger is an edge gesture or a peek runs
/// (`busy`): swallowing that finger's lift would leave the peek waiting for it.
pub fn pinch(
    pinch: &mut Pinch,
    ui: &mut Renderer,
    host: &mut Host,
    ev: &InputEvent,
    now: u64,
    busy: bool,
) -> bool {
    let InputEvent::Touch { id, x, y, phase } = *ev else {
        return false;
    };
    let lanes = host.settings.get(keys::HOME_STYLE).unwrap_or(0) == 1;
    let screen = ui.screen();
    let open_apps = screen == "switcher" || (lanes && screen == "home" && ui.home_page() == 2);
    if busy || !open_apps || host.front().is_some() || ui.overlay_open() {
        *pinch = Pinch::default();
        return false;
    }
    let was = pinch.active();
    if let Some(cols) = pinch.feed(id, phase, x, y) {
        dprintln!("neoui: pinch {} columns", cols);
        let _ = settings::set(
            &mut host.settings,
            Caller::Shell,
            keys::NAV_SWITCHER_COLS.name(),
            Value::Int(cols),
            now,
        );
    }
    if pinch.active() && !was {
        ui.cancel_touch();
    }
    id != 0 || was || pinch.active()
}

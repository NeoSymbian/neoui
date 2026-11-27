// SPDX-License-Identifier: Apache-2.0
//! (G2) Peek (docs `neoui-shell/navigation.md` G2 and the canvas's `nav-map-*` boards): once an edge gesture locks,
//! the screen as it was (`cur`, the S2b snapshot) follows the finger over the gesture's target, which Slint draws
//! once into the frame ([`Renderer::peek_show`]). Letting go short settles back; past the commit point it settles
//! onto the target, and the UI server then makes the change with no fade (the slide was the animation).
//!
//! Plain integer pixel code over rows the caller owns: nothing is allocated per frame (smooth-ui's 60 fps budget).

use crate::Renderer;
use crate::layer::{self, Slide, Transition};
use alloc::string::String;
use neoui_proto::gesture::{Edge, Gesture, Map};
use slint::{ComponentHandle, Model};

/// How long a peek settles back, or onto its target (G2: 250 ms).
pub const SETTLE_MS: u32 = 250;
/// How long the side cards take to slide in once the finger holds still (nav-map-pause).
const SIDES_MS: u32 = 200;
/// What shows where the page moved away from and nothing is behind it (Theme.bg).
pub const BG: u32 = 0xff00_0000;
/// Behind the card while it waits for Open apps, and its side cards (nav-map-pause: the board's ground and cards).
const DIM: u32 = 0xff0b_0d10;
const CARD: u32 = 0xff16_181c;

/// The five peeks of the two gesture maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// N9, up from the bottom: the page rises with the finger, home under it (past half the screen the Apps menu).
    Lift,
    /// N9, a drag in from the right: the page goes left with the finger, Open apps behind it (nav-map-n9-flick).
    Side,
    /// Back from a side edge: the page slides a quarter of the finger's way (nav-map-back-arrow; owner O4).
    Back,
    /// Thumb, along the bottom band: the page and the neighbouring app's last frame side by side (nav-map-along).
    Along,
    /// Thumb, up from the bottom: the page shrinks into a card over home; held still, Open apps' cards slide in at
    /// its sides (nav-map-pause).
    Card,
}

impl Kind {
    /// The peek of a gesture that locked on `edge` in `map`; `along`: the Thumb's move along the bottom band.
    pub fn of(map: Map, edge: Edge, along: bool) -> Option<Kind> {
        match (map, edge) {
            (_, Edge::Top) => None, // G4 draws its own drag
            (Map::Thumb, Edge::Bottom) if along => Some(Kind::Along),
            (Map::Thumb, Edge::Bottom) => Some(Kind::Card),
            (Map::N9, Edge::Bottom) => Some(Kind::Lift),
            (Map::N9, Edge::Right) => Some(Kind::Side),
            (_, Edge::Left) | (Map::Thumb, Edge::Right) => Some(Kind::Back),
        }
    }

    /// The gesture whose target Slint draws under the page (`progress` the recognizer's): none for Back and Along,
    /// whose page has nothing of the shell's behind it.
    pub fn preview(self, progress: u32) -> Option<Gesture> {
        match self {
            Kind::Lift if progress >= 512 => Some(Gesture::AppsMenu),
            Kind::Lift | Kind::Card => Some(Gesture::Home),
            Kind::Side => Some(Gesture::OpenApps),
            Kind::Back | Kind::Along => None,
        }
    }
}

/// The app beside the page in an [`Kind::Along`] peek, as the UI server kept it.
#[derive(Clone, Copy, Debug)]
pub enum Neighbour<'a> {
    /// Nothing kept: [`BG`].
    None,
    /// Its last full frame: `rows` rows of `w` pixels from screen row `y0`.
    Frame { px: &'a [u32], w: usize, y0: usize },
    /// Its Open apps thumbnail (`w` × `h`, the top of its body scaled down), scaled back up from screen row `y0`.
    Thumb {
        px: &'a [u32],
        w: usize,
        h: usize,
        y0: usize,
    },
}

impl Neighbour<'_> {
    /// Screen row `y` of the neighbour into `out` (the screen's width).
    fn row(&self, y: usize, out: &mut [u32]) {
        let sw = out.len();
        match *self {
            Neighbour::Frame { px, w, y0 }
                if y >= y0 && w == sw && (y - y0 + 1) * w <= px.len() =>
            {
                out.copy_from_slice(&px[(y - y0) * w..][..w]);
            }
            Neighbour::Thumb { px, w, h, y0 } if y >= y0 && w > 0 => {
                // one thumbnail pixel per sw / w screen pixels each way
                let ty = (y - y0) * w / sw;
                if ty >= h || (ty + 1) * w > px.len() {
                    out.fill(BG);
                    return;
                }
                let src = &px[ty * w..][..w];
                for (x, d) in out.iter_mut().enumerate() {
                    *d = src[x * w / sw];
                }
            }
            _ => out.fill(BG),
        }
    }
}

/// A peek under way: what moves, how far, and where it settles.
#[derive(Clone, Copy, Debug)]
pub struct Peek {
    pub kind: Kind,
    slide: Slide,
    w: usize,
    h: usize,
    /// the rows that move (the app's body); the status bar above and the band below stay
    rows: (usize, usize),
    radius: i32,
    gap: i32,
    /// Card: since when the side cards slide in (true) or back out (false)
    sides: Option<(Transition, bool)>,
    /// the gesture applied once it settled onto the target
    commit: Option<Gesture>,
}

impl Peek {
    /// Start where the gesture locked: the finger at `x, y`; `dir` is Along's (1 rightwards: the previous app comes
    /// in from the left; -1 the next one) and Back's (1 from the left edge); `rows` the app's body; `dp256` the
    /// server's 1/256 px per dp.
    pub fn start(
        kind: Kind,
        (w, h): (usize, usize),
        rows: (usize, usize),
        (x, y): (i32, i32),
        dir: i32,
        dp256: u32,
    ) -> Self {
        let (y0, y1) = (rows.0.min(h), rows.1.min(h).max(rows.0.min(h)));
        let slide = match kind {
            Kind::Lift => Slide::start(-1, (y1 - y0) as i32, y, 0),
            Kind::Card => Slide::start(-1, (h / 2) as i32, y, 0),
            Kind::Side => Slide::start(-1, w as i32, x, 0),
            Kind::Back => Slide::start(dir, (w / 4) as i32, x / 4, 0),
            Kind::Along => Slide::start(dir, w as i32, x, 0),
        };
        let dp = |n: i32| (n as i64 * i64::from(dp256) / 256) as i32;
        Self {
            kind,
            slide,
            w,
            h,
            rows: (y0, y1),
            radius: dp(16),
            gap: dp(20),
            sides: None,
            commit: None,
        }
    }

    /// The finger moved.
    pub fn drag(&mut self, x: i32, y: i32) {
        match self.kind {
            Kind::Lift | Kind::Card => self.slide.drag(y),
            Kind::Side | Kind::Along => self.slide.drag(x),
            Kind::Back => self.slide.drag(x / 4),
        }
    }

    /// (Card) The finger held still for Open apps (`on`), or moved on again: the side cards slide in, or back out.
    pub fn pause(&mut self, now_ms: u64, on: bool) {
        let shown = self.sides.is_some_and(|(_, s)| s);
        if self.kind == Kind::Card && on != shown {
            self.sides = Some((Transition::new(now_ms, SIDES_MS), on));
        }
    }

    /// The finger lifted with gesture `g` (None: short of the commit point). True while the page settles (onto the
    /// target, the gesture then [`Peek::committed`]; or back); false when the peek is over at once and the gesture's
    /// own change shows (Back's page redraws in the app, a flick's last app fades in, Open apps after the pause).
    pub fn release(&mut self, now_ms: u64, g: Option<Gesture>) -> bool {
        let dir = self.slide.dir();
        let onto = match (self.kind, g) {
            (_, None) => false,
            (Kind::Lift, Some(Gesture::Home | Gesture::AppsMenu))
            | (Kind::Side, Some(Gesture::OpenApps))
            | (Kind::Card, Some(Gesture::Home)) => true,
            (Kind::Along, Some(Gesture::PrevApp)) if dir > 0 => true,
            (Kind::Along, Some(Gesture::NextApp)) if dir < 0 => true,
            _ => return false,
        };
        self.commit = g.filter(|_| onto);
        self.sides = None;
        self.slide.release(now_ms, onto, SETTLE_MS);
        true
    }

    /// The gesture to apply now that the page settled onto its target.
    pub fn committed(&self) -> Option<Gesture> {
        self.commit
    }

    /// Move a settling page on: true while the finger is down or the page settles.
    pub fn step(&mut self, now_ms: u64) -> bool {
        self.slide.step(now_ms)
    }

    /// Where the page is now (px from where it was: negative up or left).
    pub fn offset(&self) -> i32 {
        self.slide.offset
    }

    /// Card: its rectangle now (x, top, width, height) and its scale in 1/256.
    fn card(&self) -> (i32, i32, i32, i32, i32) {
        let half = (self.h / 2).max(1) as i32;
        let s = (256 + 256 * self.slide.offset / half).clamp(0, 256);
        let (w, ah) = (self.w as i32, (self.rows.1 - self.rows.0) as i32);
        let (cw, ch) = (w * s / 256, ah * s / 256);
        let bottom = self.rows.1 as i32 + self.slide.offset;
        ((w - cw) / 2, bottom - ch, cw, ch, s)
    }

    /// One frame, row by row: `cur` the page (the screen when the peek began), `target` the frame Slint drew (the
    /// target, or the same screen for Back and Along), `scratch` and `out` screen-wide rows; `emit(y, row)` gets
    /// every screen row in turn, to put on the screen.
    pub fn compose(
        &self,
        now_ms: u64,
        cur: &[u32],
        target: &[u32],
        neighbour: Neighbour<'_>,
        (scratch, out): (&mut [u32], &mut [u32]),
        mut emit: impl FnMut(usize, &mut [u32]),
    ) {
        let (w, off) = (self.w, self.slide.offset);
        let s = off.unsigned_abs() as usize;
        let sides = self.sides.map_or(0, |(t, on)| {
            let e = t.progress(now_ms).unwrap_or(256) as i32;
            if on { e } else { 256 - e }
        });
        let card = self.card();
        if sides > 0 {
            scratch.fill(DIM);
        }
        for y in 0..self.h {
            let (c, t) = (&cur[y * w..][..w], &target[y * w..][..w]);
            if y < self.rows.0 || y >= self.rows.1 {
                out.copy_from_slice(t);
                emit(y, out);
                continue;
            }
            match self.kind {
                Kind::Lift => {
                    let sy = y + s;
                    out.copy_from_slice(if sy < self.rows.1 {
                        &cur[sy * w..][..w]
                    } else {
                        t
                    });
                }
                Kind::Side => {
                    let s = s.min(w);
                    out[..w - s].copy_from_slice(&c[s..]);
                    out[w - s..].copy_from_slice(&t[w - s..]);
                }
                Kind::Back => {
                    let s = s.min(w);
                    if off > 0 {
                        out[..s].fill(BG);
                        out[s..].copy_from_slice(&c[..w - s]);
                    } else {
                        out[..w - s].copy_from_slice(&c[s..]);
                        out[w - s..].fill(BG);
                    }
                }
                Kind::Along => {
                    neighbour.row(y, scratch);
                    layer::slide_row(out, c, scratch, off);
                }
                Kind::Card => {
                    if sides > 0 {
                        layer::blend_row(out, t, scratch, sides as u32);
                    } else {
                        out.copy_from_slice(t);
                    }
                    self.card_row(y as i32, card, sides, cur, out);
                }
            }
            emit(y, out);
        }
    }

    /// Card: row `y` of the side cards (sliding in by `sides`, 0..=256) and the page scaled into the centre card.
    fn card_row(
        &self,
        y: i32,
        (cx, top, cw, ch, s): (i32, i32, i32, i32, i32),
        sides: i32,
        cur: &[u32],
        out: &mut [u32],
    ) {
        if s == 0 || y < top || y >= top + ch {
            return;
        }
        let (w, i) = (self.w as i32, y - top);
        let inset = corner(i, ch, self.radius * s / 256);
        // a card's columns on this row, inside its rounded corners and the screen
        let span = |x0: i32, x1: i32| {
            let a = (x0 + inset).clamp(0, w) as usize;
            (a, ((x1 - inset).clamp(0, w) as usize).max(a))
        };
        if sides > 0 {
            let away = (256 - sides) * w / 512;
            for x0 in [cx - self.gap - cw - away, cx + cw + self.gap + away] {
                let (a, b) = span(x0, x0 + cw);
                out[a..b].fill(CARD);
            }
        }
        // nearest neighbour, in 16.16 steps: no division per pixel
        let sy = self.rows.0 + (i * 256 / s) as usize;
        let src = &cur[sy.min(self.h - 1) * self.w..][..self.w];
        let step = (256u32 << 16) / s as u32;
        let (a, b) = span(cx, cx + cw);
        let mut at = (a as i32 - cx) as u32 * step;
        for p in &mut out[a..b] {
            *p = src[((at >> 16) as usize).min(self.w - 1)];
            at += step;
        }
    }
}

/// How far row `i` of a box `n` rows tall is inset at each side by its rounded corners of radius `r`.
fn corner(i: i32, n: i32, r: i32) -> i32 {
    let r = r.min(n / 2).max(0);
    let d = if i < r {
        r - i
    } else if i >= n - r {
        i - (n - r) + 1
    } else {
        return 0;
    };
    r - ((r * r - d * d).max(0) as u32).isqrt() as i32
}

/// What Slint showed before a peek's preview, to put back when the peek ends ([`Renderer::peek_restore`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub screen: String,
    view: String,
    page: i32,
    pub front: bool,
}

impl Renderer {
    /// The app the last, previous or next app gesture would bring up (the running apps, most recent first).
    pub fn app_for(&self, g: Gesture) -> Option<String> {
        let screen = self.screen();
        let running: alloc::vec::Vec<String> = self
            .ui
            .get_running()
            .iter()
            .map(|k| k.as_str().into())
            .collect();
        let at = running.iter().position(|k| *k == screen);
        match g {
            Gesture::LastApp => running.into_iter().find(|k| *k != screen),
            // along the band: rightwards the older app, leftwards the newer one
            Gesture::PrevApp => running.get(at.map_or(0, |i| i + 1)).cloned(),
            Gesture::NextApp => at
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| running.get(i))
                .cloned(),
            _ => None,
        }
    }

    /// What shows now, for [`Renderer::peek_restore`]; `front` is whether an app is in front.
    pub fn peek_save(&self) -> Shown {
        Shown {
            screen: self.screen(),
            view: self.view(),
            page: self.home_page(),
            front: self.ui.get_app_front(),
        }
    }

    /// Draw gesture `g`'s target as the page's background: home, the Apps menu or Open apps (Lanes: its lane). No
    /// app is told and Back's origin stays: this is a preview, put back by [`Renderer::peek_restore`].
    pub fn peek_show(&mut self, g: Gesture) {
        self.ui.set_app_front(false);
        let lanes = self.ui.global::<crate::SystemSettings>().get_home_style() == 1;
        match g {
            Gesture::OpenApps if lanes => {
                self.show("home", "");
                self.ui.set_home_page(2);
            }
            Gesture::OpenApps => self.show("switcher", ""),
            Gesture::AppsMenu => self.show("menu", ""),
            _ => self.show("home", ""),
        }
    }

    /// Put back what [`Renderer::peek_save`] saw.
    pub fn peek_restore(&mut self, s: &Shown) {
        self.show(&s.screen, &s.view);
        self.ui.set_home_page(s.page);
        self.ui.set_app_front(s.front);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    const W: usize = 8;
    const H: usize = 10;

    /// A peek's frame as rows.
    fn frame(p: &Peek, cur: &[u32], target: &[u32], n: Neighbour<'_>) -> Vec<u32> {
        let mut out = vec![0u32; W * H];
        let (mut a, mut b) = (vec![0u32; W], vec![0u32; W]);
        p.compose(0, cur, target, n, (&mut a, &mut b), |y, row| {
            out[y * W..][..W].copy_from_slice(row)
        });
        out
    }

    /// Each pixel its own value: row * 100 + column (+ `base`).
    fn screen(base: u32) -> Vec<u32> {
        (0..W * H)
            .map(|i| base + (i / W * 100 + i % W) as u32)
            .collect()
    }

    #[test]
    fn kinds_follow_the_map_and_the_edge() {
        assert_eq!(Kind::of(Map::Thumb, Edge::Bottom, false), Some(Kind::Card));
        assert_eq!(Kind::of(Map::Thumb, Edge::Bottom, true), Some(Kind::Along));
        assert_eq!(Kind::of(Map::Thumb, Edge::Right, false), Some(Kind::Back));
        assert_eq!(Kind::of(Map::N9, Edge::Right, false), Some(Kind::Side));
        assert_eq!(Kind::of(Map::N9, Edge::Bottom, false), Some(Kind::Lift));
        assert_eq!(Kind::of(Map::N9, Edge::Left, false), Some(Kind::Back));
        assert_eq!(Kind::of(Map::N9, Edge::Top, false), None);
        assert_eq!(Kind::Lift.preview(300), Some(Gesture::Home));
        assert_eq!(
            Kind::Lift.preview(512),
            Some(Gesture::AppsMenu),
            "N9: past half the screen"
        );
        assert_eq!(Kind::Side.preview(0), Some(Gesture::OpenApps));
        assert_eq!(Kind::Back.preview(256), None);
    }

    #[test]
    fn lift_raises_the_body_over_the_target_and_keeps_the_bars() {
        let (cur, target) = (screen(0), screen(10_000));
        let mut p = Peek::start(Kind::Lift, (W, H), (2, 8), (3, 9), -1, 256);
        p.drag(3, 6);
        assert_eq!(p.offset(), -3);
        let f = frame(&p, &cur, &target, Neighbour::None);
        assert_eq!(f[W], 10_100, "the status bar is the target's");
        assert_eq!(f[2 * W], 500, "body row 2 shows the page's row 5");
        assert_eq!(f[4 * W + 1], 701);
        assert_eq!(f[5 * W], 10_500, "below the page: the target");
        assert_eq!(f[9 * W], 10_900, "the band");
        p.drag(3, -50);
        assert_eq!(p.offset(), -6, "never past the body");
    }

    #[test]
    fn side_reveals_open_apps_behind_and_back_slides_a_quarter() {
        let (cur, target) = (screen(0), screen(10_000));
        let mut p = Peek::start(Kind::Side, (W, H), (1, 9), (7, 4), -1, 256);
        p.drag(4, 4);
        let f = frame(&p, &cur, &target, Neighbour::None);
        assert_eq!(
            &f[2 * W..3 * W],
            &[203, 204, 205, 206, 207, 10_205, 10_206, 10_207]
        );

        // from the left edge the page moves right, a quarter of the finger's way, with Theme.bg behind
        let mut p = Peek::start(Kind::Back, (W, H), (1, 9), (0, 4), 1, 256);
        p.drag(8, 4);
        assert_eq!(p.offset(), 2);
        let f = frame(&p, &cur, &cur, Neighbour::None);
        assert_eq!(&f[3 * W..4 * W], &[BG, BG, 300, 301, 302, 303, 304, 305]);
        // from the right edge, leftwards
        let mut p = Peek::start(Kind::Back, (W, H), (1, 9), (7, 4), -1, 256);
        p.drag(3, 4);
        assert_eq!(p.offset(), -1);
        // Back ends at once: the app draws the page behind
        assert!(!p.release(0, Some(Gesture::Back)));
        assert!(p.release(0, None), "short: it settles back");
        assert!(!p.step(SETTLE_MS as u64));
        assert_eq!(p.offset(), 0);
    }

    #[test]
    fn along_puts_the_neighbours_last_frame_beside_the_page() {
        let cur = screen(0);
        let kept: Vec<u32> = (0..W * 8).map(|i| 50_000 + i as u32).collect();
        // rightwards: the previous app comes in from the left
        let mut p = Peek::start(Kind::Along, (W, H), (1, 9), (2, 9), 1, 256);
        p.drag(5, 9);
        let n = Neighbour::Frame {
            px: &kept,
            w: W,
            y0: 1,
        };
        let f = frame(&p, &cur, &cur, n);
        assert_eq!(
            &f[W..2 * W],
            &[50_005, 50_006, 50_007, 100, 101, 102, 103, 104]
        );
        assert_eq!(f[0], 0, "the status bar stays");
        // its thumbnail, a quarter wide, scaled back up
        let thumb: Vec<u32> = (0..2).map(|i| 70_000 + i as u32).collect();
        let n = Neighbour::Thumb {
            px: &thumb,
            w: 2,
            h: 1,
            y0: 1,
        };
        let f = frame(&p, &cur, &cur, n);
        assert_eq!(
            &f[W..W + 3],
            &[70_001, 70_001, 70_001],
            "the thumbnail's top right"
        );
        assert_eq!(f[8 * W], BG, "below the thumbnail: Theme.bg");
        // it settles onto the neighbour only in its own direction
        assert!(!p.clone().release(0, Some(Gesture::NextApp)));
        assert!(p.release(0, Some(Gesture::PrevApp)));
        assert!(!p.step(1000));
        assert_eq!(
            (p.offset(), p.committed()),
            (W as i32, Some(Gesture::PrevApp))
        );
    }

    #[test]
    fn card_shrinks_with_the_finger_and_the_side_cards_come_on_a_pause() {
        let (w, h) = (200, 400);
        let cur: Vec<u32> = (0..w * h).map(|i| 0xff00_0000 | (i / w) as u32).collect();
        let target = vec![0xff11_1111u32; w * h];
        let mut p = Peek::start(Kind::Card, (w, h), (20, 380), (100, 399), -1, 256);
        p.drag(100, 299);
        let (cx, top, cw, ch, s) = p.card();
        assert_eq!(s, 128, "half the scale at a quarter of the screen's height");
        assert_eq!(
            (cx, cw, top + ch),
            (50, 100, 280),
            "centred, its bottom at the finger"
        );
        let mut out = vec![0u32; w * h];
        let (mut a, mut b) = (vec![0u32; w], vec![0u32; w]);
        let mut grab = |p: &Peek, now| {
            p.compose(
                now,
                &cur,
                &target,
                Neighbour::None,
                (&mut a, &mut b),
                |y, row| out[y * w..][..w].copy_from_slice(row),
            );
            out.clone()
        };
        let f = grab(&p, 0);
        let mid = (top + ch / 2) as usize;
        assert_eq!(
            f[mid * w + 100] & 0xffff,
            (20 + (mid as i32 - top) * 2) as u32,
            "the page, scaled"
        );
        assert_eq!(f[mid * w + 10], 0xff11_1111, "home beside it");
        assert_eq!(
            f[top as usize * w + cx as usize],
            0xff11_1111,
            "a rounded corner"
        );
        p.pause(0, true);
        let f = grab(&p, SIDES_MS as u64);
        assert_eq!(f[mid * w + 10], CARD, "a side card");
        assert_eq!(f[mid * w + 45], DIM, "the gap");
        assert_eq!(f[10 * w], 0xff11_1111, "the status bar stays");
        // lifted after the pause: Open apps comes up at once
        assert!(!p.clone().release(0, Some(Gesture::OpenApps)));
        // Home settles the card away, a short swipe back to full size
        let mut q = p;
        assert!(q.release(0, Some(Gesture::Home)));
        assert!(!q.step(1000));
        assert_eq!(q.card().4, 0);
        assert!(p.release(0, None));
        assert!(!p.step(1000));
        assert_eq!(p.card(), (0, 20, 200, 360, 256));
    }

    #[test]
    fn corners_round_the_ends_only() {
        assert_eq!(corner(0, 100, 10), 10);
        assert_eq!(corner(50, 100, 10), 0);
        assert_eq!(corner(99, 100, 10), 10);
        assert!(corner(5, 100, 10) > 0 && corner(5, 100, 10) < 10);
    }
}

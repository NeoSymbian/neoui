// SPDX-License-Identifier: Apache-2.0
//! Animation frames composed from snapshots ("layers") instead of Slint renders (smooth-ui S2): a screen is drawn
//! once, then a transition only blends or shifts pixels that are already there. Plain integer code for armv5te (no
//! NEON, no hardware divide).

/// Blend two XRGB8888 rows into `dst`: each channel `(a * (256 - t) + b * t) >> 8`, `t` in 0..=256 (0 = all `a`,
/// 256 = all `b`). Red and blue share one multiply: each lane's product stays under 0x10000, so nothing carries.
pub fn blend_row(dst: &mut [u32], a: &[u32], b: &[u32], t: u32) {
    let t = t.min(256);
    let s = 256 - t;
    for ((d, &a), &b) in dst.iter_mut().zip(a).zip(b) {
        let rb = ((a & 0x00ff_00ff) * s + (b & 0x00ff_00ff) * t) >> 8 & 0x00ff_00ff;
        let g = ((a & 0x0000_ff00) * s + (b & 0x0000_ff00) * t) >> 8 & 0x0000_ff00;
        *d = 0xff00_0000 | rb | g;
    }
}

/// Ease-out-quad (Slint's `ease-out-quad`) on a 0..=256 scale: fast start, gentle stop.
pub fn ease_out_quad(x: u32) -> u32 {
    let r = 256 - x.min(256);
    256 - r * r / 256
}

/// A running animation: its eased progress (0..=256) at a given time, `None` once it is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    start_ms: u64,
    duration_ms: u32,
}

impl Transition {
    pub fn new(start_ms: u64, duration_ms: u32) -> Self {
        Self {
            start_ms,
            duration_ms: duration_ms.max(1),
        }
    }

    pub fn progress(&self, now_ms: u64) -> Option<u32> {
        let dt = now_ms.saturating_sub(self.start_ms);
        if dt >= u64::from(self.duration_ms) {
            return None;
        }
        Some(ease_out_quad(
            (dt * 256 / u64::from(self.duration_ms)) as u32,
        ))
    }
}

/// A row of two pages side by side (smooth-ui S2c): `cur` shifted right by `offset` px, the gap it leaves filled
/// from `next` (the page to the right when `offset` < 0, to the left when > 0). All three rows are as long.
pub fn slide_row(out: &mut [u32], cur: &[u32], next: &[u32], offset: i32) {
    let w = out.len();
    let s = offset.unsigned_abs() as usize;
    if s >= w {
        out.copy_from_slice(next);
    } else if offset <= 0 {
        out[..w - s].copy_from_slice(&cur[s..]);
        out[w - s..].copy_from_slice(&next[..s]);
    } else {
        out[..s].copy_from_slice(&next[w - s..]);
        out[s..].copy_from_slice(&cur[..w - s]);
    }
}

/// The home page swipe as the UI server runs it (smooth-ui S2c): the current page follows the finger 1:1 toward one
/// neighbour (`dir` -1 = the next page, coming in from the right; 1 = the previous one), then settles onto it or back.
#[derive(Clone, Copy, Debug)]
pub struct Slide {
    dir: i32,
    width: i32,
    /// offset minus the finger's x
    base: i32,
    /// where the current page is drawn now (px; 0 = in place, `dir * width` = the neighbour in place)
    pub offset: i32,
    settle: Option<(Transition, i32, i32)>,
}

impl Slide {
    /// Start at `offset` (what is on screen) with the finger at `x`.
    pub fn start(dir: i32, width: i32, x: i32, offset: i32) -> Self {
        Self {
            dir: dir.signum(),
            width,
            base: offset - x,
            offset,
            settle: None,
        }
    }

    pub fn dir(&self) -> i32 {
        self.dir
    }

    pub fn drag(&mut self, x: i32) {
        let o = self.base + x;
        self.offset = if self.dir < 0 {
            o.clamp(-self.width, 0)
        } else {
            o.clamp(0, self.width)
        };
    }

    /// The finger lifted: settle onto the neighbour (`commit`) or back, over `ms`.
    pub fn release(&mut self, now_ms: u64, commit: bool, ms: u32) {
        let to = if commit { self.dir * self.width } else { 0 };
        self.settle = Some((Transition::new(now_ms, ms), self.offset, to));
    }

    pub fn committed(&self) -> bool {
        self.settle.is_some_and(|(_, _, to)| to != 0)
    }

    /// Move the settling page on; true while the finger is down or the page settles, false once it has settled.
    pub fn step(&mut self, now_ms: u64) -> bool {
        let Some((tr, from, to)) = self.settle else {
            return true;
        };
        match tr.progress(now_ms) {
            Some(e) => {
                self.offset = from + (to - from) * e as i32 / 256;
                true
            }
            None => {
                self.offset = to;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(a: u32, b: u32, t: u32) -> u32 {
        let ch = |sh: u32| ((a >> sh & 0xff) * (256 - t) + (b >> sh & 0xff) * t) >> 8 << sh;
        0xff00_0000 | ch(16) | ch(8) | ch(0)
    }

    #[test]
    fn blend_matches_the_per_channel_formula() {
        let a = [
            0xff00_0000,
            0xffff_ffff,
            0xff12_3456,
            0xff80_7f01,
            0x00ff_00ff,
        ];
        let b = [
            0xffff_ffff,
            0xff00_0000,
            0xfffe_dcba,
            0xff01_80ff,
            0x0000_ff00,
        ];
        let mut d = [0; 5];
        for t in [0, 1, 64, 128, 200, 255, 256] {
            blend_row(&mut d, &a, &b, t);
            for i in 0..a.len() {
                assert_eq!(d[i], reference(a[i], b[i], t), "pixel {i}, t {t}");
            }
        }
        blend_row(&mut d, &a, &b, 0);
        assert_eq!(&d[..4], &a[..4]);
        blend_row(&mut d, &a, &b, 256);
        assert_eq!(&d[..4], &b[..4]);
    }

    #[test]
    fn slide_puts_the_neighbour_beside_the_current_row() {
        let cur = [1, 2, 3, 4];
        let next = [5, 6, 7, 8];
        let mut out = [0; 4];
        slide_row(&mut out, &cur, &next, 0);
        assert_eq!(out, cur);
        // finger moved left by 1: the next page comes in from the right
        slide_row(&mut out, &cur, &next, -1);
        assert_eq!(out, [2, 3, 4, 5]);
        slide_row(&mut out, &cur, &next, -4);
        assert_eq!(out, next);
        // finger moved right by 3: the previous page comes in from the left
        slide_row(&mut out, &cur, &next, 3);
        assert_eq!(out, [6, 7, 8, 1]);
    }

    #[test]
    fn slide_follows_the_finger_toward_the_neighbour_only_then_settles() {
        // toward the next page (finger moving left), 100 px wide, started with Slint's drag at -6 and the finger at 50
        let mut s = Slide::start(-1, 100, 50, -6);
        assert_eq!(s.offset, -6);
        s.drag(30);
        assert_eq!(s.offset, -26, "1:1 with the finger");
        s.drag(90);
        assert_eq!(s.offset, 0, "never past the current page");
        s.drag(-200);
        assert_eq!(s.offset, -100, "never past the neighbour");
        s.drag(10);
        s.release(1000, true, 200);
        assert!(s.committed());
        assert!(s.step(1100) && s.offset < -46 && s.offset > -100);
        assert!(!s.step(1200));
        assert_eq!(s.offset, -100);

        let mut s = Slide::start(1, 100, 0, 0);
        s.drag(20);
        assert!(s.step(0), "live while the finger is down");
        s.release(0, false, 200);
        assert!(!s.committed());
        assert!(!s.step(500));
        assert_eq!(s.offset, 0);
    }

    #[test]
    fn easing_and_transition_ends() {
        assert_eq!(ease_out_quad(0), 0);
        assert_eq!(ease_out_quad(256), 256);
        assert!(
            ease_out_quad(128) > 128,
            "ease-out runs ahead at the midpoint"
        );
        let tr = Transition::new(1000, 250);
        assert_eq!(tr.progress(1000), Some(0));
        assert!(tr.progress(1125).is_some_and(|e| e > 128 && e < 256));
        assert_eq!(tr.progress(1250), None);
        assert_eq!(
            tr.progress(900),
            Some(0),
            "a clock before the start counts as the start"
        );
    }
}

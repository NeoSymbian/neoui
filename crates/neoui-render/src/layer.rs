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

// SPDX-License-Identifier: Apache-2.0
//! The analog face, drawn into an RGBA pixel buffer: the phone's Slint (software renderer, no_std) can't draw paths.
//! [`dial`] draws the ring and the hour marks once; [`hands`] blends the hands and the centre over a copy of it each
//! second. Anti-aliased by the distance to each shape; every shape only touches its own bounding box, so the hands
//! cost a fraction of the square even with soft-float.

/// One pixel: red, green, blue, alpha (not premultiplied; Slint's `Rgba8Pixel`).
pub type Rgba = [u8; 4];

/// The ring and the 12 hour marks in `ink`, on a transparent `size` × `size` square.
pub fn dial(buf: &mut [Rgba], size: usize, ink: [u8; 3]) {
    buf.iter_mut().for_each(|p| *p = [0; 4]);
    let (c, r) = centre(size);
    ring(buf, size, c, r - r / 40.0, r / 40.0, ink, 150);
    for i in 0..12 {
        let (s, k) = sin_cos(i as f32 * 30.0);
        let long = if i % 3 == 0 { 0.16 } else { 0.08 };
        let (r0, r1) = (r * (0.9 - long), r * 0.9);
        stroke(
            buf,
            size,
            (c + s * r0, c - k * r0),
            (c + s * r1, c - k * r1),
            r / if i % 3 == 0 { 28.0 } else { 45.0 },
            ink,
            255,
        );
    }
}

/// The hands at `h:m:s` (and the seconds hand in `accent` when `seconds`), then the centre, over the dial.
pub fn hands(
    buf: &mut [Rgba],
    size: usize,
    (h, m, s): (u32, u32, u32),
    seconds: bool,
    ink: [u8; 3],
    accent: [u8; 3],
) {
    let (c, r) = centre(size);
    let hand = |buf: &mut [Rgba], deg: f32, len: f32, width: f32, color: [u8; 3]| {
        let (sn, cs) = sin_cos(deg);
        let tail = len * 0.15;
        stroke(
            buf,
            size,
            (c - sn * tail, c + cs * tail),
            (c + sn * len, c - cs * len),
            width,
            color,
            255,
        );
    };
    let (h, m, s) = (h % 12, m % 60, s % 60);
    hand(
        buf,
        h as f32 * 30.0 + m as f32 * 0.5,
        r * 0.5,
        r / 14.0,
        ink,
    );
    hand(
        buf,
        m as f32 * 6.0 + s as f32 * 0.1,
        r * 0.75,
        r / 20.0,
        ink,
    );
    if seconds {
        hand(buf, s as f32 * 6.0, r * 0.82, r / 60.0, accent);
    }
    let dot = if seconds { accent } else { ink };
    stroke(buf, size, (c, c), (c, c), r / 9.0, dot, 255);
}

fn centre(size: usize) -> (f32, f32) {
    let c = size as f32 / 2.0;
    (c, c - 1.0)
}

/// sin and cos of `deg` degrees (Taylor series after reducing to ±90°; error < 1e-5, plenty for pixels).
pub fn sin_cos(deg: f32) -> (f32, f32) {
    (sin_deg(deg), sin_deg(deg + 90.0))
}

fn sin_deg(deg: f32) -> f32 {
    let mut d = deg % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    // sin(x) = sin(180 - x): fold into [-90, 90]
    if d > 90.0 {
        d = 180.0 - d;
    } else if d < -90.0 {
        d = -180.0 - d;
    }
    let x = d * (core::f32::consts::PI / 180.0);
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0))))
}

fn sqrt(v: f32) -> f32 {
    // Newton's method from a bit-level guess; enough for edge distances (no libm in this crate)
    if v <= 0.0 {
        return 0.0;
    }
    let mut x = f32::from_bits((v.to_bits() >> 1) + 0x1fbd_1df5);
    for _ in 0..3 {
        x = 0.5 * (x + v / x);
    }
    x
}

/// Blends `color` at `alpha` × `cover` (0..=1) over the pixel ("over", not premultiplied).
fn blend(p: &mut Rgba, color: [u8; 3], alpha: u8, cover: f32) {
    let a = (alpha as f32 * cover.clamp(0.0, 1.0)) / 255.0;
    if a <= 0.0 {
        return;
    }
    let da = p[3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    for i in 0..3 {
        let v = (color[i] as f32 * a + p[i] as f32 * da * (1.0 - a)) / oa;
        p[i] = (v + 0.5) as u8;
    }
    p[3] = (oa * 255.0 + 0.5) as u8;
}

/// The pixel rows/columns a shape reaching from `lo` to `hi` covers, clipped to the square.
fn span(lo: f32, hi: f32, size: usize) -> core::ops::Range<usize> {
    let a = (lo - 1.0).max(0.0) as usize;
    let b = ((hi + 2.0).max(0.0) as usize).min(size);
    a..b.max(a)
}

/// A round-capped line from `a` to `b`, `width` px wide.
fn stroke(
    buf: &mut [Rgba],
    size: usize,
    a: (f32, f32),
    b: (f32, f32),
    width: f32,
    color: [u8; 3],
    alpha: u8,
) {
    let half = width / 2.0;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    for y in span(a.1.min(b.1) - half, a.1.max(b.1) + half, size) {
        for x in span(a.0.min(b.0) - half, a.0.max(b.0) + half, size) {
            let (px, py) = (x as f32 + 0.5 - a.0, y as f32 + 0.5 - a.1);
            let t = if len2 > 0.0 {
                ((px * dx + py * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (ex, ey) = (px - t * dx, py - t * dy);
            let d2 = ex * ex + ey * ey;
            if d2 > (half + 0.5) * (half + 0.5) {
                continue;
            }
            let cover = if d2 < (half - 0.5).max(0.0) * (half - 0.5).max(0.0) {
                1.0
            } else {
                half + 0.5 - sqrt(d2)
            };
            blend(&mut buf[y * size + x], color, alpha, cover);
        }
    }
}

/// A circle of radius `r` around (`c`, `c`), `width` px thick.
fn ring(buf: &mut [Rgba], size: usize, c: f32, r: f32, width: f32, color: [u8; 3], alpha: u8) {
    let half = width / 2.0;
    let (outer, inner) = (r + half + 0.5, (r - half - 0.5).max(0.0));
    for y in span(c - outer, c + outer, size) {
        for x in span(c - outer, c + outer, size) {
            let (px, py) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let d2 = px * px + py * py;
            if d2 > outer * outer || d2 < inner * inner {
                continue;
            }
            blend(
                &mut buf[y * size + x],
                color,
                alpha,
                half + 0.5 - (sqrt(d2) - r).abs(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const S: usize = 200;
    const INK: [u8; 3] = [255, 255, 255];
    const ACCENT: [u8; 3] = [255, 0, 0];

    fn face(t: (u32, u32, u32), seconds: bool) -> alloc::vec::Vec<Rgba> {
        let mut b = vec![[0u8; 4]; S * S];
        dial(&mut b, S, INK);
        hands(&mut b, S, t, seconds, INK, ACCENT);
        b
    }

    fn at(b: &[Rgba], x: f32, y: f32) -> Rgba {
        b[y as usize * S + x as usize]
    }

    #[test]
    fn sin_cos_is_close() {
        for d in [0.0, 30.0, 45.0, 90.0, 135.0, 180.0, 270.0, 359.5, -45.0] {
            let (s, c) = sin_cos(d);
            let r = d * core::f32::consts::PI / 180.0;
            assert!((s - libm_sin(r)).abs() < 1e-4, "sin {}", d);
            assert!(
                (c - libm_sin(r + core::f32::consts::FRAC_PI_2)).abs() < 1e-4,
                "cos {}",
                d
            );
        }
    }

    // the reference: std's sin on the host (tests only)
    fn libm_sin(x: f32) -> f32 {
        extern crate std;
        std::primitive::f32::sin(x)
    }

    #[test]
    fn three_oclock_points_right_and_twelve_up() {
        let b = face((3, 0, 0), false);
        let (c, r) = centre(S);
        assert_eq!(at(&b, c + r * 0.4, c)[3], 255, "hour hand to the right");
        assert_eq!(at(&b, c, c - r * 0.6)[3], 255, "minute hand up");
        assert_eq!(at(&b, c - r * 0.4, c)[3], 0, "nothing to the left");
    }

    #[test]
    fn seconds_hand_only_when_asked_and_in_the_accent() {
        let (c, r) = centre(S);
        let on = face((0, 0, 15), true);
        let p = at(&on, c + r * 0.7, c);
        assert_eq!((p[0], p[1]), (255, 0), "red seconds hand at 15 s");
        assert!(
            p[3] > 150,
            "a thin hand mostly covers its centre pixel ({})",
            p[3]
        );
        let off = face((0, 0, 15), false);
        assert_eq!(at(&off, c + r * 0.7, c)[3], 0);
    }

    #[test]
    fn ring_and_corners() {
        let b = face((10, 8, 30), true);
        assert_eq!(b[0][3], 0, "a corner stays transparent");
        let (c, r) = centre(S);
        assert!(at(&b, c, c - r + r / 40.0)[3] > 100, "the ring at 12");
        assert!(at(&b, c, c)[3] == 255, "the centre dot");
    }
}

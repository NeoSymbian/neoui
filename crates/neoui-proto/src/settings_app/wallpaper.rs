// SPDX-License-Identifier: Apache-2.0
//! D7, wallpaper pictures (set2-wallpaper "From Files"; protocol.md `SETTINGS_ACTION` 8 `INSTALL_WALLPAPER`): the
//! raster Settings sends and the shell keeps, and [`cover`], which fills it from a decoded picture of any size (the
//! middle kept, the rest cut, as CSS `object-fit: cover`). Decoding is NEOSET's (files' `neo-format-image`); storing
//! and drawing it is the shell's (`symbian/src/wallpaper.rs`, home's `WallpaperPicture`).

use super::wire::WallpaperRequest;
use alloc::vec;
use alloc::vec::Vec;

/// The raster's size: half the Pixel's 1080×2160 panel (home draws it scaled up).
pub const W: u16 = 540;
pub const H: u16 = 1080;
/// `display.wallpaper`'s value while home draws the installed picture (the colours keep their names).
pub const KEY: &str = "picture";

/// The one request the shell accepts.
pub const REQUEST: WallpaperRequest = WallpaperRequest {
    width: W,
    height: H,
};

/// RGB888 bytes of a [`W`]×[`H`] raster.
pub const BYTES: usize = W as usize * H as usize * 3;

/// Whether the shell takes request `r`: only [`REQUEST`] (the size it draws).
#[must_use]
pub fn accepted(r: &WallpaperRequest) -> bool {
    *r == REQUEST
}

/// `src` (`w`×`h` pixels of `ch` bytes each, 1 grey, 3 RGB or 4 RGBA; tightly packed, top-down) scaled to cover
/// `out_w`×`out_h` and cropped to its middle, as tightly packed RGB888. Each output pixel averages the source pixels
/// under it (a box filter), or takes the nearest when the picture is smaller. Alpha is put over black. `None` for an
/// empty picture, an unknown channel count or a buffer of the wrong length.
#[must_use]
pub fn cover(src: &[u8], w: u32, h: u32, ch: u8, out_w: u32, out_h: u32) -> Option<Vec<u8>> {
    let c = usize::from(ch);
    if w == 0 || h == 0 || out_w == 0 || out_h == 0 || !matches!(ch, 1 | 3 | 4) {
        return None;
    }
    if src.len() != (w as usize).checked_mul(h as usize)?.checked_mul(c)? {
        return None;
    }
    // the crop: the largest window of the output's shape, centred (in source pixels, 1/65536 fixed point)
    let (cw, chh) = if u64::from(w) * u64::from(out_h) > u64::from(h) * u64::from(out_w) {
        (
            u64::from(h) * u64::from(out_w) * 65536 / u64::from(out_h),
            u64::from(h) << 16,
        )
    } else {
        (
            u64::from(w) << 16,
            u64::from(w) * u64::from(out_h) * 65536 / u64::from(out_w),
        )
    };
    let (x0, y0) = (
        ((u64::from(w) << 16) - cw) / 2,
        ((u64::from(h) << 16) - chh) / 2,
    );
    // the source span [a, b) of output index `i` along one axis, at least one pixel
    let span = |i: u32, n: u32, start: u64, len: u64, max: u32| {
        let a = ((start + len * u64::from(i) / u64::from(n)) >> 16) as u32;
        let b = ((start + len * (u64::from(i) + 1) / u64::from(n)) >> 16) as u32;
        let a = a.min(max - 1);
        (a, b.clamp(a + 1, max))
    };
    let mut out = vec![0u8; out_w as usize * out_h as usize * 3];
    for oy in 0..out_h {
        let (ya, yb) = span(oy, out_h, y0, chh, h);
        for ox in 0..out_w {
            let (xa, xb) = span(ox, out_w, x0, cw, w);
            let mut sum = [0u32; 3];
            for y in ya..yb {
                let row = y as usize * w as usize;
                for x in xa..xb {
                    let p = &src[(row + x as usize) * c..][..c];
                    let rgb = match ch {
                        1 => [p[0]; 3],
                        3 => [p[0], p[1], p[2]],
                        _ => {
                            let a = u16::from(p[3]);
                            [0, 1, 2].map(|i| ((u16::from(p[i]) * a + 127) / 255) as u8)
                        }
                    };
                    for (s, v) in sum.iter_mut().zip(rgb) {
                        *s += u32::from(v);
                    }
                }
            }
            let n = (yb - ya) * (xb - xa);
            let o = (oy as usize * out_w as usize + ox as usize) * 3;
            for (d, s) in out[o..o + 3].iter_mut().zip(sum) {
                *d = ((s + n / 2) / n) as u8;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_the_shell_takes() {
        let b = REQUEST.encode().unwrap();
        let r = WallpaperRequest::decode(&b).unwrap();
        assert!(accepted(&r));
        assert_eq!(r.bytes(), Some(BYTES as u32));
        assert!(!accepted(&WallpaperRequest {
            width: 1080,
            height: 2160
        }));
    }

    #[test]
    fn a_wide_picture_keeps_its_middle() {
        // 8×2: left quarter red, middle half green, right quarter blue; covering 2×2 keeps the green middle
        let mut src = Vec::new();
        for _ in 0..2 {
            for x in 0..8 {
                src.extend_from_slice(match x {
                    0 | 1 => &[255, 0, 0],
                    6 | 7 => &[0, 0, 255],
                    _ => &[0, 255, 0],
                });
            }
        }
        let out = cover(&src, 8, 2, 3, 2, 2).unwrap();
        assert_eq!(out, [0, 255, 0].repeat(4));
    }

    #[test]
    fn a_tall_picture_and_the_box_average() {
        // 2×8 grey, rows 0..8 = 0, 32, .., 224; covering 1×2 keeps rows 2..6, two rows a pixel
        let src: Vec<u8> = (0..8).flat_map(|y| [y as u8 * 32; 2]).collect();
        let out = cover(&src, 2, 8, 1, 1, 2).unwrap();
        assert_eq!(out, [80, 80, 80, 144, 144, 144]);
    }

    #[test]
    fn small_pictures_grow_and_alpha_is_over_black() {
        let out = cover(&[200, 100, 50, 128], 1, 1, 4, 3, 6).unwrap();
        assert_eq!(out.len(), 3 * 6 * 3);
        assert_eq!(&out[..3], &[100, 50, 25]);
        assert!(out.chunks(3).all(|p| p == [100, 50, 25]));
        assert_eq!(cover(&[0; 5], 1, 1, 3, 2, 2), None);
        assert_eq!(cover(&[0; 2], 1, 1, 2, 2, 2), None);
        assert_eq!(cover(&[], 0, 1, 3, 2, 2), None);
        // the real size: a 4:3 photo covers the 1:2 screen
        let photo = vec![7u8; 400 * 300 * 3];
        let out = cover(&photo, 400, 300, 3, u32::from(W), u32::from(H)).unwrap();
        assert_eq!(out.len(), BYTES);
        assert!(out.iter().all(|&b| b == 7));
    }
}

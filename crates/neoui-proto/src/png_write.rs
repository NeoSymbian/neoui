// SPDX-License-Identifier: Apache-2.0
//! A small PNG writer for the shell's screenshots (Power menu v2, `power_menu::act::SCREENSHOT`): 8-bit RGB, every
//! row Up-filtered, one fixed-Huffman deflate block whose only matches reach back one byte or one pixel. A UI screen
//! is mostly rows equal to the one above, or the one above plus a gradient's step, so the filter leaves long runs that
//! this squeezes well with no tables to build.
//! shortcut: no real LZ77 matching; photos and video frames stay near raw size; use a real deflater if a screenshot of
//! one matters.

use alloc::vec;
use alloc::vec::Vec;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
/// deflate's lengths 3..=258: each code's base and extra bits (RFC 1951 3.2.5), codes 257..=285
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
};

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut c = !0u32;
    for b in parts.iter().flat_map(|p| p.iter()) {
        c = CRC_TABLE[((c ^ u32::from(*b)) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    out.extend(crc32(&[kind, data]).to_be_bytes());
}

/// deflate's bit order: values LSB first, Huffman codes MSB first.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, v: u32, n: u32) {
        self.acc |= u64::from(v) << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn code(&mut self, code: u32, len: u32) {
        self.put(code.reverse_bits() >> (32 - len), len);
    }

    /// A literal/length symbol in the fixed code (RFC 1951 3.2.6).
    fn sym(&mut self, s: u32) {
        match s {
            0..=143 => self.code(0x30 + s, 8),
            144..=255 => self.code(0x190 + s - 144, 9),
            256..=279 => self.code(s - 256, 7),
            _ => self.code(0xc0 + s - 280, 8),
        }
    }

    /// `len` (3..=258) bytes copied from distance code `dist` back (0: 1 byte, 2: 3 bytes, one pixel).
    fn copy(&mut self, len: usize, dist: u32) {
        let i = LEN_BASE
            .iter()
            .rposition(|b| usize::from(*b) <= len)
            .unwrap_or(0);
        self.sym(257 + i as u32);
        self.put(
            (len - usize::from(LEN_BASE[i])) as u32,
            u32::from(LEN_EXTRA[i]),
        );
        self.code(dist, 5);
    }
}

/// The screen as a PNG: `row(y, out)` fills `out` (`w` pixels, `0x00RRGGBB`) with row `y`.
#[must_use]
pub fn encode(w: usize, h: usize, mut row: impl FnMut(usize, &mut [u32])) -> Vec<u8> {
    let mut bits = Bits {
        out: vec![0x78, 0x01], // zlib: deflate, 32 K window, no dictionary
        acc: 0,
        n: 0,
    };
    bits.put(1, 1); // the final block
    bits.put(1, 2); // fixed Huffman codes
    let (mut a, mut b) = (1u32, 0u32); // Adler-32 of the raw stream
    let (mut prev, mut cur) = (vec![0u8; w * 3], vec![0u8; w * 3]);
    let mut px = vec![0u32; w];
    let mut line = vec![0u8; 1 + w * 3];
    // the last 3 bytes of the stream (how far back a match reaches) and how many of them are real
    let (mut hist, mut have) = ([0u8; 3], 0);
    let mut buf = vec![0u8; 3 + line.len()];
    for y in 0..h {
        row(y, &mut px);
        for (c, p) in cur.as_chunks_mut::<3>().0.iter_mut().zip(&px) {
            *c = [(p >> 16) as u8, (p >> 8) as u8, *p as u8];
        }
        line[0] = 2; // the Up filter
        for (i, (c, p)) in cur.iter().zip(&prev).enumerate() {
            line[1 + i] = c.wrapping_sub(*p);
        }
        core::mem::swap(&mut prev, &mut cur);
        for chunk in line.chunks(5552) {
            for v in chunk {
                a += u32::from(*v);
                b += a;
            }
            (a, b) = (a % 65521, b % 65521);
        }
        // matches may reach back into the row before: the stream is one
        buf[..3].copy_from_slice(&hist);
        buf[3..].copy_from_slice(&line);
        let mut i = 3;
        while i < buf.len() {
            let mut best = (0, 0);
            for (d, code) in [(1, 0), (3, 2)] {
                if have + i - 3 >= d {
                    let n = (i..buf.len().min(i + 258))
                        .take_while(|&j| buf[j] == buf[j - d])
                        .count();
                    if n > best.0 {
                        best = (n, code);
                    }
                }
            }
            if best.0 >= 3 {
                bits.copy(best.0, best.1);
                i += best.0;
            } else {
                bits.sym(u32::from(buf[i]));
                i += 1;
            }
        }
        hist.copy_from_slice(&buf[buf.len() - 3..]);
        have = (have + line.len()).min(3);
    }
    bits.sym(256);
    if bits.n > 0 {
        bits.out.push(bits.acc as u8);
    }
    let mut z = bits.out;
    z.extend(((b << 16) | a).to_be_bytes());

    let mut out = Vec::with_capacity(z.len() + 64);
    out.extend(SIGNATURE);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]); // 8-bit RGB, deflate, adaptive filters, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let mut r = ::png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, ::png::ColorType::Rgb);
        buf.truncate(info.buffer_size());
        (info.width, info.height, buf)
    }

    fn check(w: usize, h: usize, px: impl Fn(usize, usize) -> u32) -> usize {
        let png = encode(w, h, |y, out| {
            for (x, p) in out.iter_mut().enumerate() {
                *p = px(x, y);
            }
        });
        let (dw, dh, rgb) = decode(&png);
        assert_eq!((dw as usize, dh as usize), (w, h));
        for y in 0..h {
            for x in 0..w {
                let p = px(x, y);
                let i = (y * w + x) * 3;
                assert_eq!(
                    &rgb[i..i + 3],
                    &[(p >> 16) as u8, (p >> 8) as u8, p as u8],
                    "pixel {x},{y}"
                );
            }
        }
        png.len()
    }

    #[test]
    fn a_ui_like_screen_decodes_and_is_small() {
        // flat bands, a gradient and some "text": runs of every length, across rows
        let (w, h) = (539, 1079);
        let size = check(w, h, |x, y| match y {
            0..=36 => 0x0012_1418,
            37..=300 => 0x0015_181c + ((x as u32 / 7) % 3),
            301..=600 => ((y as u32) << 8) & 0xff00 | 0x40,
            _ if y % 20 < 12 && (x / 3 + y / 5) % 17 == 0 => 0x00ff_ffff,
            _ => 0x0005_070b,
        });
        let raw = h * (1 + w * 3);
        assert!(size * 4 < raw, "{size} bytes of {raw}");
    }

    #[test]
    fn noise_and_tiny_images_decode() {
        let mut s = 0x1234_5678u32;
        let noise: Vec<u32> = (0..64 * 40)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s & 0x00ff_ffff
            })
            .collect();
        check(64, 40, |x, y| noise[y * 64 + x]);
        check(1, 1, |_, _| 0x0080_4020);
        // a run of exactly 258 and one past it, and runs of 3 that start right after a literal
        check(300, 2, |x, _| if x < 4 { x as u32 } else { 0x0007_0707 });
    }
}

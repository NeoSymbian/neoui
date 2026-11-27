// SPDX-License-Identifier: Apache-2.0
//! The Power menu v2 (docs neoui-shell/top-panel.md "Power menu v2 (POWER)", board `sh-power-menu-v2`, O5): what its
//! buttons report to the UI server, the ring tile's General → Vibrate → Silent cycle through the sound profiles'
//! reducer, the dimmed backdrop under the sheet and the screenshot's file name.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::sound_profile::{Command, Mirrors, OFFLINE, Profiles, SILENT};

/// What the menu reports (`Renderer::take_power`), as `ui/components/power-sheet.slint` and `powermenu.slint` send it.
/// 0–2 are `ipc::POWER_*`, the kernel's own numbers.
pub mod act {
    pub const RESTART: i32 = crate::ipc::POWER_RESTART;
    pub const FASTBOOT: i32 = crate::ipc::POWER_FASTBOOT;
    pub const OFF: i32 = crate::ipc::POWER_OFF;
    /// Closed without a choice: a tap outside the short list's card, a swipe down on the sheet.
    pub const DISMISSED: i32 = -1;
    /// Power off or Restart let go before its ring filled: nothing happens, the sheet stays.
    pub const CANCELLED: i32 = -2;
    /// Lock now: the screen goes off (TP8's lock screen shows at the next wake).
    pub const LOCK: i32 = 10;
    /// The screen as it was under the sheet, as a PNG in [`SHOT_DIR`].
    pub const SCREENSHOT: i32 = 11;
    /// The ring tile: the next of General, Vibrate and Silent ([`super::next_ring`]); the sheet stays.
    pub const RING: i32 = 12;
    /// The Emergency call pill.
    pub const EMERGENCY: i32 = 13;
    /// The short list's profile rows: `PROFILE + id`.
    pub const PROFILE: i32 = 100;
}

/// How long Power off and Restart are held (the ring's fill, `power-sheet.slint` `HOLD`).
pub const HOLD_MS: u64 = 1200;

/// Where screenshots go: the pictures folder of S60 and of Files' Images group.
pub const SHOT_DIR: &str = "C:\\Data\\Images\\";

/// What a call does now, from the live mirrors MODEMD rings with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ring {
    /// It rings (the active profile's name is the tile's label).
    Ring,
    Vibrate,
    Silent,
}

#[must_use]
pub fn ring(m: Mirrors) -> Ring {
    match (m.silent, m.vibrate) {
        (false, _) => Ring::Ring,
        (true, true) => Ring::Vibrate,
        (true, false) => Ring::Silent,
    }
}

/// The ring tile's commands for the reducer (`sound_profile::store::apply`, in order): ringing → Silent with
/// vibration (Vibrate), Vibrate → the same profile without it (Silent), Silent → the last audible profile. From
/// Offline (it sounds like General) the first step chooses Silent, which leaves Offline as any other choice does.
#[must_use]
pub fn next_ring(p: &Profiles) -> Vec<Command> {
    let volume = |id| p.get(id).map_or(8, |q| q.volume);
    match ring(p.mirrors()) {
        Ring::Ring => vec![
            Command::Choose(SILENT),
            Command::Set {
                id: SILENT,
                ring: false,
                volume: volume(SILENT),
                vibrate: true,
            },
        ],
        Ring::Vibrate => {
            // Offline's sound is General's, which Set refuses to reach through it: go to Silent first
            let id = if p.active == OFFLINE {
                SILENT
            } else {
                p.active
            };
            vec![
                Command::Choose(id),
                Command::Set {
                    id,
                    ring: false,
                    volume: volume(id),
                    vibrate: false,
                },
            ]
        }
        Ring::Silent => vec![Command::Choose(p.last_audible)],
    }
}

/// The sheet's backdrop (board: the screen blurred under an 85 % dark scrim): the screen averaged over `step` × `step`
/// blocks, as RGB bytes and the smaller size. `row(y, out)` fills `out` (`w` pixels, `0x00RRGGBB`) with screen row `y`,
/// so the screen is never copied whole.
pub fn backdrop(
    w: usize,
    h: usize,
    step: usize,
    mut row: impl FnMut(usize, &mut [u32]),
) -> (Vec<u8>, usize, usize) {
    let step = step.max(1);
    let (bw, bh) = (w.div_ceil(step), h.div_ceil(step));
    let mut sums = vec![[0u32; 3]; bw];
    let mut out = Vec::with_capacity(bw * bh * 3);
    let mut line = vec![0u32; w];
    for by in 0..bh {
        sums.iter_mut().for_each(|s| *s = [0; 3]);
        let rows = (by * step..((by + 1) * step).min(h)).len();
        for y in by * step..by * step + rows {
            row(y, &mut line);
            for (x, p) in line.iter().enumerate() {
                let s = &mut sums[x / step];
                s[0] += (p >> 16) & 0xff;
                s[1] += (p >> 8) & 0xff;
                s[2] += p & 0xff;
            }
        }
        for (bx, s) in sums.iter().enumerate() {
            let n = ((bx * step..((bx + 1) * step).min(w)).len() * rows) as u32;
            // a block's mean fits a byte
            out.extend(s.iter().map(|c| (c / n) as u8));
        }
    }
    (out, bw, bh)
}

/// A box blur of radius `r` over an RGB image (`backdrop`'s), across then down; twice looks Gaussian. Slint's software
/// renderer stretches an image by its nearest pixel, so the backdrop is blurred before it is drawn.
pub fn blur(rgb: &mut [u8], w: usize, h: usize, r: usize) {
    let mut line = Vec::new();
    for (len, count, at) in [
        (
            w,
            h,
            (|i, j, w, _h| (j * w + i) * 3) as fn(usize, usize, usize, usize) -> usize,
        ),
        (h, w, |i, j, w, _h| (i * w + j) * 3),
    ] {
        for j in 0..count {
            line.clear();
            line.extend((0..len).map(|i| {
                let k = at(i, j, w, h);
                [rgb[k], rgb[k + 1], rgb[k + 2]]
            }));
            for i in 0..len {
                let (lo, hi) = (i.saturating_sub(r), (i + r).min(len - 1));
                let n = (hi - lo + 1) as u32;
                let k = at(i, j, w, h);
                for c in 0..3 {
                    let sum: u32 = line[lo..=hi].iter().map(|p| u32::from(p[c])).sum();
                    rgb[k + c] = (sum / n) as u8;
                }
            }
        }
    }
}

/// `C:\Data\Images\Screenshot_20261127_102400.png` for that local time.
#[must_use]
pub fn shot_path(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> String {
    format!("{SHOT_DIR}Screenshot_{year:04}{month:02}{day:02}_{hour:02}{minute:02}{second:02}.png")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_profile::{GENERAL, OUTDOOR, Radios};

    fn run(p: &mut Profiles) {
        for c in next_ring(p) {
            p.apply(c, Radios::default()).unwrap();
        }
    }

    #[test]
    fn the_ring_tile_cycles_general_vibrate_silent() {
        let mut p = Profiles::upgrade(Mirrors {
            silent: false,
            vibrate: true,
            volume: 6,
        });
        assert_eq!(ring(p.mirrors()), Ring::Ring);
        run(&mut p);
        assert_eq!((p.active, ring(p.mirrors())), (SILENT, Ring::Vibrate));
        run(&mut p);
        assert_eq!((p.active, ring(p.mirrors())), (SILENT, Ring::Silent));
        run(&mut p);
        assert_eq!((p.active, ring(p.mirrors())), (GENERAL, Ring::Ring));
        // the ring volume came through untouched
        assert_eq!(p.mirrors().volume, 6);
    }

    #[test]
    fn silent_goes_back_to_the_profile_it_came_from() {
        let mut p = Profiles::upgrade(Mirrors {
            silent: false,
            vibrate: true,
            volume: 8,
        });
        p.apply(Command::Choose(OUTDOOR), Radios::default())
            .unwrap();
        run(&mut p);
        run(&mut p);
        run(&mut p);
        assert_eq!((p.active, ring(p.mirrors())), (OUTDOOR, Ring::Ring));
    }

    #[test]
    fn offline_counts_as_general_and_the_cycle_leaves_it() {
        let mut p = Profiles::upgrade(Mirrors {
            silent: false,
            vibrate: true,
            volume: 8,
        });
        p.apply(Command::Choose(OFFLINE), Radios::default())
            .unwrap();
        assert_eq!(ring(p.mirrors()), Ring::Ring);
        run(&mut p);
        assert_eq!((p.active, ring(p.mirrors())), (SILENT, Ring::Vibrate));
    }

    #[test]
    fn the_backdrop_averages_blocks_and_keeps_the_ragged_edge() {
        // 5 × 3 pixels in blocks of 2: 3 × 2 blocks, the last column and row narrower
        let px = |x: usize, y: usize| ((x * 40) as u32) << 16 | ((y * 100) as u32) << 8 | 0x10;
        let (rgb, bw, bh) = backdrop(5, 3, 2, |y, out| {
            for (x, p) in out.iter_mut().enumerate() {
                *p = px(x, y);
            }
        });
        assert_eq!((bw, bh), (3, 2));
        assert_eq!(&rgb[..3], &[20, 50, 0x10]); // x 0..2, y 0..2
        assert_eq!(&rgb[6..9], &[160, 50, 0x10]); // x 4 only
        assert_eq!(&rgb[9..12], &[20, 200, 0x10]); // y 2 only
    }

    #[test]
    fn the_blur_spreads_a_dot_and_keeps_a_flat_image() {
        let mut flat = vec![40u8; 4 * 3 * 3];
        blur(&mut flat, 4, 3, 2);
        assert!(flat.iter().all(|c| *c == 40));
        // a white dot in the middle of 5 × 5: a 3 × 3 box spreads it to its neighbours, ninths each
        let mut dot = vec![0u8; 5 * 5 * 3];
        dot[(2 * 5 + 2) * 3..][..3].copy_from_slice(&[255, 255, 255]);
        blur(&mut dot, 5, 5, 1);
        assert_eq!(dot[(5 + 1) * 3], 255 / 3 / 3);
        assert_eq!(dot[0], 0);
    }

    #[test]
    fn the_shot_is_named_by_its_local_time() {
        assert_eq!(
            shot_path(2026, 11, 7, 9, 4, 5),
            "C:\\Data\\Images\\Screenshot_20261107_090405.png"
        );
    }
}

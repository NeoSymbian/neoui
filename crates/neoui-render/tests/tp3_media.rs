// SPDX-License-Identifier: Apache-2.0
#[allow(dead_code)]
mod common;

use neoui_proto::media::{Session, caps, cmd, modes, state};
use neoui_proto::{Rect, page::Notification};

const NOW: u64 = 1_795_691_635_000;

fn song() -> Session {
    Session {
        state: state::PLAYING,
        caps: caps::ALL,
        modes: modes::REPEAT,
        title: "Yerevan Nights".into(),
        artist: "Lusine Trio".into(),
        album: "Live at the Café".into(),
        art: 0,
        duration_ms: 245_000,
        position_ms: 70_000,
        at: NOW - 2_000,
    }
}

/// Where two frames differ (the box around every changed pixel).
fn diff(a: &[u32], b: &[u32], w: u32) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (i, (p, q)) in a.iter().zip(b).enumerate() {
        if p != q {
            let (x, y) = (i as u32 % w, i as u32 / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    assert!(x0 <= x1, "nothing changed");
    Rect::new(x0 as i32, y0 as i32, x1 - x0 + 1, y1 - y0 + 1)
}

/// The middle of the largest white disc in `r` (the play/pause button): the column with the longest run of white.
fn white_disc(buf: &[u32], w: u32, r: Rect) -> (i32, i32, u32) {
    let mut best = (0, 0, 0);
    for x in r.x..r.x + r.w as i32 {
        let mut run = 0;
        for y in r.y..r.y + r.h as i32 {
            if buf[(y as u32 * w + x as u32) as usize] & 0xFF_FFFF == 0xFF_FFFF {
                run += 1;
                if run > best.2 {
                    best = (x, y - run as i32 / 2, run);
                }
            } else {
                run = 0;
            }
        }
    }
    best
}

/// TP3 in the shell (nt-home-media, tp-shade, nt-volume): home's Media card goes live with the session and its
/// play/pause sends PAUSE; the shade's player under brightness, its pause and its seek; the last track paused once the
/// session ended (its button opens the app instead); the volume HUD under the status bar. Snapshots
/// `nt-home-media.png`, `tp-shade-media.png` (board `tp-shade`), `nt-volume.png`.
#[test]
fn home_card_shade_player_and_volume_hud() {
    let (w, h) = (1080, 2160);
    let d = common::dp(w, h);
    let mut r = common::renderer(w, h);
    let mut buf = vec![0; (w * h) as usize];
    r.show("home", "");
    common::frame(&mut r, &mut buf, w, "home");
    let quiet = buf.clone();

    // home's Media card: the session's track, its owner, prev / pause / next and (with room) the bar
    let s = song();
    r.set_media(Some(("files", &s, true)), true, NOW);
    common::frame(&mut r, &mut buf, w, "home media");
    common::write_png("nt-home-media.png", &buf, w, h);
    let card = diff(&quiet, &buf, w);
    let (x, y, run) = white_disc(&buf, w, card);
    assert!(run as f32 > 30.0 * d, "home's pause button: {run} px");
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "home pause");
    assert_eq!(r.take_media_commands(), vec![(cmd::PAUSE, 0)]);
    let home_media = buf.clone();

    // the shade: the player under brightness (no cards here)
    r.set_media(None, true, NOW);
    common::tap(&mut r, (w / 2) as i32, (12.0 * d) as i32);
    common::frame(&mut r, &mut buf, w, "shade");
    let bare = buf.clone();
    r.set_media(Some(("files", &s, true)), true, NOW);
    common::frame(&mut r, &mut buf, w, "shade media");
    common::write_png("tp-shade-media.png", &buf, w, h);
    let player = diff(&bare, &buf, w);
    assert!(
        player.y as f32 > 150.0 * d,
        "under the tiles and brightness: {player:?}"
    );
    let (px, py, run) = white_disc(&buf, w, player);
    assert!(run as f32 > 40.0 * d, "the shade's pause: {run} px");
    // NEOKIT's virt-taps step taps it on the 539x1079 screen at half these coordinates
    println!("shade pause at ({px}, {py}); virt ({}, {})", px / 2, py / 2);
    assert!(
        (px / 2 - 268).abs() <= 2 && (py / 2 - 511).abs() <= 6,
        "virt-taps' shade pause (268, 511) moved: ({}, {})",
        px / 2,
        py / 2
    );
    common::tap(&mut r, px, py);
    common::frame(&mut r, &mut buf, w, "shade pause");
    assert_eq!(r.take_media_commands(), vec![(cmd::PAUSE, 0)]);
    assert!(r.overlay_open(), "a media button leaves the shade open");

    // shell.media off: no player in the shade
    r.set_media(Some(("files", &s, true)), false, NOW);
    common::frame(&mut r, &mut buf, w, "shade, shell.media off");
    // the player's top 100 dp (home's own Media card behind the scrim differs further down)
    let same = (player.y..player.y + (100.0 * d) as i32).all(|y| {
        let row = (y as u32 * w) as usize + player.x as usize;
        buf[row..row + player.w as usize] == bare[row..row + player.w as usize]
    });
    assert!(same, "shell.media off hides the player");
    r.show("home", "");

    // the session ended: home shows the last track paused, and its play opens the app (no command)
    let mut last = s.clone();
    last.state = state::PAUSED;
    r.set_media(Some(("files", &last, false)), true, NOW);
    common::frame(&mut r, &mut buf, w, "last track");
    assert!(buf != home_media, "paused: a play glyph, no prev/next");
    let (x, y, _) = white_disc(&buf, w, card);
    common::tap(&mut r, x, y);
    common::frame(&mut r, &mut buf, w, "last track play");
    assert!(r.take_media_commands().is_empty());
    assert_eq!(r.screen(), "files", "the last track's play opens its app");
    r.show("home", "");
    r.set_media(None, true, NOW);
    common::frame(&mut r, &mut buf, w, "nothing playing");
    assert!(buf == quiet, "with nothing the card is as it was");

    // the volume HUD under the status bar, over a card from Messages (nt-volume over Files on the board)
    r.set_media(Some(("files", &s, true)), true, NOW);
    let tip = Notification {
        id: 1,
        title: "x".into(),
        ..Notification::default()
    };
    r.set_notes(&[("messages", &tip)], NOW, 0, true);
    assert_eq!(r.hud_rect(), None);
    r.show_volume(Some(6), false);
    common::frame(&mut r, &mut buf, w, "volume");
    common::write_png("nt-volume.png", &buf, w, h);
    let hud = r.hud_rect().expect("the HUD's bounds");
    assert!(
        hud.y as f32 >= 25.0 * d && (hud.h as f32) < 120.0 * d,
        "{hud:?}"
    );
    r.show_volume(None, false);
    common::frame(&mut r, &mut buf, w, "volume gone");
    assert_eq!(r.hud_rect(), None);
}

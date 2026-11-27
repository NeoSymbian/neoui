// SPDX-License-Identifier: Apache-2.0
//! The media session (top-panel TP3, protocol 1.22; docs neoui-shell/protocol.md "MEDIA"): what a player tells the
//! shell with `ipc::MEDIA` ([`Session`]), and the commands that come back as `ipc::AppEvent::Media` ([`cmd`]). One
//! session system-wide: the last app that sent Playing owns it (the shade's player, the island, the lock screen and
//! home's Media card all draw it). An empty record ends the caller's session.
//!
//! Record: `1:b, state:b, caps:b, modes:b, title:s48, artist:s48, album:s48, art:w, duration_ms:w, position_ms:w,
//! at:q`. `position_ms` was true at `at` (unix ms); while Playing the shell moves the bar on from there itself.

use crate::settings_app::wire::{R, W};
use alloc::string::String;
use alloc::vec::Vec;

/// [`Session::state`].
pub mod state {
    pub const STOPPED: u8 = 0;
    pub const PLAYING: u8 = 1;
    pub const PAUSED: u8 = 2;
    pub const BUFFERING: u8 = 3;
    pub(crate) const LAST: u8 = BUFFERING;
}

/// [`Session::caps`]: the controls the player answers (the shell draws only those).
pub mod caps {
    pub const PLAY: u8 = 1;
    pub const PAUSE: u8 = 2;
    pub const NEXT: u8 = 4;
    pub const PREV: u8 = 8;
    pub const SEEK: u8 = 16;
    pub const SHUFFLE: u8 = 32;
    pub const REPEAT: u8 = 64;
    pub const ALL: u8 = 127;
}

/// [`Session::modes`]: what shuffle and repeat are now.
pub mod modes {
    pub const SHUFFLE: u8 = 1;
    pub const REPEAT: u8 = 2;
    pub const ALL: u8 = 3;
}

/// `ipc::AppEvent::Media`'s `cmd`; `arg` is the position in ms for [`cmd::SEEK`], 0 off / 1 on for [`cmd::SHUFFLE`]
/// and [`cmd::REPEAT`], else 0.
pub mod cmd {
    pub const PLAY: u8 = 1;
    pub const PAUSE: u8 = 2;
    pub const NEXT: u8 = 3;
    pub const PREV: u8 = 4;
    pub const SEEK: u8 = 5;
    pub const SHUFFLE: u8 = 6;
    pub const REPEAT: u8 = 7;
    pub(crate) const LAST: u8 = REPEAT;
}

/// The longest title, artist or album, in bytes.
pub const TEXT_MAX: usize = 48;
/// The longest encoded [`Session`].
pub const MEDIA_MAX: usize = 4 + 3 * (1 + TEXT_MAX) + 4 + 4 + 4 + 8;

/// What is playing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    /// One of [`state`].
    pub state: u8,
    /// [`caps`] bits.
    pub caps: u8,
    /// [`modes`] bits.
    pub modes: u8,
    /// 1–48 bytes.
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The cover's `BLOB` id (TP2's reserved opcode 31); 0 = none ("No cover art").
    pub art: u32,
    /// 0 = unknown (a stream): no bar and no seek.
    pub duration_ms: u32,
    /// At most `duration_ms` when that is known.
    pub position_ms: u32,
    /// When `position_ms` was true, unix ms.
    pub at: u64,
}

impl Session {
    /// The record; strings are cut to 48 bytes on a character boundary.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.0.extend_from_slice(&[1, self.state, self.caps, self.modes]);
        for s in [&self.title, &self.artist, &self.album] {
            w.s(s, TEXT_MAX);
        }
        w.w(self.art);
        w.w(self.duration_ms);
        w.w(self.position_ms);
        w.q(self.at);
        w.0
    }

    /// `None` for a malformed record: another version, an unknown state, a reserved bit, an empty title, a long or
    /// non-UTF-8 string, a position past a known duration, a short record or trailing bytes. (The empty record, the
    /// end of the session, is the caller's to test first.)
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() > MEDIA_MAX {
            return None;
        }
        let mut r = R(b);
        if r.b()? != 1 {
            return None;
        }
        let s = Self {
            state: r.max(state::LAST)?,
            caps: r.max(caps::ALL)?,
            modes: r.max(modes::ALL)?,
            title: r.s(TEXT_MAX).filter(|t| !t.is_empty())?,
            artist: r.s(TEXT_MAX)?,
            album: r.s(TEXT_MAX)?,
            art: r.w()?,
            duration_ms: r.w()?,
            position_ms: r.w()?,
            at: r.q()?,
        };
        r.end()?;
        (s.duration_ms == 0 || s.position_ms <= s.duration_ms).then_some(s)
    }

    /// The position at `now` (unix ms): moving on from `at` while Playing, never past a known duration.
    #[must_use]
    pub fn position_at(&self, now: u64) -> u32 {
        if self.state != state::PLAYING {
            return self.position_ms;
        }
        let p = u64::from(self.position_ms).saturating_add(now.saturating_sub(self.at));
        let p = u32::try_from(p).unwrap_or(u32::MAX);
        if self.duration_ms == 0 {
            p
        } else {
            p.min(self.duration_ms)
        }
    }
}

/// Whether `(cmd, arg)` is a command the shell sends.
#[must_use]
pub fn valid_command(c: u8, arg: u32) -> bool {
    match c {
        cmd::SEEK => true,
        cmd::SHUFFLE | cmd::REPEAT => arg <= 1,
        1..=cmd::LAST => arg == 0,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Session {
        Session {
            state: state::PLAYING,
            caps: caps::PLAY | caps::PAUSE | caps::NEXT | caps::SEEK,
            modes: modes::REPEAT,
            title: "Bubble".into(),
            artist: "Frozen".into(),
            album: String::new(),
            art: 0,
            duration_ms: 180_000,
            position_ms: 1_000,
            at: 1_700_000_000_000,
        }
    }

    #[test]
    fn round_trips_and_cuts_long_text() {
        let s = song();
        assert_eq!(Session::decode(&s.encode()), Some(s));
        let mut long = song();
        long.title = "é".repeat(40); // 80 bytes: cut to 48, on a character boundary
        long.artist = "x".repeat(60);
        long.album = "y".repeat(48);
        let b = long.encode();
        assert_eq!(b.len(), MEDIA_MAX);
        let d = Session::decode(&b).unwrap();
        assert_eq!(d.title, "é".repeat(24));
        assert_eq!(d.artist.len(), 48);
    }

    #[test]
    fn malformed_records_are_refused() {
        let good = song().encode();
        for n in 0..good.len() {
            assert_eq!(Session::decode(&good[..n]), None, "{n}");
        }
        let mut long = good.clone();
        long.push(0);
        assert_eq!(Session::decode(&long), None);
        for (at, v) in [(0, 2), (1, 4), (2, 128), (3, 4)] {
            let mut b = good.clone();
            b[at] = v;
            assert_eq!(Session::decode(&b), None, "byte {at} = {v}");
        }
        // an empty title, bad UTF-8, a string over 48
        let mut t = song();
        t.title.clear();
        assert_eq!(Session::decode(&t.encode()), None);
        let mut b = good.clone();
        b[5] = 0xff;
        assert_eq!(Session::decode(&b), None);
        let mut b = good.clone();
        b[4] = 49;
        assert_eq!(Session::decode(&b), None);
        // past the end of a known duration; any position of a stream
        let mut p = song();
        p.position_ms = 180_001;
        assert_eq!(Session::decode(&p.encode()), None);
        p.duration_ms = 0;
        assert!(Session::decode(&p.encode()).is_some());
        assert_eq!(Session::decode(&[0u8; MEDIA_MAX + 1]), None);
    }

    #[test]
    fn the_bar_moves_only_while_playing() {
        let s = song();
        assert_eq!(s.position_at(s.at + 2_000), 3_000);
        assert_eq!(s.position_at(s.at - 5), 1_000, "a clock before `at`");
        assert_eq!(s.position_at(s.at + 10_000_000), 180_000);
        let paused = Session {
            state: state::PAUSED,
            ..song()
        };
        assert_eq!(paused.position_at(paused.at + 2_000), 1_000);
    }

    #[test]
    fn commands() {
        assert!(valid_command(cmd::SEEK, 90_000));
        assert!(valid_command(cmd::SHUFFLE, 1) && !valid_command(cmd::REPEAT, 2));
        assert!(valid_command(cmd::PLAY, 0) && !valid_command(cmd::PAUSE, 1));
        assert!(!valid_command(0, 0) && !valid_command(8, 0));
    }
}

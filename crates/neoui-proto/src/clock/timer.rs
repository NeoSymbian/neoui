// SPDX-License-Identifier: Apache-2.0
//! The countdown timer: a state machine on wall-clock time (Unix milliseconds), so a running timer keeps counting
//! while the Clock app is closed and across reboots. Saved as the `clock.timer` blob.

use super::wire::{Corrupt, Reader, Writer};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Shortest and longest settable durations, ms.
pub const MIN_MS: u32 = 1_000;
pub const MAX_MS: u32 = 24 * 3_600_000 - 1_000;
/// The duration a new timer starts with: 5 minutes.
pub const DEFAULT_MS: u32 = 5 * 60_000;
/// Durations offered as one-tap presets, ms.
pub const PRESETS_MS: [u32; 4] = [60_000, 5 * 60_000, 10 * 60_000, 30 * 60_000];
const BLOB_VERSION: u8 = 1;

/// Where the timer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Not started.
    Idle,
    /// Counting down until this Unix ms.
    Running { ends_ms: i64 },
    /// Stopped with this much left.
    Paused { left_ms: u32 },
    /// Reached zero (until reset).
    Done,
}

/// The countdown timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timer {
    /// The duration it was set to.
    pub set_ms: u32,
    pub phase: Phase,
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            set_ms: DEFAULT_MS,
            phase: Phase::Idle,
        }
    }
}

impl Timer {
    /// Time left at `now_ms`.
    #[must_use]
    pub fn left_ms(&self, now_ms: i64) -> u32 {
        match self.phase {
            Phase::Idle => self.set_ms,
            Phase::Running { ends_ms } => clamp_ms(ends_ms - now_ms),
            Phase::Paused { left_ms } => left_ms,
            Phase::Done => 0,
        }
    }

    /// Sets the duration (only while idle), clamped to [`MIN_MS`]..=[`MAX_MS`].
    pub fn set(&mut self, ms: i64) {
        if self.phase == Phase::Idle {
            self.set_ms = clamp_ms(ms).clamp(MIN_MS, MAX_MS);
        }
    }

    /// Moves the duration by `delta_ms` (only while idle).
    pub fn adjust(&mut self, delta_ms: i64) {
        self.set(i64::from(self.set_ms) + delta_ms);
    }

    /// Starts from idle, or resumes a paused timer.
    pub fn start(&mut self, now_ms: i64) {
        let left = match self.phase {
            Phase::Idle => self.set_ms,
            Phase::Paused { left_ms } => left_ms,
            Phase::Running { .. } | Phase::Done => return,
        };
        self.phase = Phase::Running {
            ends_ms: now_ms + i64::from(left),
        };
    }

    /// Pauses a running timer.
    pub fn pause(&mut self, now_ms: i64) {
        if let Phase::Running { .. } = self.phase {
            self.phase = Phase::Paused {
                left_ms: self.left_ms(now_ms),
            };
        }
    }

    /// Back to idle with the same duration.
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
    }

    /// Advances to [`Phase::Done`] when a running timer reached zero; true when it just did.
    pub fn tick(&mut self, now_ms: i64) -> bool {
        let finished = matches!(self.phase, Phase::Running { ends_ms } if now_ms >= ends_ms);
        if finished {
            self.phase = Phase::Done;
        }
        finished
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    /// The `clock.timer` blob.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(BLOB_VERSION);
        w.u32(self.set_ms);
        match self.phase {
            Phase::Idle => w.u8(0),
            Phase::Running { ends_ms } => w.u8(1).i64(ends_ms),
            Phase::Paused { left_ms } => w.u8(2).u32(left_ms),
            Phase::Done => w.u8(3),
        };
        w.finish()
    }

    /// Reads a `clock.timer` blob.
    ///
    /// # Errors
    /// [`Corrupt`] for a damaged blob or an out-of-range duration.
    pub fn decode(bytes: &[u8]) -> Result<Self, Corrupt> {
        let mut r = Reader::new(bytes, BLOB_VERSION)?;
        let set_ms = r.u32()?;
        let phase = match r.u8()? {
            0 => Phase::Idle,
            1 => Phase::Running { ends_ms: r.i64()? },
            2 => Phase::Paused { left_ms: r.u32()? },
            3 => Phase::Done,
            _ => return Err(Corrupt),
        };
        r.end()?;
        if !(MIN_MS..=MAX_MS).contains(&set_ms) {
            return Err(Corrupt);
        }
        Ok(Self { set_ms, phase })
    }
}

fn clamp_ms(ms: i64) -> u32 {
    // after the clamp it fits
    ms.clamp(0, i64::from(u32::MAX)) as u32
}

/// "4:59" or "1:02:03", rounded up to the second (a countdown shows 0:00 only when done).
#[must_use]
pub fn format_left(ms: u32) -> String {
    let s = ms.div_ceil(1000);
    let (h, m, s) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_794_256_200_000;

    #[test]
    fn counts_down_pauses_resumes_and_finishes() {
        let mut t = Timer::default();
        t.set(60_000);
        t.start(T0);
        assert_eq!(t.left_ms(T0 + 15_000), 45_000);
        t.pause(T0 + 15_000);
        assert_eq!(t.phase, Phase::Paused { left_ms: 45_000 });
        assert_eq!(t.left_ms(T0 + 99_000), 45_000, "a paused timer holds");
        t.start(T0 + 100_000);
        assert!(!t.tick(T0 + 144_999));
        assert!(t.tick(T0 + 145_000));
        assert_eq!(t.phase, Phase::Done);
        assert!(!t.tick(T0 + 200_000), "done once");
        t.reset();
        assert_eq!((t.phase, t.left_ms(T0)), (Phase::Idle, 60_000));
    }

    #[test]
    fn the_duration_changes_only_while_idle_and_within_bounds() {
        let mut t = Timer::default();
        t.adjust(-10 * 60_000);
        assert_eq!(t.set_ms, MIN_MS);
        t.set(i64::MAX);
        assert_eq!(t.set_ms, MAX_MS);
        t.set(90_000);
        t.start(T0);
        t.adjust(60_000);
        assert_eq!(t.set_ms, 90_000);
    }

    #[test]
    fn a_running_timer_survives_a_reboot_through_its_blob() {
        let mut t = Timer::default();
        t.start(T0);
        let back = Timer::decode(&t.encode()).expect("valid");
        // the app comes back 2 minutes later: still running, 3 minutes left
        assert_eq!(back.left_ms(T0 + 120_000), 180_000);
        // or after it would have ended
        let mut late = back;
        assert!(late.tick(T0 + 10 * 60_000));
    }

    #[test]
    fn every_phase_round_trips_and_damage_is_refused() {
        for phase in [
            Phase::Idle,
            Phase::Running { ends_ms: T0 },
            Phase::Paused { left_ms: 1234 },
            Phase::Done,
        ] {
            let t = Timer {
                set_ms: 90_000,
                phase,
            };
            assert_eq!(Timer::decode(&t.encode()), Ok(t));
        }
        let mut b = Timer::default().encode();
        b[5] = 9;
        assert_eq!(Timer::decode(&b), Err(Corrupt));
        let zero = Timer {
            set_ms: 0,
            phase: Phase::Idle,
        }
        .encode();
        assert_eq!(Timer::decode(&zero), Err(Corrupt));
    }

    #[test]
    fn time_left_is_shown_rounded_up() {
        assert_eq!(format_left(300_000), "5:00");
        assert_eq!(format_left(299_001), "5:00");
        assert_eq!(format_left(1), "0:01");
        assert_eq!(format_left(0), "0:00");
        assert_eq!(format_left(3_723_000), "1:02:03");
    }
}

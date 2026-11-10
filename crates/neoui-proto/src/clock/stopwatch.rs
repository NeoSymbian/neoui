// SPDX-License-Identifier: Apache-2.0
//! The stopwatch and its laps, on wall-clock time (Unix milliseconds) like the timer, so it keeps running while
//! the Clock app is closed. Saved as the `clock.stopwatch` blob.

use super::wire::{Corrupt, Reader, Writer};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Most laps kept; a lap beyond this is refused.
pub const LAPS_MAX: usize = 20;
const BLOB_VERSION: u8 = 1;

/// The stopwatch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stopwatch {
    /// Time counted before the current run, ms.
    banked_ms: u64,
    /// When the current run started (Unix ms), if running.
    since_ms: Option<i64>,
    /// The elapsed total at each lap, ms, oldest first.
    laps: Vec<u64>,
}

/// No room for another lap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LapsFull;

impl Stopwatch {
    /// Total elapsed at `now_ms`.
    #[must_use]
    pub fn elapsed_ms(&self, now_ms: i64) -> u64 {
        // a clock set backwards must not run the total backwards
        let running = self
            .since_ms
            .map_or(0, |s| (now_ms - s).max(0).unsigned_abs());
        self.banked_ms + running
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.since_ms.is_some()
    }

    pub fn start(&mut self, now_ms: i64) {
        if self.since_ms.is_none() {
            self.since_ms = Some(now_ms);
        }
    }

    pub fn stop(&mut self, now_ms: i64) {
        if self.since_ms.is_some() {
            self.banked_ms = self.elapsed_ms(now_ms);
            self.since_ms = None;
        }
    }

    /// Records a lap at the current total (running only).
    ///
    /// # Errors
    /// [`LapsFull`] at [`LAPS_MAX`] laps.
    pub fn lap(&mut self, now_ms: i64) -> Result<(), LapsFull> {
        if self.since_ms.is_none() {
            return Ok(());
        }
        if self.laps.len() >= LAPS_MAX {
            return Err(LapsFull);
        }
        self.laps.push(self.elapsed_ms(now_ms));
        Ok(())
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Laps newest first: (number from 1, the lap's own time, the total at the lap), ms.
    pub fn laps(&self) -> impl Iterator<Item = (usize, u64, u64)> + '_ {
        self.laps.iter().enumerate().rev().map(|(i, &total)| {
            let before = if i == 0 { 0 } else { self.laps[i - 1] };
            (i + 1, total - before, total)
        })
    }

    /// The `clock.stopwatch` blob.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(BLOB_VERSION);
        // u64 fits i64 for any real duration (millions of years)
        w.i64(self.banked_ms as i64);
        match self.since_ms {
            Some(s) => w.u8(1).i64(s),
            None => w.u8(0),
        };
        // at most LAPS_MAX
        w.u8(self.laps.len() as u8);
        for &l in &self.laps {
            w.i64(l as i64);
        }
        w.finish()
    }

    /// Reads a `clock.stopwatch` blob.
    ///
    /// # Errors
    /// [`Corrupt`] for a damaged blob, negative times, laps out of order or more than [`LAPS_MAX`].
    pub fn decode(bytes: &[u8]) -> Result<Self, Corrupt> {
        let mut r = Reader::new(bytes, BLOB_VERSION)?;
        let banked_ms = u64::try_from(r.i64()?).map_err(|_| Corrupt)?;
        let since_ms = match r.u8()? {
            0 => None,
            1 => Some(r.i64()?),
            _ => return Err(Corrupt),
        };
        let n = usize::from(r.u8()?);
        if n > LAPS_MAX {
            return Err(Corrupt);
        }
        let mut laps = Vec::with_capacity(n);
        for _ in 0..n {
            let l = u64::try_from(r.i64()?).map_err(|_| Corrupt)?;
            if laps.last().is_some_and(|&p| l < p) {
                return Err(Corrupt);
            }
            laps.push(l);
        }
        r.end()?;
        Ok(Self {
            banked_ms,
            since_ms,
            laps,
        })
    }
}

/// "02:05.37" (minutes, seconds, hundredths) or "1:02:05.37" past an hour.
#[must_use]
pub fn format_elapsed(ms: u64) -> String {
    let (cs, s) = (ms / 10 % 100, ms / 1000);
    let (h, m, s) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{cs:02}")
    } else {
        format!("{m:02}:{s:02}.{cs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const T0: i64 = 1_794_256_200_000;

    #[test]
    fn runs_stops_and_resumes() {
        let mut w = Stopwatch::default();
        assert_eq!(w.elapsed_ms(T0), 0);
        w.start(T0);
        w.start(T0 + 500); // already running: no restart
        assert_eq!(w.elapsed_ms(T0 + 1_500), 1_500);
        w.stop(T0 + 2_000);
        assert_eq!(w.elapsed_ms(T0 + 9_000), 2_000);
        w.start(T0 + 10_000);
        assert_eq!(w.elapsed_ms(T0 + 10_250), 2_250);
        assert_eq!(
            w.elapsed_ms(T0),
            2_000,
            "a clock set back does not run it backwards"
        );
        w.reset();
        assert_eq!((w.elapsed_ms(T0), w.is_running()), (0, false));
    }

    #[test]
    fn laps_list_newest_first_with_splits() {
        let mut w = Stopwatch::default();
        w.lap(T0).expect("stopped: ignored");
        w.start(T0);
        w.lap(T0 + 1_000).expect("room");
        w.lap(T0 + 3_500).expect("room");
        let laps: Vec<_> = w.laps().collect();
        assert_eq!(laps, vec![(2, 2_500, 3_500), (1, 1_000, 1_000)]);
        for i in 2..LAPS_MAX {
            w.lap(T0 + 4_000 + i as i64).expect("room");
        }
        assert_eq!(w.lap(T0 + 9_000), Err(LapsFull));
    }

    #[test]
    fn a_running_stopwatch_survives_a_reboot_through_its_blob() {
        let mut w = Stopwatch::default();
        w.start(T0);
        w.lap(T0 + 1_000).expect("room");
        let back = Stopwatch::decode(&w.encode()).expect("valid");
        assert_eq!(back, w);
        assert_eq!(back.elapsed_ms(T0 + 60_000), 60_000);
    }

    #[test]
    fn damaged_blobs_are_refused() {
        let mut w = Stopwatch::default();
        w.start(T0);
        w.lap(T0 + 2_000).expect("room");
        w.lap(T0 + 3_000).expect("room");
        let b = w.encode();
        assert_eq!(Stopwatch::decode(&b[..b.len() - 1]), Err(Corrupt));
        let mut out_of_order = b.clone();
        let last = out_of_order.len() - 8;
        out_of_order[last..].copy_from_slice(&1i64.to_le_bytes());
        assert_eq!(Stopwatch::decode(&out_of_order), Err(Corrupt));
        let mut negative = b;
        negative[1..9].copy_from_slice(&(-1i64).to_le_bytes());
        assert_eq!(Stopwatch::decode(&negative), Err(Corrupt));
    }

    #[test]
    fn elapsed_time_is_shown_to_the_hundredth() {
        assert_eq!(format_elapsed(0), "00:00.00");
        assert_eq!(format_elapsed(125_370), "02:05.37");
        assert_eq!(format_elapsed(3_725_370), "1:02:05.37");
    }
}

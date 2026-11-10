// SPDX-License-Identifier: Apache-2.0
//! Numbers over time and as text: a sample history for the Home page's bar graphs, CPU load from thread CPU times,
//! and bytes, durations and percentages formatted for the screen.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The last `cap` samples, each 0.0..=1.0 (a fraction of something: CPU busy, RAM used).
#[derive(Clone, Debug)]
pub struct History {
    samples: VecDeque<f32>,
    cap: usize,
}

impl History {
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(cap),
            cap: cap.max(1),
        }
    }

    /// Adds a sample, clamped to 0..=1 (NaN counts as 0); the oldest goes when full.
    pub fn push(&mut self, v: f32) {
        if self.samples.len() == self.cap {
            self.samples.pop_front();
        }
        self.samples
            .push_back(if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) });
    }

    #[must_use]
    pub fn last(&self) -> Option<f32> {
        self.samples.back().copied()
    }

    /// Exactly `cap` bar heights, oldest first, zeros in front while the history fills.
    #[must_use]
    pub fn bars(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.cap);
        out.resize(self.cap - self.samples.len(), 0.0);
        out.extend(self.samples.iter().copied());
        out
    }
}

/// CPU time used by every thread but the idle ("null") ones, and when it was read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuSample {
    pub busy_us: u64,
    pub wall_us: u64,
}

/// The share of `cpus` CPUs that was busy between two samples, 0..=1; None without wall time between them, or when
/// the busy time went backwards (a thread exited and took its CPU time with it).
#[must_use]
pub fn cpu_load(prev: CpuSample, now: CpuSample, cpus: u32) -> Option<f32> {
    let wall = now.wall_us.checked_sub(prev.wall_us)?;
    let busy = now.busy_us.checked_sub(prev.busy_us)?;
    if wall == 0 || cpus == 0 {
        return None;
    }
    // ratios of microsecond counts: f32 is plenty
    Some((busy as f32 / (wall as f32 * cpus as f32)).clamp(0.0, 1.0))
}

/// "512 B", "1.5 KB", "34 MB", "1.2 GB" (powers of 1024).
#[must_use]
pub fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if v < 10.0 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.0} {}", UNITS[unit])
    }
}

/// Time since boot: "42 s", "5:06", "4:05:06", "3 d 4:05".
#[must_use]
pub fn format_uptime(ms: u64) -> String {
    let s = ms / 1000;
    let (d, h, m, s) = (s / 86_400, s / 3600 % 24, s / 60 % 60, s % 60);
    match (d, h, m) {
        (0, 0, 0) => format!("{s} s"),
        (0, 0, _) => format!("{m}:{s:02}"),
        (0, _, _) => format!("{h}:{m:02}:{s:02}"),
        _ => format!("{d} d {h}:{m:02}"),
    }
}

/// CPU time: "0.25 s", "12.3 s", "4:05", "2:04:05".
#[must_use]
pub fn format_cpu_time(us: u64) -> String {
    let ms = us / 1000;
    if ms < 10_000 {
        format!("{}.{:02} s", ms / 1000, ms % 1000 / 10)
    } else if ms < 60_000 {
        format!("{}.{} s", ms / 1000, ms % 1000 / 100)
    } else {
        format_uptime(ms)
    }
}

/// "0%", "12%", "100%".
#[must_use]
pub fn format_percent(fraction: f32) -> String {
    // 0..=100 after the clamp
    format!("{}%", (fraction.clamp(0.0, 1.0) * 100.0 + 0.5) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_padded() {
        let mut h = History::new(4);
        assert_eq!(h.bars(), [0.0; 4]);
        h.push(0.5);
        h.push(2.0);
        h.push(f32::NAN);
        assert_eq!(h.bars(), [0.0, 0.5, 1.0, 0.0]);
        h.push(0.25);
        h.push(-1.0);
        assert_eq!(h.bars(), [1.0, 0.0, 0.25, 0.0]);
        assert_eq!(h.last(), Some(0.0));
    }

    #[test]
    fn cpu_load_from_two_samples() {
        let a = CpuSample {
            busy_us: 1_000,
            wall_us: 10_000,
        };
        let b = CpuSample {
            busy_us: 6_000,
            wall_us: 20_000,
        };
        assert_eq!(cpu_load(a, b, 1), Some(0.5));
        assert_eq!(cpu_load(a, b, 2), Some(0.25));
        assert_eq!(cpu_load(a, a, 1), None, "no time passed");
        assert_eq!(cpu_load(b, a, 1), None, "went backwards");
        assert_eq!(cpu_load(a, b, 0), None);
    }

    #[test]
    fn numbers_as_text() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(34 << 20), "34 MB");
        assert_eq!(format_bytes(3 << 30), "3.0 GB");
        assert_eq!(format_uptime(42_000), "42 s");
        assert_eq!(format_uptime(306_000), "5:06");
        assert_eq!(format_uptime(14_706_000), "4:05:06");
        assert_eq!(format_uptime(273_906_000), "3 d 4:05");
        assert_eq!(format_cpu_time(250_000), "0.25 s");
        assert_eq!(format_cpu_time(12_345_000), "12.3 s");
        assert_eq!(format_cpu_time(245_000_000), "4:05");
        assert_eq!(format_percent(0.123), "12%");
        assert_eq!(format_percent(1.5), "100%");
    }
}

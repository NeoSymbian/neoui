// SPDX-License-Identifier: Apache-2.0
//! Read-only battery HAL observations and bounded, checksummed history.
use alloc::format;
use alloc::{collections::VecDeque, string::String, vec::Vec};

pub mod cycle;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub valid: u32,
    pub present: u32,
    pub percent: u32,
    pub voltage_uv: u32,
    pub usb_present: u32,
    pub usb_online: u32,
    pub charge: u32,
}
impl Snapshot {
    pub const SIZE: usize = 32;
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::SIZE {
            return None;
        }
        let mut w = [0u32; 8];
        for (dst, src) in w.iter_mut().zip(bytes.as_chunks::<4>().0.iter()) {
            *dst = u32::from_le_bytes(*src);
        }
        if w[0] != 1
            || w[1] & !63 != 0
            || (w[1] & 1 != 0 && w[2] > 1)
            || (w[1] & 2 != 0 && w[3] > 100)
            || (w[1] & 8 != 0 && w[5] > 1)
            || (w[1] & 16 != 0 && w[6] > 1)
            || (w[1] & 32 != 0 && w[7] > 4)
        {
            return None;
        }
        Some(Self {
            valid: w[1],
            present: w[2],
            percent: w[3],
            voltage_uv: w[4],
            usb_present: w[5],
            usb_online: w[6],
            charge: w[7],
        })
    }
    pub fn percent(&self) -> Option<u32> {
        (self.valid & 2 != 0).then_some(self.percent)
    }
    pub fn percent_text(&self) -> String {
        self.percent()
            .map_or_else(|| "--%".into(), |p| format!("{p}%"))
    }
    pub fn voltage_text(&self) -> String {
        if self.valid & 4 != 0 {
            format!(
                "{}.{:03} V",
                self.voltage_uv / 1_000_000,
                (self.voltage_uv % 1_000_000) / 1000
            )
        } else {
            "unknown".into()
        }
    }
    pub fn charge_text(&self) -> &'static str {
        if self.valid & 32 == 0 {
            return "Unknown";
        }
        match self.charge {
            1 => "Discharging",
            2 => "Charging",
            3 => "Not charging",
            4 => "Full",
            _ => "Unknown",
        }
    }
    pub fn charging(&self) -> bool {
        self.valid & 32 != 0 && self.charge == 2
    }
    pub fn usb_connected(&self) -> bool {
        self.valid & 8 != 0 && self.usb_present == 1
    }
    pub fn full_on_usb(&self) -> bool {
        self.usb_connected() && self.valid & 48 == 48 && self.usb_online == 1 && self.charge == 4
    }
    pub fn usb_text(&self) -> &'static str {
        if self.valid & 8 == 0 {
            "unknown"
        } else if self.usb_present != 0 {
            "Connected"
        } else {
            "Disconnected"
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub utc: i64,
    pub observation: Snapshot,
    pub gap: bool,
}
#[derive(Clone, Debug, Default)]
pub struct History {
    pub generation: u64,
    points: VecDeque<Point>,
    gap: bool,
}
const MAX: usize = 1440;
const HEADER: usize = 24;
const RECORD: usize = 24;
pub const FILE_MAX: usize = HEADER + MAX * RECORD + 4;
impl History {
    /// Selects a valid slot; callers pass None for missing, unreadable or corrupt files.
    pub fn restore(a: Option<Self>, b: Option<Self>) -> Self {
        let mut h = match (a, b) {
            (Some(a), Some(b)) => {
                if a.generation >= b.generation {
                    a
                } else {
                    b
                }
            }
            (Some(h), None) | (None, Some(h)) => h,
            (None, None) => Self::default(),
        };
        h.begin_segment();
        h
    }

    pub fn begin_segment(&mut self) {
        self.gap = !self.points.is_empty();
    }
    pub fn points(&self) -> &VecDeque<Point> {
        &self.points
    }
    pub fn observe(&mut self, utc: i64, observation: Snapshot) {
        let jump = self
            .points
            .back()
            .is_some_and(|p| utc < p.utc || utc.saturating_sub(p.utc) > 90);
        self.points
            .retain(|p| p.utc <= utc && p.utc >= utc.saturating_sub(86400));
        let point = Point {
            utc,
            observation,
            gap: self.gap || jump,
        };
        if let Some(last) = self.points.back_mut()
            && !point.gap
            && last.utc / 60 == utc / 60
        {
            last.utc = utc;
            last.observation = observation;
            return;
        }
        self.points.push_back(point);
        self.gap = false;
        while self.points.len() > MAX {
            self.points.pop_front();
        }
    }
    /// Minute columns; -1 is missing or an explicit segment break, never a fabricated zero.
    pub fn bars(&self, now: i64) -> Vec<f32> {
        let mut out = alloc::vec![-1.0;1440];
        for p in &self.points {
            let age = now.saturating_sub(p.utc);
            if !(0..86400).contains(&age) {
                continue;
            }
            let index = 1439 - (age / 60) as usize;
            out[index] = if p.gap {
                -1.0
            } else {
                p.observation.percent().map_or(-1.0, |n| n as f32 / 100.0)
            };
        }
        out
    }
    pub fn encode(&self, generation: u64) -> Vec<u8> {
        let mut b = Vec::with_capacity(FILE_MAX);
        b.extend_from_slice(b"NBAT");
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&generation.to_le_bytes());
        b.extend_from_slice(&(self.points.len() as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        for p in &self.points {
            b.extend_from_slice(&p.utc.to_le_bytes());
            b.extend_from_slice(
                &((p.observation.valid & 38) | if p.gap { 1 << 31 } else { 0 }).to_le_bytes(),
            );
            b.extend_from_slice(&p.observation.percent.to_le_bytes());
            b.extend_from_slice(&p.observation.voltage_uv.to_le_bytes());
            b.extend_from_slice(&p.observation.charge.to_le_bytes());
        }
        let crc = checksum(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER + 4 || b.len() > FILE_MAX || &b[..4] != b"NBAT" {
            return None;
        }
        let word = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        if word(4) != 1 || word(20) != 0 {
            return None;
        }
        let count = word(16) as usize;
        if count > MAX
            || b.len() != HEADER + count * RECORD + 4
            || checksum(&b[..b.len() - 4]) != word(b.len() - 4)
        {
            return None;
        }
        let mut h = Self {
            generation: u64::from_le_bytes(b[8..16].try_into().ok()?),
            ..Self::default()
        };
        for r in b[HEADER..b.len() - 4].as_chunks::<RECORD>().0 {
            let n = |at| u32::from_le_bytes(r[at..at + 4].try_into().unwrap());
            let flags = n(8);
            if flags & !(38 | (1 << 31)) != 0
                || (flags & 2 != 0 && n(12) > 100)
                || (flags & 32 != 0 && n(20) > 4)
            {
                return None;
            }
            h.points.push_back(Point {
                utc: i64::from_le_bytes(r[..8].try_into().ok()?),
                gap: flags & (1 << 31) != 0,
                observation: Snapshot {
                    valid: flags & 38,
                    percent: n(12),
                    voltage_uv: n(16),
                    charge: n(20),
                    ..Snapshot::default()
                },
            });
        }
        h.begin_segment();
        Some(h)
    }
}
fn checksum(b: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in b {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32.wrapping_mul(crc & 1));
        }
    }
    !crc
}
/// Battery usage (Settings SE3a, `set2-battery-usage`) from what the history really holds: the level and the charge
/// state once a minute. No per-app shares (D4) and no screen-on time (nothing records it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// The window starts at the last full charge the phone then ran on battery from; else at the history's first
    /// reading.
    pub since_full: bool,
    pub start_utc: i64,
    pub end_utc: i64,
    /// The level at each end of the window, if read.
    pub start_percent: Option<u8>,
    pub end_percent: Option<u8>,
    /// Seconds spent discharging between readings in the window.
    pub on_battery_s: u32,
    /// Seconds without readings in the window (the phone off, a broken history): not counted.
    pub gap_s: u32,
}

impl Snapshot {
    /// Full: the charger's state when it was read (Discharging at 100 % isn't full), else the level.
    fn is_full(&self) -> bool {
        if self.valid & 32 != 0 {
            self.charge == 4
        } else {
            self.percent() == Some(100)
        }
    }
    /// A reading worth counting: the level or the charge state (a failed HAL read records neither).
    fn read(&self) -> bool {
        self.valid & (2 | 32) != 0
    }
    /// Running on the battery (the charge state says Discharging; the history doesn't keep USB presence).
    fn discharging(&self) -> bool {
        self.valid & 32 != 0 && self.charge == 1
    }
}

impl History {
    /// Battery usage over the history, or `None` before its first reading ("No data yet").
    #[must_use]
    pub fn usage(&self) -> Option<Usage> {
        // the last full charge the phone then ran down from (still full or charging on the cable: no window yet)
        let unplugged = self
            .points
            .iter()
            .rposition(|p| p.observation.discharging());
        let full = unplugged.and_then(|d| {
            self.points
                .range(..d)
                .rposition(|p| p.observation.is_full())
        });
        let start = full.or_else(|| self.points.iter().position(|p| p.observation.read()))?;
        let window = self.points.range(start..);
        let first = window.clone().next()?;
        let last = self.points.back()?;
        let level = |p: &Point| p.observation.percent().map(|n| n.min(100) as u8);
        let mut u = Usage {
            since_full: full.is_some(),
            start_utc: first.utc,
            end_utc: last.utc,
            start_percent: level(first),
            end_percent: window.clone().rev().find_map(level),
            ..Usage::default()
        };
        let mut previous = first;
        for p in window.skip(1) {
            let dt = u32::try_from(p.utc.saturating_sub(previous.utc)).unwrap_or(u32::MAX);
            // a break in the history, or a minute without a reading on either side: not observed
            if p.gap || !p.observation.read() || !previous.observation.read() {
                u.gap_s = u.gap_s.saturating_add(dt);
            } else if previous.observation.discharging() {
                u.on_battery_s = u.on_battery_s.saturating_add(dt);
            }
            previous = p;
        }
        Some(u)
    }
}

/// What Battery usage shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageText {
    /// "Since full charge" or "Last 24 hours".
    pub title: String,
    /// "From 100% to 72% over 9 h 40 min" (the font has no arrow).
    pub levels: String,
    /// "On battery 8 h 10 min".
    pub on_battery: String,
    /// "Off or not recorded for 6 h, not counted"; empty without gaps.
    pub gaps: String,
}

impl Usage {
    #[must_use]
    pub fn text(&self) -> UsageText {
        let pct = |p: Option<u8>| p.map_or_else(|| "--%".into(), |n| format!("{n}%"));
        let span =
            u32::try_from(self.end_utc.saturating_sub(self.start_utc).max(0)).unwrap_or(u32::MAX);
        UsageText {
            title: if self.since_full {
                "Since full charge"
            } else {
                "Last 24 hours (no full charge seen)"
            }
            .into(),
            levels: format!(
                "From {} to {} over {}",
                pct(self.start_percent),
                pct(self.end_percent),
                duration(span)
            ),
            on_battery: format!("On battery {}", duration(self.on_battery_s)),
            gaps: if self.gap_s == 0 {
                String::new()
            } else {
                format!(
                    "Off or not recorded for {}, not counted",
                    duration(self.gap_s)
                )
            },
        }
    }
}

/// "9 h 40 min", "40 min", "0 min".
fn duration(s: u32) -> String {
    let (h, m) = (s / 3600, s % 3600 / 60);
    match (h, m) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    fn at(percent: u32, charge: u32) -> Snapshot {
        Snapshot {
            valid: 2 | 32,
            percent,
            charge,
            ..Snapshot::default()
        }
    }

    #[test]
    fn no_data_yet() {
        assert_eq!(History::default().usage(), None);
    }

    #[test]
    fn since_the_last_full_charge_with_a_gap() {
        let mut h = History::default();
        h.observe(0, at(80, 2)); // charging
        h.observe(60, at(100, 4)); // full
        h.observe(120, at(99, 1));
        h.observe(180, at(98, 1));
        h.begin_segment(); // the phone was off
        h.observe(180 + 6 * 3600, at(90, 1));
        h.observe(180 + 6 * 3600 + 60, at(89, 1));
        let u = h.usage().unwrap();
        assert!(u.since_full);
        assert_eq!((u.start_utc, u.end_utc), (60, 180 + 6 * 3600 + 60));
        assert_eq!((u.start_percent, u.end_percent), (Some(100), Some(89)));
        // 60 → 120 starts full (not discharging), 120 → 180 and the last minute count; the 6 h gap doesn't
        assert_eq!(u.on_battery_s, 120);
        assert_eq!(u.gap_s, 6 * 3600);
        let t = u.text();
        assert_eq!(t.title, "Since full charge");
        assert_eq!(t.levels, "From 100% to 89% over 6 h 3 min");
        assert_eq!(t.on_battery, "On battery 2 min");
        assert_eq!(t.gaps, "Off or not recorded for 6 h, not counted");
    }

    #[test]
    fn without_a_full_charge_the_whole_history() {
        let mut h = History::default();
        h.observe(0, at(60, 1));
        h.observe(60, at(59, 3)); // plugged in, not charging: not on battery after this
        h.observe(120, at(59, 3));
        h.observe(180, Snapshot::default()); // a failed read: not observed
        let u = h.usage().unwrap();
        assert!(!u.since_full);
        assert_eq!((u.start_percent, u.end_percent), (Some(60), Some(59)));
        assert_eq!((u.on_battery_s, u.gap_s), (60, 60));
        let t = u.text();
        assert_eq!(t.title, "Last 24 hours (no full charge seen)");
        assert_eq!(t.levels, "From 60% to 59% over 3 min");
        assert_eq!(t.gaps, "Off or not recorded for 1 min, not counted");
    }

    #[test]
    fn full_on_the_cable_now_keeps_the_last_run_down() {
        let mut h = History::default();
        h.observe(0, at(100, 4)); // full
        h.observe(60, at(99, 1)); // unplugged
        h.observe(120, at(98, 1));
        h.observe(180, at(98, 2)); // plugged in again
        h.observe(240, at(100, 4)); // full on the cable now
        let u = h.usage().unwrap();
        assert!(u.since_full);
        assert_eq!((u.start_utc, u.end_utc), (0, 240));
        assert_eq!(u.on_battery_s, 120);
        // never unplugged since the history began: the whole history, not a window from now
        let mut h = History::default();
        h.observe(0, at(90, 2));
        h.observe(60, at(100, 4));
        let u = h.usage().unwrap();
        assert!(!u.since_full);
        assert_eq!((u.start_utc, u.on_battery_s), (0, 0));
    }

    #[test]
    fn discharging_at_100_isnt_full_and_failed_reads_are_no_data() {
        let mut h = History::default();
        h.observe(0, at(100, 4)); // full
        for m in 1..=3 {
            h.observe(m * 60, at(100, 1)); // unplugged, still 100 %
        }
        h.observe(240, at(99, 1));
        let u = h.usage().unwrap();
        assert_eq!(u.start_utc, 0);
        assert_eq!(u.on_battery_s, 180); // minutes 1 to 4
        // only failed reads: nothing to show
        let mut h = History::default();
        h.observe(0, Snapshot::default());
        h.observe(60, Snapshot::default());
        assert_eq!(h.usage(), None);
        // a damaged record's reversed window shows no time, not a huge one
        let u = Usage {
            start_utc: 60,
            end_utc: 0,
            ..Usage::default()
        };
        assert_eq!(u.text().levels, "From --% to --% over 0 min");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(p: u32) -> Snapshot {
        Snapshot {
            valid: 2,
            percent: p,
            ..Snapshot::default()
        }
    }
    #[test]
    fn unknown_and_zero_are_distinct() {
        assert_eq!(Snapshot::default().percent(), None);
        assert_eq!(sample(0).percent(), Some(0));
    }
    #[test]
    fn wire_rejects_bad_version_flags_and_percent() {
        let mut b = [0; 32];
        b[0] = 1;
        assert!(Snapshot::decode(&b).is_some());
        b[4] = 2;
        b[12] = 101;
        assert!(Snapshot::decode(&b).is_none());
        b[12] = 100;
        assert_eq!(Snapshot::decode(&b).unwrap().percent(), Some(100));
        b[0] = 2;
        assert!(Snapshot::decode(&b).is_none());
    }
    #[test]
    fn round_trip_and_torn_files() {
        let mut h = History::default();
        h.observe(1000, sample(50));
        let b = h.encode(12);
        assert_eq!(History::decode(&b).unwrap().generation, 12);
        for n in 0..b.len() {
            assert!(History::decode(&b[..n]).is_none());
        }
        let mut corrupt = b.clone();
        corrupt[25] ^= 1;
        assert!(History::decode(&corrupt).is_none());
    }
    #[test]
    fn bounded_real_history_and_clock_change() {
        let mut h = History::default();
        for i in 0..1500 {
            h.observe(i * 60, sample(20));
        }
        assert_eq!(h.points().len(), 1440);
        h.observe(10, sample(0));
        assert_eq!(h.points().len(), 1);
        assert!(h.points()[0].gap);
        assert_eq!(h.points()[0].observation.percent(), Some(0));
        assert_eq!(h.bars(10)[1439], -1.0);
        assert_eq!(h.bars(10)[0], -1.0);
    }
    #[test]
    fn unavailable_is_gap_and_reboot_is_segment() {
        let mut h = History::default();
        h.observe(0, sample(50));
        h.observe(60, Snapshot::default());
        assert_eq!(h.bars(60)[1439], -1.0);
        let mut restored = History::decode(&h.encode(1)).unwrap();
        restored.observe(65, sample(49));
        assert!(restored.points().back().unwrap().gap);
    }
}

#[cfg(test)]
mod gap_tests {
    use super::*;
    #[test]
    fn minute_gap_and_restored_segment_are_visible() {
        let mut h = History::default();
        h.observe(
            0,
            Snapshot {
                valid: 2,
                percent: 50,
                ..Snapshot::default()
            },
        );
        h.observe(60, Snapshot::default());
        h.observe(
            120,
            Snapshot {
                valid: 2,
                percent: 49,
                ..Snapshot::default()
            },
        );
        let bars = h.bars(120);
        assert_eq!(bars[1437], 0.5);
        assert_eq!(bars[1438], -1.0);
        assert_eq!(bars[1439], 0.49);
        let mut h = History::decode(&h.encode(1)).unwrap();
        h.observe(
            180,
            Snapshot {
                valid: 2,
                percent: 48,
                ..Snapshot::default()
            },
        );
        assert_eq!(h.bars(180)[1439], -1.0);
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    #[test]
    fn newest_valid_slot_and_failed_newer_write_recover() {
        let h = History::default();
        let old = h.encode(1);
        let new = h.encode(2);
        assert_eq!(
            History::restore(History::decode(&old), History::decode(&new)).generation,
            2
        );
        assert_eq!(
            History::restore(
                History::decode(&old),
                History::decode(&new[..new.len() - 1])
            )
            .generation,
            1
        );
        assert_eq!(History::restore(None, History::decode(&new)).generation, 2);
        assert!(History::restore(None, None).points().is_empty());
    }
}

/// Distinguish a wall-clock correction from normal elapsed time (UTC has second precision).
pub fn clock_discontinuity(previous: (i64, u64), current: (i64, u64)) -> bool {
    let Some(elapsed) = current.1.checked_sub(previous.1) else {
        return true;
    };
    let utc_elapsed = (i128::from(current.0) - i128::from(previous.0)) * 1000;
    (utc_elapsed - i128::from(elapsed)).abs() > 2000
}
#[cfg(test)]
mod clock_tests {
    use super::*;
    #[test]
    fn forward_clock_step_breaks_but_normal_second_quantization_does_not() {
        assert!(clock_discontinuity((100, 0), (165, 5000)));
        assert!(clock_discontinuity((100, 0), (99, 5000)));
        assert!(!clock_discontinuity((100, 0), (105, 5000)));
        assert!(!clock_discontinuity((100, 0), (106, 5000)));
        assert!(clock_discontinuity((100, 100), (101, 0)));
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;
    #[test]
    fn full_and_connected_indicators_require_valid_usable_usb() {
        let mut s = Snapshot {
            valid: 56,
            usb_present: 1,
            usb_online: 1,
            charge: 4,
            ..Snapshot::default()
        };
        assert!(s.usb_connected() && s.full_on_usb());
        s.valid &= !16;
        assert!(s.usb_connected());
        assert!(!s.full_on_usb());
        s.valid = 56;
        s.usb_online = 0;
        assert!(!s.full_on_usb());
        s.usb_present = 0;
        assert!(!s.usb_connected());
        s.valid = 0;
        s.usb_present = 1;
        s.usb_online = 1;
        s.charge = 2;
        assert!(!s.usb_connected() && !s.full_on_usb() && !s.charging());
    }
}

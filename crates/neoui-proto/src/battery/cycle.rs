// SPDX-License-Identifier: Apache-2.0
//! The charge cycle (Settings D5, docs neoui-shell/settings-data.md): on-battery and screen-on time since the last
//! full charge, past the 24-hour history. SYSSTART feeds a [`Recorder`] its five-second battery observations and the
//! display HAL's acknowledged on/off transitions, saves [`Checkpoint`]s in alternating files, and publishes
//! [`Recorder::record`] as INFO 9 (`wire::BatteryCycle`); NEOSET shows it with [`text`].
//!
//! Only what was seen counts: discharge between two valid, discharging observations at most [`MAX_STEP_MS`] apart,
//! screen-on time only while the display HAL said on in such an interval. Everything uncertain (a missing or
//! unreadable sample, a power edge between two samples, a display read that failed or disagreed, the time the shell
//! wasn't running) is a gap, counted once. Durations come from the widened tick counter ([`widen`]), never the wall
//! clock; a wall-clock correction moves the displayed start with it.
//!
//! A cycle starts at a full charge on USB ([`Snapshot::full_on_usb`]) followed by discharge, once: repeated full
//! readings while plugged in only keep the anchor, a partial recharge pauses the count, and a 100% reading while
//! discharging arms nothing. Before that the window is the time observed since the first valid reading.

use super::{Snapshot, checksum, duration};
use crate::settings_app::wire::{BatteryCycle, cycle as bits};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The longest step between two observations that still counts (two five-second samples); longer is a gap.
pub const MAX_STEP_MS: u64 = 10_000;
/// An off interval longer than this (the RTC or the clock reset) has no believable duration.
const PLAUSIBLE_OFF_S: u64 = 366 * 86_400;

/// The tick counter widened past its 32-bit wrap: `prev` is the last widened value, `raw` the counter now.
#[must_use]
pub fn widen(prev: u64, raw: u32) -> u64 {
    let v = (prev & !0xffff_ffff) | u64::from(raw);
    if v < prev { v + (1 << 32) } else { v }
}

/// The power state an observation confirms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    Discharging,
    /// Charging, full or not charging: not on the battery.
    Other,
    Unknown,
}

impl Power {
    #[must_use]
    pub fn of(s: &Snapshot) -> Self {
        match (s.valid & 32 != 0, s.charge) {
            (true, 1) => Self::Discharging,
            (true, 2..=4) => Self::Other,
            _ => Self::Unknown,
        }
    }
}

/// One battery observation (every five seconds).
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// The widened tick counter in milliseconds.
    pub mono_ms: u64,
    pub utc: i64,
    /// The board RTC's raw seconds (`e32::time::rtc_seconds`).
    pub rtc: Option<u32>,
    pub battery: Snapshot,
    /// What the display HAL says now (`None`: the read failed).
    pub display: Option<bool>,
}

pub const EMPTY: u8 = 0;
pub const OBSERVED: u8 = 1;
pub const ANCHORED: u8 = 2;

/// What a checkpoint keeps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cycle {
    /// [`EMPTY`], [`OBSERVED`] (since the first reading) or [`ANCHORED`] (since a full charge then discharge).
    pub window: u8,
    /// Some time wasn't recorded (a gap, possibly of unknown length).
    pub incomplete: bool,
    /// Some discharge had a known display state: the screen-on count means something.
    pub screen_valid: bool,
    pub start_pct: Option<u8>,
    pub end_pct: Option<u8>,
    pub start_utc: i64,
    pub last_utc: i64,
    pub on_ms: u64,
    pub screen_ms: u64,
    pub gap_ms: u64,
    /// A full charge seen on USB, not followed by discharge yet: its level.
    pub pending: Option<Option<u8>>,
    /// A restart came between that full charge and now.
    pub pending_unobserved: bool,
}

impl Cycle {
    fn gap(&mut self, ms: u64) {
        self.gap_ms = self.gap_ms.saturating_add(ms);
        self.incomplete |= ms > 0;
    }
}

/// The recorder: the saved [`Cycle`] and what it has seen since SYSSTART started.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    pub cycle: Cycle,
    /// The last saved checkpoint's generation.
    pub generation: u64,
    /// Restored from a checkpoint with this RTC reading: the first sample counts the time since as a gap.
    resume: Option<Option<u32>>,
    /// The previous sample's time and power.
    last: Option<(u64, Power)>,
    display: Option<bool>,
    display_at: u64,
    /// Since the previous sample: time the display was on, and time its state wasn't known.
    seg_on: u64,
    seg_unknown: u64,
}

impl Recorder {
    /// A recorder that starts at `now_ms` with the display read as `display`, from the newest valid checkpoint.
    #[must_use]
    pub fn new(saved: Option<Checkpoint>, now_ms: u64, display: Option<bool>) -> Self {
        let mut r = Self {
            display,
            display_at: now_ms,
            ..Self::default()
        };
        if let Some(cp) = saved {
            r.generation = cp.generation;
            r.cycle = cp.cycle;
            r.cycle.pending_unobserved |= r.cycle.pending.is_some();
            r.resume = Some(cp.rtc);
        }
        r
    }

    fn close(&mut self, now: u64) {
        let d = now.saturating_sub(self.display_at);
        match self.display {
            Some(true) => self.seg_on += d,
            Some(false) => {}
            None => self.seg_unknown += d,
        }
        self.display_at = self.display_at.max(now);
    }

    /// The display was set on or off at `now_ms` and the HAL now says `state` (`None`: the read failed).
    pub fn display(&mut self, now_ms: u64, state: Option<bool>) {
        self.close(now_ms);
        self.display = state;
    }

    /// A battery observation. True when the cycle's anchor moved (save a checkpoint now).
    pub fn sample(&mut self, s: Sample) -> bool {
        self.close(s.mono_ms);
        let (seg_on, seg_unknown) = (
            core::mem::take(&mut self.seg_on),
            core::mem::take(&mut self.seg_unknown),
        );
        // a failed read, or a state nobody here set (the monitor's `panel`): this interval's display is unknown
        let surprise = s.display.is_none() || s.display != self.display;
        self.display = s.display;
        let power = Power::of(&s.battery);
        let pct = s.battery.percent().map(|p| p.min(100) as u8);
        let c = &mut self.cycle;
        if let Some((at, before)) = self.last {
            let dt = s.mono_ms.saturating_sub(at);
            let (on, unknown) = if surprise {
                (0, dt)
            } else {
                (seg_on.min(dt), seg_unknown.min(dt))
            };
            if c.window != EMPTY {
                let discharging = (before, power) == (Power::Discharging, Power::Discharging);
                if dt > MAX_STEP_MS || before == Power::Unknown || power == Power::Unknown {
                    c.gap(dt);
                } else if discharging {
                    c.on_ms += dt;
                    c.screen_ms += on;
                    if unknown > 0 {
                        c.gap(unknown);
                    }
                    c.screen_valid |= unknown < dt;
                } else if (before == Power::Discharging) != (power == Power::Discharging) {
                    // plugged or unplugged somewhere between the two samples
                    c.gap(dt);
                }
                // the wall clock was corrected: the start moves with it, the span stays what was measured
                let drift = (s.utc - c.last_utc).saturating_mul(1000) - dt as i64;
                if drift.abs() > 2000 {
                    c.start_utc += drift / 1000;
                }
            }
        } else if let Some(rtc) = self.resume.take()
            && c.window != EMPTY
        {
            // the time since the checkpoint (the phone off, a crash): a gap, by the RTC, else by the wall clock
            let by_rtc = rtc
                .zip(s.rtc)
                .map(|(was, now)| u64::from(now.wrapping_sub(was)));
            let by_utc = u64::try_from(s.utc - c.last_utc).ok();
            match by_rtc
                .filter(|&d| d <= PLAUSIBLE_OFF_S)
                .or(by_utc.filter(|&d| d <= PLAUSIBLE_OFF_S))
            {
                Some(d) => c.gap(d * 1000),
                None => c.incomplete = true,
            }
        }
        let mut moved = false;
        if s.battery.full_on_usb() {
            // the latest full charge on the cable; it becomes the cycle's start only when discharge follows
            c.pending = Some(pct);
            c.pending_unobserved = false;
        } else if power == Power::Discharging
            && let Some(start_pct) = c.pending
        {
            *c = Cycle {
                window: ANCHORED,
                incomplete: c.pending_unobserved,
                start_pct,
                start_utc: s.utc,
                ..Cycle::default()
            };
            moved = true;
        }
        if c.window == EMPTY && power != Power::Unknown {
            c.window = OBSERVED;
            c.start_utc = s.utc;
            c.start_pct = pct;
        }
        if c.window != EMPTY {
            c.last_utc = s.utc;
            c.end_pct = pct.or(c.end_pct);
        }
        self.last = Some((s.mono_ms, power));
        moved
    }

    /// INFO 9's record; `None` before the first valid observation.
    #[must_use]
    pub fn record(&self, checkpoint_failed: bool) -> Option<BatteryCycle> {
        let c = &self.cycle;
        let bit = |on: bool, b: u8| if on { b } else { 0 };
        (c.window != EMPTY).then(|| BatteryCycle {
            flags: bit(c.window == ANCHORED, bits::FULL_THEN_DISCHARGE)
                | bit(c.incomplete, bits::INCOMPLETE)
                | bit(checkpoint_failed, bits::CHECKPOINT_FAILED)
                | bit(c.screen_valid, bits::SCREEN_VALID),
            start_percent: c.start_pct,
            end_percent: c.end_pct,
            start_utc: c.start_utc,
            last_utc: c.last_utc,
            on_battery_s: c.on_ms / 1000,
            screen_on_battery_s: c.screen_ms / 1000,
            gap_s: c.gap_ms / 1000,
        })
    }
}

/// A saved cycle: `C:\private\e0002001\cycle-{a,b}.dat` by generation, the newest valid one wins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checkpoint {
    pub generation: u64,
    pub cycle: Cycle,
    /// The RTC when it was saved.
    pub rtc: Option<u32>,
}

impl Checkpoint {
    pub const LEN: usize = 68;

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let c = &self.cycle;
        let pct = |p: Option<u8>| p.map_or(255, |p| p.min(100));
        let mut b = Vec::with_capacity(Self::LEN);
        b.extend_from_slice(b"NCYC");
        b.push(1);
        b.push(c.window);
        b.push(u8::from(c.incomplete) | u8::from(c.screen_valid) << 1);
        b.push(pct(c.start_pct));
        b.push(pct(c.end_pct));
        b.push(c.pending.map_or(254, pct));
        b.push(u8::from(c.pending_unobserved) | u8::from(self.rtc.is_some()) << 1);
        b.push(0);
        b.extend_from_slice(&self.generation.to_le_bytes());
        for v in [c.start_utc, c.last_utc] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for v in [c.on_ms, c.screen_ms, c.gap_ms] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&self.rtc.unwrap_or(0).to_le_bytes());
        let crc = checksum(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        b
    }

    /// `None` for anything but a whole, valid checkpoint (a torn write, another file).
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != Self::LEN || &b[..5] != b"NCYC\x01" {
            return None;
        }
        let w = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let q = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        if checksum(&b[..64]) != w(64) || b[5] > ANCHORED || b[6] > 3 || b[10] > 3 || b[11] != 0 {
            return None;
        }
        let pct = |v: u8| match v {
            0..=100 => Some(Some(v)),
            255 => Some(None),
            _ => None,
        };
        let cycle = Cycle {
            window: b[5],
            incomplete: b[6] & 1 != 0,
            screen_valid: b[6] & 2 != 0,
            start_pct: pct(b[7])?,
            end_pct: pct(b[8])?,
            pending: if b[9] == 254 { None } else { Some(pct(b[9])?) },
            pending_unobserved: b[10] & 1 != 0,
            start_utc: q(20) as i64,
            last_utc: q(28) as i64,
            on_ms: q(36),
            screen_ms: q(44),
            gap_ms: q(52),
        };
        if cycle.screen_ms > cycle.on_ms {
            return None;
        }
        Some(Self {
            generation: q(12),
            cycle,
            rtc: (b[10] & 2 != 0).then(|| w(60)),
        })
    }

    /// The newer of two slots (either may be missing or damaged).
    #[must_use]
    pub fn newest(a: Option<Self>, b: Option<Self>) -> Option<Self> {
        match (a, b) {
            (Some(a), Some(b)) => Some(if a.generation >= b.generation { a } else { b }),
            (a, b) => a.or(b),
        }
    }
}

/// What Battery usage and search show of a cycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CycleText {
    /// "Since full charge", "Since the last observed full charge" or "Not since a full charge yet".
    pub title: String,
    /// The levels at each end: "100%" and "72%" ("--%" unread). The page draws the arrow between them.
    pub from: String,
    pub to: String,
    /// "9 h 40 min": the window's length.
    pub span: String,
    /// "On battery 8 h 10 min".
    pub on_battery: String,
    /// "Off or not recorded for 6 h (not counted)"; empty without gaps.
    pub gaps: String,
    /// Screen-on time: "2 h 14 min since charged", "2 h 14 min in the last 9 h 40 min", "Not recorded yet".
    pub screen: String,
}

fn dur(s: u64) -> String {
    duration(u32::try_from(s).unwrap_or(u32::MAX))
}

/// The texts for a cycle. `screen_recorded`: the shell records screen time (INFO 9; INFO 5's history doesn't).
#[must_use]
pub fn text(c: &BatteryCycle, screen_recorded: bool) -> CycleText {
    let pct = |p: Option<u8>| p.map_or_else(|| "--%".into(), |n| format!("{n}%"));
    let anchored = c.flags & bits::FULL_THEN_DISCHARGE != 0;
    // shortcut: INFO 9 has one "incomplete" bit for known and unknown gaps; a known gap under a minute is the
    // sampling boundary (a plug or unplug between two samples) and isn't shown, an incomplete record without a
    // counted gap is time of unknown length. A distinct bit would tell them apart if a short unknown gap matters.
    let partial = c.gap_s >= 60 || (c.flags & bits::INCOMPLETE != 0 && c.gap_s == 0);
    let span = dur(u64::try_from(c.last_utc.saturating_sub(c.start_utc)).unwrap_or(0));
    let screen = if !screen_recorded {
        "Not recorded on this version".into()
    } else if c.flags & bits::SCREEN_VALID == 0 {
        "Not recorded yet".into()
    } else {
        let on = dur(c.screen_on_battery_s);
        let tail = if partial { " (partial)" } else { "" };
        if anchored {
            format!("{on} since charged{tail}")
        } else {
            format!("{on} in the last {span}{tail}")
        }
    };
    CycleText {
        title: match (anchored, partial) {
            (true, false) => "Since full charge",
            (true, true) => "Since the last observed full charge",
            (false, _) => "Not since a full charge yet",
        }
        .into(),
        from: pct(c.start_percent),
        to: pct(c.end_percent),
        span,
        on_battery: format!("On battery {}", dur(c.on_battery_s)),
        gaps: if c.gap_s >= 60 {
            format!("Off or not recorded for {} (not counted)", dur(c.gap_s))
        } else if partial {
            "Some time not recorded (not counted)".into()
        } else {
            String::new()
        },
        screen,
    }
}

/// INFO 5's usage (a shell without INFO 9) in INFO 9's form: no screen time.
#[must_use]
pub fn from_usage(u: &super::Usage) -> BatteryCycle {
    BatteryCycle {
        flags: if u.since_full {
            bits::FULL_THEN_DISCHARGE
        } else {
            0
        } | if u.gap_s > 0 { bits::INCOMPLETE } else { 0 },
        start_percent: u.start_percent,
        end_percent: u.end_percent,
        start_utc: u.start_utc,
        last_utc: u.end_utc,
        on_battery_s: u64::from(u.on_battery_s),
        screen_on_battery_s: 0,
        gap_s: u64::from(u.gap_s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000;

    fn snap(charge: u32, pct: u32, usb: bool) -> Snapshot {
        Snapshot {
            valid: 2 | 8 | 16 | 32,
            percent: pct,
            usb_present: u32::from(usb),
            usb_online: u32::from(usb),
            charge,
            ..Snapshot::default()
        }
    }
    const DIS: u32 = 1;
    const CHG: u32 = 2;
    const FULL: u32 = 4;

    /// A recorder fed by a test clock: samples every 5 s, transitions between them.
    struct Rig {
        r: Recorder,
        ms: u64,
        utc: i64,
        rtc: u32,
        display: Option<bool>,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                r: Recorder::new(None, 0, Some(true)),
                ms: 0,
                utc: T0,
                rtc: 50_000,
                display: Some(true),
            }
        }
        fn at(&self, b: Snapshot) -> Sample {
            Sample {
                mono_ms: self.ms,
                utc: self.utc,
                rtc: Some(self.rtc),
                battery: b,
                display: self.display,
            }
        }
        /// `n` samples of `b`, 5 s apart (the first one now).
        fn run(&mut self, n: u32, b: Snapshot) -> bool {
            let mut moved = false;
            for _ in 0..n {
                moved |= self.r.sample(self.at(b));
                self.step(5_000);
            }
            moved
        }
        fn step(&mut self, ms: u64) {
            self.ms += ms;
            self.utc += (ms / 1000) as i64;
            self.rtc += (ms / 1000) as u32;
        }
        fn set_display(&mut self, on: bool) {
            self.display = Some(on);
            self.r.display(self.ms, Some(on));
        }
        fn rec(&self) -> BatteryCycle {
            self.r.record(false).unwrap()
        }
    }

    #[test]
    fn nothing_before_the_first_valid_reading() {
        let mut g = Rig::new();
        g.run(3, Snapshot::default());
        assert_eq!(g.r.record(false), None);
        g.run(1, snap(DIS, 80, false));
        let c = g.rec();
        assert_eq!(c.flags & bits::FULL_THEN_DISCHARGE, 0);
        assert_eq!((c.start_percent, c.start_utc), (Some(80), T0 + 15));
    }

    #[test]
    fn full_then_discharge_starts_a_cycle_once() {
        let mut g = Rig::new();
        g.run(10, snap(CHG, 90, true));
        assert!(
            !g.run(10, snap(FULL, 100, true)),
            "full on the cable only arms"
        );
        let before = g.rec();
        assert_eq!(before.flags & bits::FULL_THEN_DISCHARGE, 0);
        // unplugged: the cycle starts at the first discharging sample, from the full charge's level
        assert!(g.r.sample(g.at(snap(DIS, 100, false))));
        let start = g.utc;
        g.step(5_000);
        assert!(
            !g.run(12, snap(DIS, 99, false)),
            "later discharge doesn't restart it"
        );
        let c = g.rec();
        assert_eq!(c.flags, bits::FULL_THEN_DISCHARGE | bits::SCREEN_VALID);
        assert_eq!((c.start_percent, c.end_percent), (Some(100), Some(99)));
        assert_eq!(c.start_utc, start);
        assert_eq!(
            (c.on_battery_s, c.screen_on_battery_s, c.gap_s),
            (60, 60, 0)
        );
    }

    #[test]
    fn a_full_discharging_reading_or_percent_alone_arms_nothing() {
        let mut g = Rig::new();
        g.run(5, snap(DIS, 100, false));
        // 100% with no charge state: not even a valid power reading
        g.run(
            5,
            Snapshot {
                valid: 2,
                percent: 100,
                ..Snapshot::default()
            },
        );
        g.run(5, snap(DIS, 99, false));
        assert_eq!(g.rec().flags & bits::FULL_THEN_DISCHARGE, 0);
    }

    #[test]
    fn a_recharge_without_full_pauses_the_cycle() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(13, snap(DIS, 95, false)); // 60 s on battery
        g.run(20, snap(CHG, 60, true)); // a top-up, not full
        g.run(13, snap(DIS, 70, false));
        let c = g.rec();
        assert_eq!(
            c.flags & bits::FULL_THEN_DISCHARGE,
            bits::FULL_THEN_DISCHARGE
        );
        assert_eq!(c.start_percent, Some(100));
        assert_eq!(c.on_battery_s, 120);
        // the two cable edges (5 s each) aren't known either way
        assert_eq!(c.gap_s, 10);
        // under a minute: the sampling boundary, not shown
        let t = text(&c, true);
        assert_eq!(t.title, "Since full charge");
        assert_eq!(t.gaps, "");
        assert_eq!(t.screen, "2 min since charged");
    }

    #[test]
    fn repeated_full_charges_keep_the_cycle_until_discharge() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(13, snap(DIS, 90, false));
        let shown = g.rec();
        g.run(5, snap(FULL, 100, true));
        g.run(5, snap(CHG, 99, true));
        g.run(5, snap(FULL, 100, true));
        // plugged in at full: still the old cycle
        assert_eq!(g.rec().start_utc, shown.start_utc);
        assert!(g.run(1, snap(DIS, 100, false)));
        assert_ne!(g.rec().start_utc, shown.start_utc);
        assert_eq!(g.rec().on_battery_s, 0);
    }

    #[test]
    fn screen_time_counts_only_acknowledged_on_while_discharging() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(1, snap(DIS, 100, false)); // at 10 s
        // before the next sample (15 s): off at 11 s, on at 12 s, off at 14.5 s → 1 s + 2.5 s on
        g.r.display(11_000, Some(false));
        g.r.display(12_000, Some(true));
        g.r.display(14_500, Some(false));
        g.display = Some(false);
        g.run(1, snap(DIS, 100, false));
        assert_eq!(g.r.cycle.screen_ms, 3_500);
        // dark for an hour adds nothing
        g.run(720, snap(DIS, 90, false));
        let c = g.rec();
        assert_eq!(c.screen_on_battery_s, 3);
        assert_eq!(c.on_battery_s, 5 + 3600);
        assert_eq!(c.gap_s, 0);
        // dark while charging adds nothing either, and screen on while charging isn't on battery
        g.set_display(true);
        g.run(100, snap(CHG, 90, true));
        assert_eq!(g.rec().screen_on_battery_s, 3);
    }

    #[test]
    fn a_failed_or_surprising_display_read_is_a_gap_but_discharge_counts() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(3, snap(DIS, 100, false)); // 10 s, screen on
        // a SET whose GET failed (the panel stayed on): that interval's display is unknown
        g.r.display(g.ms - 2_000, None);
        g.run(1, snap(DIS, 100, false));
        // the monitor turned the panel off behind SYSSTART's back: unknown again
        g.display = Some(false);
        g.run(1, snap(DIS, 100, false));
        // then known dark
        g.run(1, snap(DIS, 100, false));
        let c = g.rec();
        assert_eq!(c.on_battery_s, 25);
        assert_eq!(c.screen_on_battery_s, 10);
        assert_eq!(c.gap_s, 10);
        assert_eq!(c.flags & bits::INCOMPLETE, bits::INCOMPLETE);
    }

    #[test]
    fn unknown_display_all_along_is_not_a_zero() {
        let mut g = Rig::new();
        g.display = None;
        g.r = Recorder::new(None, 0, None);
        g.run(2, snap(FULL, 100, true));
        g.run(20, snap(DIS, 100, false));
        let c = g.rec();
        assert_eq!(c.flags & bits::SCREEN_VALID, 0);
        assert_eq!(text(&c, true).screen, "Not recorded yet");
        assert_eq!(text(&c, false).screen, "Not recorded on this version");
    }

    #[test]
    fn missing_samples_and_unknown_power_are_gaps() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(3, snap(DIS, 100, false));
        g.step(3_600_000); // an hour without a sample (a stalled loop)
        g.run(3, snap(DIS, 95, false));
        g.run(2, Snapshot::default()); // a failed HAL read
        g.run(3, snap(DIS, 95, false));
        let c = g.rec();
        // 10 + 10 + 10 s counted
        assert_eq!(c.on_battery_s, 30);
        // the hour and its step (3 605 s), and the 3 intervals touching the failed reads
        assert_eq!(c.gap_s, 3_605 + 15);
        let t = text(&c, true);
        assert_eq!(t.title, "Since the last observed full charge");
        assert_eq!(t.gaps, "Off or not recorded for 1 h (not counted)");
        assert_eq!(t.screen, "0 min since charged (partial)");
    }

    #[test]
    fn a_clock_change_keeps_the_durations_and_moves_the_start() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(13, snap(DIS, 100, false));
        let start = g.rec().start_utc;
        g.utc += 3 * 3600; // the network set the clock 3 h forward
        g.run(13, snap(DIS, 100, false));
        let c = g.rec();
        assert_eq!(c.on_battery_s, 125);
        assert_eq!(c.gap_s, 0);
        assert_eq!(c.start_utc, start + 3 * 3600);
        assert_eq!(c.last_utc - c.start_utc, 125);
        g.utc -= 10 * 3600;
        g.run(2, snap(DIS, 100, false));
        assert_eq!(g.rec().last_utc - g.rec().start_utc, 135);
    }

    #[test]
    fn the_tick_counter_widens_across_its_wrap() {
        assert_eq!(widen(0, 5), 5);
        assert_eq!(widen(0xffff_fff0, 0x10), 0x1_0000_0010);
        assert_eq!(widen(0x1_0000_0010, 0x20), 0x1_0000_0020);
        assert_eq!(widen(0x3_ffff_ffff, 0), 0x4_0000_0000);
        // samples across the wrap still step 5 s
        let mut g = Rig::new();
        g.ms = (1u64 << 32) - 7_000;
        g.r = Recorder::new(None, g.ms, Some(true));
        g.run(2, snap(FULL, 100, true));
        g.run(4, snap(DIS, 100, false));
        assert_eq!(g.rec().on_battery_s, 15);
    }

    fn restart(g: &mut Rig, saved: Checkpoint, off_s: u64, rtc_ok: bool) {
        g.step(off_s * 1000);
        g.ms = 0;
        if !rtc_ok {
            g.rtc = 7; // reset: the delta is implausible
        }
        g.r = Recorder::new(Some(saved), 0, Some(true));
    }

    fn save(g: &Rig) -> Checkpoint {
        Checkpoint::decode(
            &Checkpoint {
                generation: g.r.generation + 1,
                cycle: g.r.cycle,
                rtc: Some(g.rtc),
            }
            .encode(),
        )
        .unwrap()
    }

    #[test]
    fn a_restart_counts_the_off_time_once_as_a_gap() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(13, snap(DIS, 100, false));
        let cp = save(&g);
        restart(&mut g, cp, 600, true);
        g.run(13, snap(DIS, 98, false));
        let c = g.rec();
        assert_eq!(c.on_battery_s, 120);
        assert_eq!(
            c.screen_on_battery_s, 120,
            "the off time adds no screen time"
        );
        assert_eq!(c.gap_s, 600);
        assert_eq!(
            c.flags & bits::FULL_THEN_DISCHARGE,
            bits::FULL_THEN_DISCHARGE
        );
        // without a believable RTC the wall clock says how long
        let mut g2 = Rig::new();
        g2.run(2, snap(FULL, 100, true));
        g2.run(13, snap(DIS, 100, false));
        let cp = save(&g2);
        restart(&mut g2, cp, 300, false);
        g2.run(2, snap(DIS, 100, false));
        // from the last sample (5 s before the save) to the first after the restart
        assert_eq!(g2.rec().gap_s, 305);
    }

    #[test]
    fn a_restart_without_any_clock_is_an_unknown_gap() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(13, snap(DIS, 100, false));
        let mut cp = save(&g);
        cp.rtc = None;
        restart(&mut g, cp, 60, true);
        g.utc = T0 - 1_000_000; // the clock went back too
        g.run(2, snap(DIS, 100, false));
        let c = g.rec();
        assert_eq!((c.gap_s, c.flags & bits::INCOMPLETE), (0, bits::INCOMPLETE));
        let t = text(&c, true);
        assert_eq!(t.gaps, "Some time not recorded (not counted)");
        assert_eq!(t.title, "Since the last observed full charge");
    }

    #[test]
    fn a_pending_full_charge_survives_a_restart_qualified() {
        let mut g = Rig::new();
        g.run(5, snap(FULL, 100, true));
        let cp = save(&g);
        restart(&mut g, cp, 30, true);
        assert!(g.run(1, snap(DIS, 100, false)));
        let c = g.rec();
        assert_eq!(
            c.flags & bits::FULL_THEN_DISCHARGE,
            bits::FULL_THEN_DISCHARGE
        );
        assert_eq!(c.flags & bits::INCOMPLETE, bits::INCOMPLETE);
        // but a full charge seen again after the restart is a clean anchor
        let mut g = Rig::new();
        g.run(5, snap(FULL, 100, true));
        let cp = save(&g);
        restart(&mut g, cp, 30, true);
        g.run(2, snap(FULL, 100, true));
        g.run(1, snap(DIS, 100, false));
        assert_eq!(g.rec().flags & bits::INCOMPLETE, 0);
    }

    #[test]
    fn a_cycle_and_its_screen_time_past_24_hours() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        // 30 h on battery, the screen on the whole time (5 s steps: 21 600 samples)
        g.run(21_601, snap(DIS, 40, false));
        let c = g.rec();
        assert_eq!(c.on_battery_s, 30 * 3600);
        assert_eq!(c.screen_on_battery_s, 30 * 3600);
        // a save and a restart keep it
        let cp = save(&g);
        restart(&mut g, cp, 0, true);
        g.run(1, snap(DIS, 40, false));
        assert_eq!(g.rec().on_battery_s, 30 * 3600);
        let t = text(&c, true);
        assert_eq!((t.from.as_str(), t.to.as_str()), ("100%", "40%"));
        assert_eq!(t.span, "30 h");
        assert_eq!(t.screen, "30 h since charged");
        assert_eq!(t.on_battery, "On battery 30 h");
    }

    #[test]
    fn checkpoints_reject_damage_and_the_newest_wins() {
        let mut g = Rig::new();
        g.run(2, snap(FULL, 100, true));
        g.run(13, snap(DIS, 97, false));
        let cp = save(&g);
        let b = cp.encode();
        assert_eq!(b.len(), Checkpoint::LEN);
        assert_eq!(Checkpoint::decode(&b), Some(cp));
        // torn (short), a flipped byte, another version
        assert_eq!(Checkpoint::decode(&b[..40]), None);
        for at in [4, 5, 9, 20, 40, 64] {
            let mut d = b.clone();
            d[at] ^= 0x40;
            assert_eq!(Checkpoint::decode(&d), None, "byte {at}");
        }
        let older = Checkpoint {
            generation: 3,
            ..cp
        };
        let newer = Checkpoint {
            generation: 4,
            ..cp
        };
        assert_eq!(Checkpoint::newest(Some(older), Some(newer)), Some(newer));
        assert_eq!(Checkpoint::newest(Some(older), None), Some(older));
        assert_eq!(Checkpoint::newest(None, None), None);
        // a pending anchor of unknown level round-trips
        let mut p = cp;
        p.cycle.pending = Some(None);
        assert_eq!(Checkpoint::decode(&p.encode()), Some(p));
    }

    #[test]
    fn texts_before_a_full_charge_name_the_measured_interval() {
        let c = BatteryCycle {
            flags: bits::SCREEN_VALID,
            start_percent: Some(85),
            end_percent: Some(72),
            start_utc: T0,
            last_utc: T0 + 9 * 3600 + 40 * 60,
            on_battery_s: 9 * 3600,
            screen_on_battery_s: 2 * 3600 + 14 * 60,
            gap_s: 0,
        };
        let t = text(&c, true);
        assert_eq!(t.title, "Not since a full charge yet");
        assert_eq!((t.from.as_str(), t.to.as_str()), ("85%", "72%"));
        assert_eq!(t.span, "9 h 40 min");
        assert_eq!(t.screen, "2 h 14 min in the last 9 h 40 min");
    }

    #[test]
    fn an_older_shells_usage_has_no_screen_time() {
        let u = super::super::Usage {
            since_full: true,
            start_utc: T0,
            end_utc: T0 + 3600,
            start_percent: Some(100),
            end_percent: None,
            on_battery_s: 3000,
            gap_s: 600,
        };
        let t = text(&from_usage(&u), false);
        assert_eq!(t.title, "Since the last observed full charge");
        assert_eq!(t.to, "--%");
        assert_eq!(t.gaps, "Off or not recorded for 10 min (not counted)");
        assert_eq!(t.screen, "Not recorded on this version");
    }
}

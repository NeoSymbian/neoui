// SPDX-License-Identifier: Apache-2.0
//! The clock across restarts and power-offs (NeoSymbian H3a). The board RTC keeps counting seconds while the phone is
//! off, but phones don't let the OS set it, so its zero means nothing. The UI server keeps "UTC - RTC" and the UTC
//! offset in a small file and, at boot, sets the kernel's clock to RTC + that.
//!
//! Who sets the time (the Clock app, the debug monitor's `date` that `phone.py run` sends) only sets the kernel's
//! clock. The server notices the change against what it last saw ([`Saved::changed`]) and writes the file.

/// The file, in the UI server's private directory (SYSSTART.EXE, SID 0xE0002001).
pub const DIR: &str = "C:\\private\\e0002001\\";
pub const PATH: &str = "C:\\private\\e0002001\\clock.dat";

const MAGIC: [u8; 4] = *b"NCK1";
pub const LEN: usize = 20;
/// A difference smaller than this is the two clocks drifting apart, not someone setting the time.
const SLACK_S: i64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    /// UTC (Unix seconds) minus the RTC's count.
    pub rtc_offset: i64,
    /// The UTC offset (time zone), seconds.
    pub tz: i32,
    /// The RTC's count when this was saved: an RTC now behind it was reset (its supply ran out).
    pub rtc_at_save: u32,
}

impl Saved {
    /// What the kernel's clock (`utc`, `tz`) says, against RTC count `rtc`.
    pub fn of(utc: i64, tz: i32, rtc: u32) -> Self {
        Self {
            rtc_offset: utc - i64::from(rtc),
            tz,
            rtc_at_save: rtc,
        }
    }

    pub fn encode(&self) -> [u8; LEN] {
        let mut b = [0u8; LEN];
        b[..4].copy_from_slice(&MAGIC);
        b[4..12].copy_from_slice(&self.rtc_offset.to_le_bytes());
        b[12..16].copy_from_slice(&self.tz.to_le_bytes());
        b[16..20].copy_from_slice(&self.rtc_at_save.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != LEN || b[..4] != MAGIC {
            return None;
        }
        Some(Self {
            rtc_offset: i64::from_le_bytes(b[4..12].try_into().ok()?),
            tz: i32::from_le_bytes(b[12..16].try_into().ok()?),
            rtc_at_save: u32::from_le_bytes(b[16..20].try_into().ok()?),
        })
    }

    /// The UTC to set at boot from RTC count `rtc`; `None` when the RTC went back (it was reset: the time is lost).
    // shortcut: an RTC reset that has since counted past rtc_at_save goes unnoticed; a valid-time flag needs a
    // writable RTC or a network clock.
    pub fn utc_at(&self, rtc: u32) -> Option<i64> {
        (rtc >= self.rtc_at_save).then(|| i64::from(rtc) + self.rtc_offset)
    }

    /// The new state to save when the kernel's clock (`utc`, `tz`) no longer matches this one: someone set it.
    pub fn changed(&self, utc: i64, tz: i32, rtc: u32) -> Option<Self> {
        let now = Self::of(utc, tz, rtc);
        ((now.rtc_offset - self.rtc_offset).abs() > SLACK_S || now.tz != self.tz).then_some(now)
    }
}

/// A local date and time being set by hand (the Clock app), with its time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 1-12
    pub month: u32,
    /// 1-31
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// Seconds east of UTC.
    pub tz: i32,
}

/// The fields [`Local::adjust`] steps, in the order the Clock app shows them.
pub const FIELDS: [&str; 6] = ["Year", "Month", "Day", "Hour", "Minute", "Zone"];
const TZ_STEP: i32 = 15 * 60;
const TZ_MIN: i32 = -12 * 3600;
const TZ_MAX: i32 = 14 * 3600;

impl Local {
    /// Step field `field` (an index into [`FIELDS`]) by `delta`; months, days, hours and minutes wrap, the day is
    /// kept inside the month, the zone moves in 15-minute steps from UTC-12 to UTC+14.
    pub fn adjust(&mut self, field: usize, delta: i32) {
        let wrap = |v: u32, lo: u32, n: u32| {
            (lo as i32 + (v as i32 - lo as i32 + delta).rem_euclid(n as i32)) as u32
        };
        match field {
            0 => self.year = (self.year + delta).clamp(2000, 2099),
            1 => self.month = wrap(self.month, 1, 12),
            2 => self.day = wrap(self.day, 1, days_in_month(self.year, self.month)),
            3 => self.hour = wrap(self.hour, 0, 24),
            4 => self.minute = wrap(self.minute, 0, 60),
            5 => self.tz = (self.tz + delta * TZ_STEP).clamp(TZ_MIN, TZ_MAX),
            _ => {}
        }
        self.day = self.day.min(days_in_month(self.year, self.month));
    }

    /// Field `field` as the Clock app shows it.
    pub fn show(&self, field: usize) -> alloc::string::String {
        use alloc::format;
        match field {
            0 => format!("{}", self.year),
            1 => format!("{:02}", self.month),
            2 => format!("{:02}", self.day),
            3 => format!("{:02}", self.hour),
            4 => format!("{:02}", self.minute),
            _ => {
                let a = self.tz.unsigned_abs() / 60;
                format!(
                    "UTC{}{:02}:{:02}",
                    if self.tz < 0 { '-' } else { '+' },
                    a / 60,
                    a % 60
                )
            }
        }
    }

    /// UTC, Unix seconds (second 0 of the minute).
    pub fn to_unix(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * 86_400
            + i64::from(self.hour * 3600 + self.minute * 60)
            - i64::from(self.tz)
    }
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::{Local, Saved};

    #[test]
    fn local_to_unix() {
        let mut l = Local {
            year: 2026,
            month: 11,
            day: 8,
            hour: 16,
            minute: 0,
            tz: 4 * 3600,
        };
        assert_eq!(l.to_unix(), 1_794_139_200); // 12:00 UTC
        assert_eq!(l.show(5), "UTC+04:00");
        l.tz = -(3 * 3600 + 30 * 60);
        assert_eq!(l.show(5), "UTC-03:30");
        let epoch = Local {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            tz: 0,
        };
        assert_eq!(epoch.to_unix(), 0);
    }

    #[test]
    fn adjust_wraps_and_clamps() {
        let mut l = Local {
            year: 2024,
            month: 1,
            day: 31,
            hour: 23,
            minute: 59,
            tz: 0,
        };
        l.adjust(4, 1);
        l.adjust(3, 1);
        assert_eq!((l.hour, l.minute), (0, 0)); // fields wrap on their own, no carry
        l.adjust(1, 1); // February 2024 has 29 days
        assert_eq!((l.month, l.day), (2, 29));
        l.adjust(0, 1); // 2025: 28
        assert_eq!(l.day, 28);
        l.adjust(1, -2);
        assert_eq!(l.month, 12);
        l.adjust(2, 1);
        assert_eq!(l.day, 29);
        for _ in 0..200 {
            l.adjust(5, 1);
        }
        assert_eq!(l.tz, 14 * 3600);
    }

    #[test]
    fn round_trip() {
        let s = Saved::of(1_794_139_200, 4 * 3600, 123_456);
        assert_eq!(Saved::decode(&s.encode()), Some(s));
        assert_eq!(Saved::decode(&s.encode()[..19]), None);
        let mut bad = s.encode();
        bad[0] = b'X';
        assert_eq!(Saved::decode(&bad), None);
    }

    #[test]
    fn boot_time_from_rtc() {
        let s = Saved::of(1_794_139_200, 0, 1_000);
        // powered off for 10 minutes
        assert_eq!(s.utc_at(1_600), Some(1_794_139_800));
        assert_eq!(s.utc_at(1_000), Some(1_794_139_200));
        // the RTC was reset
        assert_eq!(s.utc_at(5), None);
    }

    #[test]
    fn changes_only_when_set() {
        let s = Saved::of(1_794_139_200, 0, 1_000);
        // a minute later, both clocks moved together (and a second of drift): nothing to save
        assert_eq!(s.changed(1_794_139_260, 0, 1_060), None);
        assert_eq!(s.changed(1_794_139_262, 0, 1_060), None);
        // the time was set an hour forward, or the time zone changed
        assert_eq!(
            s.changed(1_794_142_860, 0, 1_060),
            Some(Saved::of(1_794_142_860, 0, 1_060))
        );
        assert!(s.changed(1_794_139_260, 3600, 1_060).is_some());
    }
}

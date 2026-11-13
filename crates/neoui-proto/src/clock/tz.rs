// SPDX-License-Identifier: Apache-2.0
//! Time zone rules as POSIX `TZ` strings (the form IANA tzdata ends each zone with, e.g. Berlin
//! `CET-1CEST,M3.5.0,M10.5.0/3`): the standard offset, and the daylight saving offset with the two yearly changes.
//! Only today's rule: no history (docs/neoui-shell.md "Time zones").

use super::datetime::days_from_civil;
use super::local::days_in_month;

const DAY: i64 = 86_400;

/// When in the year a change happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Date {
    /// `Mm.w.d`: weekday `d` (0 = Sunday) of week `w` (1..=5, 5 = the last) of month `m`.
    Month { m: u8, w: u8, d: u8 },
    /// `Jn`: day 1..=365, 29 February never counted.
    Julian(u16),
    /// `n`: day 0..=365, 29 February counted.
    Day(u16),
}

impl Date {
    /// Days since 1970-01-01 of this date in `year`.
    fn days(self, year: i64) -> i64 {
        let jan1 = days_from_civil(year, 1, 1);
        let leap = days_in_month(year as i32, 2) == 29;
        match self {
            Date::Julian(n) => jan1 + i64::from(n) - 1 + i64::from(leap && n >= 60),
            Date::Day(n) => jan1 + i64::from(n),
            Date::Month { m, w, d } => {
                let first = days_from_civil(year, i64::from(m), 1);
                let weekday = (first + 4).rem_euclid(7); // 1970-01-01 was a Thursday
                let mut day = (i64::from(d) - weekday).rem_euclid(7) + 7 * (i64::from(w) - 1);
                let len = i64::from(days_in_month(year as i32, u32::from(m)));
                while day >= len {
                    day -= 7; // week 5: the last one in the month
                }
                first + day
            }
        }
    }
}

/// A change: the date and the local time of day it happens at (seconds; POSIX allows -167..=167 hours).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Change {
    date: Date,
    time: i32,
}

/// A parsed `TZ` rule. Offsets are seconds east of UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub standard: i32,
    /// The daylight saving offset and when it starts and ends (in the local time in force before each change).
    dst: Option<(i32, Change, Change)>,
}

impl Rule {
    /// UTC, no daylight saving.
    pub const UTC: Rule = Rule {
        standard: 0,
        dst: None,
    };

    /// Parses a POSIX `TZ` string; `None` if it isn't one (a rule with a DST name but no dates is refused too).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let mut p = Parser(s.as_bytes());
        p.name()?;
        let standard = -p.offset()?;
        if p.0.is_empty() {
            return Some(Self {
                standard,
                dst: None,
            });
        }
        p.name()?;
        let dst = if p.0.first().is_some_and(|c| *c != b',') {
            -p.offset()?
        } else {
            standard + 3600
        };
        p.eat(b',')?;
        let start = p.change()?;
        p.eat(b',')?;
        let end = p.change()?;
        p.0.is_empty().then_some(Self {
            standard,
            dst: Some((dst, start, end)),
        })
    }

    /// The offset in force at `utc` (Unix seconds).
    #[must_use]
    pub fn offset_at(&self, utc: i64) -> i32 {
        if self.is_dst(utc) {
            self.dst.map_or(self.standard, |d| d.0)
        } else {
            self.standard
        }
    }

    /// Daylight saving time is in force at `utc`.
    #[must_use]
    pub fn is_dst(&self, utc: i64) -> bool {
        let Some((dst, start, end)) = self.dst else {
            return false;
        };
        let year = civil_year(utc + i64::from(self.standard));
        let at = |c: Change, before: i32| {
            c.date.days(year) * DAY + i64::from(c.time) - i64::from(before)
        };
        let (on, off) = (at(start, self.standard), at(end, dst));
        if on < off {
            on <= utc && utc < off // northern: summer inside the year
        } else {
            utc < off || utc >= on // southern: summer across the new year
        }
    }
}

/// The proleptic Gregorian year of day `secs / 86400` since 1970 (H. Hinnant's civil_from_days, year only).
fn civil_year(secs: i64) -> i64 {
    let z = secs.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    yoe + era * 400 + i64::from(mp >= 10)
}

struct Parser<'a>(&'a [u8]);

impl Parser<'_> {
    fn eat(&mut self, c: u8) -> Option<()> {
        let (first, rest) = self.0.split_first()?;
        (*first == c).then(|| self.0 = rest)
    }

    /// `ABC` (three letters or more) or `<+0330>`.
    fn name(&mut self) -> Option<()> {
        let n = if self.0.first() == Some(&b'<') {
            self.0.iter().position(|c| *c == b'>')? + 1
        } else {
            let n = self
                .0
                .iter()
                .take_while(|c| c.is_ascii_alphabetic())
                .count();
            if n < 3 {
                return None;
            }
            n
        };
        self.0 = &self.0[n..];
        Some(())
    }

    fn number(&mut self) -> Option<i32> {
        let n = self.0.iter().take_while(|c| c.is_ascii_digit()).count();
        if n == 0 || n > 3 {
            return None;
        }
        let v = self.0[..n]
            .iter()
            .fold(0, |v, c| v * 10 + i32::from(c - b'0'));
        self.0 = &self.0[n..];
        Some(v)
    }

    /// `[+-]hh[:mm[:ss]]` as seconds (the sign as written: POSIX offsets are west of UTC).
    fn offset(&mut self) -> Option<i32> {
        let sign = match self.0.first() {
            Some(b'-') => {
                self.0 = &self.0[1..];
                -1
            }
            Some(b'+') => {
                self.0 = &self.0[1..];
                1
            }
            _ => 1,
        };
        let mut secs = self.number()? * 3600;
        for unit in [60, 1] {
            if self.eat(b':').is_none() {
                break;
            }
            let v = self.number()?;
            if v > 59 {
                return None;
            }
            secs += v * unit;
        }
        (secs <= 167 * 3600).then_some(sign * secs)
    }

    /// `Mm.w.d[/time]`, `Jn[/time]` or `n[/time]`; the time defaults to 02:00.
    fn change(&mut self) -> Option<Change> {
        let date = match self.0.first()? {
            b'M' => {
                self.0 = &self.0[1..];
                let m = self.number()?;
                self.eat(b'.')?;
                let w = self.number()?;
                self.eat(b'.')?;
                let d = self.number()?;
                if !(1..=12).contains(&m) || !(1..=5).contains(&w) || d > 6 {
                    return None;
                }
                Date::Month {
                    m: m as u8,
                    w: w as u8,
                    d: d as u8,
                }
            }
            b'J' => {
                self.0 = &self.0[1..];
                let n = self.number()?;
                if !(1..=365).contains(&n) {
                    return None;
                }
                Date::Julian(n as u16)
            }
            _ => {
                let n = self.number()?;
                if n > 365 {
                    return None;
                }
                Date::Day(n as u16)
            }
        };
        let time = if self.eat(b'/').is_some() {
            self.offset()?
        } else {
            7200
        };
        Some(Change { date, time })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unix seconds of a UTC date and time.
    fn utc(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
        days_from_civil(y, m, d) * DAY + h * 3600 + min * 60
    }

    fn rule(s: &str) -> Rule {
        Rule::parse(s).unwrap_or_else(|| panic!("{s} parses"))
    }

    #[test]
    fn fixed_zones() {
        assert_eq!(rule("<+04>-4").offset_at(0), 4 * 3600);
        assert_eq!(rule("EST5").offset_at(0), -5 * 3600);
        assert_eq!(rule("<+0545>-5:45").offset_at(0), 5 * 3600 + 45 * 60);
        assert_eq!(rule("<+0330>-3:30").offset_at(0), 3 * 3600 + 30 * 60);
        assert_eq!(rule("GMT0").offset_at(0), 0);
        assert!(!rule("<+04>-4").is_dst(0));
    }

    #[test]
    fn europe_changes_at_01_utc() {
        let berlin = rule("CET-1CEST,M3.5.0,M10.5.0/3");
        // 2026: last Sunday of March is the 29th, of October the 25th; both at 01:00 UTC
        assert_eq!(berlin.offset_at(utc(2026, 3, 29, 0, 59)), 3600);
        assert_eq!(berlin.offset_at(utc(2026, 3, 29, 1, 0)), 7200);
        assert_eq!(berlin.offset_at(utc(2026, 10, 25, 0, 59)), 7200);
        assert_eq!(berlin.offset_at(utc(2026, 10, 25, 1, 0)), 3600);
        let london = rule("GMT0BST,M3.5.0/1,M10.5.0");
        assert_eq!(london.offset_at(utc(2026, 7, 1, 12, 0)), 3600);
        assert_eq!(london.offset_at(utc(2026, 12, 1, 12, 0)), 0);
    }

    #[test]
    fn us_second_sunday_of_march() {
        let ny = rule("EST5EDT,M3.2.0,M11.1.0");
        // 2026: 8 March 02:00 EST = 07:00 UTC; 1 November 02:00 EDT = 06:00 UTC
        assert_eq!(ny.offset_at(utc(2026, 3, 8, 6, 59)), -5 * 3600);
        assert_eq!(ny.offset_at(utc(2026, 3, 8, 7, 0)), -4 * 3600);
        assert_eq!(ny.offset_at(utc(2026, 11, 1, 5, 59)), -4 * 3600);
        assert_eq!(ny.offset_at(utc(2026, 11, 1, 6, 0)), -5 * 3600);
    }

    #[test]
    fn southern_summer_spans_the_new_year() {
        let sydney = rule("AEST-10AEDT,M10.1.0,M4.1.0/3");
        // 2026: 5 April 03:00 AEDT = 4 April 16:00 UTC; 4 October 02:00 AEST = 3 October 16:00 UTC
        assert_eq!(sydney.offset_at(utc(2026, 1, 15, 0, 0)), 11 * 3600);
        assert_eq!(sydney.offset_at(utc(2026, 4, 4, 15, 59)), 11 * 3600);
        assert_eq!(sydney.offset_at(utc(2026, 4, 4, 16, 0)), 10 * 3600);
        assert_eq!(sydney.offset_at(utc(2026, 10, 3, 15, 59)), 10 * 3600);
        assert_eq!(sydney.offset_at(utc(2026, 10, 3, 16, 0)), 11 * 3600);
        assert_eq!(sydney.offset_at(utc(2026, 12, 31, 23, 0)), 11 * 3600);
        let lord_howe = rule("<+1030>-10:30<+11>-11,M10.1.0,M4.1.0");
        assert_eq!(lord_howe.offset_at(utc(2026, 1, 15, 0, 0)), 11 * 3600);
        assert_eq!(
            lord_howe.offset_at(utc(2026, 7, 15, 0, 0)),
            10 * 3600 + 1800
        );
    }

    #[test]
    fn extended_times_and_day_forms() {
        // Israel: Friday before the last Sunday of March, "26:00" on the Thursday
        let jerusalem = rule("IST-2IDT,M3.4.4/26,M10.5.0");
        // 2026: the 4th Thursday of March is the 26th; 26:00 IST = 27 March 00:00 UTC
        assert_eq!(jerusalem.offset_at(utc(2026, 3, 26, 23, 59)), 2 * 3600);
        assert_eq!(jerusalem.offset_at(utc(2026, 3, 27, 0, 0)), 3 * 3600);
        // Greenland: "-1" = 22:00 the day before
        let nuuk = rule("<-02>2<-01>,M3.5.0/-1,M10.5.0/0");
        assert_eq!(nuuk.offset_at(utc(2026, 7, 1, 0, 0)), -3600);
        let julian = rule("XXX0YYY,J60,J300");
        assert_eq!(Date::Julian(60).days(2028), days_from_civil(2028, 3, 1));
        assert!(julian.is_dst(utc(2028, 3, 1, 3, 0)));
        assert_eq!(Date::Day(59).days(2028), days_from_civil(2028, 2, 29));
    }

    #[test]
    fn bad_strings() {
        for s in [
            "",
            "CE-1",
            "CET",
            "CET-1CEST",
            "CET-1CEST,M13.5.0,M10.5.0",
            "CET-1CEST,M3.5.0",
            "<+04-4",
            "X-1",
        ] {
            assert_eq!(Rule::parse(s), None, "{s}");
        }
    }

    #[test]
    fn years_from_seconds() {
        assert_eq!(civil_year(0), 1970);
        assert_eq!(civil_year(utc(2026, 12, 31, 23, 59)), 2026);
        assert_eq!(civil_year(utc(2027, 1, 1, 0, 0)), 2027);
        assert_eq!(civil_year(utc(2000, 2, 29, 12, 0)), 2000);
    }
}

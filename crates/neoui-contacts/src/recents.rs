// SPDX-License-Identifier: Apache-2.0
//! Recents (the call log's lists, docs/symbian-phone-sources.md §11): when a call was ("21:03", "Yesterday",
//! "14 Nov"), how long it lasted, and repeated calls from one number folded into one row with a count.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// (year, month 1-12, day) of a day count since 1970-01-01 (Howard Hinnant's civil_from_days).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// (local day number, minute of the day) of UTC milliseconds.
fn local(utc_ms: i64, offset_s: i32) -> (i64, i64) {
    let s = utc_ms.div_euclid(1000) + i64::from(offset_s);
    (s.div_euclid(86_400), s.rem_euclid(86_400) / 60)
}

/// The list's short form: the time today, "Yesterday", else the date; "" for an unknown time (0).
pub fn when(utc_ms: i64, now_ms: i64, offset_s: i32) -> String {
    if utc_ms == 0 {
        return String::new();
    }
    let ((day, min), (today, _)) = (local(utc_ms, offset_s), local(now_ms, offset_s));
    match today - day {
        0 => format!("{:02}:{:02}", min / 60, min % 60),
        1 => String::from("Yesterday"),
        _ => {
            let (_, m, d) = civil(day);
            format!("{d} {}", MONTHS[m as usize - 1])
        }
    }
}

/// The details' form: "Today 21:03", "Yesterday 09:15", "14 Nov 2026 21:03".
pub fn when_long(utc_ms: i64, now_ms: i64, offset_s: i32) -> String {
    if utc_ms == 0 {
        return String::from("Unknown");
    }
    let ((day, min), (today, _)) = (local(utc_ms, offset_s), local(now_ms, offset_s));
    let hm = format!("{:02}:{:02}", min / 60, min % 60);
    match today - day {
        0 => format!("Today {hm}"),
        1 => format!("Yesterday {hm}"),
        _ => {
            let (y, m, d) = civil(day);
            format!("{d} {} {y} {hm}", MONTHS[m as usize - 1])
        }
    }
}

/// "0:42", "12:03", "1:02:03".
pub fn duration(s: u32) -> String {
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// One row per number, newest first: (index of the newest call, how many calls). Numbers are compared by their
/// digits; calls without a number each keep their own row.
pub fn group<'a>(numbers: impl IntoIterator<Item = &'a str>) -> Vec<(usize, u32)> {
    let mut rows: Vec<(usize, u32, String)> = Vec::new();
    for (i, n) in numbers.into_iter().enumerate() {
        let d = phone_rules::number::digits(n);
        match rows.iter_mut().find(|r| !d.is_empty() && r.2 == d) {
            Some(r) => r.1 += 1,
            None => rows.push((i, 1, d)),
        }
    }
    rows.into_iter().map(|(i, c, _)| (i, c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-11-14 21:03:00 UTC
    const T: i64 = 1_794_690_180_000;
    const DAY: i64 = 86_400_000;

    #[test]
    fn short_and_long() {
        assert_eq!(when(T, T + 60_000, 0), "21:03");
        // in Yerevan (+4) it's already the 15th, 01:03
        assert_eq!(when(T, T, 4 * 3600), "01:03");
        assert_eq!(when(T - DAY, T, 0), "Yesterday");
        assert_eq!(when(T - 3 * DAY, T, 0), "11 Nov");
        assert_eq!(when(0, T, 0), "");
        assert_eq!(when_long(T, T, 0), "Today 21:03");
        assert_eq!(when_long(T - DAY, T, 0), "Yesterday 21:03");
        assert_eq!(when_long(T - 365 * DAY, T, 0), "14 Nov 2025 21:03");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
    }

    #[test]
    fn durations() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(42), "0:42");
        assert_eq!(duration(723), "12:03");
        assert_eq!(duration(3723), "1:02:03");
    }

    #[test]
    fn grouping() {
        let n = ["+37491", "+374 91", "", "", "055", "+37491"];
        assert_eq!(group(n), [(0, 3), (2, 1), (3, 1), (4, 1)]);
    }
}

// SPDX-License-Identifier: Apache-2.0
//! Settings -> Date & time: the date formats (the shade's date), the time zone as the page shows it and steps it,
//! and the network time from an HTTP `Date:` header (docs/neoui-shell.md "Date & time").

use super::face::{DAYS, LocalTime, MONTHS};
use super::world::CATALOGUE;
use alloc::format;
use alloc::string::String;

/// The date formats (`time.date_format`), as their sample on 13 November 2026.
pub const DATE_FORMATS: [&str; 4] = [
    "Friday, 13 November",
    "13.11.2026",
    "2026-11-13",
    "11/13/2026",
];

/// The date in format `format` (0..=3, see [`DATE_FORMATS`]; anything else is 0).
#[must_use]
pub fn date_text(t: &LocalTime, format: i32) -> String {
    let (y, m, d) = (t.year, t.month.clamp(1, 12), t.day);
    match format {
        1 => format!("{d:02}.{m:02}.{y}"),
        2 => format!("{y}-{m:02}-{d:02}"),
        3 => format!("{m:02}/{d:02}/{y}"),
        _ => format!(
            "{}, {} {}",
            DAYS[(t.weekday % 7) as usize],
            d,
            MONTHS[(m - 1) as usize]
        ),
    }
}

/// "UTC+04:00 · Yerevan": the offset (seconds east of UTC) and the first catalogue city on it, if any.
#[must_use]
pub fn zone_text(offset_s: i32) -> String {
    let sign = if offset_s < 0 { '-' } else { '+' };
    let m = offset_s.unsigned_abs() / 60;
    let utc = format!("UTC{sign}{:02}:{:02}", m / 60, m % 60);
    match CATALOGUE
        .iter()
        .find(|c| i32::from(c.offset_min) * 60 == offset_s)
    {
        Some(c) => format!("{utc} · {}", c.name),
        None => utc,
    }
}

/// The next zone after `offset_s` among the catalogue's offsets (a tap on the Time zone row), wrapping around.
#[must_use]
pub fn next_zone(offset_s: i32) -> i32 {
    let offsets = || CATALOGUE.iter().map(|c| i32::from(c.offset_min) * 60);
    offsets()
        .filter(|&o| o > offset_s)
        .min()
        .or_else(|| offsets().min())
        .unwrap_or(0)
}

/// What a network time should change, given the clock now (`(utc, offset)`): `Some((utc, offset))` when the time is
/// more than 2 s off or the zone differs, else `None`. `net_offset` = `None` keeps the current zone (an HTTP Date, or
/// the automatic zone off).
#[must_use]
pub fn correction(now: (i64, i32), net_utc: i64, net_offset: Option<i32>) -> Option<(i64, i32)> {
    let offset = net_offset.unwrap_or(now.1);
    ((net_utc - now.0).abs() > 2 || offset != now.1).then_some((net_utc, offset))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's days_from_civil).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The UTC time (Unix seconds) in an HTTP response's `Date:` header, in its one standard form (RFC 9110 IMF-fixdate,
/// "Date: Thu, 12 Nov 2026 19:50:00 GMT"); `None` if there is no such header or it doesn't parse.
#[must_use]
pub fn http_date(head: &[u8]) -> Option<u64> {
    let text = core::str::from_utf8(head).ok()?;
    let value = text.split("\r\n").find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("date").then(|| value.trim())
    })?;
    let mut f = value.split_ascii_whitespace();
    let (_weekday, day, month, year, hms, zone) = (
        f.next()?,
        f.next()?,
        f.next()?,
        f.next()?,
        f.next()?,
        f.next()?,
    );
    if zone != "GMT" {
        return None;
    }
    let month = 1 + MONTHS.iter().position(|m| m.get(..3) == Some(month))? as i64;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    let mut t = hms.split(':').map(|v| v.parse::<i64>().ok());
    let (h, m, s) = (t.next()??, t.next()??, t.next()??);
    if !(1..=31).contains(&day) || year < 2000 || h > 23 || m > 59 || s > 60 {
        return None;
    }
    let secs = days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s;
    u64::try_from(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRI: LocalTime = LocalTime {
        year: 2026,
        month: 11,
        day: 13,
        weekday: 4,
        hour: 9,
        minute: 5,
    };

    #[test]
    fn date_formats_match_their_samples() {
        for (i, sample) in DATE_FORMATS.iter().enumerate() {
            assert_eq!(date_text(&FRI, i as i32), *sample);
        }
        assert_eq!(date_text(&FRI, 9), DATE_FORMATS[0]);
    }

    #[test]
    fn zones_show_and_step() {
        assert_eq!(zone_text(4 * 3600), "UTC+04:00 · Yerevan");
        assert_eq!(zone_text(-(3 * 3600 + 1800)), "UTC-03:30");
        assert_eq!(zone_text(5 * 3600 + 45 * 60), "UTC+05:45 · Kathmandu");
        assert_eq!(next_zone(3 * 3600), 4 * 3600);
        assert_eq!(next_zone(4 * 3600), 5 * 3600 + 30 * 60); // Dubai shares +4: one stop per offset
        assert_eq!(next_zone(12 * 3600), -10 * 3600); // wraps
        assert_eq!(next_zone(7 * 60), 3600); // an offset off the list goes to the next one on it
    }

    #[test]
    fn corrections() {
        let now = (1_794_513_000, 4 * 3600);
        assert_eq!(correction(now, 1_794_513_002, None), None); // within 2 s
        assert_eq!(correction(now, 1_794_513_002, Some(4 * 3600)), None);
        assert_eq!(
            correction(now, 1_794_513_010, None),
            Some((1_794_513_010, 4 * 3600))
        );
        assert_eq!(
            correction(now, 1_794_512_990, None),
            Some((1_794_512_990, 4 * 3600))
        );
        assert_eq!(
            correction(now, 1_794_513_000, Some(3 * 3600)),
            Some((1_794_513_000, 3 * 3600))
        );
    }

    #[test]
    fn http_dates() {
        let head = b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nDate: Thu, 12 Nov 2026 19:50:07 GMT\r\n\r\n";
        assert_eq!(http_date(head), Some(1_794_513_007));
        assert_eq!(
            http_date(b"HTTP/1.1 204\r\ndate: Thu, 01 Jan 1970 00:00:00 GMT\r\n"),
            None
        ); // before 2000
        assert_eq!(
            http_date(b"HTTP/1.1 204\r\ndate: Sat, 29 Feb 2028 23:59:59 GMT\r\n"),
            Some(1_835_481_599)
        );
        assert_eq!(http_date(b"HTTP/1.1 204\r\n\r\n"), None);
        assert_eq!(http_date(b"Date: Thu, 12 Nov 2026 19:50:07 PST\r\n"), None);
        assert_eq!(http_date(b"Date: Thu, 12 Foo 2026 19:50:07 GMT\r\n"), None);
        assert_eq!(http_date(b"Date: Thu, 12 Nov 2026 19:50 GMT\r\n"), None);
    }
}

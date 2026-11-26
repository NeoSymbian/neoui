// SPDX-License-Identifier: Apache-2.0
//! `dtsrv.next`: the next alarm and the next event, as DTSRV (NeoSymbian/datetime) publishes them for the shell's
//! home clock widget and agenda card (datetime DT7). The blob, with [`super::wire`]:
//! `[1] i64 alarm_at, str alarm_label, i64 event_at, str event_title, u32 event_colour, u8 event_all_day`. Times are
//! Unix seconds UTC, 0 = none; an all-day event's time is the instant its local day starts. Bytes after these are
//! ignored, so DTSRV can append fields without a new version.

use super::alarm::{DAY_NAMES, format_hm};
use super::face::MONTHS;
use super::wire::{Corrupt, Reader, Writer};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const VERSION: u8 = 1;
const DAY: i64 = 86_400;

/// What `dtsrv.next` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Next {
    /// When the next alarm (or timer) rings, Unix seconds UTC; 0 = none.
    pub alarm_at: i64,
    pub alarm_label: String,
    /// When the next event starts, Unix seconds UTC; 0 = none.
    pub event_at: i64,
    pub event_title: String,
    /// The event's calendar colour, 0xRRGGBB.
    pub event_colour: u32,
    pub event_all_day: bool,
}

impl Next {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(VERSION);
        w.i64(self.alarm_at)
            .str(&self.alarm_label)
            .i64(self.event_at)
            .str(&self.event_title)
            .u32(self.event_colour)
            .u8(u8::from(self.event_all_day));
        w.finish()
    }

    /// An empty blob (DTSRV hasn't published yet) is "nothing next".
    ///
    /// # Errors
    /// [`Corrupt`] for a damaged blob or another version.
    pub fn decode(bytes: &[u8]) -> Result<Self, Corrupt> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        let mut r = Reader::new(bytes, VERSION)?;
        Ok(Self {
            alarm_at: r.i64()?,
            alarm_label: r.str()?,
            event_at: r.i64()?,
            event_title: r.str()?,
            event_colour: r.u32()?,
            event_all_day: r.u8()? != 0,
        })
    }

    /// The clock face's next alarm: "Today 22:00", "Tomorrow 7:15 AM", "Mon 07:15"; empty when none (or past).
    /// `offset_secs` is the local offset from UTC now.
    #[must_use]
    pub fn alarm_text(&self, now_utc: i64, offset_secs: i32, h24: bool) -> String {
        when(self.alarm_at, false, now_utc, offset_secs, h24)
    }

    /// The agenda card's time of the next event: as [`Self::alarm_text`], or "Today · All day".
    #[must_use]
    pub fn event_text(&self, now_utc: i64, offset_secs: i32, h24: bool) -> String {
        when(self.event_at, self.event_all_day, now_utc, offset_secs, h24)
    }
}

/// "Today 22:00", "Tomorrow …", the weekday within a week, "9 Dec …" beyond it; empty for none or an earlier day.
// shortcut: the local day uses today's offset, so across a summer-time change the time shown can be an hour off;
// pass the offset at `at` once the shell knows the zone's rule for every instant.
fn when(at: i64, all_day: bool, now_utc: i64, offset_secs: i32, h24: bool) -> String {
    if at == 0 {
        return String::new();
    }
    let local = at + i64::from(offset_secs);
    let (day, now_day) = (
        local.div_euclid(DAY),
        (now_utc + i64::from(offset_secs)).div_euclid(DAY),
    );
    let name = match day - now_day {
        ..0 => return String::new(),
        0 => String::from("Today"),
        1 => String::from("Tomorrow"),
        // 1970-01-01 was a Thursday (DAY_NAMES starts on Monday)
        2..7 => String::from(DAY_NAMES[(day + 3).rem_euclid(7) as usize]),
        _ => {
            let (d, m) = day_month(day);
            format!("{d} {}", &MONTHS[m as usize - 1][..3])
        }
    };
    if all_day {
        return format!("{name} · All day");
    }
    let secs = local.rem_euclid(DAY);
    // both fit: secs < 86 400
    format!(
        "{name} {}",
        format_hm((secs / 3600) as u8, (secs / 60 % 60) as u8, h24)
    )
}

/// Day of the month and month (1-based) of `days` since 1970-01-01 (H. Hinnant's `civil_from_days`).
fn day_month(days: i64) -> (i64, i64) {
    let z = days + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (d, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 9 November 2026, 20:30 UTC
    const MON: i64 = 1_794_256_200;
    const H: i64 = 3600;

    fn alarm(at: i64) -> Next {
        Next {
            alarm_at: at,
            ..Next::default()
        }
    }

    #[test]
    fn the_next_alarm_says_today_tomorrow_or_the_day() {
        assert_eq!(
            alarm(MON + H + 1800).alarm_text(MON, 0, true),
            "Today 22:00"
        );
        let tue_0715 = MON + 10 * H + 45 * 60;
        assert_eq!(
            alarm(tue_0715).alarm_text(MON, 0, false),
            "Tomorrow 7:15 AM"
        );
        let fri = MON + 4 * 24 * H;
        let next_mon_0715 = tue_0715 + 6 * 24 * H;
        assert_eq!(alarm(next_mon_0715).alarm_text(fri, 0, true), "Mon 07:15");
        assert_eq!(alarm(0).alarm_text(MON, 0, true), "");
        assert_eq!(alarm(MON - 24 * H).alarm_text(MON, 0, true), "");
    }

    #[test]
    fn the_local_offset_moves_the_day_and_the_time() {
        // at 20:30 UTC it is 23:30 on Monday in Moscow (+3 h), and 22:00 UTC is 01:00 on Tuesday there
        assert_eq!(
            alarm(MON + H + 1800).alarm_text(MON, 3 * 3600, true),
            "Tomorrow 01:00"
        );
        // and 20:30 UTC is still Monday 15:30 in New York (-5 h)
        assert_eq!(
            alarm(MON).alarm_text(MON - H, -5 * 3600, false),
            "Today 3:30 PM"
        );
    }

    #[test]
    fn events_far_off_and_all_day() {
        let e = Next {
            event_at: MON + 30 * 24 * H,
            ..Next::default()
        };
        assert_eq!(e.event_text(MON, 0, true), "9 Dec 20:30");
        let midnight = MON - 20 * H - 1800;
        let all_day = Next {
            event_at: midnight + 24 * H,
            event_all_day: true,
            ..Next::default()
        };
        assert_eq!(all_day.event_text(MON, 0, true), "Tomorrow · All day");
        assert_eq!(day_month(0), (1, 1));
        assert_eq!(day_month(MON.div_euclid(DAY)), (9, 11));
    }

    #[test]
    fn the_blob_round_trips_and_rejects_damage() {
        let n = Next {
            alarm_at: MON,
            alarm_label: "Wake up".into(),
            event_at: MON + H,
            event_title: "Kernel review".into(),
            event_colour: 0x8656e8,
            event_all_day: true,
        };
        let b = n.encode();
        assert_eq!(Next::decode(&b), Ok(n.clone()));
        assert_eq!(Next::decode(&[]), Ok(Next::default()));
        assert_eq!(Next::decode(&b[..b.len() - 1]), Err(Corrupt));
        let mut newer = b.clone();
        newer[0] = 2;
        assert_eq!(Next::decode(&newer), Err(Corrupt));
        let mut longer = b;
        longer.push(7);
        assert_eq!(Next::decode(&longer), Ok(n));
    }
}

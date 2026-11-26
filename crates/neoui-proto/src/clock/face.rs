// SPDX-License-Identifier: Apache-2.0
//! The clock face as text and numbers: the hour in 24- or 12-hour form, the date line and the next alarm. The UI
//! server (the home clock widget) and the Clock app (its face page) both show this, so they always agree. The next
//! alarm's text comes from DTSRV's `dtsrv.next` ([`super::next::Next::alarm_text`]).

use super::alarm::Minute;
use alloc::format;
use alloc::string::String;

/// Full day names, Monday first (the kernel's weekday 0 is Monday).
pub const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
/// Month names, January first.
pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A local date and time, as the kernel reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    /// 1-12
    pub month: u32,
    /// 1-31
    pub day: u32,
    /// 0 = Monday
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

impl LocalTime {
    /// This time as a minute of the week (for alarms).
    #[must_use]
    pub fn minute_of_week(&self) -> Minute {
        // all within u8: weekday < 7, hour < 24, minute < 60
        Minute {
            weekday: (self.weekday % 7) as u8,
            hour: self.hour.min(23) as u8,
            minute: self.minute.min(59) as u8,
        }
    }
}

/// What a clock face shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    /// The hour as shown: 0-23, or 1-12 on a 12-hour clock.
    pub hours: u32,
    pub minutes: u32,
    pub h24: bool,
    /// "AM"/"PM" on a 12-hour clock, else empty.
    pub am_pm: &'static str,
    /// "Monday, 9 November".
    pub date: String,
    /// "Today 07:30", "Tomorrow 7:30 AM", "Sat 06:30"; empty when no alarm is on.
    pub next_alarm: String,
}

impl Face {
    /// `next_alarm`: "Today 07:30" and the like, empty when none ([`super::next::Next::alarm_text`]).
    #[must_use]
    pub fn new(t: &LocalTime, h24: bool, next_alarm: &str) -> Self {
        let hour = t.hour.min(23);
        let (hours, am_pm) = if h24 {
            (hour, "")
        } else {
            ((hour + 11) % 12 + 1, if hour < 12 { "AM" } else { "PM" })
        };
        let date = format!(
            "{}, {} {}",
            DAYS[(t.weekday % 7) as usize],
            t.day,
            MONTHS[(t.month.clamp(1, 12) - 1) as usize]
        );
        Self {
            hours,
            minutes: t.minute.min(59),
            h24,
            am_pm,
            date,
            next_alarm: next_alarm.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monday 9 November 2026, 20:30
    const MON: LocalTime = LocalTime {
        year: 2026,
        month: 11,
        day: 9,
        weekday: 0,
        hour: 20,
        minute: 30,
    };

    #[test]
    fn hours_and_dates_in_both_modes() {
        let f = Face::new(&MON, true, "");
        assert_eq!(
            (f.hours, f.minutes, f.am_pm, f.date.as_str()),
            (20, 30, "", "Monday, 9 November")
        );
        assert_eq!(f.next_alarm, "");
        let f = Face::new(&MON, false, "");
        assert_eq!((f.hours, f.am_pm), (8, "PM"));
        let midnight = LocalTime { hour: 0, ..MON };
        assert_eq!(Face::new(&midnight, false, "").hours, 12);
        assert_eq!(Face::new(&midnight, false, "").am_pm, "AM");
    }

    #[test]
    fn out_of_range_kernel_fields_do_not_panic() {
        let odd = LocalTime {
            month: 0,
            weekday: 9,
            hour: 30,
            minute: 99,
            ..MON
        };
        let f = Face::new(&odd, true, "");
        assert_eq!((f.hours, f.minutes), (23, 59));
        assert!(f.date.ends_with("January"));
    }
}

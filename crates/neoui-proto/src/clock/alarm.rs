// SPDX-License-Identifier: Apache-2.0
//! What the shell's clock face needs of alarms: a minute of the week, the short day names and the "07:30" / "7:30 AM"
//! format. The alarms themselves live in DTSRV (NeoSymbian/datetime `datetime-core::alarm`, which re-exports these),
//! and the shell shows the next one from `dtsrv.next` ([`super::next`]).

use alloc::string::String;
use core::fmt::Write as _;

const MINUTES_PER_DAY: u32 = 24 * 60;
const MINUTES_PER_WEEK: u32 = 7 * MINUTES_PER_DAY;

/// Short day names, Monday first (the kernel's weekday 0 is Monday).
pub const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A minute of the week in local time: what alarms are compared against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Minute {
    /// 0 = Monday .. 6 = Sunday.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

impl Minute {
    /// Minutes since Monday 00:00.
    #[must_use]
    pub fn of_week(&self) -> u32 {
        u32::from(self.weekday % 7) * MINUTES_PER_DAY
            + u32::from(self.hour) * 60
            + u32::from(self.minute)
    }

    /// This minute plus `minutes` (wrapping around the week).
    #[must_use]
    pub fn plus(&self, minutes: u32) -> Self {
        let m = (self.of_week() + minutes) % MINUTES_PER_WEEK;
        // all three fit their types: m < 7 days of minutes
        Self {
            weekday: (m / MINUTES_PER_DAY) as u8,
            hour: (m % MINUTES_PER_DAY / 60) as u8,
            minute: (m % 60) as u8,
        }
    }
}

/// "07:30" or "7:30 AM".
#[must_use]
pub fn format_hm(hour: u8, minute: u8, h24: bool) -> String {
    let mut s = String::new();
    // writing into a String cannot fail
    let _ = if h24 {
        write!(s, "{hour:02}:{minute:02}")
    } else {
        let h12 = (hour + 11) % 12 + 1;
        write!(
            s,
            "{h12}:{minute:02} {}",
            if hour < 12 { "AM" } else { "PM" }
        )
    };
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON_0700: Minute = Minute {
        weekday: 0,
        hour: 7,
        minute: 0,
    };

    #[test]
    fn times_format_in_both_modes() {
        assert_eq!(format_hm(7, 5, true), "07:05");
        assert_eq!(format_hm(0, 5, false), "12:05 AM");
        assert_eq!(format_hm(12, 0, false), "12:00 PM");
        assert_eq!(format_hm(23, 59, false), "11:59 PM");
    }

    #[test]
    fn minutes_wrap_around_the_week() {
        let sun = Minute {
            weekday: 6,
            hour: 23,
            minute: 59,
        };
        assert_eq!(
            sun.plus(1),
            Minute {
                weekday: 0,
                hour: 0,
                minute: 0
            }
        );
        assert_eq!(MON_0700.plus(0), MON_0700);
    }
}

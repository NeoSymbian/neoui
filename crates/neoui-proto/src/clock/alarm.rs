// SPDX-License-Identifier: Apache-2.0
//! Alarms: when they ring, which comes next, and the `clock.alarms` settings blob. The UI server's scheduler and
//! the Clock app share this model, so "due" means the same thing to both.

use super::wire::{Corrupt, Reader, Writer};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

/// Most alarms kept.
pub const ALARMS_MAX: usize = 16;
/// Longest label in bytes.
pub const LABEL_MAX: usize = 24;
/// How long Snooze puts an alarm off, in minutes.
pub const SNOOZE_MIN: u16 = 5;
const MINUTES_PER_DAY: u32 = 24 * 60;
const MINUTES_PER_WEEK: u32 = 7 * MINUTES_PER_DAY;
const BLOB_VERSION: u8 = 1;

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

/// The days an alarm repeats on: bit 0 = Monday .. bit 6 = Sunday; none = rings once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Weekdays(u8);

impl Weekdays {
    pub const ONCE: Self = Self(0);
    pub const WORKDAYS: Self = Self(0b001_1111);
    pub const WEEKEND: Self = Self(0b110_0000);
    pub const EVERY_DAY: Self = Self(0b111_1111);

    /// From the stored bits (the eighth bit is ignored).
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::EVERY_DAY.0)
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn is_once(self) -> bool {
        self.0 == 0
    }

    /// Whether it repeats on `weekday` (0 = Monday).
    #[must_use]
    pub const fn contains(self, weekday: u8) -> bool {
        weekday < 7 && self.0 & (1 << weekday) != 0
    }

    /// With `weekday` switched on or off.
    #[must_use]
    pub const fn toggled(self, weekday: u8) -> Self {
        if weekday < 7 {
            Self(self.0 ^ (1 << weekday))
        } else {
            self
        }
    }

    /// "Once", "Every day", "Weekdays", "Weekends" or the days ("Mon, Wed, Fri").
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Self::ONCE => "Once".into(),
            Self::EVERY_DAY => "Every day".into(),
            Self::WORKDAYS => "Weekdays".into(),
            Self::WEEKEND => "Weekends".into(),
            _ => {
                let mut s = String::new();
                for (d, name) in DAY_NAMES.iter().enumerate() {
                    // d < 7
                    if self.contains(d as u8) {
                        if !s.is_empty() {
                            s.push_str(", ");
                        }
                        s.push_str(name);
                    }
                }
                s
            }
        }
    }
}

/// One alarm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alarm {
    /// Stable while the alarm exists (list rows and `AppEvent::Alarm` name it).
    pub id: u16,
    /// 0..=23
    pub hour: u8,
    /// 0..=59
    pub minute: u8,
    pub days: Weekdays,
    /// At most [`LABEL_MAX`] bytes; empty shows as "Alarm".
    pub label: String,
    pub on: bool,
}

impl Alarm {
    /// "07:30" (24-hour) or "7:30 AM".
    #[must_use]
    pub fn time_text(&self, h24: bool) -> String {
        format_hm(self.hour, self.minute, h24)
    }

    /// Whether it rings in minute `now`.
    #[must_use]
    pub fn rings_at(&self, now: Minute) -> bool {
        self.on
            && self.hour == now.hour
            && self.minute == now.minute
            && (self.days.is_once() || self.days.contains(now.weekday))
    }

    /// Minutes from `now` until it next rings, counting `now`'s own minute as a full week away (it already rang).
    /// `None` when it is off.
    #[must_use]
    pub fn minutes_until(&self, now: Minute) -> Option<u32> {
        if !self.on {
            return None;
        }
        let at = u32::from(self.hour) * 60 + u32::from(self.minute);
        (1..=7)
            .chain(core::iter::once(0))
            .map(|ahead_days: u32| {
                let weekday = (u32::from(now.weekday) + ahead_days) % 7;
                let target = weekday * MINUTES_PER_DAY + at;
                let delta = (target + MINUTES_PER_WEEK - now.of_week()) % MINUTES_PER_WEEK;
                (weekday, if delta == 0 { MINUTES_PER_WEEK } else { delta })
            })
            // weekday < 7
            .filter(|&(weekday, _)| self.days.is_once() || self.days.contains(weekday as u8))
            .map(|(_, delta)| delta)
            .min()
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

/// The alarm list is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

/// All alarms, kept in time order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alarms {
    list: Vec<Alarm>,
    next_id: u16,
}

impl Alarms {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The alarms in time order.
    #[must_use]
    pub fn as_slice(&self) -> &[Alarm] {
        &self.list
    }

    #[must_use]
    pub fn get(&self, id: u16) -> Option<&Alarm> {
        self.list.iter().find(|a| a.id == id)
    }

    /// Adds an alarm (on, once, no label) and returns its id.
    ///
    /// # Errors
    /// [`Full`] at [`ALARMS_MAX`] alarms.
    pub fn add(&mut self, hour: u8, minute: u8, days: Weekdays, label: &str) -> Result<u16, Full> {
        if self.list.len() >= ALARMS_MAX {
            return Err(Full);
        }
        let id = self.fresh_id();
        self.list.push(Alarm {
            id,
            hour: hour.min(23),
            minute: minute.min(59),
            days,
            label: clip(label),
            on: true,
        });
        self.sort();
        Ok(id)
    }

    /// Replaces the alarm with `alarm.id`; false when there is none.
    pub fn update(&mut self, mut alarm: Alarm) -> bool {
        alarm.hour = alarm.hour.min(23);
        alarm.minute = alarm.minute.min(59);
        alarm.label = clip(&alarm.label);
        let Some(slot) = self.list.iter_mut().find(|a| a.id == alarm.id) else {
            return false;
        };
        *slot = alarm;
        self.sort();
        true
    }

    /// Switches an alarm on or off; false when there is none.
    pub fn set_on(&mut self, id: u16, on: bool) -> bool {
        self.list
            .iter_mut()
            .find(|a| a.id == id)
            .map(|a| a.on = on)
            .is_some()
    }

    /// Removes an alarm; false when there is none.
    pub fn remove(&mut self, id: u16) -> bool {
        let before = self.list.len();
        self.list.retain(|a| a.id != id);
        self.list.len() != before
    }

    /// The alarms that ring in minute `now`.
    pub fn due(&self, now: Minute) -> impl Iterator<Item = &Alarm> {
        self.list.iter().filter(move |a| a.rings_at(now))
    }

    /// After alarm `id` rang and was dismissed: a one-shot alarm switches off; a repeating one stays on.
    pub fn dismissed(&mut self, id: u16) {
        if let Some(a) = self
            .list
            .iter_mut()
            .find(|a| a.id == id && a.days.is_once())
        {
            a.on = false;
        }
    }

    /// Snooze: alarm `id` (dismissed as above) and a one-shot alarm [`SNOOZE_MIN`] minutes after `now`, labelled
    /// like it. Returns the new alarm's id.
    ///
    /// # Errors
    /// [`Full`] when there is no room for the snooze alarm.
    pub fn snooze(&mut self, id: u16, now: Minute) -> Result<u16, Full> {
        let label = self.get(id).map(|a| a.label.clone()).unwrap_or_default();
        self.dismissed(id);
        let at = now.plus(u32::from(SNOOZE_MIN));
        self.add(at.hour, at.minute, Weekdays::ONCE, &label)
    }

    /// The next alarm to ring after `now` and the minutes until then.
    #[must_use]
    pub fn next(&self, now: Minute) -> Option<(&Alarm, u32)> {
        self.list
            .iter()
            .filter_map(|a| a.minutes_until(now).map(|m| (a, m)))
            .min_by_key(|&(_, m)| m)
    }

    /// The `clock.alarms` blob.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(BLOB_VERSION);
        w.u16(self.next_id);
        // at most ALARMS_MAX entries
        w.u8(self.list.len() as u8);
        for a in &self.list {
            w.u16(a.id)
                .u8(a.hour)
                .u8(a.minute)
                .u8(a.days.bits())
                .u8(u8::from(a.on))
                .str(&a.label);
        }
        w.finish()
    }

    /// Reads a `clock.alarms` blob.
    ///
    /// # Errors
    /// [`Corrupt`] for a damaged blob, out-of-range fields or more than [`ALARMS_MAX`] alarms.
    pub fn decode(bytes: &[u8]) -> Result<Self, Corrupt> {
        let mut r = Reader::new(bytes, BLOB_VERSION)?;
        let next_id = r.u16()?;
        let n = usize::from(r.u8()?);
        if n > ALARMS_MAX {
            return Err(Corrupt);
        }
        let mut list = Vec::with_capacity(n);
        for _ in 0..n {
            let (id, hour, minute, days, on) = (r.u16()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            let label = r.str()?;
            if hour > 23 || minute > 59 || on > 1 || label.len() > LABEL_MAX {
                return Err(Corrupt);
            }
            list.push(Alarm {
                id,
                hour,
                minute,
                days: Weekdays::from_bits(days),
                label,
                on: on == 1,
            });
        }
        r.end()?;
        let mut alarms = Self { list, next_id };
        alarms.sort();
        Ok(alarms)
    }

    fn fresh_id(&mut self) -> u16 {
        // ids are never reused while the alarm with that id exists
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if self.get(id).is_none() {
                return id;
            }
        }
    }

    fn sort(&mut self) {
        self.list.sort_by_key(|a| (a.hour, a.minute, a.id));
    }
}

fn clip(label: &str) -> String {
    let mut end = label.len().min(LABEL_MAX);
    while !label.is_char_boundary(end) {
        end -= 1;
    }
    label[..end].into()
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
    fn weekdays_describe_and_toggle() {
        assert_eq!(Weekdays::ONCE.describe(), "Once");
        assert_eq!(Weekdays::WORKDAYS.describe(), "Weekdays");
        assert_eq!(Weekdays::WEEKEND.describe(), "Weekends");
        assert_eq!(Weekdays::EVERY_DAY.describe(), "Every day");
        let mwf = Weekdays::ONCE.toggled(0).toggled(2).toggled(4);
        assert_eq!(mwf.describe(), "Mon, Wed, Fri");
        assert!(mwf.contains(2) && !mwf.contains(1) && !mwf.contains(9));
        assert_eq!(mwf.toggled(2).toggled(4).toggled(0), Weekdays::ONCE);
        assert_eq!(Weekdays::from_bits(0xFF), Weekdays::EVERY_DAY);
    }

    #[test]
    fn times_format_in_both_modes() {
        assert_eq!(format_hm(7, 5, true), "07:05");
        assert_eq!(format_hm(0, 5, false), "12:05 AM");
        assert_eq!(format_hm(12, 0, false), "12:00 PM");
        assert_eq!(format_hm(23, 59, false), "11:59 PM");
    }

    #[test]
    fn rings_on_its_days_or_once() {
        let mut a = Alarms::new();
        let once = a.add(7, 0, Weekdays::ONCE, "").expect("room");
        let weekend = a.add(7, 0, Weekdays::WEEKEND, "").expect("room");
        let due: Vec<u16> = a.due(MON_0700).map(|x| x.id).collect();
        assert_eq!(due, [once]);
        let sat = Minute {
            weekday: 5,
            ..MON_0700
        };
        let due: Vec<u16> = a.due(sat).map(|x| x.id).collect();
        assert_eq!(due, [once, weekend]);
        assert!(a.set_on(once, false));
        assert_eq!(a.due(MON_0700).count(), 0);
    }

    #[test]
    fn next_alarm_skips_off_days_and_wraps_the_week() {
        let mut a = Alarms::new();
        let weekend = a.add(6, 30, Weekdays::WEEKEND, "").expect("room");
        // Monday 07:00 -> Saturday 06:30 is 4 days 23.5 hours away
        let (next, mins) = a.next(MON_0700).expect("one on");
        assert_eq!(next.id, weekend);
        assert_eq!(mins, 4 * 24 * 60 + 23 * 60 + 30);
        // Sunday 23:00 -> Saturday 06:30 wraps the week
        let sun = Minute {
            weekday: 6,
            hour: 23,
            minute: 0,
        };
        assert_eq!(a.next(sun).map(|(_, m)| m), Some(5 * 24 * 60 + 7 * 60 + 30));
        let once = a.add(7, 1, Weekdays::ONCE, "").expect("room");
        assert_eq!(a.next(MON_0700).map(|(x, m)| (x.id, m)), Some((once, 1)));
        // in its own minute it already rang: a one-shot's next time is a day away, at the earliest
        let at = Minute {
            minute: 1,
            ..MON_0700
        };
        assert_eq!(a.get(once).and_then(|x| x.minutes_until(at)), Some(24 * 60));
    }

    #[test]
    fn dismiss_and_snooze() {
        let mut a = Alarms::new();
        let once = a.add(7, 0, Weekdays::ONCE, "Gym").expect("room");
        let daily = a.add(8, 0, Weekdays::EVERY_DAY, "").expect("room");
        a.dismissed(daily);
        assert!(a.get(daily).is_some_and(|x| x.on), "repeating stays on");
        let late = Minute {
            hour: 23,
            minute: 58,
            ..MON_0700
        };
        let snoozed = a.snooze(once, late).expect("room");
        assert!(a.get(once).is_some_and(|x| !x.on), "one-shot goes off");
        let s = a.get(snoozed).expect("added");
        assert_eq!((s.hour, s.minute, s.label.as_str()), (0, 3, "Gym"));
        assert!(s.days.is_once());
    }

    #[test]
    fn the_list_is_capped_and_kept_in_time_order() {
        let mut a = Alarms::new();
        for i in 0..ALARMS_MAX {
            a.add(23 - i as u8, 0, Weekdays::ONCE, "").expect("room");
        }
        assert_eq!(a.add(1, 0, Weekdays::ONCE, ""), Err(Full));
        let hours: Vec<u8> = a.as_slice().iter().map(|x| x.hour).collect();
        assert!(hours.windows(2).all(|w| w[0] <= w[1]));
        let first = a.as_slice()[0].id;
        assert!(a.remove(first));
        assert!(!a.remove(first));
        assert!(a.add(1, 0, Weekdays::ONCE, "").is_ok());
    }

    #[test]
    fn labels_and_fields_are_clamped() {
        let mut a = Alarms::new();
        let id = a
            .add(99, 99, Weekdays::ONCE, &"x".repeat(40))
            .expect("room");
        let x = a.get(id).expect("added").clone();
        assert_eq!((x.hour, x.minute, x.label.len()), (23, 59, LABEL_MAX));
    }

    #[test]
    fn the_blob_round_trips_and_rejects_damage() {
        let mut a = Alarms::new();
        a.add(6, 45, Weekdays::WORKDAYS, "Work").expect("room");
        let id = a.add(9, 0, Weekdays::ONCE, "").expect("room");
        a.set_on(id, false);
        let b = a.encode();
        assert_eq!(Alarms::decode(&b), Ok(a.clone()));
        assert_eq!(Alarms::decode(&b[..b.len() - 1]), Err(Corrupt));
        let mut bad = b.clone();
        bad[6] = 24; // the first alarm's hour: version, next id (2), count, id (2), hour
        assert_eq!(Alarms::decode(&bad), Err(Corrupt));
        // a decoded list keeps handing out fresh ids
        let mut back = Alarms::decode(&b).expect("valid");
        let new = back.add(1, 0, Weekdays::ONCE, "").expect("room");
        assert!(a.as_slice().iter().all(|x| x.id != new));
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

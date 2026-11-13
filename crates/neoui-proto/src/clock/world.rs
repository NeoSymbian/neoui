// SPDX-License-Identifier: Apache-2.0
//! World clocks: a catalogue of cities, the time there, and the `clock.cities` settings blob (city names, so the
//! catalogue can grow or be reordered without breaking saved lists).

use super::alarm::format_hm;
use super::tz;
use super::wire::{Corrupt, Reader, Writer};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Most cities in the list.
pub const CITIES_MAX: usize = 12;
const BLOB_VERSION: u8 = 1;
const SECONDS_PER_DAY: i64 = 86_400;

/// A city and its time zone rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct City {
    pub name: &'static str,
    /// The zone's rule as a POSIX `TZ` string ([`tz::Rule`]), from IANA tzdata.
    pub tz: &'static str,
}

// shortcut: today's rules only (IANA tzdata 2025), no history; update the table when a country changes its rules.
/// The cities offered, west to east by standard offset.
pub static CATALOGUE: &[City] = &[
    City {
        name: "Pago Pago",
        tz: "SST11",
    },
    City {
        name: "Honolulu",
        tz: "HST10",
    },
    City {
        name: "Anchorage",
        tz: "AKST9AKDT,M3.2.0,M11.1.0",
    },
    City {
        name: "Los Angeles",
        tz: "PST8PDT,M3.2.0,M11.1.0",
    },
    City {
        name: "Vancouver",
        tz: "PST8PDT,M3.2.0,M11.1.0",
    },
    City {
        name: "Phoenix",
        tz: "MST7",
    },
    City {
        name: "Denver",
        tz: "MST7MDT,M3.2.0,M11.1.0",
    },
    City {
        name: "Mexico City",
        tz: "CST6",
    },
    City {
        name: "Chicago",
        tz: "CST6CDT,M3.2.0,M11.1.0",
    },
    City {
        name: "Bogotá",
        tz: "<-05>5",
    },
    City {
        name: "Lima",
        tz: "<-05>5",
    },
    City {
        name: "New York",
        tz: "EST5EDT,M3.2.0,M11.1.0",
    },
    City {
        name: "Toronto",
        tz: "EST5EDT,M3.2.0,M11.1.0",
    },
    City {
        name: "Havana",
        tz: "CST5CDT,M3.2.0/0,M11.1.0/1",
    },
    City {
        name: "Caracas",
        tz: "<-04>4",
    },
    City {
        name: "Halifax",
        tz: "AST4ADT,M3.2.0,M11.1.0",
    },
    City {
        name: "Santiago",
        tz: "<-04>4<-03>,M9.1.6/24,M4.1.6/24",
    },
    City {
        name: "St. John's",
        tz: "NST3:30NDT,M3.2.0,M11.1.0",
    },
    City {
        name: "São Paulo",
        tz: "<-03>3",
    },
    City {
        name: "Buenos Aires",
        tz: "<-03>3",
    },
    City {
        name: "Nuuk",
        tz: "<-02>2<-01>,M3.5.0/-1,M10.5.0/0",
    },
    City {
        name: "Praia",
        tz: "<-01>1",
    },
    City {
        name: "Azores",
        tz: "<-01>1<+00>,M3.5.0/0,M10.5.0/1",
    },
    City {
        name: "Reykjavik",
        tz: "GMT0",
    },
    City {
        name: "London",
        tz: "GMT0BST,M3.5.0/1,M10.5.0",
    },
    City {
        name: "Dublin",
        tz: "GMT0IST,M3.5.0/1,M10.5.0",
    },
    City {
        name: "Lisbon",
        tz: "WET0WEST,M3.5.0/1,M10.5.0",
    },
    City {
        name: "Lagos",
        tz: "WAT-1",
    },
    City {
        name: "Paris",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Berlin",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Madrid",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Rome",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Amsterdam",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Stockholm",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Vienna",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Warsaw",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Prague",
        tz: "CET-1CEST,M3.5.0,M10.5.0/3",
    },
    City {
        name: "Cairo",
        tz: "EET-2EEST,M4.5.5/0,M10.5.4/24",
    },
    City {
        name: "Johannesburg",
        tz: "SAST-2",
    },
    City {
        name: "Athens",
        tz: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    City {
        name: "Bucharest",
        tz: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    City {
        name: "Helsinki",
        tz: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    City {
        name: "Kyiv",
        tz: "EET-2EEST,M3.5.0/3,M10.5.0/4",
    },
    City {
        name: "Beirut",
        tz: "EET-2EEST,M3.5.0/0,M10.5.0/0",
    },
    City {
        name: "Jerusalem",
        tz: "IST-2IDT,M3.4.4/26,M10.5.0",
    },
    City {
        name: "Istanbul",
        tz: "<+03>-3",
    },
    City {
        name: "Moscow",
        tz: "MSK-3",
    },
    City {
        name: "Nairobi",
        tz: "EAT-3",
    },
    City {
        name: "Riyadh",
        tz: "<+03>-3",
    },
    City {
        name: "Baghdad",
        tz: "<+03>-3",
    },
    City {
        name: "Tehran",
        tz: "<+0330>-3:30",
    },
    City {
        name: "Dubai",
        tz: "<+04>-4",
    },
    City {
        name: "Yerevan",
        tz: "<+04>-4",
    },
    City {
        name: "Tbilisi",
        tz: "<+04>-4",
    },
    City {
        name: "Baku",
        tz: "<+04>-4",
    },
    City {
        name: "Kabul",
        tz: "<+0430>-4:30",
    },
    City {
        name: "Karachi",
        tz: "PKT-5",
    },
    City {
        name: "Tashkent",
        tz: "<+05>-5",
    },
    City {
        name: "Almaty",
        tz: "<+05>-5",
    },
    City {
        name: "Delhi",
        tz: "IST-5:30",
    },
    City {
        name: "Colombo",
        tz: "<+0530>-5:30",
    },
    City {
        name: "Kathmandu",
        tz: "<+0545>-5:45",
    },
    City {
        name: "Dhaka",
        tz: "<+06>-6",
    },
    City {
        name: "Yangon",
        tz: "<+0630>-6:30",
    },
    City {
        name: "Bangkok",
        tz: "<+07>-7",
    },
    City {
        name: "Ho Chi Minh City",
        tz: "<+07>-7",
    },
    City {
        name: "Jakarta",
        tz: "WIB-7",
    },
    City {
        name: "Singapore",
        tz: "<+08>-8",
    },
    City {
        name: "Beijing",
        tz: "CST-8",
    },
    City {
        name: "Hong Kong",
        tz: "HKT-8",
    },
    City {
        name: "Taipei",
        tz: "CST-8",
    },
    City {
        name: "Manila",
        tz: "PST-8",
    },
    City {
        name: "Perth",
        tz: "AWST-8",
    },
    City {
        name: "Seoul",
        tz: "KST-9",
    },
    City {
        name: "Tokyo",
        tz: "JST-9",
    },
    City {
        name: "Darwin",
        tz: "ACST-9:30",
    },
    City {
        name: "Adelaide",
        tz: "ACST-9:30ACDT,M10.1.0,M4.1.0/3",
    },
    City {
        name: "Brisbane",
        tz: "AEST-10",
    },
    City {
        name: "Sydney",
        tz: "AEST-10AEDT,M10.1.0,M4.1.0/3",
    },
    City {
        name: "Melbourne",
        tz: "AEST-10AEDT,M10.1.0,M4.1.0/3",
    },
    City {
        name: "Lord Howe",
        tz: "<+1030>-10:30<+11>-11,M10.1.0,M4.1.0",
    },
    City {
        name: "Nouméa",
        tz: "<+11>-11",
    },
    City {
        name: "Auckland",
        tz: "NZST-12NZDT,M9.5.0,M4.1.0/3",
    },
    City {
        name: "Fiji",
        tz: "<+12>-12",
    },
    City {
        name: "Chatham",
        tz: "<+1245>-12:45<+1345>,M9.5.0/2:45,M4.1.0/3:45",
    },
    City {
        name: "Tongatapu",
        tz: "<+13>-13",
    },
    City {
        name: "Kiritimati",
        tz: "<+14>-14",
    },
];

impl City {
    fn rule(&self) -> tz::Rule {
        tz::Rule::parse(self.tz).unwrap_or(tz::Rule::UTC)
    }

    /// The offset in force here at `utc` (Unix seconds), seconds east of UTC.
    #[must_use]
    pub fn offset_at(&self, utc: i64) -> i32 {
        self.rule().offset_at(utc)
    }

    /// The standard (winter) offset, seconds east of UTC.
    #[must_use]
    pub fn standard_offset(&self) -> i32 {
        self.rule().standard
    }

    /// Daylight saving time is in force here at `utc`.
    #[must_use]
    pub fn is_dst(&self, utc: i64) -> bool {
        self.rule().is_dst(utc)
    }
}

/// The catalogue entry named `name`.
#[must_use]
pub fn find(name: &str) -> Option<&'static City> {
    CATALOGUE.iter().find(|c| c.name == name)
}

/// What a world clock row shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    /// "14:05" or "2:05 PM".
    pub time: String,
    /// "Today", "Tomorrow" or "Yesterday", against the local date.
    pub day: &'static str,
    /// "Same time", "+3 h", "−4:30 h".
    pub difference: String,
}

impl City {
    /// The time at this city for `utc` (Unix seconds), against a local time zone `here_s` (seconds east of UTC).
    #[must_use]
    pub fn show(&self, utc: i64, here_s: i32, h24: bool) -> Shown {
        let there_s = i64::from(self.offset_at(utc));
        let there = utc + there_s;
        let here = utc + i64::from(here_s);
        let sod = there.rem_euclid(SECONDS_PER_DAY);
        // both < 24 / 60
        let (hour, minute) = ((sod / 3600) as u8, (sod % 3600 / 60) as u8);
        let day = match there.div_euclid(SECONDS_PER_DAY) - here.div_euclid(SECONDS_PER_DAY) {
            0 => "Today",
            d if d > 0 => "Tomorrow",
            _ => "Yesterday",
        };
        Shown {
            time: format_hm(hour, minute, h24),
            day,
            difference: difference(there_s - i64::from(here_s)),
        }
    }
}

/// "Same time", "+3 h", "−4:30 h".
fn difference(seconds: i64) -> String {
    if seconds == 0 {
        return "Same time".into();
    }
    let sign = if seconds > 0 { '+' } else { '−' };
    let m = seconds.unsigned_abs() / 60;
    match m % 60 {
        0 => format!("{sign}{} h", m / 60),
        r => format!("{sign}{}:{r:02} h", m / 60),
    }
}

/// The cities shown on the World page, in the order added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cities(Vec<&'static City>);

impl Default for Cities {
    /// London, New York and Tokyo.
    fn default() -> Self {
        Self(
            ["London", "New York", "Tokyo"]
                .iter()
                .filter_map(|n| find(n))
                .collect(),
        )
    }
}

/// The list is full, or already has the city.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddError {
    Full,
    Duplicate,
}

impl Cities {
    /// An empty list.
    #[must_use]
    pub fn empty() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn as_slice(&self) -> &[&'static City] {
        &self.0
    }

    /// Adds catalogue city `index`.
    ///
    /// # Errors
    /// [`AddError::Full`] at [`CITIES_MAX`], [`AddError::Duplicate`] when it is in the list already. An index
    /// outside the catalogue is ignored.
    pub fn add(&mut self, index: usize) -> Result<(), AddError> {
        let Some(city) = CATALOGUE.get(index) else {
            return Ok(());
        };
        if self.0.iter().any(|c| c.name == city.name) {
            return Err(AddError::Duplicate);
        }
        if self.0.len() >= CITIES_MAX {
            return Err(AddError::Full);
        }
        self.0.push(city);
        Ok(())
    }

    /// Removes the city at `position`; false when there is none.
    pub fn remove(&mut self, position: usize) -> bool {
        (position < self.0.len())
            .then(|| self.0.remove(position))
            .is_some()
    }

    /// The `clock.cities` blob.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new(BLOB_VERSION);
        // at most CITIES_MAX
        w.u8(self.0.len() as u8);
        for c in &self.0 {
            w.str(c.name);
        }
        w.finish()
    }

    /// Reads a `clock.cities` blob; names no longer in the catalogue are skipped.
    ///
    /// # Errors
    /// [`Corrupt`] for a damaged blob.
    pub fn decode(bytes: &[u8]) -> Result<Self, Corrupt> {
        let mut r = Reader::new(bytes, BLOB_VERSION)?;
        let n = usize::from(r.u8()?);
        if n > CITIES_MAX {
            return Err(Corrupt);
        }
        let mut list = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(city) = find(&r.str()?)
                && !list.iter().any(|c: &&City| c.name == city.name)
            {
                list.push(city);
            }
        }
        r.end()?;
        Ok(Self(list))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-09 20:30 UTC (a Monday)
    const UTC: i64 = 1_794_256_200;

    #[test]
    fn the_catalogue_has_unique_names_west_to_east() {
        for (i, c) in CATALOGUE.iter().enumerate() {
            assert!(
                CATALOGUE[i + 1..].iter().all(|d| d.name != c.name),
                "{}",
                c.name
            );
            assert!(tz::Rule::parse(c.tz).is_some(), "{} parses", c.name);
            assert!(
                CATALOGUE[i + 1..]
                    .iter()
                    .all(|d| d.standard_offset() >= c.standard_offset()),
                "{}",
                c.name
            );
        }
    }

    #[test]
    fn times_days_and_differences() {
        let yerevan = find("Yerevan").expect("listed");
        let tokyo = find("Tokyo").expect("listed");
        let ny = find("New York").expect("listed");
        let delhi = find("Delhi").expect("listed");
        let here = 4 * 3600; // Yerevan
        assert_eq!(
            yerevan.show(UTC, here, true),
            Shown {
                time: "00:30".into(),
                day: "Today",
                difference: "Same time".into()
            }
        );
        let t = tokyo.show(UTC, here, true);
        assert_eq!(
            (t.time.as_str(), t.day, t.difference.as_str()),
            ("05:30", "Today", "+5 h")
        );
        let n = ny.show(UTC, here, false);
        assert_eq!(
            (n.time.as_str(), n.day, n.difference.as_str()),
            ("3:30 PM", "Yesterday", "−9 h")
        );
        let d = delhi.show(UTC, 0, true);
        assert_eq!(
            (d.time.as_str(), d.day, d.difference.as_str()),
            ("02:00", "Tomorrow", "+5:30 h")
        );
    }

    #[test]
    fn the_list_adds_removes_and_caps() {
        let mut c = Cities::empty();
        assert_eq!(c.add(0), Ok(()));
        assert_eq!(c.add(0), Err(AddError::Duplicate));
        assert_eq!(
            c.add(CATALOGUE.len()),
            Ok(()),
            "outside the catalogue: ignored"
        );
        assert_eq!(c.as_slice().len(), 1);
        for i in 1..CITIES_MAX {
            c.add(i).expect("room");
        }
        assert_eq!(c.add(CITIES_MAX), Err(AddError::Full));
        assert!(c.remove(0));
        assert!(!c.remove(CITIES_MAX));
        assert_eq!(c.as_slice()[0].name, CATALOGUE[1].name);
    }

    #[test]
    fn the_blob_round_trips_and_drops_unknown_names() {
        let c = Cities::default();
        assert_eq!(Cities::decode(&c.encode()), Ok(c.clone()));
        let mut w = Writer::new(BLOB_VERSION);
        w.u8(3).str("Atlantis").str("Paris").str("Paris");
        let got = Cities::decode(&w.finish()).expect("valid");
        assert_eq!(got.as_slice().len(), 1);
        assert_eq!(got.as_slice()[0].name, "Paris");
        assert_eq!(Cities::decode(&[BLOB_VERSION, 1]), Err(Corrupt));
    }
}

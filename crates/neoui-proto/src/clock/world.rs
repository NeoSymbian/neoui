// SPDX-License-Identifier: Apache-2.0
//! World clocks: a catalogue of cities, the time there, and the `clock.cities` settings blob (city names, so the
//! catalogue can grow or be reordered without breaking saved lists).

use super::alarm::format_hm;
use super::wire::{Corrupt, Reader, Writer};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Most cities in the list.
pub const CITIES_MAX: usize = 12;
const BLOB_VERSION: u8 = 1;
const SECONDS_PER_DAY: i64 = 86_400;

/// A city and its standard UTC offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct City {
    pub name: &'static str,
    /// Minutes east of UTC.
    pub offset_min: i16,
}

// shortcut: fixed standard offsets, no daylight saving time; replace with a tz database when one is on the device.
/// The cities offered, west to east.
pub static CATALOGUE: &[City] = &[
    City {
        name: "Honolulu",
        offset_min: -10 * 60,
    },
    City {
        name: "Los Angeles",
        offset_min: -8 * 60,
    },
    City {
        name: "Denver",
        offset_min: -7 * 60,
    },
    City {
        name: "Chicago",
        offset_min: -6 * 60,
    },
    City {
        name: "New York",
        offset_min: -5 * 60,
    },
    City {
        name: "São Paulo",
        offset_min: -3 * 60,
    },
    City {
        name: "London",
        offset_min: 0,
    },
    City {
        name: "Paris",
        offset_min: 60,
    },
    City {
        name: "Berlin",
        offset_min: 60,
    },
    City {
        name: "Helsinki",
        offset_min: 2 * 60,
    },
    City {
        name: "Moscow",
        offset_min: 3 * 60,
    },
    City {
        name: "Yerevan",
        offset_min: 4 * 60,
    },
    City {
        name: "Dubai",
        offset_min: 4 * 60,
    },
    City {
        name: "Delhi",
        offset_min: 5 * 60 + 30,
    },
    City {
        name: "Kathmandu",
        offset_min: 5 * 60 + 45,
    },
    City {
        name: "Bangkok",
        offset_min: 7 * 60,
    },
    City {
        name: "Singapore",
        offset_min: 8 * 60,
    },
    City {
        name: "Beijing",
        offset_min: 8 * 60,
    },
    City {
        name: "Tokyo",
        offset_min: 9 * 60,
    },
    City {
        name: "Sydney",
        offset_min: 10 * 60,
    },
    City {
        name: "Auckland",
        offset_min: 12 * 60,
    },
];

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
        let there_s = i64::from(self.offset_min) * 60;
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
            assert!(
                CATALOGUE[i + 1..]
                    .iter()
                    .all(|d| d.offset_min >= c.offset_min),
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

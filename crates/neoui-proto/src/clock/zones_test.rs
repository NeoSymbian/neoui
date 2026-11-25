// SPDX-License-Identifier: Apache-2.0
//! The generated rules (zones.rs) against zic's own transitions, the ROM budget, and saved city names.

use super::world::find;
use super::zones::{CATALOGUE, TZDATA, YEARS_CHECKED};
use super::zones_ref::TRANSITIONS;

/// The names the 2025 table had (saved in `clock.cities` and `time.zone`): each must still resolve.
const SAVED_NAMES: &[&str] = &[
    "Pago Pago",
    "Honolulu",
    "Anchorage",
    "Los Angeles",
    "Vancouver",
    "Phoenix",
    "Denver",
    "Mexico City",
    "Chicago",
    "Bogotá",
    "Lima",
    "New York",
    "Toronto",
    "Havana",
    "Caracas",
    "Halifax",
    "Santiago",
    "St. John's",
    "São Paulo",
    "Buenos Aires",
    "Nuuk",
    "Praia",
    "Azores",
    "Reykjavik",
    "London",
    "Dublin",
    "Lisbon",
    "Lagos",
    "Paris",
    "Berlin",
    "Madrid",
    "Rome",
    "Amsterdam",
    "Stockholm",
    "Vienna",
    "Warsaw",
    "Prague",
    "Cairo",
    "Johannesburg",
    "Athens",
    "Bucharest",
    "Helsinki",
    "Kyiv",
    "Beirut",
    "Jerusalem",
    "Istanbul",
    "Moscow",
    "Nairobi",
    "Riyadh",
    "Baghdad",
    "Tehran",
    "Dubai",
    "Yerevan",
    "Tbilisi",
    "Baku",
    "Kabul",
    "Karachi",
    "Tashkent",
    "Almaty",
    "Delhi",
    "Colombo",
    "Kathmandu",
    "Dhaka",
    "Yangon",
    "Bangkok",
    "Ho Chi Minh City",
    "Jakarta",
    "Singapore",
    "Beijing",
    "Hong Kong",
    "Taipei",
    "Manila",
    "Perth",
    "Seoul",
    "Tokyo",
    "Darwin",
    "Adelaide",
    "Brisbane",
    "Sydney",
    "Melbourne",
    "Lord Howe",
    "Nouméa",
    "Auckland",
    "Fiji",
    "Chatham",
    "Tongatapu",
    "Kiritimati",
];

#[test]
fn rules_match_zic_2026_to_2030() {
    assert_eq!(TZDATA, "2025b");
    assert!(TRANSITIONS.len() > 300);
    assert_eq!(YEARS_CHECKED, (2026, 2030));
    // 2026-01-01 and 2031-01-01 UTC
    assert!(
        TRANSITIONS
            .iter()
            .all(|&(_, t, _, _)| (1_767_225_600..1_924_992_000).contains(&t))
    );
    for &(name, t, before, after) in TRANSITIONS {
        let city = find(name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(city.offset_at(t - 1), before, "{name} just before {t}");
        assert_eq!(city.offset_at(t), after, "{name} at {t}");
    }
}

#[test]
fn the_table_fits_its_rom_budget() {
    let bytes: usize = CATALOGUE
        .iter()
        .map(|c| c.name.len() + c.tz.len())
        .sum::<usize>()
        + core::mem::size_of_val(CATALOGUE);
    assert!(bytes <= 8 * 1024, "{bytes} bytes");
}

#[test]
fn saved_city_names_still_resolve() {
    assert_eq!(SAVED_NAMES.len(), 87);
    for name in SAVED_NAMES {
        assert!(find(name).is_some(), "{name}");
    }
}

#[test]
fn dublin_summer_time_is_summer() {
    // January and July 2026, noon UTC
    let (january, july) = (1_768_478_400, 1_784_116_800);
    let dublin = find("Dublin").unwrap();
    assert_eq!(dublin.standard_offset(), 0);
    assert!(!dublin.is_dst(january));
    assert!(dublin.is_dst(july));
    let london = find("London").unwrap();
    assert_eq!(london.is_dst(january), dublin.is_dst(january));
    assert_eq!(london.is_dst(july), dublin.is_dst(july));
    // every city: summer time is exactly when the clock is ahead of its winter offset
    for c in CATALOGUE {
        for t in [january, july] {
            assert_eq!(
                c.is_dst(t),
                c.offset_at(t) > c.standard_offset(),
                "{}",
                c.name
            );
        }
    }
}

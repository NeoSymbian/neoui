// SPDX-License-Identifier: Apache-2.0
//! What only the shell knows, as `ipc::SETTINGS_INFO` sends it to the Settings app: the battery's 24-hour history
//! ([`Battery`]), the Wi-Fi link's IP side and captive portal ([`crate::wifi::Ip`]) and where the clock's time came
//! from ([`Clock`]). Each encoding is bounded; a short or damaged buffer decodes to `None`.

use crate::wifi::{Internet, Ip};
use alloc::string::String;
use alloc::vec::Vec;

/// Minute columns in the battery history (24 hours).
pub const BATTERY_SLOTS: usize = 1440;
/// The longest encoding of any kind (the out buffer the app passes).
pub const INFO_MAX: usize = 1 + BATTERY_SLOTS;

/// Battery → its 24-hour chart: `[save_error: u8][slot: u8 × 1440]`, a slot 0..=100 percent or 255 = no reading.
#[derive(Clone, Debug, PartialEq)]
pub struct Battery {
    /// The history couldn't be read or saved (the page says so; live readings still show).
    pub save_error: bool,
    /// Oldest first; -1.0 = no reading, else 0.0..=1.0 (as `battery::History::bars`).
    pub bars: Vec<f32>,
}

impl Battery {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(INFO_MAX);
        b.push(u8::from(self.save_error));
        b.extend((0..BATTERY_SLOTS).map(|i| match self.bars.get(i) {
            Some(&v) if v >= 0.0 => (v.min(1.0) * 100.0 + 0.5) as u8,
            _ => 255,
        }));
        b
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != 1 + BATTERY_SLOTS || b[0] > 1 {
            return None;
        }
        let bars = b[1..]
            .iter()
            .map(|&v| if v > 100 { -1.0 } else { f32::from(v) / 100.0 })
            .collect();
        Some(Battery {
            save_error: b[0] == 1,
            bars,
        })
    }
}

/// The IP side of the link: `[has_address][address × 4][prefix][gateway × 4][dns × 4][internet]`.
#[must_use]
pub fn encode_ip(ip: &Ip) -> Vec<u8> {
    let mut b = Vec::with_capacity(15);
    let (address, prefix) = ip.address.unwrap_or(([0; 4], 0));
    b.push(u8::from(ip.address.is_some()));
    b.extend_from_slice(&address);
    b.push(prefix);
    b.extend_from_slice(&ip.gateway);
    b.extend_from_slice(&ip.dns);
    b.push(match ip.internet {
        Internet::Checking => 0,
        Internet::Ok => 1,
        Internet::NoAccess => 2,
        Internet::SignIn => 3,
    });
    b
}

#[must_use]
pub fn decode_ip(b: &[u8]) -> Option<Ip> {
    if b.len() != 15 || b[0] > 1 {
        return None;
    }
    let quad = |i: usize| [b[i], b[i + 1], b[i + 2], b[i + 3]];
    Some(Ip {
        address: (b[0] == 1).then(|| (quad(1), b[5])),
        gateway: quad(6),
        dns: quad(10),
        internet: match b[14] {
            0 => Internet::Checking,
            1 => Internet::Ok,
            2 => Internet::NoAccess,
            3 => Internet::SignIn,
            _ => return None,
        },
    })
}

/// The clock: the zone in force as Date & time shows it ("UTC+04:00 · Yerevan") and where the time last came from
/// ("Mobile network, 10:02" or "Not yet"): `[len][zone][len][synced]`, each at most 255 bytes of UTF-8.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    pub zone: String,
    pub synced: String,
}

impl Clock {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        for s in [&self.zone, &self.synced] {
            let s = clip(s, 255);
            b.push(s.len() as u8);
            b.extend_from_slice(s.as_bytes());
        }
        b
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let (zone, rest) = take_str(b)?;
        let (synced, rest) = take_str(rest)?;
        rest.is_empty().then_some(Clock { zone, synced })
    }
}

fn take_str(b: &[u8]) -> Option<(String, &[u8])> {
    let (&n, rest) = b.split_first()?;
    let n = usize::from(n);
    let s = core::str::from_utf8(rest.get(..n)?).ok()?;
    Some((String::from(s), &rest[n..]))
}

/// `s` cut to at most `max` bytes on a character boundary.
fn clip(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn battery_round_trips_with_gaps() {
        let mut bars = vec![-1.0; BATTERY_SLOTS];
        bars[0] = 0.72;
        bars[BATTERY_SLOTS - 1] = 1.0;
        let b = Battery {
            save_error: true,
            bars,
        };
        let e = b.encode();
        assert_eq!(e.len(), INFO_MAX);
        let d = Battery::decode(&e).unwrap();
        assert!(d.save_error);
        assert_eq!(d.bars[0], 0.72);
        assert_eq!(d.bars[1], -1.0);
        assert_eq!(d.bars[BATTERY_SLOTS - 1], 1.0);
        assert_eq!(Battery::decode(&e[..10]), None);
    }

    #[test]
    fn ip_round_trips() {
        let ip = Ip {
            address: Some(([192, 168, 3, 57], 24)),
            gateway: [192, 168, 3, 1],
            dns: [192, 168, 3, 1],
            internet: Internet::SignIn,
        };
        assert_eq!(decode_ip(&encode_ip(&ip)), Some(ip));
        assert_eq!(decode_ip(&encode_ip(&Ip::default())), Some(Ip::default()));
        let mut bad = encode_ip(&ip);
        bad[14] = 9;
        assert_eq!(decode_ip(&bad), None);
    }

    #[test]
    fn clock_round_trips_and_clips() {
        let c = Clock {
            zone: "UTC+04:00 · Yerevan".into(),
            synced: "Mobile network, 10:02".into(),
        };
        assert_eq!(Clock::decode(&c.encode()), Some(c));
        let long = Clock {
            zone: "é".repeat(200),
            synced: String::new(),
        };
        let d = Clock::decode(&long.encode()).unwrap();
        assert_eq!(d.zone.len(), 254); // 127 two-byte characters
        assert_eq!(Clock::decode(&[3, b'a']), None);
    }
}

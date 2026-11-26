// SPDX-License-Identifier: Apache-2.0
//! What only the shell knows, as `ipc::SETTINGS_INFO` sends it to the Settings app: the battery's 24-hour history
//! ([`Battery`]), the Wi-Fi link's IP side and captive portal ([`crate::wifi::Ip`]) and where the clock's time came
//! from ([`Clock`]), and since 1.16 Battery usage ([`encode_usage`]). Each encoding is bounded; a short or damaged
//! buffer decodes to `None`.

use crate::battery::Usage;
use crate::wifi::{Internet, Ip};
use alloc::string::String;
use alloc::vec::Vec;

/// Minute columns in the battery history (24 hours).
pub const BATTERY_SLOTS: usize = 1440;
/// The battery record's length.
pub const BATTERY_MAX: usize = 1 + BATTERY_SLOTS;
/// The longest encoding of any kind (the out buffer the app passes): the old apps' list since 1.14.
pub const INFO_MAX: usize = if OLD_APPS_MAX > BATTERY_MAX {
    OLD_APPS_MAX
} else {
    BATTERY_MAX
};

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
        let mut b = Vec::with_capacity(BATTERY_MAX);
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

/// The longest Battery usage record (1.16, `ipc::INFO_BATTERY_USAGE`).
pub const USAGE_MAX: usize = 28;

/// Battery usage: `[0]` before the history's first reading ("No data yet"), else `[1][since_full][start_utc i64 LE]
/// [end_utc i64 LE][start % or 255][end % or 255][on_battery_s u32 LE][gap_s u32 LE]` ([`USAGE_MAX`] bytes).
#[must_use]
pub fn encode_usage(u: Option<&Usage>) -> Vec<u8> {
    let Some(u) = u else {
        return alloc::vec![0];
    };
    let mut b = Vec::with_capacity(USAGE_MAX);
    b.push(1);
    b.push(u8::from(u.since_full));
    b.extend_from_slice(&u.start_utc.to_le_bytes());
    b.extend_from_slice(&u.end_utc.to_le_bytes());
    b.push(u.start_percent.map_or(255, |p| p.min(100)));
    b.push(u.end_percent.map_or(255, |p| p.min(100)));
    b.extend_from_slice(&u.on_battery_s.to_le_bytes());
    b.extend_from_slice(&u.gap_s.to_le_bytes());
    b
}

/// The other way: `Some(None)` = no data yet, `None` = damaged.
#[must_use]
pub fn decode_usage(b: &[u8]) -> Option<Option<Usage>> {
    if b == [0] {
        return Some(None);
    }
    if b.len() != USAGE_MAX || b[0] != 1 || b[1] > 1 {
        return None;
    }
    let level = |v: u8| match v {
        0..=100 => Some(Some(v)),
        255 => Some(None),
        _ => None,
    };
    let i64_at = |i: usize| i64::from_le_bytes(b[i..i + 8].try_into().unwrap_or_default());
    let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap_or_default());
    // the window can't end before it starts
    if i64_at(10) < i64_at(2) {
        return None;
    }
    Some(Some(Usage {
        since_full: b[1] == 1,
        start_utc: i64_at(2),
        end_utc: i64_at(10),
        start_percent: level(b[18])?,
        end_percent: level(b[19])?,
        on_battery_s: u32_at(20),
        gap_s: u32_at(24),
    }))
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

/// The old apps the legacy bridge registered (`ipc::INFO_APPS`): `[count u8][total u16 LE]` then `count` times
/// `[id u32 LE][len u8][caption]`. At most `ipc::ENTRIES_MAX` are listed (the shell keeps no more), each caption at
/// most `ipc::TITLE_MAX` bytes; `total` counts the ones the shell had to refuse too, so a page can say "64 of 70".
/// Empty: the bridge hasn't run since the phone started.
// shortcut: no icons (64 entries of 128 px ARGB would be 4 MB); a kind 5 asking for one small icon by id is the upgrade.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OldApps {
    pub total: u16,
    pub apps: Vec<(u32, String)>,
}

/// The longest [`OldApps`] record.
pub const OLD_APPS_MAX: usize = 3 + crate::ipc::ENTRIES_MAX * (5 + crate::ipc::TITLE_MAX);

impl OldApps {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let apps = &self.apps[..self.apps.len().min(crate::ipc::ENTRIES_MAX)];
        let total = self.total.max(apps.len() as u16);
        let mut b = alloc::vec![apps.len() as u8];
        b.extend_from_slice(&total.to_le_bytes());
        for (id, caption) in apps {
            let c = clip(caption, crate::ipc::TITLE_MAX);
            b.extend_from_slice(&id.to_le_bytes());
            b.push(c.len() as u8);
            b.extend_from_slice(c.as_bytes());
        }
        b
    }

    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        let (&[count, t0, t1], mut rest) = b.split_first_chunk::<3>()?;
        let total = u16::from_le_bytes([t0, t1]);
        if usize::from(count) > crate::ipc::ENTRIES_MAX || total < u16::from(count) {
            return None;
        }
        let mut apps = Vec::new();
        for _ in 0..count {
            let (id, r) = rest.split_first_chunk::<4>()?;
            let (caption, r) = take_str(r)?;
            if caption.len() > crate::ipc::TITLE_MAX {
                return None;
            }
            apps.push((u32::from_le_bytes(*id), caption));
            rest = r;
        }
        rest.is_empty().then_some(OldApps { total, apps })
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
    fn old_apps_round_trip_limits_and_the_total() {
        let a = OldApps {
            total: 2,
            apps: vec![
                (0x1000_0001, "X-plore".into()),
                (0x2000_0002, "Frozen Bubble".into()),
            ],
        };
        assert_eq!(OldApps::decode(&a.encode()), Some(a.clone()));
        assert_eq!(
            OldApps::decode(&OldApps::default().encode()),
            Some(OldApps::default())
        );
        // 64 entries of 48 bytes fill the record; more than the shell keeps are cut, the total says how many
        let full = OldApps {
            total: 70,
            apps: (0..70).map(|i| (i, "x".repeat(48))).collect(),
        };
        let b = full.encode();
        assert_eq!(b.len(), OLD_APPS_MAX);
        let d = OldApps::decode(&b).unwrap();
        assert_eq!((d.total, d.apps.len()), (70, 64));
        // a caption cut to 48 bytes on a character boundary
        let long = OldApps {
            total: 0,
            apps: vec![(1, "é".repeat(30))],
        };
        let d = OldApps::decode(&long.encode()).unwrap();
        assert_eq!((d.total, d.apps[0].1.len()), (1, 48));
        // damaged: short, trailing bytes, a total under the count, too many
        assert_eq!(OldApps::decode(&a.encode()[..5]), None);
        assert_eq!(OldApps::decode(&[a.encode(), vec![0]].concat()), None);
        assert_eq!(OldApps::decode(&[1, 0, 0, 1, 0, 0, 0, 0]), None);
        assert_eq!(OldApps::decode(&[65, 65, 0]), None);
        assert_eq!(OldApps::decode(&[]), None);
    }

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
        assert_eq!(e.len(), BATTERY_MAX);
        let d = Battery::decode(&e).unwrap();
        assert!(d.save_error);
        assert_eq!(d.bars[0], 0.72);
        assert_eq!(d.bars[1], -1.0);
        assert_eq!(d.bars[BATTERY_SLOTS - 1], 1.0);
        assert_eq!(Battery::decode(&e[..10]), None);
    }

    #[test]
    fn usage_round_trips_at_its_limits() {
        assert_eq!(encode_usage(None), vec![0]);
        assert_eq!(decode_usage(&[0]), Some(None));
        let most = Usage {
            since_full: true,
            start_utc: i64::MIN,
            end_utc: i64::MAX,
            start_percent: Some(100),
            end_percent: None,
            on_battery_s: u32::MAX,
            gap_s: u32::MAX,
        };
        let b = encode_usage(Some(&most));
        assert_eq!(b.len(), USAGE_MAX);
        const { assert!(USAGE_MAX <= INFO_MAX) }; // the out buffer the app passes takes it
        assert_eq!(decode_usage(&b), Some(Some(most)));
        // damaged: short, long, a bad flag, a level over 100 (not 255), the empty marker with more after it
        assert_eq!(decode_usage(&b[..USAGE_MAX - 1]), None);
        assert_eq!(decode_usage(&[b.clone(), vec![0]].concat()), None);
        let mut bad = b.clone();
        bad[1] = 2;
        assert_eq!(decode_usage(&bad), None);
        let mut bad = b.clone();
        bad[18] = 101;
        assert_eq!(decode_usage(&bad), None);
        let reversed = Usage {
            start_utc: 60,
            end_utc: 0,
            ..most
        };
        assert_eq!(decode_usage(&encode_usage(Some(&reversed))), None);
        assert_eq!(decode_usage(&[0, 0]), None);
        assert_eq!(decode_usage(&[]), None);
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

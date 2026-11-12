// SPDX-License-Identifier: Apache-2.0
//! Wi-Fi scan results from the kernel (HAL WlanScan 0x4e1e, kernel qcom_wcn3990.cpp; docs wifi.md W3):
//! `{version 1, state, count, scans % 256}` then per network `{bssid 6, channel, dBm (i8), flags (1 secured),
//! ssid length, ssid 32}`.
use alloc::{format, string::String, vec::Vec};

/// Where the radio is
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Off,
    Starting,
    Ready,
    Scanning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub bssid: [u8; 6],
    pub channel: u8,
    pub dbm: i8,
    pub secured: bool,
    /// empty = a hidden network
    pub ssid: String,
}

impl Network {
    /// 0..=4 bars, as phones draw them (-55 dBm and better is full)
    pub fn bars(&self) -> u8 {
        match self.dbm {
            d if d >= -55 => 4,
            d if d >= -66 => 3,
            d if d >= -77 => 2,
            d if d >= -88 => 1,
            _ => 0,
        }
    }

    /// "Excellent", "Good", "Fair", "Weak" or "Very weak" (the sheet's signal line)
    pub fn signal(&self) -> &'static str {
        ["Very weak", "Weak", "Fair", "Good", "Excellent"][usize::from(self.bars())]
    }

    pub fn band(&self) -> &'static str {
        if self.channel <= 14 {
            "2.4 GHz"
        } else {
            "5 GHz"
        }
    }

    /// The row's second line: "Secured · 5 GHz · -46 dBm"
    pub fn detail(&self) -> String {
        format!(
            "{} · {} · {} dBm",
            if self.secured { "Secured" } else { "Open" },
            self.band(),
            self.dbm
        )
    }

    pub fn title(&self) -> &str {
        if self.ssid.is_empty() {
            "Hidden network"
        } else {
            &self.ssid
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub state: State,
    /// completed scans, modulo 256: a new value means a new list
    pub scans: u8,
    pub networks: Vec<Network>,
}

pub const SIZE: usize = 4 + 32 * ENTRY;

/// An SSID as text; one with nothing printable in it (all zero bytes, control characters) counts as hidden
fn name(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    if s.chars()
        .all(|c| c.is_control() || c == char::REPLACEMENT_CHARACTER || c.is_whitespace())
    {
        String::new()
    } else {
        s.chars()
            .map(|c| if c.is_control() { '?' } else { c })
            .collect()
    }
}
const ENTRY: usize = 42;

impl Scan {
    pub fn decode(b: &[u8]) -> Option<Self> {
        let (&[version, state, count, scans], rest) = b.split_first_chunk::<4>()?;
        if version != 1 || count as usize > 32 || rest.len() != count as usize * ENTRY {
            return None;
        }
        let state = match state {
            0 => State::Off,
            1 => State::Starting,
            2 => State::Ready,
            3 => State::Scanning,
            4 => State::Error,
            _ => return None,
        };
        let mut networks = Vec::new();
        for e in rest.as_chunks::<ENTRY>().0 {
            let len = e[9] as usize;
            if len > 32 {
                return None;
            }
            networks.push(Network {
                bssid: e[..6].try_into().ok()?,
                channel: e[6],
                dbm: e[7] as i8,
                secured: e[8] & 1 != 0,
                ssid: name(&e[10..10 + len]),
            });
        }
        Some(Scan {
            state,
            scans,
            networks,
        })
    }

    /// One row per network name, its strongest access point first; hidden ones last
    pub fn list(&self) -> Vec<Network> {
        let mut v: Vec<Network> = Vec::new();
        for n in &self.networks {
            match v
                .iter_mut()
                .find(|m| !n.ssid.is_empty() && m.ssid == n.ssid)
            {
                Some(m) if m.dbm < n.dbm => *m = n.clone(),
                Some(_) => {}
                None => v.push(n.clone()),
            }
        }
        v.sort_by(|a, b| {
            a.ssid
                .is_empty()
                .cmp(&b.ssid.is_empty())
                .then(b.dbm.cmp(&a.dbm))
        });
        v
    }

    /// Networks with a name (hidden ones not counted), as both Settings pages show it
    pub fn named(&self) -> usize {
        self.list().iter().filter(|n| !n.ssid.is_empty()).count()
    }

    /// Access points that hide their name (the Wi-Fi page counts them instead of listing them)
    pub fn hidden(&self) -> usize {
        self.networks.iter().filter(|n| n.ssid.is_empty()).count()
    }

    /// The Wi-Fi row's subtitle
    pub fn status(&self) -> String {
        let n = self.named();
        match self.state {
            State::Off => String::from("Off"),
            State::Starting => String::from("Starting…"),
            State::Error => String::from("Not available"),
            State::Scanning if self.networks.is_empty() => String::from("Searching…"),
            _ => format!("{n} networks found · not connected"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(last: u8, ch: u8, dbm: i8, secured: bool, ssid: &str) -> Vec<u8> {
        let mut e = alloc::vec![
            2,
            0,
            0,
            0,
            0,
            last,
            ch,
            dbm as u8,
            secured as u8,
            ssid.len() as u8
        ];
        e.extend_from_slice(ssid.as_bytes());
        e.resize(ENTRY, 0);
        e
    }

    #[test]
    fn decodes_and_lists_strongest_per_name() {
        let mut b = alloc::vec![1, 2, 4, 7];
        b.extend(entry(1, 44, -70, true, "Home"));
        b.extend(entry(2, 11, -46, true, "Home"));
        b.extend(entry(3, 36, -90, false, ""));
        b.extend(entry(4, 1, -60, false, "Cafe"));
        let s = Scan::decode(&b).unwrap();
        assert_eq!((s.state, s.scans, s.networks.len()), (State::Ready, 7, 4));
        let l = s.list();
        assert_eq!(
            l.iter().map(|n| n.title()).collect::<Vec<_>>(),
            ["Home", "Cafe", "Hidden network"]
        );
        assert_eq!((l[0].dbm, l[0].bars(), l[0].band()), (-46, 4, "2.4 GHz"));
        assert_eq!(l[0].detail(), "Secured · 2.4 GHz · -46 dBm");
        assert_eq!(l[2].detail(), "Open · 5 GHz · -90 dBm");
        assert_eq!(s.status(), "2 networks found · not connected");
        assert_eq!(
            (s.named(), s.hidden(), l[0].signal(), l[2].signal()),
            (2, 1, "Excellent", "Very weak")
        );
    }

    #[test]
    fn unprintable_names_count_as_hidden() {
        assert_eq!(name(&[0; 30]), "");
        assert_eq!(name(&[0x01, 0x02, 0xff]), "");
        assert_eq!(name(b"Cafe\x01"), "Cafe?");
        assert_eq!(name("Բարև".as_bytes()), "Բարև");
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(Scan::decode(&[2, 2, 0, 0]), None); // version
        assert_eq!(Scan::decode(&[1, 9, 0, 0]), None); // state
        assert_eq!(Scan::decode(&[1, 2, 1, 0]), None); // count without entries
        let mut b = alloc::vec![1, 2, 1, 0];
        let mut e = entry(1, 1, -50, true, "x");
        e[9] = 40; // ssid length > 32
        b.extend(e);
        assert_eq!(Scan::decode(&b), None);
        assert_eq!(Scan::decode(&[1, 0, 0, 0]).unwrap().status(), "Off");
    }
}

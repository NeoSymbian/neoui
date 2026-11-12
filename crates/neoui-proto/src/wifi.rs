// SPDX-License-Identifier: Apache-2.0
//! Wi-Fi scan results from the kernel (HAL WlanScan 0x4e1e, kernel qcom_wcn3990.cpp; docs wifi.md W3):
//! `{version 1, state, count, scans % 256}` then per network `{bssid 6, channel, dBm (i8), flags (1 secured),
//! ssid length, ssid 32}`.
//! The join's status (HAL WlanLink 0x4e1f, W4): [`Link`].
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
    /// The BSSID as "aa:bb:cc:dd:ee:ff" (how the UI names an access point)
    pub fn bssid_text(&self) -> String {
        let b = self.bssid;
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }

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

/// Where the join is (HAL WlanLink 0x4e1f op 3, docs wifi.md W4)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkState {
    #[default]
    Idle,
    /// authenticating and associating
    Joining,
    /// associated: the key handshake runs
    Handshake,
    Connected,
    /// the AP dropped us (deauth/disassoc `reason`) or stopped answering
    Lost,
    /// the join failed (`status` from the AP, else `reason` is the driver's)
    Error,
}

/// The link status: `{version 1, state, aid, status, reason (u16 LE each), bssid 6, our MAC 6, rx data frames,
/// rx ARP/IP frames (u32 LE each), AP RSN element length, the element (<= 64 bytes)}`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub state: LinkState,
    pub aid: u16,
    /// the 802.11 status code of a refused authentication or association
    pub status: u16,
    /// the 802.11 reason code of a deauthentication or disassociation
    pub reason: u16,
    pub bssid: [u8; 6],
    pub mac: [u8; 6],
    pub rx_data: u32,
    pub rx_ip: u32,
    /// the joined AP's RSN element (id, length, body), for the handshake
    pub rsn: Vec<u8>,
}

pub const LINK_SIZE: usize = 29 + 64;

impl Link {
    pub fn decode(b: &[u8]) -> Option<Self> {
        let (&[version, state], _) = b.split_first_chunk::<2>()?;
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        if version != 1 || b.len() < 29 || b.len() != 29 + b[28] as usize || b[28] > 64 {
            return None;
        }
        let state = match state {
            0 => LinkState::Idle,
            1 => LinkState::Joining,
            2 => LinkState::Handshake,
            3 => LinkState::Connected,
            4 => LinkState::Lost,
            5 => LinkState::Error,
            _ => return None,
        };
        Some(Link {
            state,
            aid: u16_at(2),
            status: u16_at(4),
            reason: u16_at(6),
            bssid: b[8..14].try_into().ok()?,
            mac: b[14..20].try_into().ok()?,
            rx_data: u32_at(20),
            rx_ip: u32_at(24),
            rsn: b[29..].to_vec(),
        })
    }

    /// What the network sheet says about this join
    pub fn message(&self) -> String {
        match self.state {
            LinkState::Idle => String::new(),
            LinkState::Joining | LinkState::Handshake => String::from("Connecting…"),
            LinkState::Connected => String::from("Connected"),
            // 2 = previous authentication no longer valid, 15 = 4-way handshake timeout: the AP never accepted
            // our message 2, which is what a wrong password looks like from the station
            LinkState::Lost | LinkState::Error if matches!(self.reason, 2 | 15) => {
                String::from("Wrong password")
            }
            LinkState::Lost => format!("Disconnected (reason {})", self.reason),
            LinkState::Error if self.status != 0 => {
                format!("Refused by the network (status {})", self.status)
            }
            LinkState::Error => format!("Could not connect ({})", self.reason),
        }
    }
}

/// Networks the phone joined before, newest first: SSID and PMK (the password's hash, never the password), kept by
/// the UI server in its private directory. Encoding: per network `{ssid length, ssid, pmk 32}`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Saved(pub Vec<(Vec<u8>, [u8; 32])>);

impl Saved {
    /// How many networks are remembered; the oldest is forgotten past this
    pub const MAX: usize = 8;

    pub fn decode(mut b: &[u8]) -> Self {
        let mut v = Vec::new();
        while let Some((&n, rest)) = b.split_first() {
            let n = n as usize;
            if n == 0 || n > 32 || rest.len() < n + 32 || v.len() == Self::MAX {
                break; // a damaged tail is dropped, the networks before it kept
            }
            v.push((rest[..n].to_vec(), rest[n..n + 32].try_into().unwrap()));
            b = &rest[n + 32..];
        }
        Saved(v)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        for (ssid, pmk) in &self.0 {
            b.push(ssid.len() as u8);
            b.extend_from_slice(ssid);
            b.extend_from_slice(pmk);
        }
        b
    }

    pub fn pmk(&self, ssid: &[u8]) -> Option<[u8; 32]> {
        self.0.iter().find(|(s, _)| s == ssid).map(|(_, p)| *p)
    }

    /// Forget a network (Settings -> Wi-Fi -> its details -> Forget)
    pub fn forget(&mut self, ssid: &[u8]) {
        self.0.retain(|(s, _)| s != ssid);
    }

    /// Remember (or refresh) a network as the newest
    pub fn remember(&mut self, ssid: &[u8], pmk: [u8; 32]) {
        if ssid.is_empty() || ssid.len() > 32 {
            return;
        }
        self.0.retain(|(s, _)| s != ssid);
        self.0.insert(0, (ssid.to_vec(), pmk));
        self.0.truncate(Self::MAX);
    }
}

fn mac_text(b: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5]
    )
}

/// Whether the internet answers over the link (SYSSTART fetches a `generate_204` page once an address is held)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Internet {
    #[default]
    Checking,
    Ok,
    NoAccess,
}

/// The IP side of the link, from NEONET's status (docs wifi.md W5). `address` = `None` until DHCP gave one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ip {
    pub address: Option<([u8; 4], u8)>,
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub internet: Internet,
}

fn ip_text(a: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
}

/// What Settings and the status bar show about the current link (Settings -> Wi-Fi "Current network", its details
/// sheet). `net` = the joined network's latest scan entry, for its name and signal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkView {
    /// a link exists (joining, connected, or just lost): the page shows its section
    pub shown: bool,
    pub connected: bool,
    pub name: String,
    /// "Connected · 5 GHz · Excellent", "Connecting…"
    pub subtitle: String,
    /// 0..=4 while connected, else -1 (the status bar's Wi-Fi icon)
    pub bars: i8,
    pub details: Vec<(String, String)>,
}

impl LinkView {
    pub fn new(link: &Link, net: Option<&Network>) -> Self {
        let shown = matches!(
            link.state,
            LinkState::Joining | LinkState::Handshake | LinkState::Connected
        );
        let connected = link.state == LinkState::Connected;
        let name = net.map_or_else(|| mac_text(&link.bssid), |n| String::from(n.title()));
        let state = link.message();
        let mut subtitle = state.clone();
        if let (true, Some(n)) = (connected, net) {
            subtitle = format!("{} · {} · {}", state, n.band(), n.signal());
        }
        let mut details = Vec::new();
        let mut row = |k: &str, v: String| details.push((String::from(k), v));
        row("Status", state);
        if let Some(n) = net {
            row("Signal", format!("{} ({} dBm)", n.signal(), n.dbm));
            row("Band", format!("{} · channel {}", n.band(), n.channel));
        }
        let security = if link.rsn.is_empty() {
            "Open"
        } else {
            "WPA2-Personal (AES-CCMP)"
        };
        row("Security", String::from(security));
        row("Access point", mac_text(&link.bssid));
        row("Phone MAC", mac_text(&link.mac));
        if connected {
            row("Association ID", format!("{}", link.aid));
            row(
                "Received",
                format!("{} frames ({} ARP/IP)", link.rx_data, link.rx_ip),
            );
        }
        LinkView {
            shown,
            connected,
            name,
            subtitle,
            bars: if connected {
                net.map_or(0, |n| n.bars() as i8)
            } else {
                -1
            },
            details,
        }
    }

    /// Add the IP rows (address, gateway, DNS, internet) while connected.
    pub fn with_ip(mut self, ip: &Ip) -> Self {
        if !self.connected {
            return self;
        }
        let mut row = |k: &str, v: String| self.details.push((String::from(k), v));
        let Some((address, prefix)) = ip.address else {
            row("IP address", String::from("Getting address…"));
            return self;
        };
        row("IP address", format!("{}/{}", ip_text(address), prefix));
        if ip.gateway != [0; 4] {
            row("Gateway", ip_text(ip.gateway));
        }
        if ip.dns != [0; 4] {
            row("DNS", ip_text(ip.dns));
        }
        let internet = match ip.internet {
            Internet::Checking => "Checking…",
            Internet::Ok => "Connected",
            Internet::NoAccess => "No access",
        };
        row("Internet", String::from(internet));
        self
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

    fn link(state: u8, status: u16, reason: u16, rsn: &[u8]) -> Vec<u8> {
        let mut b = alloc::vec![1, state];
        for v in [7u16, status, reason] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[1, 2, 3, 4, 5, 6, 2, 0x4e, 0, 0, 0, 1]);
        b.extend_from_slice(&100u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.push(rsn.len() as u8);
        b.extend_from_slice(rsn);
        b
    }

    #[test]
    fn decodes_link() {
        let l = Link::decode(&link(3, 0, 0, &[0x30, 2, 1, 0])).unwrap();
        assert_eq!(
            (l.state, l.aid, l.rx_data, l.rx_ip),
            (LinkState::Connected, 7, 100, 40)
        );
        assert_eq!(
            (l.bssid[5], l.mac[1], &l.rsn[..]),
            (6, 0x4e, &[0x30, 2, 1, 0][..])
        );
        assert_eq!(l.message(), "Connected");
        assert_eq!(
            Link::decode(&link(4, 0, 15, &[])).unwrap().message(),
            "Wrong password"
        );
        assert_eq!(
            Link::decode(&link(4, 0, 3, &[])).unwrap().message(),
            "Disconnected (reason 3)"
        );
        assert_eq!(
            Link::decode(&link(5, 17, 0, &[])).unwrap().message(),
            "Refused by the network (status 17)"
        );
        assert_eq!(
            Link::decode(&link(1, 0, 0, &[])).unwrap().message(),
            "Connecting…"
        );
        // a length that disagrees with the element, an unknown state, a short buffer
        let mut b = link(3, 0, 0, &[0x30, 0]);
        b[28] = 3;
        assert_eq!(Link::decode(&b), None);
        assert_eq!(Link::decode(&link(9, 0, 0, &[])), None);
        assert_eq!(Link::decode(&[1, 0, 0]), None);
    }

    #[test]
    fn saved_networks() {
        let mut s = Saved::default();
        s.remember(b"home", [1; 32]);
        s.remember(b"cafe", [2; 32]);
        s.remember(b"home", [3; 32]);
        assert_eq!(s.0.len(), 2);
        assert_eq!(
            (s.pmk(b"home"), s.pmk(b"cafe"), s.pmk(b"x")),
            (Some([3; 32]), Some([2; 32]), None)
        );
        let mut b = s.encode();
        assert_eq!(Saved::decode(&b), s);
        b.push(200); // a damaged tail
        assert_eq!(Saved::decode(&b), s);
        for i in 0..20u8 {
            s.remember(&[b'n', i], [i; 32]);
        }
        assert_eq!((s.0.len(), &s.0[0].0[..]), (Saved::MAX, &[b'n', 19][..]));
        s.remember(&[], [0; 32]);
        assert_eq!(s.0.len(), Saved::MAX);
        s.forget(&[b'n', 19]);
        assert_eq!((s.0.len(), s.pmk(&[b'n', 19])), (Saved::MAX - 1, None));
    }

    #[test]
    fn link_view() {
        let net = Network {
            bssid: [1, 2, 3, 4, 5, 6],
            channel: 44,
            dbm: -50,
            secured: true,
            ssid: "home".into(),
        };
        let up = Link::decode(&link(3, 0, 0, &[0x30, 2, 1, 0])).unwrap();
        let v = LinkView::new(&up, Some(&net));
        assert!(v.shown && v.connected);
        assert_eq!((v.name.as_str(), v.bars), ("home", 4));
        assert_eq!(v.subtitle, "Connected · 5 GHz · Excellent");
        let get = |k: &str| {
            v.details
                .iter()
                .find(|(a, _)| a == k)
                .map(|(_, b)| b.clone())
        };
        assert_eq!(get("Security").unwrap(), "WPA2-Personal (AES-CCMP)");
        assert_eq!(get("Access point").unwrap(), "01:02:03:04:05:06");
        assert_eq!(get("Received").unwrap(), "100 frames (40 ARP/IP)");
        assert_eq!(get("Band").unwrap(), "5 GHz · channel 44");
        // joining: shown, not connected, no bars; an unknown network is named by its BSSID
        let v = LinkView::new(&Link::decode(&link(1, 0, 0, &[])).unwrap(), None);
        assert!(v.shown && !v.connected);
        assert_eq!((v.bars, v.name.as_str()), (-1, "01:02:03:04:05:06"));
        assert_eq!(v.subtitle, "Connecting…");
        assert!(!LinkView::new(&Link::default(), None).shown);
    }

    #[test]
    fn link_view_ip_rows() {
        let up = Link::decode(&link(3, 0, 0, &[])).unwrap();
        let rows = |ip: &Ip| {
            LinkView::new(&up, None)
                .with_ip(ip)
                .details
                .into_iter()
                .skip_while(|(k, _)| k != "IP address")
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(rows(&Ip::default()), ["IP address: Getting address…"]);
        let ip = Ip {
            address: Some(([192, 168, 3, 57], 24)),
            gateway: [192, 168, 3, 1],
            dns: [192, 168, 3, 1],
            internet: Internet::Ok,
        };
        assert_eq!(
            rows(&ip),
            [
                "IP address: 192.168.3.57/24",
                "Gateway: 192.168.3.1",
                "DNS: 192.168.3.1",
                "Internet: Connected"
            ]
        );
        // no gateway or DNS from DHCP: those rows are left out
        let bare = Ip {
            gateway: [0; 4],
            dns: [0; 4],
            internet: Internet::NoAccess,
            ..ip
        };
        assert_eq!(
            rows(&bare),
            ["IP address: 192.168.3.57/24", "Internet: No access"]
        );
        // not connected: no IP rows at all
        let joining = Link::decode(&link(1, 0, 0, &[])).unwrap();
        let v = LinkView::new(&joining, None).with_ip(&ip);
        assert!(v.details.iter().all(|(k, _)| k != "IP address"));
    }
}

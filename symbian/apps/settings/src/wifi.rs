// SPDX-License-Identifier: GPL-3.0-only
//! Wi-Fi over its own NeoWifi session with WIFID (moved from the UI server with Settings; docs hardware/wifi.md):
//! - the page scans when it opens unless the list is fresh (under 30 s old), rescans every 30 s while open, and its
//!   refresh button scans now (one request per 5 s: the kernel queues a single scan);
//! - Connect validates the password and asks WIFID to join; WIFID joins, runs WPA2, remembers and rejoins;
//! - the link's IP side and the captive portal are the shell's (`SETTINGS_INFO` `INFO_NET`, `SETTINGS_ACTION`);
//! - Saved networks lists what WIFID remembers (NeoWifi 1.1 `SAVED`) and forgets one by name (`FORGET`).
use alloc::vec::Vec;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sys::{TPtr8, TPtrC8};
use neoapp::App;
use neoui_proto::ipc as proto;
use neoui_proto::settings_app::info;
use neoui_proto::wifi::{Link, LinkState, LinkView, Scan, State};
use settings_ui::{SetState, SettingsApp, WifiDetail, WifiLink, WifiNet, WifiSaved};
use slint::{ComponentHandle, ModelRc, VecModel};
use wifi::ipc::{Connect, fail, op};

/// A list this old is rescanned when the page opens, and while it stays open.
const FRESH_MS: u64 = 30_000;

#[derive(Default)]
pub struct Wifi {
    session: Option<Session>,
    last: Option<Scan>,
    listed_at: Option<u64>,
    asked_at: Option<u64>,
    link: Link,
    /// The network (BSSID text) the join sheet asked WIFID to join: while the sheet is open for that network it shows
    /// how the link goes; for any other it keeps the password check's own message.
    asked: Option<alloc::string::String>,
    /// Saved networks as WIFID listed them (raw SSIDs: a row's Forget sends these bytes, not its shown name), and the
    /// rows shown (the model is replaced only when they change: a new model drops a row's press)
    saved: Vec<Vec<u8>>,
    saved_rows: Vec<(alloc::string::String, &'static str)>,
}

impl Wifi {
    /// One NeoWifi request (a session made on first use, dropped on failure so the next one reconnects).
    fn call(&mut self, function: i32, args: &TIpcArgs<'_>) -> Result<i32, i32> {
        if self.session.is_none() {
            let (major, minor, build) = wifi::ipc::VERSION;
            self.session = Some(Session::connect(
                wifi::ipc::NAME,
                TVersion {
                    major: major as u8,
                    minor: minor as u8,
                    build: build as u16,
                },
            )?);
        }
        let r = self
            .session
            .as_ref()
            .map_or(Err(-18), |s| s.send_receive(function, args));
        // a refused request (bad argument, not found, too long, a 1.0 WIFID's unknown op) keeps the session
        if matches!(r, Err(e) if ![-1, -5, -6, -9].contains(&e)) {
            self.session = None;
        }
        r
    }

    fn scan(&mut self, rescan: bool) -> Option<Scan> {
        let mut b = alloc::vec![0u8; neoui_proto::wifi::SIZE];
        let mut des = TPtr8::new(&mut b);
        let mut args = TIpcArgs::new();
        args.int(0, i32::from(rescan)).des8(1, &mut des);
        self.call(op::SCAN, &args).ok()?;
        let n = des.len();
        Scan::decode(&b[..n])
    }

    /// Read the link, the list and the shell's IP side into the pages. `page`: the Wi-Fi page is open (it scans);
    /// `refresh`: its refresh button.
    pub fn poll(&mut self, app: &App, ui: &SettingsApp, now: u64, page: bool, refresh: bool) {
        let s = ui.global::<SetState>();
        // the link and why the last join gave up
        let mut b = [0u8; neoui_proto::wifi::LINK_SIZE];
        let mut des = TPtr8::new(&mut b);
        let mut args = TIpcArgs::new();
        args.des8(0, &mut des);
        let (link, failed, up) = match self.call(op::LINK, &args) {
            Ok(f) => (Link::decode(&b[..des.len()]).unwrap_or_default(), f, true),
            Err(_) => (Link::default(), 0, false),
        };
        let message = match failed {
            fail::SECURITY => {
                "This network's security is not supported (WPA2-PSK with CCMP only)".into()
            }
            fail::RANDOM => "No random numbers for the handshake".into(),
            fail::DOWNGRADE => {
                "The network's security changed during the handshake: not joined".into()
            }
            fail::HAL => "Wi-Fi is not available".into(),
            fail::HANDSHAKE => "Wrong password".into(),
            fail::NO_DATA => "No data from the network: Wi-Fi rejoins after a restart".into(),
            _ if !up => "Wi-Fi is not available".into(),
            _ => link.message(),
        };
        // the sheet's network: is a join for it asked, going, or done (Connect waits meanwhile)
        let current = s.get_wifi_current().bssid;
        let ours = self.asked.as_deref() == Some(current.as_str());
        if ours {
            s.set_wifi_message(message.into());
        }
        let b = link.bssid;
        let this = alloc::format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5]
        ) == current.as_str();
        s.set_wifi_join(match link.state {
            LinkState::Connected if this => 2,
            LinkState::Joining | LinkState::Handshake if this || ours => 1,
            _ => 0,
        });
        self.link = link;

        // the list
        let stale = self
            .listed_at
            .is_none_or(|t| now.saturating_sub(t) >= FRESH_MS);
        let waiting = self.asked_at.is_some_and(|t| now.saturating_sub(t) < 5000);
        let rescan = page && !waiting && (refresh || stale);
        if rescan {
            self.asked_at = Some(now);
        }
        let scan = self.scan(rescan).unwrap_or_else(|| Scan {
            state: State::Error,
            ..Scan::default()
        });
        if self.last.as_ref().is_none_or(|l| l.scans != scan.scans) && scan.state != State::Off {
            self.listed_at = Some(now);
        }
        let joined = scan.networks.iter().find(|n| n.bssid == self.link.bssid);
        let ip = app
            .settings_info(proto::INFO_NET)
            .ok()
            .and_then(|b| info::decode_ip(&b))
            .unwrap_or_default();
        let view = LinkView::new(&self.link, joined).with_ip(&ip);
        s.set_wifi_status(scan.status().into());
        s.set_wifi_hidden(scan.hidden() as i32);
        s.set_wifi_scanning(scan.state == State::Scanning || rescan && scan.state == State::Ready);
        // the network we are on shows under "Current network", not in the list
        let rows: Vec<WifiNet> = scan
            .list()
            .iter()
            .filter(|n| !n.ssid.is_empty() && !(view.shown && n.ssid == view.name))
            .map(|n| WifiNet {
                name: n.title().into(),
                detail: n.detail().into(),
                bars: i32::from(n.bars()),
                secured: n.secured,
                band: n.band().into(),
                signal: n.signal().into(),
                channel: i32::from(n.channel),
                bssid: n.bssid_text().into(),
            })
            .collect();
        s.set_wifi_networks(ModelRc::new(VecModel::from(rows)));
        let details: Vec<WifiDetail> = view
            .details
            .iter()
            .map(|(l, d)| WifiDetail {
                label: l.as_str().into(),
                value: d.as_str().into(),
            })
            .collect();
        s.set_wifi_link(WifiLink {
            shown: view.shown,
            connected: view.connected,
            name: view.name.as_str().into(),
            subtitle: view.subtitle.as_str().into(),
            bars: i32::from(view.bars),
            band: view.band.as_str().into(),
            details: ModelRc::new(VecModel::from(details)),
            sign_in: view.sign_in,
        });
        self.last = Some(scan);
    }

    /// Connect on the join sheet: the network by its BSSID ("aa:bb:..") and the typed password. Returns what the sheet
    /// shows now: the password check's message, or "Connecting…" (then the link's own state, from `poll`).
    pub fn connect(&mut self, bssid: &str, password: &str) -> Option<&'static str> {
        let net = self
            .last
            .as_ref()
            .and_then(|s| s.networks.iter().find(|n| n.bssid_text() == bssid).cloned());
        match net {
            // shortcut: the SSID's bytes as the scan decoded them (UTF-8); a non-UTF-8 SSID hashes wrong
            Some(n) if !n.secured || (8..=63).contains(&password.len()) => {
                let c = Connect {
                    bssid: n.bssid,
                    ssid: n.ssid.as_bytes().to_vec(),
                    password: if n.secured {
                        password.as_bytes().to_vec()
                    } else {
                        Vec::new()
                    },
                }
                .encode();
                let des = TPtrC8::new(&c);
                let mut args = TIpcArgs::new();
                args.desc8(0, &des);
                match self.call(op::CONNECT, &args) {
                    Ok(_) => {
                        self.asked = Some(alloc::string::String::from(bssid));
                        Some("Connecting…")
                    }
                    Err(e) => {
                        e32::dprintln!("settings: wifi: connect -> {}", e);
                        Some("Wi-Fi is not available")
                    }
                }
            }
            Some(_) => Some("A WPA2 password has 8 to 63 characters"),
            None => Some("The network is gone; refresh the list"),
        }
    }

    /// The networks WIFID remembers into Saved networks: connected (the link's network) or not, in the latest scan or
    /// not. Hidden (`wifi-saved-ok` false) when WIFID has no list (NeoWifi 1.0).
    pub fn saved(&mut self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let mut b = alloc::vec![0u8; wifi::ipc::SAVED_SIZE];
        let mut des = TPtr8::new(&mut b);
        let mut args = TIpcArgs::new();
        args.des8(0, &mut des);
        let names = match self.call(op::SAVED, &args) {
            Ok(_) => wifi::ipc::decode_names(&b[..des.len()]),
            Err(e) => {
                e32::dprintln!("settings: wifi: saved -> {}", e);
                None
            }
        };
        s.set_wifi_saved_ok(names.is_some());
        let joined = self.last.as_ref().and_then(|l| {
            (self.link.state == LinkState::Connected)
                .then(|| l.networks.iter().find(|n| n.bssid == self.link.bssid))
                .flatten()
        });
        let names = names.unwrap_or_default();
        let rows: Vec<(alloc::string::String, &'static str)> = names
            .iter()
            .map(|n| {
                // shortcut: names shown as UTF-8 (lossy), as the scan list does
                let name = alloc::string::String::from_utf8_lossy(n);
                let seen = self
                    .last
                    .as_ref()
                    .is_some_and(|l| l.networks.iter().any(|x| x.ssid == name));
                let detail = if joined.is_some_and(|j| j.ssid == name) {
                    "Connected · Secured"
                } else if seen {
                    "Secured"
                } else {
                    "Secured · not in range"
                };
                (name.into_owned(), detail)
            })
            .collect();
        self.saved = names;
        if rows != self.saved_rows {
            let model: Vec<WifiSaved> = rows
                .iter()
                .map(|(n, d)| WifiSaved {
                    name: n.as_str().into(),
                    detail: (*d).into(),
                    connected: d.starts_with("Connected"),
                })
                .collect();
            s.set_wifi_saved(ModelRc::new(VecModel::from(model)));
            self.saved_rows = rows;
        }
    }

    /// The saved network on row `i`, raw, as WIFID listed it.
    pub fn saved_ssid(&self, i: usize) -> Option<Vec<u8>> {
        self.saved.get(i).cloned()
    }

    /// Forget a saved network (raw SSID; WIFID leaves it first if it is the one joined).
    pub fn forget(&mut self, ssid: &[u8]) -> Result<i32, i32> {
        let des = TPtrC8::new(ssid);
        let mut args = TIpcArgs::new();
        args.desc8(0, &des);
        let r = self.call(op::FORGET, &args);
        e32::dprintln!("settings: wifi: forget -> {:?}", r);
        r
    }

    /// Disconnect (`forget` false) or Forget (true) the current network.
    pub fn leave(&mut self, forget: bool) {
        let mut args = TIpcArgs::new();
        args.int(0, i32::from(forget));
        let r = self.call(op::LEAVE, &args);
        e32::dprintln!("settings: wifi: leave (forget {}) -> {:?}", forget, r);
    }
}

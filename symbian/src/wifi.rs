// SPDX-License-Identifier: GPL-3.0-only
//! Settings -> Wi-Fi: the kernel's scan results (docs hardware/wifi.md W3), scanned the way phones do it:
//! - opening the Wi-Fi page scans at once unless the list is fresh (under 30 s old);
//! - while the page is open it rescans every 30 s, and its refresh button scans now;
//! - nothing is scanned while the page is closed (the Settings root only shows the last list).
//!
//!
//! Joining (W4) is WIFID.EXE's (neohw, docs hardware.md S5), which SYSSTART starts: NeoWifi joins and runs the
//! WPA2 handshake, remembers networks (SSID + PMK) and rejoins them on its own, also with the page closed. This page
//! shows the scan and the link (the kernel's records, through NeoWifi) and asks to join or leave.
//!
//! IP (W5): NEONET.EXE (neohw `net`, started by WIFID) gets an address by DHCP once the link is connected. The
//! details sheet shows its lease, and one internet check per address fetches a `generate_204` page through the
//! NeoNet client API, the way apps use it.
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sync::Mutex;
use e32::sys::{TPtr8, TPtrC8};
use e32::thread::JoinHandle;
use neoui_proto::wifi::{Internet, Ip, Link, LinkState, LinkView, Scan, State};
use neoui_render::Renderer;
use wifi::ipc::{Connect, fail, op};

/// Why the internet check ended (0 while it runs); shared with its thread (leaked: it lives as long as SYSSTART, and
/// Arc needs compare-and-swap, which ARMv5TE lacks)
type Failed = Option<&'static Mutex<i32>>;
/// An HTTP Date from the internet check (Unix seconds) and when it was read (ms since boot), for the clock;
/// shared with the check's thread (leaked, as [`Failed`])
type HttpTime = Option<&'static Mutex<Option<(i64, u64)>>>;

/// A list this old is refreshed when the page opens, and rescanned while it stays open
const FRESH_MS: u64 = 30_000;

/// The internet check: Android's captive-portal probe, an empty 204 answer when the internet is reachable.
/// shortcut: a portal's login page shows as "No access"; open it when there is a browser.
const CHECK_HOST: &str = "connectivitycheck.gstatic.com";
const CHECK_OK: i32 = 1;
const CHECK_FAILED: i32 = 2;

pub struct Wifi {
    next_read: u64,
    /// when the latest scan completed (its `scans` counter changed)
    listed_at: Option<u64>,
    page_open: bool,
    asked_at: Option<u64>,
    last: Option<Scan>,
    /// a session with WIFID (made when first needed)
    neowifi: Option<Session>,
    /// the switch as WIFID last heard it
    radio_sent: Option<bool>,
    link: Link,
    /// why WIFID's last join gave up (wifi::ipc::fail, 0 = it didn't)
    failed: i32,
    /// the sheet's message as last shown, and when the link state is read next (every 200 ms)
    shown: alloc::string::String,
    next_link: u64,
    view: LinkView,
    /// the link view changed: publish the list again (with or without the current network)
    relist: bool,
    /// a session with NEONET (made when first needed), its lease as last read, and when it is read next (1 s)
    net: Option<net::client::Net>,
    ip: Ip,
    next_ip: u64,
    /// the running internet check, the address it was started for, and its result (0 while it runs)
    check: Option<JoinHandle>,
    checked: Option<[u8; 4]>,
    internet: Failed,
    http_time: HttpTime,
}

impl Wifi {
    pub fn new() -> Self {
        Self::start_net();
        Self {
            next_read: 0,
            listed_at: None,
            page_open: false,
            asked_at: None,
            last: None,
            neowifi: None,
            radio_sent: None,
            link: Link::default(),
            shown: alloc::string::String::new(),
            next_link: 0,
            view: LinkView::default(),
            relist: false,
            failed: 0,
            net: None,
            ip: Ip::default(),
            next_ip: 0,
            check: None,
            checked: None,
            internet: shared(),
            http_time: Mutex::new(None)
                .ok()
                .map(|m| &*alloc::boxed::Box::leak(alloc::boxed::Box::new(m))),
        }
    }

    /// The internet check's HTTP Date, once (for the network clock).
    pub fn take_http_time(&mut self) -> Option<(i64, u64)> {
        self.http_time.and_then(|m| m.lock().take())
    }

    /// Start WIFID (it starts the IP stack; a second start finds it running and exits).
    fn start_net() {
        match e32::process::Process::spawn("WIFID.EXE", "") {
            Ok(p) => p.resume(),
            Err(e) => e32::dprintln!("neoui: no Wi-Fi daemon ({})", e),
        }
    }

    /// One NeoWifi request (a session made on first use, dropped on failure so the next one reconnects).
    fn call(&mut self, function: i32, args: &TIpcArgs<'_>) -> Result<i32, i32> {
        if self.neowifi.is_none() {
            let (major, minor, build) = wifi::ipc::VERSION;
            self.neowifi = Some(Session::connect(
                wifi::ipc::NAME,
                TVersion {
                    major: major as u8,
                    minor: minor as u8,
                    build: build as u16,
                },
            )?);
        }
        let r = self
            .neowifi
            .as_ref()
            .map_or(Err(-18), |s| s.send_receive(function, args));
        if matches!(r, Err(e) if e != -6) {
            self.neowifi = None;
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

    /// The IP side (W5): NEONET's lease once a second while connected, and one internet check per address.
    fn poll_ip(&mut self, now: u64) {
        if self.link.state != LinkState::Connected {
            self.ip = Ip::default();
            self.checked = None;
            return;
        }
        if now < self.next_ip {
            return;
        }
        self.next_ip = now.saturating_add(1000);
        if self.net.is_none() {
            self.net = net::client::Net::connect().ok();
        }
        let Some(status) = self.net.as_ref().and_then(|n| n.status().ok()) else {
            self.net = None; // not running (yet), or it went: connect again next time
            return;
        };
        let address = (status.state == net::proto::State::Up).then_some((status.ip, status.prefix));
        let result = self.internet.map_or(0, |m| *m.lock());
        if result != 0
            && let Some(h) = self.check.take()
        {
            h.join();
        }
        if let Some((a, _)) = address
            && self.checked != Some(a)
            && self.check.is_none()
        {
            set(self.internet, 0);
            let (shared, http) = (self.internet, self.http_time);
            match e32::thread::spawn(move || internet_check(shared, http)) {
                Ok(h) => (self.check, self.checked) = (Some(h), Some(a)),
                Err(e) => e32::dprintln!("neoui: wifi: no internet check thread ({})", e),
            }
        }
        self.ip = Ip {
            address,
            gateway: status.gateway,
            dns: status.dns[0],
            internet: match self.internet.map_or(0, |m| *m.lock()) {
                CHECK_OK => Internet::Ok,
                CHECK_FAILED => Internet::NoAccess,
                _ => Internet::Checking,
            },
        };
    }

    /// The joined network's latest scan entry (for its name and signal)
    fn joined(&self) -> Option<neoui_proto::wifi::Network> {
        let b = self.link.bssid;
        self.last
            .as_ref()?
            .networks
            .iter()
            .find(|n| n.bssid == b)
            .cloned()
    }

    fn poll_link(&mut self, ui: &mut Renderer, now: u64, radio_on: bool) {
        if self.radio_sent != Some(radio_on) {
            let mut args = TIpcArgs::new();
            args.int(0, i32::from(radio_on));
            if self.call(op::RADIO, &args).is_ok() {
                self.radio_sent = Some(radio_on);
            }
        }
        if let Some(forget) = ui.take_wifi_leave() {
            let mut args = TIpcArgs::new();
            args.int(0, i32::from(forget));
            let _ = self.call(op::LEAVE, &args);
            self.next_link = 0; // show the change now
        }
        if let Some((bssid, password)) = ui.take_wifi_connect() {
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
                            alloc::vec::Vec::new()
                        },
                    }
                    .encode();
                    let des = TPtrC8::new(&c);
                    let mut args = TIpcArgs::new();
                    args.desc8(0, &des);
                    if let Err(e) = self.call(op::CONNECT, &args) {
                        e32::dprintln!("neoui: wifi: connect -> {}", e);
                        ui.set_wifi_message("Wi-Fi is not available");
                    }
                }
                Some(_) => ui.set_wifi_message("A WPA2 password has 8 to 63 characters"),
                None => ui.set_wifi_message("The network is gone; refresh the list"),
            }
        }
        if now < self.next_link {
            return;
        }
        self.next_link = now.saturating_add(200);
        let mut b = [0u8; neoui_proto::wifi::LINK_SIZE];
        let mut des = TPtr8::new(&mut b);
        let mut args = TIpcArgs::new();
        args.des8(0, &mut des);
        let (link, failed) = match self.call(op::LINK, &args) {
            Ok(f) => {
                let n = des.len();
                (Link::decode(&b[..n]).unwrap_or_default(), f)
            }
            Err(_) => (Link::default(), 0),
        };
        self.failed = failed;
        let text = match failed {
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
            _ => link.message(),
        };
        if text != self.shown {
            ui.set_wifi_message(&text);
            self.shown = text;
        }
        self.link = link;
        self.poll_ip(now);
        let view = LinkView::new(&self.link, self.joined().as_ref()).with_ip(&self.ip);
        if view != self.view {
            ui.set_wifi_link(&view);
            self.view = view;
            self.relist = true;
        }
    }

    /// `radio_on`: Settings' Wi-Fi switch (WIFID's: off leaves the network, nothing is joined or scanned on its own).
    /// shortcut: the kernel keeps the radio up while the switch is off; power it down once the driver can.
    pub fn poll(&mut self, ui: &mut Renderer, now: u64, radio_on: bool) {
        self.poll_link(ui, now, radio_on);
        let page = ui.screen() == "settings" && ui.view() == "wifi";
        let refresh = ui.take_wifi_refresh();
        let opened = page && !self.page_open;
        self.page_open = page;
        let stale = self
            .listed_at
            .is_none_or(|t| now.saturating_sub(t) >= FRESH_MS);
        // one request at a time: the kernel queues a single scan, so don't ask again for 5 s
        let waiting = self.asked_at.is_some_and(|t| now.saturating_sub(t) < 5000);
        // WIFID scans in the background for remembered networks; this page only while it is open
        let rescan =
            page && !waiting && (refresh || opened && stale || stale && self.asked_at.is_some());
        if !rescan && !opened && !self.relist && now < self.next_read {
            return;
        }
        self.next_read = now.saturating_add(if page { 500 } else { 5000 });
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
        let relist = core::mem::take(&mut self.relist);
        if self.last.as_ref() != Some(&scan) || rescan || relist {
            // a requested scan shows as searching at once, before the kernel's state catches up; the network we
            // are on shows under "Current network", not in the list
            let mut shown = scan.clone();
            if self.view.shown {
                shown
                    .networks
                    .retain(|n| n.ssid.is_empty() || n.ssid != self.view.name);
            }
            if rescan && shown.state == State::Ready {
                shown.state = State::Scanning;
            }
            ui.set_wifi(&shown);
            self.last = Some(scan);
        }
    }
}

/// A result shared with a thread (leaked: see [`Failed`])
fn shared() -> Failed {
    Mutex::new(0)
        .ok()
        .map(|m| &*alloc::boxed::Box::leak(alloc::boxed::Box::new(m)))
}

/// The internet check thread: resolve, connect, fetch `/generate_204`, expect "204"; the result goes into `result`,
/// the response's `Date:` into `http` (the clock).
fn internet_check(result: Failed, http: HttpTime) -> i32 {
    let fetch = || -> Result<bool, i32> {
        let net = net::client::Net::connect()?;
        let ip = net.resolve(CHECK_HOST)?;
        let mut s = net.tcp_connect(ip, 80)?;
        s.write_all(
            b"GET /generate_204 HTTP/1.1\r\nHost: connectivitycheck.gstatic.com\r\nConnection: close\r\n\r\n",
        )?;
        // the status line and the headers (a 204 has no body)
        let mut b = [0u8; 1024];
        let mut n = 0;
        while n < b.len() && !b[..n].windows(4).any(|w| w == b"\r\n\r\n") {
            match s.read(&mut b[n..])? {
                0 => break,
                k => n += k,
            }
        }
        if let (Some(m), Some(date)) = (http, neoui_proto::clock::datetime::http_date(&b[..n])) {
            *m.lock() = Some((date as i64, e32::time::millis()));
        }
        Ok(b[..n].starts_with(b"HTTP/1.1 204") || b[..n].starts_with(b"HTTP/1.0 204"))
    };
    let r = fetch();
    e32::dprintln!("neoui: wifi: internet check {:?}", r);
    set(
        result,
        if r == Ok(true) {
            CHECK_OK
        } else {
            CHECK_FAILED
        },
    );
    0
}

fn set(failed: Failed, why: i32) {
    if let Some(m) = failed {
        *m.lock() = why;
    }
}

// SPDX-License-Identifier: GPL-3.0-only
//! Settings -> Wi-Fi: the kernel's scan results (docs wifi.md W3), scanned the way phones do it:
//! - opening the Wi-Fi page scans at once unless the list is fresh (under 30 s old);
//! - while the page is open it rescans every 30 s, and its refresh button scans now;
//! - nothing is scanned while the page is closed (the Settings root only shows the last list).
//!
//!
//! Joining (W4): Connect on a network's sheet hashes the password into the PMK here, then a supplicant thread joins
//! through the kernel (HAL WlanLink 0x4e1f) and runs the WPA2 4-way handshake (neolink `wpa`). The page shows the
//! kernel's link state. A network that reached "Connected" is remembered (SSID + PMK, never the password) in the
//! UI server's private directory and joined again when it is seen while nothing is joined.
//!
//! IP (W5): NEONET.EXE (neolink `net`, started here) gets an address by DHCP once the link is connected. The
//! details sheet shows its lease, and one internet check per address fetches a `generate_204` page through the
//! NeoNet client API, the way apps use it.
//! shortcut: the supplicant is a thread of the UI server, not part of NEONET (which carries IP, not EAPOL); move it
//! there when Wi-Fi must work without the UI.
use e32::fs::{File, Fs, mode};
use e32::sync::Mutex;
use e32::thread::JoinHandle;
use neoui_proto::wifi::{Internet, Ip, Link, LinkState, LinkView, Saved, Scan, State};
use neoui_render::Renderer;

/// The remembered networks, in the UI server's private directory (data-caged: apps can't read it)
const SAVED: &str = "C:\\private\\e0002001\\wifi.dat";
/// A remembered network seen while nothing is joined is tried again after this long
const RETRY_MS: u64 = 60_000;

/// Why the supplicant thread gave up (0 = it ended with the link), for the sheet's message; shared with the thread
/// (leaked: it lives as long as SYSSTART, and Arc needs compare-and-swap, which ARMv5TE lacks)
type Failed = Option<&'static Mutex<i32>>;
/// a running supplicant thread, the SSID and PMK (none: an open network) it joins with, and when it started
type Join = (JoinHandle, alloc::vec::Vec<u8>, Option<[u8; 32]>, u64);
const FAIL_SECURITY: i32 = 1;
const FAIL_RANDOM: i32 = 2;
const FAIL_DOWNGRADE: i32 = 3;
const FAIL_HAL: i32 = 4;

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
    /// the supplicant thread, the SSID and PMK it joins with, and when it started
    join: Option<Join>,
    link: Link,
    saved: Saved,
    failed: Failed,
    /// the sheet's message as last shown, and when the link state is read next (every 200 ms)
    shown: alloc::string::String,
    next_link: u64,
    /// the SSID the user disconnected from: not rejoined on its own until Connect is pressed again
    left: Option<alloc::vec::Vec<u8>>,
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
            join: None,
            link: Link::default(),
            saved: load(),
            shown: alloc::string::String::new(),
            next_link: 0,
            left: None,
            view: LinkView::default(),
            relist: false,
            failed: shared(),
            net: None,
            ip: Ip::default(),
            next_ip: 0,
            check: None,
            checked: None,
            internet: shared(),
        }
    }

    /// Start the IP stack (it waits for the link; a second start finds it running and exits).
    fn start_net() {
        match e32::process::Process::spawn("NEONET.EXE", "") {
            Ok(p) => p.resume(),
            Err(e) => e32::dprintln!("neoui: no network daemon ({})", e),
        }
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
            let shared = self.internet;
            match e32::thread::spawn(move || internet_check(shared)) {
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

    /// Start joining `bssid` (a network of the latest scan) with this PMK; a running join is left first.
    /// `pmk` = `None` for an open network (no handshake, nothing remembered).
    fn start(&mut self, bssid: [u8; 6], ssid: &[u8], pmk: Option<[u8; 32]>, now: u64) {
        self.stop();
        self.left = None;
        set(self.failed, 0);
        let failed = self.failed;
        match e32::thread::spawn(move || supplicant(bssid, pmk, failed)) {
            Ok(h) => self.join = Some((h, ssid.to_vec(), pmk, now)),
            Err(e) => e32::dprintln!("neoui: wifi: no supplicant thread ({})", e),
        }
    }

    /// Leave the network and end the supplicant thread.
    fn stop(&mut self) {
        if let Some((h, ..)) = self.join.take() {
            let _ = neoapp::hal::wifi_leave();
            h.join();
        }
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

    fn poll_link(&mut self, ui: &mut Renderer, now: u64) {
        if let Some(forget) = ui.take_wifi_leave() {
            let ssid = self.joined().map(|n| n.ssid.into_bytes());
            self.stop();
            if let Some(ssid) = ssid {
                if forget {
                    self.saved.forget(&ssid);
                    store(&self.saved);
                }
                self.left = Some(ssid);
            }
            self.next_link = 0; // show the change now
        }
        if let Some((bssid, password)) = ui.take_wifi_connect() {
            let net = self
                .last
                .as_ref()
                .and_then(|s| s.networks.iter().find(|n| n.bssid_text() == bssid).cloned());
            match net {
                // shortcut: the SSID's bytes as the scan decoded them (UTF-8); a non-UTF-8 SSID hashes wrong
                Some(n) if !n.secured => self.start(n.bssid, n.ssid.as_bytes(), None, now),
                Some(n) if (8..=63).contains(&password.len()) => {
                    let pmk = wpa::psk(password.as_bytes(), n.ssid.as_bytes());
                    self.start(n.bssid, n.ssid.as_bytes(), Some(pmk), now);
                }
                Some(_) => ui.set_wifi_message("A WPA2 password has 8 to 63 characters"),
                None => ui.set_wifi_message("The network is gone; refresh the list"),
            }
        }
        if now < self.next_link {
            return;
        }
        self.next_link = now.saturating_add(200);
        let link = neoapp::hal::wifi_link_status().unwrap_or_default();
        if link.state == LinkState::Connected
            && self.link.state != LinkState::Connected
            && let Some((_, ssid, Some(pmk), _)) = &self.join
        {
            self.saved.remember(ssid, *pmk);
            store(&self.saved);
        }
        let failed = self.failed.map_or(0, |m| *m.lock());
        let text = match failed {
            FAIL_SECURITY => {
                "This network's security is not supported (WPA2-PSK with CCMP only)".into()
            }
            FAIL_RANDOM => "No random numbers for the handshake".into(),
            FAIL_DOWNGRADE => {
                "The network's security changed during the handshake: not joined".into()
            }
            FAIL_HAL => "Wi-Fi is not available".into(),
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
        // a remembered network in the list while nothing is joined: try it (once a minute at most)
        let idle = matches!(
            self.link.state,
            LinkState::Idle | LinkState::Lost | LinkState::Error
        );
        let quiet = self
            .join
            .as_ref()
            .is_none_or(|j| now.saturating_sub(j.3) >= RETRY_MS);
        if idle && quiet {
            let known = self.last.as_ref().and_then(|s| {
                s.list()
                    .into_iter()
                    .filter(|n| self.left.as_deref() != Some(n.ssid.as_bytes()))
                    .find_map(|n| Some((n.bssid, self.saved.pmk(n.ssid.as_bytes())?, n.ssid)))
            });
            if let Some((bssid, pmk, ssid)) = known {
                self.start(bssid, ssid.as_bytes(), Some(pmk), now);
            }
        }
    }

    pub fn poll(&mut self, ui: &mut Renderer, now: u64) {
        self.poll_link(ui, now);
        let page = ui.screen() == "settings" && ui.view() == "wifi";
        let refresh = ui.take_wifi_refresh();
        let opened = page && !self.page_open;
        self.page_open = page;
        let stale = self
            .listed_at
            .is_none_or(|t| now.saturating_sub(t) >= FRESH_MS);
        // one request at a time: the kernel queues a single scan, so don't ask again for 5 s
        let waiting = self.asked_at.is_some_and(|t| now.saturating_sub(t) < 5000);
        let rescan =
            page && !waiting && (refresh || opened && stale || stale && self.asked_at.is_some());
        if !rescan && !opened && !self.relist && now < self.next_read {
            return;
        }
        self.next_read = now.saturating_add(if page { 500 } else { 5000 });
        if rescan {
            self.asked_at = Some(now);
        }
        let scan = neoapp::hal::wifi_scan(rescan).unwrap_or_else(|_| Scan {
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

/// The supplicant thread: join, then answer the AP's EAPOL-Key frames until the link ends (a 10 ms poll: the
/// driver is polled too, and the AP waits about a second per handshake message).
fn supplicant(bssid: [u8; 6], pmk: Option<[u8; 32]>, failed: Failed) -> i32 {
    use neoapp::hal;
    let fail = |why: i32| {
        set(failed, why);
        let _ = hal::wifi_leave();
        why
    };
    let rsn: &[u8] = if pmk.is_some() { &wpa::RSN_IE } else { &[] };
    if hal::wifi_join(&bssid, rsn).is_err() {
        return fail(FAIL_HAL);
    }
    let mut hs: Option<wpa::Handshake> = None;
    let mut frame = [0u8; 512];
    loop {
        e32::time::sleep(10_000);
        let Ok(link) = hal::wifi_link_status() else {
            return fail(FAIL_HAL);
        };
        match link.state {
            LinkState::Idle | LinkState::Lost | LinkState::Error => return 0,
            LinkState::Joining => continue,
            LinkState::Handshake | LinkState::Connected => {}
        }
        let Some(pmk) = pmk else {
            continue; // an open network: no handshake, the kernel reports Connected after association
        };
        let hs = match &mut hs {
            Some(h) => h,
            None => {
                if !wpa::Rsn::parse(&link.rsn).is_some_and(|r| r.supported()) {
                    return fail(FAIL_SECURITY);
                }
                let mut snonce = [0u8; 32];
                if hal::random(&mut snonce).is_err() {
                    return fail(FAIL_RANDOM);
                }
                hs.insert(wpa::Handshake::new(pmk, bssid, link.mac, &link.rsn, snonce))
            }
        };
        while let Ok(Some(n)) = hal::wifi_eapol_recv(&mut frame) {
            let actions = match hs.on_eapol(&frame[..n]) {
                Ok(a) => a,
                Err(wpa::Error::RsnMismatch | wpa::Error::NoGtk) => return fail(FAIL_DOWNGRADE),
                Err(_) => continue, // malformed, replayed or forged: ignored, as the standard says
            };
            for a in actions {
                let r = match a {
                    wpa::Action::Send(f) => hal::wifi_eapol_send(&f),
                    wpa::Action::InstallPtk(k) => hal::wifi_install_key(false, 0, &k),
                    wpa::Action::InstallGtk(i, k) => hal::wifi_install_key(true, i, &k),
                    wpa::Action::Authorize => hal::wifi_authorize(),
                };
                if r.is_err() {
                    return fail(FAIL_HAL);
                }
            }
        }
    }
}

/// A result shared with a thread (leaked: see [`Failed`])
fn shared() -> Failed {
    Mutex::new(0)
        .ok()
        .map(|m| &*alloc::boxed::Box::leak(alloc::boxed::Box::new(m)))
}

/// The internet check thread: resolve, connect, fetch `/generate_204`, expect "204"; the result goes into `result`.
fn internet_check(result: Failed) -> i32 {
    let fetch = || -> Result<bool, i32> {
        let net = net::client::Net::connect()?;
        let ip = net.resolve(CHECK_HOST)?;
        let mut s = net.tcp_connect(ip, 80)?;
        s.write_all(
            b"GET /generate_204 HTTP/1.1\r\nHost: connectivitycheck.gstatic.com\r\nConnection: close\r\n\r\n",
        )?;
        let mut b = [0u8; 16];
        let mut n = 0;
        while n < b.len() {
            match s.read(&mut b[n..])? {
                0 => break,
                k => n += k,
            }
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

fn load() -> Saved {
    let read = || -> Result<alloc::vec::Vec<u8>, i32> {
        let fs = Fs::connect()?;
        let f = File::open(&fs, SAVED, mode::READ | mode::SHARE_READERS_ONLY)?;
        let mut b = alloc::vec![0u8; 1024];
        let n = f.read_at(0, &mut b)?;
        b.truncate(n);
        Ok(b)
    };
    read().map(|b| Saved::decode(&b)).unwrap_or_default()
}

fn store(saved: &Saved) {
    let write = || -> Result<(), i32> {
        let fs = Fs::connect()?;
        File::replace(&fs, SAVED, mode::SHARE_EXCLUSIVE)?.write(&saved.encode())
    };
    if let Err(e) = write() {
        e32::dprintln!("neoui: wifi: saving the network failed ({})", e);
    }
}

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
//! shortcut: the supplicant is a thread of the UI server; it moves to a daemon with the network interface (W5).
use e32::fs::{File, Fs, mode};
use e32::sync::Mutex;
use e32::thread::JoinHandle;
use neoui_proto::wifi::{Link, LinkState, Saved, Scan, State};
use neoui_render::Renderer;

/// The remembered networks, in the UI server's private directory (data-caged: apps can't read it)
const SAVED: &str = "C:\\private\\e0002001\\wifi.dat";
/// A remembered network seen while nothing is joined is tried again after this long
const RETRY_MS: u64 = 60_000;

/// Why the supplicant thread gave up (0 = it ended with the link), for the sheet's message; shared with the thread
/// (leaked: it lives as long as SYSSTART, and Arc needs compare-and-swap, which ARMv5TE lacks)
type Failed = Option<&'static Mutex<i32>>;
const FAIL_SECURITY: i32 = 1;
const FAIL_RANDOM: i32 = 2;
const FAIL_DOWNGRADE: i32 = 3;
const FAIL_HAL: i32 = 4;

/// A list this old is refreshed when the page opens, and rescanned while it stays open
const FRESH_MS: u64 = 30_000;

pub struct Wifi {
    next_read: u64,
    /// when the latest scan completed (its `scans` counter changed)
    listed_at: Option<u64>,
    page_open: bool,
    asked_at: Option<u64>,
    last: Option<Scan>,
    /// the supplicant thread, the SSID and PMK it joins with, and when it started
    join: Option<(JoinHandle, alloc::vec::Vec<u8>, [u8; 32], u64)>,
    link: Link,
    saved: Saved,
    failed: Failed,
}

impl Wifi {
    pub fn new() -> Self {
        Self {
            next_read: 0,
            listed_at: None,
            page_open: false,
            asked_at: None,
            last: None,
            join: None,
            link: Link::default(),
            saved: load(),
            failed: Mutex::new(0)
                .ok()
                .map(|m| &*alloc::boxed::Box::leak(alloc::boxed::Box::new(m))),
        }
    }

    /// Start joining `bssid` (a network of the latest scan) with this PMK; a running join is left first.
    fn start(&mut self, bssid: [u8; 6], ssid: &[u8], pmk: [u8; 32], now: u64) {
        if let Some((h, ..)) = self.join.take() {
            let _ = neoapp::hal::wifi_leave();
            h.join();
        }
        set(self.failed, 0);
        let failed = self.failed;
        match e32::thread::spawn(move || supplicant(bssid, pmk, failed)) {
            Ok(h) => self.join = Some((h, ssid.to_vec(), pmk, now)),
            Err(e) => e32::dprintln!("neoui: wifi: no supplicant thread ({})", e),
        }
    }

    fn poll_link(&mut self, ui: &mut Renderer, now: u64) {
        if let Some((bssid, password)) = ui.take_wifi_connect() {
            let net = self
                .last
                .as_ref()
                .and_then(|s| s.networks.iter().find(|n| n.bssid_text() == bssid).cloned());
            match net {
                // shortcut: the SSID's bytes as the scan decoded them (UTF-8); a non-UTF-8 SSID hashes wrong
                Some(n) if (8..=63).contains(&password.len()) || !n.secured => {
                    let pmk = wpa::psk(password.as_bytes(), n.ssid.as_bytes());
                    self.start(n.bssid, n.ssid.as_bytes(), pmk, now);
                }
                Some(_) => ui.set_wifi_message("A WPA2 password has 8 to 63 characters"),
                None => ui.set_wifi_message("The network is gone; refresh the list"),
            }
        }
        let link = neoapp::hal::wifi_link_status().unwrap_or_default();
        if link.state == LinkState::Connected && self.link.state != LinkState::Connected {
            if let Some((_, ssid, pmk, _)) = &self.join {
                self.saved.remember(ssid, *pmk);
                store(&self.saved);
            }
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
        if link != self.link || failed != 0 {
            ui.set_wifi_message(&text);
        }
        self.link = link;
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
                    .find_map(|n| Some((n.bssid, self.saved.pmk(n.ssid.as_bytes())?, n.ssid)))
            });
            if let Some((bssid, pmk, ssid)) = known {
                self.start(bssid, ssid.as_bytes(), pmk, now);
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
        if !rescan && !opened && now < self.next_read {
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
        if self.last.as_ref() != Some(&scan) || rescan {
            // a requested scan shows as searching at once, before the kernel's state catches up
            let mut shown = scan.clone();
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
fn supplicant(bssid: [u8; 6], pmk: [u8; 32], failed: Failed) -> i32 {
    use neoapp::hal;
    let fail = |why: i32| {
        set(failed, why);
        let _ = hal::wifi_leave();
        why
    };
    if hal::wifi_join(&bssid, &wpa::RSN_IE).is_err() {
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

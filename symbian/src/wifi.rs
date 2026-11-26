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
use alloc::sync::Arc;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sync::Mutex;
use e32::sys::TPtr8;
use e32::thread::JoinHandle;
use neoui_proto::wifi::{Internet, Ip, Link, LinkState, LinkView, Scan};
use neoui_render::Renderer;
use wifi::ipc::op;

/// Why the internet check ended (0 while it runs); shared with its thread
type Failed = Option<Arc<Mutex<i32>>>;
/// An HTTP Date from the internet check (Unix seconds) and when it was read (ms since boot), for the clock;
/// shared with the check's thread
type HttpTime = Option<Arc<Mutex<Option<(i64, u64)>>>>;

/// The internet check: Android's captive-portal probe, an empty 204 answer when the internet is reachable. Another
/// answer is a login page in the way (a cafe's or hotel's): Settings and the shade offer Sign in, which opens the
/// probe's address in the browser, where the network shows its page (browser UI5, br2-captive).
const CHECK_HOST: &str = "connectivitycheck.gstatic.com";
/// shortcut: for testing a login page without one, this file can name another host to check (fshell writes it).
const CHECK_HOST_FILE: &str = "C:\\private\\e0002001\\check-host.txt";
const CHECK_OK: i32 = 1;
const CHECK_FAILED: i32 = 2;
const CHECK_PORTAL: i32 = 3;
/// After the login: a network that wanted one is checked again this often, so Sign in goes once it's done
const PORTAL_RECHECK_MS: u64 = 30_000;

pub struct Wifi {
    next_read: u64,
    last: Option<Scan>,
    /// a session with WIFID (made when first needed)
    neowifi: Option<Session>,
    /// the switch as WIFID last heard it
    radio_sent: Option<bool>,
    link: Link,
    /// when the link state is read next (every 200 ms)
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
    /// when the last internet check started (a login page is checked again every [`PORTAL_RECHECK_MS`])
    checked_at: u64,
    /// the joined network wants its login page: the browser's argument for it (`<url>\n<network>`, UI5)
    portal: Option<alloc::string::String>,
    internet: Failed,
    http_time: HttpTime,
}

impl Wifi {
    pub fn new() -> Self {
        Self::start_net();
        Self {
            next_read: 0,
            last: None,
            neowifi: None,
            radio_sent: None,
            link: Link::default(),
            next_link: 0,
            view: LinkView::default(),
            relist: false,
            net: None,
            ip: Ip::default(),
            next_ip: 0,
            check: None,
            checked: None,
            checked_at: 0,
            portal: None,
            internet: shared(),
            http_time: Mutex::new(None).ok().map(Arc::new),
        }
    }

    /// The internet check's HTTP Date, once (for the network clock).
    pub fn take_http_time(&mut self) -> Option<(i64, u64)> {
        self.http_time.as_ref().and_then(|m| m.lock().take())
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
        let result = self.internet.as_ref().map_or(0, |m| *m.lock());
        if result != 0
            && let Some(h) = self.check.take()
        {
            h.join();
        }
        if self.ip.internet == Internet::SignIn
            && self.check.is_none()
            && now.saturating_sub(self.checked_at) >= PORTAL_RECHECK_MS
        {
            self.checked = None;
        }
        if let Some((a, _)) = address
            && self.checked != Some(a)
            && self.check.is_none()
        {
            self.checked_at = now;
            set(&self.internet, 0);
            let (shared, http) = (self.internet.clone(), self.http_time.clone());
            match e32::thread::spawn(move || internet_check(shared, http)) {
                Ok(h) => (self.check, self.checked) = (Some(h), Some(a)),
                Err(e) => e32::dprintln!("neoui: wifi: no internet check thread ({})", e),
            }
        }
        self.ip = Ip {
            address,
            gateway: status.gateway,
            dns: status.dns[0],
            internet: match self.internet.as_ref().map_or(0, |m| *m.lock()) {
                CHECK_OK => Internet::Ok,
                CHECK_FAILED => Internet::NoAccess,
                CHECK_PORTAL => Internet::SignIn,
                // a login page's re-check keeps Sign in on screen until it answers
                _ if self.ip.internet == Internet::SignIn => Internet::SignIn,
                _ => Internet::Checking,
            },
        };
        // the host is read once per login page; the network's name follows the scan (its MAC until the scan names it)
        if self.ip.internet != Internet::SignIn {
            self.portal = None;
        } else {
            let url = match self.portal.as_deref().and_then(|p| p.split_once('\n')) {
                Some((url, _)) => alloc::string::String::from(url),
                None => alloc::format!("http://{}/generate_204", check_host()),
            };
            self.portal = Some(alloc::format!("{url}\n{}", self.view.name));
        }
    }

    /// The joined network wants its login page: the browser's argument for it (`<url>\n<network>`).
    pub fn portal(&self) -> Option<alloc::string::String> {
        self.portal.clone()
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
        if now < self.next_link {
            return;
        }
        self.next_link = now.saturating_add(200);
        let mut b = [0u8; neoui_proto::wifi::LINK_SIZE];
        let mut des = TPtr8::new(&mut b);
        let mut args = TIpcArgs::new();
        args.des8(0, &mut des);
        // (joins, leaving and why a join failed are the Settings app's, over its own NeoWifi session)
        let link = match self.call(op::LINK, &args) {
            Ok(_) => Link::decode(&b[..des.len()]).unwrap_or_default(),
            Err(_) => Link::default(),
        };
        self.link = link;
        self.poll_ip(now);
        let view = LinkView::new(&self.link, self.joined().as_ref()).with_ip(&self.ip);
        if view != self.view {
            ui.set_wifi_link(&view);
            self.view = view;
            self.relist = true;
        }
    }

    /// `radio_on`: the Wi-Fi switch (WIFID's: off leaves the network, nothing is joined or scanned on its own).
    /// shortcut: the kernel keeps the radio up while the switch is off; power it down once the driver can.
    pub fn poll(&mut self, ui: &mut Renderer, now: u64, radio_on: bool) {
        self.poll_link(ui, now, radio_on);
        // the joined network's scan entry gives the status bar its bars and band; the Settings app lists and scans
        if !core::mem::take(&mut self.relist) && now < self.next_read {
            return;
        }
        self.next_read = now.saturating_add(5000);
        if let Some(scan) = self.scan(false) {
            self.last = Some(scan);
        }
    }

    /// The link's IP side, for the Settings app (`SETTINGS_INFO` `INFO_NET`).
    pub fn ip(&self) -> &Ip {
        &self.ip
    }
}

/// A result shared with a thread
fn shared() -> Failed {
    Mutex::new(0).ok().map(Arc::new)
}

/// The internet check thread: resolve, connect, fetch `/generate_204`, expect "204"; the result goes into `result`,
/// the response's `Date:` into `http` (the clock).
fn internet_check(result: Failed, http: HttpTime) -> i32 {
    let fetch = || -> Result<i32, i32> {
        let host = check_host();
        let net = net::client::Net::connect()?;
        let ip = net.resolve(&host)?;
        let mut s = net.tcp_connect(ip, 80)?;
        let request = alloc::format!(
            "GET /generate_204 HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        );
        s.write_all(request.as_bytes())?;
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
        Ok(
            if b[..n].starts_with(b"HTTP/1.1 204") || b[..n].starts_with(b"HTTP/1.0 204") {
                CHECK_OK
            } else if b[..n].starts_with(b"HTTP/") {
                CHECK_PORTAL // a web server answered instead: a login page in the way
            } else {
                CHECK_FAILED
            },
        )
    };
    let r = fetch();
    e32::dprintln!("neoui: wifi: internet check {:?}", r);
    set(&result, r.unwrap_or(CHECK_FAILED));
    0
}

/// The host the internet check asks: [`CHECK_HOST`], or the one [`CHECK_HOST_FILE`] names.
fn check_host() -> alloc::string::String {
    let named = e32::fs::Fs::connect().ok().and_then(|fs| {
        let f = e32::fs::File::open(&fs, CHECK_HOST_FILE, e32::fs::mode::READ).ok()?;
        let mut b = [0u8; 128];
        let n = f.read_at(0, &mut b).ok()?;
        let t = core::str::from_utf8(&b[..n]).ok()?.trim();
        (!t.is_empty()
            && t.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-:".contains(&c)))
        .then(|| alloc::string::String::from(t))
    });
    named.unwrap_or_else(|| CHECK_HOST.into())
}

fn set(failed: &Failed, why: i32) {
    if let Some(m) = failed {
        *m.lock() = why;
    }
}

/// Settings → Reset network settings (SE4a): WIFID forgets every network (NeoWifi 1.2 `RESET`), on a session of its
/// own (the shell's is for the link status). WIFID leaves the joined network first, which can take about 3 s.
pub fn reset() -> Result<(), i32> {
    let (major, minor, build) = wifi::ipc::VERSION;
    let s = Session::connect(
        wifi::ipc::NAME,
        TVersion {
            major: major as u8,
            minor: minor as u8,
            build: build as u16,
        },
    )?;
    s.send_receive(op::RESET, &TIpcArgs::new()).map(drop)
}

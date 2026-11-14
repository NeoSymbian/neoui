// SPDX-License-Identifier: GPL-3.0-only
//! The shell's phone service, NeoModem's client in the UI server (C7). Commands use one session; a separately owned
//! notification future keeps status updates outstanding without blocking input or holding self-referential
//! descriptor pointers in `Phone`.
//! The Phone app (NEOPHONE, NeoSymbian/phone) draws the dialer, Recents, Contacts and the calls, and talks to MODEMD
//! itself. The shell keeps what is the system's: the status bar's network, the SIM prompt, the "Modem unavailable"
//! banner, the shade's missed calls and "return to call", bringing the app up on its call page when a call comes,
//! the screen (wake on ring, on during a call), the volume keys in a call, the radio and ring settings, the network
//! time.

use crate::settings::Settings;
use alloc::format;
use alloc::vec::Vec;
use e32::dprintln;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::process::Process;
use e32::sys::{TPtr8, TPtrC8};
use modem_ipc::{LogEntry, Status, call, log, modem, op, rat, reg, sim};
use neoui_proto::settings::keys;
use neoui_render::Renderer;
use phone_rules::dial::{self, Flags, Kind};

fn connect() -> Result<Session, i32> {
    let (major, minor, build) = modem_ipc::VERSION;
    Session::connect(
        modem_ipc::NAME,
        TVersion {
            major: major as u8,
            minor: minor as u8,
            build: build as u16,
        },
    )
}

/// Owns its session so `next` can move into the launcher's select future without borrowing `Phone`.
pub struct Watch {
    session: Option<Session>,
    revision: u32,
    /// MODEMD was started (else there is nothing to watch: QEMU/CI, and the loop must not wake for it)
    present: bool,
    /// the wait before the next connect after a failure: 0.5 s doubling to 10 s, back to 0.5 s after a status
    backoff_ms: u32,
}

const BACKOFF_MS: u32 = 500;
const BACKOFF_MAX_MS: u32 = 10_000;

/// Cancels the held request before its future and descriptor storage are dropped, unless it completed.
struct Cancel<'a>(&'a Session, bool);
impl Drop for Cancel<'_> {
    fn drop(&mut self) {
        if !self.1 {
            let _ = self.0.send_receive(op::CANCEL_NOTIFY, &TIpcArgs::new());
        }
    }
}

impl Watch {
    pub fn new(present: bool) -> Self {
        Self {
            session: None,
            revision: u32::MAX,
            present,
            backoff_ms: BACKOFF_MS,
        }
    }

    async fn back_off(&mut self) {
        e32::time::delay(i32::try_from(self.backoff_ms * 1000).unwrap_or(i32::MAX)).await;
        self.backoff_ms = self.backoff_ms.saturating_mul(2).min(BACKOFF_MAX_MS);
    }

    pub async fn next(mut self) -> (Self, Option<Status>) {
        if !self.present {
            core::future::pending::<()>().await;
        }
        if self.session.is_none() {
            self.session = connect().ok();
            self.revision = u32::MAX;
        }
        if self.session.is_none() {
            self.back_off().await;
            return (self, None);
        }
        let session = self.session.as_ref().unwrap();
        let mut bytes = [0u8; Status::SIZE];
        let result = {
            let mut descriptor = TPtr8::new(&mut bytes);
            let mut args = TIpcArgs::new();
            args.des8(0, &mut descriptor);
            args.int(1, self.revision as i32);
            let future = session.send(op::NOTIFY, &args);
            let future = core::pin::pin!(future);
            // Declared after the pinned future: cancellation runs first on every exit path where it is still held.
            let mut cancel = Cancel(session, false);
            let r = future.await;
            cancel.1 = true; // completed: nothing to cancel
            r
        };
        let status = result.ok().and_then(|_| Status::from_bytes(&bytes));
        if let Some(s) = status {
            self.revision = s.revision;
            self.backoff_ms = BACKOFF_MS;
        } else {
            self.session = None;
            self.back_off().await;
        }
        (self, status)
    }
}

pub struct Phone {
    session: Option<Session>,
    present: bool,
    shown: Option<Status>,
    update: Option<Status>,
    disconnected: bool,
    volume: Option<u8>,
    preferences: Option<(u32, bool, modem_ipc::Ring)>,
    preferences_retry_at: u64,
    /// a call started ringing since `take_ring` (the screen wakes for it)
    rang: bool,
    /// the SIM state whose PIN/blocked prompt the user put away ("Not now", or an emergency call)
    sim_dismissed: Option<u8>,
    /// the call log as MODEMD last sent it (for the shade's missed calls) and its revision
    log: Vec<LogEntry>,
    log_revision: Option<u32>,
    /// (modem failed, new missed calls) as last shown
    availability: Option<(bool, u32)>,
}

impl Phone {
    pub fn start() -> Self {
        let present = match Process::spawn("MODEMD.EXE", "") {
            Ok(p) => {
                p.resume();
                dprintln!("neoui: modem daemon started");
                true
            }
            Err(e) => {
                dprintln!("neoui: no modem daemon ({})", e);
                false
            }
        };
        Self {
            session: None,
            present,
            shown: None,
            update: None,
            disconnected: false,
            volume: None,
            preferences: None,
            preferences_retry_at: 0,
            rang: false,
            sim_dismissed: None,
            log: Vec::new(),
            log_revision: None,
            availability: None,
        }
    }

    /// Did a call start ringing since the last call? (Once per call: main's screen wakes for it.)
    pub fn take_ring(&mut self) -> bool {
        core::mem::take(&mut self.rang)
    }

    /// A call is up (ringing, dialling or connected): the screen stays on.
    pub fn in_call(&self) -> bool {
        self.shown.as_ref().is_some_and(live)
    }

    /// The network's time (NITZ), once the modem has one.
    pub fn nitz(&self) -> Option<modem_ipc::Nitz> {
        self.shown.map(|s| s.nitz).filter(|n| n.utc != 0)
    }

    /// MODEMD was started (the watcher has something to watch).
    pub fn present(&self) -> bool {
        self.present
    }

    pub fn observe(&mut self, status: Option<Status>) {
        self.update = status;
        // only a session that had delivered status can be lost: MODEMD not up yet at boot is not "connection lost"
        self.disconnected = status.is_none() && self.shown.is_some();
        if status.is_none() {
            self.session = None;
            self.preferences = None;
        }
    }

    /// The persisted shell store owns requested radio/sound preferences. MODEMD owns confirmation and recovery;
    /// replay the stored request once per generation and after reconnect, without overwriting it with old status.
    /// (The call settings, caller ID and the log's days, are the Phone app's: it sends them itself.)
    pub fn sync_settings(&mut self, settings: &Settings, now_ms: u64) {
        let Some(status) = self.shown else {
            return;
        };
        if status.modem == modem::FAILED || now_ms < self.preferences_retry_at {
            return;
        }
        let airplane = settings.get(keys::RADIO_AIRPLANE).unwrap_or(false);
        let ring = modem_ipc::Ring {
            volume: settings
                .get(keys::SOUND_RING_VOLUME)
                .unwrap_or(8)
                .clamp(0, 10) as u8,
            silent: settings.get(keys::SOUND_SILENT).unwrap_or(false),
            vibrate: settings.get(keys::SOUND_VIBRATE).unwrap_or(true),
        };
        let desired = (status.generation, airplane, ring);
        if self.preferences == Some(desired) {
            return;
        }
        let radio_result = self.request("airplane", if airplane { "1" } else { "0" });
        let ring_result = self.session.as_ref().map_or(-18, |session| {
            let mut args = TIpcArgs::new();
            args.int(0, i32::from(ring.volume));
            args.int(1, i32::from(ring.silent));
            args.int(2, i32::from(ring.vibrate));
            session
                .send_receive(op::RING_SETTINGS, &args)
                .unwrap_or_else(|e| e)
        });
        if radio_result == 0 && ring_result == 0 {
            self.preferences = Some(desired);
        } else {
            self.preferences_retry_at = now_ms.saturating_add(1_000);
        }
    }

    /// The SIM prompt covers the shell (until it is put away for this SIM state).
    pub fn modal(&self) -> bool {
        self.shown.is_some_and(|s| {
            matches!(s.sim, sim::PIN_REQUIRED | sim::BLOCKED) && self.sim_dismissed != Some(s.sim)
        })
    }

    pub fn poll(&mut self, ui: &mut Renderer, settings: &Settings) {
        for (action, value) in ui.take_phone() {
            self.act(ui, &action, &value);
        }
        if !self.present {
            return;
        }
        if self.disconnected {
            self.disconnected = false;
            self.shown = None;
            ui.set_in_call(false);
            ui.set_network(0, "No service", "");
            ui.set_phone_feedback(false, "Modem connection lost");
        }
        if let Some(s) = self.update.take() {
            self.show(ui, s);
        }
        // phone.missed_seen moves on when the Phone app shows Recents' missed calls
        let failed = self.shown.is_some_and(|s| s.modem == modem::FAILED);
        let availability = (failed, self.new_missed(settings));
        if self.availability != Some(availability) {
            self.availability = Some(availability);
            ui.set_phone_availability(availability.0, availability.1);
        }
    }

    /// A new status from MODEMD.
    fn show(&mut self, ui: &mut Renderer, s: Status) {
        let old = self.shown;
        let ringing = |x: &Status| {
            x.calls
                .as_slice()
                .iter()
                .any(|c| matches!(c.state, call::INCOMING | call::WAITING))
        };
        let rings = ringing(&s) && !old.as_ref().is_some_and(ringing);
        let starts = live(&s) && !old.as_ref().is_some_and(live);
        let app_in_front = matches!(ui.screen().as_str(), "phone" | "contacts");
        if rings {
            self.rang = true;
        }
        // the Phone app comes up on its call page (host::PAGES): a call ringing, or one started elsewhere
        if rings || (starts && !app_in_front) {
            ui.show("phone", "call");
        }
        ui.set_in_call(live(&s));
        self.volume = None;
        // Registration and RAT survive a failed stop/recovery in MODEMD's cached protocol status. They describe
        // the previous radio session until the currently available modem confirms that it is online again.
        let registered =
            s.modem == modem::ONLINE && s.radio_online && matches!(s.reg, reg::HOME | reg::ROAMING);
        let technology = if registered {
            match s.rat {
                rat::LTE => "LTE",
                rat::UMTS => "3G",
                rat::GSM => "2G",
                _ => "",
            }
        } else {
            ""
        };
        let network = match s.modem {
            modem::FAILED => "No service",
            modem::RECOVERING => "Recovering…",
            modem::BOOTING => "Starting modem…",
            _ if s.airplane && !s.radio_online => "Airplane mode",
            _ if registered => s.operator.as_str(),
            _ => "No service",
        };
        ui.set_network(
            if registered { s.bars as i32 } else { 0 },
            network,
            technology,
        );
        let feedback = if s.result < 0 {
            format!("Action failed ({})", s.result)
        } else {
            alloc::string::String::new()
        };
        ui.set_phone_feedback(s.pending, &feedback);
        ui.set_phone_settings(s.radio_online);
        if self.sim_dismissed.is_some_and(|d| d != s.sim) {
            self.sim_dismissed = None; // a new SIM state asks again
        }
        let prompt = self.sim_dismissed.is_none();
        ui.set_sim(
            prompt && s.sim == sim::PIN_REQUIRED,
            prompt && s.sim == sim::BLOCKED,
            (s.pin_attempts != u8::MAX).then_some(s.pin_attempts),
        );
        if self.log_revision != Some(s.log_revision) && self.fetch_log() {
            self.log_revision = Some(s.log_revision);
        }
        // (no earpiece blanking until there is a proximity sensor: the screen is main's screen.rs alone)
        self.shown = Some(s);
    }

    /// One action of the SIM prompt or the modem banner (Renderer::take_phone).
    fn act(&mut self, ui: &mut Renderer, action: &str, value: &str) {
        match action {
            "sim-dismiss" => {
                self.sim_dismissed = self.shown.map(|s| s.sim);
                ui.set_sim(false, false, None);
            }
            "dial" => self.dial(ui, value),
            _ => {
                let r = self.request(action, value);
                // The action name and completion are safe; the descriptor (PIN) is never logged.
                dprintln!("neoui: phone {} -> {}", action, r);
                if r < 0 {
                    ui.set_phone_feedback(false, &format!("Could not complete action ({r})"));
                }
            }
        }
    }

    /// The SIM prompt's emergency call, as the Symbian^3 parser reads the number (docs/symbian-phone-sources.md §5).
    fn dial(&mut self, ui: &mut Renderer, typed: &str) {
        let flags = Flags {
            send: true,
            in_call: self.in_call(),
        };
        match dial::classify(typed, flags) {
            Kind::Number { clir, number, .. } => {
                let wire = format!("{}{}", clir.map_or("", |c| c.prefix()), number);
                let r = self.request("dial", &wire);
                dprintln!("neoui: phone dial -> {}", r);
                if r < 0 {
                    ui.set_phone_feedback(false, &format!("Call failed ({r})"));
                }
            }
            Kind::Emergency(_) => {
                ui.set_phone_feedback(false, "Emergency calls are not supported on this phone yet")
            }
            _ => {}
        }
    }

    /// MODEMD's call log (C8); false when it couldn't be read.
    fn fetch_log(&mut self) -> bool {
        if self.session.is_none() {
            self.session = connect().ok();
        }
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        let mut bytes = alloc::vec![0u8; modem_ipc::MAX_LOG * LogEntry::SIZE];
        let mut d = TPtr8::new(&mut bytes);
        let mut args = TIpcArgs::new();
        args.des8(0, &mut d);
        if session
            .send_receive(op::CALL_LOG, &args)
            .unwrap_or_else(|e| e)
            < 0
        {
            return false;
        }
        let n = d.len();
        self.log = bytes[..n]
            .as_chunks::<{ LogEntry::SIZE }>()
            .0
            .iter()
            .filter_map(|c| LogEntry::from_bytes(c))
            .collect();
        true
    }

    /// Missed calls the Phone app hasn't shown yet (newer than `phone.missed_seen`).
    fn new_missed(&self, settings: &Settings) -> u32 {
        let seen = settings.get(keys::PHONE_MISSED_SEEN).unwrap_or(0) as u32;
        self.log
            .iter()
            .filter(|e| e.kind == log::MISSED && e.id > seen)
            .count() as u32
    }

    pub fn volume_key(&mut self, up: bool) -> bool {
        let Some(s) = self.shown else {
            return false;
        };
        if !s.audio.on {
            return false;
        }
        let now = self.volume.unwrap_or(s.audio.volume);
        let volume = if up {
            (now + 1).min(10)
        } else {
            now.saturating_sub(1)
        };
        if self.request("volume", &format!("{volume}")) == 0 {
            self.volume = Some(volume);
        }
        true
    }

    fn request(&mut self, action: &str, value: &str) -> i32 {
        if self.session.is_none() {
            self.session = connect().ok();
        }
        let Some(session) = self.session.as_ref() else {
            return -18;
        };
        let descriptor = TPtrC8::new(value.as_bytes());
        let mut args = TIpcArgs::new();
        let function = match action {
            "dial" | "pin" => {
                args.desc8(0, &descriptor);
                if action == "dial" {
                    op::DIAL
                } else {
                    op::VERIFY_PIN
                }
            }
            "airplane" => {
                args.int(0, i32::from(value == "1"));
                op::AIRPLANE
            }
            "volume" => {
                args.int(0, value.parse().unwrap_or(8));
                op::VOLUME
            }
            "retry" => op::RETRY,
            _ => return -5,
        };
        session.send_receive(function, &args).unwrap_or_else(|e| e)
    }
}

fn live(status: &Status) -> bool {
    status
        .calls
        .as_slice()
        .iter()
        .any(|c| !matches!(c.state, call::NONE | call::END))
}

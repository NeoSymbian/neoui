// SPDX-License-Identifier: GPL-3.0-only
//! NeoModem's C7 client. Commands use one session; a separately owned notification future keeps status updates
//! outstanding without blocking input or holding self-referential descriptor pointers in `Phone`.
//! C8: the dialer (smart dialling, speed dial, the voice mailbox, dial strings), Recents from MODEMD's call log,
//! caller names, the call's duration and summary, DTMF pauses after a number, call settings and call waiting.

use crate::contacts::Contacts;
use crate::settings::{self, Settings};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::dprintln;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::process::Process;
use e32::sys::{TPtr8, TPtrC8};
use modem_ipc::{LogEntry, Status, call, log, manage, modem, op, rat, reg, sim, waiting};
use neoui_contacts::{NameOrder, recents, t9};
use neoui_proto::settings::{Bytes, Caller, Value, keys};
use neoui_render::{CallView, LogDetailItem, LogItem, Renderer};
use phone_rules::caller::caller_line;
use phone_rules::dial::{self, Action, Flags, Kind};

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
    close_at: Option<u64>,
    volume: Option<u8>,
    preferences: Option<(u32, bool, modem_ipc::Ring)>,
    preferences_retry_at: u64,
    /// a call started ringing since `take_ring` (the screen wakes for it)
    rang: bool,
    /// the SIM state whose PIN/blocked prompt the user put away ("Not now", or an emergency call)
    sim_dismissed: Option<u8>,
    /// C8: the dialer's number and when `*` was last tapped (the `* + p w` cycle)
    number: String,
    star_at: u64,
    /// the call log as MODEMD last sent it (newest first) and its revision
    log: Vec<LogEntry>,
    log_revision: Option<u32>,
    /// (generation, caller ID, log days) MODEMD confirmed
    call_settings: Option<(u32, i32, i32)>,
    /// the selected call once connected: (call id, since ms)
    connected: Option<(u8, u64)>,
    /// tones to send once the dialled call connects (the part after a `p`/`w`), and the replay in progress
    dtmf_after: Option<String>,
    dtmf: Option<Dtmf>,
    /// the note over the Phone and Contacts screens goes at this time (0 = none)
    note_until: u64,
    /// a contact's number waiting for its speed-dial key (Contacts -> Assign speed dial)
    speed_pending: Option<String>,
    waiting_shown: u8,
}

/// Tones after a number (docs/symbian-phone-sources.md §4): `p` pauses, `w` waits for the user.
struct Dtmf {
    rest: String,
    at: u64,
    confirm: bool,
}

const CLOSE_MS: u64 = 2000;
const NOTE_MS: u64 = 3000;
/// a `p` in a dial string (the GSM convention)
const PAUSE_MS: u64 = 2500;
/// between two tones (MODEMD plays each for 200 ms)
const TONE_GAP_MS: u64 = 300;
/// a key held this long is a long press; a second `*` within this cycles it
const STAR_CYCLE_MS: u64 = 1000;

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
            close_at: None,
            volume: None,
            preferences: None,
            preferences_retry_at: 0,
            rang: false,
            sim_dismissed: None,
            number: String::new(),
            star_at: 0,
            log: Vec::new(),
            log_revision: None,
            call_settings: None,
            connected: None,
            dtmf_after: None,
            dtmf: None,
            note_until: 0,
            speed_pending: None,
            waiting_shown: waiting::UNKNOWN,
        }
    }

    /// When the main loop must run again for the phone (ms from now): a note to take down, the next tone, the
    /// call's ticking time.
    pub fn next_wake_ms(&self, now_ms: u64, settings: &Settings) -> Option<u64> {
        let ticking =
            self.connected.is_some() && settings.get(keys::PHONE_SHOW_DURATION).unwrap_or(false);
        [
            (self.note_until != 0).then_some(self.note_until),
            self.dtmf.as_ref().filter(|d| !d.confirm).map(|d| d.at),
            self.close_at,
            ticking.then_some(now_ms + 1000 - (now_ms % 1000)),
        ]
        .into_iter()
        .flatten()
        .min()
        .map(|t| t.saturating_sub(now_ms))
    }

    fn note(&mut self, ui: &mut Renderer, text: &str, now_ms: u64) {
        ui.set_note(text);
        self.note_until = now_ms + NOTE_MS;
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
    pub fn sync_settings(&mut self, settings: &crate::settings::Settings, now_ms: u64) {
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

    /// Caller ID and the log's days (C8) go to MODEMD, which keeps them; again after a modem restart.
    fn sync_call_settings(&mut self, settings: &Settings, generation: u32) {
        let clir = settings.get(keys::PHONE_CLIR).unwrap_or(0);
        let days = match settings.get(keys::PHONE_LOG_DAYS).unwrap_or(30) {
            d @ (0 | 1 | 10) => d,
            _ => 30,
        };
        let want = (generation, clir, days);
        if self.call_settings == Some(want) {
            return;
        }
        let r = self.session.as_ref().map_or(-18, |session| {
            let mut args = TIpcArgs::new();
            args.int(0, clir);
            args.int(1, days);
            session
                .send_receive(op::CALL_SETTINGS, &args)
                .unwrap_or_else(|e| e)
        });
        if r == 0 {
            self.call_settings = Some(want);
        }
    }

    /// The SIM prompt covers the shell (until it is put away for this SIM state).
    pub fn modal(&self) -> bool {
        self.shown.is_some_and(|s| {
            matches!(s.sim, sim::PIN_REQUIRED | sim::BLOCKED) && self.sim_dismissed != Some(s.sim)
        })
    }

    pub fn poll(
        &mut self,
        ui: &mut Renderer,
        now_ms: u64,
        contacts: &mut Contacts,
        settings: &mut Settings,
    ) {
        if self.note_until != 0 && now_ms >= self.note_until {
            self.note_until = 0;
            ui.set_note("");
        }
        contacts.set_order(
            ui,
            NameOrder::from_setting(settings.get(keys::CONTACTS_NAME_ORDER).unwrap_or(0)),
        );
        for (action, value) in ui.take_phone() {
            if action == "sim-dismiss" {
                self.sim_dismissed = self.shown.map(|s| s.sim);
                ui.set_sim(false, false, None);
                continue;
            }
            if let Some(note) = self.act(ui, &action, &value, now_ms, contacts, settings) {
                self.note(ui, &note, now_ms);
            }
        }
        if !self.present {
            return;
        }
        self.dtmf_step(ui, now_ms);
        self.show_call_time(ui, now_ms, settings);
        if self.disconnected {
            let had_call = self.shown.as_ref().is_some_and(live);
            self.disconnected = false;
            self.shown = None;
            // a call view says "Call interrupted", then closes like an ended call (it stayed up after a reconnect)
            self.close_at = had_call.then_some(now_ms + CLOSE_MS);
            ui.set_calls(&[]);
            ui.set_call("Call interrupted", "", false);
            ui.set_call_audio(false, false, -1);
            ui.set_network(0, "No service", "");
            ui.set_phone_feedback(false, "Modem connection lost");
        }
        if self.close_at.is_some_and(|t| now_ms >= t) {
            self.close_at = None;
            if self.shown.is_none_or(|s| !live(&s)) {
                ui.close_call();
            }
        }
        let Some(s) = self.update.take() else {
            return;
        };
        let old = self.shown;
        if live(&s) {
            self.close_at = None;
        } else if old.is_some_and(|x| live(&x)) {
            self.close_at = Some(now_ms + CLOSE_MS);
        }
        let incoming = s
            .calls
            .as_slice()
            .iter()
            .any(|c| matches!(c.state, call::INCOMING | call::WAITING));
        let had_incoming = old.is_some_and(|x| {
            x.calls
                .as_slice()
                .iter()
                .any(|c| matches!(c.state, call::INCOMING | call::WAITING))
        });
        if incoming && !had_incoming {
            ui.show_call();
            self.rang = true;
        }
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
        // C8: the caller's name (one matching contact), "Private number", ...; the number under a name
        let line = |c: &modem_ipc::Call| {
            let n = c.number.as_str();
            caller_line(contacts.name_of(n).as_deref(), n, c.presentation, false)
        };
        let (main, second) = line(&s.call);
        ui.set_call(
            state_text(s.call.state, s.call.incoming),
            &main,
            matches!(s.call.state, call::INCOMING | call::WAITING),
        );
        self.track_connection(ui, &s, now_ms, settings);
        ui.set_call_extra(&second, &self.call_time(now_ms, settings));
        ui.set_call_audio(
            s.audio.muted,
            s.audio.speaker,
            if s.audio.on {
                s.audio.volume as i32
            } else {
                -1
            },
        );
        let up: Vec<(&modem_ipc::Call, String)> = s
            .calls
            .as_slice()
            .iter()
            .filter(|c| !matches!(c.state, call::NONE | call::END))
            .map(|c| (c, line(c).0))
            .collect();
        let calls: Vec<_> = up
            .iter()
            .map(|(c, title)| CallView {
                id: c.id,
                number: title,
                state: state_text(c.state, c.incoming),
                incoming: c.state == call::INCOMING,
                held: c.state == call::HOLD,
                waiting: c.state == call::WAITING,
            })
            .collect();
        ui.set_calls(&calls);
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
        // C8: the log, call settings, call waiting
        self.sync_call_settings(settings, s.generation);
        if self.log_revision != Some(s.log_revision) && self.fetch_log() {
            self.log_revision = Some(s.log_revision);
            self.show_log(ui, contacts, settings, now_ms);
        }
        if s.waiting != self.waiting_shown {
            let was_busy = self.waiting_shown == waiting::BUSY;
            self.waiting_shown = s.waiting;
            ui.set_waiting(i32::from(s.waiting));
            let note = match s.waiting {
                waiting::ON => "Call waiting active",
                waiting::OFF => "Call waiting not active",
                waiting::ERROR => "Call waiting: request not confirmed",
                _ => "",
            };
            if was_busy && !note.is_empty() {
                self.note(ui, note, now_ms);
            }
        }
        ui.set_phone_availability(s.modem == modem::FAILED, self.new_missed(settings));
        // (no earpiece blanking until there is a proximity sensor: the screen is main's screen.rs alone)
        self.shown = Some(s);
    }

    /// One action of the Phone and Contacts screens (ui/phone-data.slint, the in-call buttons); a note to show.
    fn act(
        &mut self,
        ui: &mut Renderer,
        action: &str,
        value: &str,
        now_ms: u64,
        contacts: &mut Contacts,
        settings: &mut Settings,
    ) -> Option<String> {
        let id = value.parse::<i64>().unwrap_or(0);
        let note = match action {
            "contact-speed" => {
                let c = contacts.book.get(id as u32)?;
                let number = c.numbers.first()?.1.clone();
                let name = c.title(contacts.order);
                self.speed_pending = Some(number);
                self.show_speed(ui, contacts, settings);
                ui.show("phone", "speed");
                Some(format!("Tap a key for {name}"))
            }
            a if a.starts_with("contact-") || a == "add-contact" || a == "open-contact" => {
                contacts.act(ui, a, value)
            }
            "key" => {
                self.key(value, now_ms);
                None
            }
            "key-long" => self.key_long(ui, value, now_ms, contacts, settings),
            "backspace" => {
                self.number.pop();
                None
            }
            "call" => {
                if self.number.is_empty() {
                    ui.show("phone", "log");
                    self.show_log(ui, contacts, settings, now_ms);
                    None
                } else if let Some(k) = single_key(&self.number) {
                    self.speed_dial(ui, k, now_ms, contacts, settings)
                } else {
                    let n = self.number.clone();
                    self.dial(ui, &n, now_ms)
                }
            }
            "dial" => self.dial(ui, value, now_ms),
            "dial-match" => {
                let n = contacts.book.get(id as u32)?.numbers.first()?.1.clone();
                self.dial(ui, &n, now_ms)
            }
            "log-tab" => {
                self.show_log(ui, contacts, settings, now_ms);
                None
            }
            "log-open" => {
                self.show_detail(ui, id as u32, contacts, now_ms);
                None
            }
            "log-delete" => self.clear_log(&format!("{id},0")),
            "log-clear" => {
                if value == "all" {
                    self.clear_log("0,0")
                } else {
                    let kinds: &[u8] = match ui.log_tab() {
                        0 => &[log::MISSED, log::REJECTED],
                        1 => &[log::RECEIVED],
                        _ => &[log::DIALLED],
                    };
                    kinds.iter().find_map(|k| self.clear_log(&format!("0,{k}")))
                }
            }
            "speed" => self.speed_key(ui, id as usize, now_ms, contacts, settings),
            "speed-show" => {
                self.show_speed(ui, contacts, settings);
                None
            }
            "speed-assign" => {
                let k = ui.assigning();
                ui.set_assigning(0);
                let n = contacts.book.get(id as u32)?.numbers.first()?.1.clone();
                let note = self.assign(k as usize, &n, now_ms, settings);
                self.show_speed(ui, contacts, settings);
                ui.show("phone", "speed");
                Some(note)
            }
            "waiting" => match self.request("waiting", value) {
                0 => None,
                r => Some(format!("Call waiting: not available ({r})")),
            },
            "note-dismiss" => {
                if let Some(d) = self.dtmf.as_mut().filter(|d| d.confirm) {
                    d.confirm = false;
                    d.at = now_ms;
                }
                self.note_until = 0;
                ui.set_note("");
                None
            }
            _ => {
                let r = self.request(action, value);
                // The action name and completion are safe; the descriptor (number/PIN) is never logged.
                dprintln!("neoui: phone {} -> {}", action, r);
                if r < 0 {
                    ui.set_phone_feedback(false, &format!("Could not complete action ({r})"));
                }
                None
            }
        };
        if matches!(
            action,
            "key" | "key-long" | "backspace" | "call" | "dial" | "dial-match"
        ) {
            self.show_dial(ui, contacts, settings);
        }
        note
    }

    /// A keypad key: `*` again within a second cycles `* + p w` (Symbian^3).
    fn key(&mut self, key: &str, now_ms: u64) {
        let last = self.number.chars().last();
        if key == "*"
            && now_ms.saturating_sub(self.star_at) < STAR_CYCLE_MS
            && matches!(last, Some('*' | '+' | 'p' | 'w'))
        {
            self.number.pop();
            self.number.push(match last {
                Some('*') => '+',
                Some('+') => 'p',
                Some('p') => 'w',
                _ => '*',
            });
        } else if self.number.len() < 80 {
            self.number.push_str(key);
        }
        self.star_at = if key == "*" { now_ms } else { 0 };
    }

    /// A key held: on an empty number 1 calls the voice mailbox, 2..9 their speed-dial numbers, 0 types `+`.
    fn key_long(
        &mut self,
        ui: &mut Renderer,
        key: &str,
        now_ms: u64,
        contacts: &Contacts,
        settings: &mut Settings,
    ) -> Option<String> {
        if !self.number.is_empty() {
            self.key(key, now_ms);
            return None;
        }
        match key {
            "0" => {
                self.number.push('+');
                None
            }
            k => {
                let k = k.parse().ok().filter(|k| (1..=9).contains(k))?;
                self.speed_dial(ui, k, now_ms, contacts, settings)
            }
        }
    }

    /// Call speed-dial key `k` (1 = the voice mailbox).
    fn speed_dial(
        &mut self,
        ui: &mut Renderer,
        k: usize,
        now_ms: u64,
        contacts: &Contacts,
        settings: &Settings,
    ) -> Option<String> {
        if k == 1 {
            let vm = settings.get(keys::PHONE_VOICEMAIL).unwrap_or_default();
            if vm.is_empty() {
                ui.show("settings", "phone");
                return Some("Voice mailbox number not defined".into());
            }
            self.number.clear();
            return self.dial(ui, &vm, now_ms);
        }
        let n = speed(settings)[k - 2].clone();
        if n.is_empty() {
            self.show_speed(ui, contacts, settings);
            ui.show("phone", "speed");
            return Some("No speed dial assigned. Tap the key to assign one".into());
        }
        self.number.clear();
        self.dial(ui, &n, now_ms)
    }

    /// A key of the speed-dial grid: call it, assign the waiting contact to it, or pick a contact for it.
    fn speed_key(
        &mut self,
        ui: &mut Renderer,
        k: usize,
        now_ms: u64,
        contacts: &Contacts,
        settings: &mut Settings,
    ) -> Option<String> {
        if !(1..=9).contains(&k) {
            return None;
        }
        if k > 1
            && let Some(n) = self.speed_pending.take()
        {
            let note = self.assign(k, &n, now_ms, settings);
            self.show_speed(ui, contacts, settings);
            return Some(note);
        }
        if k == 1 || !speed(settings)[k - 2].is_empty() {
            return self.speed_dial(ui, k, now_ms, contacts, settings);
        }
        ui.set_assigning(k as i32);
        contacts.show_list(ui);
        ui.show("contacts", "");
        Some(format!("Choose a contact for key {k}"))
    }

    fn assign(&mut self, k: usize, number: &str, now_ms: u64, settings: &mut Settings) -> String {
        if !(2..=9).contains(&k) {
            return "Not assigned".into();
        }
        let mut cells = speed(settings);
        cells[k - 2] = number.into();
        let blob = cells.join("\n").into_bytes();
        match settings::set(
            settings,
            Caller::Shell,
            keys::PHONE_SPEED.name(),
            Value::Bytes(blob),
            now_ms,
        ) {
            Ok(_) => format!("Number assigned for key {k}"),
            Err(e) => format!("Not assigned ({e})"),
        }
    }

    pub fn show_speed(&self, ui: &mut Renderer, contacts: &Contacts, settings: &Settings) {
        let cells = speed(settings);
        let names: Vec<String> = cells
            .iter()
            .map(|n| {
                if n.is_empty() {
                    String::new()
                } else {
                    contacts.name_of(n).unwrap_or_default()
                }
            })
            .collect();
        let shown: Vec<(&str, &str)> = names
            .iter()
            .zip(&cells)
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        ui.set_speed(&shown);
    }

    /// The dialer's number and the contacts its digits spell (Contact search).
    fn show_dial(&self, ui: &mut Renderer, contacts: &Contacts, settings: &Settings) {
        let smart = settings.get(keys::PHONE_SMART_DIAL).unwrap_or(true)
            && self.number.bytes().all(|c| c.is_ascii_digit());
        let rows: Vec<(u32, String, String)> = if smart {
            t9::smart_matches(&contacts.book, &self.number, contacts.order)
                .into_iter()
                .take(3)
                .filter_map(|id| contacts.book.get(id))
                .filter_map(|c| {
                    let n = c.numbers.first()?.1.clone();
                    Some((c.id, c.title(contacts.order), n))
                })
                .collect()
        } else {
            Vec::new()
        };
        let shown: Vec<(u32, &str, &str)> = rows
            .iter()
            .map(|(id, a, b)| (*id, a.as_str(), b.as_str()))
            .collect();
        ui.set_dial(&self.number, &shown);
    }

    /// Dial what was typed or chosen, as the Symbian^3 parser reads it (docs/symbian-phone-sources.md §5).
    fn dial(&mut self, ui: &mut Renderer, typed: &str, now_ms: u64) -> Option<String> {
        let flags = Flags {
            send: true,
            in_call: self.in_call(),
        };
        let note = match dial::classify(typed, flags) {
            Kind::Number { clir, number, dtmf } => {
                let wire = format!("{}{}", clir.map_or("", |c| c.prefix()), number);
                let r = self.request("dial", &wire);
                dprintln!("neoui: phone dial -> {}", r);
                if r < 0 {
                    if self.shown.is_none_or(|s| !live(&s)) {
                        ui.set_call("Call failed", &number, false);
                        ui.show_call();
                        self.close_at = Some(now_ms + CLOSE_MS);
                    }
                    return Some(format!("Call failed ({r})"));
                }
                self.dtmf_after = (!dtmf.is_empty()).then_some(dtmf);
                ui.show_call();
                None
            }
            Kind::Emergency(_) => {
                Some("Emergency calls are not supported on this phone yet".into())
            }
            Kind::Ss { action, code, .. } if code == "43" => {
                let v = match action {
                    Action::Activate | Action::Register => "on",
                    Action::Deactivate | Action::Erase => "off",
                    Action::Interrogate => "query",
                };
                match self.request("waiting", v) {
                    0 => Some("Call waiting: asking the network…".into()),
                    r => Some(format!("Call waiting: not available ({r})")),
                }
            }
            Kind::InCall { cmd: 2, call: None } => {
                self.request("swap", "");
                None
            }
            Kind::InCall { cmd: 3, call: None } => {
                self.request("merge", "");
                None
            }
            Kind::None => return None,
            _ => Some("Not supported yet".into()),
        };
        self.number.clear();
        note
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

    fn clear_log(&mut self, what: &str) -> Option<String> {
        match self.request("log-clear", what) {
            0 => None,
            r => Some(format!("Not cleared ({r})")),
        }
    }

    /// Missed calls since Recents' Missed tab was last open.
    fn new_missed(&self, settings: &Settings) -> u32 {
        let seen = settings.get(keys::PHONE_MISSED_SEEN).unwrap_or(0) as u32;
        self.log
            .iter()
            .filter(|e| e.kind == log::MISSED && e.id > seen)
            .count() as u32
    }

    /// Recents' current tab: one row per number (repeats counted), newest first.
    pub fn show_log(
        &mut self,
        ui: &mut Renderer,
        contacts: &Contacts,
        settings: &mut Settings,
        now_ms: u64,
    ) {
        let tab = ui.log_tab();
        let kinds: &[u8] = match tab {
            0 => &[log::MISSED, log::REJECTED],
            1 => &[log::RECEIVED],
            _ => &[log::DIALLED],
        };
        let entries: Vec<&LogEntry> = self
            .log
            .iter()
            .filter(|e| kinds.contains(&e.kind))
            .collect();
        let (utc, offset) = e32::time::utc_now();
        let rows: Vec<(u32, String, String, String, u8, u32)> =
            recents::group(entries.iter().map(|e| e.number.as_str()))
                .into_iter()
                .map(|(i, count)| {
                    let e = entries[i];
                    let n = e.number.as_str();
                    let name = contacts.name_of(n);
                    let (title, _) = caller_line(name.as_deref(), n, e.presentation, false);
                    let subtitle = match (e.kind, name.is_some()) {
                        (log::REJECTED, _) => String::from("Rejected"),
                        (_, true) => n.into(),
                        _ => String::new(),
                    };
                    let when = recents::when(e.utc, utc * 1000, offset);
                    (e.id, title, subtitle, when, e.kind, count)
                })
                .collect();
        let items: Vec<LogItem> = rows
            .iter()
            .map(|(id, title, subtitle, when, kind, count)| LogItem {
                id: *id,
                title,
                subtitle,
                when,
                kind: *kind,
                count: *count,
            })
            .collect();
        ui.set_log(&items);
        // the Missed tab on screen: its calls are seen
        if tab == 0 && ui.screen() == "phone" && ui.view() == "log" {
            let newest = self
                .log
                .iter()
                .filter(|e| e.kind == log::MISSED)
                .map(|e| e.id)
                .max()
                .unwrap_or(0);
            if newest as i32 > settings.get(keys::PHONE_MISSED_SEEN).unwrap_or(0) {
                let _ = settings::set(
                    settings,
                    Caller::Shell,
                    keys::PHONE_MISSED_SEEN.name(),
                    Value::Int(newest as i32),
                    now_ms,
                );
                let failed = self.shown.is_some_and(|s| s.modem == modem::FAILED);
                ui.set_phone_availability(failed, 0);
            }
        }
    }

    fn show_detail(&self, ui: &mut Renderer, id: u32, contacts: &Contacts, _now_ms: u64) {
        let Some(e) = self.log.iter().find(|e| e.id == id) else {
            return;
        };
        let n = e.number.as_str();
        let name = contacts.name_of(n);
        let (title, _) = caller_line(name.as_deref(), n, e.presentation, false);
        let (utc, offset) = e32::time::utc_now();
        let duration = match e.kind {
            log::DIALLED | log::RECEIVED => recents::duration(e.duration_s),
            _ => String::new(),
        };
        ui.set_log_detail(&LogDetailItem {
            id,
            title: &title,
            number: n,
            direction: match e.kind {
                log::DIALLED => "Out",
                log::RECEIVED => "In",
                log::REJECTED => "Rejected",
                _ => "Missed",
            },
            when: &recents::when_long(e.utc, utc * 1000, offset),
            duration: &duration,
            known: name.is_some(),
        });
    }

    /// The selected call's connection: its start (for the duration), the tones after its number, and the summary
    /// note when the last call ends.
    fn track_connection(
        &mut self,
        ui: &mut Renderer,
        s: &Status,
        now_ms: u64,
        settings: &Settings,
    ) {
        let c = s.call;
        if c.state == call::CONVERSATION && self.connected.is_none_or(|(id, _)| id != c.id) {
            self.connected = Some((c.id, now_ms));
            if !c.incoming
                && let Some(rest) = self.dtmf_after.take()
            {
                self.dtmf = Some(Dtmf {
                    rest,
                    at: now_ms + TONE_GAP_MS,
                    confirm: false,
                });
            }
        }
        if !live(s) {
            if let Some((_, since)) = self.connected.take()
                && settings.get(keys::PHONE_SUMMARY).unwrap_or(false)
            {
                let t = recents::duration(((now_ms - since) / 1000) as u32);
                self.note(ui, &format!("Call Terminated · Time {t}"), now_ms);
            }
            self.dtmf = None;
            self.dtmf_after = None;
        }
    }

    fn call_time(&self, now_ms: u64, settings: &Settings) -> String {
        match self.connected {
            Some((_, since)) if settings.get(keys::PHONE_SHOW_DURATION).unwrap_or(false) => {
                recents::duration(((now_ms - since) / 1000) as u32)
            }
            _ => String::new(),
        }
    }

    /// The call screen's connected time, every second while it shows.
    fn show_call_time(&self, ui: &mut Renderer, now_ms: u64, settings: &Settings) {
        if self.connected.is_some() {
            ui.set_call_time(&self.call_time(now_ms, settings));
        }
    }

    /// Send the next tone after a number: `p` pauses, `w` asks first (docs/symbian-phone-sources.md §4).
    fn dtmf_step(&mut self, ui: &mut Renderer, now_ms: u64) {
        let Some(d) = self.dtmf.as_mut() else { return };
        if d.confirm || now_ms < d.at {
            return;
        }
        let Some(c) = d.rest.chars().next() else {
            self.dtmf = None;
            return;
        };
        d.rest.remove(0);
        match c {
            'p' => d.at = now_ms + PAUSE_MS,
            'w' => {
                d.confirm = true;
                let text = format!("Send tones {}? Tap to send", d.rest);
                ui.set_note(&text);
                self.note_until = now_ms + 60_000;
                return;
            }
            '0'..='9' | '*' | '#' => {
                d.at = now_ms + TONE_GAP_MS;
                let key = String::from(c);
                self.request("dtmf", &key);
            }
            _ => {}
        }
        if self.dtmf.as_ref().is_some_and(|d| d.rest.is_empty()) {
            self.dtmf = None;
        }
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
        let target = value.parse::<u8>().unwrap_or(0) as i32;
        let function = match action {
            "dial" | "pin" => {
                args.desc8(0, &descriptor);
                if action == "dial" {
                    op::DIAL
                } else {
                    op::VERIFY_PIN
                }
            }
            "answer" | "hangup" => {
                args.int(0, target);
                if action == "answer" {
                    op::ANSWER
                } else {
                    op::HANGUP
                }
            }
            "hold" | "resume" | "swap" | "merge" => {
                args.int(
                    0,
                    match action {
                        "hold" => manage::HOLD,
                        "resume" => manage::RESUME,
                        "swap" => manage::SWAP,
                        _ => manage::MERGE,
                    },
                );
                args.int(1, target);
                op::MANAGE
            }
            "mute" | "speaker" | "airplane" => {
                args.int(0, i32::from(value == "1"));
                match action {
                    "mute" => op::MUTE,
                    "speaker" => op::SPEAKER,
                    _ => op::AIRPLANE,
                }
            }
            "volume" => {
                args.int(0, value.parse().unwrap_or(8));
                op::VOLUME
            }
            "dtmf" => {
                args.int(0, value.bytes().next().map_or(-1, i32::from));
                op::DTMF
            }
            "retry" => op::RETRY,
            "waiting" => {
                args.int(
                    0,
                    match value {
                        "on" => waiting::ACTIVATE,
                        "off" => waiting::DEACTIVATE,
                        _ => waiting::QUERY,
                    },
                );
                op::CALL_WAITING
            }
            "log-clear" => {
                let (id, kind) = value.split_once(',').unwrap_or((value, "0"));
                args.int(0, id.parse().unwrap_or(0));
                args.int(1, kind.parse().unwrap_or(0));
                op::CLEAR_LOG
            }
            _ => return -5,
        };
        session.send_receive(function, &args).unwrap_or_else(|e| e)
    }
}

/// The speed-dial numbers of keys 2..=9 (`phone.speed`).
fn speed(settings: &Settings) -> [String; 8] {
    let blob = settings
        .get(keys::PHONE_SPEED)
        .unwrap_or(Bytes(Vec::new()))
        .0;
    let text = String::from_utf8_lossy(&blob);
    let mut cells: [String; 8] = Default::default();
    for (i, n) in text.split('\n').take(8).enumerate() {
        cells[i] = n.into();
    }
    cells
}

/// A number that is one key 2..=9 (Call on it speed-dials, Symbian^3).
fn single_key(n: &str) -> Option<usize> {
    match n.as_bytes() {
        [d @ b'2'..=b'9'] => Some(usize::from(d - b'0')),
        _ => None,
    }
}

fn live(status: &Status) -> bool {
    status
        .calls
        .as_slice()
        .iter()
        .any(|c| !matches!(c.state, call::NONE | call::END))
}

fn state_text(state: u8, incoming: bool) -> &'static str {
    match state {
        call::NONE => "",
        call::INCOMING => "Incoming call",
        call::WAITING => "Call waiting",
        call::ALERTING => "Ringing",
        call::CONVERSATION => "Connected",
        call::HOLD => "On hold",
        call::DISCONNECTING => "Ending…",
        call::END => "Call ended",
        _ if incoming => "Incoming…",
        _ => "Calling…",
    }
}

// SPDX-License-Identifier: GPL-3.0-only
//! NeoModem's C7 client. Commands use one session; a separately owned notification future keeps status updates
//! outstanding without blocking input or holding self-referential descriptor pointers in `Phone`.

use alloc::format;
use alloc::vec::Vec;
use e32::dprintln;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::process::Process;
use e32::sys::{TPtr8, TPtrC8};
use modem_ipc::{Status, call, manage, modem, op, rat, reg, sim};
use neoui_proto::settings::keys;
use neoui_render::{CallView, Renderer};

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
}

/// Cancels the held request before its future and descriptor storage are dropped.
struct Cancel<'a>(&'a Session);
impl Drop for Cancel<'_> {
    fn drop(&mut self) {
        let _ = self.0.send_receive(op::CANCEL_NOTIFY, &TIpcArgs::new());
    }
}

impl Watch {
    pub fn new() -> Self {
        Self {
            session: None,
            revision: u32::MAX,
        }
    }

    pub async fn next(mut self) -> (Self, Option<Status>) {
        if self.session.is_none() {
            self.session = connect().ok();
            self.revision = u32::MAX;
        }
        let Some(session) = self.session.as_ref() else {
            e32::time::delay(500_000).await;
            return (self, None);
        };
        let mut bytes = [0u8; Status::SIZE];
        let result = {
            let mut descriptor = TPtr8::new(&mut bytes);
            let mut args = TIpcArgs::new();
            args.des8(0, &mut descriptor);
            args.int(1, self.revision as i32);
            let future = session.send(op::NOTIFY, &args);
            let future = core::pin::pin!(future);
            // Declared after the pinned future: cancellation runs first on every exit path.
            let _cancel = Cancel(session);
            future.await
        };
        let status = result.ok().and_then(|_| Status::from_bytes(&bytes));
        if let Some(s) = status {
            self.revision = s.revision;
        } else {
            self.session = None;
            e32::time::delay(500_000).await;
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
    off: bool,
    /// A deliberate power-key wake stays awake for this call generation.
    woken_for: Option<(u32, u8)>,
    preferences: Option<(u32, bool, modem_ipc::Ring)>,
    preferences_retry_at: u64,
}

const CLOSE_MS: u64 = 2000;

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
            off: false,
            woken_for: None,
            preferences: None,
            preferences_retry_at: 0,
        }
    }

    pub fn observe(&mut self, status: Option<Status>) {
        self.update = status;
        self.disconnected = status.is_none();
        if status.is_none() {
            self.session = None;
            self.preferences = None;
        }
    }

    pub fn screen_off(&self) -> bool {
        self.off
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

    pub fn modal(&self) -> bool {
        self.shown
            .is_some_and(|s| matches!(s.sim, sim::PIN_REQUIRED | sim::BLOCKED))
    }

    pub fn wake_screen(&mut self) {
        if !self.off {
            return;
        }
        if self.set_screen(true, self.shown.map(|s| (s.generation, s.call.id))) {
            self.woken_for = self.shown.map(|s| (s.generation, s.call.id));
        }
    }

    fn set_screen(&mut self, on: bool, context: Option<(u32, u8)>) -> bool {
        let started = e32::time::millis();
        let (generation, call_id) = context.unwrap_or((0, 0));
        dprintln!(
            "neoui: display enter t={} on={} gen={} call={}",
            started,
            on,
            generation,
            call_id
        );
        // EDisplayHalSetState = 20 in u32hal.h: a1 is a scalar TBool, not a pointer to storage.
        // SAFETY: group 4, function 20 accepts only scalar a1 and ignores the null a2.
        let r = unsafe {
            e32::sys::ui::hal_function(4, 20, usize::from(on) as *mut u8, core::ptr::null_mut())
        };
        if r == 0 {
            self.off = !on;
        }
        dprintln!(
            "neoui: display exit t={} on={} gen={} call={} result={} elapsed={}ms",
            e32::time::millis(),
            on,
            generation,
            call_id,
            r,
            e32::time::millis().saturating_sub(started)
        );
        r == 0
    }

    pub fn poll(&mut self, ui: &mut Renderer, now_ms: u64) {
        if !self.present {
            return;
        }
        if self.disconnected {
            let context = self.shown.map(|s| (s.generation, s.call.id));
            self.disconnected = false;
            self.shown = None;
            self.close_at = None;
            self.woken_for = None;
            ui.set_calls(&[]);
            ui.set_call("Call interrupted", "", false);
            ui.set_call_audio(false, false, -1);
            ui.set_network(0, "No service", "");
            ui.set_phone_feedback(false, "Modem connection lost");
            if self.off {
                self.set_screen(true, context);
            }
        }
        if self.close_at.is_some_and(|t| now_ms >= t) {
            self.close_at = None;
            if self.shown.is_none_or(|s| !live(&s)) {
                ui.close_call();
            }
        }
        for (action, value) in ui.take_phone() {
            let r = self.request(&action, &value);
            // The action name and completion are safe; the descriptor (number/PIN) is never logged.
            dprintln!("neoui: phone {} -> {}", action, r);
            if r < 0 {
                ui.set_phone_feedback(false, &format!("Could not complete action ({r})"));
                if action == "dial" && self.shown.is_none_or(|s| !live(&s)) {
                    ui.set_call("Call failed", &value, false);
                    self.close_at = Some(now_ms + CLOSE_MS);
                }
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
        ui.set_call(
            state_text(s.call.state, s.call.incoming),
            s.call.number.as_str(),
            matches!(s.call.state, call::INCOMING | call::WAITING),
        );
        ui.set_call_audio(
            s.audio.muted,
            s.audio.speaker,
            if s.audio.on {
                s.audio.volume as i32
            } else {
                -1
            },
        );
        let calls: Vec<_> = s
            .calls
            .as_slice()
            .iter()
            .filter(|c| !matches!(c.state, call::NONE | call::END))
            .map(|c| CallView {
                id: c.id,
                number: c.number.as_str(),
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
        ui.set_sim(
            s.sim == sim::PIN_REQUIRED,
            s.sim == sim::BLOCKED,
            (s.pin_attempts != u8::MAX).then_some(s.pin_attempts),
        );
        ui.set_phone_availability(s.modem == modem::FAILED, s.missed);
        let earpiece = s.audio.on
            && !s.audio.speaker
            && !incoming
            && s.calls
                .as_slice()
                .iter()
                .any(|c| c.state == call::CONVERSATION);
        let key = (s.generation, s.call.id);
        if earpiece && self.woken_for != Some(key) && !self.off {
            self.set_screen(false, Some(key));
        } else if !earpiece {
            self.woken_for = None;
            if self.off {
                self.set_screen(true, Some(key));
            }
        }
        self.shown = Some(s);
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
                    } as i32,
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
            "silent" | "vibrate" | "ring-volume" => {
                let mut ring = self.shown.map_or_else(modem_ipc::Ring::default, |s| s.ring);
                match action {
                    "silent" => ring.silent = value == "1",
                    "vibrate" => ring.vibrate = value == "1",
                    _ => ring.volume = value.parse().unwrap_or(ring.volume),
                }
                args.int(0, i32::from(ring.volume));
                args.int(1, i32::from(ring.silent));
                args.int(2, i32::from(ring.vibrate));
                op::RING_SETTINGS
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

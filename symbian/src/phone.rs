// SPDX-License-Identifier: GPL-3.0-only
//! The modem in the shell (NeoSymbian telephony C5): SYSSTART starts MODEMD.EXE at boot (it is the system starter;
//! nothing else launches services), then talks to its NeoModem server: the status bar's operator and bars, the Phone
//! screen's call, and the dial / answer / hang up the Phone screen asks for. Without MODEMD in the ROM (QEMU, CI, the
//! installed phone today) all of this stays quiet and the status bar shows its plain signal icon.
//! shortcut: STATUS is polled once a second (an incoming call shows within a second); an async notify (a kept
//! message, like NeoUi NEXT_EVENT) when C7 needs an instant ring.

use e32::dprintln;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::process::Process;
use e32::sys::{TPtr8, TPtrC8};
use modem_ipc::{Status, call, op, rat, reg};
use neoui_render::Renderer;

const POLL_MS: u64 = 1000;

pub struct Phone {
    session: Option<Session>,
    present: bool,
    last_poll: u64,
    /// the last Status shown (the modem's word: the screen keeps no call state of its own)
    shown: Option<Status>,
    /// the call view closes then (ms): 2 s after the call ended or the dial failed
    close_at: Option<u64>,
    /// the volume step last sent: the keys step from it, not from a status up to a second old
    volume: Option<u8>,
}

/// How long "Call ended" / "Call failed" stays before the dial pad comes back.
const CLOSE_MS: u64 = 2000;

impl Phone {
    /// Start MODEMD; a ROM without it gives a phone-less shell.
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
        Phone {
            session: None,
            present,
            last_poll: 0,
            shown: None,
            close_at: None,
            volume: None,
        }
    }

    fn session(&mut self) -> Option<&Session> {
        if self.session.is_none() {
            let (major, minor, build) = modem_ipc::VERSION;
            let v = TVersion {
                major: major as u8,
                minor: minor as u8,
                build: build as u16,
            };
            self.session = Session::connect(modem_ipc::NAME, v).ok(); // not up yet: next time
        }
        self.session.as_ref()
    }

    /// Once per loop turn: the Phone screen's requests, then (each second, or right after a request) the status.
    pub fn poll(&mut self, ui: &mut Renderer, now_ms: u64) {
        if !self.present {
            return;
        }
        if self.close_at.is_some_and(|t| now_ms >= t) {
            self.close_at = None;
            ui.close_call();
        }
        let mut acted = false;
        for (action, number) in ui.take_phone() {
            acted = true;
            let r = self.request(&action, &number);
            dprintln!("neoui: phone {} -> {}", action, r);
            match action.as_str() {
                "dial" if r != 0 => {
                    ui.set_call("Call failed", &number, false);
                    self.close_at = Some(now_ms + CLOSE_MS);
                }
                "hangup" if r != 0 => self.close_at = Some(now_ms), // no call to end: just close the view
                _ => {}
            }
        }
        if !acted && now_ms.saturating_sub(self.last_poll) < POLL_MS {
            return;
        }
        self.last_poll = now_ms;
        let Some(s) = self.status() else {
            return;
        };
        if self.shown == Some(s) {
            return;
        }
        let over = |x: &Status| matches!(x.call.state, call::NONE | call::END);
        if over(&s) && self.shown.as_ref().is_some_and(|x| !over(x)) {
            self.close_at = Some(now_ms + CLOSE_MS); // "Call ended" for a moment, then the dial pad
        }
        self.volume = None;
        let ringing = |x: &Status| x.call.incoming && x.call.state == call::INCOMING;
        if ringing(&s) && !self.shown.as_ref().is_some_and(ringing) {
            ui.show_call();
        }
        let registered = matches!(s.reg, reg::HOME | reg::ROAMING);
        let tech = match s.rat {
            rat::LTE => "LTE",
            rat::UMTS => "3G",
            rat::GSM => "2G",
            _ => "",
        };
        ui.set_network(
            if registered { s.bars as i32 } else { 0 },
            if registered {
                s.operator.as_str()
            } else {
                "No service"
            },
            tech,
        );
        ui.set_call(
            state_text(s.call.state, s.call.incoming),
            s.call.number.as_str(),
            s.call.incoming,
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
        self.shown = Some(s);
    }

    /// A volume key (true = up): during a call it steps the call's volume (and is used up); else false.
    pub fn volume_key(&mut self, up: bool) -> bool {
        let Some(s) = self.shown else {
            return false;
        };
        if !s.audio.on {
            return false;
        }
        let now = self.volume.unwrap_or(s.audio.volume);
        let v = if up {
            (now + 1).min(10)
        } else {
            now.saturating_sub(1)
        };
        self.volume = Some(v);
        let r = self.request("volume", &alloc::format!("{v}"));
        dprintln!("neoui: call volume {} -> {}", v, r);
        self.last_poll = 0; // show it at once
        true
    }

    fn status(&mut self) -> Option<Status> {
        let mut b = [0u8; Status::SIZE];
        let r = {
            let session = self.session()?;
            let mut des = TPtr8::new(&mut b);
            let mut args = TIpcArgs::new();
            args.des8(0, &mut des);
            session.send_receive(op::STATUS, &args)
        };
        if r.is_err() {
            self.session = None; // MODEMD went away: reconnect later
            return None;
        }
        Status::from_bytes(&b)
    }

    fn request(&mut self, action: &str, number: &str) -> i32 {
        let Some(session) = self.session() else {
            return -18;
        };
        let n = TPtrC8::new(number.as_bytes());
        let mut args = TIpcArgs::new();
        let f = match action {
            "dial" => {
                args.desc8(0, &n);
                op::DIAL
            }
            "answer" => op::ANSWER,
            "mute" | "speaker" => {
                args.int(0, (number == "1") as i32);
                if action == "mute" {
                    op::MUTE
                } else {
                    op::SPEAKER
                }
            }
            "volume" => {
                args.int(0, number.parse().unwrap_or(8));
                op::VOLUME
            }
            "dtmf" => {
                args.int(0, number.bytes().next().map_or(-1, i32::from));
                op::DTMF
            }
            _ => op::HANGUP,
        };
        session.send_receive(f, &args).unwrap_or_else(|e| e)
    }
}

fn state_text(state: u8, incoming: bool) -> &'static str {
    match state {
        call::NONE => "",
        call::INCOMING | call::WAITING => "Incoming call",
        call::ALERTING => "Ringing",
        call::CONVERSATION => "Connected",
        call::HOLD => "On hold",
        call::DISCONNECTING => "Ending…",
        call::END => "Call ended",
        _ if incoming => "Incoming…", // setup: the modem refuses an answer until "incoming"
        _ => "Calling…",
    }
}

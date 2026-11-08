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
    shown: Option<Status>,
}

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
        let mut acted = false;
        for (action, number) in ui.take_phone() {
            acted = true;
            let r = self.request(&action, &number);
            dprintln!("neoui: phone {} -> {}", action, r);
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
        self.shown = Some(s);
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
            _ => op::HANGUP,
        };
        session.send_receive(f, &args).map_or_else(|e| e, |r| r)
    }
}

fn state_text(state: u8, incoming: bool) -> &'static str {
    match state {
        call::NONE => "",
        call::INCOMING | call::WAITING => "Incoming call",
        call::ALERTING => "Ringing",
        call::CONVERSATION => "Connected",
        call::HOLD => "On hold",
        call::DISCONNECTING | call::END => "Call ended",
        _ if incoming => "Incoming…", // setup: the modem refuses an answer until "incoming"
        _ => "Calling…",
    }
}

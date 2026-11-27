// SPDX-License-Identifier: GPL-3.0-only
//! Mobile network and Connectivity: what MODEMD reports (NeoModem `STATUS`): the operator, the SIM, the technology
//! while registered (never a made-up "4G", D4), roaming, whether the radio is on, and the user's last request.
use alloc::format;
use alloc::string::String;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sys::TPtr8;
use modem_ipc::{Status, op, rat, reg, sim};
use settings_ui::{SetState, SettingsApp};
use slint::ComponentHandle;

#[derive(Default)]
pub struct Modem {
    session: Option<Session>,
    /// (D11) the SIM PIN's page and its requests
    pub sim: crate::sim_lock::SimLock,
}

impl Modem {
    /// Clear data (SE4a): Phone's call history (`CLEAR_LOG` of every entry) or Messages' texts (`SMS_EDIT`
    /// `EDIT_DELETE_ALL`: blocked senders' messages stay until they expire). MODEMD owns both.
    pub fn clear(&mut self, sms: bool) -> Result<(), i32> {
        self.connect();
        let mut args = TIpcArgs::new();
        let function = if sms {
            args.int(0, modem_ipc::sms::EDIT_DELETE_ALL).int(1, 0);
            op::SMS_EDIT
        } else {
            args.int(0, 0).int(1, 0);
            op::CLEAR_LOG
        };
        let r = self
            .session
            .as_ref()
            .map_or(Err(-18), |s| s.send_receive(function, &args)); // KErrNotReady
        if r.is_err() {
            self.session = None;
        }
        r.map(drop)
    }

    fn connect(&mut self) {
        if self.session.is_none() {
            let (major, minor, build) = modem_ipc::VERSION;
            let v = TVersion {
                major: major as u8,
                minor: minor as u8,
                build: build as u16,
            };
            self.session = Session::connect(modem_ipc::NAME, v).ok();
        }
    }

    /// (D6) Messages' alert setting (`SMS_SETTINGS` get: modem-ipc `sms::ALERT_*`); `None` when MODEMD can't say.
    pub fn sms_alert(&mut self) -> Option<u8> {
        self.connect();
        let mut b = [0u8; modem_ipc::SmsSettings::SIZE];
        let mut des = TPtr8::new(&mut b);
        let mut args = TIpcArgs::new();
        args.int(0, 0).des8(1, &mut des);
        let r = self.session.as_ref()?.send_receive(op::SMS_SETTINGS, &args);
        if r.is_err() {
            self.session = None;
            return None;
        }
        modem_ipc::SmsSettings::from_bytes(&b).map(|s| s.alert)
    }

    /// (D11) the extended status: with the SIM PIN's protection from a 2.9 MODEMD, without it from an older one
    fn status(&mut self) -> Option<(Status, Option<modem_ipc::Protection>)> {
        self.connect();
        let r = crate::sim_lock::read(self.session.as_ref()?);
        if r.is_none() {
            self.session = None; // MODEMD restarted: connect again next time
        }
        r
    }

    /// Read MODEMD's status into the pages (no modem: "No service", "No modem").
    pub fn poll(&mut self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let read = self.status();
        self.sim.show(ui, read.as_ref());
        let Some((st, _)) = read else {
            s.set_operator(String::new().into());
            s.set_sim("No modem".into());
            s.set_rat("".into());
            s.set_radio_online(false);
            return;
        };
        let registered = matches!(st.reg, reg::HOME | reg::ROAMING);
        let tech = match st.rat {
            rat::GSM => "2G",
            rat::UMTS => "3G",
            rat::LTE => "LTE",
            _ => "",
        };
        // registered on a technology the modem doesn't name: still in service
        let shown = match (registered, tech) {
            (false, _) => "",
            (true, "") => "Registered",
            (true, t) => t,
        };
        s.set_rat(shown.into());
        s.set_roaming(st.reg == reg::ROAMING);
        s.set_operator(String::from(st.operator.as_str()).into());
        let sim = match st.sim {
            sim::ABSENT => "No SIM",
            sim::PRESENT if registered => "SIM ready",
            sim::PRESENT => "SIM present, not registered",
            sim::ERROR => "SIM error",
            sim::PIN_REQUIRED => "SIM PIN needed",
            sim::BLOCKED => "SIM blocked",
            _ => "SIM unknown",
        };
        s.set_sim(sim.into());
        s.set_radio_online(st.radio_online);
        let feedback = if st.pending {
            String::from("Applying…")
        } else if st.result < 0 {
            format!("Action failed ({})", st.result)
        } else {
            String::new()
        };
        s.set_phone_feedback(feedback.into());
    }
}

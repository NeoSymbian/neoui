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
}

impl Modem {
    fn status(&mut self) -> Option<Status> {
        if self.session.is_none() {
            let (major, minor, build) = modem_ipc::VERSION;
            let v = TVersion {
                major: major as u8,
                minor: minor as u8,
                build: build as u16,
            };
            self.session = Session::connect(modem_ipc::NAME, v).ok();
        }
        let mut b = [0u8; Status::SIZE];
        let mut des = TPtr8::new(&mut b);
        let mut args = TIpcArgs::new();
        args.des8(0, &mut des);
        let r = self.session.as_ref()?.send_receive(op::STATUS, &args);
        if r.is_err() {
            self.session = None; // MODEMD restarted: connect again next time
            return None;
        }
        Status::from_bytes(&b)
    }

    /// Read MODEMD's status into the pages (no modem: "No service", "No modem").
    pub fn poll(&mut self, ui: &SettingsApp) {
        let s = ui.global::<SetState>();
        let Some(st) = self.status() else {
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

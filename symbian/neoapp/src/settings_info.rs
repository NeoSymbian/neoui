// SPDX-License-Identifier: GPL-3.0-only
//! Protocol 1.20's Settings snapshots and owner work on the app side (docs neoui-shell/protocol.md "Settings
//! snapshots, owner reports and panel policy"); the codecs are `neoui_proto::settings_app::wire`.
//!
//! - Settings reads a paged kind whole with [`App::settings_records`] (the cursor loop; one restart when the snapshot
//!   changes under it), asks for work with [`App::settings_job`] (the ticket) and reads an old app's small icon with
//!   [`App::entry_thumb`]. `AppEvent::SettingsInfo` says a kind it read changed.
//! - An app that owns data registers with [`App::register_owner`]; on `AppEvent::OwnerWork` / `AppEvent::ClearTicket`
//!   it takes the work with [`App::claim_owner_work`] (nothing is deleted before that) and reports with
//!   [`App::finish_owner_work`].

use crate::App;
use alloc::vec::Vec;
use e32::ipc::TIpcArgs;
use e32::sys::{TPtr8, TPtrC8};
use neoui_proto::ipc as proto;
use neoui_proto::settings_app::wire::{self, OwnerReport, OwnerWork, Traversal};

impl App {
    /// One page of paged `SETTINGS_INFO` kind `kind` from record `first` (`expected_rev` 0 on a first read): the raw
    /// page (`wire::parse_page` reads it). `Err(wire::KERR_NOT_READY)` when the snapshot changed.
    pub fn settings_info_page(
        &self,
        kind: i32,
        first: u32,
        expected_rev: u32,
    ) -> Result<Vec<u8>, i32> {
        let mut buf = alloc::vec![0u8; wire::PAGE_MAX];
        let n = {
            let mut des = TPtr8::new(&mut buf);
            let mut args = TIpcArgs::new();
            args.int(0, kind)
                .int(1, first as i32)
                .des8(2, &mut des)
                .int(3, expected_rev as i32);
            self.server.send_receive(proto::SETTINGS_INFO, &args)?;
            des.len()
        };
        buf.truncate(n);
        Ok(buf)
    }

    /// Paged kind `kind` whole: its revision and records, all from one snapshot. When it changes between pages the
    /// read starts again once; a second change gives `Err(wire::KERR_NOT_READY)` (keep the last result, marked stale,
    /// and read again from the app loop). A damaged page gives `KErrCorrupt`.
    pub fn settings_records(&self, kind: i32) -> Result<(u32, Vec<Vec<u8>>), i32> {
        let k = u8::try_from(kind).map_err(|_| neoui_proto::settings::kerr::ARGUMENT)?;
        let mut restarted = false;
        loop {
            let mut t = Traversal::new(k);
            let done = loop {
                let (first, rev) = t.request();
                let page = match self.settings_info_page(kind, first, rev) {
                    Err(wire::KERR_NOT_READY) => break false,
                    r => r?,
                };
                match t.feed(&page) {
                    Some(true) => break true,
                    Some(false) => {}
                    None => return Err(neoui_proto::settings::kerr::CORRUPT),
                }
            };
            if done {
                return Ok(t.finish());
            }
            if restarted {
                return Err(wire::KERR_NOT_READY);
            }
            restarted = true;
        }
    }

    /// A `SETTINGS_ACTION` that answers with a ticket (1.20: `ACTION_MEASURE_APP`, `ACTION_CLEAR`'s confirmed form;
    /// the request from `wire::MeasureRequest` / `wire::ClearRequest`): INFO 13 has its outcome. Retry with the same
    /// request (and nonce) to get the same ticket.
    pub fn settings_job(&self, action: i32, request: &[u8]) -> Result<u32, i32> {
        let des = TPtrC8::new(request);
        let mut args = TIpcArgs::new();
        args.int(0, action).desc8(1, &des);
        let r = self.server.send_receive(proto::SETTINGS_ACTION, &args)?;
        u32::try_from(r)
            .ok()
            .filter(|&t| t > 0)
            .ok_or(neoui_proto::settings::kerr::ARGUMENT)
    }

    /// An old app's launcher icon at most 32 × 32 (`proto::ENTRY_THUMB`, the `ipc::ENTRY` icon encoding; empty = the
    /// generic icon). `rev` is INFO 8's revision it goes with (0: any); `Err(wire::KERR_NOT_READY)` when it moved on.
    pub fn entry_thumb(&self, id: u32, rev: u32) -> Result<Vec<u8>, i32> {
        let mut buf = alloc::vec![0u8; wire::THUMB_MAX];
        let n = {
            let mut des = TPtr8::new(&mut buf);
            let mut args = TIpcArgs::new();
            args.int(0, id as i32).des8(1, &mut des).int(2, rev as i32);
            self.server.send_receive(proto::ENTRY_THUMB, &args)?;
            des.len()
        };
        buf.truncate(n);
        Ok(buf)
    }

    fn owner_report(&self, mode: i32, ticket: u32, payload: &[u8]) -> Result<i32, i32> {
        let des = TPtrC8::new(payload);
        let mut args = TIpcArgs::new();
        args.int(0, mode).int(1, ticket as i32).desc8(2, &des);
        self.server.send_receive(proto::OWNER_REPORT, &args)
    }

    /// Register this app as the owner of its data (`proto::OWNER_REPORT` mode 0) with `wire::capability` bits: the
    /// shell then wakes it for work. Again after every connect.
    pub fn register_owner(&self, capabilities: u8) -> Result<(), i32> {
        self.owner_report(
            wire::OWNER_REGISTER,
            0,
            &wire::encode_register(capabilities),
        )
        .map(drop)
    }

    /// Take the work `ticket` (from `AppEvent::OwnerWork` / `AppEvent::ClearTicket`): what to measure or clear.
    pub fn claim_owner_work(&self, ticket: u32) -> Result<OwnerWork, i32> {
        let mut buf = [0u8; wire::OWNER_WORK_MAX];
        let n = {
            let mut des = TPtr8::new(&mut buf);
            let mut args = TIpcArgs::new();
            args.int(0, wire::OWNER_CLAIM)
                .int(1, ticket as i32)
                .des8(2, &mut des);
            self.server.send_receive(proto::OWNER_REPORT, &args)?;
            des.len()
        };
        OwnerWork::decode(&buf[..n]).ok_or(neoui_proto::settings::kerr::CORRUPT)
    }

    /// Report the claimed `ticket`'s result; `Ok` once the shell saved the receipt. Retrying the same report is safe.
    pub fn finish_owner_work(&self, ticket: u32, report: &OwnerReport) -> Result<(), i32> {
        self.owner_report(wire::OWNER_FINISH, ticket, &report.encode())
            .map(drop)
    }
}

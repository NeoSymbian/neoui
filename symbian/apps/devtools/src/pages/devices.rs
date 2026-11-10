// SPDX-License-Identifier: GPL-3.0-only
//! Devices: every component by section, its state and source, and "Run checks".

use super::{Ctx, rows};
use crate::state::{Model, Registry, View};
use crate::{DevState, DeviceRow};
use alloc::format;
use alloc::rc::Rc;
use neoui_proto::devtools::device::{Device, State, Summary};

/// DeviceRow.level: 0 ok, 1 partial, 2 off / starting / unknown, 3 no driver, 4 problem.
pub fn level(state: State) -> i32 {
    match state {
        State::Ok => 0,
        State::Partial => 1,
        State::NoDriver => 3,
        s if s.is_problem() => 4,
        _ => 2,
    }
}

pub fn bind(ctx: &Rc<Ctx>, s: &DevState<'_>) {
    let c = ctx.clone();
    s.on_device_chosen(move |i| {
        c.model.borrow_mut().device = usize::try_from(i).unwrap_or(0);
        c.model.borrow_mut().dev_result.clear();
        c.open(View::Device);
    });
    let c = ctx.clone();
    s.on_run_checks(move || {
        c.refresh_devices();
        c.run_checks();
        c.show();
    });
}

fn row(d: &Device, first: bool, m: &Model) -> DeviceRow {
    DeviceRow {
        name: d.name.as_str().into(),
        group: d.group.label().into(),
        first,
        state: d.state.label().into(),
        level: level(d.state),
        detail: if d.detail.is_empty() {
            match d.state {
                State::NoDriver => "not ported yet".into(),
                _ => d.state.label().into(),
            }
        } else {
            d.detail.as_str().into()
        },
        source: d.source.label().into(),
        check: m.check(&d.key).map_or("", |c| c.word()).into(),
    }
}

pub fn show(s: &DevState<'_>, m: &Model) {
    let sum = Summary::of(&m.devices);
    s.set_devices_summary(
        format!(
            "{} OK · {} partial · {} errors",
            sum.ok, sum.partial, sum.problems
        )
        .into(),
    );
    let checks = if m.checks.is_empty() {
        alloc::string::String::new()
    } else {
        let failed = m.checks.iter().filter(|(_, c)| c.word() == "fail").count();
        format!(" · checks: {failed} failed")
    };
    s.set_devices_note(
        match m.registry {
            Registry::Listed { devices, bad: 0 } => {
                format!(
                    "Kernel: {devices} drivers · {} not ported{checks}",
                    sum.no_driver
                )
            }
            Registry::Listed { devices, bad } => {
                format!("Kernel: {devices} drivers, {bad} unreadable lines{checks}")
            }
            Registry::Missing(e) => {
                format!("No device registry in this kernel ({e}){checks}")
            }
        }
        .into(),
    );
    s.set_checking(false);
    s.set_devices(rows(m.devices.iter().enumerate().map(|(i, d)| {
        let first = i == 0 || m.devices[i - 1].group != d.group;
        row(d, first, m)
    })));
}

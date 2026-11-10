// SPDX-License-Identifier: GPL-3.0-only
//! More and its pages: Settings (every registered key, read-only, live), Tools (save the logs, the kernel console,
//! power, each power action behind a confirm), About (which builds run, on what).

use super::{Ctx, rows};
use crate::probe;
use crate::state::Model;
use crate::{DevState, Fact, Row};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use e32::dprintln;
use e32::ipc::TIpcArgs;
use e32::time;
use neoapp::{App, hal};
use neoui_proto::devtools::stats::{format_bytes, format_uptime};
use neoui_proto::ipc as proto;
use neoui_proto::settings::REGISTRY;
use neoui_proto::settings::registry::{Limit, Owner};

/// Power actions in Tools' order: restart, restart to fastboot, power off (the UI server carries them out).
const POWER: [i32; 3] = [
    proto::POWER_RESTART,
    proto::POWER_FASTBOOT,
    proto::POWER_OFF,
];

pub fn bind(ctx: &Rc<Ctx>, s: &DevState<'_>) {
    let c = ctx.clone();
    s.on_save_logs(move || {
        c.update(|m| m.tools_status = probe::save_logs(&m.build));
    });
    let c = ctx.clone();
    s.on_console_toggled(move |on| {
        c.update(|m| {
            m.tools_status = match hal::set_console(on) {
                Ok(()) => format!("kernel console {}", if on { "on" } else { "off" }),
                Err(e) => format!("no kernel console on this board ({e})"),
            };
            m.readings.console = Some(hal::console_shown());
        });
    });
    let c = ctx.clone();
    s.on_power(move |i| {
        c.update(|m| {
            m.confirm = (0..3).contains(&i).then_some(i);
            m.tools_status.clear();
        });
    });
    let c = ctx.clone();
    s.on_power_cancelled(move || c.update(|m| m.confirm = None));
    let c = ctx.clone();
    s.on_power_confirmed(move || {
        let Some(i) = c.model.borrow_mut().confirm.take() else {
            return;
        };
        let mut args = TIpcArgs::new();
        args.int(0, POWER[i as usize]);
        dprintln!("devtools: power {}", i);
        // on success the phone restarts or stops here
        let r = c.app.server.send_receive(proto::POWER, &args);
        c.update(|m| m.tools_status = format!("power request failed: {r:?}"));
    });
}

pub fn show_more(s: &DevState<'_>) {
    s.set_settings_summary(format!("{} keys, read-only", REGISTRY.len()).into());
}

pub fn show_settings(s: &DevState<'_>, app: &App) {
    let mut changed = 0;
    s.set_settings(rows(REGISTRY.iter().map(|spec| {
        let value = app.setting_raw(spec.name);
        let default = spec.default.to_value();
        if value.as_ref().is_ok_and(|v| *v != default) {
            changed += 1;
        }
        let owner = match spec.owner {
            Owner::Shell => String::from("shell"),
            Owner::App(a) => format!("app {a}"),
        };
        let limit = match spec.limit {
            Limit::None => String::new(),
            Limit::Range { min, max } => format!(" · {min}..{max}"),
            Limit::MaxLen(n) => format!(" · up to {n} bytes"),
        };
        Row {
            title: spec.name.into(),
            sub: format!("{owner} · {:?}{limit} · default {default}", spec.ty()).into(),
            value: value
                .map_or_else(|e| format!("error {e}"), |v| format!("{v}"))
                .into(),
        }
    })));
    s.set_settings_summary(
        format!(
            "{} keys, {changed} changed from their default; read-only",
            REGISTRY.len()
        )
        .into(),
    );
}

pub fn show_tools(s: &DevState<'_>, m: &Model) {
    s.set_console_on(matches!(m.readings.console, Some(Ok(true))));
    s.set_confirm(m.confirm.unwrap_or(-1));
    s.set_tools_status(m.tools_status.as_str().into());
}

pub fn show_about(s: &DevState<'_>, m: &Model) {
    let mut facts: alloc::vec::Vec<(&str, String)> = alloc::vec![
        ("Kernel", m.build.first().cloned().unwrap_or_default()),
        ("ROM", m.build.get(1).cloned().unwrap_or_default()),
        ("neoui", m.build.get(2).cloned().unwrap_or_default()),
    ];
    if let Some(Ok(slot)) = &m.readings.slot {
        facts.push(("Slots", slot.clone()));
    }
    if let Some(Ok((w, h))) = m.readings.display {
        facts.push(("Display", format!("{w}x{h}")));
    }
    facts.push((
        "CPU",
        format!(
            "{}{}",
            super::home::cpus(m.cpus),
            m.cpu_khz
                .map_or(String::new(), |k| format!(", top {} MHz", k / 1000))
        ),
    ));
    if let Some(mem) = m.memory {
        facts.push((
            "RAM",
            format!(
                "{} ({} free)",
                format_bytes(mem.total_ram),
                format_bytes(mem.free_ram)
            ),
        ));
    }
    facts.push(("Uptime", format_uptime(time::millis())));
    s.set_about(rows(facts.into_iter().map(|(k, v)| Fact {
        key: k.into(),
        value: v.into(),
    })));
}

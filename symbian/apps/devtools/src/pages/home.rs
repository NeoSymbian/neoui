// SPDX-License-Identifier: GPL-3.0-only
//! Home: the system at a glance (uptime, memory, CPU, storage, processes, devices, the modem, the kernel) as cards,
//! then every page of the app.

use super::rows;
use crate::state::Model;
use crate::{DevState, Row, StatCard};
use alloc::format;
use alloc::string::String;
use e32::time;
use neoui_proto::devtools::device::Summary;
use neoui_proto::devtools::stats::{format_bytes, format_percent, format_uptime};
use slint::SharedString;

fn card(
    title: &str,
    target: &str,
    value: String,
    sub: String,
    bars: &[f32],
    level: i32,
) -> StatCard {
    StatCard {
        title: title.into(),
        target: target.into(),
        value: value.into(),
        sub: sub.into(),
        bars: rows(bars.iter().copied()),
        level,
    }
}

/// The kernel build line's board and commit: "kernel 9a259c9 2026-11-09T10:00Z bonito" → ("bonito", "9a259c9").
fn kernel_parts(line: &str) -> (String, String) {
    let words: alloc::vec::Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["kernel", commit, _, board, ..] => ((*board).into(), (*commit).into()),
        _ => ("2010 kernel".into(), String::new()),
    }
}

/// "1 CPU", "8 CPUs".
pub fn cpus(n: u32) -> String {
    format!("{n} CPU{}", if n == 1 { "" } else { "s" })
}

pub fn show(s: &DevState<'_>, m: &Model) {
    let (board, commit) = kernel_parts(m.build.first().map_or("", String::as_str));
    s.set_headline(
        if commit.is_empty() {
            format!("{board}: no NeoSymbian kernel calls on this board")
        } else {
            format!("NeoSymbian · {board} · kernel {commit}")
        }
        .into(),
    );
    let none: &[f32] = &[];
    let (mem_value, mem_sub) = match m.memory {
        Some(mem) if mem.total_ram > 0 => (
            format_percent(1.0 - mem.free_ram as f32 / mem.total_ram as f32),
            format!(
                "{} free of {}",
                format_bytes(mem.free_ram),
                format_bytes(mem.total_ram)
            ),
        ),
        _ => ("-".into(), "no memory figures".into()),
    };
    let cpu_value = m.cpu_load.map_or("-".into(), format_percent);
    let cpu_sub = match (m.cpu_load, m.cpu_khz) {
        (None, _) => format!("{} · load not measured", cpus(m.cpus)),
        (_, Some(khz)) => format!("{} · {} MHz top", cpus(m.cpus), khz / 1000),
        _ => cpus(m.cpus),
    };
    let drive = |d: char| {
        m.readings
            .volumes
            .iter()
            .find(|(x, _)| *x == d)
            .and_then(|(_, v)| v.as_ref().ok())
            .map(|v| format!("{d}: {} free", format_bytes(v.free as u64)))
    };
    let thread_count = m.threads.len();
    let sum = Summary::of(&m.devices);
    let modem = m.devices.iter().find(|d| d.key == "modem");
    let slot = match &m.readings.slot {
        Some(Ok(s)) => s.clone(),
        _ => String::from("no A/B slots"),
    };
    let cpu_bars = m.cpu_history.bars();
    let cards = [
        card(
            "Uptime",
            "",
            format_uptime(time::millis()),
            "since this boot".into(),
            none,
            0,
        ),
        card("Memory", "", mem_value, mem_sub, &m.mem_history.bars(), 0),
        card(
            "CPU",
            "procs",
            cpu_value,
            cpu_sub,
            if m.cpu_load.is_some() {
                &cpu_bars
            } else {
                none
            },
            0,
        ),
        card(
            "Storage",
            "",
            drive('C').unwrap_or_else(|| "C: -".into()),
            drive('E').unwrap_or_else(|| "no E: drive".into()),
            none,
            0,
        ),
        card(
            "Processes",
            "procs",
            format!("{}", m.procs.len()),
            format!("{thread_count} threads"),
            none,
            0,
        ),
        card(
            "Devices",
            "devices",
            format!("{} OK", sum.ok),
            format!(
                "{} partial · {} errors · {} no driver",
                sum.partial, sum.problems, sum.no_driver
            ),
            none,
            if sum.problems > 0 { 4 } else { 0 },
        ),
        card(
            "Modem",
            "devices",
            modem.map_or("-".into(), |d| String::from(d.state.label())),
            modem.map_or(String::new(), |d| d.detail.clone()),
            none,
            if modem.is_some_and(|d| d.state.is_problem()) {
                4
            } else {
                0
            },
        ),
        card("Kernel", "about", board, slot, none, 0),
    ];
    s.set_cards(rows(cards));
    let page = |title: &str, sub: String, key: &str| Row {
        title: title.into(),
        sub: sub.into(),
        value: SharedString::from(key),
    };
    s.set_pages(rows([
        page(
            "Devices",
            format!(
                "{} components, {} with a driver",
                m.devices.len(),
                m.devices.len() - sum.no_driver
            ),
            "devices",
        ),
        page(
            "Console",
            format!("{} log lines this boot", m.live.total()),
            "console",
        ),
        page(
            "Processes",
            format!("{} processes, {thread_count} threads", m.procs.len()),
            "procs",
        ),
        page("Settings", "Every setting and its value".into(), "settings"),
        page("Tools", "Save logs, kernel console, power".into(), "tools"),
        page("About", "Kernel, ROM and neoui builds".into(), "about"),
    ]));
}

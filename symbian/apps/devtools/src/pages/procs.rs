// SPDX-License-Identifier: GPL-3.0-only
//! Procs: every process (name, id, threads, code and data), and a process's own view with its threads.

use super::{Ctx, rows};
use crate::state::{Model, View};
use crate::{DevState, Fact, Row};
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use neoapp::sys::{ProcessInfo, ThreadInfo};
use neoui_proto::devtools::stats::{format_bytes, format_cpu_time};

pub fn bind(ctx: &Rc<Ctx>, s: &DevState<'_>) {
    let c = ctx.clone();
    s.on_proc_chosen(move |i| {
        c.model.borrow_mut().proc = usize::try_from(i).unwrap_or(0);
        c.open(View::Proc);
    });
}

/// `TThreadPriority` in words.
fn priority(p: i32) -> String {
    match p {
        -30 => "null".into(),
        -20 => "much less".into(),
        -10 => "less".into(),
        0 => "normal".into(),
        10 => "more".into(),
        20 => "much more".into(),
        30 => "real time".into(),
        p if p >= 100 => format!("absolute {}", p - 100),
        p => format!("{p}"),
    }
}

fn threads_of<'a>(m: &'a Model, p: &'a ProcessInfo) -> impl Iterator<Item = &'a ThreadInfo> + 'a {
    m.threads.iter().filter(move |t| t.process == p.full)
}

/// Processes, the busiest (by CPU time when measured, else by memory) first.
fn sorted(m: &Model) -> Vec<&ProcessInfo> {
    let cpu = |p: &ProcessInfo| -> u64 { threads_of(m, p).filter_map(|t| t.cpu_us).sum() };
    let mut v: Vec<&ProcessInfo> = m.procs.iter().collect();
    v.sort_by(|a, b| {
        cpu(b).cmp(&cpu(a)).then(
            (u64::from(b.code) + u64::from(b.data)).cmp(&(u64::from(a.code) + u64::from(a.data))),
        )
    });
    v
}

pub fn show(s: &DevState<'_>, m: &Model) {
    let measured = m.threads.iter().any(|t| t.cpu_us.is_some());
    s.set_procs_summary(
        format!(
            "{} processes · {} threads · CPU time {}",
            m.procs.len(),
            m.threads.len(),
            if measured {
                "measured"
            } else {
                "not measured by this kernel"
            }
        )
        .into(),
    );
    s.set_procs(rows(sorted(m).into_iter().map(|p| {
        let n = threads_of(m, p).count();
        Row {
            title: p.name().into(),
            sub: format!("id {} · {n} thread{}", p.id, if n == 1 { "" } else { "s" }).into(),
            value: format_bytes(u64::from(p.code) + u64::from(p.data)).into(),
        }
    })));
}

pub fn show_proc(s: &DevState<'_>, m: &Model) {
    // the view opened on a row of the sorted list
    let list = sorted(m);
    let Some(p) = list.get(m.proc) else { return };
    s.set_proc_name(p.name().into());
    let facts = [
        ("Full name", p.full.clone()),
        ("Process id", format!("{}", p.id)),
        ("Code", format_bytes(u64::from(p.code))),
        ("Data", format_bytes(u64::from(p.data))),
    ];
    s.set_proc_facts(rows(facts.into_iter().map(|(k, v)| Fact {
        key: k.into(),
        value: v.into(),
    })));
    s.set_threads(rows(threads_of(m, p).map(|t| Row {
        title: t.name.as_str().into(),
        sub: format!("priority {}", priority(t.priority)).into(),
        value: t.cpu_us.map_or(String::from("-"), format_cpu_time).into(),
    })));
}

// SPDX-License-Identifier: GPL-3.0-only
//! Console: this boot's or the last boot's kernel log, filtered by tag, followed or scrolled back by whole rows.
//! Only the rows that fit are handed to Slint, and only the ones that changed.

use super::{Ctx, strings};
use crate::DevState;
use crate::probe;
use crate::state::Model;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use e32::dprintln;
use neoapp::hal;
use neoui_proto::devtools::log::Lines;
use slint::{Model as _, SharedString, VecModel};

/// Tag chips after "All".
const TAG_CHIPS: usize = 8;

pub fn bind(ctx: &Rc<Ctx>, s: &DevState<'_>) {
    let c = ctx.clone();
    s.on_console_tag_chosen(move |i| {
        c.update(|m| {
            m.console.tag = usize::try_from(i - 1)
                .ok()
                .and_then(|i| m.console.tags.get(i).cloned());
            m.console.back = 0;
            m.console.follow = true;
            dprintln!("devtools: console tag {:?}", m.console.tag);
        });
    });
    let c = ctx.clone();
    s.on_console_follow_toggled(move || {
        c.update(|m| {
            m.console.follow = !m.console.follow;
            if m.console.follow {
                m.console.back = 0;
            }
        });
    });
    let c = ctx.clone();
    s.on_console_source_toggled(move || {
        c.update(|m| {
            m.console.last = !m.console.last;
            if m.console.last && m.last_boot.is_none() {
                m.last_boot = Some(hal::last_log().map(|bytes| {
                    let mut l = Lines::new();
                    l.push(&bytes);
                    l
                }));
            }
            m.console.back = 0;
            m.console.tag = None;
            dprintln!(
                "devtools: console {}",
                if m.console.last { "last boot" } else { "live" }
            );
        });
    });
    let c = ctx.clone();
    s.on_console_clear(move || {
        c.update(|m| {
            if m.console.last {
                m.last_boot = Some(Ok(Lines::new()));
            } else {
                m.live.clear();
            }
            m.console.back = 0;
        });
    });
    let c = ctx.clone();
    s.on_console_save(move || {
        c.update(|m| {
            m.tools_status = probe::save_logs(&m.build);
        });
    });
    let c = ctx.clone();
    s.on_console_scroll(move |rows| {
        c.update(|m| {
            let max = m.console.matched.saturating_sub(m.console_rows);
            let back = i64::try_from(m.console.back).unwrap_or(0) + i64::from(rows);
            m.console.back = usize::try_from(back.max(0)).unwrap_or(0).min(max);
            m.console.follow = m.console.back == 0;
        });
    });
}

/// The log the Console shows, or why there is none.
fn source(m: &Model) -> Result<&Lines, String> {
    if m.console.last {
        match &m.last_boot {
            Some(Ok(l)) => Ok(l),
            Some(Err(e)) => Err(format!("no last-boot log on this board ({e})")),
            None => Err(String::from("loading")),
        }
    } else {
        match m.log_error {
            Some(e) if m.live.total() == 0 => Err(format!("no kernel log on this board ({e})")),
            _ => Ok(&m.live),
        }
    }
}

/// Keeps the chips current and a scrolled-back view still while lines arrive (called when the log grows).
pub fn follow_new_lines(m: &mut Model) {
    let (tags, matched) = match source(m) {
        Ok(lines) => {
            let tags: Vec<String> = lines
                .tags()
                .into_iter()
                .take(TAG_CHIPS)
                .map(|(t, _)| String::from(t))
                .collect();
            let filter: Vec<&str> = m.console.tag.iter().map(String::as_str).collect();
            (tags, lines.filtered(&filter).count())
        }
        Err(_) => (Vec::new(), 0),
    };
    if !m.console.follow {
        m.console.back += matched.saturating_sub(m.console.matched);
    }
    m.console.matched = matched;
    m.console.tags = tags;
    if let Some(t) = &m.console.tag
        && !m.console.tags.contains(t)
    {
        m.console.tags.push(t.clone());
    }
}

pub fn show(s: &DevState<'_>, m: &Model, rows: &VecModel<SharedString>) {
    let filter: Vec<&str> = m.console.tag.iter().map(String::as_str).collect();
    let (lines, status): (Vec<&str>, String) = match source(m) {
        Ok(l) => {
            let lines: Vec<&str> = l.filtered(&filter).collect();
            let n = lines.len();
            let what = if m.console.last {
                "Last boot"
            } else {
                "This boot"
            };
            let tag = m
                .console
                .tag
                .as_ref()
                .map_or(String::new(), |t| format!(" [{t}]"));
            let place = if m.console.back == 0 {
                String::from("newest")
            } else {
                format!("{} lines back", m.console.back)
            };
            (lines, format!("{what}{tag}: {n} lines · {place}"))
        }
        Err(e) => (Vec::new(), e),
    };
    let end = lines.len().saturating_sub(m.console.back);
    let start = end.saturating_sub(m.console_rows);
    for y in 0..rows.row_count() {
        let text = SharedString::from(
            lines
                .get(start + y)
                .filter(|_| start + y < end)
                .copied()
                .unwrap_or(""),
        );
        if rows.row_data(y).as_ref() != Some(&text) {
            rows.set_row_data(y, text);
        }
    }
    s.set_console_status(status.into());
    s.set_console_follow(m.console.follow);
    s.set_console_last(m.console.last);
    let chosen = m
        .console
        .tag
        .as_ref()
        .and_then(|t| m.console.tags.iter().position(|x| x == t))
        .map_or(0, |i| i + 1);
    s.set_console_tags(strings(
        core::iter::once("All").chain(m.console.tags.iter().map(String::as_str)),
    ));
    s.set_console_tag(i32::try_from(chosen).unwrap_or(0));
}

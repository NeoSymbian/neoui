// SPDX-License-Identifier: GPL-3.0-only
//! A device's own view: what is known about it (driver, source, live values, its check), its tests, and the log
//! lines its drivers wrote.

use super::devices::level;
use super::{Ctx, rows, strings};
use crate::probe;
use crate::state::{Model, View};
use crate::{DevState, Fact};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};
use e32::dprintln;
use neoapp::hal;

/// Log lines shown under a device.
const LOG_LINES: usize = 14;

pub fn bind(ctx: &Rc<Ctx>, s: &DevState<'_>) {
    let c = ctx.clone();
    s.on_dev_action(move |i| {
        let Some(key) = c
            .model
            .borrow()
            .devices
            .get(c.model.borrow().device)
            .map(|d| d.key.clone())
        else {
            return;
        };
        let action = probe::actions(&key)
            .get(usize::try_from(i).unwrap_or(usize::MAX))
            .copied();
        dprintln!("devtools: {} action {:?}", key, action);
        match action {
            Some("Run this check") => {
                c.refresh_devices();
                let mut m = c.model.borrow_mut();
                if let Some(d) = m.devices.get(m.device).cloned() {
                    let check = probe::check(&d, &m.readings);
                    dprintln!(
                        "devtools: check {} {} {}",
                        d.key,
                        check.word(),
                        check.text()
                    );
                    m.dev_result = format!("Check {}: {}", check.word(), check.text());
                    m.checks.retain(|(k, _)| *k != d.key);
                    m.checks.push((d.key, check));
                }
            }
            Some("Touch test") => {
                let mut m = c.model.borrow_mut();
                m.touch = vec![false; crate::pages::tests::CELLS];
                m.touch_last = None;
                drop(m);
                c.open(View::TouchTest);
                return;
            }
            Some("Colour test") => {
                c.model.borrow_mut().fill = 0;
                c.open(View::DisplayTest);
                return;
            }
            Some("Kernel console on or off") => {
                let text = match hal::console_shown() {
                    Ok(on) => match hal::set_console(!on) {
                        Ok(()) => format!("kernel console {}", if on { "off" } else { "on" }),
                        Err(e) => format!("console switch failed ({e})"),
                    },
                    Err(e) => format!("no kernel console on this board ({e})"),
                };
                c.model.borrow_mut().dev_result = text;
                c.refresh_devices();
            }
            Some(a) if a.starts_with("Speed test") => {
                c.model.borrow_mut().dev_result = match probe::speed_test() {
                    Ok(t) => t,
                    Err(e) => format!("speed test failed ({e})"),
                };
            }
            Some(a) if a.starts_with("Loopback") || a.starts_with("Stop") => {
                let what = match a {
                    "Loopback: mic to the loudspeaker" => 1,
                    "Loopback: mic to the earpiece" => 2,
                    _ => 0,
                };
                c.model.borrow_mut().dev_result = probe::audio_test(what);
            }
            Some("Refresh the status") => {
                c.refresh_devices();
                c.model.borrow_mut().dev_result = String::from("refreshed");
            }
            _ => {}
        }
        c.show();
    });
}

pub fn show(s: &DevState<'_>, m: &Model) {
    let Some(d) = m.devices.get(m.device) else {
        return;
    };
    s.set_dev_name(d.name.as_str().into());
    s.set_dev_state(d.state.label().into());
    s.set_dev_level(level(d.state));
    let mut facts: Vec<(String, String)> = vec![
        (
            "Driver".into(),
            if d.driver.is_empty() {
                "none reported".into()
            } else {
                d.driver.clone()
            },
        ),
        (
            "Source".into(),
            match d.source.label() {
                "" => "nothing reports it".into(),
                s => format!("the {s}"),
            },
        ),
    ];
    if !d.detail.is_empty() {
        facts.push(("Status".into(), d.detail.clone()));
    }
    facts.extend(probe::facts(d, &m.readings, m));
    if let Some(c) = m.check(&d.key) {
        facts.push(("Check".into(), format!("{}: {}", c.word(), c.text())));
    }
    s.set_dev_facts(rows(facts.into_iter().map(|(k, v)| Fact {
        key: k.into(),
        value: v.into(),
    })));
    s.set_dev_actions(strings(probe::actions(&d.key)));
    s.set_dev_result(m.dev_result.as_str().into());
    let mine: Vec<&str> = if d.log.is_empty() {
        Vec::new()
    } else {
        m.live.filtered(d.log).collect()
    };
    let start = mine.len().saturating_sub(LOG_LINES);
    s.set_dev_log(strings(mine[start..].iter().copied()));
}

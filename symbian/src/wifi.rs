// SPDX-License-Identifier: GPL-3.0-only
//! Settings -> Wi-Fi: the kernel's scan results (docs wifi.md W3). A fresh scan every 20 s while Settings is open;
//! nothing is joined yet.
use neoui_proto::wifi::{Scan, State};
use neoui_render::Renderer;

pub struct Wifi {
    next_read: u64,
    next_scan: u64,
    last: Option<Scan>,
}

impl Wifi {
    pub fn new() -> Self {
        Self {
            next_read: 0,
            next_scan: 0,
            last: None,
        }
    }

    pub fn poll(&mut self, ui: &mut Renderer, now: u64) {
        if now < self.next_read {
            return;
        }
        let open = ui.screen() == "settings";
        self.next_read = now.saturating_add(if open { 1000 } else { 10_000 });
        let rescan = open && now >= self.next_scan;
        if rescan {
            self.next_scan = now.saturating_add(20_000);
        }
        let scan = neoapp::hal::wifi_scan(rescan).unwrap_or_else(|_| Scan {
            state: State::Error,
            ..Scan::default()
        });
        if self.last.as_ref() != Some(&scan) {
            ui.set_wifi(&scan);
            self.last = Some(scan);
        }
    }
}

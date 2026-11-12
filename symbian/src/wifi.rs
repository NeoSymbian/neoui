// SPDX-License-Identifier: GPL-3.0-only
//! Settings -> Wi-Fi: the kernel's scan results (docs wifi.md W3), scanned the way phones do it:
//! - opening the Wi-Fi page scans at once unless the list is fresh (under 30 s old);
//! - while the page is open it rescans every 30 s, and its refresh button scans now;
//! - nothing is scanned while the page is closed (the Settings root only shows the last list).
//!
//! Nothing is joined yet (W4).
use neoui_proto::wifi::{Scan, State};
use neoui_render::Renderer;

/// A list this old is refreshed when the page opens, and rescanned while it stays open
const FRESH_MS: u64 = 30_000;

pub struct Wifi {
    next_read: u64,
    /// when the latest scan completed (its `scans` counter changed)
    listed_at: Option<u64>,
    page_open: bool,
    asked_at: Option<u64>,
    last: Option<Scan>,
}

impl Wifi {
    pub fn new() -> Self {
        Self {
            next_read: 0,
            listed_at: None,
            page_open: false,
            asked_at: None,
            last: None,
        }
    }

    pub fn poll(&mut self, ui: &mut Renderer, now: u64) {
        let page = ui.screen() == "settings" && ui.view() == "wifi";
        let refresh = ui.take_wifi_refresh();
        let opened = page && !self.page_open;
        self.page_open = page;
        let stale = self
            .listed_at
            .is_none_or(|t| now.saturating_sub(t) >= FRESH_MS);
        // one request at a time: the kernel queues a single scan, so don't ask again for 5 s
        let waiting = self.asked_at.is_some_and(|t| now.saturating_sub(t) < 5000);
        let rescan =
            page && !waiting && (refresh || opened && stale || stale && self.asked_at.is_some());
        if !rescan && !opened && now < self.next_read {
            return;
        }
        self.next_read = now.saturating_add(if page { 500 } else { 5000 });
        if rescan {
            self.asked_at = Some(now);
        }
        let scan = neoapp::hal::wifi_scan(rescan).unwrap_or_else(|_| Scan {
            state: State::Error,
            ..Scan::default()
        });
        if self.last.as_ref().is_none_or(|l| l.scans != scan.scans) && scan.state != State::Off {
            self.listed_at = Some(now);
        }
        if self.last.as_ref() != Some(&scan) || rescan {
            // a requested scan shows as searching at once, before the kernel's state catches up
            let mut shown = scan.clone();
            if rescan && shown.state == State::Ready {
                shown.state = State::Scanning;
            }
            ui.set_wifi(&shown);
            self.last = Some(scan);
        }
    }
}

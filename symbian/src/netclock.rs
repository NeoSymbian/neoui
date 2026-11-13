// SPDX-License-Identifier: GPL-3.0-only
//! Settings -> Date & time "Automatic time": the network's time sets the clock. The mobile network's time (NITZ,
//! through MODEMD's status) brings the zone too when "Automatic time zone" is on; on Wi-Fi the internet check's HTTP
//! `Date:` header sets the time only. [`crate::clock::Keeper`] saves whatever is set within 5 s.

use crate::settings::Settings;
use e32::{dprintln, time};
use neoui_proto::clock::datetime;
use neoui_proto::settings::keys;

#[derive(Default)]
pub struct NetClock {
    /// The last NITZ value seen (its count and time: a restarted MODEMD counts from 1 again).
    seen: Option<(u16, u32)>,
}

impl NetClock {
    /// Once per frame. Returns where the time came from when a network time arrived (Settings' "Last set").
    /// `http` = an HTTP Date (Unix seconds) and when it was read (ms since boot).
    pub fn frame(
        &mut self,
        settings: &Settings,
        nitz: Option<modem_ipc::Nitz>,
        http: Option<(i64, u64)>,
        now_ms: u64,
    ) -> Option<&'static str> {
        let fresh = nitz.filter(|n| self.seen != Some((n.seq, n.utc)));
        if let Some(n) = fresh {
            self.seen = Some((n.seq, n.utc));
        }
        if !settings.get(keys::TIME_AUTO).unwrap_or(true) {
            return None;
        }
        let auto_zone = settings.get(keys::TIME_AUTO_ZONE).unwrap_or(true);
        // shortcut: NITZ is taken as of now (MODEMD publishes it at once); its age in the status is not counted
        let (utc, offset, source) = match (fresh, http) {
            (Some(n), _) => (
                i64::from(n.utc),
                auto_zone.then_some(i32::from(n.tz) * 900),
                "Mobile network",
            ),
            (None, Some((date, at))) => (
                date + (now_ms.saturating_sub(at) / 1000) as i64,
                None,
                "Internet",
            ),
            (None, None) => return None,
        };
        if let Some((utc, offset)) = datetime::correction(time::utc_now(), utc, offset) {
            let r = time::set_utc_and_offset(utc, offset);
            dprintln!(
                "neoui: clock: {} -> {} UTC, offset {} s ({:?})",
                source,
                utc,
                offset,
                r
            );
        }
        Some(source)
    }
}

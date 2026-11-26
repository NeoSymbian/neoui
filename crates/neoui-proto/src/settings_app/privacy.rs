// SPDX-License-Identifier: Apache-2.0
//! (D8) The clipboard notice (`privacy.clip_notice`, set2-privacy): after an app's nonempty `ipc::CLIP_GET` the shell
//! shows a toast naming the app, "Browser read the clipboard". The reader is named by its process's secure ID, never
//! by what the app says it is, and the toast never holds the clipboard's text. Repeated reads by one app within
//! [`COOLDOWN_MS`] get one toast (later reads don't extend it); readers waiting behind another toast queue.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// One toast per reader in this long.
pub const COOLDOWN_MS: u64 = 3_000;
/// At most this many readers are remembered (cooling down) or waiting.
pub const READERS: usize = 8;

#[derive(Debug, Default)]
pub struct ClipNotices {
    /// Readers that had a toast: their SID and when their cooldown ends.
    recent: Vec<(u32, u64)>,
    /// Readers waiting for the toast slot, oldest first.
    pending: Vec<u32>,
}

impl ClipNotices {
    /// App `sid` read the clipboard at `now_ms`: true when it gets a toast (queued, see [`Self::take_next`]).
    pub fn read(&mut self, sid: u32, now_ms: u64) -> bool {
        self.recent.retain(|&(_, until)| until > now_ms);
        if self.recent.iter().any(|&(s, _)| s == sid) {
            return false;
        }
        if self.recent.len() == READERS {
            self.recent.remove(0);
        }
        self.recent.push((sid, now_ms + COOLDOWN_MS));
        if self.pending.len() == READERS {
            self.pending.remove(0);
        }
        self.pending.push(sid);
        true
    }

    /// The next toast's text, once the toast slot is free.
    pub fn take_next(&mut self) -> Option<String> {
        (!self.pending.is_empty()).then(|| reader_text(self.pending.remove(0)))
    }

    /// The notice was turned off: waiting toasts go.
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

/// "Files read the clipboard" for a ROM app's SID; any other process is "An app".
#[must_use]
pub fn reader_text(sid: u32) -> String {
    match crate::apps::APPS
        .iter()
        .find(|a| crate::apps::sid(a.key) == Some(sid))
    {
        Some(a) => format!("{} read the clipboard", a.name),
        None => "An app read the clipboard".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BROWSER: u32 = 0xE000_7001;
    const FILES: u32 = 0xE000_6001;

    #[test]
    fn one_toast_per_reader_per_cooldown() {
        let mut n = ClipNotices::default();
        assert!(n.read(BROWSER, 1000));
        assert!(!n.read(BROWSER, 2000)); // within 3 s, and it doesn't extend the cooldown
        assert!(n.read(FILES, 2500)); // another reader queues
        assert!(n.read(BROWSER, 4000)); // A/B/A: the first cooldown ended at 4000
        assert_eq!(n.take_next().as_deref(), Some("Browser read the clipboard"));
        assert_eq!(n.take_next().as_deref(), Some("Files read the clipboard"));
        assert_eq!(n.take_next().as_deref(), Some("Browser read the clipboard"));
        assert_eq!(n.take_next(), None);
    }

    #[test]
    fn unknown_readers_and_bounds() {
        assert_eq!(reader_text(0x1234_5678), "An app read the clipboard");
        let mut n = ClipNotices::default();
        for sid in 0..20 {
            assert!(n.read(sid, 0));
        }
        assert!(n.recent.len() <= READERS && n.pending.len() <= READERS);
        n.clear();
        assert_eq!(n.take_next(), None);
        // a reader that fell out of the table is a new reader again
        assert!(n.read(0, 1));
    }
}

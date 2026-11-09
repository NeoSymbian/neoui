// SPDX-License-Identifier: Apache-2.0
//! The clock across restarts and power-offs (NeoSymbian H3a). The board RTC keeps counting seconds while the phone is
//! off, but phones don't let the OS set it, so its zero means nothing. The UI server keeps "UTC - RTC" and the UTC
//! offset in a small file and, at boot, sets the kernel's clock to RTC + that.
//!
//! Who sets the time (the Clock app, the debug monitor's `date` that `phone.py run` sends) only sets the kernel's
//! clock. The server notices the change against what it last saw ([`Saved::changed`]) and writes the file.

/// The file, in the UI server's private directory (SYSSTART.EXE, SID 0xE0002001).
pub const DIR: &str = "C:\\private\\e0002001\\";
pub const PATH: &str = "C:\\private\\e0002001\\clock.dat";

const MAGIC: [u8; 4] = *b"NCK1";
pub const LEN: usize = 20;
/// A difference smaller than this is the two clocks drifting apart, not someone setting the time.
const SLACK_S: i64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    /// UTC (Unix seconds) minus the RTC's count.
    pub rtc_offset: i64,
    /// The UTC offset (time zone), seconds.
    pub tz: i32,
    /// The RTC's count when this was saved: an RTC now behind it was reset (its supply ran out).
    pub rtc_at_save: u32,
}

impl Saved {
    /// What the kernel's clock (`utc`, `tz`) says, against RTC count `rtc`.
    pub fn of(utc: i64, tz: i32, rtc: u32) -> Self {
        Self {
            rtc_offset: utc - i64::from(rtc),
            tz,
            rtc_at_save: rtc,
        }
    }

    pub fn encode(&self) -> [u8; LEN] {
        let mut b = [0u8; LEN];
        b[..4].copy_from_slice(&MAGIC);
        b[4..12].copy_from_slice(&self.rtc_offset.to_le_bytes());
        b[12..16].copy_from_slice(&self.tz.to_le_bytes());
        b[16..20].copy_from_slice(&self.rtc_at_save.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != LEN || b[..4] != MAGIC {
            return None;
        }
        Some(Self {
            rtc_offset: i64::from_le_bytes(b[4..12].try_into().ok()?),
            tz: i32::from_le_bytes(b[12..16].try_into().ok()?),
            rtc_at_save: u32::from_le_bytes(b[16..20].try_into().ok()?),
        })
    }

    /// The UTC to set at boot from RTC count `rtc`; `None` when the RTC went back (it was reset: the time is lost).
    // shortcut: an RTC reset that has since counted past rtc_at_save goes unnoticed; a valid-time flag needs a
    // writable RTC or a network clock.
    pub fn utc_at(&self, rtc: u32) -> Option<i64> {
        (rtc >= self.rtc_at_save).then(|| i64::from(rtc) + self.rtc_offset)
    }

    /// The new state to save when the kernel's clock (`utc`, `tz`) no longer matches this one: someone set it.
    pub fn changed(&self, utc: i64, tz: i32, rtc: u32) -> Option<Self> {
        let now = Self::of(utc, tz, rtc);
        ((now.rtc_offset - self.rtc_offset).abs() > SLACK_S || now.tz != self.tz).then_some(now)
    }
}

#[cfg(test)]
mod tests {
    use super::Saved;

    #[test]
    fn round_trip() {
        let s = Saved::of(1_794_139_200, 4 * 3600, 123_456);
        assert_eq!(Saved::decode(&s.encode()), Some(s));
        assert_eq!(Saved::decode(&s.encode()[..19]), None);
        let mut bad = s.encode();
        bad[0] = b'X';
        assert_eq!(Saved::decode(&bad), None);
    }

    #[test]
    fn boot_time_from_rtc() {
        let s = Saved::of(1_794_139_200, 0, 1_000);
        // powered off for 10 minutes
        assert_eq!(s.utc_at(1_600), Some(1_794_139_800));
        assert_eq!(s.utc_at(1_000), Some(1_794_139_200));
        // the RTC was reset
        assert_eq!(s.utc_at(5), None);
    }

    #[test]
    fn changes_only_when_set() {
        let s = Saved::of(1_794_139_200, 0, 1_000);
        // a minute later, both clocks moved together (and a second of drift): nothing to save
        assert_eq!(s.changed(1_794_139_260, 0, 1_060), None);
        assert_eq!(s.changed(1_794_139_262, 0, 1_060), None);
        // the time was set an hour forward, or the time zone changed
        assert_eq!(
            s.changed(1_794_142_860, 0, 1_060),
            Some(Saved::of(1_794_142_860, 0, 1_060))
        );
        assert!(s.changed(1_794_139_260, 3600, 1_060).is_some());
    }
}

// SPDX-License-Identifier: Apache-2.0
//! The kernel logs Dev tools saved (More → Save logs: `<drive>:\logs\kernel-<ms>.txt`, `lastlog-<ms>.txt`), as its
//! private list of them (D4, docs neoui-shell/settings-data.md): Settings → Apps → Dev tools counts and clears only
//! these, and only while a file still has the length and digest it was written with. A file of the same name it
//! didn't write (an older export, another card in the slot, an edited file) is never counted or deleted; it makes the
//! count partial.
//!
//! The list is text, one export a line: `path<TAB>length<TAB>digest` (digest: [`digest`], 16 hex digits).

use crate::settings_app::wire::DriveUsage;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Where Dev tools keeps the list (its private folder).
pub const LIST_PATH: &str = "C:\\private\\e0002005\\exports.txt";
/// Where a new list is written and flushed first; it then replaces [`LIST_PATH`] whole.
pub const LIST_STAGING: &str = "C:\\private\\e0002005\\exports.tmp";

/// One saved log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub path: String,
    pub len: u64,
    pub digest: u64,
}

/// [`digest`]'s start value (FNV-1a 64's offset basis).
pub const DIGEST_START: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64 over `bytes`, continuing from `h` ([`DIGEST_START`] for the first chunk): it tells a file Dev tools
/// wrote from another with the same name and length (not a cryptographic check: nobody gains by forging a log).
#[must_use]
pub fn digest(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The list as saved.
#[must_use]
pub fn encode(list: &[Export]) -> String {
    list.iter()
        .map(|e| format!("{}\t{}\t{:016x}\n", e.path, e.len, e.digest))
        .collect()
}

/// The saved list; `None` when a line is damaged (then nothing is cleared and the count is partial).
#[must_use]
pub fn decode(text: &str) -> Option<Vec<Export>> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut f = l.split('\t');
            let e = Export {
                path: String::from(f.next().filter(|p| is_export_path(p))?),
                len: f.next()?.parse().ok()?,
                digest: u64::from_str_radix(f.next()?, 16).ok()?,
            };
            f.next().is_none().then_some(e)
        })
        .collect()
}

/// `kernel-<digits>.txt` or `lastlog-<digits>.txt`: a name Save logs writes.
#[must_use]
pub fn is_export_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["kernel-", "lastlog-"].iter().any(|p| {
        n.strip_prefix(p)
            .and_then(|r| r.strip_suffix(".txt"))
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Where Save logs writes: `C:\logs\` or `E:\logs\` (any case) and an export's name, nothing deeper. The list
/// takes no other path, so a clear can never reach another file.
#[must_use]
pub fn is_export_path(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() > 8
        && matches!(b[0].to_ascii_uppercase(), b'C' | b'E')
        && b[1..8].eq_ignore_ascii_case(b":\\logs\\")
        && is_export_name(&path[8..])
}

/// Where a clear moves an export before it checks it again and deletes it (the same folder, so the same volume:
/// the move is one rename).
#[must_use]
pub fn aside(path: &str) -> String {
    format!("{path}.del")
}

/// The drive of an absolute path (`E:\logs\…` → 4), `None` for another kind of path.
#[must_use]
pub fn drive_of(path: &str) -> Option<u8> {
    match path.as_bytes() {
        [l, b':', b'\\', ..] if l.is_ascii_alphabetic() => Some(l.to_ascii_uppercase() - b'A'),
        _ => None,
    }
}

/// The report's drives: each drive in `present` (mounted now) with its counted log bytes; a drive that held a
/// tracked export but isn't mounted is left out (unknown).
#[must_use]
pub fn usage(counted: &[(u8, u64)], present: &[u8]) -> Vec<DriveUsage> {
    present
        .iter()
        .map(|&drive| DriveUsage {
            drive,
            logs: Some(
                counted
                    .iter()
                    .filter(|(d, _)| *d == drive)
                    .fold(0u64, |s, (_, n)| s.saturating_add(*n)),
            ),
            ..DriveUsage::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn list_round_trip_and_damage() {
        let list = vec![
            Export {
                path: "E:\\logs\\kernel-123.txt".into(),
                len: 1_153_433,
                digest: 0xdead_beef,
            },
            Export {
                path: "C:\\logs\\lastlog-9.txt".into(),
                len: 0,
                digest: DIGEST_START,
            },
        ];
        let text = encode(&list);
        assert_eq!(decode(&text), Some(list));
        assert_eq!(decode(""), Some(vec![]));
        assert_eq!(decode("E:\\logs\\kernel-1.txt\t12\n"), None, "no digest");
        assert_eq!(decode("logs\\kernel-1.txt\t1\t0\n"), None, "not absolute");
        assert_eq!(decode("C:\\data\\notes.txt\t1\t0\n"), None, "not an export");
        assert_eq!(
            decode("E:\\logs\\sub\\kernel-1.txt\t1\t0\n"),
            None,
            "deeper"
        );
        assert_eq!(decode("E:\\a\t1\tzz\n"), None, "bad digest");
        assert_eq!(decode("E:\\a\t1\t0\tx\n"), None, "a field more");
    }

    #[test]
    fn only_exports_in_the_logs_folder() {
        for p in ["C:\\logs\\kernel-1700000.txt", "e:\\LOGS\\lastlog-5.txt"] {
            assert!(is_export_path(p), "{p}");
        }
        for p in [
            "D:\\logs\\kernel-1.txt",
            "C:\\logs\\notes.txt",
            "C:\\logs\\..\\kernel-1.txt",
            "C:\\logs\\x\\kernel-1.txt",
            "C:\\logsx\\kernel-1.txt",
            "C:\\private\\e0002005\\kernel-1.txt",
            "C:\\logs\\kernel-1.txt.del",
            "C:\\logs\\",
        ] {
            assert!(!is_export_path(p), "{p}");
        }
        assert_eq!(
            aside("E:\\logs\\kernel-1.txt"),
            "E:\\logs\\kernel-1.txt.del"
        );
        assert!(
            !is_export_name("kernel-1.txt.del"),
            "aside isn't a lookalike"
        );
    }

    #[test]
    fn digest_is_fnv1a_and_streams() {
        assert_eq!(digest(DIGEST_START, b""), DIGEST_START);
        assert_eq!(digest(DIGEST_START, b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(
            digest(digest(DIGEST_START, b"neo"), b"symbian"),
            digest(DIGEST_START, b"neosymbian")
        );
    }

    #[test]
    fn names_and_drives() {
        assert!(is_export_name("kernel-1700000.txt") && is_export_name("LASTLOG-5.TXT"));
        for n in ["kernel-.txt", "kernel-1a.txt", "kernel-1.log", "notes.txt"] {
            assert!(!is_export_name(n), "{n}");
        }
        assert_eq!(drive_of("E:\\logs\\x"), Some(4));
        assert_eq!(drive_of("c:\\x"), Some(2));
        assert_eq!(drive_of("\\logs"), None);
    }

    #[test]
    fn usage_per_mounted_drive() {
        let u = usage(&[(4, 10), (4, 5), (3, 7)], &[2, 4]);
        assert_eq!(u.len(), 2);
        assert_eq!((u[0].drive, u[0].logs), (2, Some(0)));
        assert_eq!((u[1].drive, u[1].logs), (4, Some(15)));
        assert!(u.iter().all(|d| d.data.is_none() && d.installed.is_none()));
    }
}

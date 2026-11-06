// SPDX-License-Identifier: Apache-2.0
//! The Files app's model as data: Symbian paths (`X:\dir\`), folder ordering, the texts a row shows (size, date),
//! drive labels, text-file previews, name checks and error messages. Pure logic, tested on the host; the file server
//! calls live in `e32::fs` and the UI in neoui's Files app.
#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// One row of a folder listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// `TTime`: microseconds since 0000-01-01 (UTC).
    pub modified: i64,
}

/// The folder above `dir` (`X:\a\b\` → `X:\a\`); `None` at a drive root.
pub fn parent(dir: &str) -> Option<String> {
    let trimmed = dir.strip_suffix('\\')?;
    let cut = trimmed.rfind('\\')?;
    Some(String::from(&dir[..=cut]))
}

/// The folder `name` inside `dir`.
pub fn join_dir(dir: &str, name: &str) -> String {
    format!("{dir}{name}\\")
}

/// The file `name` inside `dir`.
pub fn join_file(dir: &str, name: &str) -> String {
    format!("{dir}{name}")
}

/// The last component of a folder path, for titles (`X:\a\b\` → `b`, `C:\` → `C:`).
pub fn folder_name(dir: &str) -> &str {
    let t = dir.strip_suffix('\\').unwrap_or(dir);
    t.rsplit('\\').next().unwrap_or(t)
}

/// What a row is, for its icon and whether the viewer opens it: `folder`, `text`, `image` or `file`.
pub fn kind(name: &str, is_dir: bool) -> &'static str {
    const TEXT: [&str; 12] = [
        "txt", "ini", "bat", "log", "cfg", "csv", "xml", "htm", "html", "md", "rss", "inf",
    ];
    const IMAGE: [&str; 6] = ["png", "jpg", "jpeg", "gif", "bmp", "mbm"];
    if is_dir {
        return "folder";
    }
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    let is = |list: &[&str]| list.iter().any(|x| x.eq_ignore_ascii_case(ext));
    if is(&TEXT) {
        "text"
    } else if is(&IMAGE) {
        "image"
    } else {
        "file"
    }
}

/// Folders first, then by name ignoring ASCII case.
pub fn sort(items: &mut [Item]) {
    items.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            let (x, y) = (a.name.bytes(), b.name.bytes());
            x.map(|c| c.to_ascii_lowercase())
                .cmp(y.map(|c| c.to_ascii_lowercase()))
        })
    });
}

/// `512 B`, `1.5 KB`, `12 MB`, `1.2 GB`: one decimal below 10.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64 / 1024.0;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v < 10.0 {
        // one decimal, rounded half up, without float formatting's std dependencies
        let tenths = (v * 10.0 + 0.5) as u64;
        if tenths < 100 {
            return format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[u]);
        }
    }
    format!("{} {}", (v + 0.5) as u64, UNITS[u])
}

/// Days from Symbian's `TTime` zero to 1970-01-01. Symbian counts leap years before 1600 the Julian way
/// (`euser/us_time.cpp:2486`; 2000-01-01 is day 730,497, `kernel/stimer.cpp:29`), 12 days more than proleptic
/// Gregorian; the kernel clock and ROM timestamps use it.
// shortcut: dates before 1600 come out Gregorian, not Symbian's Julian; nothing on a phone has them.
const DAYS_TO_UNIX: i64 = 719_540;

/// Year, month (1-12), day of the civil date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `2026-11-06 14:32` (UTC) for a `TTime`.
pub fn date_text(ttime: i64) -> String {
    let secs = ttime.div_euclid(1_000_000);
    let days = secs.div_euclid(86_400) - DAYS_TO_UNIX;
    let s = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", s / 3600, s / 60 % 60)
}

/// A row's subtitle: `Folder`, or `size, date` (ASCII: the apps embed only some glyphs).
pub fn detail_text(item: &Item) -> String {
    if item.is_dir {
        String::from("Folder")
    } else {
        format!("{}, {}", size_text(item.size), date_text(item.modified))
    }
}

/// What to call a drive, from its letter, `TDriveInfo::iDriveAtt` and `TMediaType`.
pub fn drive_label(letter: char, drive_att: u32, media: u32) -> &'static str {
    const ROM: u32 = 0x02;
    const REMOVABLE: u32 = 0x20;
    const MEDIA_RAM: u32 = 5;
    const MEDIA_ROM: u32 = 7;
    if drive_att & ROM != 0 || media == MEDIA_ROM {
        "System (ROM)"
    } else if drive_att & REMOVABLE != 0 {
        "Memory card"
    } else if letter == 'C' {
        "Phone memory"
    } else if media == MEDIA_RAM {
        "RAM disk"
    } else {
        "Drive"
    }
}

/// `40 MB free of 64 MB`, or `Read-only` for a full or ROM drive with nothing free.
pub fn volume_text(size: i64, free: i64) -> String {
    if free <= 0 {
        return format!("Read-only, {}", size_text(size.max(0) as u64));
    }
    format!(
        "{} free of {}",
        size_text(free as u64),
        size_text(size.max(0) as u64)
    )
}

/// The used fraction (0..1) of a drive, for its usage bar.
pub fn used_fraction(size: i64, free: i64) -> f32 {
    if size <= 0 {
        return 0.0;
    }
    (1.0 - free.clamp(0, size) as f32 / size as f32).clamp(0.0, 1.0)
}

/// Most of a file the viewer shows.
pub const PREVIEW_MAX: usize = 64 * 1024;

/// A file's bytes as text to show: UTF-16LE when it starts with a byte-order mark, else UTF-8 (bad sequences become
/// U+FFFD). Line ends become `\n`, tabs 4 spaces; past [`PREVIEW_MAX`] bytes a note says it was cut.
pub fn text_preview(bytes: &[u8]) -> String {
    let cut = bytes.len() > PREVIEW_MAX;
    let b = &bytes[..bytes.len().min(PREVIEW_MAX)];
    let raw: String = if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        char::decode_utf16(
            rest.as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes(*p)),
        )
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
    } else {
        String::from_utf8_lossy(b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b)).into()
    };
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                chars.next_if_eq(&'\n');
                out.push('\n');
            }
            '\t' => out.push_str("    "),
            c => out.push(c),
        }
    }
    if cut {
        out.push_str("\n… (first 64 KB shown)");
    }
    out
}

/// A name the file server will take for a new file or folder: not empty, not `.`/`..`, at most 256 characters, none
/// of `\ / : * ? " < > |` or control characters, and no trailing dot or space.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.encode_utf16().count() <= 256
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
}

/// A short message for a file-server error code.
pub fn error_text(code: i32) -> String {
    let s = match code {
        -1 => "Not found",
        -11 => "Already exists",
        -12 => "Path not found",
        -14 => "In use, or folder not empty",
        -21 => "Read-only drive",
        -28 => "Bad name",
        -46 => "Permission denied",
        _ => return format!("Error {code}"),
    };
    String::from(s)
}

/// The names in `items`, for tests and logs.
pub fn names(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.name.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn item(name: &str, is_dir: bool) -> Item {
        Item {
            name: name.to_string(),
            is_dir,
            size: 0,
            modified: 0,
        }
    }

    #[test]
    fn paths() {
        assert_eq!(parent("C:\\a\\b\\").as_deref(), Some("C:\\a\\"));
        assert_eq!(parent("C:\\a\\").as_deref(), Some("C:\\"));
        assert_eq!(parent("C:\\"), None);
        assert_eq!(join_dir("Z:\\sys\\", "bin"), "Z:\\sys\\bin\\");
        assert_eq!(join_file("C:\\", "a.txt"), "C:\\a.txt");
        assert_eq!(folder_name("Z:\\sys\\bin\\"), "bin");
        assert_eq!(folder_name("C:\\"), "C:");
    }

    #[test]
    fn folders_first_then_names_without_case() {
        let mut v = vec![
            item("b.txt", false),
            item("System", true),
            item("A.txt", false),
            item("private", true),
            item("sys", true),
        ];
        sort(&mut v);
        assert_eq!(names(&v), ["private", "sys", "System", "A.txt", "b.txt"]);
    }

    #[test]
    fn sizes() {
        assert_eq!(size_text(0), "0 B");
        assert_eq!(size_text(1023), "1023 B");
        assert_eq!(size_text(1024), "1.0 KB");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(10 * 1024 - 1), "10 KB");
        assert_eq!(size_text(66_959_360), "64 MB");
        assert_eq!(size_text(3 << 30), "3.0 GB");
    }

    #[test]
    fn dates() {
        // 2026-11-06 14:32:59 UTC: 20,763 days after 1970-01-01
        let t = ((DAYS_TO_UNIX + 20_763) * 86_400 + 14 * 3600 + 32 * 60 + 59) * 1_000_000;
        assert_eq!(date_text(t), "2026-11-06 14:32");
        assert_eq!(
            date_text(DAYS_TO_UNIX * 86_400 * 1_000_000),
            "1970-01-01 00:00"
        );
        // Symbian's anchor: 2000-01-01 is day 730,497 of TTime
        assert_eq!(date_text(730_497 * 86_400 * 1_000_000), "2000-01-01 00:00");
        let f = Item {
            name: "a".into(),
            is_dir: false,
            size: 5,
            modified: t,
        };
        assert_eq!(detail_text(&f), "5 B, 2026-11-06 14:32");
        assert_eq!(detail_text(&item("d", true)), "Folder");
    }

    #[test]
    fn drives() {
        assert_eq!(drive_label('Z', 0x12, 7), "System (ROM)");
        assert_eq!(drive_label('C', 0x11, 5), "Phone memory");
        assert_eq!(drive_label('E', 0x21, 6), "Memory card");
        assert_eq!(drive_label('D', 0x11, 5), "RAM disk");
        assert_eq!(volume_text(64 << 20, 40 << 20), "40 MB free of 64 MB");
        assert_eq!(volume_text(2 << 20, 0), "Read-only, 2.0 MB");
        assert_eq!(used_fraction(100, 25), 0.75);
        assert_eq!(used_fraction(0, 0), 0.0);
        assert_eq!(used_fraction(100, 200), 0.0);
    }

    #[test]
    fn previews() {
        assert_eq!(text_preview(b"a\r\nb\rc\n\td"), "a\nb\nc\n    d");
        assert_eq!(text_preview(&[0xEF, 0xBB, 0xBF, b'h', b'i']), "hi");
        assert_eq!(text_preview(&[0xFF, 0xFE, b'O', 0, b'K', 0]), "OK");
        assert_eq!(text_preview(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
        let big = vec![b'x'; PREVIEW_MAX + 10];
        let p = text_preview(&big);
        assert!(p.starts_with("xxx") && p.ends_with("(first 64 KB shown)"));
        assert_eq!(p.matches('x').count(), PREVIEW_MAX);
    }

    #[test]
    fn names_the_server_takes() {
        for ok in ["neo", "My Notes", "a.b.txt", "ünï"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "", ".", "..", "a\\b", "a:b", "x*", "q?", "trail.", "trail ", "a\u{7}",
        ] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert!(!valid_name(&"n".repeat(257)));
    }

    #[test]
    fn kinds() {
        assert_eq!(kind("sys", true), "folder");
        assert_eq!(kind("estart.TXT", false), "text");
        assert_eq!(kind("a.b.ini", false), "text");
        assert_eq!(kind("photo.JPG", false), "image");
        assert_eq!(kind("euser.dll", false), "file");
        assert_eq!(kind("README", false), "file");
    }

    #[test]
    fn errors() {
        assert_eq!(error_text(-21), "Read-only drive");
        assert_eq!(error_text(-14), "In use, or folder not empty");
        assert_eq!(error_text(-99), "Error -99");
    }
}

// SPDX-License-Identifier: Apache-2.0
//! Settings → Storage (SE3b, `set2-storage`): what the drives hold, by kind. The scan lists folders only (it never
//! opens or writes a file); the Settings app runs it on a low-priority thread through [`scan`], giving it the
//! directory listing, so the walk, its limits and its sums are tested here.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// What a file is, by its extension (the families Files' viewers use).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Images = 0,
    Video = 1,
    Audio = 2,
    Documents = 3,
    Other = 4,
}

pub const KINDS: [Kind; 5] = [
    Kind::Images,
    Kind::Video,
    Kind::Audio,
    Kind::Documents,
    Kind::Other,
];

impl Kind {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Kind::Images => "Images",
            Kind::Video => "Video",
            Kind::Audio => "Audio",
            Kind::Documents => "Documents",
            Kind::Other => "Other files",
        }
    }
}

const IMAGES: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "svg", "tif", "tiff",
];
const VIDEO: &[&str] = &[
    "mp4", "m4v", "3gp", "3g2", "mkv", "avi", "mov", "webm", "wmv", "mpg", "mpeg",
];
const AUDIO: &[&str] = &[
    "mp3", "aac", "m4a", "wav", "ogg", "oga", "opus", "flac", "amr", "wma", "mid", "midi",
];
const DOCUMENTS: &[&str] = &[
    "pdf", "txt", "md", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf",
    "html", "htm", "csv", "tsv", "epub",
];

/// The kind of a file named `name` (any case).
#[must_use]
pub fn kind_of(name: &str) -> Kind {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return Kind::Other;
    };
    let ext = ext.to_ascii_lowercase();
    let is = |list: &[&str]| list.contains(&ext.as_str());
    if is(IMAGES) {
        Kind::Images
    } else if is(VIDEO) {
        Kind::Video
    } else if is(AUDIO) {
        Kind::Audio
    } else if is(DOCUMENTS) {
        Kind::Documents
    } else {
        Kind::Other
    }
}

/// The folders at a drive's root the files count skips: apps' private data, the system and installed binaries
/// (`\sys` is counted as Apps through `\sys\bin`, separately).
pub const SKIPPED: &[&str] = &["private", "sys", "resource", "system"];

/// `KErrNotFound`, `KErrPathNotFound`: a folder that isn't there.
const NOT_FOUND: i32 = -1;
const PATH_NOT_FOUND: i32 = -12;

/// The deepest folder the scan enters (below a drive's root).
pub const DEPTH_MAX: usize = 64;
/// The most folders waiting to be listed at once (a very wide tree): more are counted in [`Tally::skipped`], so the
/// walk's memory stays bounded.
pub const PENDING_MAX: usize = 4096;

/// The running count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Bytes by [`Kind`].
    pub files: [u64; 5],
    /// Bytes of installed binaries (`\sys\bin` on each drive).
    pub apps: u64,
    /// Folders listed.
    pub folders: u32,
    /// Folders that couldn't be listed (access denied, the medium removed): skipped, counted.
    pub unreadable: u32,
    /// Of those, drive roots: nothing on that drive was counted.
    pub roots_unread: u32,
    /// Folders not entered: deeper than [`DEPTH_MAX`], or past [`PENDING_MAX`] waiting.
    pub skipped: u32,
    /// The whole walk ran (not cancelled).
    pub done: bool,
}

impl Tally {
    #[must_use]
    pub fn files_total(&self) -> u64 {
        self.files.iter().sum()
    }

    /// `self` and `b` together (C: and E:'s counts for the overview); whole only when both are.
    #[must_use]
    pub fn plus(&self, b: &Tally) -> Tally {
        let mut t = *self;
        for (f, g) in t.files.iter_mut().zip(b.files) {
            *f += g;
        }
        t.apps += b.apps;
        t.folders += b.folders;
        t.unreadable += b.unreadable;
        t.roots_unread += b.roots_unread;
        t.skipped += b.skipped;
        t.done = self.done && b.done;
        t
    }
}

/// One listed entry: its name, whether it is a folder, its size.
pub type Listed = (String, bool, u64);

/// Walk `roots` (drive roots such as `"C:\\"`) with `list` (one folder's entries, or an error), summing files by kind
/// outside [`SKIPPED`] and the binaries in each root's `sys\bin`. `cancelled` is asked before each folder; `progress`
/// gets the count after each one. A folder that fails to list is counted in [`Tally::unreadable`] and skipped.
pub fn scan(
    roots: &[&str],
    mut list: impl FnMut(&str) -> Result<Vec<Listed>, i32>,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(&Tally),
) -> Tally {
    let mut t = Tally::default();
    for root in roots {
        // installed binaries: one folder, not walked further
        if cancelled() {
            return t;
        }
        match list(&format!("{root}sys\\bin\\")) {
            Ok(entries) => {
                t.folders += 1;
                t.apps += entries.iter().filter(|e| !e.1).map(|e| e.2).sum::<u64>();
            }
            // no \sys\bin on this drive (E: often has none): no apps there, nothing unread
            Err(NOT_FOUND | PATH_NOT_FOUND) => {}
            Err(_) => t.unreadable += 1,
        }
        let mut stack: Vec<(String, usize)> = alloc::vec![(String::from(*root), 0)];
        while let Some((dir, depth)) = stack.pop() {
            if cancelled() {
                return t;
            }
            let Ok(entries) = list(&dir) else {
                t.unreadable += 1;
                if depth == 0 {
                    t.roots_unread += 1;
                }
                progress(&t);
                continue;
            };
            t.folders += 1;
            for (name, is_dir, size) in entries {
                if is_dir {
                    if depth == 0 && SKIPPED.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
                        continue;
                    }
                    if depth + 1 > DEPTH_MAX || stack.len() >= PENDING_MAX {
                        t.skipped += 1;
                        continue;
                    }
                    stack.push((format!("{dir}{name}\\"), depth + 1));
                } else {
                    t.files[kind_of(&name) as usize] += size;
                }
            }
            progress(&t);
        }
    }
    t.done = true;
    t
}

/// How one drive's count ended (D3), so a count never puts files it couldn't see under Other as if it had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// every folder read, the same mount throughout
    Complete,
    /// done, but some folders couldn't be read: shown marked partial only while the drive has no complete count
    Partial,
    /// stopped, remounted, gone or its root unreadable: nothing of it is kept
    Dropped,
}

impl Tally {
    /// This count's [`Outcome`]; `remounted`: the drive's mount changed while it was counted.
    #[must_use]
    pub fn outcome(&self, remounted: bool) -> Outcome {
        if !self.done || remounted || self.roots_unread > 0 {
            Outcome::Dropped
        } else if self.unreadable > 0 {
            Outcome::Partial
        } else {
            Outcome::Complete
        }
    }
}

/// Whether a drive's new count with outcome `o` replaces the one kept for it (`kept`: `None`, or whether that one
/// is partial): a complete count always, a partial one only over nothing or another partial one, a dropped one never.
#[must_use]
pub fn replaces(kept: Option<bool>, o: Outcome) -> bool {
    match o {
        Outcome::Complete => true,
        Outcome::Partial => kept.is_none_or(|partial| partial),
        Outcome::Dropped => false,
    }
}

/// "Other (system and app data)": what the drives use besides the counted files and apps. Never below zero: the
/// skipped folders, unreadable ones and the file system's cluster slack aren't counted, so the parts can disagree.
#[must_use]
pub fn other(used: u64, t: &Tally) -> u64 {
    used.saturating_sub(t.files_total().saturating_add(t.apps))
}

/// One drive's bar (D3, the C: row of `set2-storage`): Apps, Files and Other bytes that add up to `used` exactly, so
/// the parts end where the drive's used fill does: counted bytes past `used` (files deleted since the count) are cut,
/// and Other is the rest.
#[must_use]
pub fn drive_parts(used: u64, t: &Tally) -> [u64; 3] {
    let apps = t.apps.min(used);
    let files = t.files_total().min(used - apps);
    [apps, files, used - apps - files]
}

/// What a drive holds, with its mounted file system (D3): `fs` is `RFs::FileSystemSubType` (`FAT32`, or the name of
/// a file system without sub types: `Rom`, `Rofs`, `Composite`), empty when unknown. The read-only Z: says what makes
/// it up instead.
#[must_use]
pub fn drive_note(letter: char, fs: &str) -> String {
    let (_, note) = drive_names(letter);
    if letter.eq_ignore_ascii_case(&'Z') {
        return String::from(match fs {
            "Composite" => "ROFS + core ROM · read-only",
            "Rom" => "Core ROM · read-only",
            "Rofs" => "ROFS · read-only",
            _ => note,
        });
    }
    match (note, fs) {
        (n, "") => String::from(n),
        ("", f) => String::from(f),
        (n, f) => format!("{n} · {f}"),
    }
}

/// A drive as the page names it: (name, what it holds), by its letter.
#[must_use]
pub fn drive_names(letter: char) -> (&'static str, &'static str) {
    match letter.to_ascii_uppercase() {
        'C' => ("Phone", "Settings, installed apps, app data"),
        'D' => ("RAM", "Temporary files"),
        'E' => ("Mass memory", "Media and documents"),
        'Z' => ("System", "Read-only"),
        _ => ("Drive", ""),
    }
}

/// `512 B`, `1.5 KB`, `12 MB`, `41.8 GB`: binary units with Files' labels and rounding (files-core `size_text`;
/// keep the two the same, so a drive reads alike in both apps).
#[must_use]
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
    let decimals_below = if u >= 2 { 100.0 } else { 10.0 };
    if v < decimals_below {
        let tenths = (v * 10.0 + 0.5) as u64;
        if u >= 2 && tenths.is_multiple_of(10) {
            return format!("{} {}", tenths / 10, UNITS[u]);
        }
        if (tenths as f64) < decimals_below * 10.0 {
            return format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[u]);
        }
    }
    format!("{} {}", (v + 0.5) as u64, UNITS[u])
}

/// "12.4 of 41.8 GB" (the board's form: one unit when both share it), else "512 MB of 8 GB".
#[must_use]
pub fn used_text(used: u64, total: u64) -> String {
    let (u, t) = (size_text(used), size_text(total));
    match (u.rsplit_once(' '), t.rsplit_once(' ')) {
        (Some((n, a)), Some((_, b))) if a == b => format!("{n} of {t}"),
        _ => format!("{u} of {t}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    use alloc::vec;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of("IMG_0001.JPG"), Kind::Images);
        assert_eq!(kind_of("clip.3gp"), Kind::Video);
        assert_eq!(kind_of("song.Mp3"), Kind::Audio);
        assert_eq!(kind_of("notes.md"), Kind::Documents);
        assert_eq!(kind_of("game.sis"), Kind::Other);
        assert_eq!(kind_of("README"), Kind::Other);
        assert_eq!(kind_of(".hidden"), Kind::Other);
    }

    #[test]
    fn sizes_read_like_files() {
        // files-core size_text's own cases: the two apps must agree
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(12 * 1024 * 1024), "12 MB");
        assert_eq!(size_text(8 * 1024 * 1024 * 1024), "8 GB");
        assert_eq!(size_text(44_884_574_208), "41.8 GB");
        assert_eq!(size_text(150 * 1024 * 1024 * 1024), "150 GB");
        assert_eq!(used_text(13_314_398_618, 44_884_574_208), "12.4 of 41.8 GB");
        assert_eq!(
            used_text(512 * 1024 * 1024, 8 * 1024 * 1024 * 1024),
            "512 MB of 8 GB"
        );
    }

    #[test]
    fn other_is_never_negative() {
        let t = Tally {
            files: [100, 0, 0, 0, 50],
            apps: 25,
            ..Tally::default()
        };
        assert_eq!(other(1000, &t), 825);
        assert_eq!(other(100, &t), 0); // more counted than used: clamped
        assert_eq!(other(0, &Tally::default()), 0);
    }

    /// A fake drive: folder path → entries; a missing path fails to list.
    fn fs() -> BTreeMap<&'static str, Vec<Listed>> {
        let e = |n: &str, d: bool, s: u64| (String::from(n), d, s);
        BTreeMap::from([
            (
                "C:\\",
                vec![
                    e("Data", true, 0),
                    e("private", true, 0),
                    e("SYS", true, 0),
                    e("x.txt", false, 5),
                ],
            ),
            (
                "C:\\Data\\",
                vec![
                    e("a.jpg", false, 100),
                    e("Music", true, 0),
                    e("Gone", true, 0),
                ],
            ),
            ("C:\\Data\\Music\\", vec![e("b.mp3", false, 200)]),
            (
                "C:\\sys\\bin\\",
                vec![e("APP.EXE", false, 1000), e("LIB.DLL", false, 24)],
            ),
        ])
    }

    #[test]
    fn the_walk_sums_skips_and_counts_what_it_couldnt_read() {
        let f = fs();
        let mut seen = 0;
        let t = scan(
            &["C:\\", "E:\\"],
            |p| f.get(p).cloned().ok_or(-12),
            || false,
            |_| seen += 1,
        );
        assert!(t.done);
        assert_eq!(t.files[Kind::Images as usize], 100);
        assert_eq!(t.files[Kind::Audio as usize], 200);
        assert_eq!(t.files[Kind::Documents as usize], 5);
        assert_eq!(t.apps, 1024);
        // C:\Data\Gone and E:\ itself couldn't be listed (E:\sys\bin not being there is no apps, not unread);
        // private and SYS were skipped, not listed
        assert_eq!(t.unreadable, 2);
        assert_eq!(t.folders, 4);
        assert!(seen >= 4);
    }

    #[test]
    fn cancelling_stops_and_depth_is_capped() {
        let f = fs();
        let t = scan(&["C:\\"], |p| f.get(p).cloned().ok_or(-1), || true, |_| {});
        assert!(!t.done);
        assert_eq!(t.folders, 0);
        // a folder chain deeper than the cap stops there, counted
        let t = scan(
            &["C:\\"],
            |p| {
                Ok(if p.ends_with("sys\\bin\\") {
                    vec![]
                } else {
                    vec![(String::from("d"), true, 0)]
                })
            },
            || false,
            |_| {},
        );
        assert!(t.done);
        assert_eq!(t.skipped, 1);
        assert_eq!(t.folders as usize, 1 + DEPTH_MAX + 1); // sys\bin, the root and 64 levels
    }

    #[test]
    fn a_wide_tree_stays_bounded() {
        // past PENDING_MAX waiting folders, the rest are skipped, counted
        let wide: Vec<Listed> = (0..PENDING_MAX + 10)
            .map(|i| (alloc::format!("d{i}"), true, 0))
            .collect();
        let t = scan(
            &["C:\\"],
            |p| Ok(if p == "C:\\" { wide.clone() } else { vec![] }),
            || false,
            |_| {},
        );
        assert!(t.done);
        assert_eq!(t.skipped, 10);
        assert_eq!(t.folders as usize, 2 + PENDING_MAX);
        assert_eq!(kind_of("budget.TSV"), Kind::Documents);
    }

    #[test]
    fn a_drive_bar_ends_at_its_used_bytes() {
        let t = Tally {
            files: [100, 0, 50, 0, 0],
            apps: 300,
            ..Tally::default()
        };
        assert_eq!(drive_parts(1000, &t), [300, 150, 550]);
        // files deleted since the count: Files is cut, nothing is negative
        assert_eq!(drive_parts(400, &t), [300, 100, 0]);
        assert_eq!(drive_parts(200, &t), [200, 0, 0]);
        // not counted yet: all of it is Other
        assert_eq!(drive_parts(7, &Tally::default()), [0, 0, 7]);
        // two drives' counts add up, whole only when both are
        let c = Tally { done: true, ..t };
        let both = c.plus(&Tally {
            apps: 1,
            folders: 2,
            ..Tally::default()
        });
        assert_eq!(
            (both.apps, both.folders, both.files_total(), both.done),
            (301, 2, 150, false)
        );
        assert!(c.plus(&c).done);
    }

    #[test]
    fn a_drive_keeps_its_last_complete_count() {
        let f = fs();
        // an unreadable root (E:\ here) is dropped, not a drive of Other
        let e = scan(
            &["E:\\"],
            |p| f.get(p).cloned().ok_or(-12),
            || false,
            |_| {},
        );
        assert_eq!((e.roots_unread, e.outcome(false)), (1, Outcome::Dropped));
        // C:\Data\Gone couldn't be read: partial
        let c = scan(
            &["C:\\"],
            |p| f.get(p).cloned().ok_or(-12),
            || false,
            |_| {},
        );
        assert_eq!((c.roots_unread, c.outcome(false)), (0, Outcome::Partial));
        assert_eq!(c.outcome(true), Outcome::Dropped); // remounted while counted
        let whole = Tally {
            done: true,
            ..Tally::default()
        };
        assert_eq!(whole.outcome(false), Outcome::Complete);
        assert_eq!(Tally::default().outcome(false), Outcome::Dropped); // stopped
        // what replaces the kept count
        assert!(replaces(Some(false), Outcome::Complete));
        assert!(!replaces(Some(false), Outcome::Partial));
        assert!(replaces(Some(true), Outcome::Partial));
        assert!(replaces(None, Outcome::Partial));
        assert!(!replaces(None, Outcome::Dropped));
        assert!(!replaces(Some(true), Outcome::Dropped));
    }

    #[test]
    fn notes_name_the_file_system() {
        assert_eq!(drive_note('E', "FAT32"), "Media and documents · FAT32");
        assert_eq!(drive_note('C', ""), "Settings, installed apps, app data");
        assert_eq!(drive_note('Q', "FAT16"), "FAT16");
        assert_eq!(drive_note('Z', "Composite"), "ROFS + core ROM · read-only");
        assert_eq!(drive_note('z', "Rom"), "Core ROM · read-only");
        assert_eq!(drive_note('Z', ""), "Read-only");
    }

    #[test]
    fn drives_are_named() {
        assert_eq!(drive_names('e'), ("Mass memory", "Media and documents"));
        assert_eq!(drive_names('Z').1, "Read-only");
        assert_eq!(drive_names('Q'), ("Drive", ""));
    }
}

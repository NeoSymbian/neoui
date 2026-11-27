// SPDX-License-Identifier: Apache-2.0
//! D4: what Settings says about an app from its INFO 8 record ([`AppDetail`], docs neoui-shell/settings-data.md D4):
//! the Apps list's subtitle (set2-apps "ROM · 2.4 MB data on C:"), the app page's identity line (set2-app-browser
//! "NEOWEB.EXE · Servo 0.8.0") and its "Storage used" row. Only what the record says: an unknown count is never shown
//! as zero, a partial one reads "at least", and nothing is inferred from the board.

use super::wire::{AppDetail, DriveUsage, coverage};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// `512 B`, `16 KB`, `2.4 MB`, `48.2 MB`, `2 MB`, `1.5 GB`: binary units, whole KB, one decimal from MB up unless it
/// is a whole number (the boards' sizes; Storage's `storage::size_text` rounds MB to whole ones).
#[must_use]
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kb = (bytes + 512) / 1024;
    if kb < 1024 {
        return format!("{kb} KB");
    }
    let mut v = bytes as f64 / (1024.0 * 1024.0);
    let mut u = 0;
    while v >= 1023.95 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    let tenths = (v * 10.0 + 0.5) as u64;
    if tenths.is_multiple_of(10) {
        format!("{} {}", tenths / 10, UNITS[u])
    } else {
        format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[u])
    }
}

fn letter(drive: u8) -> char {
    char::from(b'A' + drive.min(25))
}

/// Where the app is installed: "ROM" on Z:, else its drive ("C:"); "ROM" when the shell didn't find it (the catalogue's
/// apps are ROM apps).
#[must_use]
pub fn place(d: Option<&AppDetail>) -> String {
    match d.and_then(|d| d.drives.iter().find(|u| u.installed.is_some())) {
        Some(u) if u.drive != 25 => format!("{}:", letter(u.drive)),
        _ => String::from("ROM"),
    }
}

/// The mutable bytes on one drive (data, cache, logs; installed files apart).
fn mutable(u: &DriveUsage) -> u64 {
    [u.data, u.cache, u.logs]
        .iter()
        .map(|c| c.unwrap_or(0))
        .fold(0, u64::saturating_add)
}

fn total(d: &AppDetail) -> u64 {
    d.drives.iter().map(mutable).fold(0, u64::saturating_add)
}

/// The Apps list's subtitle after "ROM · ": "2.4 MB data on C:", "1.1 MB logs on E:", "no data", "at least …" for a
/// partial count, "counting…", "not measured yet", "size unavailable" (only for a measurement that couldn't run);
/// Messages keeps "SMS kept by the modem server" while its texts aren't counted. A measurement that ran and found
/// nothing says "no data", partial or not.
#[must_use]
pub fn list_subtitle(key: &str, d: Option<&AppDetail>) -> String {
    let place = place(d);
    let what = match d.map(|d| (d.coverage, d)) {
        Some((coverage::MEASURING, _)) => String::from("counting…"),
        _ if key == "messages" && d.is_none_or(|d| d.coverage != coverage::COMPLETE) => {
            String::from("SMS kept by the modem server")
        }
        None | Some((coverage::UNKNOWN, _)) => String::from("not measured yet"),
        Some((coverage::FAILED, _)) => String::from("size unavailable"),
        Some((c, d)) => {
            let sum = total(d);
            if sum == 0 {
                String::from("no data")
            } else {
                let kind = if d
                    .drives
                    .iter()
                    .all(|u| u.data.unwrap_or(0) == 0 && u.cache.unwrap_or(0) == 0)
                {
                    "logs"
                } else {
                    "data"
                };
                let on: Vec<String> = d
                    .drives
                    .iter()
                    .filter(|u| mutable(u) > 0)
                    .map(|u| format!("{}:", letter(u.drive)))
                    .collect();
                format!(
                    "{}{} {kind} on {}",
                    if c == coverage::PARTIAL {
                        "at least "
                    } else {
                        ""
                    },
                    size_text(sum),
                    on.join(" and ")
                )
            }
        }
    };
    format!("{place} · {what}")
}

/// The app page's identity line: its EXE, and the version its owner reported ("NEOWEB.EXE · Servo 0.8.0"); `""` when
/// the shell named no EXE.
#[must_use]
pub fn identity(d: &AppDetail) -> String {
    match (d.exe.is_empty(), d.version.is_empty()) {
        (true, _) => String::new(),
        (false, true) => d.exe.clone(),
        (false, false) => format!("{} · {}", d.exe, d.version),
    }
}

/// The app page's "Storage used" row: its subtitle ("Data 12.6 MB · cache 35.6 MB", a part on another drive than C:
/// says which) and its value ("48.2 MB", "At least 2.4 MB").
#[must_use]
pub fn storage_used(d: &AppDetail) -> (String, String) {
    let plain = |s: &str| (String::from(s), String::new());
    match d.coverage {
        coverage::MEASURING => return plain("Counting…"),
        coverage::UNKNOWN => return plain("Not measured yet"),
        coverage::FAILED => return plain("Size unavailable"),
        _ => {}
    }
    let sum = total(d);
    let partial = d.coverage == coverage::PARTIAL;
    if sum == 0 {
        // it ran and found nothing; partial: what it could look at holds nothing (no "0 B" for what it couldn't)
        return if partial {
            plain("No data found")
        } else {
            (String::from("No data"), size_text(0))
        };
    }
    let mut parts: Vec<String> = Vec::new();
    for (name, get) in [
        (
            "data",
            (|u: &DriveUsage| u.data) as fn(&DriveUsage) -> Option<u64>,
        ),
        ("cache", |u| u.cache),
        ("logs", |u| u.logs),
    ] {
        for u in &d.drives {
            let n = get(u).unwrap_or(0);
            if n == 0 {
                continue;
            }
            let on = if u.drive == 2 {
                String::new()
            } else {
                format!(" on {}:", letter(u.drive))
            };
            parts.push(format!("{name} {}{on}", size_text(n)));
        }
    }
    let mut sub = parts.join(" · ");
    if let Some(first) = sub.get(..1) {
        sub = first.to_uppercase() + &sub[1..];
    }
    let value = if partial {
        format!("At least {}", size_text(sum))
    } else {
        size_text(sum)
    };
    (sub, value)
}

/// `drives` with the shell's own reading of where the EXE is (`installed`: drive and bytes) merged in: one tuple per
/// drive, the owner's counts kept, the installed count always the shell's.
#[must_use]
pub fn with_installed(drives: &[DriveUsage], installed: Option<(u8, u64)>) -> Vec<DriveUsage> {
    let mut out: Vec<DriveUsage> = drives
        .iter()
        .map(|u| DriveUsage {
            installed: None,
            ..*u
        })
        .collect();
    if let Some((drive, bytes)) = installed {
        match out.iter_mut().find(|u| u.drive == drive) {
            Some(u) => u.installed = Some(bytes),
            None => out.push(DriveUsage {
                drive,
                installed: Some(bytes),
                ..DriveUsage::default()
            }),
        }
    }
    out.sort_by_key(|u| u.drive);
    out
}

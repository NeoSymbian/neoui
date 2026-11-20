// SPDX-License-Identifier: Apache-2.0
//! Which apps open which files (Open with, docs files-app.md FA8), the pages of Files that other apps open (its file
//! picker), and the user's "Always" choices (setting `open.always`).
//!
//! shortcut: a fixed table, like the server's `APPS`; the store's app registry (docs store.md ST) replaces it with
//! what installed apps declare.

use alloc::string::String;
use alloc::vec::Vec;

/// An app that opens files: [`crate::ipc::OPEN_FILE`] on `key`'s page `page`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handler {
    pub key: &'static str,
    pub page: u16,
    /// What the Open with sheet calls it.
    pub label: &'static str,
    /// Lower-case extensions without the dot; `"*"` takes any file.
    pub exts: &'static [&'static str],
}

/// Files, as other apps see it.
pub mod files {
    pub const KEY: &str = "files";
    /// Pick a file: the arg filters what is shown (`""` everything, a family `image`, or extensions `txt,md`); Files
    /// answers with the file opened for reading ([`crate::ipc::DONE_FILE`]), or [`crate::ipc::DONE`] when cancelled.
    pub const PICK: u16 = 1;
    /// Save a file: the arg is the suggested name; Files answers with the new file, empty and open for writing.
    pub const SAVE: u16 = 2;
}

/// The kit (`NEOKIT.EXE`): the gallery and the new-app template, also a demo of opening files.
pub mod kit {
    pub const KEY: &str = "kit";
    /// Show a file opened with [`crate::ipc::OPEN_FILE`]: its name, size and first line.
    pub const VIEW: u16 = 1;
}

/// The apps that open files besides the opener itself (Files' own viewers come first in its sheet).
pub const HANDLERS: &[Handler] = &[
    Handler {
        key: "contacts",
        page: 3, // phone-core's contacts::IMPORT
        label: "Contacts",
        exts: &["vcf"],
    },
    Handler {
        key: kit::KEY,
        page: kit::VIEW,
        label: "Kit file viewer",
        exts: &["*"],
    },
];

/// The handlers for files with extension `ext` (any case, no dot), in table order.
pub fn handlers_for(ext: &str) -> impl Iterator<Item = &'static Handler> + '_ {
    HANDLERS.iter().filter(move |h| {
        h.exts
            .iter()
            .any(|e| *e == "*" || e.eq_ignore_ascii_case(ext))
    })
}

/// The app key `open.always` names for `ext` (any case), if any. The value is `ext=key;` entries, lower-case.
pub fn always<'a>(table: &'a [u8], ext: &str) -> Option<&'a str> {
    let text = core::str::from_utf8(table).ok()?;
    text.split(';').find_map(|entry| {
        let (e, key) = entry.split_once('=')?;
        (e.eq_ignore_ascii_case(ext) && !key.is_empty()).then_some(key)
    })
}

/// `table` with `ext` set to `key`, or removed for `None`. Other entries keep their order.
pub fn set_always(table: &[u8], ext: &str, key: Option<&str>) -> Vec<u8> {
    let text = core::str::from_utf8(table).unwrap_or("");
    let mut out = String::new();
    for entry in text.split(';').filter(|e| !e.is_empty()) {
        match entry.split_once('=') {
            Some((e, _)) if e.eq_ignore_ascii_case(ext) => {}
            Some(_) => {
                out.push_str(entry);
                out.push(';');
            }
            None => {}
        }
    }
    if let Some(key) = key {
        out.push_str(&ext.to_ascii_lowercase());
        out.push('=');
        out.push_str(key);
        out.push(';');
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handlers_by_extension() {
        let vcf: Vec<_> = handlers_for("VCF").map(|h| h.key).collect();
        assert_eq!(vcf, ["contacts", "kit"]);
        let txt: Vec<_> = handlers_for("txt").map(|h| h.key).collect();
        assert_eq!(txt, ["kit"]);
    }

    #[test]
    fn always_round_trip() {
        let t = set_always(b"", "TXT", Some("kit"));
        assert_eq!(t, b"txt=kit;");
        let t = set_always(&t, "vcf", Some("contacts"));
        assert_eq!(always(&t, "Txt"), Some("kit"));
        assert_eq!(always(&t, "vcf"), Some("contacts"));
        assert_eq!(always(&t, "jpg"), None);
        let t = set_always(&t, "txt", None);
        assert_eq!(t, b"vcf=contacts;");
        let t = set_always(&t, "vcf", Some("kit"));
        assert_eq!(t, b"vcf=kit;");
        assert_eq!(always(b"garbage", "x"), None);
    }

    #[test]
    fn open_always_is_writable_by_any_app() {
        use crate::settings::access::Caller;
        assert!(Caller::App("files").may_write("open.always"));
        assert!(Caller::App("kit").may_write("open.always"));
    }
}

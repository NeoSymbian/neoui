// SPDX-License-Identifier: GPL-3.0-only
//! The Settings app's screens (`NEOSET.EXE`, docs neoui-shell/settings.md "The Settings app"): `SettingsApp` and
//! its `SetState` global. The device crate (`symbian/apps/settings`) fills `SetState` and answers its `act`
//! callback; the tests here draw every page into `snapshots/set2-*.png`.
#![no_std]

slint::include_modules!();

extern crate alloc;

pub mod row_model;

use alloc::string::String;

/// A search result's title split around the first case-insensitive match of the query, for the highlight
/// (set2-search). The row draws the pieces as separate texts, which drop spaces at their edges, so the pieces come
/// trimmed and the gaps as flags. No match (an alias or the page matched) leaves it all in `pre`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub pre: String,
    pub mark: String,
    pub post: String,
    /// a space between `pre` and `mark`
    pub gap_before: bool,
    /// a space between `mark` and `post`
    pub gap_after: bool,
}

#[must_use]
pub fn split_match(title: &str, query: &str) -> Split {
    let q: alloc::vec::Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    if !q.is_empty() {
        for (start, _) in title.char_indices() {
            let mut got = 0;
            for (off, c) in title[start..].char_indices() {
                for l in c.to_lowercase() {
                    if got < q.len() && l == q[got] {
                        got += 1;
                    } else {
                        got = usize::MAX;
                        break;
                    }
                }
                if got == usize::MAX {
                    break;
                }
                if got == q.len() {
                    let end = start + off + c.len_utf8();
                    let (pre, mark, post) = (&title[..start], &title[start..end], &title[end..]);
                    return Split {
                        pre: pre.trim_end().into(),
                        mark: mark.trim().into(),
                        post: post.trim_start().into(),
                        // whatever whitespace the trims take is drawn as one gap
                        gap_before: pre.trim_end() != pre || mark.trim_start() != mark,
                        gap_after: post.trim_start() != post || mark.trim_end() != mark,
                    };
                }
            }
        }
    }
    Split {
        pre: title.into(),
        ..Split::default()
    }
}

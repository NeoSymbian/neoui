// SPDX-License-Identifier: Apache-2.0
//! The kernel log as lines: bytes from the HAL log reads go in, a bounded ring of lines comes out. Each line's tag
//! (`[touch] …`, `[modemd] …`, `neoui: …`) drives the Console's filters and a device's own log.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

/// Lines kept for the screen (Save writes the kernel's whole log, not these).
pub const KEEP_LINES: usize = 2000;
/// Longest tag recognised; longer brackets are text, not a tag.
const TAG_MAX: usize = 16;

/// A ring of complete lines plus the unfinished last one.
#[derive(Clone, Debug, Default)]
pub struct Lines {
    lines: VecDeque<String>,
    partial: String,
    /// Lines pushed since the start (or [`Lines::clear`]), dropped ones included.
    total: u64,
}

impl Lines {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.partial.clear();
        self.total = 0;
    }

    /// Appends log bytes: CR dropped, non-ASCII shown as `?`, a partial last line kept for the next chunk.
    pub fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match b {
                b'\n' => {
                    let line = core::mem::take(&mut self.partial);
                    if self.lines.len() == KEEP_LINES {
                        self.lines.pop_front();
                    }
                    self.lines.push_back(line);
                    self.total += 1;
                }
                b'\r' => {}
                b'\t' => self.partial.push(' '),
                b if b.is_ascii() && !b.is_ascii_control() => self.partial.push(b as char),
                _ => self.partial.push('?'),
            }
        }
    }

    /// Lines pushed so far, dropped ones included: a change means there is something new to show.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total + u64::from(!self.partial.is_empty())
    }

    /// Every kept line, oldest first, the partial one last.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.lines
            .iter()
            .map(String::as_str)
            .chain((!self.partial.is_empty()).then_some(self.partial.as_str()))
    }

    /// The kept lines whose tag is in `tags` (all of them when `tags` is empty), oldest first.
    pub fn filtered<'a>(&'a self, tags: &'a [&'a str]) -> impl Iterator<Item = &'a str> + 'a {
        self.iter()
            .filter(move |l| tags.is_empty() || tag_of(l).is_some_and(|t| tags.contains(&t)))
    }

    /// The tags in the kept lines, most frequent first (ties by name), with their counts.
    #[must_use]
    pub fn tags(&self) -> Vec<(&str, usize)> {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for tag in self.iter().filter_map(tag_of) {
            match counts.iter_mut().find(|(t, _)| *t == tag) {
                Some((_, n)) => *n += 1,
                None => counts.push((tag, 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        counts
    }

    /// True when some kept line has tag `tag`.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.iter().any(|l| tag_of(l) == Some(tag))
    }
}

/// A line's tag: `[touch] …` → `touch`, `neoui: …` → `neoui`. Leading spaces are skipped; a tag is lowercase ASCII
/// letters, digits, `_` or `-`.
#[must_use]
pub fn tag_of(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let word = |s: &str| {
        !s.is_empty()
            && s.len() <= TAG_MAX
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    };
    if let Some(rest) = line.strip_prefix('[') {
        let tag = &rest[..rest.find(']')?];
        return word(tag).then_some(tag);
    }
    let tag = &line[..line.find(": ")?];
    word(tag).then_some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_from_brackets_and_prefixes() {
        assert_eq!(tag_of("[touch] 3 down 10 20"), Some("touch"));
        assert_eq!(tag_of("  [modemd] adsp: up"), Some("modemd"));
        assert_eq!(tag_of("neoui: clock: set"), Some("neoui"));
        assert_eq!(tag_of("devtools: tab 1"), Some("devtools"));
        assert_eq!(tag_of("[Touch] caps"), None);
        assert_eq!(tag_of("[a very long bracketed thing] x"), None);
        assert_eq!(tag_of("Boot: done"), None);
        assert_eq!(tag_of("plain text"), None);
        assert_eq!(tag_of("[unclosed"), None);
    }

    #[test]
    fn chunks_become_lines_and_the_ring_is_bounded() {
        let mut l = Lines::new();
        l.push(b"[a] one\r\n[b] tw");
        assert_eq!(l.iter().collect::<Vec<_>>(), ["[a] one", "[b] tw"]);
        assert_eq!(l.total(), 2);
        l.push(b"o\n\x07\xff\tx\n");
        assert_eq!(l.iter().collect::<Vec<_>>(), ["[a] one", "[b] two", "?? x"]);
        for _ in 0..KEEP_LINES {
            l.push(b"[c] more\n");
        }
        assert_eq!(l.iter().count(), KEEP_LINES);
        assert_eq!(l.total(), (KEEP_LINES + 3) as u64);
        assert!(!l.has_tag("a"), "the oldest lines are gone");
        l.clear();
        assert_eq!((l.iter().count(), l.total()), (0, 0));
    }

    #[test]
    fn filters_and_tag_counts() {
        let mut l = Lines::new();
        l.push(b"[touch] a\nneoui: b\n[touch] c\nplain\n[adsp] d\n");
        assert_eq!(
            l.filtered(&["touch"]).collect::<Vec<_>>(),
            ["[touch] a", "[touch] c"]
        );
        assert_eq!(l.filtered(&[]).count(), 5);
        assert_eq!(l.filtered(&["adsp", "neoui"]).count(), 2);
        assert_eq!(l.tags(), [("touch", 2), ("adsp", 1), ("neoui", 1)]);
        assert!(l.has_tag("adsp") && !l.has_tag("usb"));
    }
}

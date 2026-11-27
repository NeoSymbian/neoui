// SPDX-License-Identifier: Apache-2.0
//! Word suggestions for the system keyboard (`keyboard.suggest`): the most frequent words that start with the word
//! being typed. Each language's list (`data/words-<code>.txt`, made by tools/make_wordlists.py from public-domain
//! texts) is `word\trank` per line, sorted by code point, and stays in the binary's read-only data: a prefix is
//! found by bisecting the bytes, so nothing is built on the heap.
//!
//! shortcut: Western Armenian uses the Eastern list (modern spelling); give it its own list (classical spelling) when
//! someone types it daily.

use alloc::string::String;
use alloc::vec::Vec;

const EN: &str = include_str!("../data/words-en.txt");
const HY: &str = include_str!("../data/words-hy.txt");
const RU: &str = include_str!("../data/words-ru.txt");

/// The list of a layout code, or none.
fn list(code: &str) -> Option<&'static str> {
    match code {
        "en" => Some(EN),
        "hy" | "hyw" => Some(HY),
        "ru" => Some(RU),
        _ => None,
    }
}

/// The word of the line starting at byte `start`, and where that line ends (its '\n', or the end).
fn line(list: &str, start: usize) -> (&str, usize) {
    let end = list[start..].find('\n').map_or(list.len(), |i| start + i);
    let l = &list[start..end];
    (l.split('\t').next().unwrap_or(l), end)
}

/// The byte offset of the first line whose word is not below `p` (code point order).
fn lower_bound(list: &str, p: &str) -> usize {
    let b = list.as_bytes();
    let (mut lo, mut hi) = (0, list.len());
    // every line starting before `lo` is below `p`; every line starting at or after `hi` is not
    while lo < hi {
        let mid = (lo + hi) / 2;
        let start = b[..mid]
            .iter()
            .rposition(|&c| c == b'\n')
            .map_or(0, |i| i + 1);
        let (w, end) = line(list, start);
        if w < p {
            lo = end + 1;
        } else {
            hi = start;
        }
    }
    lo.min(list.len())
}

/// Up to `n` words of language `code` that start with `word` and are longer, the most frequent first, in `word`'s
/// case (a capital first letter, or all capitals for two or more).
pub fn words(code: &str, word: &str, n: usize) -> Vec<String> {
    let Some(list) = list(code) else {
        return Vec::new();
    };
    let p = word.to_lowercase();
    if p.is_empty() {
        return Vec::new();
    }
    let mut best: Vec<(u32, &str)> = Vec::new();
    let mut at = lower_bound(list, &p);
    while at < list.len() {
        let (w, end) = line(list, at);
        if !w.starts_with(p.as_str()) {
            break;
        }
        if w.len() > p.len() {
            let rank = list[at + w.len()..end]
                .trim_start_matches('\t')
                .parse()
                .unwrap_or(u32::MAX);
            best.push((rank, w));
            best.sort_unstable();
            best.truncate(n);
        }
        at = end + 1;
    }
    let mut c = word.chars();
    let first_up = c.next().is_some_and(char::is_uppercase);
    let all_up = first_up && word.chars().count() > 1 && word.chars().all(|c| !c.is_lowercase());
    best.into_iter()
        .map(|(_, w)| {
            if all_up {
                w.to_uppercase()
            } else if first_up {
                let mut cs = w.chars();
                cs.next()
                    .map(|f| f.to_uppercase().chain(cs).collect())
                    .unwrap_or_default()
            } else {
                String::from(w)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "a\t1\nable\t9\nabout\t4\nabove\t7\nact\t8\nb\t2\nbe\t3\n";

    #[test]
    fn bisects_to_the_prefix() {
        assert_eq!(lower_bound(LIST, ""), 0);
        assert_eq!(&LIST[lower_bound(LIST, "ab")..][..4], "able");
        assert_eq!(&LIST[lower_bound(LIST, "ac")..][..3], "act");
        assert_eq!(&LIST[lower_bound(LIST, "b")..][..1], "b");
        assert_eq!(lower_bound(LIST, "z"), LIST.len());
    }

    #[test]
    fn every_list_is_sorted_and_ranked() {
        for code in ["en", "hy", "ru"] {
            let l = list(code).unwrap();
            let mut last = "";
            let mut n = 0;
            for line in l.lines() {
                let (w, r) = line.split_once('\t').expect(line);
                assert!(w > last, "{code}: {last} then {w}");
                assert!(r.parse::<u32>().is_ok(), "{code}: {line}");
                last = w;
                n += 1;
            }
            assert!(n >= 2000, "{code}: only {n} words");
        }
    }

    #[test]
    fn suggests_frequent_words_in_case() {
        assert_eq!(words("en", "th", 3).len(), 3);
        // "the" is the most frequent English word
        assert_eq!(words("en", "th", 3)[0], "the");
        assert_eq!(words("en", "Th", 1), ["The"]);
        assert_eq!(words("en", "TH", 1), ["THE"]);
        assert!(
            words("en", "the", 3)
                .iter()
                .all(|w| w.starts_with("the") && w.len() > 3)
        );
        assert!(words("en", "", 3).is_empty());
        assert!(words("xx", "a", 3).is_empty());
        assert!(words("en", "zzzzq", 3).is_empty());
        assert!(!words("ru", "пр", 3).is_empty());
        assert_eq!(words("ru", "Пр", 1)[0].chars().next(), Some('П'));
        assert!(!words("hy", "բա", 3).is_empty());
        assert_eq!(words("hyw", "բա", 3), words("hy", "բա", 3));
    }
}

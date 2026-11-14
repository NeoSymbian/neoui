// SPDX-License-Identifier: Apache-2.0
//! Smart dialling: the contacts whose names the digits typed in the dialer spell on the keypad
//! (docs/symbian-phone-sources.md §2, the Symbian^3 "Enter number or name" rules):
//! - names, company and nickname are split into words; each query word must start a different word, key by key;
//! - a 0 after the leading zeros also works as a space: the query is searched as typed and again split at those 0s;
//! - at most [`MAX`] results, by name, favourites last (the list is drawn upwards from the number field).

use crate::{Book, NameOrder, fold};
use alloc::string::String;
use alloc::vec::Vec;

pub const MAX: usize = 100;

/// The keys 2..=9 of each script we type: Latin (E.161), Russian (E.161 Cyrillic) and Armenian.
const KEYS: [[&str; 8]; 3] = [
    ["abc", "def", "ghi", "jkl", "mno", "pqrs", "tuv", "wxyz"],
    [
        "абвг",
        "деёжз",
        "ийкл",
        "мноп",
        "рсту",
        "фхцч",
        "шщъы",
        "ьэюя",
    ],
    // shortcut: no standard Armenian 12-key layout was found; the alphabet in order over keys 2..=9. Revisit if a
    // phone-standard one turns up.
    [
        "աբգդե",
        "զէըթժ",
        "իլխծկ",
        "հձղճմ",
        "յնշոչ",
        "պջռսվ",
        "տրցւփ",
        "քօֆև",
    ],
];

/// The key `c` is on, or `c` itself (digits, `*`, `#`, letters of other scripts).
pub fn key(c: char) -> char {
    for script in &KEYS {
        for (i, letters) in script.iter().enumerate() {
            if letters.contains(c) {
                return char::from(b'2' + i as u8);
            }
        }
    }
    c
}

fn keyed(word: &str) -> String {
    fold(word).chars().map(key).collect()
}

/// Every query word starts a different one of `words` (all keyed).
fn spells(query: &[&str], words: &[String]) -> bool {
    fn assign(q: &[&str], words: &[String], used: &mut Vec<bool>) -> bool {
        let Some((first, rest)) = q.split_first() else {
            return true;
        };
        for (i, w) in words.iter().enumerate() {
            if !used[i] && w.starts_with(first) {
                used[i] = true;
                if assign(rest, words, used) {
                    return true;
                }
                used[i] = false;
            }
        }
        false
    }
    assign(query, words, &mut alloc::vec![false; words.len()])
}

/// The contacts `digits` spells, best first for the list above the dialer's number field.
pub fn smart_matches(book: &Book, digits: &str, order: NameOrder) -> Vec<u32> {
    let typed = digits.trim();
    if typed.is_empty() {
        return Vec::new();
    }
    let lead = typed.len() - typed.trim_start_matches('0').len();
    let split: Vec<&str> = typed[lead..].split('0').filter(|w| !w.is_empty()).collect();
    let mut queries: Vec<Vec<&str>> = alloc::vec![alloc::vec![typed]];
    if split.len() > 1 || (split.len() == 1 && split[0].len() + lead != typed.len()) {
        // "abc0" -> "abc"; "00zxc" -> "zxc"; "00" alone stays as typed
        queries.push(split);
    }
    let mut hits: Vec<(bool, String, u32)> = book
        .contacts
        .iter()
        .filter(|c| {
            let words: Vec<String> = c.words().map(keyed).collect();
            queries.iter().any(|q| !q.is_empty() && spells(q, &words))
        })
        .map(|c| (c.favourite, c.sort_key(order), c.id))
        .collect();
    hits.sort();
    hits.truncate(MAX);
    hits.into_iter().map(|(_, _, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::person;
    use crate::{Book, Contact};

    fn book() -> Book {
        let mut b = Book::default();
        for (f, l) in [
            ("Ani", "Petrosyan"),
            ("Anna", "Smith"),
            ("Иван", "Петров"),
            ("Արամ", "Սարգսյան"),
            ("Bob", ""),
        ] {
            b.add(person(f, l, &["+37400000000"])).unwrap();
        }
        let mut fav = person("Anahit", "", &["1"]);
        fav.favourite = true;
        b.add(fav).unwrap();
        b.add(Contact {
            company: "Ucom Shop".into(),
            ..person("", "", &["2"])
        })
        .unwrap();
        b
    }

    #[test]
    fn keys() {
        assert_eq!(key('A'.to_ascii_lowercase()), '2');
        assert_eq!(key('я'), '9');
        assert_eq!(key('ա'), '2');
        assert_eq!(key('7'), '7');
        assert_eq!(keyed("Ani"), "264");
    }

    #[test]
    fn matches_word_prefixes() {
        let b = book();
        let ids = |d: &str| smart_matches(&b, d, NameOrder::FirstLast);
        // "26" = an.. or bo..: Ani, Anna, Bob, Anahit (favourite last)
        assert_eq!(ids("26"), [1, 2, 5, 6]);
        // "738" = Pet.. (Petrosyan); Иван = и(4)в(2)а(2)н(5)
        assert_eq!(ids("738"), [1]);
        assert_eq!(ids("4225"), [3]);
        // Armenian: Արամ = ա(2)ր(8)ա(2)մ(5)
        assert_eq!(ids("2825"), [4]);
        // company words
        assert_eq!(ids("7467"), [7]);
        assert!(ids("").is_empty());
        assert!(ids("99999").is_empty());
    }

    #[test]
    fn zero_is_a_space() {
        let b = book();
        let ids = |d: &str| smart_matches(&b, d, NameOrder::FirstLast);
        // "an" 0 "sm" -> Anna Smith; each word a different name word
        assert_eq!(ids("26076"), [2]);
        // "264" 0 -> also "264": Ani
        assert_eq!(ids("2640"), [1]);
        // two query words may not use the same name word
        assert!(ids("2620262").is_empty());
        assert!(ids("00").is_empty());
    }
}

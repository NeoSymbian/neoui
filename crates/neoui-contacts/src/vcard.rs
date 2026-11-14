// SPDX-License-Identifier: Apache-2.0
//! vCard 2.1 and 3.0 (docs/symbian-phone-sources.md §14): import as the Nokia phones wrote them (2.1, UTF-8 in
//! quoted-printable for anything non-ASCII) and as most other software does (3.0, folded lines, `TYPE=` parameters),
//! export as 2.1 so an old Nokia reads the file back. Unknown properties are skipped; a broken card is skipped and
//! counted, never a panic.

use crate::{Contact, Label, NUMBERS_MAX};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

/// What an import found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Import {
    pub contacts: Vec<Contact>,
    /// cards with nothing usable (no name, company or number)
    pub skipped: usize,
}

/// The logical lines: folded lines (a leading space or tab) and quoted-printable soft breaks (`=` at the end) joined.
fn lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut soft = false;
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(prev) = out.last_mut()
            && (soft || raw.starts_with([' ', '\t']))
        {
            if soft {
                prev.pop(); // the '='
                prev.push_str(raw);
            } else {
                prev.push_str(&raw[1..]);
            }
        } else {
            out.push(String::from(raw));
        }
        let last = out.last().map_or("", |s| s.as_str());
        soft = last.ends_with('=') && head(last).to_ascii_uppercase().contains("QUOTED-PRINTABLE");
    }
    out
}

/// The part of a line before its value.
fn head(line: &str) -> &str {
    line.split_once(':').map_or(line, |(h, _)| h)
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

fn quoted_printable(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (
            b[i],
            b.get(i + 1).copied().and_then(hex),
            b.get(i + 2).copied().and_then(hex),
        ) {
            (b'=', Some(h), Some(l)) => {
                out.push(h << 4 | l);
                i += 3;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A value's components (`;`-separated, `\;` escaped) with 3.0's `\n`, `\,` and `\\` undone.
fn components(v: &str) -> Vec<String> {
    let mut parts = alloc::vec![String::new()];
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n' | 'N') => parts.last_mut().unwrap().push('\n'),
                Some(x) => parts.last_mut().unwrap().push(x),
                None => {}
            },
            ';' => parts.push(String::new()),
            c => parts.last_mut().unwrap().push(c),
        }
    }
    parts
}

fn label(params: &[String]) -> Label {
    let has = |t: &str| params.iter().any(|p| p == t);
    match (has("CELL"), has("HOME"), has("WORK"), has("FAX")) {
        (_, _, _, true) => Label::Fax,
        (true, true, _, _) => Label::MobileHome,
        (true, _, true, _) => Label::MobileWork,
        (true, _, _, _) => Label::Mobile,
        (_, true, _, _) => Label::TelephoneHome,
        (_, _, true, _) => Label::TelephoneWork,
        _ => Label::Telephone,
    }
}

pub fn import(text: &str) -> Import {
    let mut out = Import::default();
    let mut card: Option<Contact> = None;
    for line in lines(text) {
        let Some((head, value)) = line.split_once(':') else {
            continue;
        };
        let mut params = head.split(';');
        let name = params.next().unwrap_or("").to_ascii_uppercase();
        // a group prefix ("item1.TEL")
        let name = name.rsplit('.').next().unwrap_or("");
        // "TYPE=CELL,HOME", "CELL", "ENCODING=QUOTED-PRINTABLE"
        let params: Vec<String> = params
            .flat_map(|p| {
                let p = p.to_ascii_uppercase();
                let v = p
                    .split_once('=')
                    .map_or(p.clone(), |(_, v)| String::from(v));
                v.split(',')
                    .map(|s| String::from(s.trim_matches('"')))
                    .collect::<Vec<_>>()
            })
            .collect();
        let value = if params.iter().any(|p| p == "QUOTED-PRINTABLE") {
            quoted_printable(value)
        } else {
            String::from(value)
        };
        match name {
            "BEGIN" if value.eq_ignore_ascii_case("VCARD") => card = Some(Contact::default()),
            "END" => {
                if let Some(mut c) = card.take() {
                    if c.tidy() {
                        out.contacts.push(c);
                    } else {
                        out.skipped += 1;
                    }
                }
            }
            _ => {
                let Some(c) = card.as_mut() else { continue };
                let parts = components(&value);
                let first = || parts.first().cloned().unwrap_or_default();
                match name {
                    "N" => {
                        c.last = first();
                        c.first = parts.get(1).cloned().unwrap_or_default();
                    }
                    "FN" if c.first.is_empty() && c.last.is_empty() => c.first = first(),
                    "ORG" => c.company = first(),
                    "NICKNAME" | "X-NICKNAME" => c.nickname = first(),
                    "TEL" if c.numbers.len() < NUMBERS_MAX => {
                        c.numbers.push((label(&params), value.trim().into()));
                    }
                    "EMAIL" if c.email.is_empty() => c.email = value.trim().into(),
                    "NOTE" => c.note = parts.join(";"),
                    "X-FAVORITE" => c.favourite = !matches!(value.trim(), "" | "0" | "false"),
                    _ => {}
                }
            }
        }
    }
    if card.is_some() {
        out.skipped += 1; // no END
    }
    out
}

/// `name;params:value` with UTF-8 quoted-printable when the value isn't plain ASCII.
fn property(out: &mut String, name: &str, value: &str) {
    if value.is_ascii() && !value.contains(['\n', '\r']) {
        let _ = write!(out, "{name}:{value}\r\n");
        return;
    }
    let _ = write!(out, "{name};CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:");
    for &b in value.as_bytes() {
        if (b' '..=b'~').contains(&b) && b != b'=' {
            out.push(b as char);
        } else {
            let _ = write!(out, "={b:02X}");
        }
    }
    out.push_str("\r\n");
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;")
}

pub fn export<'a>(contacts: impl IntoIterator<Item = &'a Contact>) -> String {
    let mut out = String::new();
    for c in contacts {
        out.push_str("BEGIN:VCARD\r\nVERSION:2.1\r\n");
        property(
            &mut out,
            "N",
            &alloc::format!("{};{};;;", escape(&c.last), escape(&c.first)),
        );
        let full = c.title(crate::NameOrder::FirstLast);
        if !(c.first.is_empty() && c.last.is_empty()) {
            property(&mut out, "FN", &full);
        }
        for (name, v) in [("ORG", &c.company), ("X-NICKNAME", &c.nickname)] {
            if !v.is_empty() {
                property(&mut out, name, &escape(v));
            }
        }
        for (l, n) in &c.numbers {
            let ty = match l {
                Label::Mobile => "TEL;CELL",
                Label::MobileHome => "TEL;CELL;HOME",
                Label::MobileWork => "TEL;CELL;WORK",
                Label::Telephone => "TEL;VOICE",
                Label::TelephoneHome => "TEL;HOME;VOICE",
                Label::TelephoneWork => "TEL;WORK;VOICE",
                Label::Fax => "TEL;FAX",
            };
            property(&mut out, ty, n);
        }
        if !c.email.is_empty() {
            property(&mut out, "EMAIL;INTERNET", &c.email);
        }
        if !c.note.is_empty() {
            property(&mut out, "NOTE", &c.note);
        }
        if c.favourite {
            out.push_str("X-FAVORITE:1\r\n");
        }
        out.push_str("END:VCARD\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn nokia_21_quoted_printable() {
        // "Ա" is D4 B1 in UTF-8; a soft line break splits the name
        let f = "BEGIN:VCARD\r\nVERSION:2.1\r\n\
                 N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=D5=8A=D5=A5=D5=BF=D6=80=D5=B8=D5=BD=D5=B5=D5=A1=D5=B6;=D4=\r\n\
                 =B1=D5=B6=D5=AB;;;\r\n\
                 TEL;CELL:+37491234567\r\nTEL;HOME;VOICE:010123456\r\nTEL;WORK;FAX:010999\r\n\
                 X-FAVORITE:1\r\nPHOTO;ENCODING=BASE64:AAAA\r\nEND:VCARD\r\n";
        let r = import(f);
        assert_eq!(r.skipped, 0);
        let c = &r.contacts[0];
        assert_eq!((c.first.as_str(), c.last.as_str()), ("Անի", "Պետրոսյան"));
        assert_eq!(
            c.numbers,
            vec![
                (Label::Mobile, "+37491234567".into()),
                (Label::TelephoneHome, "010123456".into()),
                (Label::Fax, "010999".into()),
            ]
        );
        assert!(c.favourite);
    }

    #[test]
    fn v30_folded_and_typed() {
        let f = "BEGIN:VCARD\nVERSION:3.0\nN:Петров;Иван;;;\nFN:Иван Петров\n\
                 item1.TEL;TYPE=CELL,HOME:+7 916 123-45-67\nTEL;TYPE=work:+74951234567\n\
                 EMAIL;TYPE=INTERNET:ivan@example.org\nNOTE:line one\\nline two\\, more\n  folded\n\
                 ORG:Some\\; Co;Dept\nEND:VCARD\n\
                 BEGIN:VCARD\nVERSION:3.0\nPHOTO:xyz\nEND:VCARD\nBEGIN:VCARD\nFN:No end";
        let r = import(f);
        assert_eq!(r.skipped, 2);
        let c = &r.contacts[0];
        assert_eq!((c.first.as_str(), c.last.as_str()), ("Иван", "Петров"));
        assert_eq!(c.numbers[0], (Label::MobileHome, "+79161234567".into()));
        assert_eq!(c.numbers[1].0, Label::TelephoneWork);
        assert_eq!(c.email, "ivan@example.org");
        assert_eq!(c.note, "line one line two, more folded");
        assert_eq!(c.company, "Some; Co");
    }

    #[test]
    fn fn_only_and_garbage() {
        let r =
            import("BEGIN:VCARD\nFN:Just Name\nTEL:123\nEND:VCARD\n\u{0}\u{1}junk:::\nEND:VCARD\n");
        assert_eq!(r.contacts[0].first, "Just Name");
        assert_eq!(r.contacts[0].numbers[0].0, Label::Telephone);
        assert_eq!(import("").contacts.len(), 0);
        assert_eq!(import("=\n=\n=").contacts.len(), 0);
    }

    #[test]
    fn export_reimports() {
        let cards = vec![
            Contact {
                first: "Անի".into(),
                last: "Pe;trosyan".into(),
                nickname: "Ան".into(),
                numbers: vec![
                    (Label::MobileWork, "+37491234567".into()),
                    (Label::Fax, "010999".into()),
                ],
                email: "ani@example.am".into(),
                note: "Ծանոթ = friend".into(),
                favourite: true,
                ..Default::default()
            },
            Contact {
                company: "Ucom".into(),
                numbers: vec![(Label::Telephone, "+37410000000".into())],
                ..Default::default()
            },
        ];
        let text = export(&cards);
        assert!(text.contains(
            "N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Pe\\;trosyan;=D4=B1=D5=B6=D5=AB;;;\r\n"
        ));
        assert!(text.contains("TEL;CELL;WORK:+37491234567\r\n"));
        let back = import(&text);
        assert_eq!(back.contacts, cards);
    }
}

// SPDX-License-Identifier: Apache-2.0
//! The contacts model (docs/telephony.md C8): the store and its file, names and sorting, search, caller-name lookup
//! ([`Book::lookup`]), smart dialling ([`t9`]) and vCard import/export ([`vcard`]). Pure logic, tested on the host;
//! the shell (SYSSTART) keeps the book in `C:\private\e0002001\contacts.dat` and draws it.
//! Field set and limits follow the Symbian^3 phonebook (docs/symbian-phone-sources.md §13).
#![no_std]

extern crate alloc;

pub mod t9;
pub mod vcard;

use alloc::string::String;
use alloc::vec::Vec;

/// Names, company and nickname, in characters.
pub const NAME_MAX: usize = 50;
pub const NUMBER_MAX: usize = 48;
pub const EMAIL_MAX: usize = 150;
pub const NOTE_MAX: usize = 1000;
/// Numbers per contact.
pub const NUMBERS_MAX: usize = 4;
/// Contacts per book (my card not counted).
pub const CONTACTS_MAX: usize = 1000;

/// What a phone number is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Label {
    #[default]
    Mobile = 0,
    MobileHome = 1,
    MobileWork = 2,
    Telephone = 3,
    TelephoneHome = 4,
    TelephoneWork = 5,
    Fax = 6,
}

impl Label {
    pub const ALL: [Label; 7] = [
        Label::Mobile,
        Label::MobileHome,
        Label::MobileWork,
        Label::Telephone,
        Label::TelephoneHome,
        Label::TelephoneWork,
        Label::Fax,
    ];

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(usize::from(v)).copied()
    }

    pub fn text(self) -> &'static str {
        match self {
            Label::Mobile => "Mobile",
            Label::MobileHome => "Mobile (home)",
            Label::MobileWork => "Mobile (work)",
            Label::Telephone => "Telephone",
            Label::TelephoneHome => "Telephone (home)",
            Label::TelephoneWork => "Telephone (work)",
            Label::Fax => "Fax",
        }
    }

    /// The next label (the editor's label cycler).
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }
}

/// How names show: "First Last" (the default) or "Last First".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameOrder {
    #[default]
    FirstLast,
    LastFirst,
}

impl NameOrder {
    pub fn from_setting(v: i32) -> Self {
        if v == 1 {
            NameOrder::LastFirst
        } else {
            NameOrder::FirstLast
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    /// 0 until the book adds it
    pub id: u32,
    pub first: String,
    pub last: String,
    pub company: String,
    pub nickname: String,
    pub numbers: Vec<(Label, String)>,
    pub email: String,
    pub note: String,
    pub favourite: bool,
}

fn clip(s: &mut String, max: usize) {
    let t = s.trim();
    let mut out: String = t
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect();
    out.truncate(out.trim_end().len());
    *s = out;
}

impl Contact {
    /// Trim, cut to the limits, drop empty numbers; true if anything is left to keep.
    pub fn tidy(&mut self) -> bool {
        for (s, max) in [
            (&mut self.first, NAME_MAX),
            (&mut self.last, NAME_MAX),
            (&mut self.company, NAME_MAX),
            (&mut self.nickname, NAME_MAX),
            (&mut self.email, EMAIL_MAX),
            (&mut self.note, NOTE_MAX),
        ] {
            clip(s, max);
        }
        for (_, n) in &mut self.numbers {
            n.retain(|c| c.is_ascii_digit() || "+*#pw".contains(c));
            n.truncate(NUMBER_MAX);
        }
        self.numbers.retain(|(_, n)| !n.is_empty());
        self.numbers.truncate(NUMBERS_MAX);
        !(self.first.is_empty()
            && self.last.is_empty()
            && self.company.is_empty()
            && self.nickname.is_empty()
            && self.numbers.is_empty()
            && self.email.is_empty())
    }

    /// The name lists, the card and the call screens show: first and last name in `order`, else the company, else
    /// the nickname, else "Unnamed".
    pub fn title(&self, order: NameOrder) -> String {
        let (a, b) = match order {
            NameOrder::FirstLast => (&self.first, &self.last),
            NameOrder::LastFirst => (&self.last, &self.first),
        };
        let mut t = String::from(a.as_str());
        if !b.is_empty() {
            if !t.is_empty() {
                t.push(' ');
            }
            t.push_str(b);
        }
        if t.is_empty() {
            t = [&self.company, &self.nickname]
                .into_iter()
                .find(|s| !s.is_empty())
                .map_or_else(|| String::from("Unnamed"), |s| s.clone());
        }
        t
    }

    /// The text sorting compares: the title, case folded.
    fn sort_key(&self, order: NameOrder) -> String {
        fold(&self.title(order))
    }

    /// The words search and smart dialling look at.
    fn words(&self) -> impl Iterator<Item = &str> {
        [&self.first, &self.last, &self.company, &self.nickname]
            .into_iter()
            .flat_map(|s| s.split_whitespace())
    }
}

/// Lower case, for comparing names in any script.
pub fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Book {
    pub contacts: Vec<Contact>,
    /// The owner's own card ("My Card"); never matched as a caller.
    pub my_card: Option<Contact>,
    next_id: u32,
}

/// Why a change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Empty,
    Full,
    NotFound,
}

impl Book {
    pub fn get(&self, id: u32) -> Option<&Contact> {
        self.contacts.iter().find(|c| c.id == id)
    }

    /// Add `c` (tidied); its new id.
    pub fn add(&mut self, mut c: Contact) -> Result<u32, Refused> {
        if !c.tidy() {
            return Err(Refused::Empty);
        }
        if self.contacts.len() >= CONTACTS_MAX {
            return Err(Refused::Full);
        }
        self.next_id = self.next_id.max(1);
        c.id = self.next_id;
        self.next_id += 1;
        self.contacts.push(c);
        Ok(self.next_id - 1)
    }

    /// Replace the contact with `c.id`; an edit that empties it deletes it.
    pub fn update(&mut self, mut c: Contact) -> Result<(), Refused> {
        let i = self
            .contacts
            .iter()
            .position(|x| x.id == c.id)
            .ok_or(Refused::NotFound)?;
        if c.tidy() {
            self.contacts[i] = c;
        } else {
            self.contacts.remove(i);
        }
        Ok(())
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let n = self.contacts.len();
        self.contacts.retain(|c| c.id != id);
        n != self.contacts.len()
    }

    pub fn set_my_card(&mut self, c: Option<Contact>) {
        self.my_card = c.and_then(|mut c| c.tidy().then_some(Contact { id: 0, ..c }));
    }

    /// Every contact's id: favourites first, then by name.
    pub fn sorted(&self, order: NameOrder) -> Vec<u32> {
        let mut v: Vec<(bool, String, u32)> = self
            .contacts
            .iter()
            .map(|c| (!c.favourite, c.sort_key(order), c.id))
            .collect();
        v.sort();
        v.into_iter().map(|(_, _, id)| id).collect()
    }

    /// Contacts for the list's search field, in [`Book::sorted`] order: every word of `text` starts a word of a name
    /// (case folded), or `text` is part of a number.
    pub fn search(&self, text: &str, order: NameOrder) -> Vec<u32> {
        let q: Vec<String> = text.split_whitespace().map(fold).collect();
        if q.is_empty() {
            return self.sorted(order);
        }
        let digits = phone_rules::number::digits(text);
        self.sorted(order)
            .into_iter()
            .filter(|id| {
                let c = self.get(*id).unwrap();
                let words: Vec<String> = c.words().map(fold).collect();
                q.iter()
                    .all(|w| words.iter().any(|x| x.starts_with(w.as_str())))
                    || (digits.len() >= 3
                        && digits.len() == text.trim().trim_start_matches('+').len()
                        && c.numbers.iter().any(|(_, n)| n.contains(digits.as_str())))
            })
            .collect()
    }

    /// The contact a caller's `number` belongs to: only when exactly one contact matches (docs/symbian-phone-sources
    /// §13), so nobody is ever shown under a wrong name.
    pub fn lookup(&self, number: &str) -> Option<&Contact> {
        let id = phone_rules::number::unique_match(
            number,
            self.contacts
                .iter()
                .flat_map(|c| c.numbers.iter().map(move |(_, n)| (c.id, n.as_str()))),
        )?;
        self.get(id)
    }

    /// `contacts.dat`: "NCT1", next id, the count, a my-card flag, then the contacts (my card first).
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&self.next_id.to_le_bytes());
        b.extend_from_slice(&(self.contacts.len() as u16).to_le_bytes());
        b.push(self.my_card.is_some() as u8);
        for c in self.my_card.iter().chain(&self.contacts) {
            b.extend_from_slice(&c.id.to_le_bytes());
            b.push(c.favourite as u8);
            for s in [
                &c.first,
                &c.last,
                &c.company,
                &c.nickname,
                &c.email,
                &c.note,
            ] {
                put(&mut b, s);
            }
            b.push(c.numbers.len() as u8);
            for (l, n) in &c.numbers {
                b.push(*l as u8);
                put(&mut b, n);
            }
        }
        b
    }

    /// A saved book; `None` when the file is not one (the shell then keeps the file aside, never overwrites it).
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != MAGIC {
            return None;
        }
        let next_id = r.u32()?;
        let n = usize::from(r.u16()?);
        let mine = r.u8()? != 0;
        let mut book = Book {
            next_id,
            ..Default::default()
        };
        for i in 0..n + usize::from(mine) {
            let mut c = Contact {
                id: r.u32()?,
                favourite: r.u8()? != 0,
                ..Default::default()
            };
            for s in [
                &mut c.first,
                &mut c.last,
                &mut c.company,
                &mut c.nickname,
                &mut c.email,
                &mut c.note,
            ] {
                *s = r.string()?;
            }
            for _ in 0..r.u8()? {
                let l = Label::from_u8(r.u8()?)?;
                c.numbers.push((l, r.string()?));
            }
            if mine && i == 0 {
                book.my_card = Some(c);
            } else {
                book.next_id = book.next_id.max(c.id + 1);
                book.contacts.push(c);
            }
        }
        (r.at == b.len()).then_some(book)
    }
}

const MAGIC: &[u8; 4] = b"NCT1";

fn put(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(&(s.len() as u16).to_le_bytes());
    b.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn string(&mut self) -> Option<String> {
        let n = usize::from(self.u16()?);
        core::str::from_utf8(self.take(n)?).ok().map(String::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    pub(crate) fn person(first: &str, last: &str, numbers: &[&str]) -> Contact {
        Contact {
            first: first.into(),
            last: last.into(),
            numbers: numbers
                .iter()
                .map(|n| (Label::Mobile, String::from(*n)))
                .collect(),
            ..Default::default()
        }
    }

    fn book() -> Book {
        let mut b = Book::default();
        b.add(person("Ani", "Petrosyan", &["+37491234567"]))
            .unwrap();
        b.add(person("Ivan", "Петров", &["+79161234567", "84951234567"]))
            .unwrap();
        let mut fav = person("Zara", "", &["+37455000111"]);
        fav.favourite = true;
        b.add(fav).unwrap();
        b.add(Contact {
            company: "Ucom".into(),
            numbers: vec![(Label::Telephone, "+37410000000".into())],
            ..Default::default()
        })
        .unwrap();
        b
    }

    #[test]
    fn titles_and_order() {
        let b = book();
        assert_eq!(
            b.get(1).unwrap().title(NameOrder::LastFirst),
            "Petrosyan Ani"
        );
        assert_eq!(b.get(4).unwrap().title(NameOrder::FirstLast), "Ucom");
        assert_eq!(Contact::default().title(NameOrder::FirstLast), "Unnamed");
        assert_eq!(b.sorted(NameOrder::FirstLast), [3, 1, 2, 4]);
        assert_eq!(b.sorted(NameOrder::LastFirst), [3, 1, 4, 2]);
    }

    #[test]
    fn search_names_and_numbers() {
        let b = book();
        assert_eq!(b.search("an pe", NameOrder::FirstLast), [1]);
        assert_eq!(b.search("пет", NameOrder::FirstLast), [2]);
        assert_eq!(b.search("1234567", NameOrder::FirstLast), [1, 2]);
        assert_eq!(b.search("  ", NameOrder::FirstLast).len(), 4);
    }

    #[test]
    fn lookup_unique_only() {
        let mut b = book();
        assert_eq!(b.lookup("091234567").map(|c| c.id), Some(1));
        assert_eq!(b.lookup("+7 916 123-45-67").map(|c| c.id), Some(2));
        assert_eq!(b.lookup("+37499999999"), None);
        b.add(person("Ani", "Twin", &["091234567"])).unwrap();
        assert_eq!(b.lookup("+37491234567"), None);
        b.set_my_card(Some(person("Me", "", &["+37477777777"])));
        assert_eq!(b.lookup("+37477777777"), None);
    }

    #[test]
    fn tidy_limits() {
        let mut b = Book::default();
        assert_eq!(b.add(person(" ", "", &[" "])), Err(Refused::Empty));
        let id = b
            .add(person(
                "  Ani\n ",
                "",
                &["+374 91-23", "", "1", "2", "3", "4"],
            ))
            .unwrap();
        let c = b.get(id).unwrap();
        assert_eq!(c.first, "Ani");
        assert_eq!(c.numbers.len(), NUMBERS_MAX);
        assert_eq!(c.numbers[0].1, "+3749123");
        let mut e = c.clone();
        e.first.clear();
        e.numbers.clear();
        b.update(e).unwrap();
        assert!(b.get(id).is_none(), "an emptied contact is deleted");
        assert_eq!(b.update(person("X", "", &[])), Err(Refused::NotFound));
    }

    #[test]
    fn file_round_trip() {
        let mut b = book();
        b.set_my_card(Some(person("Me", "Myself", &["+37477777777"])));
        b.remove(2);
        let bytes = b.encode();
        let back = Book::decode(&bytes).unwrap();
        assert_eq!(back, b);
        let mut more = back.clone();
        assert_eq!(more.add(person("New", "", &["1"])), Ok(5));
        assert_eq!(Book::decode(&bytes[..bytes.len() - 1]), None);
        assert_eq!(Book::decode(b"NCT0"), None);
        let mut bad = bytes.clone();
        bad.push(0);
        assert_eq!(Book::decode(&bad), None);
    }
}

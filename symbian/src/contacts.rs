// SPDX-License-Identifier: GPL-3.0-only
//! Contacts (telephony C8): the book in `C:\private\e0002001\contacts.dat`, the Contacts screens' actions, vCard
//! import (every `.vcf` in `E:\` and `C:\Data\`) and export (`C:\Data\contacts.vcf`). The model is the host-tested
//! `neoui-contacts` crate; this module adds the files and the UI.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use e32::sys::err;
use neoui_contacts::{Book, Contact, Label, NameOrder, vcard};
use neoui_render::{CardItem, Editor, PersonItem, Renderer};

const DIR: &str = "C:\\private\\e0002001\\";
const PATH: &str = "C:\\private\\e0002001\\contacts.dat";
const NEW_PATH: &str = "C:\\private\\e0002001\\contacts.new";
/// An unreadable book is moved here, never overwritten.
const BAD_PATH: &str = "C:\\private\\e0002001\\contacts.bad";
const EXPORT_DIR: &str = "C:\\Data\\";
const EXPORT_PATH: &str = "C:\\Data\\contacts.vcf";
/// The largest file read (a book of 1000 full contacts is well under this).
const FILE_MAX: usize = 2 << 20;

pub struct Contacts {
    pub book: Book,
    pub order: NameOrder,
    /// the card shown: a contact id, or -1 for my card
    card: i32,
}

fn read(fs: &Fs, path: &str) -> Result<Vec<u8>, i32> {
    let f = File::open(fs, path, mode::READ | mode::SHARE_READERS_ONLY)?;
    let size = usize::try_from(f.size()?).unwrap_or(usize::MAX);
    if size > FILE_MAX {
        return Err(-40); // KErrTooBig
    }
    let mut b = vec![0u8; size];
    let n = f.read_at(0, &mut b)?;
    b.truncate(n);
    Ok(b)
}

fn mkdirs(fs: &Fs, dirs: &[&str]) -> Result<(), i32> {
    for d in dirs {
        match fs.mkdir(d) {
            Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl Contacts {
    /// The saved book (also from `contacts.new` after a cut between delete and rename); an unreadable file is kept
    /// aside as `contacts.bad` and the book starts empty.
    pub fn load() -> Self {
        let mut book = Book::default();
        if let Ok(fs) = Fs::connect() {
            for path in [PATH, NEW_PATH] {
                match read(&fs, path) {
                    Ok(b) => {
                        match Book::decode(&b) {
                            Some(x) => {
                                book = x;
                                dprintln!("neoui: contacts loaded ({})", book.contacts.len());
                            }
                            None => {
                                let _ = fs.delete(BAD_PATH);
                                let r = fs.rename(path, BAD_PATH);
                                dprintln!(
                                    "neoui: contacts unreadable, kept as contacts.bad ({:?})",
                                    r
                                );
                            }
                        }
                        break;
                    }
                    Err(err::KERR_NOT_FOUND | err::KERR_PATH_NOT_FOUND) => {}
                    Err(e) => {
                        dprintln!("neoui: contacts not read ({})", e);
                        break;
                    }
                }
            }
        }
        Self {
            book,
            order: NameOrder::FirstLast,
            card: 0,
        }
    }

    fn save(&self) -> Result<(), i32> {
        let fs = Fs::connect()?;
        mkdirs(&fs, &["C:\\private\\", DIR])?;
        File::replace(&fs, NEW_PATH, mode::SHARE_EXCLUSIVE)?.write(&self.book.encode())?;
        match fs.delete(PATH) {
            Ok(()) | Err(err::KERR_NOT_FOUND) => {}
            Err(e) => return Err(e),
        }
        fs.rename(NEW_PATH, PATH)
    }

    /// Save, reporting a failure as a note.
    fn store(&self) -> Option<String> {
        self.save()
            .err()
            .map(|e| format!("Contacts could not be saved ({e})"))
    }

    /// The name a caller's number shows, when exactly one contact has it.
    pub fn name_of(&self, number: &str) -> Option<String> {
        self.book.lookup(number).map(|c| c.title(self.order))
    }

    /// The list as the search field filters it.
    pub fn show_list(&self, ui: &mut Renderer) {
        let ids = self.book.search(&ui.contact_search(), self.order);
        let rows: Vec<(u32, String, String, bool)> = ids
            .iter()
            .filter_map(|id| self.book.get(*id))
            .map(|c| {
                let detail = c
                    .numbers
                    .first()
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| c.company.clone());
                (c.id, c.title(self.order), detail, c.favourite)
            })
            .collect();
        let items: Vec<PersonItem> = rows
            .iter()
            .map(|(id, name, detail, favourite)| PersonItem {
                id: *id,
                name,
                detail,
                favourite: *favourite,
            })
            .collect();
        ui.set_people(
            &items,
            self.book.contacts.len(),
            self.book.my_card.is_some(),
        );
    }

    fn contact(&self, id: i32) -> Option<&Contact> {
        if id < 0 {
            self.book.my_card.as_ref()
        } else {
            self.book.get(id as u32)
        }
    }

    fn show_card(&mut self, ui: &mut Renderer, id: i32) {
        self.card = id;
        let Some(c) = self.contact(id) else { return };
        let name = c.title(self.order);
        let numbers: Vec<(&str, &str)> = c
            .numbers
            .iter()
            .map(|(l, n)| (l.text(), n.as_str()))
            .collect();
        ui.set_card(&CardItem {
            id,
            name: &name,
            company: &c.company,
            numbers: &numbers,
            email: &c.email,
            note: &c.note,
            favourite: c.favourite,
        });
    }

    fn edit(&self, ui: &mut Renderer, id: i32, number: &str) {
        let mut e = Editor {
            id,
            ..Default::default()
        };
        if let Some(c) = self.contact(id) {
            e.first = c.first.clone();
            e.last = c.last.clone();
            e.company = c.company.clone();
            e.email = c.email.clone();
            e.note = c.note.clone();
            e.numbers = c
                .numbers
                .iter()
                .map(|(l, n)| (l.text().into(), n.clone()))
                .collect();
        } else if !number.is_empty() {
            e.numbers.push((Label::Mobile.text().into(), number.into()));
        }
        ui.set_editor(&e);
    }

    fn from_editor(e: &Editor) -> Contact {
        let label = |t: &str| {
            Label::ALL
                .into_iter()
                .find(|l| l.text() == t)
                .unwrap_or_default()
        };
        Contact {
            id: e.id.max(0) as u32,
            first: e.first.clone(),
            last: e.last.clone(),
            company: e.company.clone(),
            email: e.email.clone(),
            note: e.note.clone(),
            numbers: e
                .numbers
                .iter()
                .map(|(l, n)| (label(l), n.clone()))
                .collect(),
            ..Default::default()
        }
    }

    /// A Contacts action from the screens (see ui/phone-data.slint); a note to show, if any.
    pub fn act(&mut self, ui: &mut Renderer, action: &str, value: &str) -> Option<String> {
        let id = value.parse::<i32>().unwrap_or(0);
        match action {
            "contact-search" => self.show_list(ui),
            "contact-open" => {
                if id < 0 && self.book.my_card.is_none() {
                    self.edit(ui, -1, "");
                } else {
                    self.show_card(ui, id);
                }
            }
            "contact-new" => self.edit(ui, 0, ""),
            "contact-edit" => self.edit(ui, id, ""),
            // from Recents or the dialer: a new contact with this number, or the contact that has it
            "add-contact" => {
                self.edit(ui, 0, value);
                ui.show("contacts", "edit");
            }
            "open-contact" => {
                if let Some(c) = self.book.lookup(value).map(|c| c.id as i32) {
                    self.show_card(ui, c);
                    ui.show("contacts", "card");
                }
            }
            "contact-label" => {
                let i = usize::try_from(id).unwrap_or(0);
                let now = ui.editor().numbers.get(i).map(|(l, _)| l.clone());
                let next = Label::ALL
                    .into_iter()
                    .find(|l| Some(l.text()) == now.as_deref())
                    .unwrap_or_default()
                    .next();
                ui.set_edit_label(i, next.text());
            }
            "contact-save" => {
                let e = ui.editor();
                let mut c = Self::from_editor(&e);
                let shown = match e.id {
                    -1 => {
                        c.favourite = false;
                        self.book.set_my_card(Some(c));
                        -1
                    }
                    0 => match self.book.add(c) {
                        Ok(id) => id as i32,
                        Err(e) => return Some(format!("Not saved ({e:?})")),
                    },
                    id => {
                        c.favourite = self.book.get(id as u32).is_some_and(|x| x.favourite);
                        if self.book.update(c).is_err() {
                            return Some("Not saved".into());
                        }
                        id
                    }
                };
                self.show_card(ui, shown);
                self.show_list(ui);
                return self.store().or_else(|| {
                    Some(
                        if e.id == 0 {
                            "New contact created"
                        } else {
                            "Contact updated"
                        }
                        .into(),
                    )
                });
            }
            "contact-delete" => {
                if id < 0 {
                    self.book.set_my_card(None);
                } else {
                    self.book.remove(id as u32);
                }
                self.show_list(ui);
                return self.store();
            }
            "contact-favourite" => {
                if let Some(c) = self.book.contacts.iter_mut().find(|c| c.id as i32 == id) {
                    c.favourite = !c.favourite;
                }
                self.show_card(ui, id);
                self.show_list(ui);
                return self.store();
            }
            "contact-import" => return Some(self.import(ui)),
            "contact-export" => return Some(self.export()),
            _ => {}
        }
        None
    }

    /// The name order setting changed: titles and sorting follow.
    pub fn set_order(&mut self, ui: &mut Renderer, order: NameOrder) {
        if order != self.order {
            self.order = order;
            self.show_list(ui);
            if self.card != 0 {
                let card = self.card;
                self.show_card(ui, card);
            }
        }
    }

    fn import(&mut self, ui: &mut Renderer) -> String {
        let Ok(fs) = Fs::connect() else {
            return "No file server".into();
        };
        let (mut added, mut skipped, mut files) = (0, 0, 0);
        for dir in ["E:\\", EXPORT_DIR] {
            let Ok(entries) = fs.read_dir(dir) else {
                continue;
            };
            for e in entries
                .iter()
                .filter(|e| !e.is_dir() && e.name.to_ascii_lowercase().ends_with(".vcf"))
            {
                let path = format!("{dir}{}", e.name);
                // our own export is not imported back
                if path.eq_ignore_ascii_case(EXPORT_PATH) {
                    continue;
                }
                let Ok(bytes) = read(&fs, &path) else {
                    skipped += 1;
                    continue;
                };
                files += 1;
                let found = vcard::import(&String::from_utf8_lossy(&bytes));
                skipped += found.skipped;
                for c in found.contacts {
                    let dup = self.book.contacts.iter().any(|x| {
                        x.title(NameOrder::FirstLast) == c.title(NameOrder::FirstLast)
                            && x.numbers.first() == c.numbers.first()
                    });
                    if dup || self.book.add(c).is_err() {
                        skipped += 1;
                    } else {
                        added += 1;
                    }
                }
            }
        }
        dprintln!(
            "neoui: contacts imported {} ({} skipped, {} files)",
            added,
            skipped,
            files
        );
        self.show_list(ui);
        if let Some(e) = self.store() {
            return e;
        }
        match files {
            0 => "No .vcf files in E:\\ or C:\\Data\\".into(),
            _ => format!("Imported {added} contacts ({skipped} skipped)"),
        }
    }

    fn export(&self) -> String {
        let text = vcard::export(self.book.my_card.iter().chain(&self.book.contacts));
        let r = Fs::connect().and_then(|fs| {
            mkdirs(&fs, &[EXPORT_DIR])?;
            File::replace(&fs, EXPORT_PATH, mode::SHARE_EXCLUSIVE)?.write(text.as_bytes())
        });
        match r {
            Ok(()) => format!(
                "Exported {} contacts to {EXPORT_PATH}",
                self.book.contacts.len()
            ),
            Err(e) => format!("Export failed ({e})"),
        }
    }
}

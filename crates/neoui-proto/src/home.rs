// SPDX-License-Identifier: Apache-2.0
//! (D10) Home's pages and dock (docs `neoui-shell/home.md` Part 3): the model, the packer that places a page's items
//! on its 4 × 6 grid, the edits the shell's home editor makes (each checks that the result fits before it changes
//! anything), and the `home.layout` bytes.
//!
//! The bytes (version 1): `[1, pageCount, keyCount]`, `keyCount × [len, key ASCII]`, four dock refs, then each page
//! `[itemCount, items…]`; a shortcut is `[0, keyRef]`, a widget `[1, keyRef, widgetId, size]`. Every count, ref and
//! discriminant is a `u8`; ref 0 is an empty dock slot and `1..=keyCount` a key. Anything else (another version, a
//! widget not in [`CATALOGUE`], a page that doesn't pack, trailing bytes) is refused and home shows [`Layout::default`].
//!
//! Version 2 (protocol 1.22, home.md Part 4) adds an app's widget, `[2, keyRef, nameLen, name ASCII, size]`, whose
//! `(app, name)` and size are in [`crate::widget::CATALOGUE`]. A layout without one is still written as version 1,
//! and both versions are read.

use alloc::string::String;
use alloc::vec::Vec;

/// A page's grid: 4 columns, 6 rows of cells.
pub const COLS: u8 = 4;
pub const ROWS: u8 = 6;
/// Home has 1 to 5 pages.
pub const MAX_PAGES: usize = 5;
/// The dock's editable slots (Apps sits after them, fixed).
pub const DOCK: usize = 4;
/// At most this many distinct app keys, each 1–16 ASCII bytes, in at most this many bytes.
pub const MAX_KEYS: usize = 48;
pub const MAX_KEY_LEN: usize = 16;
pub const MAX_BYTES: usize = 2048;
pub const VERSION: u8 = 2;
/// The version a layout without app widgets is written as (so an older shell still reads it).
pub const VERSION_1: u8 = 1;

/// A widget home draws (the shell feeds them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Widget {
    Clock = 1,
    Agenda = 2,
    Favourites = 3,
    Media = 4,
}

/// A widget's size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Size {
    /// 4 × 1
    Wide = 1,
    /// 4 × 2
    Tall = 2,
    /// 2 × 2
    Square = 3,
}

impl Size {
    /// (columns, rows)
    #[must_use]
    pub const fn cells(self) -> (u8, u8) {
        match self {
            Size::Wide => (4, 1),
            Size::Tall => (4, 2),
            Size::Square => (2, 2),
        }
    }
    fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            1 => Size::Wide,
            2 => Size::Tall,
            3 => Size::Square,
            _ => return None,
        })
    }
}

impl Widget {
    fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            1 => Widget::Clock,
            2 => Widget::Agenda,
            3 => Widget::Favourites,
            4 => Widget::Media,
            _ => return None,
        })
    }
}

/// The widgets on offer: the widget, the app it opens, its sizes (the Add sheet offers each; the first is the one a
/// version-1 layout knew), its label.
/// shortcut: Media has no 4 × 2 yet, it comes with the media session (top-panel.md TP3).
pub const CATALOGUE: &[(Widget, &str, &[Size], &str)] = &[
    (Widget::Clock, "clock", &[Size::Tall, Size::Square], "Clock"),
    (
        Widget::Agenda,
        "calendar",
        &[Size::Tall, Size::Square],
        "Agenda",
    ),
    (
        Widget::Favourites,
        "contacts",
        &[Size::Wide, Size::Square],
        "Favourites",
    ),
    (Widget::Media, "music", &[Size::Wide, Size::Square], "Music"),
];

/// The catalogue row of `w`.
#[must_use]
pub fn entry(w: Widget) -> &'static (Widget, &'static str, &'static [Size], &'static str) {
    CATALOGUE.iter().find(|e| e.0 == w).unwrap_or(&CATALOGUE[0])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// An app's icon, 1 × 1.
    Shortcut(String),
    Widget {
        widget: Widget,
        size: Size,
    },
    /// (v2) An app's widget of [`crate::widget::CATALOGUE`]: its content comes with `ipc::WIDGET`.
    AppWidget {
        app: String,
        name: String,
        size: Size,
    },
}

impl Item {
    /// (columns, rows)
    #[must_use]
    pub fn cells(&self) -> (u8, u8) {
        match self {
            Item::Shortcut(_) => (1, 1),
            Item::Widget { size, .. } | Item::AppWidget { size, .. } => size.cells(),
        }
    }
    /// The app it opens.
    #[must_use]
    pub fn app(&self) -> &str {
        match self {
            Item::Shortcut(k) => k,
            Item::Widget { widget, .. } => entry(*widget).1,
            Item::AppWidget { app, .. } => app,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub pages: Vec<Page>,
    pub dock: [Option<String>; DOCK],
}

/// Where the packer put item `index` of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub index: usize,
    pub col: u8,
    pub row: u8,
    pub w: u8,
    pub h: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The page has no room (more than 6 rows of cells).
    Full,
    /// Five pages already.
    TooManyPages,
    /// Not a widget and size of [`CATALOGUE`], an app key that isn't 1–16 ASCII bytes, or too many keys or bytes.
    Invalid,
    /// No such page, item or dock slot.
    NotFound,
}

/// Packs `items` in order, row by row: each at the first spot at or after the previous one's where it fits, so the
/// reading order is the stored order (a 2 × 2 beside another or beside two shortcuts, a 4 × n on its own rows).
///
/// # Errors
/// [`Error::Full`] when an item would end below row 6.
pub fn pack(items: &[Item]) -> Result<Vec<Placed>, Error> {
    pack_in(items, usize::from(ROWS))
}

/// (G5) Lanes' Home lane: every page's items in one column, packed the same way.
#[must_use]
pub fn pack_lane(layout: &Layout) -> Vec<(usize, usize, Placed)> {
    let all: Vec<(usize, usize, Item)> = layout
        .pages
        .iter()
        .enumerate()
        .flat_map(|(p, page)| {
            page.items
                .iter()
                .enumerate()
                .map(move |(i, it)| (p, i, it.clone()))
        })
        .collect();
    let items: Vec<Item> = all.iter().map(|a| a.2.clone()).collect();
    // every page fits its 6 rows, so all of them fit 6 a page
    pack_in(&items, usize::from(ROWS) * MAX_PAGES)
        .unwrap_or_default()
        .into_iter()
        .map(|placed| (all[placed.index].0, all[placed.index].1, placed))
        .collect()
}

fn pack_in(items: &[Item], rows: usize) -> Result<Vec<Placed>, Error> {
    let cols = usize::from(COLS);
    let mut used = alloc::vec![[false; COLS as usize]; rows];
    let mut out = Vec::with_capacity(items.len());
    let mut from = 0usize; // row-major cell index to search from
    for (index, item) in items.iter().enumerate() {
        let (w, h) = item.cells();
        let (w, h) = (usize::from(w), usize::from(h));
        let at = (from..rows * cols).find(|&c| {
            let (row, col) = (c / cols, c % cols);
            col + w <= cols
                && row + h <= rows
                && (row..row + h).all(|r| (col..col + w).all(|c| !used[r][c]))
        });
        let Some(at) = at else {
            return Err(Error::Full);
        };
        let (row, col) = (at / cols, at % cols);
        for r in &mut used[row..row + h] {
            r[col..col + w].fill(true);
        }
        out.push(Placed {
            index,
            col: col as u8,
            row: row as u8,
            w: w as u8,
            h: h as u8,
        });
        from = at + w;
    }
    Ok(out)
}

/// Where an item let go over cell (`col`, `row`) of `page` goes in its order: before the first other item placed at
/// or after that cell (reading order), else last. `index` is the item being moved (left out of the count).
#[must_use]
pub fn drop_index(items: &[Item], index: usize, col: u8, row: u8) -> usize {
    let mut rest = items.to_vec();
    if index < rest.len() {
        rest.remove(index);
    }
    pack(&rest)
        .unwrap_or_default()
        .iter()
        .find(|p| (p.row, p.col) >= (row, col))
        .map_or(rest.len(), |p| p.index)
}

fn app_widget_ok(app: &str, name: &str, size: Size) -> bool {
    crate::widget::find(app, name).is_some_and(|e| e.3.contains(&size))
}

fn valid_key(k: &str) -> bool {
    (1..=MAX_KEY_LEN).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_graphic())
}

impl Default for Layout {
    /// Page 0: Clock 4 × 2, Agenda 4 × 2, Favourites 4 × 1, Music 4 × 1; pages 1–2 empty; the dock Phone, Messages,
    /// Browser, Camera.
    fn default() -> Self {
        let w = |widget, size| Item::Widget { widget, size };
        Self {
            pages: alloc::vec![
                Page {
                    items: alloc::vec![
                        w(Widget::Clock, Size::Tall),
                        w(Widget::Agenda, Size::Tall),
                        w(Widget::Favourites, Size::Wide),
                        w(Widget::Media, Size::Wide),
                    ],
                },
                Page::default(),
                Page::default(),
            ],
            dock: ["phone", "messages", "browser", "camera"].map(|k| Some(String::from(k))),
        }
    }
}

impl Layout {
    /// Changes page `page` with `f` only if the result still packs.
    fn edit_page(&mut self, page: usize, f: impl FnOnce(&mut Vec<Item>)) -> Result<(), Error> {
        let p = self.pages.get(page).ok_or(Error::NotFound)?;
        let mut items = p.items.clone();
        f(&mut items);
        pack(&items)?;
        self.pages[page].items = items;
        Ok(())
    }

    /// Adds `app`'s icon at the end of page `page`.
    ///
    /// # Errors
    /// [`Error::Full`], [`Error::NotFound`], [`Error::Invalid`] for a bad key.
    pub fn pin(&mut self, app: &str, page: usize) -> Result<(), Error> {
        if !valid_key(app) {
            return Err(Error::Invalid);
        }
        let mut next = self.clone();
        next.edit_page(page, |items| items.push(Item::Shortcut(String::from(app))))?;
        next.check_size()?;
        *self = next;
        Ok(())
    }

    /// Adds `app`'s icon to page `page`, or else to the first page with room (Add to home); returns the page.
    ///
    /// # Errors
    /// [`Error::Full`] when no page has room.
    pub fn pin_anywhere(&mut self, app: &str, page: usize) -> Result<usize, Error> {
        let order = core::iter::once(page).chain((0..self.pages.len()).filter(|&p| p != page));
        for p in order {
            match self.pin(app, p) {
                Ok(()) => return Ok(p),
                Err(Error::Full | Error::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        Err(Error::Full)
    }

    /// Removes every icon of `app` from the pages (not the dock).
    pub fn unpin(&mut self, app: &str) {
        for p in &mut self.pages {
            p.items
                .retain(|i| !matches!(i, Item::Shortcut(k) if k == app));
        }
    }

    /// Puts `app` in dock slot `slot` (replacing what was there), or empties it (`None`).
    ///
    /// # Errors
    /// [`Error::NotFound`] for a slot past 3, [`Error::Invalid`] for a bad key.
    pub fn pin_dock(&mut self, app: Option<&str>, slot: usize) -> Result<(), Error> {
        if slot >= DOCK || app.is_some_and(|a| !valid_key(a)) {
            return Err(if slot >= DOCK {
                Error::NotFound
            } else {
                Error::Invalid
            });
        }
        let old = core::mem::replace(&mut self.dock[slot], app.map(String::from));
        self.check_size().inspect_err(|_| self.dock[slot] = old)
    }

    /// Adds a widget of [`CATALOGUE`] at the end of page `page`.
    ///
    /// # Errors
    /// [`Error::Invalid`] for a size the widget doesn't have, [`Error::Full`], [`Error::NotFound`].
    pub fn add_widget(&mut self, widget: Widget, size: Size, page: usize) -> Result<(), Error> {
        if !entry(widget).2.contains(&size) {
            return Err(Error::Invalid);
        }
        self.edit_page(page, |items| items.push(Item::Widget { widget, size }))
    }

    /// (v2) Adds app `app`'s widget `name` of [`crate::widget::CATALOGUE`] at the end of page `page`.
    ///
    /// # Errors
    /// [`Error::Invalid`] for a widget or size the catalogue doesn't have, [`Error::Full`], [`Error::NotFound`].
    pub fn add_app_widget(
        &mut self,
        app: &str,
        name: &str,
        size: Size,
        page: usize,
    ) -> Result<(), Error> {
        if !app_widget_ok(app, name, size) {
            return Err(Error::Invalid);
        }
        let mut next = self.clone();
        next.edit_page(page, |items| {
            items.push(Item::AppWidget {
                app: String::from(app),
                name: String::from(name),
                size,
            });
        })?;
        next.check_size()?;
        *self = next;
        Ok(())
    }

    /// Removes item `index` of page `page`.
    ///
    /// # Errors
    /// [`Error::NotFound`].
    pub fn remove(&mut self, page: usize, index: usize) -> Result<(), Error> {
        let p = self.pages.get_mut(page).ok_or(Error::NotFound)?;
        if index >= p.items.len() {
            return Err(Error::NotFound);
        }
        p.items.remove(index);
        Ok(())
    }

    /// Moves item `index` of page `from` to position `to_index` (clamped to the end) of page `to`; a page past the
    /// last is added first (at most five).
    ///
    /// # Errors
    /// [`Error::Full`] (nothing changes), [`Error::TooManyPages`], [`Error::NotFound`].
    pub fn move_to(
        &mut self,
        from: usize,
        index: usize,
        to: usize,
        to_index: usize,
    ) -> Result<(), Error> {
        let item = self
            .pages
            .get(from)
            .and_then(|p| p.items.get(index))
            .cloned()
            .ok_or(Error::NotFound)?;
        if to > self.pages.len() {
            return Err(Error::NotFound);
        }
        let mut next = self.clone();
        if to == next.pages.len() {
            next.add_page()?;
        }
        next.pages[from].items.remove(index);
        let dest = &mut next.pages[to].items;
        dest.insert(to_index.min(dest.len()), item);
        pack(dest)?;
        *self = next;
        Ok(())
    }

    /// Adds an empty page at the end.
    ///
    /// # Errors
    /// [`Error::TooManyPages`] with five already.
    pub fn add_page(&mut self) -> Result<(), Error> {
        if self.pages.len() >= MAX_PAGES {
            return Err(Error::TooManyPages);
        }
        self.pages.push(Page::default());
        Ok(())
    }

    /// Drops empty pages at the end, keeping the first (Done and Back in the editor).
    pub fn drop_empty_pages(&mut self) {
        while self.pages.len() > 1 && self.pages.last().is_some_and(|p| p.items.is_empty()) {
            self.pages.pop();
        }
    }

    fn check_size(&self) -> Result<(), Error> {
        self.encode().map(drop)
    }

    /// The `home.layout` bytes.
    ///
    /// # Errors
    /// [`Error::Invalid`] past [`MAX_KEYS`] keys or [`MAX_BYTES`] bytes, or with a bad key.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut keys: Vec<String> = Vec::new();
        let mut key_ref = |k: &str| -> Result<u8, Error> {
            if !valid_key(k) {
                return Err(Error::Invalid);
            }
            let i = match keys.iter().position(|x| *x == k) {
                Some(i) => i,
                None => {
                    keys.push(String::from(k));
                    keys.len() - 1
                }
            };
            if i >= MAX_KEYS {
                return Err(Error::Invalid);
            }
            Ok(i as u8 + 1)
        };
        if self.pages.is_empty() || self.pages.len() > MAX_PAGES {
            return Err(Error::Invalid);
        }
        let mut dock = [0u8; DOCK];
        for (d, k) in dock.iter_mut().zip(&self.dock) {
            if let Some(k) = k {
                *d = key_ref(k)?;
            }
        }
        let mut pages = Vec::new();
        for p in &self.pages {
            pack(&p.items)?;
            pages.push(p.items.len() as u8);
            for i in &p.items {
                match i {
                    Item::Shortcut(k) => pages.extend([0, key_ref(k)?]),
                    Item::Widget { widget, size } => {
                        pages.extend([1, key_ref(entry(*widget).1)?, *widget as u8, *size as u8]);
                    }
                    Item::AppWidget { app, name, size } => {
                        if !app_widget_ok(app, name, *size) {
                            return Err(Error::Invalid);
                        }
                        pages.extend([2, key_ref(app)?, name.len() as u8]);
                        pages.extend_from_slice(name.as_bytes());
                        pages.push(*size as u8);
                    }
                }
            }
        }
        // an app's widget, or a built-in at a size a version-1 shell doesn't know (its catalogue had the first only)
        let v2 = self.pages.iter().any(|p| {
            p.items.iter().any(|i| match i {
                Item::AppWidget { .. } => true,
                Item::Widget { widget, size } => *size != entry(*widget).2[0],
                Item::Shortcut(_) => false,
            })
        });
        let version = if v2 { VERSION } else { VERSION_1 };
        let mut out = alloc::vec![version, self.pages.len() as u8, keys.len() as u8];
        for k in &keys {
            out.push(k.len() as u8);
            out.extend_from_slice(k.as_bytes());
        }
        out.extend_from_slice(&dock);
        out.extend(pages);
        if out.len() > MAX_BYTES {
            return Err(Error::Invalid);
        }
        Ok(out)
    }

    /// The layout `bytes` hold, or `None` when they aren't a valid version-1 or version-2 layout.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let mut b = bytes.iter().copied();
        let mut next = || b.next();
        let version = next()?;
        if version != VERSION && version != VERSION_1 {
            return None;
        }
        let (page_count, key_count) = (usize::from(next()?), usize::from(next()?));
        if !(1..=MAX_PAGES).contains(&page_count) || key_count > MAX_KEYS {
            return None;
        }
        let mut keys: Vec<String> = Vec::with_capacity(key_count);
        for _ in 0..key_count {
            let len = usize::from(next()?);
            let k: Vec<u8> = (0..len).map(|_| next()).collect::<Option<_>>()?;
            let k = String::from_utf8(k).ok()?;
            if !valid_key(&k) || keys.contains(&k) {
                return None;
            }
            keys.push(k);
        }
        // W0d: the old apps' screen (`legacy`) is Classic apps (`classic`) now; saved layouts move with it
        let key = |r: u8| {
            let k = keys.get(usize::from(r).checked_sub(1)?)?;
            Some(if k == crate::apps::LEGACY {
                String::from(crate::apps::CLASSIC)
            } else {
                k.clone()
            })
        };
        let mut dock: [Option<String>; DOCK] = Default::default();
        for d in &mut dock {
            let r = next()?;
            if r != 0 {
                *d = Some(key(r)?);
            }
        }
        let mut pages = Vec::with_capacity(page_count);
        for _ in 0..page_count {
            let n = usize::from(next()?);
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                items.push(match next()? {
                    0 => Item::Shortcut(key(next()?)?),
                    1 => {
                        let app = key(next()?)?;
                        let widget = Widget::from_u8(next()?)?;
                        let size = Size::from_u8(next()?)?;
                        let e = entry(widget);
                        if e.1 != app || !e.2.contains(&size) {
                            return None;
                        }
                        Item::Widget { widget, size }
                    }
                    2 if version == VERSION => {
                        let app = key(next()?)?;
                        let len = usize::from(next()?);
                        let name: Vec<u8> = (0..len).map(|_| next()).collect::<Option<_>>()?;
                        let name = String::from_utf8(name).ok()?;
                        let size = Size::from_u8(next()?)?;
                        if !app_widget_ok(&app, &name, size) {
                            return None;
                        }
                        Item::AppWidget { app, name, size }
                    }
                    _ => return None,
                });
            }
            pack(&items).ok()?;
            pages.push(Page { items });
        }
        if next().is_some() {
            return None;
        }
        Some(Self { pages, dock })
    }

    /// The stored layout, or the default for empty or damaged bytes.
    #[must_use]
    pub fn load(bytes: &[u8]) -> Self {
        Self::decode(bytes).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn s(k: &str) -> Item {
        Item::Shortcut(k.into())
    }
    fn at(p: &[Placed]) -> Vec<(u8, u8)> {
        p.iter().map(|p| (p.col, p.row)).collect()
    }

    /// W0d: the old apps' screen key `legacy` is Classic apps' `classic` now; a saved shortcut or dock slot moves
    /// with it on load (nothing else changes).
    #[test]
    fn a_saved_legacy_key_loads_as_classic() {
        let mut l = Layout::default();
        l.pages[1].items = vec![s("files"), s("legacy")];
        l.dock[3] = Some("legacy".into());
        let loaded = Layout::load(&l.encode().unwrap());
        assert_eq!(loaded.pages[1].items, [s("files"), s("classic")]);
        assert_eq!(loaded.dock[3].as_deref(), Some("classic"));
        assert_eq!(loaded.pages[0], l.pages[0]);
        assert_eq!(loaded.dock[..3], l.dock[..3]);
    }

    #[test]
    fn packs_in_order_row_by_row() {
        let sq = Item::Widget {
            widget: Widget::Clock,
            size: Size::Square,
        };
        let wide = Item::Widget {
            widget: Widget::Media,
            size: Size::Wide,
        };
        // two shortcuts then a 2×2 beside them; the next shortcut under the shortcuts, a 4×1 on its own row
        let p = pack(&[s("a"), s("b"), sq.clone(), s("c"), wide.clone(), s("d")]).unwrap();
        assert_eq!(at(&p), [(0, 0), (1, 0), (2, 0), (0, 1), (0, 2), (0, 3)]);
        // a 4×1 after a shortcut starts a new row: the gap stays (reading order is stored order)
        assert_eq!(
            at(&pack(&[s("a"), wide, s("b")]).unwrap()),
            [(0, 0), (0, 1), (0, 2)]
        );
        // two 2×2 side by side
        assert_eq!(at(&pack(&[sq.clone(), sq]).unwrap()), [(0, 0), (2, 0)]);
        // 24 shortcuts fill a page, a 25th doesn't fit
        let full: Vec<Item> = (0..24).map(|_| s("a")).collect();
        assert!(pack(&full).is_ok());
        let over: Vec<Item> = (0..25).map(|_| s("a")).collect();
        assert_eq!(pack(&over), Err(Error::Full));
    }

    #[test]
    fn the_default_fills_page_zero_and_round_trips() {
        let d = Layout::default();
        let p = pack(&d.pages[0].items).unwrap();
        assert_eq!(at(&p), [(0, 0), (0, 2), (0, 4), (0, 5)]);
        assert_eq!(d.pages.len(), 3);
        let bytes = d.encode().unwrap();
        assert_eq!(Layout::decode(&bytes), Some(d.clone()));
        assert_eq!(Layout::load(&[]), d);
    }

    #[test]
    fn edits_fit_before_they_change_anything() {
        let mut l = Layout::default();
        // page 0 is full: pinning there fails and changes nothing; Add to home takes the next page with room
        assert_eq!(l.pin("files", 0), Err(Error::Full));
        assert_eq!(l, Layout::default());
        assert_eq!(l.pin_anywhere("files", 0), Ok(1));
        assert_eq!(l.pages[1].items, [s("files")]);
        // a widget size the catalogue doesn't have
        assert_eq!(
            l.add_widget(Widget::Clock, Size::Wide, 1),
            Err(Error::Invalid)
        );
        assert_eq!(l.add_widget(Widget::Media, Size::Wide, 1), Ok(()));
        // move the shortcut after the widget, then to page 0 (full: refused, nothing moves)
        l.move_to(1, 0, 1, 9).unwrap();
        assert_eq!(l.pages[1].items[1], s("files"));
        assert_eq!(l.move_to(1, 1, 0, 0), Err(Error::Full));
        assert_eq!(l.pages[1].items.len(), 2);
        // unpin and remove
        l.unpin("files");
        l.remove(1, 0).unwrap();
        assert!(l.pages[1].items.is_empty());
        assert_eq!(l.remove(1, 0), Err(Error::NotFound));
        // the dock: replace, empty
        l.pin_dock(Some("files"), 3).unwrap();
        l.pin_dock(None, 0).unwrap();
        assert_eq!(l.dock[0], None);
        assert_eq!(l.dock[3].as_deref(), Some("files"));
        assert_eq!(l.pin_dock(Some("x"), 4), Err(Error::NotFound));
    }

    #[test]
    fn five_pages_at_most_and_empty_ones_trimmed() {
        let mut l = Layout::default();
        l.pin("files", 2).unwrap();
        l.add_page().unwrap();
        l.add_page().unwrap();
        assert_eq!(l.add_page(), Err(Error::TooManyPages));
        // moving to a sixth page is refused; to the fifth (existing) is fine
        assert_eq!(l.move_to(2, 0, 5, 0), Err(Error::TooManyPages));
        l.move_to(2, 0, 4, 0).unwrap();
        l.drop_empty_pages();
        assert_eq!(l.pages.len(), 5);
        l.remove(4, 0).unwrap();
        l.drop_empty_pages();
        assert_eq!(l.pages.len(), 1);
        // moving past the last page adds one
        l.pages[0].items.truncate(1);
        l.pin("files", 0).unwrap();
        l.move_to(0, 1, 1, 0).unwrap();
        assert_eq!(l.pages.len(), 2);
        let bytes = l.encode().unwrap();
        assert_eq!(Layout::decode(&bytes), Some(l));
    }

    #[test]
    fn damaged_bytes_fall_back_to_the_default() {
        let good = Layout::default().encode().unwrap();
        // every truncation, a trailing byte, another version
        for n in 0..good.len() {
            assert_eq!(Layout::decode(&good[..n]), None, "{n}");
        }
        let mut long = good.clone();
        long.push(0);
        assert_eq!(Layout::decode(&long), None);
        let mut v3 = good.clone();
        v3[0] = 3;
        assert_eq!(Layout::load(&v3), Layout::default());
        // a widget with an app that isn't its catalogue's, an unknown widget, a ref past the keys, zero pages
        let bad = [
            vec![
                1, 1, 1, 5, b'p', b'h', b'o', b'n', b'e', 0, 0, 0, 0, 1, 1, 1, 1, 2,
            ],
            vec![
                1, 1, 1, 5, b'c', b'l', b'o', b'c', b'k', 0, 0, 0, 0, 1, 1, 1, 9, 2,
            ],
            vec![1, 1, 1, 1, b'a', 0, 0, 0, 0, 1, 0, 2],
            vec![1, 0, 0, 0, 0, 0, 0],
            // a duplicate key
            vec![1, 1, 2, 1, b'a', 1, b'a', 0, 0, 0, 0, 0],
        ];
        for b in bad {
            assert_eq!(Layout::decode(&b), None, "{b:?}");
        }
        // the clock widget with its own key decodes
        let ok = vec![
            1, 1, 1, 5, b'c', b'l', b'o', b'c', b'k', 1, 0, 0, 0, 1, 1, 1, 1, 2,
        ];
        assert!(Layout::decode(&ok).is_some());
        // a page that doesn't pack: 25 shortcuts
        let mut full = vec![1, 1, 1, 1, b'a', 0, 0, 0, 0, 25];
        full.extend((0..25).flat_map(|_| [0, 1]));
        assert_eq!(Layout::decode(&full), None);
    }

    #[test]
    fn version_2_carries_app_widgets_and_reads_version_1() {
        let v1 = Layout::default().encode().unwrap();
        assert_eq!(
            v1[0], VERSION_1,
            "no app widget: still written as version 1"
        );
        // a version-1 layout read as it was; the same bytes marked version 2 too
        let mut as_v2 = v1.clone();
        as_v2[0] = VERSION;
        assert_eq!(Layout::decode(&as_v2), Layout::decode(&v1));
        let mut l = Layout::default();
        l.add_app_widget("clock", "next", Size::Square, 1).unwrap();
        l.pin("files", 1).unwrap();
        let b = l.encode().unwrap();
        assert_eq!(b[0], VERSION);
        let d = Layout::decode(&b).unwrap();
        assert_eq!(d, l);
        assert_eq!(d.pages[1].items[0].app(), "clock");
        assert_eq!(at(&pack(&d.pages[1].items).unwrap()), [(0, 0), (2, 0)]);
        // not in the catalogue, or a size it doesn't have: refused, nothing changes
        assert_eq!(
            l.add_app_widget("clock", "next", Size::Tall, 1),
            Err(Error::Invalid)
        );
        assert_eq!(
            l.add_app_widget("files", "next", Size::Square, 1),
            Err(Error::Invalid)
        );
        assert_eq!(l, d);
        // every truncation; an app widget in a version-1 layout; a name or size the catalogue doesn't have
        for n in 0..b.len() {
            assert_eq!(Layout::decode(&b[..n]), None, "{n}");
        }
        let mut old = b.clone();
        old[0] = VERSION_1;
        assert_eq!(Layout::decode(&old), None);
        let size_at = b.len() - 4; // [.., 2, ref, 4, "next", size, 0, ref(files)], then page 2: [0]
        assert_eq!(b[size_at], Size::Square as u8);
        let mut tall = b.clone();
        tall[size_at] = Size::Tall as u8;
        assert_eq!(Layout::decode(&tall), None);
        let mut other = b.clone();
        other[size_at - 1] = b'x';
        assert_eq!(Layout::decode(&other), None);
        // an item past the kinds
        let mut kind = b.clone();
        kind[size_at - 7] = 3;
        assert_eq!(Layout::decode(&kind), None);
    }

    #[test]
    fn two_by_two_built_ins_pair_up_and_write_version_2() {
        let mut l = Layout::default();
        for w in [
            Widget::Clock,
            Widget::Agenda,
            Widget::Favourites,
            Widget::Media,
        ] {
            l.add_widget(w, Size::Square, 1).unwrap();
        }
        assert_eq!(
            at(&pack(&l.pages[1].items).unwrap()),
            [(0, 0), (2, 0), (0, 2), (2, 2)]
        );
        let b = l.encode().unwrap();
        assert_eq!(b[0], VERSION, "a 2 × 2 built-in is unknown to version 1");
        assert_eq!(Layout::decode(&b), Some(l));
        assert_eq!(
            Layout::default().add_widget(Widget::Media, Size::Tall, 1),
            Err(Error::Invalid)
        );
    }

    #[test]
    fn the_lane_and_drops() {
        let mut l = Layout::default();
        l.pin("files", 1).unwrap();
        l.pin("clock", 2).unwrap();
        let lane = pack_lane(&l);
        // page 0's six rows, then the two shortcuts side by side on row 6
        let rows: Vec<(usize, usize, u8, u8)> = lane
            .iter()
            .map(|(p, i, at)| (*p, *i, at.col, at.row))
            .collect();
        assert_eq!(
            rows,
            [
                (0, 0, 0, 0),
                (0, 1, 0, 2),
                (0, 2, 0, 4),
                (0, 3, 0, 5),
                (1, 0, 0, 6),
                (2, 0, 1, 6)
            ]
        );
        // dropping the Music widget (index 3) over row 0 puts it first; over an empty spot, last
        assert_eq!(drop_index(&l.pages[0].items, 3, 0, 0), 0);
        assert_eq!(drop_index(&l.pages[0].items, 0, 2, 5), 3);
        let items = [s("a"), s("b"), s("c")];
        assert_eq!(drop_index(&items, 0, 1, 0), 1); // a over c's cell (b, c moved up): before c
        assert_eq!(drop_index(&items, 2, 0, 0), 0);
    }

    #[test]
    fn keys_are_bounded() {
        let mut l = Layout::default();
        assert_eq!(l.pin("", 1), Err(Error::Invalid));
        assert_eq!(l.pin("seventeen-chars-x", 1), Err(Error::Invalid));
        assert_eq!(l.pin("sp ace", 1), Err(Error::Invalid));
        // 48 distinct keys at most (the default uses 8)
        let mut n = 0;
        'fill: for p in 1..3 {
            for _ in 0..24 {
                if l.pin(&alloc::format!("k{n}"), p).is_err() {
                    break 'fill;
                }
                n += 1;
            }
        }
        assert_eq!(n, 40);
        assert!(l.encode().unwrap().len() <= MAX_BYTES);
        l.add_page().unwrap();
        assert_eq!(l.pin("one-too-many", 3), Err(Error::Invalid));
        assert!(l.pages[3].items.is_empty());
    }
}

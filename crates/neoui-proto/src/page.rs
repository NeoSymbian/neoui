// SPDX-License-Identifier: Apache-2.0
//! What an app tells the shell about its screen (protocol 1.8, docs `neoui-kit.md`): the page it shows ([`Page`]:
//! its name, whether Back stays inside it, full screen), a dialog ([`Query`]), a toast ([`Note`]) and an entry in the
//! shade ([`Notification`]). Since 1.8 the app draws its own header and footer with the kit; the shell draws the
//! status bar, the key row or gesture band, dialogs, toasts and the shade.
//!
//! Wire format (all lengths u8, strings UTF-8): `Page` = `[flags, 0, 0, 0, 0]` then the title (a 1.7 page, with a
//! primary action, options and dots after the header, still decodes; those parts are ignored). `Query` = `[flags,
//! buttons n]` then the text and the buttons. `Note` = the text then the action. `Notification` = `[id u32 LE, page u16
//! LE]` then the title and the text; since 1.9 `[actions n]` and the action labels may follow (a 1.8 one still
//! decodes, with none).

use alloc::string::String;
use alloc::vec::Vec;

use crate::ipc::TITLE_MAX;

/// The longest dialog button or toast action, in bytes.
pub const LABEL_MAX: usize = 32;
/// The most options a 1.7 page had (still read, then ignored).
const OPTIONS_MAX: usize = 12;
/// The longest dialog or notification text, in bytes.
pub const TEXT_MAX: usize = 160;
/// The most dialog buttons.
pub const BUTTONS_MAX: usize = 3;
/// The most page dots a 1.7 page had.
const DOTS_MAX: u8 = 8;
/// The largest encoded [`Page`] (a 1.7 page with everything).
pub const PAGE_MAX: usize = 5 + 1 + TITLE_MAX + 1 + LABEL_MAX + OPTIONS_MAX * (1 + LABEL_MAX);
/// The largest encoded [`Query`].
pub const QUERY_MAX: usize = 2 + 1 + TEXT_MAX + BUTTONS_MAX * (1 + LABEL_MAX);
/// The longest toast text, in bytes.
pub const NOTE_TEXT_MAX: usize = 80;
/// The largest encoded [`Note`].
pub const NOTE_MAX: usize = 1 + NOTE_TEXT_MAX + 1 + LABEL_MAX;
/// The most actions on a [`Notification`] (1.9).
pub const NOTIFY_ACTIONS_MAX: usize = 2;
/// The longest notification action label, in bytes.
pub const ACTION_MAX: usize = 16;
/// The longest notification avatar's initials and time text, in bytes.
pub const INITIALS_MAX: usize = 8;
pub const TIME_MAX: usize = 16;
/// The largest encoded [`Notification`].
pub const NOTIFICATION_MAX: usize = 6
    + 1
    + TITLE_MAX
    + 1
    + TEXT_MAX
    + 1
    + NOTIFY_ACTIONS_MAX * (1 + ACTION_MAX)
    + 1
    + INITIALS_MAX
    + 4
    + 1
    + TIME_MAX;

/// What an app shows now. Send it again whenever any of it changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    /// The page's name: the app's card in Open apps (empty: the app's name). The header draws it; the status bar
    /// shows no title (Navigation v2 D16).
    pub title: String,
    /// Back stays inside the app (a kit layer open, a page to go up to, a form to cancel): the app gets
    /// [`crate::ipc::AppEvent::Back`]. Otherwise Back leaves the app for where the user came from.
    pub can_back: bool,
    /// Hide the status bar and the key row or band (a call, the camera, a video).
    pub full: bool,
}

/// A shell dialog. The answer comes back as [`crate::ipc::AppEvent::Answer`] with the button's index; Back answers
/// the last button.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    /// 1 to [`BUTTONS_MAX`], the confirming one first ("Delete", "Cancel").
    pub buttons: Vec<String>,
    /// The first button is red.
    pub destructive: bool,
}

/// A toast above the key row or band for 3 s: text and an optional action ("Undo"). A tap on the action sends
/// [`crate::ipc::AppEvent::NoteAction`]. Empty text takes the toast away.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    /// At most [`NOTE_TEXT_MAX`] bytes.
    pub text: String,
    /// At most [`LABEL_MAX`] bytes; empty = no action.
    pub action: String,
}

/// An entry in the shade. A tap brings the app to the front with [`crate::ipc::AppEvent::Page`] `{ page }` and then
/// [`crate::ipc::AppEvent::Notified`] `{ id, action: 0 }`; a tap on action `i` (1.9) brings it with `{ id, action: i +
/// 1 }`. Posting the same `id` again replaces it; an empty title removes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notification {
    /// The app's own numbering.
    pub id: u32,
    /// At most [`TITLE_MAX`] bytes.
    pub title: String,
    /// At most [`TEXT_MAX`] bytes; a newline splits it into two lines (the earlier message dim, the newest under it).
    pub text: String,
    /// The page the app opens on a tap.
    pub page: u16,
    /// Up to [`NOTIFY_ACTIONS_MAX`] buttons under the text ("Reply", "Call"), each at most [`ACTION_MAX`] bytes; the
    /// first is drawn as the main one.
    pub actions: Vec<String>,
    /// An avatar instead of the bell: initials (at most [`INITIALS_MAX`] bytes; "" = the bell) on `tint`
    /// (0xRRGGBB).
    pub initials: String,
    pub tint: u32,
    /// When, as the app writes it ("10:05"; at most [`TIME_MAX`] bytes; "" = none).
    pub time: String,
}

/// A [`Page`] or [`Query`] that is malformed or over a limit (`KErrArgument`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageError;

const CAN_BACK: u8 = 1;
const FULL: u8 = 4;
const PRIMARY: u8 = 8;
const DOTS: u8 = 16;
const DESTRUCTIVE: u8 = 1;

fn put(out: &mut Vec<u8>, s: &str, max: usize) -> Result<(), PageError> {
    if s.len() > max {
        return Err(PageError);
    }
    out.push(s.len() as u8);
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, PageError> {
        let (&b, rest) = self.0.split_first().ok_or(PageError)?;
        self.0 = rest;
        Ok(b)
    }
    fn text(&mut self, max: usize) -> Result<String, PageError> {
        let n = usize::from(self.byte()?);
        if n > max || n > self.0.len() {
            return Err(PageError);
        }
        let (s, rest) = self.0.split_at(n);
        self.0 = rest;
        core::str::from_utf8(s)
            .map(String::from)
            .map_err(|_| PageError)
    }
}

fn dots_ok((n, at): (u8, u8)) -> bool {
    n <= DOTS_MAX && at < n
}

impl Page {
    pub fn encode(&self) -> Result<Vec<u8>, PageError> {
        let flags = (self.can_back as u8 * CAN_BACK) | (self.full as u8 * FULL);
        let mut out = alloc::vec![flags, 0, 0, 0, 0];
        put(&mut out, &self.title, TITLE_MAX)?;
        Ok(out)
    }

    /// A 1.8 page, or a 1.7 one (its primary action, Cancel, options and dots are read and checked, then dropped).
    pub fn decode(b: &[u8]) -> Result<Self, PageError> {
        if b.len() > PAGE_MAX {
            return Err(PageError);
        }
        let mut r = Reader(b);
        let flags = r.byte()?;
        let icon = r.byte()?;
        let dots = (r.byte()?, r.byte()?);
        let n = usize::from(r.byte()?);
        // 1.7's TOOL_* icons were 0..=6
        if flags >> 5 != 0 || icon > 6 || n > OPTIONS_MAX {
            return Err(PageError);
        }
        if flags & DOTS != 0 && !dots_ok(dots) {
            return Err(PageError);
        }
        let title = r.text(TITLE_MAX)?;
        if flags & PRIMARY != 0 {
            r.text(LABEL_MAX)?;
        }
        for _ in 0..n {
            r.text(LABEL_MAX)?;
        }
        if !r.0.is_empty() {
            return Err(PageError);
        }
        Ok(Page {
            title,
            can_back: flags & CAN_BACK != 0,
            full: flags & FULL != 0,
        })
    }
}

impl Note {
    pub fn encode(&self) -> Result<Vec<u8>, PageError> {
        let mut out = Vec::new();
        put(&mut out, &self.text, NOTE_TEXT_MAX)?;
        put(&mut out, &self.action, LABEL_MAX)?;
        Ok(out)
    }

    pub fn decode(b: &[u8]) -> Result<Self, PageError> {
        let mut r = Reader(b);
        let note = Note {
            text: r.text(NOTE_TEXT_MAX)?,
            action: r.text(LABEL_MAX)?,
        };
        if !r.0.is_empty() {
            return Err(PageError);
        }
        Ok(note)
    }
}

impl Notification {
    pub fn encode(&self) -> Result<Vec<u8>, PageError> {
        let mut out = Vec::with_capacity(NOTIFICATION_MAX);
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.page.to_le_bytes());
        put(&mut out, &self.title, TITLE_MAX)?;
        put(&mut out, &self.text, TEXT_MAX)?;
        let extra = !self.initials.is_empty() || !self.time.is_empty() || self.tint != 0;
        if !self.actions.is_empty() || extra {
            if self.actions.len() > NOTIFY_ACTIONS_MAX {
                return Err(PageError);
            }
            out.push(self.actions.len() as u8);
            for a in &self.actions {
                put(&mut out, a, ACTION_MAX)?;
            }
        }
        if extra {
            put(&mut out, &self.initials, INITIALS_MAX)?;
            out.extend_from_slice(&self.tint.to_le_bytes());
            put(&mut out, &self.time, TIME_MAX)?;
        }
        Ok(out)
    }

    pub fn decode(b: &[u8]) -> Result<Self, PageError> {
        if b.len() < 6 || b.len() > NOTIFICATION_MAX {
            return Err(PageError);
        }
        let id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let page = u16::from_le_bytes([b[4], b[5]]);
        let mut r = Reader(&b[6..]);
        let mut n = Notification {
            id,
            title: r.text(TITLE_MAX)?,
            text: r.text(TEXT_MAX)?,
            page,
            ..Notification::default()
        };
        if !r.0.is_empty() {
            let count = usize::from(r.byte()?);
            if count > NOTIFY_ACTIONS_MAX {
                return Err(PageError);
            }
            for _ in 0..count {
                n.actions.push(r.text(ACTION_MAX)?);
            }
        }
        if !r.0.is_empty() {
            n.initials = r.text(INITIALS_MAX)?;
            let t = r.0.get(..4).ok_or(PageError)?;
            n.tint = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
            r.0 = &r.0[4..];
            n.time = r.text(TIME_MAX)?;
        }
        if !r.0.is_empty() {
            return Err(PageError);
        }
        Ok(n)
    }
}

impl Query {
    pub fn encode(&self) -> Result<Vec<u8>, PageError> {
        if !(1..=BUTTONS_MAX).contains(&self.buttons.len()) {
            return Err(PageError);
        }
        let mut out = alloc::vec![
            self.destructive as u8 * DESTRUCTIVE,
            self.buttons.len() as u8
        ];
        put(&mut out, &self.text, TEXT_MAX)?;
        for b in &self.buttons {
            put(&mut out, b, LABEL_MAX)?;
        }
        Ok(out)
    }

    pub fn decode(b: &[u8]) -> Result<Self, PageError> {
        if b.len() > QUERY_MAX {
            return Err(PageError);
        }
        let mut r = Reader(b);
        let flags = r.byte()?;
        let n = usize::from(r.byte()?);
        if flags & !DESTRUCTIVE != 0 || !(1..=BUTTONS_MAX).contains(&n) {
            return Err(PageError);
        }
        let text = r.text(TEXT_MAX)?;
        let buttons = (0..n)
            .map(|_| r.text(LABEL_MAX))
            .collect::<Result<_, _>>()?;
        if !r.0.is_empty() {
            return Err(PageError);
        }
        Ok(Query {
            text,
            buttons,
            destructive: flags & DESTRUCTIVE != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn s(t: &str) -> String {
        t.to_string()
    }

    /// A 1.7 page with a primary action, Cancel, two options and dots, as Contacts sent it.
    fn page_1_7() -> Vec<u8> {
        let mut b = vec![1 | 2 | 8 | 16, 3, 2, 1, 2];
        for t in ["Contacts", "New", "Mark several", "Import vCards"] {
            b.push(t.len() as u8);
            b.extend_from_slice(t.as_bytes());
        }
        b
    }

    #[test]
    fn pages_round_trip() {
        let pages = [
            Page::default(),
            Page {
                title: s("Ԥ Мария"),
                can_back: true,
                ..Page::default()
            },
            Page {
                title: "x".repeat(TITLE_MAX),
                can_back: true,
                full: true,
            },
        ];
        for p in pages {
            let b = p.encode().unwrap();
            assert!(b.len() <= PAGE_MAX);
            assert_eq!(Page::decode(&b), Ok(p));
        }
    }

    #[test]
    fn a_1_7_page_still_decodes_without_its_actions() {
        assert_eq!(
            Page::decode(&page_1_7()),
            Ok(Page {
                title: s("Contacts"),
                can_back: true,
                full: false,
            })
        );
        // the 1.8 bytes are a 1.7 page with no actions: an old shell reads a new app the same way
        let p = Page {
            title: s("Files"),
            can_back: true,
            full: true,
        };
        assert_eq!(p.encode().unwrap()[..5], [1 | 4, 0, 0, 0, 0]);
    }

    #[test]
    fn pages_over_a_limit_are_refused() {
        let p = Page {
            title: "x".repeat(TITLE_MAX + 1),
            ..Page::default()
        };
        assert_eq!(p.encode(), Err(PageError));
    }

    #[test]
    fn malformed_pages_are_refused() {
        let good = page_1_7();
        // every truncation, a trailing byte, an unknown flag, a 1.7 icon past TOOL_DELETE, bad UTF-8, a bad dot, too
        // many options, too long
        for n in 0..good.len() {
            assert_eq!(Page::decode(&good[..n]), Err(PageError));
        }
        let mut b = good.clone();
        b.push(0);
        assert_eq!(Page::decode(&b), Err(PageError));
        let mut b = good.clone();
        b[0] = 32;
        assert_eq!(Page::decode(&b), Err(PageError));
        let mut b = good.clone();
        b[1] = 7;
        assert_eq!(Page::decode(&b), Err(PageError));
        let mut b = good.clone();
        b[6] = 0xFF;
        assert_eq!(Page::decode(&b), Err(PageError));
        let mut b = good.clone();
        b[2] = 2;
        b[3] = 2;
        assert_eq!(Page::decode(&b), Err(PageError));
        let mut b = good.clone();
        b[4] = OPTIONS_MAX as u8 + 1;
        assert_eq!(Page::decode(&b), Err(PageError));
        assert_eq!(Page::decode(&[0; PAGE_MAX + 1]), Err(PageError));
    }

    #[test]
    fn notes_round_trip_and_check_limits() {
        for n in [
            Note::default(),
            Note {
                text: s("Alarm deleted"),
                action: s("Undo"),
            },
            Note {
                text: "t".repeat(NOTE_TEXT_MAX),
                action: "a".repeat(LABEL_MAX),
            },
        ] {
            let b = n.encode().unwrap();
            assert!(b.len() <= NOTE_MAX);
            assert_eq!(Note::decode(&b), Ok(n));
        }
        let long = Note {
            text: "t".repeat(NOTE_TEXT_MAX + 1),
            action: s(""),
        };
        assert_eq!(long.encode(), Err(PageError));
        let good = Note {
            text: s("ok"),
            action: s("Undo"),
        }
        .encode()
        .unwrap();
        for n in 0..good.len() {
            assert_eq!(Note::decode(&good[..n]), Err(PageError));
        }
        let mut b = good;
        b.push(0);
        assert_eq!(Note::decode(&b), Err(PageError));
    }

    #[test]
    fn notifications_round_trip_and_check_limits() {
        let n = Notification {
            id: 0xE000_5001,
            title: s("Ava Lee"),
            text: s("Running late, start without me"),
            page: 6,
            ..Notification::default()
        };
        let b = n.encode().unwrap();
        assert_eq!(Notification::decode(&b), Ok(n.clone()));
        // a 1.8 notification has no actions byte; a 1.9 one with actions adds them after the text
        let with = Notification {
            actions: vec![s("Reply"), s("Call")],
            ..n.clone()
        };
        let wb = with.encode().unwrap();
        assert_eq!(&wb[..b.len()], &b[..]);
        assert_eq!(Notification::decode(&wb), Ok(with));
        // the avatar and time come after the actions, which may be none
        let avatar = Notification {
            initials: s("AL"),
            tint: 0x22b46a,
            time: s("10:05"),
            text: s("Perfect. I'll book a table.\nRunning late, see you at 7?"),
            ..n.clone()
        };
        assert_eq!(Notification::decode(&avatar.encode().unwrap()), Ok(avatar));
        let three = Notification {
            actions: vec![s("a"), s("b"), s("c")],
            ..n.clone()
        };
        assert_eq!(three.encode(), Err(PageError));
        let long = Notification {
            actions: vec!["z".repeat(ACTION_MAX + 1)],
            ..n
        };
        assert_eq!(long.encode(), Err(PageError));
        let big = Notification {
            id: u32::MAX,
            title: "x".repeat(TITLE_MAX),
            text: "y".repeat(TEXT_MAX),
            page: u16::MAX,
            actions: vec!["a".repeat(ACTION_MAX), "b".repeat(ACTION_MAX)],
            initials: "i".repeat(INITIALS_MAX),
            tint: 0x22b46a,
            time: "t".repeat(TIME_MAX),
        };
        assert_eq!(big.encode().unwrap().len(), NOTIFICATION_MAX);
        assert_eq!(Notification::decode(&big.encode().unwrap()), Ok(big));
        let over = Notification {
            text: "y".repeat(TEXT_MAX + 1),
            ..Notification::default()
        };
        assert_eq!(over.encode(), Err(PageError));
        let good = Notification::default().encode().unwrap();
        for n in 0..good.len() {
            assert_eq!(Notification::decode(&good[..n]), Err(PageError));
        }
    }

    #[test]
    fn queries_round_trip_and_check_limits() {
        let q = Query {
            text: s("Delete Ava Lindqvist?"),
            buttons: vec![s("Delete"), s("Cancel")],
            destructive: true,
        };
        assert_eq!(Query::decode(&q.encode().unwrap()), Ok(q));
        let big = Query {
            text: "t".repeat(TEXT_MAX),
            buttons: vec!["b".repeat(LABEL_MAX); BUTTONS_MAX],
            destructive: false,
        };
        assert_eq!(big.encode().unwrap().len(), QUERY_MAX);
        assert_eq!(Query::decode(&big.encode().unwrap()), Ok(big));
        for bad in [
            Query {
                text: s("?"),
                buttons: vec![],
                destructive: false,
            },
            Query {
                text: s("?"),
                buttons: vec![s("b"); BUTTONS_MAX + 1],
                destructive: false,
            },
            Query {
                text: "t".repeat(TEXT_MAX + 1),
                buttons: vec![s("OK")],
                destructive: false,
            },
        ] {
            assert_eq!(bad.encode(), Err(PageError));
        }
        let good = Query {
            text: s("?"),
            buttons: vec![s("OK")],
            destructive: false,
        }
        .encode()
        .unwrap();
        for n in 0..good.len() {
            assert_eq!(Query::decode(&good[..n]), Err(PageError));
        }
        let mut b = good.clone();
        b[0] = 2;
        assert_eq!(Query::decode(&b), Err(PageError));
        let mut b = good;
        b[1] = 0;
        assert_eq!(Query::decode(&b), Err(PageError));
    }
}

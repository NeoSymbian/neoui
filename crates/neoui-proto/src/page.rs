// SPDX-License-Identifier: Apache-2.0
//! What an app tells the shell about its screen (protocol 1.8, docs `neoui-shell/kit.md`): the page it shows ([`Page`]:
//! its name, whether Back stays inside it, full screen), a dialog ([`Query`]), a toast ([`Note`]) and an entry in the
//! shade ([`Notification`]). Since 1.8 the app draws its own header and footer with the kit; the shell draws the
//! status bar, the key row or gesture band, dialogs, toasts and the shade.
//!
//! Wire format (all lengths u8, strings UTF-8): `Page` = `[flags, 0, 0, 0, 0]` then the title (a 1.7 page, with a
//! primary action, options and dots after the header, still decodes; those parts are ignored). `Query` = `[flags,
//! buttons n]` then the text and the buttons. `Note` = the text then the action. `Notification` = `[id u32 LE, page u16
//! LE]` then the title and the text; since 1.9 `[actions n]` and the action labels may follow (a 1.8 one still
//! decodes, with none), then the avatar's initials, tint and time; since 1.11 one progress byte (0-100) may end it.

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
/// The most actions on a [`Notification`] (1.9: 2; 1.18: 3).
pub const NOTIFY_ACTIONS_MAX: usize = 3;
/// The longest notification group key (1.18), in bytes.
pub const GROUP_MAX: usize = 16;
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
    + TIME_MAX
    + 1
    + 3
    + 1
    + GROUP_MAX
    + 8
    + NOTIFY_ACTIONS_MAX;

/// [`Notification::flags`] (1.18): an ongoing card (a call, a download) can't be swiped away and sits in Live.
pub const NOTIFY_ONGOING: u8 = 1;
/// Updating the card makes no sound and shows no banner (TP4).
pub const NOTIFY_SILENT_UPDATE: u8 = 2;
/// The text is hidden on the lock screen (TP8).
pub const NOTIFY_PRIVATE: u8 = 4;
const NOTIFY_FLAGS: u8 = NOTIFY_ONGOING | NOTIFY_SILENT_UPDATE | NOTIFY_PRIVATE;

/// [`Notification::style`] (1.18; docs neoui-shell/top-panel.md "Cards").
pub mod style {
    pub const BASIC: u8 = 0;
    pub const BIG_TEXT: u8 = 1;
    pub const PROGRESS: u8 = 2;
    /// One card per conversation: the Conversations section.
    pub const CONVERSATION: u8 = 3;
    pub const CALL: u8 = 4;
    pub const CHRONO: u8 = 5;
    pub(crate) const LAST: u8 = CHRONO;
}

/// The kind of a [`Notification`] action (1.18): a plain button, the inline Reply (TP4; drawn as the main one), one
/// that destroys something (red), or one done in place (Mark read): the app hears it without coming to the front,
/// started hidden if it isn't running.
pub mod action {
    pub const BUTTON: u8 = 0;
    pub const REPLY: u8 = 1;
    pub const DANGER: u8 = 2;
    pub const IN_PLACE: u8 = 3;
    pub(crate) const LAST: u8 = IN_PLACE;
}

/// `progress` written as this when there is none but 1.18's fields follow.
const NO_PROGRESS: u8 = 255;

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
    /// A bar under the text, 0-100 (1.11: a copy in Files); `None` = no bar.
    pub progress: Option<u8>,
    /// 1.18: [`NOTIFY_ONGOING`], [`NOTIFY_SILENT_UPDATE`], [`NOTIFY_PRIVATE`].
    pub flags: u8,
    /// 1.18: one of [`style`].
    pub style: u8,
    /// 1.18: the app's channel (TP4's `CHANNEL`; 0 until then).
    pub channel: u8,
    /// 1.18: cards of one app with the same group (at most [`GROUP_MAX`] bytes; "" = none) stack as one.
    pub group: String,
    /// 1.18: when it happened, unix ms (0 = when posted); the shade writes "now", "2 min", then the clock time.
    pub when: u64,
    /// 1.18: each action's kind ([`action`]); missing ones are buttons.
    pub action_kinds: Vec<u8>,
}

impl Notification {
    /// It can't be swiped away or cleared (a call in progress, a download).
    #[must_use]
    pub fn ongoing(&self) -> bool {
        self.flags & NOTIFY_ONGOING != 0
    }

    /// Action `i`'s kind ([`action`]).
    #[must_use]
    pub fn action_kind(&self, i: usize) -> u8 {
        self.action_kinds.get(i).copied().unwrap_or(action::BUTTON)
    }

    fn has_1_18(&self) -> bool {
        self.flags != 0
            || self.style != 0
            || self.channel != 0
            || !self.group.is_empty()
            || self.when != 0
            || self.action_kinds.iter().any(|k| *k != action::BUTTON)
    }
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
        // kinds belong to actions, whether or not anything else of 1.18 is set
        if self.action_kinds.len() > self.actions.len() {
            return Err(PageError);
        }
        let ext = self.has_1_18();
        let extra = !self.initials.is_empty()
            || !self.time.is_empty()
            || self.tint != 0
            || self.progress.is_some()
            || ext;
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
        if let Some(p) = self.progress {
            if p > 100 {
                return Err(PageError);
            }
            out.push(p);
        } else if ext {
            out.push(NO_PROGRESS);
        }
        if ext {
            if self.flags & !NOTIFY_FLAGS != 0
                || self.style > style::LAST
                || self.action_kinds.len() > self.actions.len()
                || self.action_kinds.iter().any(|k| *k > action::LAST)
            {
                return Err(PageError);
            }
            out.extend([self.flags, self.style, self.channel]);
            put(&mut out, &self.group, GROUP_MAX)?;
            out.extend_from_slice(&self.when.to_le_bytes());
            out.extend((0..self.actions.len()).map(|i| self.action_kind(i)));
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
            let p = r.byte()?;
            if p > 100 && !(p == NO_PROGRESS && r.0.len() > 1) {
                return Err(PageError);
            }
            n.progress = (p <= 100).then_some(p);
        }
        if !r.0.is_empty() {
            n.flags = r.byte()?;
            n.style = r.byte()?;
            n.channel = r.byte()?;
            n.group = r.text(GROUP_MAX)?;
            let w = r.0.get(..8).ok_or(PageError)?;
            n.when = u64::from_le_bytes([w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]);
            r.0 = &r.0[8..];
            n.action_kinds = (0..n.actions.len())
                .map(|_| r.byte())
                .collect::<Result<_, _>>()?;
            if n.flags & !NOTIFY_FLAGS != 0
                || n.style > style::LAST
                || n.action_kinds.iter().any(|k| *k > action::LAST)
            {
                return Err(PageError);
            }
            // a 1.18 record with nothing of 1.18 in it isn't one the encoder writes
            while n.action_kinds.last() == Some(&action::BUTTON) {
                n.action_kinds.pop();
            }
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
        let four = Notification {
            actions: vec![s("a"), s("b"), s("c"), s("d")],
            ..n.clone()
        };
        assert_eq!(four.encode(), Err(PageError));
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
            actions: vec![
                "a".repeat(ACTION_MAX),
                "b".repeat(ACTION_MAX),
                "c".repeat(ACTION_MAX),
            ],
            initials: "i".repeat(INITIALS_MAX),
            tint: 0x22b46a,
            time: "t".repeat(TIME_MAX),
            progress: Some(100),
            flags: NOTIFY_ONGOING | NOTIFY_PRIVATE,
            style: style::CALL,
            channel: 255,
            group: "g".repeat(GROUP_MAX),
            when: u64::MAX,
            action_kinds: vec![action::REPLY, action::BUTTON, action::DANGER],
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
    fn notifications_grow_in_1_18_and_old_records_still_read() {
        let old = Notification {
            id: 7,
            title: s("Ava Lindqvist"),
            text: s("Running late, see you at 7?"),
            actions: vec![s("Reply"), s("Call")],
            initials: s("AL"),
            tint: 0x22b46a,
            ..Notification::default()
        };
        let ob = old.encode().unwrap();
        // nothing of 1.18: the 1.12 record, byte for byte
        assert_eq!(Notification::decode(&ob), Ok(old.clone()));
        let new = Notification {
            style: style::CONVERSATION,
            group: s("thread"),
            when: 1_795_691_635_000,
            actions: vec![s("Reply"), s("Mark read"), s("Call")],
            action_kinds: vec![action::REPLY],
            ..old.clone()
        };
        let nb = new.encode().unwrap();
        assert_eq!(Notification::decode(&nb), Ok(new.clone()));
        // no progress, but 1.18 follows: the bar byte says none
        assert_eq!(Notification::decode(&nb).unwrap().progress, None);
        assert!(!new.ongoing());
        assert_eq!(new.action_kind(0), action::REPLY);
        assert_eq!(new.action_kind(2), action::BUTTON);
        // a bare ongoing card: the actions count and the avatar block come along
        let bare = Notification {
            id: 1,
            title: s("Call in progress"),
            flags: NOTIFY_ONGOING,
            ..Notification::default()
        };
        let bb = bare.encode().unwrap();
        assert_eq!(Notification::decode(&bb), Ok(bare.clone()));
        assert!(Notification::decode(&bb).unwrap().ongoing());
        // limits: unknown flags, styles and kinds, a long group, more kinds than actions
        for bad in [
            Notification {
                flags: 8,
                ..bare.clone()
            },
            Notification {
                style: style::LAST + 1,
                ..bare.clone()
            },
            Notification {
                group: "g".repeat(GROUP_MAX + 1),
                ..bare.clone()
            },
            Notification {
                action_kinds: vec![action::DANGER],
                ..bare.clone()
            },
            // a plain card without actions: a kind with nothing to belong to
            Notification {
                id: 1,
                title: s("x"),
                action_kinds: vec![action::BUTTON],
                ..Notification::default()
            },
            Notification {
                actions: vec![s("x")],
                action_kinds: vec![action::LAST + 1],
                ..bare.clone()
            },
        ] {
            assert_eq!(bad.encode(), Err(PageError), "{bad:?}");
        }
        // every cut inside the 1.18 part (from the bar byte on) is refused, and so is a trailing byte; cuts before
        // it are older records
        let tail = 3 + 1 + new.group.len() + 8 + new.actions.len();
        for n in nb.len() - tail..nb.len() {
            assert_eq!(Notification::decode(&nb[..n]), Err(PageError), "{n}");
        }
        let mut long = nb;
        long.push(0);
        assert_eq!(Notification::decode(&long), Err(PageError));
    }

    #[test]
    fn notifications_carry_progress_since_1_11() {
        let plain = Notification {
            id: 1,
            title: s("Files"),
            text: s("Copying file 2 of 2"),
            actions: vec![s("Cancel")],
            ..Notification::default()
        };
        let pb = plain.encode().unwrap();
        // a 1.10 record (no progress byte) still decodes, without a bar
        assert_eq!(Notification::decode(&pb), Ok(plain.clone()));
        let copying = Notification {
            progress: Some(45),
            ..plain
        };
        let cb = copying.encode().unwrap();
        assert_eq!(cb.last(), Some(&45));
        assert_eq!(Notification::decode(&cb), Ok(copying.clone()));
        // with nothing else after the text, the progress still has its actions/avatar bytes in front of it
        let bare = Notification {
            progress: Some(0),
            ..Notification::default()
        };
        assert_eq!(Notification::decode(&bare.encode().unwrap()), Ok(bare));
        let over = Notification {
            progress: Some(101),
            ..copying
        };
        assert_eq!(over.encode(), Err(PageError));
        let mut bad = cb;
        *bad.last_mut().unwrap() = 101;
        assert_eq!(Notification::decode(&bad), Err(PageError));
        bad.pop();
        bad.extend([50, 0]);
        assert_eq!(Notification::decode(&bad), Err(PageError));
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

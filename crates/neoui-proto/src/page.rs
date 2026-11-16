// SPDX-License-Identifier: Apache-2.0
//! The Page API (protocol 1.7, docs `gestures.md` "Five places"): an app describes the page it shows ([`Page`]) and
//! the shell draws everything around it: the title in the status bar, the bottom strip (Cancel, the primary action,
//! ⋮), the Options sheet, and dialogs ([`Query`]). Taps come back as [`crate::ipc::AppEvent`]s.
//!
//! Wire format (all lengths u8, strings UTF-8): `Page` = `[flags, icon, dots count, dots current, options n]` then
//! the title, the primary's label when there is one, and the options. `Query` = `[flags, buttons n]` then the text
//! and the buttons.

use alloc::string::String;
use alloc::vec::Vec;

use crate::ipc::{TITLE_MAX, TOOL_DELETE};

/// The longest primary label, option or dialog button, in bytes.
pub const LABEL_MAX: usize = 32;
/// The most options a page has (more belong on a page of their own).
pub const OPTIONS_MAX: usize = 12;
/// The longest dialog text, in bytes.
pub const TEXT_MAX: usize = 160;
/// The most dialog buttons.
pub const BUTTONS_MAX: usize = 3;
/// The most page dots.
pub const DOTS_MAX: u8 = 8;
/// The largest encoded [`Page`].
pub const PAGE_MAX: usize = 5 + 1 + TITLE_MAX + 1 + LABEL_MAX + OPTIONS_MAX * (1 + LABEL_MAX);
/// The largest encoded [`Query`].
pub const QUERY_MAX: usize = 2 + 1 + TEXT_MAX + BUTTONS_MAX * (1 + LABEL_MAX);

/// A page's main action: a green pill in the strip. A tap sends [`crate::ipc::AppEvent::Primary`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Action {
    /// `TOOL_*` ([`crate::ipc::TOOL_NONE`]: label only).
    pub icon: i32,
    pub label: String,
}

/// What an app shows now. Send it again whenever any of it changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    /// The status bar's title (empty: the app's name).
    pub title: String,
    /// Back stays inside the app (a page to go up to, a form to cancel): the app gets
    /// [`crate::ipc::AppEvent::Back`]. Otherwise Back leaves the app for where the user came from, and ← is dimmed.
    pub can_back: bool,
    pub primary: Option<Action>,
    /// A form: the strip shows Cancel ([`crate::ipc::AppEvent::Cancel`]).
    pub cancel: bool,
    /// The ⋮ sheet's items ([`crate::ipc::AppEvent::Option`] with the index); no ⋮ when empty.
    pub options: Vec<String>,
    /// Hide the status bar and the strip (a call, the camera, a video).
    pub full: bool,
    /// Sideways pages (count, current): dots beside the title.
    pub dots: Option<(u8, u8)>,
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

/// A [`Page`] or [`Query`] that is malformed or over a limit (`KErrArgument`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageError;

const CAN_BACK: u8 = 1;
const CANCEL: u8 = 2;
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
        let icon = self.primary.as_ref().map_or(0, |a| a.icon);
        if self.options.len() > OPTIONS_MAX
            || !(0..=TOOL_DELETE).contains(&icon)
            || self.dots.is_some_and(|d| !dots_ok(d))
        {
            return Err(PageError);
        }
        let flags = (self.can_back as u8 * CAN_BACK)
            | (self.cancel as u8 * CANCEL)
            | (self.full as u8 * FULL)
            | (self.primary.is_some() as u8 * PRIMARY)
            | (self.dots.is_some() as u8 * DOTS);
        let (n, at) = self.dots.unwrap_or((0, 0));
        let mut out = alloc::vec![flags, icon as u8, n, at, self.options.len() as u8];
        put(&mut out, &self.title, TITLE_MAX)?;
        if let Some(a) = &self.primary {
            put(&mut out, &a.label, LABEL_MAX)?;
        }
        for o in &self.options {
            put(&mut out, o, LABEL_MAX)?;
        }
        Ok(out)
    }

    pub fn decode(b: &[u8]) -> Result<Self, PageError> {
        if b.len() > PAGE_MAX {
            return Err(PageError);
        }
        let mut r = Reader(b);
        let flags = r.byte()?;
        let icon = i32::from(r.byte()?);
        let dots = (r.byte()?, r.byte()?);
        let n = usize::from(r.byte()?);
        if flags >> 5 != 0 || icon > TOOL_DELETE || n > OPTIONS_MAX {
            return Err(PageError);
        }
        let dots = match (flags & DOTS != 0, dots_ok(dots)) {
            (false, _) => None,
            (true, true) => Some(dots),
            (true, false) => return Err(PageError),
        };
        let title = r.text(TITLE_MAX)?;
        let primary = if flags & PRIMARY != 0 {
            Some(Action {
                icon,
                label: r.text(LABEL_MAX)?,
            })
        } else {
            None
        };
        let options = (0..n)
            .map(|_| r.text(LABEL_MAX))
            .collect::<Result<_, _>>()?;
        if !r.0.is_empty() {
            return Err(PageError);
        }
        Ok(Page {
            title,
            can_back: flags & CAN_BACK != 0,
            primary,
            cancel: flags & CANCEL != 0,
            options,
            full: flags & FULL != 0,
            dots,
        })
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
    use crate::ipc::TOOL_CHECK;
    use alloc::string::ToString;
    use alloc::vec;

    fn s(t: &str) -> String {
        t.to_string()
    }

    fn full_page() -> Page {
        Page {
            title: "x".repeat(TITLE_MAX),
            can_back: true,
            primary: Some(Action {
                icon: TOOL_CHECK,
                label: "y".repeat(LABEL_MAX),
            }),
            cancel: true,
            options: vec!["z".repeat(LABEL_MAX); OPTIONS_MAX],
            full: true,
            dots: Some((DOTS_MAX, DOTS_MAX - 1)),
        }
    }

    #[test]
    fn pages_round_trip() {
        let pages = [
            Page::default(),
            Page {
                title: s("Contacts"),
                primary: Some(Action {
                    icon: 3,
                    label: s("New"),
                }),
                options: vec![s("Mark several"), s("Import vCards")],
                dots: Some((2, 1)),
                ..Page::default()
            },
            Page {
                title: s("Ԥ Мария"),
                can_back: true,
                cancel: true,
                ..Page::default()
            },
            full_page(),
        ];
        for p in pages {
            let b = p.encode().unwrap();
            assert!(b.len() <= PAGE_MAX);
            assert_eq!(Page::decode(&b), Ok(p));
        }
        assert_eq!(full_page().encode().unwrap().len(), PAGE_MAX);
    }

    #[test]
    fn pages_over_a_limit_are_refused() {
        let over = [
            Page {
                title: "x".repeat(TITLE_MAX + 1),
                ..Page::default()
            },
            Page {
                primary: Some(Action {
                    icon: 0,
                    label: "y".repeat(LABEL_MAX + 1),
                }),
                ..Page::default()
            },
            Page {
                primary: Some(Action {
                    icon: 7,
                    label: s("?"),
                }),
                ..Page::default()
            },
            Page {
                primary: Some(Action {
                    icon: -1,
                    label: s("?"),
                }),
                ..Page::default()
            },
            Page {
                options: vec![s("o"); OPTIONS_MAX + 1],
                ..Page::default()
            },
            Page {
                options: vec!["o".repeat(LABEL_MAX + 1)],
                ..Page::default()
            },
            Page {
                dots: Some((2, 2)),
                ..Page::default()
            },
            Page {
                dots: Some((DOTS_MAX + 1, 0)),
                ..Page::default()
            },
        ];
        for p in over {
            assert_eq!(p.encode(), Err(PageError), "{p:?}");
        }
    }

    #[test]
    fn malformed_pages_are_refused() {
        let good = Page {
            title: s("ab"),
            options: vec![s("c")],
            ..Page::default()
        }
        .encode()
        .unwrap();
        // every truncation, a trailing byte, an unknown flag, bad UTF-8, a bad dot, too many options, too long
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
        b[6] = 0xFF;
        assert_eq!(Page::decode(&b), Err(PageError));
        let mut b = good.clone();
        b[0] = DOTS;
        b[2] = 2;
        b[3] = 2;
        assert_eq!(Page::decode(&b), Err(PageError));
        let mut b = good.clone();
        b[4] = OPTIONS_MAX as u8 + 1;
        assert_eq!(Page::decode(&b), Err(PageError));
        assert_eq!(Page::decode(&[0; PAGE_MAX + 1]), Err(PageError));
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

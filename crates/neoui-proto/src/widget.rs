// SPDX-License-Identifier: Apache-2.0
//! App widgets on home (home.md Part 4, protocol 1.22; docs neoui-shell/protocol.md "WIDGET"): an app declared in
//! [`CATALOGUE`] sends the content of one of its widgets with `ipc::WIDGET` ([`Content`]); the shell keeps the latest
//! under `widget.<app>.<name>` ([`setting_key`]) so home shows it after a reboot without the app running, and draws it
//! with the widget's template at the size the user picked (home's layout v2, `crate::home::Item::AppWidget`). Before
//! any content the widget shows its label greyed out.
//!
//! Record: `1:b, name:s16, template:b, page:h, payload`. Payloads: Text `icon:h, title:s32, lines:b ≤ 3, line:s48*`;
//! Count `icon:h, value:w, label:s32`; People `count:b ≤ 5, (initials:s8, tint:w, name:s32)*`.

use crate::home::Size;
use crate::settings_app::wire::{R, W};
use alloc::string::String;
use alloc::vec::Vec;

/// A widget's template (`Content`'s third byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Template {
    Text = 1,
    Count = 2,
    People = 3,
}

/// An app widget the shell knows: `(app, name, template, sizes, label)`. The Add sheet offers every size.
pub const CATALOGUE: &[(&str, &str, Template, &[Size], &str)] = &[
    (
        "clock",
        "next",
        Template::Text,
        &[Size::Square, Size::Wide],
        "Next alarm",
    ),
    // NEOKIT's demo (its Notifications page sends it), so the shell's side runs on virt without DTSRV
    (
        "kit",
        "demo",
        Template::Text,
        &[Size::Square, Size::Wide],
        "Kit demo",
    ),
];

/// The catalogue row of `(app, name)`.
#[must_use]
pub fn find(
    app: &str,
    name: &str,
) -> Option<&'static (
    &'static str,
    &'static str,
    Template,
    &'static [Size],
    &'static str,
)> {
    CATALOGUE.iter().find(|e| e.0 == app && e.1 == name)
}

pub const NAME_MAX: usize = 16;
pub const TITLE_MAX: usize = 32;
pub const LINE_MAX: usize = 48;
pub const LINES_MAX: usize = 3;
pub const LABEL_MAX: usize = 32;
pub const INITIALS_MAX: usize = 8;
pub const PEOPLE_MAX: usize = 5;
/// The longest encoded [`Content`] (a People widget with five).
pub const WIDGET_MAX: usize =
    1 + (1 + NAME_MAX) + 1 + 2 + 1 + PEOPLE_MAX * ((1 + INITIALS_MAX) + 4 + (1 + TITLE_MAX));

/// The setting that keeps the content: `widget.<app>.<name>`; `None` when either isn't a key segment
/// (`[a-z0-9_]`) or the whole is over 32 bytes.
#[must_use]
pub fn setting_key(app: &str, name: &str) -> Option<String> {
    let key = alloc::format!("widget.{app}.{name}");
    (crate::settings::key::is_valid_name(&key) && key.split('.').count() == 3).then_some(key)
}

/// One person of a People widget.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    /// At most 8 bytes.
    pub initials: String,
    /// 0xRRGGBB.
    pub tint: u32,
    /// At most 32 bytes.
    pub name: String,
}

/// A template's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// An icon (a kit glyph; 0 = the app's), a title and up to three lines.
    Text {
        icon: u16,
        title: String,
        lines: Vec<String>,
    },
    /// A big number with its label.
    Count {
        icon: u16,
        value: u32,
        label: String,
    },
    /// Up to five people.
    People(Vec<Person>),
}

impl Body {
    #[must_use]
    pub fn template(&self) -> Template {
        match self {
            Body::Text { .. } => Template::Text,
            Body::Count { .. } => Template::Count,
            Body::People(_) => Template::People,
        }
    }
}

/// What the app sends for widget `name`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    /// The catalogue's name (`next`), 1–16 bytes.
    pub name: String,
    /// The page a tap opens.
    pub page: u16,
    pub body: Body,
}

impl Content {
    /// The record; strings are cut on a character boundary, lines past three and people past five left out.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        w.b(1);
        w.s(&self.name, NAME_MAX);
        w.b(self.body.template() as u8);
        w.h(self.page);
        match &self.body {
            Body::Text { icon, title, lines } => {
                w.h(*icon);
                w.s(title, TITLE_MAX);
                let n = lines.len().min(LINES_MAX);
                w.b(n as u8);
                for l in &lines[..n] {
                    w.s(l, LINE_MAX);
                }
            }
            Body::Count { icon, value, label } => {
                w.h(*icon);
                w.w(*value);
                w.s(label, LABEL_MAX);
            }
            Body::People(people) => {
                let n = people.len().min(PEOPLE_MAX);
                w.b(n as u8);
                for p in &people[..n] {
                    w.s(&p.initials, INITIALS_MAX);
                    w.w(p.tint);
                    w.s(&p.name, TITLE_MAX);
                }
            }
        }
        w.0
    }

    /// `None` for a malformed record: another version, an empty or long name, an unknown template, too many lines or
    /// people, a long or non-UTF-8 string, a short record or trailing bytes. Whether the name is the app's and its
    /// template the catalogue's is [`Content::fits`].
    #[must_use]
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() > WIDGET_MAX {
            return None;
        }
        let mut r = R(b);
        if r.b()? != 1 {
            return None;
        }
        let name = r.s(NAME_MAX).filter(|n| !n.is_empty())?;
        let template = r.b()?;
        let page = r.h()?;
        let body = match template {
            1 => {
                let (icon, title) = (r.h()?, r.s(TITLE_MAX)?);
                let n = usize::from(r.b()?);
                if n > LINES_MAX {
                    return None;
                }
                let lines = (0..n).map(|_| r.s(LINE_MAX)).collect::<Option<_>>()?;
                Body::Text { icon, title, lines }
            }
            2 => Body::Count {
                icon: r.h()?,
                value: r.w()?,
                label: r.s(LABEL_MAX)?,
            },
            3 => {
                let n = usize::from(r.b()?);
                if n > PEOPLE_MAX {
                    return None;
                }
                let people = (0..n)
                    .map(|_| {
                        Some(Person {
                            initials: r.s(INITIALS_MAX)?,
                            tint: r.w()?,
                            name: r.s(TITLE_MAX)?,
                        })
                    })
                    .collect::<Option<_>>()?;
                Body::People(people)
            }
            _ => return None,
        };
        r.end()?;
        Some(Self { name, page, body })
    }

    /// Whether `app` may send this: its catalogue has the name, with this template (`KErrNotFound` /
    /// `KErrArgument` otherwise).
    ///
    /// # Errors
    /// `Err(false)` for a name not in `app`'s catalogue, `Err(true)` for another template.
    pub fn fits(&self, app: &str) -> Result<(), bool> {
        let e = find(app, &self.name).ok_or(false)?;
        if e.2 == self.body.template() {
            Ok(())
        } else {
            Err(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn next() -> Content {
        Content {
            name: "next".into(),
            page: 1,
            body: Body::Text {
                icon: 0,
                title: "07:30".into(),
                lines: vec!["Tomorrow".into(), "Gym".into()],
            },
        }
    }

    #[test]
    fn round_trips_every_template() {
        let people = Body::People(
            (0..5)
                .map(|i| Person {
                    initials: "AB".repeat(5),
                    tint: 0x112233 + i,
                    name: "n".repeat(40),
                })
                .collect(),
        );
        for body in [
            next().body,
            Body::Count {
                icon: 3,
                value: u32::MAX,
                label: "steps".into(),
            },
            Body::People(vec![]),
        ] {
            let c = Content { body, ..next() };
            assert_eq!(Content::decode(&c.encode()), Some(c));
        }
        let big = Content {
            body: people,
            name: "n".repeat(20),
            ..next()
        };
        let b = big.encode();
        assert_eq!(b.len(), WIDGET_MAX);
        let d = Content::decode(&b).unwrap();
        assert_eq!(d.name.len(), 16);
        let Body::People(p) = d.body else { panic!() };
        assert_eq!((p[0].initials.len(), p[0].name.len()), (8, 32));
    }

    #[test]
    fn malformed_records_are_refused() {
        let good = next().encode();
        for n in 0..good.len() {
            assert_eq!(Content::decode(&good[..n]), None, "{n}");
        }
        let mut long = good.clone();
        long.push(0);
        assert_eq!(Content::decode(&long), None);
        let template_at = 1 + 1 + 4;
        for (at, v) in [(0, 2), (template_at, 0), (template_at, 4)] {
            let mut b = good.clone();
            b[at] = v;
            assert_eq!(Content::decode(&b), None, "byte {at} = {v}");
        }
        // an empty name; four lines on the wire; six people
        let mut b = good.clone();
        b[1] = 0;
        b.drain(2..6);
        assert_eq!(Content::decode(&b), None);
        let mut b = good.clone();
        let lines_at = template_at + 1 + 2 + 2 + 1 + 5;
        assert_eq!(b[lines_at], 2);
        b[lines_at] = 4;
        assert_eq!(Content::decode(&b), None);
        let six = [1u8, 4, b'n', b'e', b'x', b't', 3, 0, 0, 6];
        assert_eq!(Content::decode(&six), None);
        assert_eq!(Content::decode(&[0u8; WIDGET_MAX + 1]), None);
    }

    #[test]
    fn the_catalogue_decides() {
        assert_eq!(next().fits("clock"), Ok(()));
        assert_eq!(next().fits("files"), Err(false));
        let count = Content {
            body: Body::Count {
                icon: 0,
                value: 1,
                label: String::new(),
            },
            ..next()
        };
        assert_eq!(count.fits("clock"), Err(true));
        // every catalogue row has a key, sizes, a label
        for (app, name, _, sizes, label) in CATALOGUE {
            assert!(setting_key(app, name).is_some() && !sizes.is_empty() && !label.is_empty());
        }
    }

    #[test]
    fn setting_keys() {
        assert_eq!(
            setting_key("clock", "next").as_deref(),
            Some("widget.clock.next")
        );
        assert_eq!(setting_key("clock", "Next"), None);
        assert_eq!(setting_key("clock", ""), None);
        assert_eq!(setting_key("clock", "a.b"), None);
        // 7 + 16 + 1 + 9 = 33 bytes
        assert_eq!(setting_key("sixteen_chars_ab", "ninechars"), None);
        assert!(setting_key("sixteen_chars_ab", "eightchr").is_some());
    }
}

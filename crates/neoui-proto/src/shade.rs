// SPDX-License-Identifier: Apache-2.0
//! The shade's cards in order (docs neoui-shell/top-panel.md "Cards", TP2a): sections, groups, "2 min".
//!
//! The shell keeps every app's cards newest first (`ipc::NOTIFY`); [`layout`] gives the rows the shade draws:
//! **Live** (ongoing cards), **Conversations** (style Conversation), then **Alerts**, each newest first. Cards of one
//! app with the same `group` stack under their newest, which shows how many there are; the open group shows the rest
//! right under it. Cards of a Min channel (TP4) fold under **Silent**, last.

use crate::page::{Notification, style};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The shade's sections, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Live = 0,
    Conversations = 1,
    Alerts = 2,
    /// (TP4) A Min channel's cards, at the bottom.
    Silent = 3,
}

impl Section {
    /// The section's label over its first card.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Section::Live => "LIVE",
            Section::Conversations => "CONVERSATIONS",
            Section::Alerts => "ALERTS",
            Section::Silent => "SILENT",
        }
    }
}

/// Where card `n` goes.
#[must_use]
pub fn section(n: &Notification) -> Section {
    if n.ongoing() {
        Section::Live
    } else if n.style == style::CONVERSATION {
        Section::Conversations
    } else {
        Section::Alerts
    }
}

/// One row of the shade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// The card's place in the shell's list.
    pub index: usize,
    pub section: Section,
    /// The first row of its section (it carries the label).
    pub first: bool,
    /// A group's head: how many cards the group has (1 for a card on its own, or a group member under an open head).
    pub count: usize,
    /// Under an open group's head.
    pub member: bool,
}

/// The rows for the shell's `list` (app key, card; newest first). `open` names the group shown open (app key, group).
#[must_use]
pub fn layout(list: &[(&str, &Notification)], open: Option<(&str, &str)>) -> Vec<Row> {
    layout_with(list, open, |_| false)
}

/// [`layout`], with the cards `silent(i)` (a Min channel, TP4) under Silent unless they are ongoing.
#[must_use]
pub fn layout_with(
    list: &[(&str, &Notification)],
    open: Option<(&str, &str)>,
    silent: impl Fn(usize) -> bool,
) -> Vec<Row> {
    let at = |i: usize, n: &Notification| {
        if !n.ongoing() && silent(i) {
            Section::Silent
        } else {
            section(n)
        }
    };
    let mut rows = Vec::new();
    for s in [
        Section::Live,
        Section::Conversations,
        Section::Alerts,
        Section::Silent,
    ] {
        let mut first = true;
        for (i, (key, n)) in list.iter().enumerate() {
            if at(i, n) != s {
                continue;
            }
            // a group is one stack whatever its cards' sections: it stands where its newest card goes
            let grouped = |(k, m): &(&str, &Notification)| *k == *key && m.group == n.group;
            if !n.group.is_empty() {
                // only the group's newest card stands for it
                if list[..i].iter().any(grouped) {
                    continue;
                }
                let count = list.iter().filter(|e| grouped(e)).count();
                rows.push(Row {
                    index: i,
                    section: s,
                    first,
                    count,
                    member: false,
                });
                first = false;
                if count > 1 && open == Some((*key, n.group.as_str())) {
                    for (j, e) in list.iter().enumerate().skip(i + 1) {
                        if grouped(e) {
                            rows.push(Row {
                                index: j,
                                section: s,
                                first: false,
                                count: 1,
                                member: true,
                            });
                        }
                    }
                }
                continue;
            }
            rows.push(Row {
                index: i,
                section: s,
                first,
                count: 1,
                member: false,
            });
            first = false;
        }
    }
    rows
}

/// When a card happened, as its header shows it: "now" within a minute, "N min" within the hour, else the local
/// clock time (`offset_s` east of UTC; 24-hour or not). Times ahead of `now_ms` read "now".
#[must_use]
pub fn when_text(when_ms: u64, now_ms: u64, offset_s: i32, h24: bool) -> String {
    let ago = now_ms.saturating_sub(when_ms) / 1000;
    if ago < 60 {
        return "now".into();
    }
    if ago < 3600 {
        return format!("{} min", ago / 60);
    }
    let local = (when_ms / 1000) as i64 + i64::from(offset_s);
    let minute_of_day = local.rem_euclid(86_400) / 60;
    let (h, m) = (minute_of_day / 60, minute_of_day % 60);
    if h24 {
        format!("{h:02}:{m:02}")
    } else {
        format!("{}:{m:02}", (h + 11) % 12 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::NOTIFY_ONGOING;
    use alloc::vec;

    fn card(id: u32, group: &str, style: u8, flags: u8) -> Notification {
        Notification {
            id,
            title: "t".into(),
            group: group.into(),
            style,
            flags,
            ..Notification::default()
        }
    }

    #[test]
    fn sections_then_newest_first_and_groups_stack() {
        let call = card(1, "", style::CALL, NOTIFY_ONGOING);
        let ava = card(2, "chat", style::CONVERSATION, 0);
        let ben = card(3, "chat", style::CONVERSATION, 0);
        let copy = card(4, "", style::PROGRESS, 0);
        let ava2 = card(5, "chat", style::CONVERSATION, 0);
        // the shell's list, newest first: an alert, a conversation, the call, more conversations
        let list: Vec<(&str, &Notification)> = vec![
            ("files", &copy),
            ("messages", &ava),
            ("phone", &call),
            ("messages", &ben),
            ("messages", &ava2),
        ];
        let rows = layout(&list, None);
        let order: Vec<(usize, Section, bool, usize)> = rows
            .iter()
            .map(|r| (r.index, r.section, r.first, r.count))
            .collect();
        assert_eq!(
            order,
            vec![
                (2, Section::Live, true, 1),
                (1, Section::Conversations, true, 3),
                (0, Section::Alerts, true, 1),
            ]
        );
        // the open group shows its other cards under its head, newest first
        let open = layout(&list, Some(("messages", "chat")));
        let members: Vec<(usize, bool)> = open.iter().map(|r| (r.index, r.member)).collect();
        assert_eq!(
            members,
            vec![(2, false), (1, false), (3, true), (4, true), (0, false)]
        );
        // a group whose cards fall in two sections is still one stack, under its newest card's section
        let live = card(7, "chat", style::CONVERSATION, NOTIFY_ONGOING);
        let split: Vec<(&str, &Notification)> = vec![("messages", &ava), ("messages", &live)];
        let rows = layout(&split, Some(("messages", "chat")));
        let got: Vec<(usize, Section, usize, bool)> = rows
            .iter()
            .map(|r| (r.index, r.section, r.count, r.member))
            .collect();
        assert_eq!(
            got,
            vec![
                (0, Section::Conversations, 2, false),
                (1, Section::Conversations, 1, true)
            ]
        );
        // another app's group of the same name is another group
        let other = card(6, "chat", style::CONVERSATION, 0);
        let two: Vec<(&str, &Notification)> = vec![("messages", &ava), ("kit", &other)];
        assert_eq!(layout(&two, None).iter().map(|r| r.count).sum::<usize>(), 2);
        assert_eq!(layout(&two, None).len(), 2);
        // a Min channel's card folds under Silent, last; an ongoing one stays Live
        let rows = layout_with(&list, None, |i| i == 0 || i == 2);
        let got: Vec<(usize, Section)> = rows.iter().map(|r| (r.index, r.section)).collect();
        assert_eq!(
            got,
            vec![
                (2, Section::Live),
                (1, Section::Conversations),
                (0, Section::Silent)
            ]
        );
    }

    #[test]
    fn when_reads_now_minutes_then_the_clock() {
        let now = 1_795_691_635_000; // 2026-11-26 11:13:55 UTC
        assert_eq!(when_text(now - 30_000, now, 0, true), "now");
        assert_eq!(when_text(now + 5_000, now, 0, true), "now");
        assert_eq!(when_text(now - 120_000, now, 0, true), "2 min");
        assert_eq!(when_text(now - 59 * 60_000, now, 0, true), "59 min");
        // two hours earlier, at UTC+4: 09:13 UTC is 13:13 there
        assert_eq!(when_text(now - 2 * 3_600_000, now, 14_400, true), "13:13");
        assert_eq!(
            when_text(now - 2 * 3_600_000, now, 14_400 + 4 * 3600, false),
            "5:13"
        );
        assert_eq!(
            when_text(now - 2 * 3_600_000, now, -6 * 3600, false),
            "3:13"
        );
    }
}

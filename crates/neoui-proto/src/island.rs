// SPDX-License-Identifier: Apache-2.0
//! (TP6) The live island's state in the shell (docs neoui-shell/top-panel.md "Live island (TP6)", protocol.md
//! "ACTIVITY"): the apps' activities ([`Island`], by app key and id), the order the island draws them in (Call ›
//! Recording › Timer › Media › Progress › other; the two highest show, the rest wait in the shade's Live section), who
//! may start one ([`may_start`]), the media session's own activity ([`from_media`]), and the times the shell ticks
//! itself ([`trail_text`]).

use crate::activity::{Activity, Trail, kind};
use crate::media::{Session, state};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The most activities the island draws (a pill and a circle).
pub const SHOWN: usize = 2;
/// A transient system alert's time on the island (the ringer to silent, the charger in).
pub const ALERT_MS: u64 = 2000;
/// The shell redraws an activity at most this often (an app's faster updates wait; the ticks are the shell's).
pub const UPDATE_MS: u64 = 1000;
/// The activity id the media session's own activity takes (an app's ids are 0–3).
pub const MEDIA_ID: u8 = 0xff;

/// Whether app activity `kind` may start while its app isn't in front: a running call, recording, timer or transfer
/// (top-panel.md "Rules"); anything else only from the front.
#[must_use]
pub fn may_start(kind: u8, in_front: bool) -> bool {
    in_front
        || matches!(
            kind,
            kind::CALL | kind::RECORDING | kind::TIMER | kind::PROGRESS
        )
}

/// The apps' activities, in the order they came.
#[derive(Clone, Debug)]
pub struct Island<K> {
    list: Vec<(K, Activity)>,
}

impl<K> Default for Island<K> {
    fn default() -> Self {
        Self { list: Vec::new() }
    }
}

/// What a post did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A new activity, or one ended: the island is laid out again now.
    Shape,
    /// The same activity with new content: drawn at the next update (at most one a second).
    Content,
    /// Nothing (an end of an activity that isn't there).
    None,
}

impl<K: Copy + PartialEq> Island<K> {
    /// App `key` sent `a` (checked by `Activity::decode`; an end has an empty title). `in_front`: the app is in front.
    ///
    /// # Errors
    /// `KErrPermissionDenied` for a new activity the app may not start now ([`may_start`]).
    pub fn post(&mut self, key: K, a: Activity, in_front: bool) -> Result<Change, i32> {
        let at = self
            .list
            .iter()
            .position(|(k, o)| *k == key && o.id == a.id);
        match at {
            Some(i) if a.is_end() => {
                self.list.remove(i);
                Ok(Change::Shape)
            }
            None if a.is_end() => Ok(Change::None),
            Some(i) => {
                let shape = self.list[i].1.kind != a.kind;
                self.list[i].1 = a;
                Ok(if shape {
                    Change::Shape
                } else {
                    Change::Content
                })
            }
            None if !may_start(a.kind, in_front) => Err(crate::settings::kerr::PERMISSION_DENIED),
            None => {
                self.list.push((key, a));
                Ok(Change::Shape)
            }
        }
    }

    /// App `key` exited or was turned off: its activities go. Whether it had any.
    pub fn end_app(&mut self, key: K) -> bool {
        let n = self.list.len();
        self.list.retain(|(k, _)| *k != key);
        n != self.list.len()
    }

    /// The apps with activities.
    pub fn keys(&self) -> impl Iterator<Item = K> + '_ {
        self.list.iter().map(|(k, _)| *k)
    }

    /// Every activity with `media` (the session's, [`from_media`]) among them, in the island's order: by kind, then
    /// the oldest first.
    #[must_use]
    pub fn ordered(&self, media: Option<(K, Activity)>) -> Vec<(K, Activity)> {
        let mut all: Vec<(K, Activity)> = self.list.clone();
        all.extend(media);
        // a stable sort keeps the order they came in within a kind
        all.sort_by_key(|(_, a)| a.kind);
        all
    }
}

/// What the island draws of `ordered`: the first [`SHOWN`]; over a full-screen app only a call or a recording.
#[must_use]
pub fn shown<K: Copy>(ordered: &[(K, Activity)], full_screen: bool) -> Vec<(K, Activity)> {
    ordered
        .iter()
        .filter(|(_, a)| !full_screen || matches!(a.kind, kind::CALL | kind::RECORDING))
        .take(SHOWN)
        .cloned()
        .collect()
}

/// The media session's activity (derived: a player never sends one): while it plays, its title and artist.
#[must_use]
pub fn from_media(s: &Session) -> Option<Activity> {
    matches!(s.state, state::PLAYING | state::BUFFERING).then(|| Activity {
        id: MEDIA_ID,
        kind: kind::MEDIA,
        tint: 0x22_c55e,
        title: s.title.clone(),
        text: s.artist.clone(),
        ..Activity::default()
    })
}

/// A span as the island writes it: "04:12", "1:04:12".
#[must_use]
pub fn span_text(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// The trail as drawn at `now` (unix ms): its text, a call's time since, a timer's time left (never below 0), a
/// ring's percent.
#[must_use]
pub fn trail_text(t: &Trail, now: u64) -> String {
    match t {
        Trail::Text(s) => s.clone(),
        Trail::Since(at) => span_text(now.saturating_sub(*at)),
        Trail::Until(at) => span_text(at.saturating_sub(now)),
        Trail::Ring { value, max } => {
            format!("{}%", u32::from(*value) * 100 / u32::from(*max).max(1))
        }
    }
}

/// A ring's fill, 0..=1 (-1: no ring).
#[must_use]
pub fn ring(t: &Trail) -> f32 {
    match t {
        Trail::Ring { value, max } => f32::from(*value) / f32::from((*max).max(1)),
        _ => -1.0,
    }
}

/// Whether the shell ticks the trail each second.
#[must_use]
pub fn ticks(t: &Trail) -> bool {
    matches!(t, Trail::Since(_) | Trail::Until(_))
}

/// The avatar's initials for a call's title ("Ava Lindqvist" → "AL"; a number gets none).
#[must_use]
pub fn initials(title: &str) -> String {
    title
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .filter(|c| c.is_alphabetic())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// `tint` clamped to a colour that reads on the dark status bar: too dark a tint is lifted to a light one of the same
/// hue; 0 is the kind's own (green for a call, red for a recording, amber for a timer, blue otherwise).
#[must_use]
pub fn tint(rgb: u32, kind: u8) -> u32 {
    if rgb == 0 {
        return match kind {
            kind::CALL | kind::MEDIA => 0x22_c55e,
            kind::RECORDING => 0xef_4444,
            kind::TIMER => 0xf5_9e0b,
            _ => 0x60_a5fa,
        };
    }
    let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
    let top = r.max(g).max(b);
    if top >= 0x80 {
        return rgb & 0xff_ffff;
    }
    // scale up so the brightest channel is 0x80 (black becomes grey)
    let lift = |c: u32| (c * 0x80).checked_div(top).unwrap_or(0x80);
    lift(r) << 16 | lift(g) << 8 | lift(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::caps;
    use alloc::vec;

    fn act(id: u8, kind: u8, title: &str) -> Activity {
        Activity {
            id,
            kind,
            title: title.into(),
            ..Activity::default()
        }
    }

    #[test]
    fn order_and_the_two_shown() {
        let mut i = Island::default();
        assert_eq!(
            i.post("files", act(0, kind::PROGRESS, "Copy"), false),
            Ok(Change::Shape)
        );
        assert_eq!(
            i.post("clock", act(1, kind::TIMER, "Pasta"), false),
            Ok(Change::Shape)
        );
        assert_eq!(
            i.post("kit", act(0, kind::OTHER, "Demo"), false),
            Err(crate::settings::kerr::PERMISSION_DENIED),
            "an Other only from the front"
        );
        assert_eq!(
            i.post("kit", act(0, kind::OTHER, "Demo"), true),
            Ok(Change::Shape)
        );
        assert_eq!(
            i.post("phone", act(0, kind::CALL, "Ava"), false),
            Ok(Change::Shape)
        );
        let music = act(MEDIA_ID, kind::MEDIA, "Northern Lights");
        let all = i.ordered(Some(("files", music)));
        let kinds: Vec<u8> = all.iter().map(|(_, a)| a.kind).collect();
        assert_eq!(
            kinds,
            vec![
                kind::CALL,
                kind::TIMER,
                kind::MEDIA,
                kind::PROGRESS,
                kind::OTHER
            ]
        );
        let two = shown(&all, false);
        assert_eq!((two[0].0, two[1].0), ("phone", "clock"));
        // over a full-screen app only the call
        assert_eq!(shown(&all, true).len(), 1);
        // an update is content; a changed kind lays out again; an end goes
        let mut t = act(1, kind::TIMER, "Pasta");
        t.text = "Clock · Timer".into();
        assert_eq!(i.post("clock", t, false), Ok(Change::Content));
        assert_eq!(i.post("clock", Activity::end(1), false), Ok(Change::Shape));
        assert_eq!(i.post("clock", Activity::end(1), false), Ok(Change::None));
        assert!(i.end_app("phone"));
        assert!(!i.end_app("phone"));
        assert_eq!(i.keys().collect::<Vec<_>>(), vec!["files", "kit"]);
    }

    #[test]
    fn the_shell_ticks_the_times() {
        let now = 10_000_000;
        assert_eq!(trail_text(&Trail::Since(now - 252_000), now), "04:12");
        assert_eq!(trail_text(&Trail::Until(now + 192_400), now), "03:12");
        assert_eq!(
            trail_text(&Trail::Until(now - 5), now),
            "00:00",
            "never below 0"
        );
        assert_eq!(trail_text(&Trail::Since(now - 3_852_000), now), "1:04:12");
        assert_eq!(
            trail_text(
                &Trail::Ring {
                    value: 64,
                    max: 100
                },
                now
            ),
            "64%"
        );
        assert_eq!(trail_text(&Trail::Text("2 of 5".into()), now), "2 of 5");
        assert!(ticks(&Trail::Since(0)) && !ticks(&Trail::Ring { value: 1, max: 2 }));
        assert_eq!(ring(&Trail::Ring { value: 1, max: 4 }), 0.25);
        assert_eq!(ring(&Trail::Since(0)), -1.0);
    }

    #[test]
    fn media_initials_and_tints() {
        let mut s = Session {
            state: state::PLAYING,
            caps: caps::PLAY,
            modes: 0,
            title: "Northern Lights".into(),
            artist: "The Harbour".into(),
            album: String::new(),
            art: 0,
            duration_ms: 0,
            position_ms: 0,
            at: 0,
        };
        let a = from_media(&s).expect("playing");
        assert_eq!(
            (a.kind, a.id, a.text.as_str()),
            (kind::MEDIA, MEDIA_ID, "The Harbour")
        );
        s.state = state::PAUSED;
        assert!(
            from_media(&s).is_none(),
            "a paused player leaves the island"
        );
        assert_eq!(initials("Ava Lindqvist"), "AL");
        assert_eq!(initials("+1 555 0142"), "");
        assert_eq!(initials("ben"), "B");
        assert_eq!(tint(0, kind::TIMER), 0xf5_9e0b);
        assert_eq!(tint(0x22_c55e, kind::CALL), 0x22_c55e);
        assert_eq!(tint(0x00_0010, kind::OTHER), 0x00_0080);
        assert_eq!(
            tint(0x01_0101, kind::OTHER),
            0x80_8080,
            "near black lifts to grey"
        );
    }
}

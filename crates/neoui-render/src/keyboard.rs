// SPDX-License-Identifier: Apache-2.0
//! The system keyboard: its language choice, which Slint can't loop over itself (answers for the `Layouts` global's
//! pure callbacks, `ui/components/keyboard/layouts.slint`), its key codes as Slint key events, what it types into the
//! shell's own fields (with the edit bar's Ctrl chords and a clipboard), and what it logs.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use slint::platform::{Key, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::suggest;
use crate::{KeyboardFocus, Layouts, Renderer, Shell};

/// The keyboard's state the renderer keeps: its log and the shell's clipboard (shared with the platform).
#[derive(Default)]
pub(crate) struct State {
    pub log: Rc<RefCell<Vec<String>>>,
    pub clipboard: Rc<Clipboard>,
    /// the edit bar sent Control (0x11): the next key is a chord
    pub ctrl: Cell<bool>,
}

/// The clipboard Slint's text fields in the shell copy to and paste from: the UI server keeps it equal to the apps'
/// (`Renderer::set_clipboard`) and takes what the shell copied (`Renderer::take_copied`).
#[derive(Default)]
pub(crate) struct Clipboard {
    pub text: RefCell<String>,
    pub copied: Cell<bool>,
}

/// Answers the `Layouts` callbacks and keeps the keyboard's log; before anything draws (bindings that call these keep
/// what they first returned).
pub(crate) fn wire(ui: &Shell, clipboard: Rc<Clipboard>) -> State {
    let layouts = ui.global::<Layouts>();
    layouts.on_pick(|on, from| pick(&on.iter().collect::<Vec<_>>(), from));
    layouts.on_tallest_of(|on, all| {
        let rows: Vec<usize> = all.iter().map(|l| l.rows.row_count()).collect();
        tallest(&on.iter().collect::<Vec<_>>(), &rows)
    });
    layouts.on_alternates(|alts, key| strings(alternates(alts.iter(), &key)));
    layouts.on_word_after(|word, t| word_after(&word, &t).into());
    layouts.on_suggest(|code, word| strings(suggest::words(&code, &word, 3)));
    layouts.on_replacement(|word, pick| replacement(&word, &pick).into());
    layouts.on_control(|key| control(&key).map(SharedString::from).unwrap_or_default());
    let state = State {
        clipboard,
        ..State::default()
    };
    let log = state.log.clone();
    ui.global::<KeyboardFocus>()
        .on_logged(move |s| log.borrow_mut().push(s.as_str().into()));
    state
}

fn strings<S: Into<SharedString>>(v: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        v.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
}

/// A long press's alternates for `key`: the rest of its entry in a layout's `alts` (an entry is the key then what it
/// offers, one character each), or none.
pub fn alternates(alts: impl IntoIterator<Item = SharedString>, key: &str) -> Vec<SharedString> {
    alts.into_iter()
        .find(|e| {
            let mut c = e.chars();
            c.next().is_some_and(|k| key.chars().eq([k]))
        })
        .map(|e| {
            e.chars()
                .skip(1)
                .map(|c| SharedString::from(c.encode_utf8(&mut [0; 4]) as &str))
                .collect()
        })
        .unwrap_or_default()
}

/// The word being typed after the keyboard typed `t`: a letter (or an apostrophe inside a word) adds to it, Backspace
/// takes its last letter off, anything else (space, punctuation, Enter, a chord) ends it.
pub fn word_after(word: &str, t: &str) -> String {
    if t == "\u{8}" {
        let mut w = String::from(word);
        w.pop();
        return w;
    }
    let mut c = t.chars();
    match (c.next(), c.next()) {
        (Some(ch), None) if ch.is_alphabetic() || (ch == '\'' && !word.is_empty()) => {
            let mut w = String::from(word);
            w.push(ch);
            w
        }
        _ => String::new(),
    }
}

/// What replaces `word` with `pick` in the field: a Backspace for each of the word's characters, then `pick` and a
/// space (the suggestion is already in the word's case, [`suggest::words`]).
pub fn replacement(word: &str, pick: &str) -> String {
    let mut s: String = word.chars().map(|_| '\u{8}').collect();
    s.push_str(pick);
    s.push(' ');
    s
}

/// Ctrl + `key`: a letter's (either case) or @ [ \ ] ^ _'s control code (Ctrl+C 3, Ctrl+[ Escape), for the terminal.
/// Not Ctrl+Q: 0x11 is the edit bar's Control, which neoapp and the shell hold for the next key.
pub fn control(key: &str) -> Option<char> {
    let mut c = key.chars();
    let (Some(ch), None) = (c.next(), c.next()) else {
        return None;
    };
    let up = ch.to_ascii_uppercase();
    (('@'..='_').contains(&up) && up != 'Q').then(|| char::from(up as u8 & 0x1F))
}

/// A chord that would put a password field's text on the clipboard the apps share (Ctrl+C, Ctrl+X): dropped (Slint's
/// TextInput copies a password's text like any other).
fn copies_secret(secret: bool, text: &str) -> bool {
    secret && matches!(text, "c" | "C" | "x" | "X")
}

/// Types the keyboard's `codes` into the shell's own focused field as Slint key events. The edit bar's Control (0x11)
/// makes the next key a chord (Ctrl+A, Ctrl+C, Ctrl+V), as neoapp does for an app's fields; `secret`: the field is a
/// password, so no Copy or Cut.
pub(crate) fn type_into_shell(
    window: &slint::Window,
    state: &State,
    codes: Vec<u32>,
    secret: bool,
) {
    let control: SharedString = Key::Control.into();
    for code in codes {
        if code == 0x11 {
            state.ctrl.set(true);
            continue;
        }
        let Some(text) = key_text(code) else {
            continue;
        };
        let ctrl = state.ctrl.replace(false);
        if ctrl && copies_secret(secret, &text) {
            continue;
        }
        if ctrl {
            window.dispatch_event(WindowEvent::KeyPressed {
                text: control.clone(),
            });
        }
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
        if ctrl {
            window.dispatch_event(WindowEvent::KeyReleased {
                text: control.clone(),
            });
        }
    }
}

/// A key code from the system keyboard or a hardware key (Unicode; Enter 13, Backspace 8, Escape 27, Tab 9, Symbian's
/// arrows 0xF807-0xF80A) as the text of a Slint key event. The same mapping as `neoapp::App::dispatch_key`.
pub fn key_text(code: u32) -> Option<SharedString> {
    Some(match code {
        13 | 10 => Key::Return.into(),
        8 => Key::Backspace.into(),
        27 => Key::Escape.into(),
        9 => Key::Tab.into(),
        0xF807 => Key::LeftArrow.into(),
        0xF808 => Key::RightArrow.into(),
        0xF809 => Key::UpArrow.into(),
        0xF80A => Key::DownArrow.into(),
        c => char::from_u32(c)?.into(),
    })
}

/// The first layout turned on at or after `from`, wrapping round; 0 if none is (or there are none).
pub fn pick(on: &[bool], from: i32) -> i32 {
    let n = on.len() as i32;
    if n == 0 {
        return 0;
    }
    (0..n)
        .map(|d| (from + d).rem_euclid(n))
        .find(|&i| on[i as usize])
        .unwrap_or(0)
}

/// The most letter rows among the layouts turned on (`rows[i]` is layout i's).
pub fn tallest(on: &[bool], rows: &[usize]) -> i32 {
    on.iter()
        .zip(rows)
        .filter(|(on, _)| **on)
        .map(|(_, r)| *r as i32)
        .max()
        .unwrap_or(0)
}

/// The keyboard's part of the renderer's API.
impl Renderer {
    /// The system keyboard typed into one of the shell's own fields, not for an app (after a touch, and after its
    /// timers: a held key's repeats).
    pub(crate) fn type_into_shell(&mut self) {
        let focus = self.ui.global::<KeyboardFocus>();
        if focus.get_open() {
            let secret = focus.get_secret();
            let codes = self.take_typed();
            type_into_shell(self.window.window(), &self.kb, codes, secret);
        }
    }

    /// A finger went down at `y` (physical pixels): above the keyboard, the word it was suggesting for is over (the
    /// cursor may have moved). The UI server calls it for touches that go to an app.
    pub fn touch_outside_keyboard(&mut self, y: i32) {
        let body = self.app_body();
        if y < body.y + body.h as i32 - self.keyboard_h() as i32 {
            let f = self.ui.global::<KeyboardFocus>();
            f.set_outside(f.get_outside().wrapping_add(1));
        }
    }

    /// What the keyboard logged since the last call (a held key's repeats, a long press's pick, a suggestion, Ctrl),
    /// for the UI server's serial log. Never the text typed: a password field's keys go through it too.
    pub fn take_keyboard_log(&mut self) -> Vec<String> {
        core::mem::take(&mut *self.kb.log.borrow_mut())
    }

    /// The current keyboard language's code (`en`, `hy`, `ru`, `hyw`), for a hardware keyboard's keymap
    /// (`neoui_proto::hwkeys`).
    pub fn keyboard_code(&self) -> SharedString {
        let l = self.ui.global::<Layouts>();
        l.get_all()
            .row_data(l.get_current() as usize)
            .map(|x| x.code)
            .unwrap_or_default()
    }

    /// The apps' clipboard text (or "" when it holds files or nothing), for the shell's own fields' Paste.
    pub fn set_clipboard(&mut self, text: &str) {
        if *self.kb.clipboard.text.borrow() != text {
            *self.kb.clipboard.text.borrow_mut() = text.into();
        }
    }

    /// What the shell's own fields copied since the last call (the edit bar's Copy), for the apps' clipboard.
    pub fn take_copied(&mut self) -> Option<String> {
        self.kb
            .clipboard
            .copied
            .replace(false)
            .then(|| self.kb.clipboard.text.borrow().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternates_of_a_key() {
        let alts = ["eéè", "ոօ"].map(SharedString::from);
        assert_eq!(
            alternates(alts.clone(), "e"),
            ["é", "è"].map(SharedString::from)
        );
        assert_eq!(alternates(alts.clone(), "ո"), ["օ"].map(SharedString::from));
        assert!(alternates(alts.clone(), "x").is_empty());
        assert!(alternates(alts, "").is_empty());
    }

    #[test]
    fn words_follow_the_keys() {
        assert_eq!(word_after("", "h"), "h");
        assert_eq!(word_after("he", "l"), "hel");
        assert_eq!(word_after("барев", "\u{8}"), "баре");
        assert_eq!(word_after("", "\u{8}"), "");
        assert_eq!(word_after("don", "'"), "don'");
        assert_eq!(word_after("", "'"), "");
        assert_eq!(word_after("word", " "), "");
        assert_eq!(word_after("word", "\r"), "");
        assert_eq!(word_after("word", "1"), "");
        assert_eq!(word_after("բար", "և"), "բարև");
    }

    #[test]
    fn replacement_deletes_the_word() {
        assert_eq!(replacement("hel", "hello"), "\u{8}\u{8}\u{8}hello ");
        assert_eq!(replacement("բա", "բարև"), "\u{8}\u{8}բարև ");
    }

    #[test]
    fn a_password_never_reaches_the_clipboard() {
        assert!(copies_secret(true, "c") && copies_secret(true, "X"));
        assert!(!copies_secret(true, "v") && !copies_secret(true, "a"));
        assert!(!copies_secret(false, "c"));
    }

    #[test]
    fn control_codes() {
        assert_eq!(control("c"), Some('\u{3}'));
        assert_eq!(control("C"), Some('\u{3}'));
        assert_eq!(control("["), Some('\u{1b}'));
        assert_eq!(control("1"), None);
        assert_eq!(control("ф"), None);
        assert_eq!(control("ab"), None);
        // 0x11 is the edit bar's Control (neoapp and the shell hold it for the next key), not Ctrl+Q
        assert_eq!(control("q"), None);
        assert_eq!(control("s"), Some('\u{13}'));
    }

    #[test]
    fn key_text_maps_control_keys() {
        assert_eq!(key_text(13), Some(Key::Return.into()));
        assert_eq!(key_text(8), Some(Key::Backspace.into()));
        assert_eq!(key_text(0xF807), Some(Key::LeftArrow.into()));
        assert_eq!(key_text('ա' as u32), Some("ա".into()));
        assert_eq!(key_text(0xD800), None);
    }

    #[test]
    fn pick_wraps_and_skips_off() {
        let on = [true, false, true];
        assert_eq!(pick(&on, 0), 0);
        assert_eq!(pick(&on, 1), 2);
        assert_eq!(pick(&on, 3), 0);
        assert_eq!(pick(&on, -1), 2);
        assert_eq!(pick(&[false, true], 0), 1);
        assert_eq!(pick(&[false, false], 1), 0);
        assert_eq!(pick(&[], 5), 0);
    }

    #[test]
    fn tallest_counts_only_layouts_on() {
        assert_eq!(tallest(&[true, false, true], &[3, 4, 3]), 3);
        assert_eq!(tallest(&[true, true, true], &[3, 4, 3]), 4);
        assert_eq!(tallest(&[false, false], &[3, 4]), 0);
    }
}

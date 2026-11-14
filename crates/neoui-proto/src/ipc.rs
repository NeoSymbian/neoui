// SPDX-License-Identifier: Apache-2.0
//! The app ↔ UI server protocol (NeoSymbian R6). An app process connects to the Symbian IPC server [`SERVER`],
//! asks for its [`GEOMETRY`] and its drawing [`SURFACE`] (a shared chunk, `width * height` XRGB8888 pixels, stride =
//! width), draws, reports [`DAMAGE`], and keeps one [`NEXT_EVENT`] request outstanding for input and quit.
//!
//! Since 1.1 an app also reads, writes and watches settings ([`SETTING_GET`], [`SETTING_SET`], [`SETTING_WATCH`],
//! [`AppEvent::Settings`]; the model is [`crate::settings`]). Since 1.2 the server wakes the Clock app for a due
//! alarm ([`AppEvent::Alarm`]). Since 1.3 the system keyboard has modes ([`KEYBOARD_TEXT`], [`KEYBOARD_TERMINAL`]) and
//! a height that changes with its language and settings ([`AppEvent::Keyboard`]). Since 1.4 an app sets its title
//! ([`TITLE`]) and the toolbar's middle button ([`TOOLBAR`], [`AppEvent::Action`]), and the server opens one of its
//! pages ([`AppEvent::Page`]): the Phone app is one process behind the Phone and Contacts icons.

use crate::{InputEvent, TouchPhase};

/// The server's name (no leading `!`: that needs ProtServ).
pub const SERVER: &str = "NeoUi";
/// Protocol version (major, minor); a connect with another major is refused.
pub const VERSION: (u8, u8) = (1, 4);

/// `[Des8 out: GEOMETRY_LEN bytes = width, height, dp × 256 (u32 LE)]` → 0. `dp` is the shell's design unit in
/// pixels (`min(screen w, h) / 360`), so apps size things like the shell does.
pub const GEOMETRY: i32 = 1;
pub const GEOMETRY_LEN: usize = 12;
/// `[]` → completes with a handle (in the app's thread) to the surface chunk.
pub const SURFACE: i32 = 2;
/// `[x, y, w, h]` (surface pixels) → 0 once the server has shown them.
pub const DAMAGE: i32 = 3;
/// `[Des8 out: EVENT_LEN bytes]` → asynchronous: completes when there is an [`AppEvent`] for the app.
pub const NEXT_EVENT: i32 = 4;
/// `[mode: KEYBOARD_HIDDEN | KEYBOARD_TEXT | KEYBOARD_TERMINAL]` → the height in pixels of the system keyboard now
/// covering the bottom of the surface (0 when hidden). Its keys arrive as [`InputEvent::Key`] like hardware keys
/// (NeoSymbian R6c). The height can change later (language, settings): [`AppEvent::Keyboard`] tells the app.
pub const KEYBOARD: i32 = 5;
pub const KEYBOARD_HIDDEN: i32 = 0;
/// Letters, the symbols layer and the languages on (1.2's `show = 1`).
pub const KEYBOARD_TEXT: i32 = 1;
/// [`KEYBOARD_TEXT`] with a row of Escape (27), Tab (9) and the arrows (Symbian's `EKeyLeftArrow` 0xF807,
/// `EKeyRightArrow` 0xF808, `EKeyUpArrow` 0xF809, `EKeyDownArrow` 0xF80A) on top.
pub const KEYBOARD_TERMINAL: i32 = 2;
/// `[claims: NAV_BACK | NAV_OPTIONS]` → 0. The app takes over the toolbar's Back and/or Options buttons: the server
/// sends [`AppEvent::Back`] / [`AppEvent::Options`] instead of the shell acting on them. Claims end with the app.
pub const NAV: i32 = 6;
pub const NAV_BACK: i32 = 1;
pub const NAV_OPTIONS: i32 = 2;
/// `[action: POWER_RESTART | POWER_FASTBOOT | POWER_OFF]` → 0: the server saves the clock, finalises the drives
/// and asks the kernel (NeoSymbian H3a), as the Power menu does. Nothing comes back when that works.
pub const POWER: i32 = 7;
pub const POWER_RESTART: i32 = 0;
pub const POWER_FASTBOOT: i32 = 1;
pub const POWER_OFF: i32 = 2;
/// `[Des8 key, Des8 out value]` → the value's encoded length, the value written into argument 1
/// ([`crate::settings::Value`] encoding, at most [`crate::settings::value::ENCODED_MAX`] bytes). Fails with the
/// [`crate::settings::Error`] mapping: `KErrNotFound` for a key with no value and no default, `KErrArgument` for an
/// invalid key, `KErrOverflow` when the buffer is too short.
pub const SETTING_GET: i32 = 8;
/// `[Des8 key, Des8 value]` → 0. Fails with `KErrPermissionDenied` for a key the app may not write
/// ([`crate::settings::access`]), `KErrArgument`/`KErrTooBig` for an invalid key or value.
pub const SETTING_SET: i32 = 9;
/// `[Des8 prefix]` → 0: send [`AppEvent::Settings`] when a key starting with `prefix` changes (empty: any key). At
/// most [`crate::settings::service::WATCHES_MAX`] prefixes per session (`KErrOverflow`).
pub const SETTING_WATCH: i32 = 10;
/// `[Des8 title]` (UTF-8, at most [`TITLE_MAX`] bytes) → 0: the status bar shows it while the app is in front; empty
/// gives the shell's title for the app back.
pub const TITLE: i32 = 11;
pub const TITLE_MAX: usize = 48;
/// `[icon: TOOL_*, style: TOOLBAR_* bits, Des8 label]` → 0: the toolbar's middle button while the app is in front
/// ([`TOOL_NONE`]: none). A tap sends [`AppEvent::Action`]. The label (at most [`TITLE_MAX`] bytes) shows beside the
/// icon.
pub const TOOLBAR: i32 = 12;
pub const TOOL_NONE: i32 = 0;
pub const TOOL_CALL: i32 = 1;
pub const TOOL_ADD: i32 = 2;
pub const TOOL_PERSON_ADD: i32 = 3;
pub const TOOL_EDIT: i32 = 4;
pub const TOOL_CHECK: i32 = 5;
pub const TOOL_DELETE: i32 = 6;
/// The middle button is a green pill (the dialer's Call).
pub const TOOLBAR_PILL: i32 = 1;
/// Back shows as "Cancel" (an editor); it still sends Back (claim it with [`NAV`]).
pub const TOOLBAR_CANCEL: i32 = 2;
/// No buttons at all: the toolbar is an empty strip (full-screen call views).
pub const TOOLBAR_EMPTY: i32 = 4;
/// The toolbar's middle button as the shell reports it with the claimed Back/Options ([`NAV_BACK`], [`NAV_OPTIONS`]).
pub const NAV_ACTION: i32 = 4;

/// Encoded size of an [`AppEvent`].
pub const EVENT_LEN: usize = 16;

/// What the server tells an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// Input, positions relative to the surface.
    Input(InputEvent),
    /// Leave now (the user went back or home); the app should exit.
    Quit,
    /// The toolbar's Back, which the app claimed ([`NAV`]).
    Back,
    /// The toolbar's Options, which the app claimed ([`NAV`]).
    Options,
    /// The app went to the background (false) or came back (true); it keeps running either way (R10).
    Visible(bool),
    /// A watched setting changed ([`SETTING_WATCH`]); the app reads the keys it cares about again. `seq` is the
    /// store's change counter, for logs.
    Settings { seq: u32 },
    /// Alarm `id` (`crate::clock::alarm`) is due: the server brought the Clock app to the front to ring it.
    Alarm { id: u16 },
    /// The open system keyboard now covers `h` pixels of the surface's bottom (its language or settings changed).
    Keyboard { h: u32 },
    /// Show page `page` (the app's own numbering): the user opened it from an icon, a notification or Settings.
    Page { page: u16 },
    /// The toolbar's middle button ([`TOOLBAR`]).
    Action,
}

const KIND_TOUCH: u32 = 1;
const KIND_KEY: u32 = 2;
const KIND_QUIT: u32 = 3;
const KIND_BACK: u32 = 4;
const KIND_OPTIONS: u32 = 5;
const KIND_VISIBLE: u32 = 6;
const KIND_SETTINGS: u32 = 7;
const KIND_ALARM: u32 = 8;
const KIND_KEYBOARD: u32 = 9;
const KIND_PAGE: u32 = 10;
const KIND_ACTION: u32 = 11;

impl AppEvent {
    /// `[kind, a, b, c]` as little-endian u32s: touch = (phase | id << 8, x, y), key = (down, code, 0).
    pub fn encode(&self) -> [u8; EVENT_LEN] {
        let w: [u32; 4] = match *self {
            AppEvent::Input(InputEvent::Touch { id, x, y, phase }) => {
                let p = match phase {
                    TouchPhase::Down => 0,
                    TouchPhase::Move => 1,
                    TouchPhase::Up => 2,
                };
                [KIND_TOUCH, p | u32::from(id) << 8, x as u32, y as u32]
            }
            AppEvent::Input(InputEvent::Key { code, down }) => [KIND_KEY, down as u32, code, 0],
            AppEvent::Quit => [KIND_QUIT, 0, 0, 0],
            AppEvent::Back => [KIND_BACK, 0, 0, 0],
            AppEvent::Options => [KIND_OPTIONS, 0, 0, 0],
            AppEvent::Visible(v) => [KIND_VISIBLE, v as u32, 0, 0],
            AppEvent::Settings { seq } => [KIND_SETTINGS, seq, 0, 0],
            AppEvent::Alarm { id } => [KIND_ALARM, u32::from(id), 0, 0],
            AppEvent::Keyboard { h } => [KIND_KEYBOARD, h, 0, 0],
            AppEvent::Page { page } => [KIND_PAGE, u32::from(page), 0, 0],
            AppEvent::Action => [KIND_ACTION, 0, 0, 0],
        };
        let mut out = [0u8; EVENT_LEN];
        for (i, v) in w.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }

    pub fn decode(b: &[u8; EVENT_LEN]) -> Option<Self> {
        let w = |i: usize| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        Some(match w(0) {
            KIND_TOUCH => AppEvent::Input(InputEvent::Touch {
                id: (w(1) >> 8) as u8,
                x: w(2) as i32,
                y: w(3) as i32,
                phase: match w(1) & 0xFF {
                    0 => TouchPhase::Down,
                    1 => TouchPhase::Move,
                    2 => TouchPhase::Up,
                    _ => return None,
                },
            }),
            KIND_KEY => AppEvent::Input(InputEvent::Key {
                code: w(2),
                down: w(1) != 0,
            }),
            KIND_QUIT => AppEvent::Quit,
            KIND_BACK => AppEvent::Back,
            KIND_OPTIONS => AppEvent::Options,
            KIND_VISIBLE => AppEvent::Visible(w(1) != 0),
            KIND_SETTINGS => AppEvent::Settings { seq: w(1) },
            KIND_ALARM => AppEvent::Alarm {
                id: u16::try_from(w(1)).ok()?,
            },
            KIND_KEYBOARD => AppEvent::Keyboard { h: w(1) },
            KIND_PAGE => AppEvent::Page {
                page: u16::try_from(w(1)).ok()?,
            },
            KIND_ACTION => AppEvent::Action,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip() {
        let all = [
            AppEvent::Input(InputEvent::Touch {
                id: 2,
                x: -5,
                y: 470,
                phase: TouchPhase::Move,
            }),
            AppEvent::Input(InputEvent::Key {
                code: 'é' as u32,
                down: true,
            }),
            AppEvent::Quit,
            AppEvent::Back,
            AppEvent::Options,
            AppEvent::Visible(true),
            AppEvent::Visible(false),
            AppEvent::Settings { seq: 0 },
            AppEvent::Settings { seq: u32::MAX },
            AppEvent::Alarm { id: 0 },
            AppEvent::Alarm { id: u16::MAX },
            AppEvent::Keyboard { h: 0 },
            AppEvent::Keyboard { h: 369 },
            AppEvent::Page { page: 1 },
            AppEvent::Page { page: u16::MAX },
            AppEvent::Action,
        ];
        for e in all {
            assert_eq!(AppEvent::decode(&e.encode()), Some(e));
        }
        assert_eq!(AppEvent::decode(&[0; EVENT_LEN]), None);
        let mut big_page = AppEvent::Page { page: 0 }.encode();
        big_page[6] = 1; // page 65536
        assert_eq!(AppEvent::decode(&big_page), None);
    }
}

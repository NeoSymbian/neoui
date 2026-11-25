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
//! pages ([`AppEvent::Page`]): the Phone app is one process behind the Phone and Contacts icons. Since 1.5 the
//! keyboard has number and phone pads ([`KEYBOARD_NUMBER`], [`KEYBOARD_PHONE`]), and the user can put it away with
//! the toolbar's keyboard button: the app then gets [`AppEvent::Keyboard`] with `h = 0` (its whole body back) while
//! its mode stays; any [`KEYBOARD`] request with a mode above 0 brings it back. Since 1.6 an app supplies launcher
//! entries ([`ENTRY`], [`AppEvent::Entry`]) and switches between its surface and the shell's screen ([`FRONT`]).
//! Since 1.7 an app describes its page ([`PAGE`], [`crate::page`]) and the shell draws the strip, the Options sheet
//! and dialogs ([`QUERY`]): [`AppEvent::Primary`], [`AppEvent::Cancel`], [`AppEvent::Option`] and
//! [`AppEvent::Answer`] come back. Since 1.8 (Navigation v2, Shell v2) the app draws its own header and footer with
//! the kit (`neoui-kit`), so the page is only its name, Back and full screen; the shell adds toasts ([`NOTE`],
//! [`AppEvent::NoteAction`]), entries in the shade ([`NOTIFY`]), badge counts ([`BADGE`]), a clipboard ([`CLIP_SET`],
//! [`CLIP_GET`]) and tells an app when its area changes height ([`AppEvent::Area`]). 1.7's `TITLE` (11), `TOOLBAR`
//! (12) and the strip's events are gone (`KErrNotSupported`); their numbers stay reserved. [`NAV`] and
//! [`AppEvent::Options`] stay for the legacy bridge (LEGACY.EXE, C++, no kit): the shell draws a header for it. Since
//! 1.9 apps open each other ([`OPEN`], [`ARG`], [`AppEvent::Open`]: Contacts' Call opens Phone on its call page with
//! the number), hand the user back when a task ends ([`DONE`]), and a notification carries actions
//! ([`AppEvent::Notified`]). Since 1.10 an app can ask for a short vibration ([`HAPTIC`]: the old apps' key pads) and is
//! told when the fingers it had were taken away ([`AppEvent::InputReset`]: the screen went off, the app lost the
//! front), and the legacy bridge loses the shell's header once it declares a [`PAGE`]. Since 1.11 a shade entry can
//! carry a progress bar ([`crate::page::Notification::progress`]). Since 1.12 apps pass open files to each other
//! (Files' Open with and its file picker, docs files-app.md FA8): [`OPEN_FILE`] opens an app with a file, which it takes
//! with [`ARG_FILE`]; a picker answers with [`DONE_FILE`] and its opener takes the file with [`RESULT_FILE`]. Files go
//! by handle (`e32::fs::SharedFile`), so a private file or a long path is no problem. The clipboard has a kind
//! ([`CLIP_SET`]'s second argument, [`CLIP_KIND`]): copied files are their paths, one per line, so plain-text readers
//! still paste something. Which apps open which file types is [`crate::open`].
//! Since 1.13 Settings is an app (`NEOSET.EXE`, docs neoui-shell/settings.md SE0): it reads what only the shell
//! knows ([`SETTINGS_INFO`]: battery history, the Wi-Fi link's IP and captive portal, the clock's sync) and asks the
//! shell to act ([`SETTINGS_ACTION`]); its writes to shell-owned settings are allowed by its secure ID.
//! Since 1.14 (SE1d) it also lists the old apps the legacy bridge registered ([`INFO_APPS`]) and opens one
//! ([`ACTION_OPEN_ENTRY`]).

use crate::{InputEvent, TouchPhase};

/// The server's name (no leading `!`: that needs ProtServ).
pub const SERVER: &str = "NeoUi";
/// Protocol version (major, minor); a connect with another major is refused.
pub const VERSION: (u8, u8) = (1, 14);

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
/// `[mode: KEYBOARD_HIDDEN | KEYBOARD_TEXT | KEYBOARD_TERMINAL | KEYBOARD_NUMBER | KEYBOARD_PHONE]` → the height in pixels of the system keyboard now
/// covering the bottom of the surface (0 when hidden). Its keys arrive as [`InputEvent::Key`] like hardware keys
/// (NeoSymbian R6c). The height can change later (language, settings): [`AppEvent::Keyboard`] tells the app.
pub const KEYBOARD: i32 = 5;
pub const KEYBOARD_HIDDEN: i32 = 0;
/// Letters, the symbols layer and the languages on (1.2's `show = 1`).
pub const KEYBOARD_TEXT: i32 = 1;
/// [`KEYBOARD_TEXT`] with a row of Escape (27), Tab (9) and the arrows (Symbian's `EKeyLeftArrow` 0xF807,
/// `EKeyRightArrow` 0xF808, `EKeyUpArrow` 0xF809, `EKeyDownArrow` 0xF80A) on top.
pub const KEYBOARD_TERMINAL: i32 = 2;
/// A PIN or amount pad: 0-9, Backspace (8) and Done (Enter, 13).
pub const KEYBOARD_NUMBER: i32 = 3;
/// A phone pad: 0-9, `*`, `#`, `+`, `p` and `w` (pause and wait), Backspace (8) and Done (Enter, 13).
pub const KEYBOARD_PHONE: i32 = 4;
/// How many keyboard modes there are (hidden included): the length of the server's height table.
pub const KEYBOARD_MODES: usize = 5;
/// `[claims: NAV_BACK | NAV_OPTIONS]` → 0, for the legacy bridge (an app that draws no kit header): Back and the
/// Options key come to the app ([`AppEvent::Back`], [`AppEvent::Options`]); the shell draws ← and ⋮ in a header over
/// it. Claims end with the app.
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
/// The longest launcher caption, page title or notification title, in bytes.
pub const TITLE_MAX: usize = 48;

/// `[id: u32, Des8 caption, Des8 icon]` → 0: add or replace the calling app's launcher entry `id`. The caption is
/// UTF-8, at most [`TITLE_MAX`] bytes; empty removes the entry. The icon is empty (a generic tile), or `w: u16 LE,
/// h: u16 LE` followed by `w * h` u32 LE `0xAARRGGBB` pixels with straight, not premultiplied, alpha. Both dimensions
/// must be in `1..=ENTRY_ICON_MAX`. Malformed captions/icons give `KErrArgument`; adding beyond [`ENTRIES_MAX`]
/// entries per app gives `KErrOverflow`.
pub const ENTRY: i32 = 13;
pub const ENTRY_ICON_MAX: u16 = 128;
pub const ENTRIES_MAX: usize = 64;
/// `[front: 0 | 1]` → 0: 1 shows the calling app's surface on its screen and sends [`AppEvent::Visible`] with `true`;
/// 0 shows the shell's screen for its key again and sends `Visible(false)`. The app keeps running.
pub const FRONT: i32 = 14;
/// `[Des8 page]` ([`crate::page::Page`], at most [`crate::page::PAGE_MAX`] bytes) → the height in pixels of the
/// app's area: the body, or the whole screen for a full-screen page. A malformed page gives `KErrArgument`.
pub const PAGE: i32 = 15;
/// `[Des8 query]` ([`crate::page::Query`], at most [`crate::page::QUERY_MAX`] bytes) → 0: show a dialog over the
/// app; the answer comes as [`AppEvent::Answer`]. An empty descriptor takes the open dialog away unanswered.
pub const QUERY: i32 = 16;
/// `[Des8 note]` ([`crate::page::Note`], at most [`crate::page::NOTE_MAX`] bytes) → 0: a toast above the key row or
/// band for [`NOTE_MS`]; a tap on its action sends [`AppEvent::NoteAction`]. Empty text takes it away.
pub const NOTE: i32 = 17;
pub const NOTE_MS: u64 = 3000;
/// `[Des8 notification]` ([`crate::page::Notification`], at most [`crate::page::NOTIFICATION_MAX`] bytes) → 0: add or
/// replace the app's entry `id` in the shade (an empty title removes it). At most [`NOTIFICATIONS_MAX`] per app
/// (`KErrOverflow`). A tap brings the app to the front with [`AppEvent::Page`].
pub const NOTIFY: i32 = 18;
pub const NOTIFICATIONS_MAX: usize = 16;
/// `[id: u32, count: u32]` → 0: a red count on the app's launcher icon (missed calls, unread messages); `id` names
/// the counter (the app sums them), 0 removes it. Shown as 99+ past 99.
pub const BADGE: i32 = 19;
/// `[Des8 text, kind]` (UTF-8, at most [`CLIP_MAX`] bytes) → 0: the clipboard's text, shared by every app and the
/// shell. `kind` (since 1.12; older apps send 0) says what the text is: [`CLIP_TEXT`], or files to copy or move
/// ([`CLIP_PATHS_COPY`], [`CLIP_PATHS_MOVE`]: full paths, one per line). Another kind gives `KErrArgument`.
pub const CLIP_SET: i32 = 20;
pub const CLIP_TEXT: i32 = 0;
pub const CLIP_PATHS_COPY: i32 = 1;
pub const CLIP_PATHS_MOVE: i32 = 2;
/// `[Des8 out text]` → the text's length (0: empty), written into argument 0; `KErrOverflow` when it doesn't fit.
pub const CLIP_GET: i32 = 21;
pub const CLIP_MAX: usize = 2048;
/// `[Des8 key, page: u16, Des8 arg]` → 0: open app `key` (its launcher key: `"phone"`, `"contacts"`) on page `page`
/// (the target's own numbering) with `arg` (UTF-8, at most [`ARG_MAX`] bytes; a number, an id). The server starts
/// the app or brings it to the front, keeps `arg` for [`ARG`], sends [`AppEvent::Open`] and remembers the caller:
/// Back at the target's root ([`crate::page::Page::can_back`] false) or [`DONE`] returns there. `KErrNotFound` for
/// an unknown key, `KErrArgument` for a malformed one or an arg over the limit. An app may open itself (a footer tab
/// for the app already in front is a no-op on the user's side).
pub const OPEN: i32 = 22;
pub const ARG_MAX: usize = 256;
/// `[Des8 out arg]` → the length of the arg the last [`OPEN`] of this app carried (0: none), written into argument
/// 0; reading takes it. `KErrOverflow` when it doesn't fit.
pub const ARG: i32 = 23;
/// `[]` → 0: the app finished the task it was opened for (a call ended): the server shows the app that opened it, or
/// the previous running app, or home. The app keeps running.
pub const DONE: i32 = 24;
/// `[ms, amplitude]` → 0: a short vibration (a key pad's tick), at most [`HAPTIC_MS_MAX`] ms and amplitude 0..=127.
/// `KErrNotSupported` on a device without a vibration motor.
pub const HAPTIC: i32 = 25;
pub const HAPTIC_MS_MAX: i32 = 50;
/// `[Des8 key, page: u16, handle fs, file]` → 0: [`OPEN`] with an open file instead of an arg: the opener puts the
/// file in slots 2 and 3 (`File::transfer_to_server`; its session shared with `Fs::share_protected`). The target
/// takes it with [`ARG_FILE`]. Same errors as `OPEN`, or the file's (`KErrBadHandle`, `KErrPermissionDenied`).
pub const OPEN_FILE: i32 = 26;
/// `[Des8 out file: 4 bytes]` → a file-server session handle, with the file's handle written into argument 0
/// (`SharedFile::adopt_from_server`): the file the last [`OPEN_FILE`] of this app carried; taking it removes it.
/// `KErrNotFound` when there is none.
pub const ARG_FILE: i32 = 27;
/// `[handle fs, file]` → 0: [`DONE`] with a result: a file picked or saved for the app that opened this one (the
/// file in slots 0 and 1, as for [`OPEN_FILE`]). That app takes it with [`RESULT_FILE`] when it is shown again.
pub const DONE_FILE: i32 = 28;
/// `[Des8 out file: 4 bytes]` → as [`ARG_FILE`], for the file the app this one opened answered with ([`DONE_FILE`]).
pub const RESULT_FILE: i32 = 29;
/// `[]` → the clipboard's kind ([`CLIP_TEXT`] when empty or set by a 1.11 app).
pub const CLIP_KIND: i32 = 30;
/// `[kind, cursor: unused (0), Des8 out]` → the snapshot's length: something only the shell knows, encoded by
/// [`crate::settings_app::info`] ([`INFO_BATTERY`], [`INFO_NET`], [`INFO_CLOCK`], [`INFO_APPS`]). `KErrOverflow` when `out` is
/// too short, `KErrArgument` for an unknown kind. Opcodes 31–39 are reserved (docs neoui-shell/protocol.md).
pub const SETTINGS_INFO: i32 = 40;
/// `[kind, Des8 request]` → 0: ask the shell to act. [`ACTION_SIGN_IN`]: open the browser on the current Wi-Fi
/// network's sign-in page (`KErrNotReady` when there is no captive portal now). [`ACTION_OPEN_ENTRY`] (1.14,
/// Settings only, else `KErrPermissionDenied`): request = an id (u32 LE) of [`INFO_APPS`]; the legacy bridge opens that
/// old app, as a tap in its grid does (`KErrNotFound` for an id it hasn't registered).
pub const SETTINGS_ACTION: i32 = 41;
/// [`SETTINGS_INFO`] kinds.
pub const INFO_BATTERY: i32 = 1;
pub const INFO_NET: i32 = 2;
pub const INFO_CLOCK: i32 = 3;
/// 1.14: the old apps the legacy bridge registered with [`ENTRY`] ([`crate::settings_app::info::OldApps`]).
pub const INFO_APPS: i32 = 4;
/// [`SETTINGS_ACTION`] kinds.
pub const ACTION_SIGN_IN: i32 = 1;
pub const ACTION_OPEN_ENTRY: i32 = 2;
/// The longest launcher key, in bytes.
pub const KEY_MAX: usize = 16;

/// A malformed [`ENTRY`] icon (`KErrArgument`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryIconError;

/// Decode an [`ENTRY`] icon without allocating. The returned bytes are the unchanged little-endian ARGB pixels.
/// An empty icon returns `(0, 0, &[])`, asking the shell for a generic tile.
///
/// # Errors
/// Returns [`EntryIconError`] for an incomplete header, invalid dimensions, or a pixel length other than `w * h * 4`.
pub fn decode_entry_icon(bytes: &[u8]) -> Result<(u16, u16, &[u8]), EntryIconError> {
    if bytes.is_empty() {
        return Ok((0, 0, bytes));
    }
    let header = bytes.get(..4).ok_or(EntryIconError)?;
    let w = u16::from_le_bytes([header[0], header[1]]);
    let h = u16::from_le_bytes([header[2], header[3]]);
    if !(1..=ENTRY_ICON_MAX).contains(&w) || !(1..=ENTRY_ICON_MAX).contains(&h) {
        return Err(EntryIconError);
    }
    let pixels = &bytes[4..];
    if pixels.len() != usize::from(w) * usize::from(h) * 4 {
        return Err(EntryIconError);
    }
    Ok((w, h, pixels))
}

/// Encoded size of an [`AppEvent`].
pub const EVENT_LEN: usize = 16;

/// What the server tells an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// Input, positions relative to the surface.
    Input(InputEvent),
    /// Leave now (closed in Open apps, or more than `MAX_RUNNING` apps run); the app should exit.
    Quit,
    /// Back (the key row's ←, an edge gesture), while the page has `can_back` (or [`NAV_BACK`] is claimed).
    Back,
    /// The legacy bridge's Options key: ⋮ in the header the shell draws for it ([`NAV_OPTIONS`]).
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
    /// The user tapped launcher entry `id` ([`ENTRY`]).
    Entry { id: u32 },
    /// Button `i` of the dialog ([`QUERY`]); Back answers the last button.
    Answer { i: u16 },
    /// The user tapped the toast's action ([`NOTE`]).
    NoteAction,
    /// The app's area is now `h` pixels tall (the navigation mode changed: Buttons 639 dp, Gestures 679 dp); the
    /// app resizes its window and redraws. A full-screen page keeps the whole screen.
    Area { h: u32 },
    /// Another app (or this one) opened it on `page` ([`OPEN`]); [`ARG`] gives the argument.
    Open { page: u16 },
    /// The user tapped the app's notification `id` ([`NOTIFY`]): `action` 0 is the entry itself, `i + 1` its
    /// action `i`.
    Notified { id: u32, action: u16 },
    /// The fingers the app had are gone without their lifts (the screen went off, the app lost the front): whatever
    /// it holds for them (an on-screen key down) is released now.
    InputReset,
}

const KIND_TOUCH: u32 = 1;
const KIND_KEY: u32 = 2;
const KIND_QUIT: u32 = 3;
const KIND_BACK: u32 = 4;
const KIND_OPTIONS: u32 = 5;
// 11 (1.7's Action), 13 (Primary), 14 (Cancel) and 15 (Option) are retired
const KIND_VISIBLE: u32 = 6;
const KIND_SETTINGS: u32 = 7;
const KIND_ALARM: u32 = 8;
const KIND_KEYBOARD: u32 = 9;
const KIND_PAGE: u32 = 10;
const KIND_ENTRY: u32 = 12;
const KIND_ANSWER: u32 = 16;
const KIND_NOTE_ACTION: u32 = 17;
const KIND_AREA: u32 = 18;
const KIND_OPEN: u32 = 19;
const KIND_NOTIFIED: u32 = 20;
const KIND_INPUT_RESET: u32 = 21;

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
            AppEvent::Entry { id } => [KIND_ENTRY, id, 0, 0],
            AppEvent::Answer { i } => [KIND_ANSWER, u32::from(i), 0, 0],
            AppEvent::NoteAction => [KIND_NOTE_ACTION, 0, 0, 0],
            AppEvent::Area { h } => [KIND_AREA, h, 0, 0],
            AppEvent::Open { page } => [KIND_OPEN, u32::from(page), 0, 0],
            AppEvent::Notified { id, action } => [KIND_NOTIFIED, id, u32::from(action), 0],
            AppEvent::InputReset => [KIND_INPUT_RESET, 0, 0, 0],
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
            KIND_ENTRY => AppEvent::Entry { id: w(1) },
            KIND_ANSWER => AppEvent::Answer {
                i: u16::try_from(w(1)).ok()?,
            },
            KIND_NOTE_ACTION => AppEvent::NoteAction,
            KIND_AREA => AppEvent::Area { h: w(1) },
            KIND_OPEN => AppEvent::Open {
                page: u16::try_from(w(1)).ok()?,
            },
            KIND_NOTIFIED => AppEvent::Notified {
                id: w(1),
                action: u16::try_from(w(2)).ok()?,
            },
            KIND_INPUT_RESET => AppEvent::InputReset,
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
            AppEvent::Entry { id: 0 },
            AppEvent::Entry { id: u32::MAX },
            AppEvent::Answer { i: 2 },
            AppEvent::NoteAction,
            AppEvent::Area { h: 0 },
            AppEvent::Area { h: 2037 },
            AppEvent::Open { page: 0 },
            AppEvent::Open { page: 3 },
            AppEvent::Notified { id: 7, action: 0 },
            AppEvent::InputReset,
            AppEvent::Notified {
                id: u32::MAX,
                action: 2,
            },
        ];
        for e in all {
            assert_eq!(AppEvent::decode(&e.encode()), Some(e));
        }
        assert_eq!(AppEvent::decode(&[0; EVENT_LEN]), None);
        let mut big_page = AppEvent::Page { page: 0 }.encode();
        big_page[6] = 1; // page 65536
        assert_eq!(AppEvent::decode(&big_page), None);
        let mut big_answer = AppEvent::Answer { i: 0 }.encode();
        big_answer[6] = 1;
        assert_eq!(AppEvent::decode(&big_answer), None);
        // 1.7's retired kinds are not events any more
        for kind in [11u8, 13, 14, 15] {
            let mut b = [0; EVENT_LEN];
            b[0] = kind;
            assert_eq!(AppEvent::decode(&b), None, "kind {kind}");
        }
    }

    #[test]
    fn entry_event_wire_format() {
        let bytes = [12, 0, 0, 0, 0x78, 0x56, 0x34, 0xe2, 0, 0, 0, 0, 0, 0, 0, 0];
        let event = AppEvent::Entry { id: 0xe2345678 };
        assert_eq!(event.encode(), bytes);
        assert_eq!(AppEvent::decode(&bytes), Some(event));
    }

    #[test]
    fn entry_icons_decode() {
        assert_eq!(decode_entry_icon(&[]), Ok((0, 0, &[][..])));
        // A 2 x 1 icon: preserve straight alpha and little-endian channel bytes, including transparent pixels.
        let bytes = [2, 0, 1, 0, 0x33, 0x22, 0x11, 0x80, 0xff, 0xff, 0xff, 0];
        assert_eq!(decode_entry_icon(&bytes), Ok((2, 1, &bytes[4..])));
        let mut largest = [0u8; 4 + 128 * 128 * 4];
        largest[..4].copy_from_slice(&[128, 0, 128, 0]);
        assert_eq!(decode_entry_icon(&largest), Ok((128, 128, &largest[4..])));
    }

    #[test]
    fn malformed_entry_icons_are_rejected() {
        for bytes in [&[1][..], &[1, 0], &[1, 0, 1]] {
            assert_eq!(decode_entry_icon(bytes), Err(EntryIconError));
        }
        for (w, h) in [
            (0u16, 1u16),
            (1, 0),
            (129, 1),
            (1, 129),
            (u16::MAX, u16::MAX),
        ] {
            let mut bytes = [0; 8];
            bytes[..2].copy_from_slice(&w.to_le_bytes());
            bytes[2..4].copy_from_slice(&h.to_le_bytes());
            assert_eq!(decode_entry_icon(&bytes), Err(EntryIconError));
        }
        let bytes = [1, 0, 1, 0, 0, 0, 0, 0, 0];
        for len in [4, 5, 6, 7, 9] {
            assert_eq!(decode_entry_icon(&bytes[..len]), Err(EntryIconError));
        }
    }
}

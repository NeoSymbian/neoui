// SPDX-License-Identifier: GPL-3.0-only
//! The system keyboard's side of the main loop (W1): what its timers typed between touches (a held key's repeats),
//! its log, the shell fields' clipboard, and a hardware keyboard's keys through `neoui_proto::hwkeys`.

use e32::dprintln;
use e32::event::{TRawEvent, raw};
use neoui_proto::InputEvent;
use neoui_proto::ipc::{AppEvent, CLIP_TEXT};
use neoui_render::Renderer;

use crate::host::Host;

/// After each frame: the keys the keyboard's timers typed go to the app in front (a shell field got them in the
/// render), its log goes to the serial log, and the shell's fields share the apps' clipboard.
pub(crate) fn after_frame(ui: &mut Renderer, host: &mut Host) {
    for code in ui.take_typed() {
        host.send(AppEvent::Input(InputEvent::Key { code, down: true }));
    }
    for line in ui.take_keyboard_log() {
        dprintln!("neoui: key {}", line);
    }
    if let Some(t) = ui.take_copied() {
        host.clipboard = t;
        host.clip_kind = CLIP_TEXT;
    }
    ui.set_clipboard(if host.clip_kind == CLIP_TEXT {
        &host.clipboard
    } else {
        ""
    });
}

/// A raw key event as a key code, in the on-screen keyboard's current language (`hwkeys` keeps Shift, Caps Lock
/// and Ctrl); `None` for a modifier or a key that types nothing.
pub(crate) fn hardware_key(
    ev: &TRawEvent,
    hw: &mut neoui_proto::hwkeys::State,
    ui: &Renderer,
) -> Option<InputEvent> {
    let down = match ev.i_type {
        raw::EKEY_DOWN => true,
        raw::EKEY_UP => false,
        _ => return None,
    };
    hw.key(ev.scan_code(), down, &ui.keyboard_code())
        .map(|code| InputEvent::Key { code, down })
}

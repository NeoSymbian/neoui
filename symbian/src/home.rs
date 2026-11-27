// SPDX-License-Identifier: Apache-2.0
//! (D10) The shell side of Settings' Edit home (`ipc::ACTION_EDIT_HOME`) and (G3) Open apps' thumbnails: a child of
//! `host` (it works on the apps there). The editor itself is the renderer's (neoui-render `home.rs`); the main loop
//! hands it what is asked here.

use super::Host;
use alloc::vec::Vec;
use e32::dprintln;
use e32::server::Message;
use neoui_proto::settings::{Caller, access, kerr};
use neoui_proto::settings_app::wire;

/// What the main loop takes over each frame.
#[derive(Default)]
pub struct D10 {
    /// Settings asked for the home editor.
    pub edit_home: bool,
    /// Thumbnails taken as apps went to the back: app key, width, height, XRGB8888 pixels.
    pub thumbs: Vec<(&'static str, usize, usize, Vec<u32>)>,
}

/// A thumbnail is at most this many pixels wide (the top of the app, 3:2 tall): about 0.3 MB on the Pixel.
const THUMB_MAX_W: usize = 320;

impl Host {
    /// `SETTINGS_ACTION` `ACTION_EDIT_HOME`: Settings only, an empty request.
    pub(super) fn edit_home_action(&mut self, key: &str, m: &Message) -> i32 {
        if access::caller(key, m.client_sid()) != Caller::Settings {
            return kerr::PERMISSION_DENIED;
        }
        // at most one byte read: anything there is already too much
        let mut b = [0u8; 1];
        let n = match m.des_len(1) {
            Ok(0) => 0,
            Ok(_) => m.read8(1, &mut b, 0).unwrap_or(1).max(1),
            Err(_) => return kerr::ARGUMENT,
        };
        if !wire::edit_home_request(&b[..n]) {
            return kerr::ARGUMENT;
        }
        dprintln!("neoui: settings opens the home editor");
        self.d10.edit_home = true;
        0
    }

    /// The front app is going to the back: its body as it shows now, downscaled, for Open apps (before its frame is
    /// released, so it is the last frame shown). None while it has the keyboard up (a password or PIN may show).
    pub(super) fn take_thumb(&mut self) {
        let area = self.app_area();
        let Some(app) = self.front() else { return };
        if app.keyboard != 0 {
            return;
        }
        let Some(surface) = app.surface.as_ref() else {
            return;
        };
        let (key, stride) = (app.key, self.body.w as usize);
        let (w, rows) = (area.w as usize, area.h as usize);
        if w == 0 || rows == 0 || w > stride {
            return;
        }
        // SAFETY: the chunk is `stride * surface_rows * 4` bytes, committed at creation and mapped here; the app only
        // writes pixels.
        let src = unsafe {
            core::slice::from_raw_parts(surface.base().cast::<u32>(), stride * self.surface_rows())
        };
        let k = w.div_ceil(THUMB_MAX_W);
        let (tw, th, px) = neoui_render::layer::downsample(src, stride, w, rows.min(w * 3 / 2), k);
        self.d10.thumbs.retain(|t| t.0 != key);
        self.d10.thumbs.push((key, tw, th, px));
    }
}

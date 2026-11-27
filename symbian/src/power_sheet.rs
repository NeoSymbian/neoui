// SPDX-License-Identifier: GPL-3.0-only
//! The Power menu's shell side (H3a; v2 = POWER, O5, docs neoui-shell/top-panel.md "Power menu v2"): opening it
//! (the sound profiles for both styles, the blurred screen under the v2 sheet) and what its buttons ask for
//! (`neoui_proto::power_menu::act`). The sheet itself is `ui/components/power-sheet.slint`; the short list
//! (`power.style` 1) is `powermenu.slint`.

use crate::host::Host;
use crate::{app_row, front_surface};
use alloc::vec::Vec;
use e32::dprintln;
use e32::fs::{File, Fs, mode};
use e32::sys::err;
use neoui_proto::page::Note;
use neoui_proto::power_menu::{self, act};
use neoui_proto::sound_profile;
use neoui_render::Renderer;

/// What the main loop does after an action.
pub enum Then {
    /// The menu stays (a cancelled hold, the ring tile).
    Stay,
    Close,
    /// Restart, fastboot or power off (`ipc::POWER_*`).
    Power(i32),
    /// Close it and turn the screen off.
    Lock,
}

#[derive(Default)]
pub struct Sheet {
    /// A screenshot waits for the first frame without the menu.
    shot: bool,
}

/// The sound profiles, the active one first (the short list shows as many as fit; the sheet names the first).
fn show_profiles(ui: &mut Renderer, host: &Host) {
    let p = sound_profile::store::load(&host.settings);
    let mut names: Vec<(u8, &str)> = p.list.iter().map(|x| (x.id, x.name.as_str())).collect();
    names.sort_by_key(|(id, _)| *id != p.active);
    ui.set_power_profiles(&names, p.active);
}

/// The screen as it shows now, row `y`: the shell frame with the front app's surface over it.
fn screen_row(host: &Host, frame: &[u32], w: usize, y: usize, out: &mut [u32]) {
    out.copy_from_slice(&frame[y * w..][..w]);
    app_row(front_surface(host), y, out);
}

impl Sheet {
    pub fn open(&mut self, ui: &mut Renderer, host: &Host, frame: &[u32], (w, h): (usize, usize)) {
        dprintln!("neoui: power menu");
        show_profiles(ui, host);
        ui.set_power_backdrop(w, h, |y, out| screen_row(host, frame, w, y, out));
        ui.set_power_menu(true);
    }

    /// One action of the menu's.
    pub fn chosen(&mut self, a: i32, ui: &mut Renderer, host: &mut Host, ms: u64) -> Then {
        match a {
            act::CANCELLED => {
                dprintln!("neoui: power: cancelled");
                Then::Stay
            }
            act::RING => {
                let p = sound_profile::store::load(&host.settings);
                for c in power_menu::next_ring(&p) {
                    let r = sound_profile::store::apply(&mut host.settings, c, ms);
                    if r.is_err() {
                        dprintln!("neoui: power: ring tile ({:?})", r);
                    }
                }
                let p = sound_profile::store::load(&host.settings);
                dprintln!("neoui: power: ring {:?}", power_menu::ring(p.mirrors()));
                show_profiles(ui, host);
                Then::Stay
            }
            act::LOCK => {
                dprintln!("neoui: power: lock now");
                Then::Lock
            }
            act::SCREENSHOT => {
                self.shot = true;
                Then::Close
            }
            act::EMERGENCY => {
                // shortcut: Phone's ordinary dialer, which can't place emergency calls yet (phone.rs `dial`); point
                // this at TP9's restricted emergency dialer once it exists
                dprintln!("neoui: power: emergency call");
                ui.show("phone", "dial");
                Then::Close
            }
            act::RESTART | act::FASTBOOT | act::OFF => Then::Power(a),
            a if a >= act::PROFILE => {
                // the short list's profile rows (SE2a)
                let r = sound_profile::store::apply(
                    &mut host.settings,
                    sound_profile::Command::Choose((a - act::PROFILE) as u8),
                    ms,
                );
                dprintln!("neoui: power menu profile {} ({:?})", a - act::PROFILE, r);
                Then::Close
            }
            _ => {
                dprintln!("neoui: power menu closed");
                Then::Close
            }
        }
    }

    /// After a frame was drawn: a screenshot asked for, once the menu is gone, of the screen as it shows.
    pub fn after_frame(&mut self, host: &mut Host, frame: &[u32], (w, h): (usize, usize)) {
        if !core::mem::take(&mut self.shot) {
            return;
        }
        let png = neoui_proto::png_write::encode(w, h, |y, out| screen_row(host, frame, w, y, out));
        let t = e32::time::now();
        let path = power_menu::shot_path(t.year, t.month, t.day, t.hour, t.minute, t.second);
        let r = save(&path, &png);
        dprintln!(
            "neoui: power: screenshot {} ({} bytes, {:?})",
            path,
            png.len(),
            r
        );
        let text = if r.is_ok() {
            "Screenshot saved to Pictures"
        } else {
            "Couldn't save the screenshot"
        };
        host.note = Some((
            "shell",
            Note {
                text: text.into(),
                action: alloc::string::String::new(),
            },
            e32::time::millis() + neoui_proto::ipc::NOTE_MS,
        ));
        host.notes_changed = true;
    }
}

fn save(path: &str, data: &[u8]) -> Result<(), i32> {
    let fs = Fs::connect()?;
    for dir in ["C:\\Data\\", power_menu::SHOT_DIR] {
        match fs.mkdir(dir) {
            Ok(()) | Err(err::KERR_ALREADY_EXISTS) => {}
            Err(e) => return Err(e),
        }
    }
    let f = File::replace(&fs, path, mode::SHARE_EXCLUSIVE)?;
    f.write(data)?;
    f.flush()
}

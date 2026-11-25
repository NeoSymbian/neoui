// SPDX-License-Identifier: GPL-3.0-only
//! Settings -> Sound -> Media (docs/hardware/audio.md A6): the media volume and route go to NeoAudio (MODEMD), and the
//! volume keys set the media volume outside a call. The test buttons are the Settings app's (NEOSET).
use crate::settings::Settings;
use e32::dprintln;
use e32::ipc::{Session, TIpcArgs, TVersion};
use neoui_proto::settings::registry::keys;
use neoui_proto::settings::{Caller, Value};

pub struct Sound {
    /// the (volume, route, mic) NeoAudio has
    applied: Option<(i32, i32, i32)>,
    retry_at: u64,
}

fn session() -> Result<Session, i32> {
    let (major, minor, build) = neoaudio::VERSION;
    Session::connect(
        neoaudio::NAME,
        TVersion {
            major: major as u8,
            minor: minor as u8,
            build: build as u16,
        },
    )
}

fn call(s: &Session, op: i32, a0: i32) -> Result<i32, i32> {
    let mut a = TIpcArgs::new();
    a.int(0, a0);
    s.send_receive(op, &a)
}

impl Sound {
    pub fn new() -> Self {
        Sound {
            applied: None,
            retry_at: 0,
        }
    }

    /// Every loop: the settings to NeoAudio (again after it was unavailable).
    pub fn poll(&mut self, settings: &Settings, now: u64) {
        let want = (
            settings
                .get(keys::SOUND_MEDIA_VOLUME)
                .unwrap_or(7)
                .clamp(0, 10),
            settings
                .get(keys::SOUND_MEDIA_ROUTE)
                .unwrap_or(0)
                .clamp(0, 2),
            settings.get(keys::SOUND_MIC).unwrap_or(0).clamp(0, 1),
        );
        if self.applied != Some(want) && now >= self.retry_at {
            let r = session().and_then(|s| {
                call(&s, neoaudio::op::MASTER, want.0)?;
                call(&s, neoaudio::op::ROUTE, want.1)?;
                call(&s, neoaudio::op::MIC, want.2)
            });
            match r {
                Ok(_) => {
                    dprintln!(
                        "neoui: media volume {} route {} mic {}",
                        want.0,
                        want.1,
                        want.2
                    );
                    self.applied = Some(want);
                }
                Err(_) => self.retry_at = now + 2_000, // MODEMD not up yet
            }
        }
    }

    /// A volume key with no call: the media volume one step up or down (the store shows it in Settings).
    pub fn volume_key(&mut self, up: bool, settings: &mut Settings, now: u64) -> bool {
        let v = settings.get(keys::SOUND_MEDIA_VOLUME).unwrap_or(7);
        let v = if up { (v + 1).min(10) } else { (v - 1).max(0) };
        let _ = crate::settings::set(
            settings,
            Caller::Shell,
            keys::SOUND_MEDIA_VOLUME.name(),
            Value::Int(v),
            now,
        );
        true
    }
}

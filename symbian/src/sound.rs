// SPDX-License-Identifier: GPL-3.0-only
//! Settings -> Sound -> Media (docs/audio.md A6): the media volume and route go to NeoAudio (MODEMD), the volume keys
//! set the media volume outside a call, and the test buttons play a test sound or record 5 s and play it back, on a
//! thread of their own (the UI keeps running), reporting through a shared state the main loop shows.
use crate::settings::Settings;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sync::Mutex;
use e32::sys::{TPtr8, TPtrC8};
use e32::{dprintln, time};
use neoui_proto::settings::registry::keys;
use neoui_proto::settings::{Caller, Value};
use neoui_render::Renderer;

/// What a test does: 0 nothing yet, 1 playing, 2 recording, 3 playing back, 4 done, negative = failed with that error.
type State = &'static Mutex<i32>;

pub struct Sound {
    /// the (volume, route) NeoAudio has
    applied: Option<(i32, i32)>,
    retry_at: u64,
    state: Option<State>,
    shown: i32,
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
            state: Mutex::new(0).ok().map(|m| &*Box::leak(Box::new(m))),
            shown: 0,
        }
    }

    /// Every loop: the settings to NeoAudio (again after it was unavailable), the test buttons, their result.
    pub fn poll(&mut self, settings: &Settings, ui: &mut Renderer, now: u64) {
        let want = (
            settings
                .get(keys::SOUND_MEDIA_VOLUME)
                .unwrap_or(7)
                .clamp(0, 10),
            settings
                .get(keys::SOUND_MEDIA_ROUTE)
                .unwrap_or(0)
                .clamp(0, 2),
        );
        if self.applied != Some(want) && now >= self.retry_at {
            let r = session().and_then(|s| {
                call(&s, neoaudio::op::MASTER, want.0)?;
                call(&s, neoaudio::op::ROUTE, want.1)
            });
            match r {
                Ok(_) => {
                    dprintln!("neoui: media volume {} route {}", want.0, want.1);
                    self.applied = Some(want);
                }
                Err(_) => self.retry_at = now + 2_000, // MODEMD not up yet
            }
        }
        for action in ui.take_sound_actions() {
            let Some(state) = self.state else {
                ui.set_sound_feedback("No test without a mutex");
                continue;
            };
            if matches!(*state.lock(), 1..=3) {
                continue; // one test at a time
            }
            *state.lock() = if action == "record" { 2 } else { 1 };
            let record = action == "record";
            if let Err(e) = e32::thread::spawn(move || {
                let r = if record {
                    record_test(state)
                } else {
                    test_sound()
                };
                *state.lock() = if r < 0 { r } else { 4 };
                0
            }) {
                *state.lock() = e;
            }
        }
        if let Some(state) = self.state {
            let s = *state.lock();
            if s != self.shown {
                self.shown = s;
                ui.set_sound_feedback(&feedback(s));
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

fn feedback(s: i32) -> String {
    match s {
        0 => String::new(),
        1 => "Playing the test sound…".into(),
        2 => "Recording 5 s: speak now…".into(),
        3 => "Playing the recording back…".into(),
        4 => "Done".into(),
        -1 => "No audio service (MODEMD)".into(),
        e => format!("Sound test failed ({e})"),
    }
}

/// Open a stream on its own session.
fn open(rate: u32, channels: u32, record: bool) -> Result<Session, i32> {
    let s = session()?;
    let mut a = TIpcArgs::new();
    a.int(0, rate as i32)
        .int(1, channels as i32)
        .int(2, if record { neoaudio::flag::RECORD } else { 0 });
    s.send_receive(neoaudio::op::OPEN, &a)?;
    Ok(s)
}

/// Write all of `pcm`, then wait for the queue to play out.
fn play(s: &Session, pcm: &[u8]) -> Result<(), i32> {
    for chunk in pcm.chunks(8_192) {
        let des = TPtrC8::new(chunk);
        let mut a = TIpcArgs::new();
        a.desc8(0, &des);
        s.send_receive(neoaudio::op::WRITE, &a)?;
    }
    for _ in 0..100 {
        let mut b = [0u8; neoaudio::Info::SIZE];
        let mut des = TPtr8::new(&mut b);
        let mut a = TIpcArgs::new();
        a.des8(0, &mut des);
        s.send_receive(neoaudio::op::INFO, &a)?;
        let info = neoaudio::Info::from_bytes(&b).unwrap_or_default();
        if info.queued == 0 {
            dprintln!(
                "neoui: sound test: {} frames, {} underruns, sink {}",
                info.frames,
                info.underruns,
                info.sink
            );
            break;
        }
        time::sleep(50_000);
    }
    time::sleep(150_000);
    Ok(())
}

/// A short rising chime (C5 E5 G5 C6) at 44.1 kHz stereo: the resampler and the mixer on the way.
fn test_sound() -> i32 {
    let r = open(44_100, 2, false).and_then(|s| {
        let mut pcm = Vec::new();
        for f in [523, 659, 784, 1047] {
            pcm.extend(neoaudio::tone(f, 220, 44_100, 2, 9_000));
        }
        play(&s, &pcm)
    });
    dprintln!("neoui: sound test sound -> {:?}", r);
    r.map_or_else(|e| e, |_| 0)
}

/// Record 5 s (48 kHz mono, the mic as it is), then play it back.
fn record_test(state: State) -> i32 {
    let r = (|| {
        let rec = open(48_000, 1, true)?;
        let mut pcm = alloc::vec![0u8; 48_000 * 2 * 5];
        let mut got = 0;
        while got < pcm.len() {
            let n = (pcm.len() - got).min(16_384);
            let mut des = TPtr8::new(&mut pcm[got..got + n]);
            let mut a = TIpcArgs::new();
            a.des8(0, &mut des);
            let r = rec.send_receive(neoaudio::op::READ, &a)?;
            got += r as usize;
        }
        drop(rec);
        let peak = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b).unsigned_abs())
            .max()
            .unwrap_or(0);
        dprintln!("neoui: sound test: recorded {} bytes, peak {}", got, peak);
        *state.lock() = 3;
        let s = open(48_000, 1, false)?;
        play(&s, &pcm)
    })();
    dprintln!("neoui: sound test record -> {:?}", r);
    r.map_or_else(|e| e, |_| 0)
}

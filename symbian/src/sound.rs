// SPDX-License-Identifier: GPL-3.0-only
//! Settings -> Sound -> Media (docs/hardware/audio.md A6): the media volume and route go to NeoAudio (MODEMD), and the
//! volume keys set the media volume outside a call. The test buttons are the Settings app's (NEOSET).
use crate::settings::Settings;
use e32::dprintln;
use e32::ipc::{Session, TIpcArgs, TVersion};
use neoui_proto::settings::registry::keys;
use neoui_proto::settings::{Caller, Value};

/// (D6) A dial key's tone is playing: one at a time, a key pressed meanwhile is silent.
static TONE_BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// (D6) Answer an accepted control as the active profile says (`effects::FeedbackPolicy`): a pulse with vibrate on
/// touch, a dial key's DTMF pair with keypad tones (out of a call, in a profile that rings at a volume above 0).
pub fn feedback(settings: &Settings, in_call: bool, kind: i32, key: u8) {
    let policy = neoui_proto::effects::FeedbackPolicy {
        touch: settings.get(keys::SOUND_TOUCH_VIBRATE).unwrap_or(false),
        keypad: settings.get(keys::SOUND_KEYPAD_TONES).unwrap_or(false),
        audible: !settings.get(keys::SOUND_SILENT).unwrap_or(false)
            && settings.get(keys::SOUND_RING_VOLUME).unwrap_or(8) > 0,
        in_call,
    };
    let answer = policy.answer(kind, key);
    if let Some((ms, amp)) = answer.pulse {
        let r = crate::haptic(ms, amp);
        if r != 0 {
            dprintln!("neoui: touch pulse -> {}", r);
        }
    }
    if let Some((low, high)) = answer.tone
        && !TONE_BUSY.swap(true, core::sync::atomic::Ordering::AcqRel)
    {
        // a thread of its own: NeoAudio's writes wait for the queue, the UI doesn't
        let r = e32::thread::spawn(move || {
            let r = play_dtmf(low, high);
            if r.is_err() {
                dprintln!("neoui: key tone {}+{} Hz -> {:?}", low, high, r);
            }
            TONE_BUSY.store(false, core::sync::atomic::Ordering::Release);
            0
        });
        if r.is_err() {
            TONE_BUSY.store(false, core::sync::atomic::Ordering::Release);
        }
    }
}

/// 120 ms of a DTMF pair, 48 kHz mono, on a media stream of its own (closed when the session goes).
fn play_dtmf(low: u32, high: u32) -> Result<(), i32> {
    let rate = neoaudio::RATE;
    let (a, b) = (
        neoaudio::tone(low, 120, rate, 1, 5_000),
        neoaudio::tone(high, 120, rate, 1, 5_000),
    );
    let pcm: alloc::vec::Vec<u8> = a
        .as_chunks::<2>()
        .0
        .iter()
        .zip(b.as_chunks::<2>().0)
        .flat_map(|(x, y)| {
            (i16::from_le_bytes(*x).saturating_add(i16::from_le_bytes(*y))).to_le_bytes()
        })
        .collect();
    let s = session()?;
    let mut args = TIpcArgs::new();
    args.int(0, rate as i32).int(1, 1).int(2, 0);
    s.send_receive(neoaudio::op::OPEN, &args)?;
    for chunk in pcm.chunks(8_192) {
        let des = e32::sys::TPtrC8::new(chunk);
        let mut args = TIpcArgs::new();
        args.desc8(0, &des);
        s.send_receive(neoaudio::op::WRITE, &args)?;
    }
    // let the queue play out before the session (and its stream) goes
    e32::time::sleep(200_000);
    Ok(())
}

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

/// (TP4) A card's chime: the first second of `tone` (the profile's message tones, `neoaudio::tones`) at `volume`
/// 0..=10, on a thread of its own (the UI keeps running). One at a time: a chime while one plays is dropped.
pub fn chime(tone: u8, volume: u8) {
    use core::sync::atomic::{AtomicBool, Ordering};
    static PLAYING: AtomicBool = AtomicBool::new(false);
    if tone == neoaudio::tones::NONE || volume == 0 || PLAYING.swap(true, Ordering::SeqCst) {
        return;
    }
    let r = e32::thread::spawn(move || {
        // (D6) a sound of the user's: its file, as AUDIOD rings it (the built-in tone when it is missing)
        let pcm = match neoaudio::tones::file(tone) {
            Some(path) => your_pcm(&path, volume)
                .unwrap_or_else(|| neoaudio::tones::pcm(neoaudio::tones::BUILT_IN, volume)),
            None => neoaudio::tones::pcm(tone, volume),
        };
        let r = session().and_then(|s| {
            let mut a = TIpcArgs::new();
            a.int(0, neoaudio::tones::RATE as i32).int(1, 1).int(2, 0);
            s.send_receive(neoaudio::op::OPEN, &a)?;
            // a second is the chime (the period's rest is the loop's silence)
            for chunk in pcm[..pcm.len().min(96_000)].chunks(8_192) {
                let des = e32::sys::TPtrC8::new(chunk);
                let mut a = TIpcArgs::new();
                a.desc8(0, &des);
                s.send_receive(neoaudio::op::WRITE, &a)?;
            }
            // let the queue play out before the session (and its stream) closes
            e32::time::sleep(1_200_000);
            Ok(())
        });
        if let Err(e) = r {
            dprintln!("neoui: chime failed ({})", e);
        }
        PLAYING.store(false, Ordering::SeqCst);
        0
    });
    if let Err(e) = r {
        dprintln!("neoui: chime thread failed ({})", e);
        PLAYING.store(false, Ordering::SeqCst);
    }
}

/// (D6) One of the user's sounds (NEOSET's `tones\<id>.pcm`) at `volume`: exactly one period, else `None`.
fn your_pcm(path: &str, volume: u8) -> Option<alloc::vec::Vec<u8>> {
    use e32::fs::{File, Fs, mode};
    let fs = Fs::connect().ok()?;
    let f = File::open(&fs, path, mode::READ | mode::SHARE_READERS_ONLY).ok()?;
    let mut b = alloc::vec![0u8; neoaudio::tones::BYTES + 1];
    let n = f.read_at(0, &mut b).ok()?;
    if n != neoaudio::tones::BYTES {
        return None;
    }
    b.truncate(n);
    neoaudio::tones::scale(&mut b, volume);
    Some(b)
}

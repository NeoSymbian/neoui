// SPDX-License-Identifier: GPL-3.0-only
//! Sounds → Media's test buttons (moved from the UI server with Settings): a test sound, or 5 s recorded and played
//! back, on a thread of their own (the UI keeps running), reporting through a shared state the page shows.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sync::Mutex;
use e32::sys::{TPtr8, TPtrC8};
use e32::{dprintln, time};

/// What a test does: 0 nothing yet, 1 playing, 2 recording, 3 playing back, 4 done, negative = failed with that error.
type State = &'static Mutex<i32>;

/// Bumped to stop what plays now (a new preview, leaving the page, the app hidden): a playback ends once the value it
/// started with is gone.
static PLAYING: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn current() -> u32 {
    PLAYING.load(core::sync::atomic::Ordering::SeqCst)
}

pub struct Sound {
    state: Option<State>,
    shown: core::cell::Cell<i32>,
}

impl Sound {
    pub fn new() -> Self {
        Sound {
            state: Mutex::new(0).ok().map(|m| &*Box::leak(Box::new(m))),
            shown: core::cell::Cell::new(0),
        }
    }

    /// "test" or "record": one test at a time.
    pub fn start(&self, action: &str) {
        let Some(state) = self.state else {
            return;
        };
        if matches!(*state.lock(), 1..=3) {
            return;
        }
        let record = action == "record";
        *state.lock() = if record { 2 } else { 1 };
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

    /// (SE2b) Play one period of a ringtone or message tone at `volume`, the PCM AUDIOD rings with
    /// (`neoaudio::tones`). An incoming call preempts it (AUDIOD's media yields to the ring); [`Self::stop`] ends it.
    pub fn preview(&self, tone: u8, volume: u8) {
        // a preview replaces the one that plays (not a test: those finish)
        let Some(state) = self.state else {
            return;
        };
        if matches!(*state.lock(), 2 | 3) {
            return;
        }
        let mine = PLAYING.fetch_add(1, core::sync::atomic::Ordering::SeqCst) + 1;
        *state.lock() = 1;
        if let Err(e) = e32::thread::spawn(move || {
            let pcm = neoaudio::tones::pcm(tone, volume);
            let r = open(neoaudio::tones::RATE, 1, false).and_then(|s| play(&s, &pcm, mine));
            dprintln!(
                "settings: preview tone {} volume {} -> {:?}",
                tone,
                volume,
                r
            );
            *state.lock() = r.map_or_else(|e| e, |()| 4);
            0
        }) {
            *state.lock() = e;
        }
    }

    /// Stop a preview (or test) that plays now.
    pub fn stop(&self) {
        PLAYING.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    }

    /// The test's progress, when it changed since the last call.
    pub fn changed(&self) -> Option<String> {
        let s = *self.state?.lock();
        (s != self.shown.replace(s)).then(|| feedback(s))
    }
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

/// Write all of `pcm`, then wait for the queue to play out; stop early once [`PLAYING`] moves on from `mine`.
fn play(s: &Session, pcm: &[u8], mine: u32) -> Result<(), i32> {
    let stop = || current() != mine;
    for chunk in pcm.chunks(8_192) {
        if stop() {
            break;
        }
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
        if stop() {
            let _ = s.send_receive(neoaudio::op::STOP, &TIpcArgs::new());
            break;
        }
        if info.queued == 0 {
            dprintln!(
                "settings: sound test: {} frames, {} underruns, sink {}",
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
        play(&s, &pcm, current())
    });
    dprintln!("settings: sound test sound -> {:?}", r);
    r.map_or_else(|e| e, |_| 0)
}

/// Record 5 s (48 kHz mono, the mic as it is), then play it back.
fn record_test(state: State) -> i32 {
    let r = (|| {
        let rec = open(48_000, 1, true)?;
        let mut pcm = alloc::vec![0u8; 48_000 * 2 * 5];
        let mut got = 0;
        let mut since = time::millis();
        while got < pcm.len() {
            // read only what is there: a recording that never gets data fails after 8 s instead of hanging
            let mut b = [0u8; neoaudio::Info::SIZE];
            let mut des = TPtr8::new(&mut b);
            let mut a = TIpcArgs::new();
            a.des8(0, &mut des);
            rec.send_receive(neoaudio::op::INFO, &a)?;
            let queued = neoaudio::Info::from_bytes(&b).unwrap_or_default().queued as usize;
            if queued == 0 {
                if time::millis() - since > 8_000 {
                    dprintln!("settings: sound test: no recorded data after {} bytes", got);
                    return Err(-33); // KErrTimedOut
                }
                time::sleep(50_000);
                continue;
            }
            since = time::millis();
            let n = (pcm.len() - got).min(queued).min(16_384);
            let mut des = TPtr8::new(&mut pcm[got..got + n]);
            let mut a = TIpcArgs::new();
            a.des8(0, &mut des);
            let r = rec.send_receive(neoaudio::op::READ, &a)?;
            got += r as usize;
        }
        drop(rec);
        // the level after the first 0.5 s (the mic's start can pop): RMS, peak, clipped samples, the first peak's place
        let samples: Vec<i32> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i32::from(i16::from_le_bytes(*b)))
            .collect();
        let settled = &samples[samples.len().min(24_000)..];
        let n = settled.len().max(1) as i64;
        let mean = settled.iter().map(|&v| i64::from(v)).sum::<i64>() / n;
        let var = settled
            .iter()
            .map(|&v| (i64::from(v) - mean).pow(2))
            .sum::<i64>()
            / n;
        let rms = isqrt(var as u64);
        let peak = settled.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        let clipped = settled
            .iter()
            .filter(|v| v.unsigned_abs() >= 32_767)
            .count();
        let first_peak = samples
            .iter()
            .position(|v| v.unsigned_abs() >= 32_767)
            .map_or(-1, |i| i as i64);
        dprintln!(
            "settings: sound test: recorded {} bytes; after 0.5 s: dc {} rms {} peak {} clipped {}; first full-scale sample at {}",
            got,
            mean,
            rms,
            peak,
            clipped,
            first_peak
        );
        *state.lock() = 3;
        let s = open(48_000, 1, false)?;
        play(&s, &pcm, current())
    })();
    dprintln!("settings: sound test record -> {:?}", r);
    r.map_or_else(|e| e, |_| 0)
}

/// Integer square root (the level statistics; no float formatting needed).
fn isqrt(v: u64) -> u64 {
    let mut x = v;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

// SPDX-License-Identifier: GPL-3.0-only
//! The hardware side: what the app reads itself (HAL values, the NeoModem server's status, the drives), the checks
//! "Run checks" runs, each device's facts and tests, and the tools that touch the system (save the logs, the audio
//! loopback, the storage speed test). Every read returns its error, so a board without a part says so.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use e32::display::Display;
use e32::fs::{File, Fs, Volume};
use e32::ipc::{Session, TIpcArgs, TVersion};
use e32::sys::TPtr8;
use e32::{dprintln, time};
use modem_ipc::{Status, op};
use neoapp::{hal, sys};
use neoui_proto::devtools::device::{Device, KernelDevice, Probe, State, parse_kernel_list};
use neoui_proto::devtools::stats::format_bytes;

/// KErrNotSupported: the kernel or board has no such function.
pub const NOT_SUPPORTED: i32 = -5;
/// The drives the Storage device shows.
const DRIVES: [char; 3] = ['C', 'E', 'Z'];
const SPEED_FILE: &str = r"C:\devtools-speed.tmp";
const SPEED_BYTES: usize = 1 << 20;

/// One refresh of everything the probes and the device views read.
#[derive(Clone, Debug, Default)]
pub struct Readings {
    pub memory: Option<Result<sys::Memory, i32>>,
    pub display: Option<Result<(usize, usize), i32>>,
    pub volumes: Vec<(char, Result<Volume, i32>)>,
    /// The board RTC's seconds, and the kernel's UTC (Unix seconds) read with it.
    pub rtc: Option<(u32, i64)>,
    pub modem_hal: Option<Result<[u32; 4], i32>>,
    pub modem: Option<Status>,
    pub adsp: Option<Result<[u32; 4], i32>>,
    pub slot: Option<Result<String, i32>>,
    pub console: Option<Result<bool, i32>>,
}

/// Reads everything (a few HAL calls, one NeoModem request, the drives' volume info).
pub fn read() -> Readings {
    let fs = Fs::connect();
    let volumes = DRIVES
        .iter()
        .map(|&d| {
            let v = match &fs {
                Ok(fs) => fs.volume(i32::from(d as u8 - b'A')),
                Err(e) => Err(*e),
            };
            (d, v)
        })
        .collect();
    Readings {
        memory: Some(sys::memory()),
        display: Some(Display::current().map(|d| (d.width, d.height))),
        volumes,
        rtc: time::rtc_seconds().map(|s| (s, time::utc_now().0)),
        modem_hal: Some(hal::modem_state()),
        modem: modem_status(),
        adsp: Some(hal::adsp_state()),
        slot: Some(hal::slot_info()),
        console: Some(hal::console_shown()),
    }
}

fn modem_session() -> Result<Session, i32> {
    let (major, minor, build) = modem_ipc::VERSION;
    let v = TVersion {
        major: major as u8,
        minor: minor as u8,
        build: build as u16,
    };
    Session::connect(modem_ipc::NAME, v)
}

/// C7 recovery action, kept in the existing Tools page alongside the other device actions.
pub fn modem_retry() -> String {
    match modem_session().and_then(|s| s.send_receive(op::RETRY, &TIpcArgs::new())) {
        Ok(_) => String::from("Modem/audio recovery requested"),
        Err(e) => format!("Modem/audio recovery unavailable ({e})"),
    }
}

/// NeoModem's status; None without MODEMD (Syborg, or not up yet).
fn modem_status() -> Option<Status> {
    let session = modem_session().ok()?;
    let mut b = [0u8; Status::SIZE];
    let mut des = TPtr8::new(&mut b);
    let mut args = TIpcArgs::new();
    args.des8(0, &mut des);
    session.send_receive(op::STATUS, &args).ok()?;
    Status::from_bytes(&b)
}

/// The kernel's device registry (HAL 0x4e19).
pub fn kernel_list() -> Result<(Vec<KernelDevice>, usize), i32> {
    hal::device_list().map(|text| parse_kernel_list(&text))
}

fn modem_words(w: [u32; 4]) -> (State, String) {
    let state = match (w[0], w[1]) {
        (_, e) if e != 0 => State::Error,
        (8, _) => State::Ok,
        (0, _) => State::Off,
        _ => State::Starting,
    };
    (
        state,
        format!(
            "kernel boot step {} of 8, error {}, IPCRTR {}",
            w[0], w[1] as i32, w[3]
        ),
    )
}

fn modem_text(s: &Status) -> String {
    let sim = match s.sim {
        modem_ipc::sim::ABSENT => "no SIM",
        modem_ipc::sim::PRESENT => "SIM",
        modem_ipc::sim::ERROR => "SIM error",
        _ => "SIM unknown",
    };
    let rat = match s.rat {
        modem_ipc::rat::GSM => "2G",
        modem_ipc::rat::UMTS => "3G",
        modem_ipc::rat::LTE => "LTE",
        _ => "no network",
    };
    let op = s.operator.as_str();
    format!(
        "{sim}, {rat}, {} bars{}{op}",
        s.bars,
        if op.is_empty() { "" } else { ", " }
    )
}

/// What the app can tell about each catalogue component by itself.
pub fn probes(r: &Readings, cpus: u32, cpu_khz: Option<u32>) -> Vec<Probe> {
    let mut out = Vec::new();
    let mut put = |key, state, detail: String| out.push(Probe { key, state, detail });
    if let Some(Ok(m)) = r.memory {
        put(
            "memory",
            State::Ok,
            format!(
                "{} free of {}",
                format_bytes(m.free_ram),
                format_bytes(m.total_ram)
            ),
        );
    }
    if let Some(Ok((w, h))) = r.display {
        put("display", State::Ok, format!("{w}x{h}"));
    }
    if let Some(khz) = cpu_khz {
        put(
            "cpu",
            State::Ok,
            format!("{}, top {} MHz", crate::pages::home::cpus(cpus), khz / 1000),
        );
    }
    let drives: Vec<String> = r
        .volumes
        .iter()
        .filter_map(|(d, v)| {
            v.as_ref()
                .ok()
                .map(|v| format!("{d}: {} free", format_bytes(v.free as u64)))
        })
        .collect();
    if !drives.is_empty() {
        put("storage", State::Ok, drives.join(", "));
    }
    if let Some((rtc, utc)) = r.rtc {
        put(
            "rtc",
            State::Ok,
            format!(
                "{rtc} s, {:+} s from the kernel clock",
                i64::from(rtc) - utc
            ),
        );
    }
    match (&r.modem, r.modem_hal) {
        (Some(s), _) => {
            let state = match s.modem {
                modem_ipc::modem::RUNNING | modem_ipc::modem::ONLINE => State::Ok,
                modem_ipc::modem::FAILED => State::Error,
                _ => State::Starting,
            };
            put("modem", state, modem_text(s));
        }
        (None, Some(Ok(w))) => {
            let (state, detail) = modem_words(w);
            put("modem", state, detail);
        }
        _ => {}
    }
    if let Some(Ok(w)) = r.adsp {
        let state = match (w[0], w[1]) {
            (_, e) if e != 0 => State::Error,
            (4, _) => State::Ok,
            (0, _) => State::Off,
            _ => State::Starting,
        };
        put(
            "adsp",
            state,
            format!("step {} of 4, channels {:#x}", w[0], w[3]),
        );
    }
    if let Some(Ok(s)) = &r.slot {
        put("slot", State::Ok, s.clone());
    }
    out
}

/// A check's outcome, with what it saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    Pass(String),
    Fail(String),
    Skip(String),
}

impl Check {
    pub fn word(&self) -> &'static str {
        match self {
            Check::Pass(_) => "pass",
            Check::Fail(_) => "fail",
            Check::Skip(_) => "skip",
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Check::Pass(t) | Check::Fail(t) | Check::Skip(t) => t,
        }
    }
}

/// A 4 KB write, read back and delete on C:.
fn storage_roundtrip() -> Result<String, i32> {
    let fs = Fs::connect()?;
    let data: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
    let t0 = time::millis();
    {
        let f = File::replace(&fs, SPEED_FILE, e32::fs::mode::WRITE)?;
        f.write(&data)?;
        let mut back = alloc::vec![0u8; data.len()];
        let n = f.read_at(0, &mut back)?;
        if back[..n] != data[..] {
            let _ = fs.delete(SPEED_FILE);
            return Err(-20); // KErrCorrupt
        }
    }
    fs.delete(SPEED_FILE)?;
    Ok(format!(
        "4 KB written, read back and deleted on C: in {} ms",
        time::millis() - t0
    ))
}

/// One device's check: its own test where it has one, else what its state says. Never has side effects beyond a
/// temporary file on C:.
pub fn check(dev: &Device, r: &Readings) -> Check {
    let from_state = || match dev.state {
        State::Ok | State::Partial => Check::Pass(format!("{}: {}", dev.state.label(), dev.detail)),
        s if s.is_problem() => Check::Fail(format!("{}: {}", s.label(), dev.detail)),
        s => Check::Skip(format!("nothing to test ({})", s.label())),
    };
    match dev.key.as_str() {
        "memory" => match r.memory {
            Some(Ok(m)) if m.free_ram > 0 => {
                Check::Pass(format!("{} free", format_bytes(m.free_ram)))
            }
            Some(Ok(_)) => Check::Fail("no free RAM".into()),
            Some(Err(e)) => Check::Fail(format!("memory HAL: {e}")),
            None => Check::Skip("not read yet".into()),
        },
        "storage" => match storage_roundtrip() {
            Ok(t) => Check::Pass(t),
            Err(e) => Check::Fail(format!("C: write/read failed ({e})")),
        },
        "display" => match r.display {
            Some(Ok((w, h))) if w > 0 && h > 0 => Check::Pass(format!("{w}x{h}")),
            Some(Err(e)) => Check::Fail(format!("display HAL: {e}")),
            _ => from_state(),
        },
        "rtc" => match r.rtc {
            Some((rtc, utc)) => {
                let drift = i64::from(rtc) - utc;
                // the kernel clock is the RTC plus the saved offset (neoui's clock keeper), so only its change matters
                Check::Pass(format!("reads {rtc} s ({drift:+} s from the kernel clock)"))
            }
            None => match dev.state {
                State::NoDriver => Check::Skip("no RTC on this board".into()),
                _ => from_state(),
            },
        },
        "modem" => match (&r.modem, r.modem_hal) {
            (Some(s), _) if s.modem == modem_ipc::modem::FAILED => {
                Check::Fail("NeoModem: the modem failed".into())
            }
            (Some(s), _) => Check::Pass(modem_text(s)),
            (None, Some(Ok(w))) => match modem_words(w) {
                (State::Error, t) => Check::Fail(t),
                (_, t) => Check::Skip(format!("no NeoModem server; {t}")),
            },
            _ => Check::Skip("no modem on this board".into()),
        },
        "adsp" => match r.adsp {
            Some(Ok(w)) if w[1] != 0 => Check::Fail(format!("error {}", w[1] as i32)),
            Some(Ok(w)) if w[0] == 4 => Check::Pass("running".into()),
            Some(Ok(w)) => Check::Skip(format!("not running (step {})", w[0])),
            _ => Check::Skip("no audio DSP on this board".into()),
        },
        "slot" => match &r.slot {
            Some(Ok(s)) => Check::Pass(s.clone()),
            Some(Err(e)) if *e == NOT_SUPPORTED => Check::Skip("no A/B slots on this board".into()),
            Some(Err(e)) => Check::Fail(format!("slot info: {e}")),
            None => from_state(),
        },
        _ => from_state(),
    }
}

/// The tests a device's view offers, in order. Index 0 is always "Run this check".
pub fn actions(key: &str) -> Vec<&'static str> {
    let mut a = alloc::vec!["Run this check"];
    a.extend_from_slice(match key {
        "touch" => &["Touch test"][..],
        "display" => &["Colour test", "Kernel console on or off"][..],
        "storage" => &["Speed test: 1 MB on C:"][..],
        "audio" | "adsp" => &[
            "Loopback: mic to the loudspeaker",
            "Loopback: mic to the earpiece",
            "Stop the loopback",
        ][..],
        "modem" => &["Refresh the status"][..],
        _ => &[][..],
    });
    a
}

/// Live facts for a device's view, after the common ones (driver, source, detail).
pub fn facts(dev: &Device, r: &Readings, extra: &crate::state::Model) -> Vec<(String, String)> {
    let mut f: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: String| f.push((k.to_string(), v));
    match dev.key.as_str() {
        "memory" => {
            if let Some(Ok(m)) = r.memory {
                put("Total RAM", format_bytes(m.total_ram));
                put("Free RAM", format_bytes(m.free_ram));
                put("ROM", format_bytes(m.rom));
            }
        }
        "cpu" => {
            put("CPUs", format!("{}", extra.cpus));
            put(
                "Top clock",
                extra
                    .cpu_khz
                    .map_or("unknown".into(), |k| format!("{} MHz", k / 1000)),
            );
            put("Fast counter", format!("{} Hz", time::fast_counter_hz()));
            put(
                "Load",
                extra.cpu_load.map_or(
                    "not measured (kernel without MONITOR_THREAD_CPU_TIME)".into(),
                    neoui_proto::devtools::stats::format_percent,
                ),
            );
        }
        "display" => {
            if let Some(Ok((w, h))) = r.display {
                put("Screen", format!("{w}x{h}"));
            }
            put(
                "App surface",
                format!("{}x{}", extra.surface.0, extra.surface.1),
            );
            if let Some(c) = r.console {
                put(
                    "Kernel console",
                    c.map_or_else(
                        |e| format!("not on this board ({e})"),
                        |on| String::from(if on { "on screen" } else { "off" }),
                    ),
                );
            }
        }
        "storage" => {
            for (d, v) in &r.volumes {
                let text = match v {
                    Ok(v) => format!(
                        "{} free of {}{}",
                        format_bytes(v.free as u64),
                        format_bytes(v.size as u64),
                        if v.name.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", v.name)
                        }
                    ),
                    Err(e) => format!("not mounted ({e})"),
                };
                put(&format!("{d}:"), text);
            }
        }
        "rtc" => match r.rtc {
            Some((rtc, utc)) => {
                put("RTC", format!("{rtc} s"));
                put("Kernel UTC", format!("{utc} s"));
            }
            None => put("RTC", "not readable here (no NeoSymbian RTC call)".into()),
        },
        "modem" => {
            if let Some(s) = &r.modem {
                put("NeoModem", modem_text(s));
                put("Registration", format!("{}", s.reg));
                put(
                    "Call audio",
                    format!(
                        "on {}, speaker {}, volume {}",
                        s.audio.on, s.audio.speaker, s.audio.volume
                    ),
                );
            } else {
                put("NeoModem", "not running".into());
            }
            if let Some(m) = r.modem_hal {
                put(
                    "Kernel",
                    m.map_or_else(|e| format!("no modem HAL ({e})"), |w| modem_words(w).1),
                );
            }
        }
        "adsp" | "audio" => {
            if let Some(a) = r.adsp {
                put(
                    "Audio DSP",
                    a.map_or_else(
                        |e| format!("not on this board ({e})"),
                        |w| {
                            format!(
                                "step {} of 4, error {}, channels {:#x}",
                                w[0], w[1] as i32, w[3]
                            )
                        },
                    ),
                );
            }
        }
        "keys" => put(
            "Last key",
            extra.last_key.map_or("press one".into(), |(c, down)| {
                format!("code {c:#x} {}", if down { "down" } else { "up" })
            }),
        ),
        "touch" => put(
            "Last point",
            extra
                .touch_last
                .map_or("run the touch test".into(), |(x, y)| format!("{x}, {y}")),
        ),
        "slot" => {
            if let Some(Ok(s)) = &r.slot {
                put("Slots", s.clone());
            }
        }
        _ => {}
    }
    f
}

/// NeoModem's audio loopback: 1 loudspeaker, 2 earpiece, 0 stop. What happened, in words.
pub fn audio_test(what: i32) -> String {
    let r = match modem_session() {
        Ok(session) => {
            let mut args = TIpcArgs::new();
            args.int(0, what);
            session
                .send_receive(op::AUDIO_TEST, &args)
                .unwrap_or_else(|e| e)
        }
        Err(e) => e,
    };
    let text = match (what, r) {
        (0, 0) => "loopback stopped".to_string(),
        (_, 0) => "loopback on: talk into the mic".to_string(),
        (_, -14) => "busy: a call, or a test running".to_string(),
        (_, -18) => "the audio DSP is not up yet".to_string(),
        (_, e) => format!("no NeoModem, or it failed ({e})"),
    };
    dprintln!("devtools: audio test {} -> {} ({})", what, r, text);
    text
}

/// Writes 1 MB to C: in 64 KB blocks, reads it back, deletes it: the speeds.
pub fn speed_test() -> Result<String, i32> {
    let fs = Fs::connect()?;
    let block: Vec<u8> = (0..65_536u32).map(|i| (i ^ (i >> 8)) as u8).collect();
    let mut back = alloc::vec![0u8; block.len()];
    let us =
        |from: u32| time::counter_us(from, time::fast_counter(), time::fast_counter_hz()).max(1);
    let (write_us, read_us) = {
        let f = File::replace(&fs, SPEED_FILE, e32::fs::mode::WRITE)?;
        let t = time::fast_counter();
        for _ in 0..SPEED_BYTES / block.len() {
            f.write(&block)?;
        }
        let write_us = us(t);
        let t = time::fast_counter();
        let mut pos = 0;
        while pos < SPEED_BYTES {
            let n = f.read_at(pos as i32, &mut back)?;
            if n == 0 {
                break;
            }
            pos += n;
        }
        (write_us, us(t))
    };
    fs.delete(SPEED_FILE)?;
    let mbps = |us: u32| (SPEED_BYTES as u64 * 1_000_000 / u64::from(us)) as f32 / (1 << 20) as f32;
    let text = format!(
        "1 MB on C: written at {:.1} MB/s ({} ms), read at {:.1} MB/s ({} ms)",
        mbps(write_us),
        write_us / 1000,
        mbps(read_us),
        read_us / 1000
    );
    dprintln!("devtools: {}", text);
    Ok(text)
}

/// Both kernel logs, whole, to `<drive>:\logs\kernel-<ms>.txt` and `lastlog-<ms>.txt` (E:, else C:), each headed by
/// the build lines. What was written, in words.
pub fn save_logs(build: &[String]) -> String {
    let stamp = time::millis();
    let result = (|| -> Result<String, i32> {
        let fs = Fs::connect()?;
        let dir = if fs.volume(4).is_ok() {
            r"E:\logs\"
        } else {
            r"C:\logs\"
        };
        match fs.mkdir(dir) {
            Ok(()) | Err(-11) => {} // KErrAlreadyExists
            Err(e) => return Err(e),
        }
        let mut names = Vec::new();
        let mut this = Vec::new();
        hal::log_read(&mut 0, &mut this)?;
        let last = hal::last_log().unwrap_or_default();
        for (bytes, name) in [(this, "kernel"), (last, "lastlog")] {
            if bytes.is_empty() {
                continue;
            }
            let path = format!("{dir}{name}-{stamp}.txt");
            let file = File::replace(&fs, &path, e32::fs::mode::WRITE)?;
            let mut head = build.join("\n");
            head.push_str("\n\n");
            file.write(head.as_bytes())?;
            file.write(&bytes)?;
            names.push(path);
        }
        Ok(names.join(", "))
    })();
    let text = match result {
        Ok(names) if names.is_empty() => String::from("nothing to save"),
        Ok(names) => format!("saved {names}"),
        Err(NOT_SUPPORTED) => String::from("no kernel log on this board (-5)"),
        Err(e) => format!("save failed ({e})"),
    };
    dprintln!("devtools: {}", text);
    text
}

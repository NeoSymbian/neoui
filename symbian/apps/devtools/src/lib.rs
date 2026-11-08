// SPDX-License-Identifier: GPL-3.0-only
//! The Dev tools app (`NEODEV.EXE`): the NeoSymbian kernel's own log, followed live and from the previous boot,
//! saved to `E:\logs\`, the on-screen kernel console switched on and off, and the A/B slot state. Everything goes
//! through the kernel's `EHalGroupVariant` functions (docs/rust-integration.md "Dev tools"); boards without them
//! (Syborg) answer -5 and the app says so. It is a ROM EXE, so it holds ReadDeviceData / WriteDeviceData.
#![no_std]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::pin::pin;
use e32::fs::{File, Fs};
use e32::ipc::TIpcArgs;
use e32::rt::{self, Either};
use e32::sys::TPtr8;
use e32::{dprintln, time};
use neoapp::App;
use neoui_proto::InputEvent;
use neoui_proto::ipc::{self as proto, AppEvent};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

e32::main!(main);

const GROUP_VARIANT: i32 = 1;
const LOG_READ: i32 = 0x4e00;
const LAST_LOG_READ: i32 = 0x4e01;
const CONSOLE_SET: i32 = 0x4e02;
const CONSOLE_GET: i32 = 0x4e03;
const SLOT_INFO: i32 = 0x4e09;
/// How often the live log is read.
const POLL_US: i32 = 500_000;
/// Lines kept for the screen (the saved files have everything the kernel kept).
const KEEP_LINES: usize = 500;

/// The action buttons, in order.
const ACTIONS: [&str; 6] = ["Log", "Last boot", "Build", "Save", "Console", "Power"];
/// "Power" swaps the row for these (the UI server carries them out, as its Power menu does: proto::POWER).
const POWER_ACTIONS: [&str; 4] = ["Restart", "Power off", "Fastboot", "Back"];

fn main() -> i32 {
    match run() {
        Ok(()) => 0,
        Err(e) => {
            dprintln!("devtools: failed {}", e);
            e
        }
    }
}

/// One HAL call that fills a descriptor (`a2`, or `a1` when `cursor` is None); the bytes the kernel wrote.
fn hal_read(function: i32, cursor: Option<&mut u32>, buf: &mut [u8]) -> Result<usize, i32> {
    let mut des = TPtr8::new(buf);
    let des_ptr: *mut u8 = (&mut des as *mut TPtr8).cast();
    // SAFETY: the kernel functions take a TUint32* cursor and/or a TDes8*; both live across the call.
    let r = unsafe {
        match cursor {
            Some(c) => {
                e32::sys::ui::hal_function(GROUP_VARIANT, function, (c as *mut u32).cast(), des_ptr)
            }
            None => {
                e32::sys::ui::hal_function(GROUP_VARIANT, function, des_ptr, core::ptr::null_mut())
            }
        }
    };
    if r < 0 { Err(r) } else { Ok(des.len()) }
}

/// Everything a log function returns from `*cursor` on (1 KB per call), the cursor left after it.
fn read_log(function: i32, cursor: &mut u32, out: &mut Vec<u8>) -> Result<(), i32> {
    let mut buf = [0u8; 1024];
    loop {
        let n = hal_read(function, Some(cursor), &mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn console_shown() -> Result<bool, i32> {
    let mut on: i32 = 0;
    // SAFETY: the kernel writes one TInt through a1.
    let r = unsafe {
        e32::sys::ui::hal_function(
            GROUP_VARIANT,
            CONSOLE_GET,
            (&mut on as *mut i32).cast(),
            core::ptr::null_mut(),
        )
    };
    if r < 0 { Err(r) } else { Ok(on != 0) }
}

fn set_console(on: bool) -> i32 {
    // SAFETY: a1 is the value itself (1 on, 0 off).
    unsafe {
        e32::sys::ui::hal_function(
            GROUP_VARIANT,
            CONSOLE_SET,
            on as usize as *mut u8,
            core::ptr::null_mut(),
        )
    }
}

fn slot_info() -> String {
    let mut buf = [0u8; 400];
    match hal_read(SLOT_INFO, None, &mut buf) {
        Ok(n) => String::from_utf8_lossy(&buf[..n]).into_owned(),
        Err(e) => format!("slots: not on this board ({e})"),
    }
}

/// Bytes of log text as screen lines (CR dropped, a partial last line kept for the next chunk).
struct Lines {
    lines: VecDeque<String>,
    partial: String,
}

impl Lines {
    fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            partial: String::new(),
        }
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.partial.clear();
    }

    fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match b {
                b'\n' => {
                    let line = core::mem::take(&mut self.partial);
                    self.lines.push_back(line);
                    if self.lines.len() > KEEP_LINES {
                        self.lines.pop_front();
                    }
                }
                b'\r' => {}
                b if b.is_ascii() => self.partial.push(b as char),
                _ => self.partial.push('?'),
            }
        }
    }

    /// The last `rows` lines (the partial one included), top first.
    fn tail(&self, rows: usize) -> Vec<&str> {
        let mut all: Vec<&str> = self.lines.iter().map(String::as_str).collect();
        if !self.partial.is_empty() {
            all.push(&self.partial);
        }
        let skip = all.len().saturating_sub(rows);
        all.split_off(skip)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Live,
    LastBoot,
    /// which kernel, ROM and neoui run, and the A/B slots
    Build,
}

struct State {
    view: View,
    live: Lines,
    live_cursor: u32,
    last: Lines,
    status: String,
    slots: String,
    // neoapp::build::lines(): kernel, ROM, neoui
    build: Vec<String>,
}

impl State {
    /// New live log text since the last poll; true if any.
    fn poll(&mut self) -> bool {
        let mut bytes = Vec::new();
        match read_log(LOG_READ, &mut self.live_cursor, &mut bytes) {
            Ok(()) => {
                self.live.push(&bytes);
                !bytes.is_empty()
            }
            Err(e) => {
                self.status = format!("kernel log: not on this board ({e})");
                false
            }
        }
    }

    fn load_last(&mut self) {
        let mut bytes = Vec::new();
        let mut cursor = 0;
        self.last.clear();
        match read_log(LAST_LOG_READ, &mut cursor, &mut bytes) {
            Ok(()) if bytes.is_empty() => {
                self.status = String::from("no log kept from the previous boot")
            }
            Ok(()) => {
                self.last.push(&bytes);
                self.status = format!("previous boot: {} bytes", bytes.len());
            }
            Err(e) => self.status = format!("last boot log: not on this board ({e})"),
        }
    }

    /// Both logs, whole, to E:\logs\kernel-<ms>.txt and lastlog-<ms>.txt.
    fn save(&mut self) {
        let stamp = time::millis();
        let result = (|| -> Result<String, i32> {
            let fs = Fs::connect()?;
            match fs.mkdir(r"E:\logs\") {
                Ok(()) | Err(-11) => {} // KErrAlreadyExists
                Err(e) => return Err(e),
            }
            let mut names = Vec::new();
            for (function, name) in [(LOG_READ, "kernel"), (LAST_LOG_READ, "lastlog")] {
                let mut bytes = Vec::new();
                let mut cursor = 0;
                read_log(function, &mut cursor, &mut bytes)?;
                if bytes.is_empty() {
                    continue;
                }
                let path = format!(r"E:\logs\{name}-{stamp}.txt");
                let file = File::replace(&fs, &path, 0)?;
                // which build wrote it, then the log
                let mut head = self.build.join("\n");
                head.push_str("\n\n");
                file.write(head.as_bytes())?;
                file.write(&bytes)?;
                names.push(path);
            }
            Ok(names.join(", "))
        })();
        self.status = match result {
            Ok(names) if names.is_empty() => String::from("nothing to save"),
            Ok(names) => format!("saved {names}"),
            Err(e) => format!("save failed ({e})"),
        };
        dprintln!("devtools: {}", self.status);
    }

    fn toggle_console(&mut self) {
        self.status = match console_shown() {
            Ok(on) => {
                let r = set_console(!on);
                if r < 0 {
                    format!("console: switch failed ({r})")
                } else {
                    format!("on-screen kernel console {}", if on { "off" } else { "on" })
                }
            }
            Err(e) => format!("console: not on this board ({e})"),
        };
    }
}

fn round(v: f32) -> f32 {
    (v + 0.5) as i32 as f32
}

fn run() -> Result<(), i32> {
    let app = App::connect()?;
    let ui = DevApp::new().map_err(|_| -2)?;
    let dp = app.dp;
    ui.global::<Theme>().set_dp(dp);
    let font_px = round(11.5 * dp);
    let cell_h = round(font_px * 1.25);
    let (pad, bar_h) = (round(8.0 * dp), round(48.0 * dp));
    let rows = ((app.height as f32 - 2.0 * pad - 1.6 * bar_h) / cell_h).max(1.0) as usize;
    ui.set_font_px(font_px);
    ui.set_cell_h(cell_h);
    ui.set_pad(pad);
    ui.set_bar_h(bar_h);
    let set_actions = |ui: &DevApp, names: &[&str]| {
        let actions: Vec<SharedString> = names.iter().map(|a| SharedString::from(*a)).collect();
        ui.set_actions(ModelRc::from(Rc::new(VecModel::from(actions))));
    };
    set_actions(&ui, &ACTIONS);
    let mut power_row = false;
    let model = Rc::new(VecModel::from(
        (0..rows).map(|_| SharedString::new()).collect::<Vec<_>>(),
    ));
    ui.set_rows(ModelRc::from(model.clone()));
    let pressed: Rc<RefCell<VecDeque<i32>>> = Rc::default();
    let queue = pressed.clone();
    ui.on_action(move |i| queue.borrow_mut().push_back(i));
    ui.show().map_err(|_| -2)?;

    let mut st = State {
        view: View::Live,
        live: Lines::new(),
        live_cursor: 0,
        last: Lines::new(),
        status: String::new(),
        slots: slot_info(),
        build: neoapp::build::lines(),
    };
    st.poll();
    if st.status.is_empty() {
        st.status = st.slots.clone();
    }
    show(&ui, &model, &st, rows);
    app.draw()?;
    dprintln!("devtools: {} rows; {}", rows, st.slots);

    rt::block_on(async {
        loop {
            let mut ev = [0u8; proto::EVENT_LEN];
            let got = {
                let mut des = TPtr8::new(&mut ev);
                let mut args = TIpcArgs::new();
                args.des8(0, &mut des);
                let mut next = pin!(app.server.send(proto::NEXT_EVENT, &args));
                loop {
                    match rt::select(next.as_mut(), time::delay(POLL_US)).await {
                        Either::Left(r) => break r,
                        Either::Right(()) => {
                            if st.poll() && st.view == View::Live {
                                show(&ui, &model, &st, rows);
                                app.draw()?;
                            }
                        }
                    }
                }
            };
            got?;
            match AppEvent::decode(&ev) {
                Some(AppEvent::Quit) => {
                    dprintln!("devtools: quit");
                    return Ok(());
                }
                Some(AppEvent::Input(touch @ InputEvent::Touch { .. })) => {
                    app.dispatch_touch(touch);
                    let picked: Vec<i32> = pressed.borrow_mut().drain(..).collect();
                    for i in picked {
                        if power_row {
                            let action = match i {
                                0 => Some(proto::POWER_RESTART),
                                1 => Some(proto::POWER_OFF),
                                2 => Some(proto::POWER_FASTBOOT),
                                _ => None,
                            };
                            if let Some(a) = action {
                                let mut args = TIpcArgs::new();
                                args.int(0, a);
                                let r = app.server.send_receive(proto::POWER, &args);
                                st.status = format!("power {}: {:?}", POWER_ACTIONS[i as usize], r);
                                dprintln!("devtools: {}", st.status);
                            }
                            power_row = false;
                            set_actions(&ui, &ACTIONS);
                            continue;
                        }
                        match i {
                            0 => {
                                st.view = View::Live;
                                st.slots = slot_info();
                                st.status = st.slots.clone();
                            }
                            1 => {
                                st.view = View::LastBoot;
                                st.load_last();
                            }
                            2 => {
                                st.view = View::Build;
                                st.slots = slot_info();
                                st.status = String::from("build");
                            }
                            3 => st.save(),
                            4 => st.toggle_console(),
                            5 => {
                                power_row = true;
                                set_actions(&ui, &POWER_ACTIONS);
                            }
                            _ => {}
                        }
                    }
                    ui.set_current(match st.view {
                        View::Live => 0,
                        View::LastBoot => 1,
                        View::Build => 2,
                    });
                    show(&ui, &model, &st, rows);
                    app.draw()?;
                }
                _ => {}
            }
        }
    })
}

/// The status line and the tail of the shown log into the UI (changed rows only).
fn show(ui: &DevApp, model: &VecModel<SharedString>, st: &State, rows: usize) {
    ui.set_status(SharedString::from(st.status.as_str()));
    let lines = match st.view {
        View::Live => st.live.tail(rows),
        View::LastBoot => st.last.tail(rows),
        View::Build => st
            .build
            .iter()
            .map(String::as_str)
            .chain([st.slots.as_str()])
            .collect(),
    };
    for y in 0..rows.min(model.row_count()) {
        let text = SharedString::from(lines.get(y).copied().unwrap_or(""));
        if model.row_data(y).as_ref() != Some(&text) {
            model.set_row_data(y, text);
        }
    }
}

// SPDX-License-Identifier: GPL-3.0-only
//! The Symbian text console server `!E32WindowServer`, as the stock `econs.dll` (`CConsoleTextWin`/`RConsole`,
//! kernelhwsrv `kernel/eka/ewsrv/co_*.cpp`) expects it — what the stock text window server `ewsrv` does
//! (`ws_main.cpp:160-513`), minus drawing: each client session is one console window (a [`Window`] grid), the newest
//! window is on top and gets the keys, and a `Read` is held until a key arrives.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::ops::ControlFlow;
use e32::server::{Message, Service};
use e32::sys::{TVersion, err};
use neoui_term::Window;

/// The server name `econs.dll` connects to (`e32twin.h:27`); a leading `!` needs ProtServ (ROM EXEs have it).
pub const NAME: &str = "!E32WindowServer";

// TConsoleWinOpcodes (`ws_std.h:286-309`)
const CREATE: i32 = 0;
const SET: i32 = 1;
const CLEAR_SCREEN: i32 = 2;
const CLEAR_TO_END_OF_LINE: i32 = 3;
const SET_WINDOW_POS_ABS: i32 = 4;
const SET_CURSOR_HEIGHT: i32 = 5;
const SET_CURSOR_POS_ABS: i32 = 6;
const SET_CURSOR_POS_REL: i32 = 7;
const CURSOR_POS: i32 = 8;
const CONTROL: i32 = 9;
const WRITE: i32 = 10;
const READ: i32 = 11;
const READ_CANCEL: i32 = 12;
const DESTROY: i32 = 13;
const SET_TITLE: i32 = 14;
const SET_SIZE: i32 = 15;
const SIZE: i32 = 16;
const SCREEN_SIZE: i32 = 17;
const SET_MODE: i32 = 18;
const SET_PALETTE_ENTRY: i32 = 19;
const GET_PALETTE_ENTRY: i32 = 20;
const SET_TEXT_COLORS: i32 = 21;
const SET_UI_COLORS: i32 = 22;
const SET_TEXT_ATTRIBUTE: i32 = 23;
/// What the stock server answers a second `Read` while one is pending (`ws_main.cpp`).
const READ_PENDING: i32 = 7;

pub struct ConsoleWindow {
    id: u32,
    pub grid: Window,
    pub cursor_on: bool,
    read: Option<Message>,
    keys: VecDeque<i32>,
}

pub struct Console {
    cols: usize,
    rows: usize,
    next_id: u32,
    /// Bottom to top; the last one is shown and gets the keys.
    pub windows: Vec<ConsoleWindow>,
    /// Something changed on screen since the last draw.
    pub dirty: bool,
}

impl Console {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            next_id: 1,
            windows: Vec::new(),
            dirty: true,
        }
    }

    pub fn top(&self) -> Option<&ConsoleWindow> {
        self.windows.last()
    }

    /// Complete every held `Read` (KErrCancel) -- before the server closes, which invalidates their handles.
    pub fn close(&mut self) {
        for w in self.windows.iter_mut() {
            if let Some(r) = w.read.take() {
                r.complete(err::KERR_CANCEL);
            }
        }
    }

    /// A key for the top window: answers its held `Read`, else waits in its queue.
    pub fn key(&mut self, code: i32) {
        let Some(w) = self.windows.last_mut() else {
            return;
        };
        match w.read.take() {
            Some(m) => deliver(m, code),
            None => w.keys.push_back(code),
        }
    }

    fn window(&mut self, id: u32) -> Option<&mut ConsoleWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// `TSize` of a window including the stock 1-character frame on each side (ESHELL subtracts it).
    fn framed_size(&self) -> [u8; 8] {
        point(self.cols as i32 + 2, self.rows as i32 + 2)
    }
}

/// `TPckgBuf<SConsoleKey>` (`e32twin.h:53-60`, 24 bytes): code, modifiers, mouse position, type 0 = key press.
fn deliver(m: Message, code: i32) {
    let mut k = [0u8; 24];
    k[..4].copy_from_slice(&code.to_le_bytes());
    let r = m.write8(0, &k, 0);
    m.complete(r.map_or_else(|e| e, |()| 0));
}

fn point(x: i32, y: i32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&x.to_le_bytes());
    b[4..].copy_from_slice(&y.to_le_bytes());
    b
}

fn read_point(m: &Message) -> Option<(i32, i32)> {
    let mut b = [0u8; 8];
    let n = m.read8(0, &mut b, 0).ok()?;
    (n == 8).then(|| {
        (
            i32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            i32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        )
    })
}

impl Service for Console {
    type Session = u32;

    fn connect(&mut self, version: TVersion) -> Result<u32, i32> {
        // econs sends 1.0.4016 (ws_std.h:37-39); accept that major version
        if version.major != 1 {
            return Err(err::KERR_NOT_SUPPORTED);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.windows.push(ConsoleWindow {
            id,
            grid: Window::new(self.cols, self.rows),
            cursor_on: true,
            read: None,
            keys: VecDeque::new(),
        });
        self.dirty = true;
        Ok(id)
    }

    fn message(&mut self, id: &mut u32, m: Message) {
        let size = self.framed_size();
        let Some(w) = self.window(*id) else {
            return m.complete(err::KERR_GENERAL);
        };
        let mut changed = true;
        let r = match m.function() {
            // Set carries the title and the requested size; every window here is full size
            CREATE | SET | SET_WINDOW_POS_ABS | SET_TITLE | DESTROY | SET_MODE
            | SET_PALETTE_ENTRY | SET_TEXT_COLORS | SET_UI_COLORS | SET_TEXT_ATTRIBUTE => {
                changed = false;
                0
            }
            CLEAR_SCREEN => {
                w.grid.clear();
                0
            }
            CLEAR_TO_END_OF_LINE => {
                w.grid.clear_to_end_of_line();
                0
            }
            SET_CURSOR_HEIGHT => {
                w.cursor_on = m.int(0) != 0;
                0
            }
            SET_CURSOR_POS_ABS => read_point(&m).map_or(err::KERR_ARGUMENT, |(x, y)| {
                w.grid.set_cursor(x, y);
                0
            }),
            SET_CURSOR_POS_REL => read_point(&m).map_or(err::KERR_ARGUMENT, |(x, y)| {
                w.grid.move_cursor(x, y);
                0
            }),
            CURSOR_POS => {
                changed = false;
                let (x, y) = w.grid.cursor();
                m.write8(0, &point(x as i32, y as i32), 0)
                    .map_or_else(|e| e, |()| 0)
            }
            CONTROL => m.read16_vec(0).map_or_else(
                |e| e,
                |s| {
                    w.grid.control(&s);
                    0
                },
            ),
            WRITE => m.read16_vec(0).map_or_else(
                |e| e,
                |s| {
                    w.grid.write(&s);
                    0
                },
            ),
            READ => {
                if w.read.is_some() {
                    m.complete(READ_PENDING);
                } else if let Some(code) = w.keys.pop_front() {
                    deliver(m, code);
                } else {
                    w.read = Some(m);
                }
                return;
            }
            READ_CANCEL => {
                if let Some(r) = w.read.take() {
                    r.complete(err::KERR_CANCEL);
                }
                changed = false;
                0
            }
            SIZE | SCREEN_SIZE => {
                changed = false;
                m.write8(0, &size, 0).map_or_else(|e| e, |()| 0)
            }
            GET_PALETTE_ENTRY => {
                changed = false;
                for slot in 1..4 {
                    let _ = m.write8(slot, &[0], 0);
                }
                0
            }
            SET_SIZE => {
                changed = false;
                err::KERR_NOT_SUPPORTED
            }
            _ => {
                changed = false;
                err::KERR_NOT_SUPPORTED
            }
        };
        m.complete(r);
        self.dirty |= changed;
    }

    fn disconnect(&mut self, id: u32) -> ControlFlow<()> {
        if let Some(i) = self.windows.iter().position(|w| w.id == id) {
            let mut w = self.windows.remove(i);
            if let Some(r) = w.read.take() {
                r.complete(err::KERR_CANCEL);
            }
            self.dirty = true;
        }
        ControlFlow::Continue(())
    }
}

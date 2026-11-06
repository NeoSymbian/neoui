// SPDX-License-Identifier: Apache-2.0
//! A Symbian text console window as data: a grid of UTF-16 cells, a cursor, and how console output moves it — the
//! semantics of the stock text window server (kernelhwsrv `kernel/eka/ewsrv/ws_win.cpp`) that the 2010 `econs.dll`
//! and ESHELL expect. Rendering and IPC live elsewhere (neoui's Terminal app); this is pure logic, tested on the host.
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Tab stops every 4 columns (`KTabSize`, ws_win.cpp).
pub const TAB: usize = 4;

pub struct Window {
    cols: usize,
    rows: usize,
    cells: Vec<u16>,
    /// Column; may equal `cols` right after writing the last column (wraps on the next character).
    cx: usize,
    cy: usize,
    /// `\n` also returns to column 0 (`+NewLine`).
    pub newline_mode: bool,
    /// Writing past the right edge continues on the next line (`-Wrap`, i.e. wrap lock off).
    pub wrap: bool,
    /// Lines ended by a line feed (or a wrap) since the last [`Window::take_lines`], for logging.
    finished: Vec<String>,
}

impl Window {
    pub fn new(cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Self {
            cols,
            rows,
            cells: vec![b' ' as u16; cols * rows],
            cx: 0,
            cy: 0,
            newline_mode: false,
            wrap: true,
            finished: Vec::new(),
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn row(&self, y: usize) -> &[u16] {
        &self.cells[y * self.cols..(y + 1) * self.cols]
    }

    pub fn row_string(&self, y: usize) -> String {
        String::from_utf16_lossy(self.row(y))
    }

    /// Console output (`RConsole::Write`): printable characters and the stock control set.
    pub fn write(&mut self, text: &[u16]) {
        for &c in text {
            match c {
                0x00 | 0x07 | 0x0B => {}
                0x08 | 0x7F => self.backspace(),
                0x09 => self.cx = ((self.cx / TAB + 1) * TAB).min(self.cols - 1),
                0x0A => {
                    if self.newline_mode {
                        self.cx = 0;
                    }
                    self.line_feed();
                }
                0x0C => self.clear(),
                0x0D => self.cx = 0,
                _ => {
                    if self.cx >= self.cols {
                        if !self.wrap {
                            self.cx = self.cols - 1;
                        } else {
                            self.cx = 0;
                            self.line_feed();
                        }
                    }
                    let i = self.cy * self.cols + self.cx;
                    self.cells[i] = c;
                    self.cx += 1;
                }
            }
        }
    }

    /// Move left (to the end of the previous line at column 0) and blank that cell; nothing at the top left.
    fn backspace(&mut self) {
        if self.cx > 0 {
            self.cx = self.cx.min(self.cols) - 1;
        } else if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.cols - 1;
        } else {
            return;
        }
        let i = self.cy * self.cols + self.cx;
        self.cells[i] = b' ' as u16;
    }

    fn line_feed(&mut self) {
        let line = self.row_string(self.cy);
        self.finished.push(String::from(line.trim_end()));
        if self.cy + 1 < self.rows {
            self.cy += 1;
        } else {
            self.cells.copy_within(self.cols.., 0);
            let last = (self.rows - 1) * self.cols;
            self.cells[last..].fill(b' ' as u16);
        }
    }

    /// Blank everything and home the cursor (`ClearScreen`, form feed).
    pub fn clear(&mut self) {
        self.cells.fill(b' ' as u16);
        self.cx = 0;
        self.cy = 0;
    }

    /// Blank from the cursor to the end of its line; the cursor stays.
    pub fn clear_to_end_of_line(&mut self) {
        let start = self.cy * self.cols + self.cx.min(self.cols);
        let end = (self.cy + 1) * self.cols;
        self.cells[start..end].fill(b' ' as u16);
    }

    /// Where the cursor is shown (0-based; a pending wrap shows on the last column).
    pub fn cursor(&self) -> (usize, usize) {
        (self.cx.min(self.cols - 1), self.cy)
    }

    /// `SetCursorPosAbs`, clamped to the window.
    pub fn set_cursor(&mut self, x: i32, y: i32) {
        self.cx = x.clamp(0, self.cols as i32 - 1) as usize;
        self.cy = y.clamp(0, self.rows as i32 - 1) as usize;
    }

    /// `SetCursorPosRel`, clamped to the window.
    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        let (x, y) = self.cursor();
        self.set_cursor(x as i32 + dx, y as i32 + dy);
    }

    /// `RConsole::Control`: `+`/`-` then flag letters; only upper-case letters count (ws_main.cpp:335-381). Kept:
    /// `N` newline mode, `W` wrap lock. The rest (maximise, scroll lock, cursor, ...) do not apply here.
    pub fn control(&mut self, flags: &[u16]) {
        let mut on = true;
        for &c in flags {
            match c {
                0x2B => on = true,
                0x2D => on = false,
                0x4E => self.newline_mode = on,
                0x57 => self.wrap = !on,
                _ => {}
            }
        }
    }

    /// The lines finished since the last call (trailing blanks trimmed).
    pub fn take_lines(&mut self) -> Vec<String> {
        core::mem::take(&mut self.finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn text(win: &Window, y: usize) -> String {
        win.row_string(y).trim_end().to_string()
    }

    #[test]
    fn prints_and_wraps_at_the_right_edge() {
        let mut win = Window::new(4, 3);
        win.write(&w("abcdef"));
        assert_eq!((text(&win, 0), text(&win, 1)), ("abcd".into(), "ef".into()));
        assert_eq!(win.cursor(), (2, 1));
    }

    #[test]
    fn newline_mode_returns_to_column_zero() {
        let mut win = Window::new(8, 3);
        win.write(&w("ab\ncd"));
        assert_eq!(text(&win, 1), "  cd", "plain LF keeps the column");
        let mut win = Window::new(8, 3);
        win.control(&w("+Maximize +NewLine -Lock -Wrap"));
        assert!(win.newline_mode && win.wrap);
        win.write(&w("ab\ncd"));
        assert_eq!((text(&win, 0), text(&win, 1)), ("ab".into(), "cd".into()));
        assert_eq!(win.take_lines(), ["ab"]);
    }

    #[test]
    fn carriage_return_tab_and_form_feed() {
        let mut win = Window::new(10, 2);
        win.write(&w("abc\rX\tY"));
        assert_eq!(text(&win, 0), "Xbc Y");
        win.write(&w("\u{c}"));
        assert_eq!((text(&win, 0), win.cursor()), ("".into(), (0, 0)));
    }

    #[test]
    fn backspace_is_destructive_and_wraps_back() {
        let mut win = Window::new(3, 2);
        win.write(&w("abcd"));
        win.write(&w("\u{8}\u{8}"));
        assert_eq!(
            (text(&win, 0), text(&win, 1), win.cursor()),
            ("ab".into(), "".into(), (2, 0))
        );
        let mut home = Window::new(3, 2);
        home.write(&w("\u{8}"));
        assert_eq!(home.cursor(), (0, 0));
    }

    #[test]
    fn scrolls_at_the_bottom() {
        let mut win = Window::new(4, 2);
        win.newline_mode = true;
        win.write(&w("1\n2\n3"));
        assert_eq!((text(&win, 0), text(&win, 1)), ("2".into(), "3".into()));
        assert_eq!(win.take_lines(), ["1", "2"]);
    }

    #[test]
    fn cursor_positioning_is_clamped() {
        let mut win = Window::new(5, 4);
        win.set_cursor(9, -2);
        assert_eq!(win.cursor(), (4, 0));
        win.move_cursor(-10, 2);
        assert_eq!(win.cursor(), (0, 2));
        win.write(&w("hello"));
        win.set_cursor(2, 2);
        win.clear_to_end_of_line();
        assert_eq!(text(&win, 2), "he");
        win.clear();
        assert_eq!((text(&win, 2), win.cursor()), ("".into(), (0, 0)));
    }
}

// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton, so one renderer runs every check.
#[allow(dead_code)] // helpers shared with the snapshot tests
mod common;

use neoui_proto::settings::Value;
use neoui_proto::{InputEvent, TouchPhase};
use neoui_render::Renderer;

const W: u32 = 539;
const H: u32 = 1079;

/// Centre of the key `at`..`at + units` (in `cols` key widths) of row `k` counted up from the bottom row (0) of the
/// open keyboard; mirrors ui/components/keyboard/keyboard.slint (row k centred 27 + 48 k dp above the keyboard's
/// bottom, the body's). `symbian/taps.sh` uses the same numbers.
fn key(r: &Renderer, k: usize, at: f32, units: f32, cols: f32) -> (i32, i32) {
    let d = common::dp(W, H);
    // the keyboard sits on the body's bottom: above the key row or the gesture band (Navigation v2 rule 14)
    let bottom = (r.app_body().y + r.app_body().h as i32) as f32;
    (
        ((at + units / 2.0) * W as f32 / cols) as i32,
        (bottom - (27.0 + k as f32 * 48.0) * d) as i32,
    )
}

struct Kb {
    r: Renderer,
    buf: Vec<u32>,
}

impl Kb {
    fn tap(&mut self, k: usize, at: f32, units: f32, cols: f32) {
        let (x, y) = key(&self.r, k, at, units, cols);
        common::tap(&mut self.r, x, y);
        common::settle();
        self.r.render(&mut self.buf, W as usize);
    }
    /// A letter key one unit wide.
    fn letter(&mut self, k: usize, at: f32, cols: f32) {
        self.tap(k, at, 1.0, cols);
    }
    fn typed(&mut self) -> String {
        self.r
            .take_typed()
            .into_iter()
            .map(|c| char::from_u32(c).unwrap())
            .collect()
    }
    fn globe(&mut self) {
        self.tap(0, 1.5, 1.0, 10.0);
    }
    fn set(&mut self, key: &str, v: Value) {
        assert!(self.r.apply_setting(key, &v), "{key}");
        common::frame(&mut self.r, &mut self.buf, W, key);
        let _ = self.r.take_setting_edits(); // the applied value echoes back
    }
    /// How much of an app's body the keyboard covers: its rows and the 30 dp hide band (dp, to the nearest one:
    /// pixels are truncated).
    fn h_dp(&self) -> u32 {
        (self.r.keyboard_h() as f32 / common::dp(W, H)).round() as u32
    }
    fn touch(&mut self, phase: TouchPhase, x: i32, y: i32) {
        self.r.dispatch(InputEvent::Touch { id: 0, x, y, phase });
    }
    /// Press the key at `at` and hold it `ms` (rendering a frame every 17 ms, as the UI server does while the
    /// keyboard's timers run), still pressed; returns where.
    fn hold(&mut self, k: usize, at: f32, units: f32, cols: f32, ms: u64) -> (i32, i32) {
        let (x, y) = key(&self.r, k, at, units, cols);
        self.touch(TouchPhase::Down, x, y);
        self.wait(ms);
        (x, y)
    }
    fn wait(&mut self, ms: u64) {
        let mut t = 0;
        while t < ms {
            common::advance(17.min(ms - t));
            t += 17;
            self.r.render(&mut self.buf, W as usize);
        }
    }
    fn release(&mut self, (x, y): (i32, i32)) {
        self.touch(TouchPhase::Up, x, y);
        common::settle();
        self.r.render(&mut self.buf, W as usize);
    }
    fn log(&mut self) -> Vec<String> {
        self.r.take_keyboard_log()
    }
    fn snapshot(&mut self, name: &str) {
        common::frame(&mut self.r, &mut self.buf, W, name);
        common::write_png(&format!("{name}-{W}x{H}.png"), &self.buf, W, H);
    }
}

#[test]
fn keyboard() {
    let mut kb = Kb {
        r: common::renderer(W, H),
        buf: vec![0u32; (W * H) as usize],
    };
    kb.r.show("terminal", "");
    kb.r.set_keyboard(1);
    kb.snapshot("keyboard-en");
    // all languages on, all as tall as Armenian's four letter rows: English gets the digit row (5 rows)
    assert_eq!(kb.h_dp(), 276);

    // English: rows 3 (qwerty) .. 1 (zxcv), 10 wide; the digit row is 4
    kb.letter(2, 0.5 + 2.0, 10.0); // d
    kb.letter(3, 7.0, 10.0); // i
    kb.letter(3, 3.0, 10.0); // r
    kb.tap(0, 8.5, 1.5, 10.0); // enter
    kb.tap(1, 0.0, 1.5, 10.0); // shift
    kb.letter(2, 0.5, 10.0); // A
    kb.letter(2, 0.5, 10.0); // a (shift is one-shot)
    kb.tap(1, 8.5, 1.5, 10.0); // backspace
    kb.tap(0, 3.5, 4.0, 10.0); // space
    kb.tap(0, 0.0, 1.5, 10.0); // ?123
    kb.letter(2, 2.0, 10.0); // backslash
    kb.letter(3, 0.0, 10.0); // 1
    kb.tap(0, 0.0, 1.5, 10.0); // ABC
    kb.letter(3, 0.0, 10.0); // q
    kb.letter(4, 6.0, 10.0); // 7, the digit row
    assert_eq!(kb.typed(), "dir\rAa\u{8} \\1q7");
    assert!(kb.r.take_typed().is_empty());
    assert_eq!(kb.r.take_setting_edits(), []);

    // the globe key: Armenian, 12 wide, four letter rows; the choice is stored
    kb.globe();
    assert_eq!(
        kb.r.take_setting_edits(),
        [("keyboard.layout", Value::Int(1))]
    );
    kb.snapshot("keyboard-hy");
    assert_eq!(kb.h_dp(), 276);
    kb.letter(4, 1.0, 12.0); // է (ten keys, centred)
    kb.letter(3, 0.0, 12.0); // ք
    kb.tap(1, 0.0, 1.5, 12.0); // shift
    kb.letter(2, 0.5, 12.0); // Ա
    kb.tap(0, 7.5, 1.0, 10.0); // ։
    assert_eq!(kb.typed(), "էքԱ։");

    // Russian: 12 wide, three letter rows plus the digit row
    kb.globe();
    kb.snapshot("keyboard-ru");
    assert_eq!(kb.h_dp(), 276);
    kb.letter(3, 0.0, 12.0); // й
    kb.letter(1, 1.0, 12.0); // ё (after a one-key-wide shift)
    kb.letter(2, 0.5 + 3.0, 12.0); // а
    assert_eq!(kb.typed(), "йёа");

    // and round to English
    kb.globe();
    kb.letter(3, 1.0, 10.0);
    assert_eq!(kb.typed(), "w");
    let _ = kb.r.take_setting_edits();

    // Armenian off: English and Russian are three rows again, and the globe skips Armenian
    kb.set("keyboard.hy", Value::Bool(false));
    assert_eq!(kb.h_dp(), 228);
    kb.globe();
    kb.letter(3, 0.0, 12.0);
    kb.globe();
    kb.letter(3, 0.0, 10.0);
    assert_eq!(kb.typed(), "йq");
    let _ = kb.r.take_setting_edits();

    // each language its own height
    kb.set("keyboard.hy", Value::Bool(true));
    kb.set("keyboard.same_height", Value::Bool(false));
    assert_eq!(kb.h_dp(), 228);
    kb.globe();
    assert_eq!(kb.h_dp(), 276, "Armenian");
    kb.globe();
    assert_eq!(kb.h_dp(), 228, "Russian");
    kb.globe();
    let _ = kb.r.take_setting_edits();

    // terminal mode: Esc, Ctrl, Tab and the arrows in a row on top (English: row 4 here), seven across
    kb.r.set_keyboard(2);
    kb.snapshot("keyboard-term");
    assert_eq!(kb.h_dp(), 276);
    assert_eq!(
        kb.r.keyboard_heights()
            .map(|h| (h as f32 / common::dp(W, H)).round() as u32),
        [0, 228, 276, 228, 276]
    );
    for i in [0, 2, 3, 4, 5, 6] {
        kb.letter(4, i as f32, 7.0);
    }
    assert_eq!(kb.r.take_typed(), [0x1b, 9, 0xF807, 0xF809, 0xF80A, 0xF808]);
    held_keys(&mut kb);

    kb.r.set_keyboard(0);
    assert_eq!(kb.r.keyboard_h(), 0);
    kb.letter(1, 1.0, 10.0);
    assert!(
        kb.r.take_typed().is_empty(),
        "a closed keyboard types nothing"
    );
}

/// Every letter of each language has a key (the layouts are hand-written data).
#[test]
fn layouts_have_every_letter() {
    let hy = include_str!("../../../ui/components/keyboard/layouts/hy.slint");
    let ru = include_str!("../../../ui/components/keyboard/layouts/ru.slint");
    let en = include_str!("../../../ui/components/keyboard/layouts/en.slint");
    let has = |src: &str, c: char| src.contains(&format!("\"{c}\""));
    for c in ('\u{561}'..='\u{586}').chain(['\u{587}']) {
        assert!(has(hy, c), "Armenian {c}");
    }
    for c in ('а'..='я').chain(['ё']) {
        assert!(has(ru, c), "Russian {c}");
    }
    for c in 'a'..='z' {
        assert!(has(en, c), "English {c}");
    }
}

/// Key repeat, long-press alternates, the terminal's Ctrl, word suggestions and Western Armenian (W1), on the
/// keyboard in terminal mode (English, all four rows plus the terminal row) and then text mode.
fn held_keys(kb: &mut Kb) {
    let _ = kb.log();
    // Ctrl (the terminal row's second key) then c: Ctrl+C's code; one-shot, so the next c is a c
    kb.letter(4, 1.0, 7.0);
    kb.snapshot("keyboard-term-ctrl");
    kb.letter(1, 3.5, 10.0); // c (zxcv row: after the 1.5 wide shift)
    kb.letter(1, 3.5, 10.0);
    assert_eq!(kb.r.take_typed(), [3, u32::from('c')]);
    assert_eq!(kb.log(), ["ctrl"]);

    // an arrow held: one at once, the next after 400 ms, then about 15 a second
    let at = kb.hold(4, 3.0, 1.0, 7.0, 380);
    assert_eq!(
        kb.r.take_typed(),
        [0xF807],
        "one at the press, none before 400 ms"
    );
    kb.wait(630); // 1 s held
    kb.release(at);
    let n = kb.r.take_typed().len();
    assert!((7..=10).contains(&n), "{n} repeats from 400 ms to 1 s");
    assert_eq!(kb.log(), [format!("repeat {n}")]);

    // text mode: Backspace held deletes several, a letter held types one
    kb.r.set_keyboard(1);
    let at = kb.hold(1, 8.5, 1.5, 10.0, 1000);
    kb.release(at);
    let typed = kb.r.take_typed();
    assert!(
        typed.len() >= 8 && typed.iter().all(|&c| c == 8),
        "{typed:?}"
    );
    let _ = kb.log();
    let at = kb.hold(3, 3.0, 1.0, 10.0, 1000); // r: no alternates
    kb.release(at);
    assert_eq!(kb.typed(), "r", "letters don't repeat");

    // e held: its alternates over it, é picked; let go types it
    let at = kb.hold(3, 2.0, 1.0, 10.0, 450);
    kb.snapshot("keyboard-longpress");
    assert!(
        kb.r.take_typed().is_empty(),
        "nothing until the finger lets go"
    );
    kb.release(at);
    assert_eq!(kb.typed(), "é");
    assert_eq!(kb.log(), ["alternate 1 of 5"], "the log never has the text");
    // slide two alternates to the right: ê; with shift: upper case
    let (x, y) = kb.hold(3, 2.0, 1.0, 10.0, 450);
    let cell = W as i32 / 10;
    kb.touch(TouchPhase::Move, x + cell, y - 40);
    kb.touch(TouchPhase::Move, x + 2 * cell, y - 60);
    kb.release((x + 2 * cell, y - 60));
    kb.tap(1, 0.0, 1.5, 10.0); // shift
    let at = kb.hold(3, 2.0, 1.0, 10.0, 450);
    kb.release(at);
    assert_eq!(kb.typed(), "êÉ");
    // a quick tap on e is still e
    kb.letter(3, 2.0, 10.0);
    assert_eq!(kb.typed(), "e");
    let _ = kb.log();

    // word suggestions: off by default; on, "th" offers the most frequent word in the middle, a tap replaces it
    kb.letter(2, 5.5, 10.0); // h
    kb.tap(0, 3.5, 4.0, 10.0); // space ends the word
    let _ = kb.typed();
    kb.set("keyboard.suggest", Value::Bool(true));
    kb.letter(3, 4.0, 10.0); // t
    kb.letter(2, 5.5, 10.0); // h
    kb.snapshot("keyboard-suggest");
    let d = common::dp(W, H);
    let bottom = (kb.r.app_body().y + kb.r.app_body().h as i32) as f32;
    let band = (bottom - kb.h_dp() as f32 * d + 15.0 * d) as i32;
    let strip = W as f32 - 68.0 * d; // 4 dp in, the hide key's 64 dp off
    common::tap(&mut kb.r, (4.0 * d + strip / 2.0) as i32, band);
    common::settle();
    kb.r.render(&mut kb.buf, W as usize);
    assert_eq!(kb.typed(), "th\u{8}\u{8}the ");
    assert_eq!(kb.log(), ["suggestion"]);
    kb.set("keyboard.suggest", Value::Bool(false));

    // Russian: е held gives ё
    kb.globe(); // Armenian
    kb.globe(); // Russian
    let at = kb.hold(3, 4.0, 1.0, 12.0, 450);
    kb.release(at);
    assert_eq!(kb.typed(), "ё");
    // Western Armenian: on, after Russian; its b key (Eastern բ) types պ
    kb.set("keyboard.hyw", Value::Bool(true));
    kb.globe();
    kb.snapshot("keyboard-hyw");
    kb.letter(1, 5.5, 12.0); // after the 1.5 wide shift: զ ղ ց վ, then the b key
    assert_eq!(kb.typed(), "պ");
    kb.set("keyboard.hyw", Value::Bool(false));
    let _ = kb.r.take_setting_edits();
    let _ = kb.log();
}

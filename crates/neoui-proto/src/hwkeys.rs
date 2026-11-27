// SPDX-License-Identifier: Apache-2.0
//! Hardware-keyboard layouts as data: what a key with a Symbian standard scan code (`TStdScanCode`, e32keys.h) types
//! in each of the system keyboard's languages, with Shift, Caps Lock and Ctrl. The UI server feeds it every raw key
//! event (`EKeyDown`/`EKeyUp`) and sends the character on like a key of the on-screen keyboard; the language is the
//! on-screen keyboard's current one (`keyboard.layout`), so the globe key switches both.
//!
//! Each language is two strings of [`KEYS`] characters, unshifted and shifted, in the order of the US keys' scan
//! codes in [`POSITIONS`]. The Armenian layouts follow the on-screen phonetic ones (ui/components/keyboard/layouts/
//! hy.slint, hyw.slint): the letters the on-screen keyboard keeps in its top row are Shift + 1..7, ՛ ՞ Shift + 8, 9,
//! and ։ Shift + /.
// shortcut: no upper case for է ձ ջ ռ չ ժ on a hardware keyboard (Shift gives the letter); add an AltGr layer if
// someone types Armenian on one.

/// Keys a layout covers: the US keyboard's printable keys, from ` to /.
pub const KEYS: usize = 47;

/// The scan codes of those keys in layout order: `, 1-0, - =, Q-P, [ ], A-L, ; ', \, Z-M, , . /
pub const POSITIONS: [u8; KEYS] = [
    0x78, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', 0x82, 0x83, //
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I', b'O', b'P', 0x80, 0x81, //
    b'A', b'S', b'D', b'F', b'G', b'H', b'J', b'K', b'L', 0x7D, 0x7E, 0x7C, //
    b'Z', b'X', b'C', b'V', b'B', b'N', b'M', 0x79, 0x7A, 0x7B,
];

/// One language's keys: `lower[i]` and `upper[i]` are what the key `POSITIONS[i]` types without and with Shift.
pub struct Keymap {
    /// The on-screen layout's code (types.slint `Layout.code`).
    pub code: &'static str,
    pub lower: &'static str,
    pub upper: &'static str,
}

/// The keymaps, by the on-screen layout's code (layouts.slint's order: en, hy, ru, hyw).
pub const KEYMAPS: [Keymap; 4] = [
    Keymap {
        code: "en",
        lower: "`1234567890-=qwertyuiop[]asdfghjkl;'\\zxcvbnm,./",
        upper: "~!@#$%^&*()_+QWERTYUIOP{}ASDFGHJKL:\"|ZXCVBNM<>?",
    },
    Keymap {
        code: "hy",
        lower: "՝1234567890-=քոերտըւիօպխծասդֆգհյկլթփ\\զղցվբնմշճ/",
        upper: "՜էձջռչժև՛՞)_+ՔՈԵՐՏԸՒԻՕՊԽԾԱՍԴՖԳՀՅԿԼԹՓ|ԶՂՑՎԲՆՄՇՃ։",
    },
    Keymap {
        code: "ru",
        lower: "ё1234567890-=йцукенгшщзхъфывапролджэ\\ячсмитьбю.",
        upper: "Ё!\"№;%:?*()_+ЙЦУКЕНГШЩЗХЪФЫВАПРОЛДЖЭ/ЯЧСМИТЬБЮ,",
    },
    // Western Armenian: the Eastern phonetic keys with the pairs Western pronounces the other way round swapped
    // (բ/պ, գ/կ, դ/տ, ձ/ծ, ջ/ճ)
    Keymap {
        code: "hyw",
        lower: "՝1234567890-=քոերդըւիօբխձաստֆկհյգլթփ\\զղցվպնմշջ/",
        upper: "՜էծճռչժև՛՞)_+ՔՈԵՐԴԸՒԻՕԲԽՁԱՍՏՖԿՀՅԳԼԹՓ|ԶՂՑՎՊՆՄՇՋ։",
    },
];

/// The keymap of a layout code; English for an unknown one.
#[must_use]
pub fn keymap(code: &str) -> &'static Keymap {
    KEYMAPS
        .iter()
        .find(|k| k.code == code)
        .unwrap_or(&KEYMAPS[0])
}

const SHIFT_L: i32 = 0x12;
const SHIFT_R: i32 = 0x13;
const CTRL_L: i32 = 0x16;
const CTRL_R: i32 = 0x17;
const CAPS: i32 = 0x1A;

/// The modifiers held on a hardware keyboard (fed every key event, down and up).
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    /// one bit per modifier key held (bit = scan code - SHIFT_L): a held key's repeated downs set it once
    held: u16,
    caps: bool,
}

impl State {
    /// A key event: the code it types (Unicode; Backspace 8, Tab 9, Enter 13, Escape 27, Delete 127, Symbian's arrows
    /// 0xF807-0xF80A; Ctrl + a letter its control code 1-26, but not Ctrl+Q), or `None` for a modifier or a key that
    /// types nothing.
    /// `code` picks the language ([`keymap`]). A key's release maps the same way, with the modifiers as they are then.
    pub fn key(&mut self, scan: i32, down: bool, code: &str) -> Option<u32> {
        if matches!(scan, SHIFT_L | SHIFT_R | CTRL_L | CTRL_R | CAPS) {
            let bit = 1 << (scan - SHIFT_L);
            // Caps Lock toggles on its first down only
            self.caps ^= scan == CAPS && down && self.held & bit == 0;
            if down {
                self.held |= bit;
            } else {
                self.held &= !bit;
            }
            return None;
        }
        let is_held = |a: i32, b: i32| self.held & (1 << (a - SHIFT_L) | 1 << (b - SHIFT_L)) != 0;
        let (shift, ctrl) = (is_held(SHIFT_L, SHIFT_R), is_held(CTRL_L, CTRL_R));
        let fixed = match scan {
            1 => Some(8),       // EStdKeyBackspace
            2 => Some(9),       // EStdKeyTab
            3 => Some(13),      // EStdKeyEnter: EKeyEnter is 13
            4 => Some(0x1B),    // EStdKeyEscape
            5 => Some(0x20),    // EStdKeySpace
            0x0D => Some(0x7F), // EStdKeyDelete
            // EStdKeyLeft/Right/Up/DownArrow -> EKeyLeftArrow.. (0xF807..0xF80A, Unicode private use)
            0x0E..=0x11 => Some(0xF807 + (scan - 0x0E) as u32),
            _ => None,
        };
        if fixed.is_some() {
            return fixed;
        }
        let i = POSITIONS.iter().position(|&p| i32::from(p) == scan)?;
        let letter = (0x41..=0x5A).contains(&scan);
        if ctrl {
            // Ctrl + a letter is its control code whatever the language (Ctrl+C is 3 in Armenian too); not Ctrl+Q:
            // 0x11 is the edit bar's Control, which neoapp holds for the next key
            return (letter && scan != i32::from(b'Q')).then_some((scan as u32) & 0x1F);
        }
        let map = keymap(code);
        // Caps Lock shifts the letters only; Shift with it gives lower case
        let upper = shift ^ (self.caps && map.lower.chars().nth(i)?.is_alphabetic());
        let s = if upper { map.upper } else { map.lower };
        s.chars().nth(i).map(u32::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(s: &mut State, scan: i32, code: &str) -> Option<char> {
        let c = s.key(scan, true, code);
        s.key(scan, false, code);
        c.and_then(char::from_u32)
    }

    #[test]
    fn every_keymap_has_every_key() {
        for k in &KEYMAPS {
            assert_eq!(k.lower.chars().count(), KEYS, "{} lower", k.code);
            assert_eq!(k.upper.chars().count(), KEYS, "{} upper", k.code);
        }
    }

    #[test]
    fn keymaps_cover_every_letter() {
        let has = |k: &Keymap, c: char| k.lower.contains(c) || k.upper.contains(c);
        for c in ('\u{561}'..='\u{586}').chain(['\u{587}']) {
            assert!(has(keymap("hy"), c), "Armenian {c}");
            assert!(has(keymap("hyw"), c), "Western Armenian {c}");
        }
        for c in ('а'..='я').chain(['ё']) {
            assert!(has(keymap("ru"), c), "Russian {c}");
        }
        for c in 'a'..='z' {
            assert!(has(keymap("en"), c), "English {c}");
        }
    }

    #[test]
    fn letters_shift_and_caps() {
        let mut s = State::default();
        assert_eq!(tap(&mut s, i32::from(b'Q'), "en"), Some('q'));
        assert_eq!(tap(&mut s, i32::from(b'Q'), "ru"), Some('й'));
        assert_eq!(tap(&mut s, i32::from(b'Q'), "hy"), Some('ք'));
        assert_eq!(tap(&mut s, i32::from(b'B'), "hy"), Some('բ'));
        assert_eq!(tap(&mut s, i32::from(b'B'), "hyw"), Some('պ'));
        assert_eq!(tap(&mut s, 0x7A, "ru"), Some('ю'));
        assert_eq!(s.key(SHIFT_L, true, "en"), None);
        assert_eq!(tap(&mut s, i32::from(b'A'), "en"), Some('A'));
        assert_eq!(tap(&mut s, i32::from(b'1'), "en"), Some('!'));
        assert_eq!(tap(&mut s, i32::from(b'1'), "hy"), Some('է'));
        assert_eq!(tap(&mut s, i32::from(b'A'), "hy"), Some('Ա'));
        s.key(SHIFT_L, false, "en");
        assert_eq!(tap(&mut s, i32::from(b'A'), "en"), Some('a'));
        // Caps Lock: letters upper case, digits not; Shift undoes it for letters
        tap(&mut s, CAPS, "en");
        assert_eq!(tap(&mut s, i32::from(b'A'), "ru"), Some('Ф'));
        assert_eq!(tap(&mut s, i32::from(b'2'), "en"), Some('2'));
        s.key(SHIFT_R, true, "en");
        assert_eq!(tap(&mut s, i32::from(b'A'), "en"), Some('a'));
        s.key(SHIFT_R, false, "en");
        tap(&mut s, CAPS, "en");
        assert_eq!(tap(&mut s, i32::from(b'A'), "en"), Some('a'));
    }

    #[test]
    fn controls_and_ctrl() {
        let mut s = State::default();
        assert_eq!(s.key(1, true, "hy"), Some(8));
        assert_eq!(s.key(3, true, "en"), Some(13));
        assert_eq!(s.key(0x0E, true, "en"), Some(0xF807));
        assert_eq!(s.key(0x11, true, "en"), Some(0xF80A));
        s.key(CTRL_L, true, "ru");
        assert_eq!(s.key(i32::from(b'C'), true, "ru"), Some(3));
        assert_eq!(s.key(i32::from(b'1'), true, "ru"), None);
        s.key(CTRL_L, false, "ru");
        assert_eq!(
            tap(&mut s, i32::from(b'C'), "xx"),
            Some('c'),
            "unknown: English"
        );
        assert_eq!(s.key(0x60, true, "en"), None, "F1 types nothing");
        // Ctrl+Q would be 0x11, the edit bar's Control (neoapp holds it for the next key): it types nothing
        s.key(CTRL_R, true, "en");
        assert_eq!(s.key(i32::from(b'Q'), true, "en"), None);
        assert_eq!(s.key(i32::from(b'S'), true, "en"), Some(0x13));
    }

    #[test]
    fn repeated_downs_hold_once() {
        // a held key comes again as more downs (VNC, QEMU's virtio keyboard): one up still lets go of it
        let mut s = State::default();
        for _ in 0..3 {
            s.key(SHIFT_L, true, "en");
            s.key(CTRL_L, true, "en");
            s.key(CAPS, true, "en");
        }
        s.key(SHIFT_L, false, "en");
        s.key(CTRL_L, false, "en");
        s.key(CAPS, false, "en");
        assert_eq!(
            tap(&mut s, i32::from(b'A'), "en"),
            Some('A'),
            "Caps Lock on once"
        );
        tap(&mut s, CAPS, "en");
        assert_eq!(tap(&mut s, i32::from(b'A'), "en"), Some('a'));
    }
}

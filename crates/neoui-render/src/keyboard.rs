// SPDX-License-Identifier: Apache-2.0
//! The system keyboard: its language choice, which Slint can't loop over itself (answers for the `Layouts` global's
//! pure callbacks, `ui/components/keyboard/layouts.slint`), and its key codes as Slint key events.

use slint::SharedString;
use slint::platform::Key;

/// A key code from the system keyboard or a hardware key (Unicode; Enter 13, Backspace 8, Escape 27, Tab 9, Symbian's
/// arrows 0xF807-0xF80A) as the text of a Slint key event. The same mapping as `neoapp::App::dispatch_key`.
pub fn key_text(code: u32) -> Option<SharedString> {
    Some(match code {
        13 | 10 => Key::Return.into(),
        8 => Key::Backspace.into(),
        27 => Key::Escape.into(),
        9 => Key::Tab.into(),
        0xF807 => Key::LeftArrow.into(),
        0xF808 => Key::RightArrow.into(),
        0xF809 => Key::UpArrow.into(),
        0xF80A => Key::DownArrow.into(),
        c => char::from_u32(c)?.into(),
    })
}

/// The first layout turned on at or after `from`, wrapping round; 0 if none is (or there are none).
pub fn pick(on: &[bool], from: i32) -> i32 {
    let n = on.len() as i32;
    if n == 0 {
        return 0;
    }
    (0..n)
        .map(|d| (from + d).rem_euclid(n))
        .find(|&i| on[i as usize])
        .unwrap_or(0)
}

/// The most letter rows among the layouts turned on (`rows[i]` is layout i's).
pub fn tallest(on: &[bool], rows: &[usize]) -> i32 {
    on.iter()
        .zip(rows)
        .filter(|(on, _)| **on)
        .map(|(_, r)| *r as i32)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_text_maps_control_keys() {
        assert_eq!(key_text(13), Some(Key::Return.into()));
        assert_eq!(key_text(8), Some(Key::Backspace.into()));
        assert_eq!(key_text(0xF807), Some(Key::LeftArrow.into()));
        assert_eq!(key_text('ա' as u32), Some("ա".into()));
        assert_eq!(key_text(0xD800), None);
    }

    #[test]
    fn pick_wraps_and_skips_off() {
        let on = [true, false, true];
        assert_eq!(pick(&on, 0), 0);
        assert_eq!(pick(&on, 1), 2);
        assert_eq!(pick(&on, 3), 0);
        assert_eq!(pick(&on, -1), 2);
        assert_eq!(pick(&[false, true], 0), 1);
        assert_eq!(pick(&[false, false], 1), 0);
        assert_eq!(pick(&[], 5), 0);
    }

    #[test]
    fn tallest_counts_only_layouts_on() {
        assert_eq!(tallest(&[true, false, true], &[3, 4, 3]), 3);
        assert_eq!(tallest(&[true, true, true], &[3, 4, 3]), 4);
        assert_eq!(tallest(&[false, false], &[3, 4]), 0);
    }
}

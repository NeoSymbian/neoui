// SPDX-License-Identifier: Apache-2.0
//! The system keyboard's language choice, which Slint can't loop over itself: answers for the `Layouts` global's
//! pure callbacks (`ui/components/keyboard/layouts.slint`).

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

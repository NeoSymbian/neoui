// SPDX-License-Identifier: Apache-2.0
//! (G3) Pinch on Open apps (docs `neoui-shell/navigation.md`): two fingers spread apart switch to 2 columns (bigger
//! cards), pinched together to 3. Finger 0 and finger 1 only; one switch per pair of presses. Phone only: QEMU's
//! virtio tablet has one pointer.

use crate::TouchPhase;

/// Spread or pinch by this ratio of the starting distance (squared: 1.3² ≈ 169 / 100).
const RATIO_SQ: (i64, i64) = (169, 100);

#[derive(Clone, Copy, Debug, Default)]
pub struct Pinch {
    at: [Option<(i32, i32)>; 2],
    /// the squared distance when the second finger went down; 0 once this pair switched
    d0: i64,
}

impl Pinch {
    /// Both fingers are down: the shell's widgets must not see finger 0's press as a tap.
    pub fn active(&self) -> bool {
        self.at.iter().all(Option::is_some)
    }

    fn dist(&self) -> i64 {
        match self.at {
            [Some(a), Some(b)] => {
                let (dx, dy) = (i64::from(a.0 - b.0), i64::from(a.1 - b.1));
                dx * dx + dy * dy
            }
            _ => 0,
        }
    }

    /// Feed one touch; `Some(cols)` when the pair just spread (2) or pinched (3) far enough.
    pub fn feed(&mut self, id: u8, phase: TouchPhase, x: i32, y: i32) -> Option<i32> {
        let slot = self.at.get_mut(usize::from(id))?;
        if phase == TouchPhase::Up {
            *slot = None;
            self.d0 = 0;
            return None;
        }
        if phase == TouchPhase::Move && slot.is_none() {
            return None; // a pointer moving unpressed
        }
        *slot = Some((x, y));
        if phase == TouchPhase::Down {
            self.d0 = self.dist();
            return None;
        }
        let (d0, d) = (self.d0, self.dist());
        if d0 == 0 || !self.active() {
            return None;
        }
        let cols = if d * RATIO_SQ.1 >= d0 * RATIO_SQ.0 {
            2
        } else if d * RATIO_SQ.0 <= d0 * RATIO_SQ.1 {
            3
        } else {
            return None;
        };
        self.d0 = 0;
        Some(cols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TouchPhase::{Down, Move, Up};

    #[test]
    fn spread_is_two_columns_pinch_is_three_once_per_pair() {
        let mut p = Pinch::default();
        assert_eq!(p.feed(0, Down, 100, 500), None);
        assert!(!p.active());
        assert_eq!(p.feed(0, Move, 110, 500), None, "one finger is no pinch");
        assert_eq!(p.feed(1, Down, 310, 500), None);
        assert!(p.active());
        assert_eq!(p.feed(1, Move, 340, 500), None, "a little: nothing yet");
        assert_eq!(p.feed(1, Move, 400, 500), Some(2), "spread by 1.45×");
        assert_eq!(p.feed(1, Move, 600, 500), None, "once per pair");
        assert_eq!(p.feed(1, Up, 600, 500), None);
        assert!(!p.active());
        assert_eq!(p.feed(1, Down, 410, 500), None);
        assert_eq!(p.feed(0, Move, 200, 500), Some(3), "pinched to 0.7×");
        p.feed(0, Up, 0, 0);
        p.feed(1, Up, 0, 0);
        assert_eq!(p.feed(1, Move, 1, 1), None, "unpressed");
        assert_eq!(p.feed(2, Down, 1, 1), None, "a third finger is ignored");
    }
}

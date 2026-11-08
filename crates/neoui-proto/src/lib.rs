// SPDX-License-Identifier: Apache-2.0
//! Types shared by the neoui server, its renderer and its clients.
#![no_std]

extern crate alloc;

pub mod ipc;
pub mod recents;

/// An axis-aligned rectangle in physical pixels. Empty when `w == 0 || h == 0`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    // i64 edges so x + w never overflows.
    fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }
    fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }

    /// Smallest rect containing both. Empty rects are ignored.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(
            x,
            y,
            (self.right().max(other.right()) - x as i64) as u32,
            (self.bottom().max(other.bottom()) - y as i64) as u32,
        )
    }

    /// Overlap of both, or `Rect::default()` (empty) when they do not overlap.
    pub fn intersection(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        if r <= x as i64 || b <= y as i64 {
            return Rect::default();
        }
        Rect::new(x, y, (r - x as i64) as u32, (b - y as i64) as u32)
    }
}

/// Dirty region of a frame: up to `Damage::CAP` rects. Overlapping rects are
/// merged; once full, new rects are merged into the last slot (over-approximation).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    rects: [Rect; Damage::CAP],
    len: usize,
}

impl Damage {
    pub const CAP: usize = 8;

    pub const fn new() -> Self {
        Self {
            rects: [Rect::new(0, 0, 0, 0); Self::CAP],
            len: 0,
        }
    }

    pub fn add(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        if let Some(slot) = self.rects[..self.len]
            .iter_mut()
            .find(|s| !s.intersection(&r).is_empty())
        {
            *slot = slot.union(&r);
        } else if self.len < Self::CAP {
            self.rects[self.len] = r;
            self.len += 1;
        } else {
            self.rects[Self::CAP - 1] = self.rects[Self::CAP - 1].union(&r);
        }
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects[..self.len]
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Union of all rects (empty if there is no damage).
    pub fn bounds(&self) -> Rect {
        self.rects().iter().fold(Rect::default(), |a, r| a.union(r))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// Position in physical pixels of the target surface.
    Touch {
        id: u8,
        x: i32,
        y: i32,
        phase: TouchPhase,
    },
    /// `code` is a Unicode scalar value for now.
    Key { code: u32, down: bool },
}

/// Fold each run of touch moves of one finger into its last move; presses, releases and keys are kept, in order.
/// (One render per batch of input, not per event: smooth-ui S1.)
pub fn coalesce_moves(evs: &mut alloc::vec::Vec<InputEvent>) {
    use InputEvent::Touch;
    evs.dedup_by(|later, earlier| {
        let run = matches!(
            (&*earlier, &*later),
            (Touch { id: a, phase: TouchPhase::Move, .. }, Touch { id: b, phase: TouchPhase::Move, .. }) if a == b
        );
        if run {
            *earlier = *later;
        }
        run
    });
}

/// The next frame deadline after `now`: the first `last + k * period` (k ≥ 1) past `now`, so an overrun frame skips
/// the deadlines it missed instead of queueing frames. `last` itself while `now` is still before it.
pub fn next_deadline(last: u64, now: u64, period: u64) -> u64 {
    if now < last {
        return last;
    }
    last + ((now - last) / period + 1) * period
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceId(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        assert!(Rect::new(5, 5, 0, 3).is_empty());
        assert!(Rect::new(5, 5, 3, 0).is_empty());
        assert!(!Rect::new(5, 5, 1, 1).is_empty());
    }

    #[test]
    fn union() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(20, -5, 5, 5);
        assert_eq!(a.union(&b), Rect::new(0, -5, 25, 15));
        assert_eq!(a.union(&Rect::default()), a);
        assert_eq!(Rect::new(100, 100, 0, 0).union(&b), b);
    }

    #[test]
    fn intersection() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(
            a.intersection(&Rect::new(5, 5, 10, 10)),
            Rect::new(5, 5, 5, 5)
        );
        assert!(a.intersection(&Rect::new(10, 0, 5, 5)).is_empty()); // touching edge
        assert!(a.intersection(&Rect::new(-20, -20, 5, 5)).is_empty());
    }

    #[test]
    fn no_overflow_at_edges() {
        let a = Rect::new(i32::MAX - 1, 0, u32::MAX, 1);
        assert_eq!(a.intersection(&a), a);
    }

    #[test]
    fn damage_merges_overlaps() {
        let mut d = Damage::new();
        assert!(d.is_empty());
        d.add(Rect::new(0, 0, 10, 10));
        d.add(Rect::new(5, 5, 10, 10));
        d.add(Rect::new(100, 100, 1, 1));
        d.add(Rect::default());
        assert_eq!(
            d.rects(),
            &[Rect::new(0, 0, 15, 15), Rect::new(100, 100, 1, 1)]
        );
        assert_eq!(d.bounds(), Rect::new(0, 0, 101, 101));
    }

    fn touch(id: u8, x: i32, phase: TouchPhase) -> InputEvent {
        InputEvent::Touch { id, x, y: 0, phase }
    }

    #[test]
    fn coalesce_keeps_the_last_move_of_a_run() {
        use TouchPhase::*;
        let key = InputEvent::Key {
            code: 'a' as u32,
            down: true,
        };
        let mut evs = alloc::vec![
            touch(0, 1, Down),
            touch(0, 2, Move),
            touch(0, 3, Move),
            touch(0, 4, Move),
            key,
            touch(0, 5, Move),
            touch(1, 6, Move),
            touch(0, 7, Move),
            touch(0, 8, Up),
            touch(0, 9, Down),
            touch(0, 10, Up),
        ];
        coalesce_moves(&mut evs);
        assert_eq!(
            evs,
            [
                touch(0, 1, Down),
                touch(0, 4, Move),
                key,
                touch(0, 5, Move),
                touch(1, 6, Move),
                touch(0, 7, Move),
                touch(0, 8, Up),
                touch(0, 9, Down),
                touch(0, 10, Up),
            ]
        );
    }

    #[test]
    fn deadlines_skip_missed_frames() {
        assert_eq!(next_deadline(0, 0, 16), 16); // first frame
        assert_eq!(next_deadline(100, 105, 16), 116); // on time
        assert_eq!(next_deadline(100, 116, 16), 132); // exactly at the next one
        assert_eq!(next_deadline(100, 120, 16), 132); // overran by one period
        assert_eq!(next_deadline(100, 150, 16), 164); // overran by three
        assert_eq!(next_deadline(100, 40, 16), 100); // still before `last` (timer early): keep it
    }

    #[test]
    fn damage_overflow_keeps_coverage() {
        let mut d = Damage::new();
        for i in 0..20 {
            d.add(Rect::new(i * 10, 0, 1, 1));
        }
        assert_eq!(d.rects().len(), Damage::CAP);
        assert_eq!(d.bounds(), Rect::new(0, 0, 191, 1));
    }
}

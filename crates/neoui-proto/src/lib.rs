// SPDX-License-Identifier: Apache-2.0
//! Types shared by the neoui server, its renderer and its clients.
#![no_std]

extern crate alloc;

pub mod battery;
pub mod clock;
pub mod devtools;
pub mod ipc;
pub mod recents;
pub mod settings;

/// This neoui build: "neoui <commit>[+dirty] <UTC time>, e32-rs <commit>" (build.rs).
pub const BUILD: &str = env!("NEOUI_BUILD");

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

/// A list scrolled by `dy` px (smooth-ui S2d; > 0 = its content moved down): move the pixels already drawn inside
/// `list` with it (`stride` px per row), so only the rows [`exposed`] by the move need drawing. `list` must lie
/// inside `buf`.
pub fn shift_rows<T: Copy>(buf: &mut [T], stride: usize, list: Rect, dy: i32) {
    let (x, y0, w, h) = (
        list.x as usize,
        list.y as usize,
        list.w as usize,
        list.h as usize,
    );
    let n = dy.unsigned_abs() as usize;
    if n >= h {
        return;
    }
    let row = |y: usize| (y0 + y) * stride + x;
    if dy < 0 {
        for y in 0..h - n {
            buf.copy_within(row(y + n)..row(y + n) + w, row(y));
        }
    } else {
        for y in (n..h).rev() {
            buf.copy_within(row(y - n)..row(y - n) + w, row(y));
        }
    }
}

/// The rows of `list` (screen y) a scroll by `dy` uncovers, which [`shift_rows`] could not fill.
pub fn exposed(list: Rect, dy: i32) -> core::ops::Range<i32> {
    let n = (dy.unsigned_abs() as i32).min(list.h as i32);
    let bottom = list.y + list.h as i32;
    if dy < 0 {
        bottom - n..bottom
    } else {
        list.y..list.y + n
    }
}

/// How to draw a frame, given what the lists reported (smooth-ui S2d; `ui/components/scroll.slint`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScrollPlan {
    /// as usual
    Normal,
    /// the list's pixels were moved: Slint draws only the rows `draw` of `list`, and everything outside it
    Shift {
        list: Rect,
        draw: core::ops::Range<i32>,
    },
    /// the scrolling stopped: draw the whole window once, to put right what the skipped rows missed (a highlight,
    /// a sub-pixel position)
    Full,
}

/// Whether frames since the last full one were drawn by shifting.
#[derive(Default)]
pub struct Scrolling {
    shifted: bool,
}

impl Scrolling {
    /// `lists` lists scrolled by `dy` px in all since the last frame, the last of them at `list`. Exactly one list on
    /// `screen` that moved less than its height is shifted in `buf` here (`stride` px per row).
    pub fn plan<T: Copy>(
        &mut self,
        buf: &mut [T],
        stride: usize,
        screen: Rect,
        (dy, lists): (i32, i32),
        list: Rect,
    ) -> ScrollPlan {
        let list = list.intersection(&screen);
        if lists == 1 && dy != 0 && !list.is_empty() && dy.unsigned_abs() < list.h {
            shift_rows(buf, stride, list, dy);
            self.shifted = true;
            ScrollPlan::Shift {
                list,
                draw: exposed(list, dy),
            }
        } else if lists == 0 && core::mem::take(&mut self.shifted) {
            ScrollPlan::Full
        } else {
            ScrollPlan::Normal
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceId(pub u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_shift_moves_rows_and_exposes_the_rest() {
        // a 3-wide buffer of 5 rows, row r filled with r; the list covers columns 1..3 of rows 1..4
        let mut buf: [u32; 15] = core::array::from_fn(|i| (i / 3) as u32);
        let list = Rect::new(1, 1, 2, 3);
        // content up by 1: rows 2,3 move to 1,2; row 3 is exposed; column 0 and rows 0/4 stay
        shift_rows(&mut buf, 3, list, -1);
        assert_eq!(buf, [0, 0, 0, 1, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4]);
        assert_eq!(exposed(list, -1), 3..4);
        let mut buf: [u32; 15] = core::array::from_fn(|i| (i / 3) as u32);
        // content down by 2: row 1 moves to 3; rows 1..3 are exposed
        shift_rows(&mut buf, 3, list, 2);
        assert_eq!(&buf[9..12], &[3, 1, 1]);
        assert_eq!(exposed(list, 2), 1..3);
        // a shift as tall as the list exposes all of it and moves nothing
        let before = buf;
        shift_rows(&mut buf, 3, list, 3);
        assert_eq!(buf, before);
        assert_eq!(exposed(list, -3), 1..4);
        assert_eq!(exposed(list, 0), 1..1);
    }

    #[test]
    fn scroll_plan_shifts_one_list_then_redraws_once_when_it_stops() {
        let screen = Rect::new(0, 0, 3, 5);
        let list = Rect::new(0, 1, 3, 3);
        let mut buf = [0u32; 15];
        let mut s = Scrolling::default();
        assert_eq!(
            s.plan(&mut buf, 3, screen, (0, 0), list),
            ScrollPlan::Normal
        );
        assert_eq!(
            s.plan(&mut buf, 3, screen, (-1, 1), list),
            ScrollPlan::Shift { list, draw: 3..4 }
        );
        // two lists moved, or one by its whole height: no shift, but still owed a full redraw
        assert_eq!(
            s.plan(&mut buf, 3, screen, (-1, 2), list),
            ScrollPlan::Normal
        );
        assert_eq!(
            s.plan(&mut buf, 3, screen, (3, 1), list),
            ScrollPlan::Normal
        );
        assert_eq!(s.plan(&mut buf, 3, screen, (0, 0), list), ScrollPlan::Full);
        assert_eq!(
            s.plan(&mut buf, 3, screen, (0, 0), list),
            ScrollPlan::Normal
        );
        // a list partly off screen is shifted where it shows
        let low = Rect::new(0, 3, 3, 4);
        assert_eq!(
            s.plan(&mut buf, 3, screen, (1, 1), low),
            ScrollPlan::Shift {
                list: Rect::new(0, 3, 3, 2),
                draw: 3..4
            }
        );
    }

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

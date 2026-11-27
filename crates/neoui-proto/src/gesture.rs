// SPDX-License-Identifier: Apache-2.0
//! Edge gestures (docs `neoui-shell/navigation.md`, Navigation v2 rules 07 and 08), in two maps ([`Map`]):
//! - the Thumb map (Gestures, the default): in from either side for Back; up from the bottom for Home, up and hold
//!   still (about 300 ms, [`Recognizer::tick`]) for Open apps; along the bottom band for the previous / next app;
//! - the N9 map (N9 gestures): in from the left for Back; from the right a drag for Open apps, a quick flick for the
//!   last app; up from the bottom for Home and on past half the screen for the Apps menu;
//! - in either map with `nav.swipe_close` (D10, G4): down from the status bar over an app, released strictly beyond
//!   half the screen's height, closes it ([`Gesture::Close`]); a tap on the bar is replayed (the shade).
//!
//! The UI server feeds every touch of the screen to [`Recognizer::feed`] and acts on the [`Step`] it returns. A
//! press in an edge zone is held back until it either turns into an inward swipe (the gesture) or not (it is replayed
//! to the app or the shell, which never learn it was held).

use crate::TouchPhase;

/// Where a gesture starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Bottom,
    /// (G4) The status bar, live only over an app with `nav.swipe_close` on.
    Top,
}

/// Which gestures the edges make (the `nav.mode` setting: 1 Thumb, 2 N9; Buttons has none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Map {
    #[default]
    Thumb,
    N9,
}

/// What a finished gesture asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Back,
    OpenApps,
    Home,
    AppsMenu,
    /// N9: the app used before this one (a quick flick from the right).
    LastApp,
    /// Thumb: along the bottom band, rightwards (the older app) and leftwards (the newer one).
    PrevApp,
    NextApp,
    /// (G4) Down from the top past half the screen: close the app in front.
    Close,
}

impl Gesture {
    /// The name in the server's log (`neoui: gesture <name>`).
    pub const fn name(self) -> &'static str {
        match self {
            Gesture::Back => "back",
            Gesture::OpenApps => "apps",
            Gesture::Home => "home",
            Gesture::AppsMenu => "menu",
            Gesture::LastApp => "last",
            Gesture::PrevApp => "prev",
            Gesture::NextApp => "next",
            Gesture::Close => "close",
        }
    }
}

/// What the server does with the touch it just fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Not a gesture: route this touch as usual.
    Pass,
    /// Keep this touch back for now.
    Hold,
    /// Not a gesture after all: route the touches held so far, then this one, as usual.
    Replay,
    /// Drop this touch (the rest of a gesture the server cancelled).
    Drop,
    /// A gesture is under way: `progress` is 0..=256 up to the commit point; from the bottom edge (N9) it goes on to
    /// 512 at the Apps menu point, and (Thumb) it is 512 once the finger held still for Open apps.
    Track { edge: Edge, progress: u32 },
    /// The finger lifted past the commit point (or flung).
    Commit(Gesture),
    /// The finger lifted short of the commit point.
    Cancel,
}

/// Sizes in dp (docs `neoui-shell/navigation.md`, "The recognizer").
const ZONE_DP: i32 = 16;
const SLOP_DP: i32 = 8;
const SIDE_DP: i32 = 72;
const HOME_DP: i32 = 80;
const FLING_DP: i32 = 24;
/// (G4) The top edge's zone: the status bar.
const TOP_DP: i32 = 25;
/// Thumb: up from the bottom past Home's point and still this long = Open apps.
const PAUSE_MS: u64 = 300;
/// A fling: the last segment moved inward at 0.5 dp/ms or more, written as `d * 512 >= dt * dp256`.
const FLING_NUM: i64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    /// Routed as usual until the finger lifts.
    Passing,
    /// Dropped until the finger lifts.
    Dropping,
    /// Pressed in the zones of these edges (a corner is in two); not yet moved `slop`.
    Held {
        x0: i32,
        y0: i32,
        edges: u8,
    },
    Tracking {
        edge: Edge,
        x0: i32,
        y0: i32,
        /// the inward distance and time of the last sample with a later time, and whether it was a fling
        last: (i32, u64),
        fast: bool,
        /// Thumb, bottom: where and since when the finger is still, and whether it held long enough (Open apps)
        still: (i32, i32, u64),
        paused: bool,
    },
    /// Thumb: a move along the bottom band (the previous / next app); `dx` so far.
    Along {
        x0: i32,
        dx: i32,
    },
}

pub struct Recognizer {
    w: i32,
    h: i32,
    zone: i32,
    top: i32,
    slop: i32,
    side: i32,
    home: i32,
    menu: i32,
    fling: i32,
    dp256: i64,
    map: Map,
    state: State,
}

const fn bit(e: Edge) -> u8 {
    1 << e as u8
}

impl Recognizer {
    /// For a `w` × `h` px screen where 1 dp is `dp256 / 256` px (the server's `dp256`).
    pub fn new(w: u32, h: u32, dp256: u32) -> Self {
        let px = |dp: i32| (dp as i64 * dp256 as i64 / 256) as i32;
        Self {
            w: w as i32,
            h: h as i32,
            zone: px(ZONE_DP),
            top: px(TOP_DP),
            slop: px(SLOP_DP),
            side: px(SIDE_DP),
            home: px(HOME_DP),
            menu: (h / 2) as i32,
            fling: px(FLING_DP),
            dp256: dp256 as i64,
            map: Map::default(),
            state: State::Idle,
        }
    }

    /// The gestures the edges make from the next press on.
    pub fn set_map(&mut self, map: Map) {
        self.map = map;
    }

    /// Time passing with the finger still (no moves come then): in the Thumb map, up from the bottom past Home's
    /// point and held still for 300 ms becomes Open apps. Returns the step to act on, if any. Call each frame.
    pub fn tick(&mut self, ms: u64) -> Option<Step> {
        if let State::Tracking {
            edge: Edge::Bottom,
            last,
            still,
            paused: paused @ false,
            ..
        } = &mut self.state
            && self.map == Map::Thumb
            && last.0 >= self.home
            && ms.saturating_sub(still.2) >= PAUSE_MS
        {
            *paused = true;
            return Some(Step::Track {
                edge: Edge::Bottom,
                progress: 512,
            });
        }
        None
    }

    /// The edge zones `x, y` is in, among `live`.
    fn zones(&self, x: i32, y: i32, live: &[Edge]) -> u8 {
        live.iter().fold(0, |m, &e| {
            let hit = match e {
                Edge::Left => x < self.zone,
                Edge::Right => x >= self.w - self.zone,
                Edge::Bottom => y >= self.h - self.zone,
                Edge::Top => y < self.top,
            };
            if hit { m | bit(e) } else { m }
        })
    }

    /// The inward distance from the press, and the distance across it.
    fn along(edge: Edge, dx: i32, dy: i32) -> (i32, i32) {
        match edge {
            Edge::Left => (dx, dy),
            Edge::Right => (-dx, dy),
            Edge::Bottom => (-dy, dx),
            Edge::Top => (dy, dx),
        }
    }

    fn along_progress(&self, dx: i32) -> u32 {
        (dx.unsigned_abs() as i64 * 256 / self.side as i64).min(256) as u32
    }

    fn progress(&self, edge: Edge, d: i32) -> u32 {
        let d = d.max(0) as i64;
        let p = match edge {
            Edge::Left | Edge::Right => d * 256 / self.side as i64,
            // 256 at exactly half the height: closing takes more than that
            Edge::Top => d * 256 / (self.h / 2).max(1) as i64,
            Edge::Bottom if d <= self.home as i64 || self.map == Map::Thumb => {
                (d * 256 / self.home as i64).min(256)
            }
            Edge::Bottom => {
                256 + (d - self.home as i64) * 256 / (self.menu - self.home).max(1) as i64
            }
        };
        p.min(if edge == Edge::Bottom { 512 } else { 256 }) as u32
    }

    /// Whether a touch is being held or tracked: the server must not start other touch handling meanwhile.
    pub fn busy(&self) -> bool {
        matches!(
            self.state,
            State::Held { .. } | State::Tracking { .. } | State::Along { .. }
        )
    }

    /// (G2) The touch is a Thumb move along the bottom band (its `Track`s are the previous / next app's, not a swipe
    /// up from it): how far it went (px, rightwards positive).
    pub fn along_dx(&self) -> Option<i32> {
        match self.state {
            State::Along { dx, .. } => Some(dx),
            _ => None,
        }
    }

    /// Forget the touch in progress (the screen changed under it): the server drops what it held, and the rest of
    /// the touch is dropped too.
    pub fn reset(&mut self) {
        if self.busy() {
            self.state = State::Dropping;
        }
    }

    /// Feed one touch (screen pixels, `ms` the time it was read). On a press, only the `live` edges count.
    pub fn feed(&mut self, phase: TouchPhase, x: i32, y: i32, ms: u64, live: &[Edge]) -> Step {
        let up = phase == TouchPhase::Up;
        match (self.state, phase) {
            (_, TouchPhase::Down) => {
                let edges = self.zones(x, y, live);
                if edges == 0 {
                    self.state = State::Passing;
                    Step::Pass
                } else {
                    self.state = State::Held {
                        x0: x,
                        y0: y,
                        edges,
                    };
                    Step::Hold
                }
            }
            (State::Idle, _) => Step::Pass,
            (State::Passing, _) => {
                if up {
                    self.state = State::Idle;
                }
                Step::Pass
            }
            (State::Dropping, _) => {
                if up {
                    self.state = State::Idle;
                }
                Step::Drop
            }
            (State::Held { x0, y0, edges }, _) => {
                let (dx, dy) = (x - x0, y - y0);
                if !up && dx.abs().max(dy.abs()) < self.slop {
                    return Step::Hold;
                }
                // the edge whose inward direction this move mostly follows
                let locked = [Edge::Left, Edge::Right, Edge::Bottom, Edge::Top]
                    .into_iter()
                    .filter(|&e| edges & bit(e) != 0)
                    .find(|&e| {
                        let (a, c) = Self::along(e, dx, dy);
                        a >= self.slop && a > c.abs()
                    });
                // Thumb: a move along the bottom band
                let sideways = self.map == Map::Thumb
                    && edges & bit(Edge::Bottom) != 0
                    && dx.abs() >= self.slop
                    && dx.abs() > dy.abs();
                match locked {
                    Some(edge) if !up => {
                        let (d, _) = Self::along(edge, dx, dy);
                        self.state = State::Tracking {
                            edge,
                            x0,
                            y0,
                            last: (d, ms),
                            fast: false,
                            still: (x, y, ms),
                            paused: false,
                        };
                        Step::Track {
                            edge,
                            progress: self.progress(edge, d),
                        }
                    }
                    None if sideways && !up => {
                        self.state = State::Along { x0, dx };
                        Step::Track {
                            edge: Edge::Bottom,
                            progress: self.along_progress(dx),
                        }
                    }
                    // a tap in the zone, a move along the edge or outward, or a flick that lifted at once
                    _ => {
                        self.state = if up { State::Idle } else { State::Passing };
                        Step::Replay
                    }
                }
            }
            (State::Along { x0, .. }, _) => {
                let dx = x - x0;
                if !up {
                    self.state = State::Along { x0, dx };
                    return Step::Track {
                        edge: Edge::Bottom,
                        progress: self.along_progress(dx),
                    };
                }
                self.state = State::Idle;
                match dx {
                    _ if dx >= self.side => Step::Commit(Gesture::PrevApp),
                    _ if dx <= -self.side => Step::Commit(Gesture::NextApp),
                    _ => Step::Cancel,
                }
            }
            (
                State::Tracking {
                    edge,
                    x0,
                    y0,
                    last,
                    fast,
                    still,
                    paused,
                },
                _,
            ) => {
                let (d, _) = Self::along(edge, x - x0, y - y0);
                // coalesced moves and a release in the same batch share a time: keep the last speed then
                let fast = if ms > last.1 {
                    let dt = (ms - last.1) as i64;
                    (d - last.0) as i64 * FLING_NUM >= dt * self.dp256
                } else {
                    fast
                };
                let last = if ms > last.1 { (d, ms) } else { last };
                // the finger moved more than the slop since it was last still: it is not holding
                let moved = (x - still.0).abs().max((y - still.1).abs()) >= self.slop;
                let (still, paused) = if moved {
                    ((x, y, ms), false)
                } else {
                    (still, paused)
                };
                if !up {
                    self.state = State::Tracking {
                        edge,
                        x0,
                        y0,
                        last,
                        fast,
                        still,
                        paused,
                    };
                    return Step::Track {
                        edge,
                        progress: if paused { 512 } else { self.progress(edge, d) },
                    };
                }
                self.state = State::Idle;
                let flung = fast && d >= self.fling;
                match (self.map, edge) {
                    // strictly beyond half the height; no fling
                    (_, Edge::Top) if d > self.h / 2 => Step::Commit(Gesture::Close),
                    (_, Edge::Top) => Step::Cancel,
                    (Map::Thumb, Edge::Bottom) if paused => Step::Commit(Gesture::OpenApps),
                    (Map::N9, Edge::Bottom) if d >= self.menu => Step::Commit(Gesture::AppsMenu),
                    // no fling from the bottom: a press near the bottom that slips up fast must not go home
                    (_, Edge::Bottom) if d >= self.home => Step::Commit(Gesture::Home),
                    (_, Edge::Left) | (Map::Thumb, Edge::Right) if d >= self.side || flung => {
                        Step::Commit(Gesture::Back)
                    }
                    // N9: a quick flick from the right is the last app, a drag Open apps
                    (Map::N9, Edge::Right) if flung => Step::Commit(Gesture::LastApp),
                    (Map::N9, Edge::Right) if d >= self.side => Step::Commit(Gesture::OpenApps),
                    _ => Step::Cancel,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TouchPhase::*;

    const ALL: &[Edge] = &[Edge::Left, Edge::Right, Edge::Bottom];

    // The Pixel: 1080 × 2160, 3 px a dp: zone 48, slop 24, side 216, home 240, menu 1080, fling 72.
    fn pixel() -> Recognizer {
        Recognizer::new(1080, 2160, 768)
    }

    /// Feeds `(phase, x, y, ms)` and returns the last step.
    fn run(r: &mut Recognizer, touches: &[(TouchPhase, i32, i32, u64)]) -> Step {
        touches
            .iter()
            .map(|&(p, x, y, ms)| r.feed(p, x, y, ms, ALL))
            .last()
            .unwrap()
    }

    #[test]
    fn outside_the_zones_passes() {
        let mut r = pixel();
        assert_eq!(r.feed(Down, 540, 1000, 0, ALL), Step::Pass);
        assert_eq!(r.feed(Move, 900, 1000, 10, ALL), Step::Pass);
        assert_eq!(r.feed(Up, 900, 1000, 20, ALL), Step::Pass);
        assert!(!r.busy());
    }

    #[test]
    fn a_tap_in_a_zone_is_replayed() {
        let mut r = pixel();
        assert_eq!(r.feed(Down, 10, 1000, 0, ALL), Step::Hold);
        assert!(r.busy());
        assert_eq!(r.feed(Move, 15, 1003, 5, ALL), Step::Hold);
        assert_eq!(r.feed(Up, 15, 1003, 80, ALL), Step::Replay);
        assert!(!r.busy());
        // the toolbar's buttons sit in the bottom zone
        assert_eq!(
            run(&mut r, &[(Down, 540, 2150, 0), (Up, 540, 2150, 90)]),
            Step::Replay
        );
    }

    #[test]
    fn a_move_along_the_edge_is_replayed_then_passes() {
        let mut r = pixel();
        r.feed(Down, 10, 1000, 0, ALL);
        assert_eq!(r.feed(Move, 12, 1100, 10, ALL), Step::Replay);
        assert_eq!(r.feed(Move, 300, 1100, 20, ALL), Step::Pass);
        assert_eq!(r.feed(Up, 300, 1100, 30, ALL), Step::Pass);
        assert!(!r.busy());
    }

    #[test]
    fn dead_edges_pass() {
        let mut r = pixel();
        assert_eq!(r.feed(Down, 10, 1000, 0, &[Edge::Bottom]), Step::Pass);
        assert_eq!(r.feed(Up, 10, 1000, 0, &[Edge::Bottom]), Step::Pass);
    }

    #[test]
    fn side_swipes_commit_past_72_dp() {
        let mut r = pixel();
        r.set_map(Map::N9);
        let back = run(
            &mut r,
            &[
                (Down, 5, 900, 0),
                (Move, 100, 905, 100),
                (Move, 230, 910, 400),
                (Up, 230, 910, 500),
            ],
        );
        assert_eq!(back, Step::Commit(Gesture::Back));
        let apps = run(
            &mut r,
            &[
                (Down, 1075, 900, 0),
                (Move, 1000, 900, 100),
                (Move, 850, 900, 400),
                (Up, 850, 900, 500),
            ],
        );
        assert_eq!(apps, Step::Commit(Gesture::OpenApps));
    }

    #[test]
    fn short_and_slow_cancels_but_a_flick_commits() {
        let mut r = pixel();
        // 150 px (50 dp) over 600 ms: slow and short
        let slow = run(
            &mut r,
            &[
                (Down, 5, 900, 0),
                (Move, 60, 900, 300),
                (Move, 155, 900, 600),
                (Up, 155, 900, 600),
            ],
        );
        assert_eq!(slow, Step::Cancel);
        // the same distance, the last 95 px in 40 ms (0.8 dp/ms)
        let flick = run(
            &mut r,
            &[
                (Down, 5, 900, 0),
                (Move, 60, 900, 300),
                (Move, 155, 900, 340),
                (Up, 155, 900, 340),
            ],
        );
        assert_eq!(flick, Step::Commit(Gesture::Back));
        // fast but under 24 dp is not a flick
        let tiny = run(
            &mut r,
            &[
                (Down, 5, 900, 0),
                (Move, 35, 900, 10),
                (Move, 60, 900, 20),
                (Up, 60, 900, 20),
            ],
        );
        assert_eq!(tiny, Step::Cancel);
    }

    #[test]
    fn a_fast_short_slip_up_from_the_toolbar_stays() {
        let mut r = pixel();
        // 150 px (50 dp) up in 30 ms: fast, but the bottom edge takes no flings
        let s = run(
            &mut r,
            &[
                (Down, 540, 2150, 0),
                (Move, 540, 2100, 10),
                (Move, 540, 2000, 40),
                (Up, 540, 2000, 40),
            ],
        );
        assert_eq!(s, Step::Cancel);
    }

    #[test]
    fn going_back_toward_the_edge_cancels() {
        let mut r = pixel();
        let s = run(
            &mut r,
            &[
                (Down, 5, 900, 0),
                (Move, 300, 900, 100),
                (Move, 40, 900, 400),
                (Up, 40, 900, 500),
            ],
        );
        assert_eq!(s, Step::Cancel);
    }

    #[test]
    fn the_bottom_edge_goes_home_then_to_the_menu() {
        let mut r = pixel();
        r.set_map(Map::N9);
        let home = run(
            &mut r,
            &[
                (Down, 540, 2155, 0),
                (Move, 540, 2000, 200),
                (Move, 540, 1900, 500),
                (Up, 540, 1900, 600),
            ],
        );
        assert_eq!(home, Step::Commit(Gesture::Home));
        r.feed(Down, 540, 2155, 0, ALL);
        assert_eq!(
            r.feed(Move, 540, 1915, 100, ALL),
            Step::Track {
                edge: Edge::Bottom,
                progress: 256
            }
        );
        assert_eq!(
            r.feed(Move, 540, 1000, 300, ALL),
            Step::Track {
                edge: Edge::Bottom,
                progress: 512
            }
        );
        assert_eq!(
            r.feed(Up, 540, 1000, 400, ALL),
            Step::Commit(Gesture::AppsMenu)
        );
    }

    #[test]
    fn a_corner_follows_the_direction() {
        let mut r = pixel();
        let up = run(
            &mut r,
            &[
                (Down, 5, 2155, 0),
                (Move, 10, 1800, 200),
                (Up, 10, 1800, 500),
            ],
        );
        assert_eq!(up, Step::Commit(Gesture::Home));
        let right = run(
            &mut r,
            &[
                (Down, 5, 2155, 0),
                (Move, 300, 2150, 200),
                (Up, 300, 2150, 500),
            ],
        );
        assert_eq!(right, Step::Commit(Gesture::Back));
    }

    #[test]
    fn reset_drops_the_rest_of_the_touch() {
        let mut r = pixel();
        r.feed(Down, 5, 900, 0, ALL);
        r.feed(Move, 100, 900, 50, ALL);
        assert!(r.busy());
        r.reset();
        assert_eq!(r.feed(Move, 300, 900, 100, ALL), Step::Drop);
        assert_eq!(r.feed(Up, 300, 900, 150, ALL), Step::Drop);
        assert_eq!(r.feed(Down, 540, 900, 200, ALL), Step::Pass);
    }

    #[test]
    fn zones_scale_with_dp() {
        // Syborg: 768 × 640, 1.78 px a dp (min side 640 / 360): zone 28 px
        let mut r = Recognizer::new(768, 640, 640 * 256 / 360);
        assert_eq!(r.feed(Down, 27, 300, 0, ALL), Step::Hold);
        r.feed(Up, 27, 300, 0, ALL);
        assert_eq!(r.feed(Down, 29, 300, 0, ALL), Step::Pass);
    }

    /// Thumb (Gestures, the default): both sides go Back; the right edge's drag is Back too.
    #[test]
    fn thumb_both_sides_go_back() {
        let mut r = pixel();
        for x in [(5, 230), (1075, 850)] {
            let back = run(
                &mut r,
                &[
                    (Down, x.0, 900, 0),
                    (Move, (x.0 + x.1) / 2, 900, 100),
                    (Move, x.1, 900, 400),
                    (Up, x.1, 900, 500),
                ],
            );
            assert_eq!(back, Step::Commit(Gesture::Back));
        }
    }

    /// Thumb: up from the bottom is Home (never the Apps menu by distance); up past Home's point and held still for
    /// 300 ms is Open apps, until the finger moves on.
    #[test]
    fn thumb_up_is_home_and_up_and_hold_is_open_apps() {
        let mut r = pixel();
        let far = run(
            &mut r,
            &[
                (Down, 540, 2155, 0),
                (Move, 540, 1500, 100),
                (Move, 540, 1000, 200),
                (Up, 540, 1000, 300),
            ],
        );
        assert_eq!(far, Step::Commit(Gesture::Home));

        r.feed(Down, 540, 2155, 0, ALL);
        r.feed(Move, 540, 2000, 50, ALL);
        r.feed(Move, 540, 1880, 100, ALL);
        assert_eq!(r.tick(300), None, "not still for 300 ms yet");
        assert_eq!(
            r.tick(400),
            Some(Step::Track {
                edge: Edge::Bottom,
                progress: 512
            })
        );
        assert_eq!(r.tick(500), None, "once");
        assert_eq!(
            r.feed(Up, 540, 1880, 600, ALL),
            Step::Commit(Gesture::OpenApps)
        );

        // short of Home's point, holding does nothing
        r.feed(Down, 540, 2155, 0, ALL);
        r.feed(Move, 540, 2080, 50, ALL);
        assert_eq!(r.tick(1000), None);
        assert_eq!(r.feed(Up, 540, 2080, 1100, ALL), Step::Cancel);

        // held, then moved on: Home again
        r.feed(Down, 540, 2155, 0, ALL);
        r.feed(Move, 540, 1880, 100, ALL);
        r.tick(500);
        r.feed(Move, 540, 1700, 600, ALL);
        assert_eq!(r.feed(Up, 540, 1700, 700, ALL), Step::Commit(Gesture::Home));
    }

    /// Thumb: along the bottom band, past 72 dp, the previous (rightwards) or next (leftwards) app; less cancels.
    #[test]
    fn thumb_along_the_band_switches_apps() {
        let mut r = pixel();
        let prev = run(
            &mut r,
            &[
                (Down, 300, 2150, 0),
                (Move, 340, 2150, 50),
                (Move, 600, 2145, 150),
                (Up, 600, 2145, 200),
            ],
        );
        assert_eq!(prev, Step::Commit(Gesture::PrevApp));
        let next = run(
            &mut r,
            &[
                (Down, 800, 2150, 0),
                (Move, 760, 2150, 50),
                (Move, 500, 2150, 150),
                (Up, 500, 2150, 200),
            ],
        );
        assert_eq!(next, Step::Commit(Gesture::NextApp));
        // (G2) the server tells a move along the band from a swipe up by the state, not the Track
        r.feed(Down, 300, 2150, 0, ALL);
        r.feed(Move, 340, 2150, 50, ALL);
        assert_eq!(r.along_dx(), Some(40));
        r.feed(Up, 340, 2150, 60, ALL);
        assert_eq!(r.along_dx(), None);
        r.feed(Down, 300, 2150, 0, ALL);
        r.feed(Move, 300, 2100, 50, ALL);
        assert_eq!(r.along_dx(), None, "up from the band");
        r.feed(Up, 300, 2100, 60, ALL);
        let short = run(
            &mut r,
            &[
                (Down, 300, 2150, 0),
                (Move, 340, 2150, 50),
                (Move, 400, 2150, 150),
                (Up, 400, 2150, 200),
            ],
        );
        assert_eq!(short, Step::Cancel);
        // the N9 map replays it, as v1 did
        r.set_map(Map::N9);
        r.feed(Down, 300, 2150, 0, ALL);
        assert_eq!(r.feed(Move, 340, 2150, 50, ALL), Step::Replay);
        r.feed(Up, 340, 2150, 60, ALL);
    }

    /// N9: a quick flick from the right is the last app; a slow drag Open apps.
    #[test]
    fn n9_right_flick_is_the_last_app() {
        let mut r = pixel();
        r.set_map(Map::N9);
        let flick = run(
            &mut r,
            &[
                (Down, 1075, 900, 0),
                (Move, 1020, 900, 300),
                (Move, 925, 900, 340),
                (Up, 925, 900, 340),
            ],
        );
        assert_eq!(flick, Step::Commit(Gesture::LastApp));
        let drag = run(
            &mut r,
            &[
                (Down, 1075, 900, 0),
                (Move, 1000, 900, 300),
                (Move, 850, 900, 800),
                (Up, 850, 900, 900),
            ],
        );
        assert_eq!(drag, Step::Commit(Gesture::OpenApps));
    }

    /// (G4) Down from the status bar: strictly beyond half the height closes; exactly half, a short drag or a tap don't.
    #[test]
    fn the_top_edge_closes_past_half_the_screen() {
        const TOP: &[Edge] = &[Edge::Left, Edge::Right, Edge::Bottom, Edge::Top];
        use alloc::vec::Vec;
        let swipe = |to: i32| {
            let mut r = pixel();
            let steps: Vec<Step> = [
                (Down, 540, 30, 0),
                (Move, 540, 30 + 40, 10),
                (Move, 540, 30 + to, 400),
                (Up, 540, 30 + to, 410),
            ]
            .iter()
            .map(|&(p, x, y, ms)| r.feed(p, x, y, ms, TOP))
            .collect();
            assert!(!r.busy());
            steps
        };
        // the bar is 25 dp = 75 px; half of 2160 is 1080
        let past = swipe(1081);
        assert_eq!(
            past[1],
            Step::Track {
                edge: Edge::Top,
                progress: 40 * 256 / 1080
            }
        );
        assert_eq!(past[3], Step::Commit(Gesture::Close));
        assert_eq!(swipe(1080)[3], Step::Cancel);
        assert_eq!(swipe(300)[3], Step::Cancel);
        // a quick flick is no close (no fling from the top)
        let mut r = pixel();
        let flick = [(Down, 540, 30, 0), (Move, 540, 400, 10), (Up, 540, 400, 11)]
            .iter()
            .map(|&(p, x, y, ms)| r.feed(p, x, y, ms, TOP))
            .last();
        assert_eq!(flick, Some(Step::Cancel));
        // a tap on the bar is replayed (it opens the shade); below the bar, or without the live edge, nothing is held
        assert_eq!(
            run_on(&mut r, TOP, &[(Down, 540, 30, 0), (Up, 541, 31, 80)]),
            Step::Replay
        );
        assert_eq!(r.feed(Down, 540, 80, 0, TOP), Step::Pass);
        r.feed(Up, 540, 80, 1, TOP);
        assert_eq!(r.feed(Down, 540, 30, 0, ALL), Step::Pass);
    }

    fn run_on(r: &mut Recognizer, live: &[Edge], touches: &[(TouchPhase, i32, i32, u64)]) -> Step {
        touches
            .iter()
            .map(|&(p, x, y, ms)| r.feed(p, x, y, ms, live))
            .last()
            .unwrap()
    }
}

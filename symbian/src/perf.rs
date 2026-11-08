// SPDX-License-Identifier: GPL-3.0-only
//! Frame cost (NeoSymbian R9): how long each frame's render, framebuffer copy and app blits take, summed over about a
//! second of activity and logged once as `neoui: perf ...`, so the phone's speed can be measured without flooding the
//! log. Idle seconds log nothing.

use e32::{dprintln, time};

/// One second's worth of frames.
const WINDOW_MS: u64 = 1000;

#[derive(Default, Clone, Copy)]
struct Stat {
    sum_us: u64,
    max_us: u32,
}

impl Stat {
    fn add(&mut self, us: u32) {
        self.sum_us += u64::from(us);
        self.max_us = self.max_us.max(us);
    }
}

pub struct Perf {
    hz: u32,
    started_ms: u64,
    frames: u32,
    render: Stat,
    copy: Stat,
    app: Stat,
    px: u64,
    // frames composed from snapshots, no Slint render (smooth-ui S2)
    layers: u32,
    layer: Stat,
    // frames that scrolled a list by shifting its pixels (smooth-ui S2d)
    shifts: u32,
}

/// A point in time on the fast counter.
pub struct Mark(u32);

impl Perf {
    pub fn new() -> Self {
        let hz = time::fast_counter_hz();
        dprintln!("neoui: fast counter {} Hz", hz);
        Self {
            hz,
            started_ms: 0,
            frames: 0,
            render: Stat::default(),
            copy: Stat::default(),
            app: Stat::default(),
            px: 0,
            layers: 0,
            layer: Stat::default(),
            shifts: 0,
        }
    }

    pub fn mark(&self) -> Mark {
        Mark(time::fast_counter())
    }

    /// Microseconds since `m`.
    pub fn since(&self, m: &Mark) -> u32 {
        time::counter_us(m.0, time::fast_counter(), self.hz)
    }

    /// Count a frame that drew `px` pixels (shell and app); frames that drew nothing are not frames.
    pub fn frame(&mut self, render_us: u32, copy_us: u32, app_us: u32, px: usize) {
        if px == 0 {
            return;
        }
        let now = time::millis();
        if self.frames + self.layers == 0 {
            self.started_ms = now;
        }
        self.frames += 1;
        self.render.add(render_us);
        self.copy.add(copy_us);
        self.app.add(app_us);
        self.px += px as u64;
        if now.saturating_sub(self.started_ms) >= WINDOW_MS {
            self.flush(now);
        }
    }

    /// Count a frame composed from snapshots (smooth-ui S2): `us` to blend and copy it to the screen.
    pub fn layer(&mut self, us: u32) {
        let now = time::millis();
        if self.frames + self.layers == 0 {
            self.started_ms = now;
        }
        self.layers += 1;
        self.layer.add(us);
        if now.saturating_sub(self.started_ms) >= WINDOW_MS {
            self.flush(now);
        }
    }

    /// Count frames whose render scrolled a list by shifting it (smooth-ui S2d).
    pub fn shifts(&mut self, n: u32) {
        self.shifts += n;
    }

    /// Log a window that activity stopped in the middle of (call on every wake-up).
    pub fn idle(&mut self) {
        let now = time::millis();
        if self.frames + self.layers > 0 && now.saturating_sub(self.started_ms) >= WINDOW_MS {
            self.flush(now);
        }
    }

    fn flush(&mut self, now: u64) {
        let n = u64::from(self.frames.max(1));
        dprintln!(
            "neoui: perf {} frames/{} ms: render avg {} max {} us, copy avg {} max {} us, app avg {} max {} us, {} px, \
             {} layer frames avg {} max {} us, {} shifted",
            self.frames,
            now - self.started_ms,
            self.render.sum_us / n,
            self.render.max_us,
            self.copy.sum_us / n,
            self.copy.max_us,
            self.app.sum_us / n,
            self.app.max_us,
            self.px,
            self.layers,
            self.layer.sum_us / u64::from(self.layers.max(1)),
            self.layer.max_us,
            self.shifts
        );
        let hz = self.hz;
        *self = Self {
            hz,
            started_ms: 0,
            frames: 0,
            render: Stat::default(),
            copy: Stat::default(),
            app: Stat::default(),
            px: 0,
            layers: 0,
            layer: Stat::default(),
            shifts: 0,
        };
    }
}

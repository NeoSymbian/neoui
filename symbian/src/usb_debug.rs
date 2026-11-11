// SPDX-License-Identifier: GPL-3.0-only
//! USB debugging (Settings -> Developer options, `developer.usb_debugging`), like Android's adb switch
//! (docs debug-access.md, roadmap T10). On: the kernel exposes fshell's USB serial port (HAL 0x4e04; the Mac
//! re-enumerates the phone) and this worker keeps `FSHELL.EXE` running on it, restarting it after an exit with a
//! bounded backoff. Off: fshell is killed and the port goes away. Loader IPC, the kill and the reap run on the
//! worker thread, so a missing or slow EXE never stalls the UI.
//!
//! Host tests (the state machine only): `rustc --edition=2024 --test symbian/src/usb_debug.rs -o /tmp/t && /tmp/t`.

/// The first restart waits this long; each quick exit doubles it, up to [`BACKOFF_MAX_MS`].
const BACKOFF_MIN_MS: u64 = 1000;
const BACKOFF_MAX_MS: u64 = 32_000;
/// A run at least this long counts as healthy: the next restart waits [`BACKOFF_MIN_MS`] again.
const STABLE_MS: u64 = 60_000;
/// `KErrNotFound`: no fshell in this ROM. The worker stops trying until the switch goes off and on again.
const NOT_FOUND: i32 = -1;

trait Backend {
    type Child;

    /// A started child, owned by the supervisor until it exits or is killed.
    fn spawn(&mut self) -> Result<Self::Child, i32>;
    /// `None` while it runs, else `(exit type, reason)`.
    fn exit_info(&mut self, child: &Self::Child) -> Option<(i32, i32)>;
    fn kill(&mut self, child: &Self::Child);
    /// Exposes (or removes) the USB port; the kernel's answer.
    fn set_port(&mut self, on: bool) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    /// Off, waiting for a restart's time, or no fshell in the ROM.
    Idle,
    Running,
    /// The switch changed: the port on/off and the kernel's answer.
    Switched(bool, i32),
    Started,
    /// fshell exited (type, reason); the next start waits this many ms.
    Exited(i32, i32, u64),
    SpawnFailed(i32),
    Missing,
}

struct Supervisor<B: Backend> {
    on: bool,
    child: Option<B::Child>,
    missing: bool,
    started_ms: u64,
    next_spawn_ms: u64,
    backoff_ms: u64,
}

impl<B: Backend> Supervisor<B> {
    fn new() -> Self {
        Self {
            on: false,
            child: None,
            missing: false,
            started_ms: 0,
            next_spawn_ms: 0,
            backoff_ms: BACKOFF_MIN_MS,
        }
    }

    /// The delay before the next start; doubles for the one after, up to the cap.
    fn take_backoff(&mut self) -> u64 {
        let delay = self.backoff_ms;
        self.backoff_ms = (delay * 2).min(BACKOFF_MAX_MS);
        delay
    }

    /// One step towards `want` at time `now_ms`; at most one action per call.
    fn tick(&mut self, backend: &mut B, want: bool, now_ms: u64) -> Event {
        if want != self.on {
            self.on = want;
            if !want && let Some(child) = self.child.take() {
                backend.kill(&child);
            }
            self.missing = false;
            self.backoff_ms = BACKOFF_MIN_MS;
            self.next_spawn_ms = now_ms;
            return Event::Switched(want, backend.set_port(want));
        }
        if !self.on || self.missing {
            return Event::Idle;
        }
        if let Some(child) = &self.child {
            let Some((kind, reason)) = backend.exit_info(child) else {
                return Event::Running;
            };
            self.child = None;
            if now_ms.saturating_sub(self.started_ms) >= STABLE_MS {
                self.backoff_ms = BACKOFF_MIN_MS;
            }
            let delay = self.take_backoff();
            self.next_spawn_ms = now_ms + delay;
            return Event::Exited(kind, reason, delay);
        }
        if now_ms < self.next_spawn_ms {
            return Event::Idle;
        }
        match backend.spawn() {
            Ok(child) => {
                self.child = Some(child);
                self.started_ms = now_ms;
                Event::Started
            }
            Err(NOT_FOUND) => {
                self.missing = true;
                Event::Missing
            }
            Err(error) => {
                self.next_spawn_ms = now_ms + self.take_backoff();
                Event::SpawnFailed(error)
            }
        }
    }
}

#[cfg(not(test))]
mod native {
    use super::{Backend, Event, Supervisor};
    use alloc::boxed::Box;
    use e32::process::Process;
    use e32::sync::{Mutex, Semaphore};
    use e32::thread::{self, JoinHandle};
    use e32::{dprintln, time};

    const ENEO_HAL_USB_DEBUG_SET: i32 = 0x4e04;
    /// The worker looks at fshell this often (and at once when the switch moves).
    const TICK_US: i32 = 500_000;
    const SHELL: &str = "FSHELL.EXE";
    /// No `waitdcd`: fshell outlives each host session (T9), so a Mac closing the port doesn't end it.
    const ARGUMENTS: &str =
        "--console vt100busdevcons --console-title pdd=euart1,ldd=ecomm,port=1,sizedetect=0";

    struct Native;

    impl Backend for Native {
        type Child = Process;

        fn spawn(&mut self) -> Result<Process, i32> {
            let child = Process::spawn(SHELL, ARGUMENTS)?;
            child.resume();
            Ok(child)
        }

        fn exit_info(&mut self, child: &Process) -> Option<(i32, i32)> {
            child.exit_info()
        }

        fn kill(&mut self, child: &Process) {
            child.kill(0);
        }

        fn set_port(&mut self, on: bool) -> i32 {
            // SAFETY: the function takes its argument by value in a1 (1 on, 0 off); a2 is unused.
            unsafe {
                e32::sys::ui::hal_function(
                    1,
                    ENEO_HAL_USB_DEBUG_SET,
                    usize::from(on) as *mut u8,
                    core::ptr::null_mut(),
                )
            }
        }
    }

    struct Shared {
        want: Mutex<bool>,
        wake: Semaphore,
    }

    /// The switch's worker: [`UsbDebug::set`] from the UI thread, everything else on the worker.
    pub struct UsbDebug {
        shared: &'static Shared,
        last: bool,
        _worker: JoinHandle,
    }

    impl UsbDebug {
        /// Starts the worker with the switch at `on`.
        ///
        /// # Errors
        /// The kernel's error creating the mutex, semaphore or thread.
        pub fn start(on: bool) -> Result<Self, i32> {
            // lives as long as SYSSTART (Arc needs compare-and-swap, which ARMv5TE lacks)
            let shared: &'static Shared = Box::leak(Box::new(Shared {
                want: Mutex::new(on)?,
                wake: Semaphore::new(0)?,
            }));
            let worker = thread::spawn(move || {
                let mut backend = Native;
                let mut supervisor = Supervisor::new();
                let mut last_error = None;
                loop {
                    let want = *shared.want.lock();
                    match supervisor.tick(&mut backend, want, time::millis()) {
                        Event::Switched(on, r) => {
                            last_error = None;
                            dprintln!(
                                "neoui: USB debugging {} (port -> {})",
                                if on { "on" } else { "off" },
                                r
                            );
                            continue; // start fshell now, not a tick later
                        }
                        Event::Started => dprintln!("neoui: USB debugging: fshell started"),
                        Event::Exited(kind, reason, delay) => dprintln!(
                            "neoui: USB debugging: fshell exited ({}, {}); restart in {} ms",
                            kind,
                            reason,
                            delay
                        ),
                        Event::SpawnFailed(error) if last_error != Some(error) => {
                            last_error = Some(error);
                            dprintln!("neoui: USB debugging: fshell didn't start ({})", error);
                        }
                        Event::Missing => {
                            dprintln!("neoui: USB debugging: no {} in this ROM", SHELL)
                        }
                        Event::Idle | Event::Running | Event::SpawnFailed(_) => {}
                    }
                    shared.wake.wait_timeout(TICK_US);
                }
            })?;
            Ok(Self {
                shared,
                last: on,
                _worker: worker,
            })
        }

        /// The switch's current value; wakes the worker when it changed.
        pub fn set(&mut self, on: bool) {
            if on != self.last {
                self.last = on;
                *self.shared.want.lock() = on;
                self.shared.wake.signal();
            }
        }
    }
}

#[cfg(not(test))]
pub use native::UsbDebug;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Child(Rc<Cell<u32>>);

    impl Drop for Child {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }

    #[derive(Default)]
    struct Fake {
        live: Rc<Cell<u32>>,
        spawns: u32,
        kills: u32,
        port: Vec<bool>,
        exit: Option<(i32, i32)>,
        failure: Option<i32>,
    }

    impl Backend for Fake {
        type Child = Child;

        fn spawn(&mut self) -> Result<Child, i32> {
            self.spawns += 1;
            if let Some(error) = self.failure {
                return Err(error);
            }
            assert_eq!(self.live.get(), 0, "one fshell at a time");
            self.live.set(1);
            Ok(Child(self.live.clone()))
        }

        fn exit_info(&mut self, _child: &Child) -> Option<(i32, i32)> {
            self.exit.take()
        }

        fn kill(&mut self, _child: &Child) {
            self.kills += 1;
        }

        fn set_port(&mut self, on: bool) -> i32 {
            self.port.push(on);
            0
        }
    }

    #[test]
    fn off_by_default_does_nothing() {
        let mut b = Fake::default();
        let mut s = Supervisor::new();
        for t in 0..10 {
            assert_eq!(s.tick(&mut b, false, t * 1000), Event::Idle);
        }
        assert_eq!((b.spawns, b.port.len()), (0, 0));
    }

    #[test]
    fn on_exposes_the_port_then_starts_one_fshell() {
        let mut b = Fake::default();
        let mut s = Supervisor::new();
        assert_eq!(s.tick(&mut b, true, 0), Event::Switched(true, 0));
        assert_eq!(s.tick(&mut b, true, 0), Event::Started);
        for t in 1..100 {
            assert_eq!(s.tick(&mut b, true, t * 500), Event::Running);
        }
        assert_eq!(
            (b.spawns, b.live.get(), b.port.as_slice()),
            (1, 1, &[true][..])
        );
    }

    #[test]
    fn quick_exits_back_off_to_the_cap_and_a_stable_run_resets() {
        let mut b = Fake::default();
        let mut s = Supervisor::new();
        let mut now = 0;
        s.tick(&mut b, true, now);
        let mut delays = Vec::new();
        for _ in 0..8 {
            assert_eq!(s.tick(&mut b, true, now), Event::Started);
            b.exit = Some((0, -29));
            now += 10;
            let Event::Exited(0, -29, delay) = s.tick(&mut b, true, now) else {
                panic!("expected an exit");
            };
            assert_eq!(b.live.get(), 0, "reaped before a restart");
            assert_eq!(
                s.tick(&mut b, true, now + delay - 1),
                Event::Idle,
                "not before the delay"
            );
            delays.push(delay);
            now += delay;
        }
        assert_eq!(delays, [1000, 2000, 4000, 8000, 16000, 32000, 32000, 32000]);
        // a healthy run: the next restart is quick again
        assert_eq!(s.tick(&mut b, true, now), Event::Started);
        now += STABLE_MS;
        b.exit = Some((2, 3));
        assert_eq!(s.tick(&mut b, true, now), Event::Exited(2, 3, 1000));
    }

    #[test]
    fn off_kills_fshell_removes_the_port_and_never_respawns() {
        let mut b = Fake::default();
        let mut s = Supervisor::new();
        s.tick(&mut b, true, 0);
        s.tick(&mut b, true, 0);
        assert_eq!(s.tick(&mut b, false, 5), Event::Switched(false, 0));
        assert_eq!((b.kills, b.live.get()), (1, 0));
        assert_eq!(b.port, [true, false]);
        for t in 0..20 {
            assert_eq!(s.tick(&mut b, false, 10 + t * 1000), Event::Idle);
        }
        assert_eq!(b.spawns, 1);
    }

    #[test]
    fn missing_fshell_stops_until_switched_again() {
        let mut b = Fake {
            failure: Some(NOT_FOUND),
            ..Fake::default()
        };
        let mut s = Supervisor::new();
        s.tick(&mut b, true, 0);
        assert_eq!(s.tick(&mut b, true, 0), Event::Missing);
        for t in 1..50 {
            assert_eq!(s.tick(&mut b, true, t * 1000), Event::Idle);
        }
        assert_eq!(b.spawns, 1);
        b.failure = None;
        s.tick(&mut b, false, 60_000);
        s.tick(&mut b, true, 60_001);
        assert_eq!(s.tick(&mut b, true, 60_001), Event::Started);
    }

    #[test]
    fn failed_starts_retry_with_backoff_without_holding_a_child() {
        let mut b = Fake {
            failure: Some(-4),
            ..Fake::default()
        };
        let mut s = Supervisor::new();
        s.tick(&mut b, true, 0);
        assert_eq!(s.tick(&mut b, true, 0), Event::SpawnFailed(-4));
        assert_eq!(s.tick(&mut b, true, 999), Event::Idle);
        assert_eq!(s.tick(&mut b, true, 1000), Event::SpawnFailed(-4));
        assert_eq!(s.tick(&mut b, true, 2999), Event::Idle);
        b.failure = None;
        assert_eq!(s.tick(&mut b, true, 3000), Event::Started);
        assert_eq!(b.live.get(), 1);
    }
}

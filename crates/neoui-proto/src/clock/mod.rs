// SPDX-License-Identifier: Apache-2.0
//! Time keeping and the Clock app's models. Everything here is pure (no clock, no file system) and host-tested; the
//! UI server and the Clock app pass the time in.
//!
//! | Module | Owns |
//! |---|---|
//! | [`saved`] | the clock across restarts: "UTC minus a reference clock" and the zone (`clock.dat`) |
//! | [`local`] | the "Set time" editor ([`Local`]) and calendar helpers |
//! | [`alarm`] | alarms: weekdays, next due, snooze, and their settings blob |
//! | [`face`] | the face as text: 24/12-hour, the date line, the next alarm (home widget and the app) |
//! | [`world`] | world clocks: the city catalogue, times there, and the saved list |
//! | [`timer`] | the countdown timer's state machine |
//! | [`stopwatch`] | the stopwatch and its laps |
//! | [`wire`] | the small byte codec the settings blobs share |

pub mod alarm;
pub mod face;
pub mod local;
pub mod saved;
pub mod stopwatch;
pub mod timer;
pub mod wire;
pub mod world;

pub use local::{FIELDS, Local, days_in_month};
pub use saved::{DIR, LEN, PATH, Saved};

// SPDX-License-Identifier: Apache-2.0
//! Time keeping and what the shell and the Clock app share. Everything here is pure (no clock, no file system) and
//! host-tested; the UI server and the Clock app pass the time in.
//!
//! | Module | Owns |
//! |---|---|
//! | [`saved`] | the clock across restarts: "UTC minus a reference clock" and the zone (`clock.dat`) |
//! | [`local`] | the "Set time" editor ([`Local`]) and calendar helpers |
//! | [`alarm`] | a minute of the week, day names, "07:30" (the alarms themselves are DTSRV's, NeoSymbian/datetime) |
//! | [`face`] | the face as text: 24/12-hour, the date line, the next alarm (home widget and the app) |
//! | [`next`] | `dtsrv.next`: the next alarm and event DTSRV publishes, and their text |
//! | [`world`] | world clocks: the city catalogue, times there, and the saved list |
//! | [`tz`] | time zone rules (POSIX `TZ` strings): the offset at a time, daylight saving |
//! | [`wire`] | the small byte codec the settings blobs share |

pub mod alarm;
pub mod datetime;
pub mod face;
pub mod local;
pub mod next;
pub mod saved;
pub mod tz;
pub mod wire;
pub mod world;
pub mod zones;
#[cfg(test)]
mod zones_ref;
#[cfg(test)]
mod zones_test;

pub use local::{FIELDS, Local, days_in_month};
pub use saved::{DIR, LEN, PATH, Saved};

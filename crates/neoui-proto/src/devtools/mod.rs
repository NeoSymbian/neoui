// SPDX-License-Identifier: Apache-2.0
//! The Dev tools app's models. Everything here is pure (no HAL, no file system) and host-tested; the app reads the
//! kernel and the servers and passes the results in.
//!
//! | Module | Owns |
//! |---|---|
//! | [`device`] | the phone's components, the kernel device registry's lines (HAL 0x4e19), and merging them with probes |
//! | [`log`] | the kernel log as lines: a bounded ring, log tags, filters |
//! | [`stats`] | sample histories, CPU load, and the numbers as text |

pub mod device;
pub mod log;
pub mod stats;

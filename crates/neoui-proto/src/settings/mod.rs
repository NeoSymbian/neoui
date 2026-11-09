// SPDX-License-Identifier: Apache-2.0
//! The settings system (docs/neoui-shell.md, Part 1): one store of typed, validated settings that the UI server
//! keeps for the shell and every app, saved on `C:` and readable, writable and watchable over the `NeoUi` IPC
//! protocol ([`crate::ipc::SETTING_GET`], [`crate::ipc::SETTING_SET`], [`crate::ipc::SETTING_WATCH`]).
//!
//! | Module | Owns |
//! |---|---|
//! | [`value`] | [`Value`], [`ValueType`], the value encoding (IPC and file) |
//! | [`key`] | key names, typed [`Key<T>`] handles, the [`Setting`] types |
//! | [`registry`] | every system setting: type, default, limit, owner; the typed [`keys`] |
//! | [`store`] | [`Store`]: validation, defaults, the change journal |
//! | [`file`] | the versioned file format, forgiving decode, migrations |
//! | [`access`] | [`Caller`] and who may write what |
//! | [`service`] | [`Service`]: store + [`Backend`] + save delay + subscriptions |
//!
//! Everything here is pure (`no_std` + `alloc`) and host-tested; the UI server adds only the file backend.

pub mod access;
mod error;
pub mod file;
pub mod key;
pub mod registry;
pub mod service;
pub mod store;
pub mod value;

pub use access::Caller;
pub use error::{Error, kerr};
pub use key::{Bytes, KEY_MAX, Key, Setting};
pub use registry::{REGISTRY, keys};
pub use service::{Backend, Service};
pub use store::{Changed, Store};
pub use value::{Value, ValueType};

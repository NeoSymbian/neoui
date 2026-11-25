// SPDX-License-Identifier: Apache-2.0
//! The Settings app (`NEOSET.EXE`, docs neoui-shell/settings.md "The Settings app"): its pages, what the shell tells
//! it ([`info`], over `ipc::SETTINGS_INFO`), and the fixed choices its sheets offer.

pub mod choices;
pub mod info;
pub mod page;

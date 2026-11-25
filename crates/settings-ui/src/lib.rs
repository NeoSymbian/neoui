// SPDX-License-Identifier: GPL-3.0-only
//! The Settings app's screens (`NEOSET.EXE`, docs neoui-shell/settings.md "The Settings app"): `SettingsApp` and
//! its `SetState` global. The device crate (`symbian/apps/settings`) fills `SetState` and answers its `act`
//! callback; the tests here draw every page into `snapshots/set2-*.png`.
#![no_std]

slint::include_modules!();

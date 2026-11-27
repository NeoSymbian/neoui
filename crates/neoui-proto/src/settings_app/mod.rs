// SPDX-License-Identifier: Apache-2.0
//! The Settings app (`NEOSET.EXE`, docs neoui-shell/settings.md "The Settings app"): its pages, what the shell tells
//! it ([`info`], over `ipc::SETTINGS_INFO`; since 1.20 the paged kinds and owner reports, [`wire`] and [`jobs`]), and the fixed choices its sheets offer.

pub mod app_detail;
pub mod asks;
pub mod choices;
pub mod info;
pub mod jobs;
pub mod owner;
pub mod page;
pub mod permissions;
pub mod privacy;
pub mod row_model;
pub mod rows;
pub mod sim_lock;
pub mod slot;
pub mod storage;
pub mod wallpaper;
pub mod wire;

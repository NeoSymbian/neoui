// SPDX-License-Identifier: GPL-3.0-only
//! (D3) One drive's page (set2-storage-drive): C: or E: from Storage's own count of it (`storage::Storage::drive`):
//! used and free now, Images & video, Music, Documents and Other adding up to the used bytes
//! (`settings_app::storage::drive_kinds`), its file system and size. The count is Storage's scan: it runs while this
//! page or Storage is shown. Eject for USB waits for FS6 (the page says so).
use crate::Ctx;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use e32::dprintln;
use neoui_proto::settings_app::page::Page;
use neoui_proto::settings_app::storage as st;
use settings_ui::{PartRow, SetState, SettingsApp};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

/// "storage-drive" + a letter: that drive's page (C: and E:, the drives Storage counts).
pub fn open(ctx: &Ctx, ui: &SettingsApp, letter: &str) {
    let s = ui.global::<SetState>();
    let Some(d) = s.get_drives().iter().find(|d| d.letter == letter) else {
        return;
    };
    if ctx
        .storage
        .drive(letter.chars().next().unwrap_or(' '))
        .is_none()
    {
        return;
    }
    dprintln!("settings: drive page {}", letter);
    let title = d.title.clone();
    s.set_drive_current(d);
    ctx.go(Page::StorageDrive);
    s.set_title(title);
    show(ctx, ui);
}

/// The open drive's page from its count now (each tick while it is shown).
pub fn show(ctx: &Ctx, ui: &SettingsApp) {
    let s = ui.global::<SetState>();
    let letter = s.get_drive_current().letter.chars().next().unwrap_or(' ');
    let Some(v) = ctx.storage.drive(letter) else {
        return;
    };
    s.set_drive_used(st::size_text(v.used).into());
    s.set_drive_of(
        format!(
            "of {} · {} free",
            st::size_text(v.size),
            st::size_text(v.free)
        )
        .into(),
    );
    // the kinds once counted (the running count while counting); none before: nothing is shown as zero
    let mut rows = Vec::new();
    if v.counted || v.counting {
        let mut start = 0.0;
        for (name, bytes) in st::drive_kinds(v.used, &v.tally) {
            let share = if v.size > 0 {
                bytes as f32 / v.size as f32
            } else {
                0.0
            };
            rows.push(PartRow {
                name: name.into(),
                size: st::size_text(bytes).into(),
                start,
                share,
            });
            start += share;
        }
    }
    s.set_drive_kinds(ModelRc::new(VecModel::from(rows)));
    s.set_drive_fs(
        if v.fs.is_empty() {
            String::from("Unknown")
        } else {
            v.fs
        }
        .into(),
    );
    s.set_drive_size(st::size_text(v.size).into());
    s.set_drive_status(
        match (v.counting, v.counted, &v.at) {
            (true, _, _) => String::from("Counting…"),
            (false, true, Some(at)) if v.partial => {
                format!("Last counted {at} · some folders couldn't be read")
            }
            (false, true, Some(at)) => format!("Last counted {at}"),
            _ => String::from("Not counted yet"),
        }
        .into(),
    );
}

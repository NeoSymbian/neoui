// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton.
mod common;

#[test]
fn launcher_pixel() {
    common::snapshot_and_tap("pixel", 1080, 2160);
}

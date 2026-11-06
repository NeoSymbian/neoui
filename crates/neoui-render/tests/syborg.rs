// SPDX-License-Identifier: Apache-2.0
// Own test binary (process): Slint's platform is a per-process singleton.
mod common;

#[test]
fn launcher_syborg() {
    common::snapshot_and_tap("syborg", 768, 640);
}

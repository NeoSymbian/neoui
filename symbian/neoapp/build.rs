// SPDX-License-Identifier: GPL-3.0-only
//! Tells dependent crates where neoui's shared Slint files are (`DEP_NEOAPP_UI`): an app in another repo imports
//! `@neoui/theme.slint`, the fonts and the components from the same checkout as this crate, so there is one theme.

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui");
    let ui = dir.canonicalize().unwrap_or(dir);
    println!("cargo:ui={}", ui.display());
    println!("cargo:rerun-if-changed=build.rs");
}

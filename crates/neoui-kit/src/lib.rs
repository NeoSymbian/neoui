// SPDX-License-Identifier: Apache-2.0
//! The neoui kit for apps in any repo: one theme, one set of components, one look (docs `neoui-shell/kit.md`).
//!
//! The kit is the Slint library in neoui's `ui/` folder, imported as `@neoui`:
//! `import { Theme, Header, Footer, NeoList, ListRow } from "@neoui/kit.slint";`. Use this crate as a
//! build-dependency and compile the app's UI with it:
//!
//! ```no_run
//! // build.rs
//! neoui_kit::compile("ui/app.slint");
//! ```
//!
//! The app side of the protocol (connect, draw, events, pages, dialogs, toasts) is `neoapp`.
use std::path::PathBuf;

/// neoui's `ui/` folder: the `@neoui` library (theme, fonts, glyphs, components). It sits beside this crate in a
/// neoui checkout, so it works from a path or a git dependency alike.
pub fn ui_dir() -> PathBuf {
    let ui = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ui");
    ui.canonicalize().unwrap_or(ui)
}

/// The compiler setup every neoui app uses: images and glyphs baked in for the software renderer (no files on the
/// target), SDF text (font sizes scale with the screen) and `@neoui` mapped to [`ui_dir`].
pub fn config() -> slint_build::CompilerConfiguration {
    config_with(&[])
}

/// [`config`] plus the app's own libraries (`("phone-shared", dir)` → `@phone-shared/...`): parts several apps of one
/// repo share.
pub fn config_with(libs: &[(&str, PathBuf)]) -> slint_build::CompilerConfiguration {
    let mut map = std::collections::HashMap::from([(String::from("neoui"), ui_dir())]);
    for (name, dir) in libs {
        map.insert(
            String::from(*name),
            dir.canonicalize().unwrap_or(dir.clone()),
        );
    }
    let libs = map;
    slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer)
        .with_sdf_fonts(true)
        .with_library_paths(libs)
}

/// Compile `path` (relative to the calling crate) with [`config`]; panics with the compiler's message on an error,
/// as a build script should.
pub fn compile(path: &str) {
    compile_with(path, &[]);
}

/// [`compile`] with the app's own libraries (see [`config_with`]); paths relative to the calling crate.
pub fn compile_with(path: &str, libs: &[(&str, PathBuf)]) {
    if let Err(e) = slint_build::compile_with_config(path, config_with(libs)) {
        panic!("compile {path}: {e}");
    }
    println!("cargo:rerun-if-changed={}", ui_dir().display());
    for (_, dir) in libs {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_library_has_the_kit() {
        assert!(super::ui_dir().join("kit.slint").is_file());
        assert!(super::ui_dir().join("theme.slint").is_file());
    }
}

// SPDX-License-Identifier: Apache-2.0
fn main() {
    // Glyphs are baked in at build time (no font files on the target). SDF so
    // the window-scaled font sizes render at any pixel size.
    slint_build::compile_with_config(
        "../../ui/shell.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer)
            .with_sdf_fonts(true),
    )
    .expect("compile ui/shell.slint");
}

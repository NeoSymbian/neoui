// SPDX-License-Identifier: GPL-3.0-only
fn main() {
    // As neoui-render: glyphs and images baked in (no font files on the target), SDF text.
    slint_build::compile_with_config(
        "ui/devtools.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer)
            .with_sdf_fonts(true),
    )
    .expect("compile ui/devtools.slint");
}

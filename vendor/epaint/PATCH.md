# Vendored epaint 0.31.1 (patched)

Upstream: https://crates.io/crates/epaint/0.31.1 (MIT OR Apache-2.0, license unchanged).

Patch (src/text/fonts.rs only): `FontData` gains `variations: Vec<([u8; 4], f32)>` and a
`variations()` builder; `ab_glyph_font_from_font_data` applies them with
`ab_glyph::VariableFont::set_variation` so a variable system font (SF Pro) can be loaded at a
real weight. With no variations the behavior is identical to upstream. Remove this directory and
the `[patch.crates-io]` entry once upstream exposes variable-font axes.

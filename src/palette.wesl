//! The finished picture times the engine's palette multiplier.
//!
//! Runs as a `FullscreenMaterial` on the world camera, after its main pass: the source is the
//! picture the camera just produced, and the output replaces it. `palette.rs` owns the numbers.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct PaletteTint {
    multiplier: vec4<f32>,
}

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;
@group(0) @binding(2) var<uniform> palette: PaletteTint;

@fragment
fn fs_main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let source = textureSample(screen_texture, texture_sampler, in.uv);
    // `stdPalEffects_UpdatePalette`: the palette byte is scaled by `1 + tint/2` and clamped at 255,
    // which the sRGB target's clamp of a >1 channel gives for free.
    return vec4<f32>(source.rgb * palette.multiplier.rgb, source.a);
}
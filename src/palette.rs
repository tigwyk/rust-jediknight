//! The engine's palette tint as a fullscreen pass over the finished picture.
//!
//! `stdPalEffects_UpdatePalette` scales palette **bytes** by `1 + tint/2`: the player's damage tint
//! (`sithActor_DamageActor` -> `sithPlayer_AddDynamicTint(damage * 0.04, 0, 0)`, decaying 0.4/s) is
//! applied after the renderer has produced the picture, not to any one mesh. A fixed-function blend
//! cannot express it: a multiply blend's source factor is clamped by the Unorm target, so the
//! brightening half (`byte + byte*tint/2`) saturates and an identity multiply reads as white but a
//! 1.5x one cannot be asked for.
//!
//! A `FullscreenMaterial` pass on the world camera runs after that camera's main pass (a non-HDR
//! camera tone maps its fragments in-shader), so the shader sees the finished picture and can do the
//! engine's byte rule exactly.
//!
//! `ponytail:` this pass carries the damage tint only. The scripted `NewColorEffect` channels
//! (filter/tint/add) still go through the wash quad; fold them in here when the wash's approximation
//! is retired (roadmap Phase 1 step 2).

use bevy::asset::uuid::Uuid;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::RenderApp;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;

/// Per-camera palette multiplier, from the local player's dynamic tint.
#[derive(Component, ExtractComponent, Clone, Copy, Debug, ShaderType)]
#[extract_app(RenderApp)]
pub struct PaletteTint {
    /// Linear-light factor per channel. A `vec4` so the uniform is one 16-byte block.
    pub multiplier: Vec4,
}

impl Default for PaletteTint {
    fn default() -> Self {
        Self { multiplier: Vec4::ONE }
    }
}

impl PaletteTint {
    /// The uniform for a runtime tint.
    ///
    /// The pass samples the sRGB picture (decoded to linear) and writes an sRGB target (encoded), so
    /// the factor that scales the stored **bytes** by `1 + tint/2` is that ratio to the 2.2 power.
    /// The engine clamps each byte at 255; the sRGB target clamps a channel above 1 the same way.
    pub fn from_tint(tint: [f32; 3]) -> Self {
        let mut multiplier = Vec4::ONE;
        for (channel, value) in tint.iter().enumerate() {
            multiplier[channel] = (1.0 + value * 0.5).max(0.0).powf(2.2);
        }
        Self { multiplier }
    }

    /// An identity multiplier is a copy of the picture, so the pass has nothing to do.
    pub fn is_identity(&self) -> bool {
        (self.multiplier - Vec4::ONE).abs().max_element() < 1e-4
    }
}

impl FullscreenMaterial for PaletteTint {
    fn fragment_shader() -> ShaderRef {
        PALETTE_SHADER.into()
    }
}

/// The shader text and the handle it is registered under.
pub const PALETTE_SHADER: Handle<Shader> =
    Handle::Uuid(Uuid::from_u128(0x5041_4c54_5449_4e54_0000_0000_0000_0001), core::marker::PhantomData);

/// Registers the palette pass and its shader.
pub struct PalettePlugin;

impl Plugin for PalettePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::load_internal_asset!(app, PALETTE_SHADER, "palette.wesl", Shader::from_wesl);
        app.add_plugins(FullscreenMaterialPlugin::<PaletteTint>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::PaletteTint;

    /// The engine's palette rule: a channel's displayed byte scales by `1 + tint/2`, nothing else.
    #[test]
    fn the_palette_multiplier_is_the_engine_ratio() {
        let identity = PaletteTint::from_tint([0.0; 3]);
        assert!(identity.is_identity(), "{identity:?}");
        let red = PaletteTint::from_tint([1.0, 0.0, 0.0]);
        // `byte * 1.5` in the picture means `1.5^2.2` in linear light.
        assert!((red.multiplier.x - 1.5f32.powf(2.2)).abs() < 1e-4, "{red:?}");
        assert_eq!((red.multiplier.y, red.multiplier.z), (1.0, 1.0));
        // A tint below -2 would ask for a negative byte, which a palette cannot hold.
        assert_eq!(PaletteTint::from_tint([-4.0, 0.0, 0.0]).multiplier.x, 0.0);
    }
}

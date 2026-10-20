//! A level-surface material with the `.cmp` light ramp in the shader.
//!
//! `StandardMaterial` cannot express what the retail renderer does here: it walks a *light level*
//! down with distance through the colormap's brightness tables (§6), and force seeing swaps in an
//! infrared pass. Both are per-fragment terms, so they need a fragment shader.
//!
//! The shader is registered from a string (`load_internal_asset!`), so a drop-in install needs no
//! asset folder next to the executable.
//!
//! `ponytail:` the light level is still the baked vertex colour the mesh carries; the shader adds
//! the **distance** term (a fade toward the colormap's darkest table) and the infrared pass. The
//! engine's own distance-to-level curve is not recovered — this reproduces its shape, with the fade
//! stops calibrated against the retail screenshots rather than read out of the binary.

use bevy::asset::uuid::Uuid;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// The shader's per-draw parameters. `ShaderType` is what `AsBindGroup` needs for a uniform struct.
// Scalars, not `[f32; 4]`: encase requires a uniform array's stride to be a multiple of 16, and a
// `[f32; 4]` field has stride 4, which panics the uniform writer ("array stride must be a multiple
// of 16"). Four floats say the same thing and cost nothing.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct LevelParams {
    /// The colour to fade toward, from the colormap's darkest table.
    pub far_r: f32,
    pub far_g: f32,
    pub far_b: f32,
    /// The distance the fade starts at.
    pub fade_start: f32,
    /// The distance the fade reaches full strength at.
    pub fade_end: f32,
    /// `EnableIRMode`: force seeing's infrared pass.
    pub infra: f32,
    /// How many cels this material's bitmap holds, from its `.mat` header. A surface's cel arrives in
    /// the vertex colour's alpha (see `Batch::push_polygon`), and `level.wesl` maps it into the packed
    /// column; 1 means "no animation" and the term is skipped.
    pub cels: f32,
    pub _pad0: f32,
    pub _pad1: f32,
    pub _pad2: f32,
}

impl Default for LevelParams {
    fn default() -> Self {
        Self {
            // The colormap's darkest table is `[20, 13, 5]` on level 1 (§6); `--cmp` prints it.
            far_r: 20.0 / 255.0,
            far_g: 13.0 / 255.0,
            far_b: 5.0 / 255.0,
            fade_start: 30.0,
            fade_end: 320.0,
            infra: 0.0,
            cels: 1.0,
            _pad0: 0.0,
            _pad1: 0.0,
            _pad2: 0.0,
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct LevelMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub base_color_texture: Option<Handle<Image>>,
    #[uniform(2)]
    pub params: LevelParams,
}

impl Material for LevelMaterial {
    fn fragment_shader() -> ShaderRef {
        LEVEL_HANDLE.into()
    }

    // Draw both sides, like the CPU path does (§16.13). `StandardMaterial` says `cull_mode: None`
    // because the `.jkl` winding is not consistent; a custom material has no such field, but the
    // pipeline descriptor does. Without this the shader path culls back faces and a wall whose front
    // face points away reveals whatever is behind it — which is how this material first rendered a
    // bar's hazard stripes where the other path showed panelling.
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The shader text and the handle it is registered under.

pub const LEVEL_HANDLE: Handle<Shader> =
    Handle::Uuid(Uuid::from_u128(0x4a4b_4c45_5645_4c00_0001_0000_0000_0001), core::marker::PhantomData);

/// Registers the shader and the material plugin.
pub struct LevelMaterialPlugin;

impl Plugin for LevelMaterialPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::load_internal_asset!(app, LEVEL_HANDLE, "level.wesl", Shader::from_wesl);
        app.add_plugins(MaterialPlugin::<LevelMaterial>::default());
    }
}

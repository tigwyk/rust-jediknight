// The level-surface shader: a *fragment-only* material, which is how Bevy 0.20 custom materials work
// — the framework's mesh pipeline supplies the vertex stage and the inter-stage `VertexOutput`, so a
// material never declares its own vertex entry point or its own output struct.
//
// Bindings live at `MATERIAL_BIND_GROUP` (3): group 0 is the view, group 2 the mesh, and a material's
// own `AsBindGroup` bindings are group 3. Declaring them at 2 fails pipeline validation with
// "group 2 binding 0 is not available in the pipeline layout".
import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::forward_io::VertexOutput;

struct LevelParams {
    far_r: f32,
    far_g: f32,
    far_b: f32,
    fade_start: f32,
    fade_end: f32,
    infra: f32,
    pad0: f32,
    pad1: f32,
};

@group(3) @binding(0) var base_color_texture: texture_2d<f32>;
@group(3) @binding(1) var base_color_sampler: sampler;
@group(3) @binding(2) var<uniform> params: LevelParams;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex = textureSample(base_color_texture, base_color_sampler, in.uv);
    var base = tex * in.color;
    // The light ramp's distance term: near geometry is untouched, far geometry walks toward the
    // colormap's darkest table, which is what the retail renderer does with its brightness tables.
    let d = distance(in.world_position.xyz, view.world_position.xyz);
    let t = clamp((d - params.fade_start) / max(params.fade_end - params.fade_start, 0.001), 0.0, 1.0);
    let far = vec3<f32>(params.far_r, params.far_g, params.far_b);
    base = vec4<f32>(mix(base.rgb, far, t), base.a);

    // Force seeing: lift the reds and drop the blues, the way the infrared pass reads.
    if (params.infra > 0.5) {
        let grey = dot(base.rgb, vec3<f32>(0.3, 0.6, 0.1));
        base = vec4<f32>(vec3<f32>(grey * 1.4, grey * 0.6, grey * 0.4), base.a);
    }
    return base;
}

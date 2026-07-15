// Camera-relative dust mote / velocity streak.
//
// The mote textures are white with their shape carried entirely in alpha, so
// this shader tints them at runtime and every mote in a layer can share one
// material. Per-mote variation (size, streak length, position) rides in the
// Transform instead of a uniform, which is what keeps the material count at
// one-per-layer rather than one-per-mote.
//
// The two screen-space masks are the reason this is a custom material rather
// than a StandardMaterial:
//
//  - centre fade: motes crossing the middle of the viewscreen distract from
//    targeting and navigation, so they fade out there and the streaks live in
//    peripheral vision. This also conveniently hides the degenerate case in
//    the CPU-side billboard alignment, where a mote travelling straight at the
//    camera has no meaningful projected direction.
//  - edge fade: motes fade shortly before leaving view instead of popping.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view

struct DustMoteMaterial {
    tint_r: f32,
    tint_g: f32,
    tint_b: f32,
    brightness: f32,
    opacity: f32,
    centre_fade_inner: f32,
    centre_fade_outer: f32,
    edge_fade: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> material: DustMoteMaterial;
@group(#{MATERIAL_BIND_GROUP}) @binding(1)
var mote_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2)
var mote_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // White RGB, shape in alpha — sample alpha only and supply our own colour.
    let texel = textureSample(mote_texture, mote_sampler, in.uv);

    // Fragment position is in physical pixels; normalise against the viewport
    // to get a resolution-independent screen position in -1..1.
    let screen_uv = in.position.xy / view.viewport.zw;
    let ndc = (screen_uv - vec2<f32>(0.5)) * 2.0;

    // Radial distance from screen centre drives the centre mask: 0 at the
    // inner radius (fully hidden), 1 beyond the outer radius (fully visible).
    let centre_dist = length(ndc);
    let centre_mask = smoothstep(
        material.centre_fade_inner,
        material.centre_fade_outer,
        centre_dist,
    );

    // Chebyshev distance reaches 1 exactly at each screen edge, so the edge
    // fade band is uniform along all four sides (unlike a radial measure,
    // which would fade the corners far too early).
    let edge_dist = max(abs(ndc.x), abs(ndc.y));
    let edge_mask = 1.0 - smoothstep(1.0 - material.edge_fade, 1.0, edge_dist);

    let alpha = texel.a * material.opacity * centre_mask * edge_mask;
    if (alpha <= 0.001) {
        discard;
    }

    let tint = vec3<f32>(material.tint_r, material.tint_g, material.tint_b);
    return vec4<f32>(tint * material.brightness, alpha);
}

// Engine trail ribbon shader — layered "ion trail" look.
//
// Samples a scrolling wispy-noise texture (the flowing plasma), perturbs its
// UVs with a distortion map (the wiggle), shapes the cross-ribbon brightness
// with a soft gradient profile (bright core, soft edges), and breaks up the
// tail fade with a dissolve mask instead of a hard linear cutoff. Additive
// blending makes overlapping trail segments read as glowing energy rather
// than opaque smoke.

#import bevy_pbr::forward_io::VertexOutput

struct EngineTrailMaterial {
    color_r: f32,
    color_g: f32,
    color_b: f32,
    color_a: f32,
    time: f32,
    scroll_speed: f32,
    distortion_strength: f32,
    _pad0: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var noise_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var noise_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var distortion_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var distortion_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var gradient_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var gradient_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var dissolve_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var dissolve_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var<uniform> material: EngineTrailMaterial;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // in.color.a carries the crumb's age fade (1.0 fresh near the ship, 0.0
    // at end of life); in.uv.x runs 0 (head) -> 1 (tail) along the ribbon,
    // in.uv.y runs 0 -> 1 across its width.
    let age_alpha = in.color.a;
    let u = in.uv.x;
    let v = in.uv.y;

    let scroll = material.time * material.scroll_speed;

    // UV distortion: sample the distortion map on its own slow scroll and
    // use it to offset the flow-texture lookup, giving the ribbon a subtle
    // wiggle instead of looking like a rigid strip.
    let distort_uv = vec2<f32>(u * 1.5 - scroll * 0.6, v);
    let distortion = textureSample(distortion_texture, distortion_sampler, distort_uv).rg * 2.0 - 1.0;

    let flow_uv = vec2<f32>(u * 2.0 - scroll, v) + distortion * material.distortion_strength;
    let noise = textureSample(noise_texture, noise_sampler, flow_uv).r;

    // Soft gradient across the ribbon width: bright core, feathered edges.
    let profile = textureSample(gradient_texture, gradient_sampler, vec2<f32>(0.5, v)).r;

    // Dissolve from the tail first: the dissolve mask's brightness raises or
    // lowers the effective age threshold at which a pixel disappears, so the
    // trail evaporates unevenly rather than fading as a flat gradient.
    let dissolve_sample = textureSample(
        dissolve_texture,
        dissolve_sampler,
        vec2<f32>(u * 3.0 - scroll * 0.3, v),
    ).r;
    let dissolve_edge = smoothstep(0.0, 0.35, age_alpha - (1.0 - dissolve_sample) * 0.5);

    let intensity = noise * profile * dissolve_edge;

    let base_color = vec3<f32>(material.color_r, material.color_g, material.color_b);
    let rgb = base_color * (0.6 + intensity * 1.8);
    let alpha = clamp(intensity * material.color_a * 2.0, 0.0, 1.0);

    return vec4<f32>(rgb, alpha);
}

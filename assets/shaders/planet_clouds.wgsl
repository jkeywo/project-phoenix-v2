// Planet cloud/smog/ash shell: alpha-blended sphere slightly larger than the
// surface sphere. Same star-relative soft-terminator lighting as the surface.
// `misc.x` (drift_speed) scrolls the UVs longitudinally; the texture sampler
// repeats in U so the seam wraps.

#import bevy_pbr::forward_io::VertexOutput

struct PlanetCloudParams {
    light_dir: vec3<f32>,
    time: f32,
    // x: drift_speed (UV wraps/sec), y: has_opacity, z: ambient_floor, w: reserved
    misc: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> params: PlanetCloudParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var albedo_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var albedo_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var opacity_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var opacity_smp: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let light_dir = normalize(params.light_dir);
    let ambient_floor = params.misc.z;

    let uv = vec2<f32>(in.uv.x + params.time * params.misc.x, in.uv.y);

    let albedo = textureSample(albedo_tex, albedo_smp, uv).rgb;
    var alpha: f32;
    if (params.misc.y > 0.5) {
        alpha = textureSample(opacity_tex, opacity_smp, uv).r;
    } else {
        // No opacity map: use albedo luminance so dark cloud gaps go clear.
        alpha = dot(albedo, vec3<f32>(0.299, 0.587, 0.114));
    }

    let ndotl = dot(n, light_dir);
    let day = smoothstep(-0.05, 0.15, ndotl);
    let lit = ambient_floor + day * max(ndotl, 0.0);

    return vec4<f32>(albedo * lit, alpha);
}

// Planet surface shader: equirectangular texture maps on a UV sphere with
// custom star-relative lighting.
//
// Bevy's directional sun light is `face_player = true` (non-physical), so
// lighting here uses `params.light_dir` — the world-space direction from the
// planet to the star, updated per frame by `update_planet_materials`.

#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view

struct PlanetSurfaceParams {
    light_dir: vec3<f32>,
    emissive_strength: f32,
    atmosphere_colour: vec3<f32>,
    atmosphere_strength: f32,
    // x: has_normal, y: has_roughness, z: has_emissive, w: emissive_night_only
    flags: vec4<f32>,
    // x: has_emissive_mask, y: ambient_floor, z: reserved, w: directional_strength
    misc: vec4<f32>,
    texture_x: vec4<f32>,
    texture_y: vec4<f32>,
    texture_z: vec4<f32>,
    planet_center: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> params: PlanetSurfaceParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var albedo_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var albedo_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var normal_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var rough_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var rough_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var emissive_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var emissive_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var emask_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var emask_smp: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // A sphere's exact geometric normal is radial. Derive it from position
    // instead of using the mesh-normal varying, whose duplicated UV seam can
    // interpolate differently on the two longitude strips in WebGL.
    let n_geo = normalize(in.world_position.xyz - params.planet_center.xyz);
    let light_dir = normalize(params.light_dir);
    let ambient_floor = params.misc.y;
    let texture_normal = vec3<f32>(
        dot(n_geo, params.texture_x.xyz),
        dot(n_geo, params.texture_y.xyz),
        dot(n_geo, params.texture_z.xyz),
    );
    // The UV sphere duplicates its vertices at u = 0/1, so interpolation stays
    // continuous inside every triangle and the repeat sampler performs the one
    // wrap.
    let uv = in.uv;

    // Shading normal: perturb by the tangent-space normal map using an
    // analytic TBN. The mesh (uv_sphere_mesh) has no tangent attribute, but
    // for a Y-up UV sphere dPos/du is proportional to (-n.z, 0, n.x).
    var n = n_geo;
    if (params.flags.x > 0.5) {
        let local_t_raw = vec3<f32>(-texture_normal.z, 0.0, texture_normal.x);
        let t_len = length(local_t_raw);
        // Degenerate at the exact poles — skip perturbation there.
        if (t_len > 1e-4) {
            let local_t = local_t_raw / t_len;
            let local_b = cross(texture_normal, local_t);
            let t = normalize(
                params.texture_x.xyz * local_t.x
                + params.texture_y.xyz * local_t.y
                + params.texture_z.xyz * local_t.z,
            );
            let b = normalize(
                params.texture_x.xyz * local_b.x
                + params.texture_y.xyz * local_b.y
                + params.texture_z.xyz * local_b.z,
            );
            let nm = textureSample(normal_tex, normal_smp, uv).xyz * 2.0 - 1.0;
            n = normalize(t * nm.x + b * nm.y + n_geo * nm.z);
        }
    }

    let ndotl = dot(n, light_dir);
    // Soft terminator so the day/night boundary doesn't alias.
    let day = smoothstep(-0.05, 0.15, ndotl);

    let albedo = textureSample(albedo_tex, albedo_smp, uv).rgb;
    var colour = albedo * (ambient_floor + day * max(ndotl, 0.0) * params.misc.w);

    // Roughness-modulated specular glint (oceans, ice). Subtle by design.
    if (params.flags.y > 0.5) {
        let roughness = textureSample(rough_tex, rough_smp, uv).r;
        let view_dir = normalize(view.world_position - in.world_position.xyz);
        let half_dir = normalize(light_dir + view_dir);
        let gloss = 1.0 - roughness;
        let spec_power = mix(4.0, 64.0, gloss * gloss);
        let spec = pow(max(dot(n, half_dir), 0.0), spec_power) * gloss * 0.5;
        colour += vec3<f32>(spec) * day;
    }

    // Emissive: city lights / nightglow (gated to the night side) or lava
    // (always on). Gate uses the geometric terminator, fading in as the
    // surface leaves daylight.
    if (params.flags.z > 0.5) {
        var night_gate = 1.0;
        if (params.flags.w > 0.5) {
            night_gate = smoothstep(0.1, -0.15, ndotl);
        }
        var mask = 1.0;
        if (params.misc.x > 0.5) {
            mask = textureSample(emask_tex, emask_smp, uv).r;
        }
        let emissive = textureSample(emissive_tex, emissive_smp, uv).rgb;
        colour += emissive * mask * params.emissive_strength * night_gate;
    }

    // Fresnel atmosphere rim, brighter on the day side.
    if (params.atmosphere_strength > 0.0) {
        let view_dir = normalize(view.world_position - in.world_position.xyz);
        let rim = pow(1.0 - clamp(dot(n_geo, view_dir), 0.0, 1.0), 3.0);
        colour += params.atmosphere_colour * params.atmosphere_strength * rim
            * (0.25 + 0.75 * day);
    }

    return vec4<f32>(colour, 1.0);
}

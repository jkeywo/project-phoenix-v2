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
    // x: has_emissive_mask, y: ambient_floor, z: elapsed time, w: directional_strength
    misc: vec4<f32>,
    texture_x: vec4<f32>,
    texture_y: vec4<f32>,
    texture_z: vec4<f32>,
    planet_center: vec4<f32>,
    city: vec4<f32>,
    city_lights: vec4<f32>,
    weather: vec4<f32>,
    neon_colour: vec4<f32>,
    thermal_colour: vec4<f32>,
    traffic_colour: vec4<f32>,
    natural: vec4<f32>, flow: vec4<f32>, effects: vec4<f32>,
    scatter_colour: vec4<f32>, event_colour: vec4<f32>, cloud_flow: vec4<f32>,
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
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var cloud_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(12) var cloud_smp: sampler;


// Bounded shear preserves the authored storms over long sessions. The base
// rotation wraps naturally; latitude-dependent flow never accumulates stretch.
fn advect(uv: vec2<f32>, flow: vec4<f32>, time: f32) -> vec2<f32> {
    let latitude = sin(uv.y * 3.14159265);
    let band = sin(uv.y * flow.w * 6.2831853);
    let phase = uv.x * 6.2831853;
    let eddy = sin(phase * 5.0 + uv.y * 31.0 + time * 0.035)
        * cos(phase * 3.0 - uv.y * 19.0 - time * 0.025);
    return vec2<f32>(uv.x + flow.x * time + latitude * latitude *
        (flow.y * band * sin(time * 0.045) + flow.z * eddy), uv.y);
}

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
    var uv = in.uv;
    if ((params.city.x > 0.5 || params.natural.x > 0.5) && abs(texture_normal.y) > 0.9) {
        // Interpolated UVs fan across the pole's triangles. Recover spherical
        // coordinates there, keeping longitude on this triangle's wrap branch.
        let longitude = atan2(texture_normal.z, texture_normal.x) / 6.2831853;
        uv.x += fract(longitude - uv.x + 0.5) - 0.5;
        uv.y = acos(clamp(texture_normal.y, -1.0, 1.0)) / 3.14159265;
    }

    var normal_strength = select(params.city.y, params.natural.y, params.natural.x > 0.5);
    if (params.natural.x > 0.5) {
        let control = textureSample(rough_tex, rough_smp, uv);
        var flow = params.flow;
        if (params.natural.x < 1.5) {
            flow.z *= mix(0.35, 1.0, control.b);
            flow.y *= mix(0.6, 1.0, control.g);
            normal_strength *= mix(0.6, 1.2, clamp((control.a * 255.0 - 128.0) / 127.0, 0.0, 1.0));
        } else {
            normal_strength *= mix(0.85, 1.1, clamp((control.a * 255.0 - 128.0) / 127.0, 0.0, 1.0));
        }
        uv = advect(uv, flow, params.misc.z);
    }

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
            n = normalize(t * nm.x * normal_strength + b * nm.y * normal_strength + n_geo * nm.z);
        }
    }

    let ndotl = dot(n, light_dir);
    let geometric_light = dot(n_geo, light_dir);
    let night = 1.0 - smoothstep(-0.15, 0.1, geometric_light);
    // Soft terminator so the day/night boundary doesn't alias.
    let day = smoothstep(-0.05, 0.15, ndotl);

    let albedo = textureSample(albedo_tex, albedo_smp, uv).rgb;
    var colour = albedo * (ambient_floor + day * max(ndotl, 0.0) * params.misc.w);
    var day_activity = 0.0;

    if (params.city.x > 0.5) {
        let material = textureSample(rough_tex, rough_smp, uv);
        day_activity = clamp((material.a * 255.0 - 128.0) / 127.0, 0.0, 1.0);
        let view_dir = normalize(view.world_position - in.world_position.xyz);
        let half_dir = normalize(light_dir + view_dir);
        let rough = clamp(material.r, 0.22, 1.0);
        let metal = material.b;
        let nv = max(dot(n, view_dir), 0.001);
        let nl = max(ndotl, 0.0);
        let nh = max(dot(n, half_dir), 0.0);
        let vh = max(dot(view_dir, half_dir), 0.0);
        let a2 = pow(rough, 4.0);
        let d = a2 / max(3.14159265 * pow(nh * nh * (a2 - 1.0) + 1.0, 2.0), 0.0001);
        let k = pow(rough + 1.0, 2.0) / 8.0;
        let geometry = nv / (nv * (1.0 - k) + k) * nl / (nl * (1.0 - k) + k);
        let f0 = mix(vec3<f32>(0.04), albedo, metal);
        let fresnel = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
        let specular = d * geometry * fresnel / max(4.0 * nv * nl, 0.001);
        var shadow = 1.0;
        if (params.weather.z > 0.0) {
            // Intersect the sun ray with the actual cloud shell; a fixed UV
            // offset produces detached shadows near the terminator and poles.
            let scale = params.weather.x;
            let distance = -geometric_light + sqrt(geometric_light * geometric_light + scale * scale - 1.0);
            let q = normalize(n_geo + light_dir * distance);
            let local = vec3<f32>(dot(q, params.texture_x.xyz), dot(q, params.texture_y.xyz), dot(q, params.texture_z.xyz));
            let cloud_uv = vec2<f32>(atan2(local.z, local.x) / 6.2831853 + params.weather.w, acos(clamp(local.y, -1.0, 1.0)) / 3.14159265);
            shadow -= textureSample(cloud_tex, cloud_smp, cloud_uv).r * params.weather.z;
        }
        colour = albedo * ambient_floor * material.g
            + (albedo * (1.0 - metal * 0.65) + specular) * nl * day * shadow * params.misc.w;
    }

    if (params.natural.x > 0.5) {
        let material = textureSample(rough_tex, rough_smp, uv);
        let view_dir = normalize(view.world_position - in.world_position.xyz);
        let half_dir = normalize(light_dir + view_dir);
        let nl = max(dot(n, light_dir), 0.0);
        var shadow = 1.0;
        if (params.natural.z > 0.0 && params.weather.x > 1.0) {
            let shell = params.weather.x;
            let distance = -geometric_light + sqrt(geometric_light * geometric_light + shell * shell - 1.0);
            let q = normalize(n_geo + light_dir * distance);
            let local = vec3<f32>(dot(q, params.texture_x.xyz), dot(q, params.texture_y.xyz), dot(q, params.texture_z.xyz));
            let cloud_uv = vec2<f32>(atan2(local.z, local.x) / 6.2831853, acos(clamp(local.y, -1.0, 1.0)) / 3.14159265);
            shadow -= textureSample(cloud_tex, cloud_smp, advect(cloud_uv, params.cloud_flow, params.misc.z)).r * params.natural.z;
        }
        let ao = select(1.0, material.g, params.natural.x > 1.5);
        colour = albedo * (ambient_floor * ao + nl * day * shadow * params.misc.w);
        if (params.natural.x > 1.5) {
            // Dielectric GGX: frost roughens the lobe, exposed ice stays smooth.
            let rough = clamp(material.r, 0.18, 1.0);
            let nh = max(dot(n, half_dir), 0.0);
            let nv = max(dot(n, view_dir), 0.001);
            let vh = max(dot(view_dir, half_dir), 0.0);
            let a2 = pow(rough, 4.0);
            let distribution = a2 / max(3.14159265 * pow(nh * nh * (a2 - 1.0) + 1.0, 2.0), 0.0001);
            let k = pow(rough + 1.0, 2.0) / 8.0;
            let geometry = nv / (nv * (1.0 - k) + k) * nl / (nl * (1.0 - k) + k);
            let fresnel = 0.018 + 0.982 * pow(1.0 - vh, 5.0);
            colour += vec3<f32>(distribution * geometry * fresnel / max(4.0 * nv, 0.001)) * day * shadow * params.misc.w;
            // Thin-ice mask controls a local wrap-light approximation, not a
            // transparent globe or a full subsurface transport simulation.
            let wrap = max((geometric_light + 0.22) / 1.22, 0.0);
            let scatter = wrap * (1.0 - smoothstep(0.05, 0.65, geometric_light));
            colour += params.scatter_colour.rgb * scatter * material.b * params.effects.w * params.misc.w;
        }
        let masks = textureSample(emask_tex, emask_smp, uv).rgb;
        let emission = textureSample(emissive_tex, emissive_smp, uv).rgb;
        // Events stay pinned to authored masks, with independent regional phases.
        let longitude = uv.x * 6.2831853;
        let seed = 0.5 + 0.25 * sin(longitude * 7.0 + uv.y * 17.0)
            + 0.25 * cos(longitude * 11.0 - uv.y * 23.0);
        let pulse = pow(max(sin(params.misc.z * params.effects.z + seed * 6.2831853), 0.0), 24.0);
        colour += emission * masks.r * params.effects.x * night;
        colour += params.event_colour.rgb * (masks.g * pulse * params.effects.y + masks.b * 0.12) * night;
    }

    // Roughness-modulated specular glint (oceans, ice). Subtle by design.
    if (params.flags.y > 0.5 && params.city.x < 0.5 && params.natural.x < 0.5) {
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
    if (params.flags.z > 0.5 && params.natural.x < 0.5) {
        var night_gate = 1.0;
        if (params.flags.w > 0.5) {
            night_gate = night;
        }
        var mask = 1.0;
        if (params.misc.x > 0.5) {
            mask = textureSample(emask_tex, emask_smp, uv).r;
        }
        let emissive = textureSample(emissive_tex, emissive_smp, uv).rgb;
        if (params.city.x > 0.5) {
            let channels = textureSample(emask_tex, emask_smp, uv);
            // Modulate intensity along authored routes, never scroll the whole
            // light map across buildings. Fine channels are already mipmapped.
            let traffic_phase = uv.x * 804.2477 + uv.y * 402.1239;
            let traffic_visibility = 1.0 - smoothstep(0.5, 3.0, fwidth(traffic_phase));
            let movement = 0.7 + 0.3 * traffic_visibility * sin(traffic_phase - params.city.z * 6.2831853);
            let city = emissive * channels.r * params.city_lights.x
                + params.neon_colour.rgb * channels.g * params.city_lights.y
                + params.traffic_colour.rgb * channels.a * params.city_lights.w * movement;
            let heat = params.thermal_colour.rgb * channels.b * params.city_lights.z;
            // Material alpha selects daytime activity independently of the
            // original night-light maps. Active patches retain 60% intensity.
            let city_gate = mix(day_activity * 0.6, 1.0, night_gate);
            colour += (city * city_gate + heat) * params.emissive_strength;
        } else {
            colour += emissive * mask * params.emissive_strength * night_gate;
        }
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

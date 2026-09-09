// Moving smog or integrated atmospheric scattering, with premultiplied output.
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
struct PlanetCloudParams {
    light_dir: vec3<f32>, time: f32,
    misc: vec4<f32>, texture_x: vec4<f32>, texture_y: vec4<f32>, texture_z: vec4<f32>,
    planet_center: vec4<f32>, layer: vec4<f32>, geometry: vec4<f32>,
    rayleigh: vec4<f32>, mie: vec4<f32>, flow: vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PlanetCloudParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var albedo_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var albedo_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var opacity_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var opacity_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var normal_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var normal_smp: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var glow_tex: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var glow_smp: sampler;

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

fn local_normal(n: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(n, params.texture_x.xyz), dot(n, params.texture_y.xyz), dot(n, params.texture_z.xyz));
}
fn spherical_uv(n: vec3<f32>) -> vec2<f32> {
    let q = local_normal(n);
    return vec2<f32>(atan2(q.z, q.x) / 6.2831853, acos(clamp(q.y, -1.0, 1.0)) / 3.14159265);
}
fn sphere_hit(p: vec3<f32>, d: vec3<f32>, radius: f32) -> vec2<f32> {
    let b = dot(p, d);
    let disc = b * b - dot(p, p) + radius * radius;
    if (disc < 0.0) { return vec2<f32>(1e8, -1e8); }
    let h = sqrt(disc);
    return vec2<f32>(-b - h, -b + h);
}
fn atmosphere(in: VertexOutput) -> vec4<f32> {
    // Unit planet coordinates avoid cancellation in large world coordinates.
    let outer = params.geometry.y / params.geometry.x;
    let thickness = outer - 1.0;
    let camera = (view.world_position - params.planet_center.xyz) / params.geometry.x;
    let ray = normalize(in.world_position.xyz - view.world_position);
    let interval = sphere_hit(camera, ray, outer);
    let start = max(interval.x, 0.0);
    var finish = interval.y;
    let ground = sphere_hit(camera, ray, 1.0);
    if (ground.x > 0.0 && ground.x < finish) { finish = ground.x; }
    if (finish <= start) { return vec4<f32>(0.0); }
    let step = (finish - start) / 12.0;
    let light = normalize(params.light_dir);
    let cosine = dot(ray, light);
    let g = params.geometry.z;
    let rayleigh_phase = 0.75 * (1.0 + cosine * cosine);
    let mie_phase = (1.0 - g * g) / max(pow(1.0 + g * g - 2.0 * g * cosine, 1.5), 0.05);
    let middle = normalize(camera + ray * (start + finish) * 0.5);
    let uv = spherical_uv(middle);
    let haze = textureSampleLevel(opacity_tex, opacity_smp, uv, 1.0).r;
    // Regional pollution scales the aerosol column. The sun LUT remains a
    // radial approximation, using this column's pollution on its light path.
    let aerosol = params.mie.rgb * (0.65 + haze * 0.85);
    var depth = vec3<f32>(0.0);
    var scattered = vec3<f32>(0.0);
    for (var s = 0; s < 12; s += 1) {
        let p = camera + ray * (start + (f32(s) + 0.5) * step);
        let r = length(p);
        let altitude = clamp((r - 1.0) / thickness, 0.0, 1.0);
        let density = vec2<f32>(exp(-altitude * 6.0), exp(-altitude * 12.0));
        let extinction = params.rayleigh.rgb * density.x + aerosol * density.y;
        let segment_depth = extinction * step / thickness;
        let sun_hit = sphere_hit(p, light, 1.0);
        var visibility = select(1.0, 0.0, sun_hit.x > 0.0 && sun_hit.y > sun_hit.x);
        var mu = dot(p / r, light);
        if (params.geometry.w > 0.0) {
            // Resolve the sun's finite angular width at the local horizon.
            // Binary occultation makes the twelve integration samples appear
            // as hard arcs when a dense atmosphere turns out of sunlight.
            let horizon = -sqrt(max(1.0 - 1.0 / (r * r), 0.0));
            visibility = smoothstep(-params.geometry.w, params.geometry.w, mu - horizon);
            mu = max(mu, horizon);
        }
        if (visibility > 0.0) {
            let lut_uv = (vec2<f32>(mu * 0.5 + 0.5, altitude) * vec2<f32>(255.0, 127.0) + 0.5) / vec2<f32>(256.0, 128.0);
            let encoded = textureSampleLevel(albedo_tex, albedo_smp, lut_uv, 0.0).rg;
            let optical = -8.0 * log(max(vec2<f32>(1.0) - encoded, vec2<f32>(0.0001)));
            let transmittance = exp(-params.rayleigh.rgb * optical.x - aerosol * optical.y - depth - segment_depth * 0.5);
            let source = params.rayleigh.rgb * density.x * rayleigh_phase + aerosol * density.y * mie_phase;
            scattered += transmittance * source * step / thickness * visibility;
        }
        depth += segment_depth;
    }
    var glow = vec3<f32>(0.0);
    if (params.layer.w > 0.0) {
        glow = textureSampleLevel(glow_tex, glow_smp, uv, 1.0).rgb;
    }
    let night = 1.0 - smoothstep(-0.2, 0.1, dot(middle, light));
    // Scalar shell coverage approximates RGB extinction; scattered light
    // retains colour-dependent transmission along both view and sun paths.
    let alpha = 1.0 - exp(-dot(depth, vec3<f32>(0.2126, 0.7152, 0.0722)));
    return vec4<f32>(scattered * params.misc.w + glow * haze * night * alpha * params.layer.w, alpha);
}
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let inside = distance(view.world_position, params.planet_center.xyz) < params.geometry.y;
    if (front == inside) { discard; }
    if (params.layer.x > 0.5) { return atmosphere(in); }
    let n_geo = normalize(in.world_position.xyz - params.planet_center.xyz);
    let light = normalize(params.light_dir);
    var surface_uv = in.uv;
    if (params.layer.w > 0.0 && abs(local_normal(n_geo).y) > 0.9) {
        let polar_uv = spherical_uv(n_geo);
        surface_uv.x += fract(polar_uv.x - surface_uv.x + 0.5) - 0.5;
        surface_uv.y = polar_uv.y;
    }
    let uv = advect(surface_uv, params.flow, params.time);
    let albedo = textureSample(albedo_tex, albedo_smp, uv).rgb;
    var alpha = dot(albedo, vec3<f32>(0.299, 0.587, 0.114));
    if (params.misc.y > 0.5) { alpha = textureSample(opacity_tex, opacity_smp, uv).r; }
    alpha = clamp(alpha * params.layer.z, 0.0, 1.0);
    var n = n_geo;
    if (params.geometry.w > 0.5) {
        let q = local_normal(n_geo);
        let t = normalize(params.texture_x.xyz * -q.z + params.texture_z.xyz * q.x + params.texture_x.xyz * 0.00001);
        let b = cross(n_geo, t);
        let map = textureSample(normal_tex, normal_smp, uv).xyz * 2.0 - 1.0;
        n = normalize(n_geo * map.z + (t * map.x + b * map.y) * params.layer.y);
    }
    let ndotl = dot(n, light);
    let day = smoothstep(-0.05, 0.15, dot(n_geo, light));
    let lit = params.misc.z + day * max(ndotl, 0.0) * params.misc.w;
    var colour = albedo * lit;
    if (params.layer.w > 0.0) {
        let glow = textureSample(glow_tex, glow_smp, surface_uv).rgb;
        let night = 1.0 - smoothstep(-0.15, 0.1, dot(n_geo, light));
        colour += glow * night * params.layer.w;
    }
    return vec4<f32>(colour * alpha, alpha);
}

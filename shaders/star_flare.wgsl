#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
struct Settings { source: vec4<f32>, shape: vec4<f32> }
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: Settings;
#ifdef MULTISAMPLED_DEPTH
@group(0) @binding(3) var scene_depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(3) var scene_depth: texture_depth_2d;
#endif

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let original = textureSample(scene, scene_sampler, in.uv);
    let dimensions = vec2<f32>(textureDimensions(scene_depth));
    var visible = 0.0;
    // Disc sampling uses opaque scene depth, including GLB geometry. No GPU readback.
    for (var i = 0u; i < 24u; i += 1u) {
        let theta = f32(i) * 2.399963;
        let radius = sqrt((f32(i) + 0.5) / 24.0) * 0.88;
        let uv = settings.source.xy + vec2(cos(theta), sin(theta)) * radius * settings.shape.xy;
        if all(uv >= vec2(0.0)) && all(uv < vec2(1.0)) {
            let depth = textureLoad(scene_depth, vec2<i32>(uv * dimensions), 0);
            if depth <= settings.source.z + 0.00001 { visible += 1.0 / 24.0; }
        }
    }
    if visible <= 0.0 { return original; }
    let axis = settings.source.xy - vec2(0.5);
    let aspect = vec2(settings.shape.z, 1.0);
    let delta = (in.uv - settings.source.xy) * aspect;
    let halo = exp(-length(delta) * 27.0) * 0.35;
    let streak = exp(-abs(delta.y) * 850.0 - abs(delta.x) * 8.0) * 0.2;
    let ghost1 = exp(-pow(length((in.uv - (vec2(0.5) - axis * 0.55)) * aspect) / 0.023, 2.0));
    let ghost2 = exp(-pow(length((in.uv - (vec2(0.5) - axis * 1.2)) * aspect) / 0.042, 2.0));
    let light = vec3(1.0, 0.76, 0.43) * (halo + streak) + vec3(0.3, 0.55, 0.6) * ghost1 * 0.08 + vec3(0.55, 0.3, 0.18) * ghost2 * 0.06;
    return vec4(original.rgb + light * visible * settings.source.w, original.a);
}

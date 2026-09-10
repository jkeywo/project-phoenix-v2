#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
struct Settings { source: vec4<f32>, shape: vec4<f32> }
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: Settings;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let original = textureSample(scene, scene_sampler, in.uv);
    let visible = settings.shape.w;
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

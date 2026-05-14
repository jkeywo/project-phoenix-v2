// Red Alert vignette — inset radial gradient.
//
// Driven by the `RedAlertVignetteMaterial` UiMaterial (PRD #180, slice
// #183). The single `intensity` uniform fades the whole effect from
// invisible (0.0) to fully bright (1.0); the per-frame ramp + sine
// pulse is computed in Rust by `viewscreen_border::pulse_intensity`.
//
// The visual matches the original CSS box-shadow + radial-gradient
// recipe in `server.html`:
//
// - A narrow inner-edge "core" of intense red just inside the screen
//   border (mimicking `box-shadow: inset 0 0 60px 10px rgba(255,30,30,0.7)`).
// - A wider outer falloff that softens toward the centre and dies
//   completely well before the middle of the viewport (mimicking the
//   two-stop radial-gradient from 65% → 85% → 100%).
//
// Geometry uses the UiVertexOutput.uv (range 0..1 across the node), so
// the vignette is independent of node aspect ratio. We compute a radial
// distance from the centre normalised to the corner, then build two
// inset-from-the-edge falloffs by working with `1 - r`.

#import bevy_ui::ui_vertex_output::UiVertexOutput

struct RedAlertVignetteMaterial {
    intensity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0)
var<uniform> material: RedAlertVignetteMaterial;

// Anti-aliased step from `edge0` to `edge1`. `value` outside the range
// is clamped to 0/1; inside, a smoothstep interpolates.
fn smooth_band(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = clamp((value - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // Centre-relative coordinates in [-1, 1]. Treating x and y on the
    // same axis (no aspect divide) gives an elliptical glow that hugs
    // the screen edges, matching the original `radial-gradient(ellipse...)`.
    let p = in.uv * 2.0 - vec2<f32>(1.0, 1.0);
    let r = length(p);

    // Inset distance from the nearest screen edge: 0 at the very edge,
    // grows toward the centre. We work with this so both falloffs are
    // edge-anchored regardless of viewport size.
    let inset = 1.0 - clamp(r, 0.0, 1.0);

    // Outer falloff — soft red glow leaking inward roughly the inner
    // 35% of the radial range (matches the 65%→100% gradient stops).
    let outer = (1.0 - smooth_band(0.0, 0.35, inset)) * 0.55;

    // Inner core — narrow, brighter ring just inside the edge (matches
    // the inset box-shadow's tight 60px spread). Lives in the inner ~8%.
    let core = (1.0 - smooth_band(0.0, 0.08, inset)) * 0.85;

    // Combine and apply the master intensity uniform. Cap the alpha at
    // 1.0 so a steady-on glow doesn't over-saturate at the corners.
    let alpha = clamp(max(outer, core) * material.intensity, 0.0, 1.0);

    // Slight orange-shifted red — same hue family as the CSS overlay
    // (`rgba(255, 30, 30, ...)` and `rgba(255, 0, 0, ...)`).
    let colour = vec3<f32>(1.0, 0.12, 0.12);
    return vec4<f32>(colour * alpha, alpha);
}

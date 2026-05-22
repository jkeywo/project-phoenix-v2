// Radar arc — renders a circular sector (wedge) on the radar widget,
// tinted by `color_*` and bounded by `half_arc_rad` around `facing_rad`.
//
// The fragment shader runs over the full radar quad. For each pixel we
// compute its position in normalised radar-space [-1, 1] and:
//   - discard pixels outside the unit circle (radar boundary);
//   - discard pixels whose bearing from origin is outside the wedge.
//
// Bearing convention: 0 rad = +Y (forward / "up" on the radar), increasing
// clockwise (matches RadarCenter yaw + ship-relative orientation).

#import bevy_ui::ui_vertex_output::UiVertexOutput

struct RadarArcMaterial {
    color_r: f32,
    color_g: f32,
    color_b: f32,
    color_a: f32,
    facing_rad: f32,
    half_arc_rad: f32,
    _pad0: f32,
    _pad1: f32,
};

@group(1) @binding(0) var<uniform> material: RadarArcMaterial;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // Convert UV (0..1, Y-down) to radar space (-1..1, Y-up).
    let rx = (in.uv.x - 0.5) * 2.0;
    let ry = -(in.uv.y - 0.5) * 2.0;
    let r2 = rx * rx + ry * ry;
    if r2 > 1.0 {
        discard;
    }
    // Bearing from +Y axis, clockwise.
    let bearing = atan2(rx, ry);
    // Shortest signed angle from facing to bearing in [-PI, PI].
    var delta = bearing - material.facing_rad;
    let TAU = 6.28318530717958647692;
    delta = delta - TAU * floor((delta + 3.14159265358979323846) / TAU);
    if abs(delta) > material.half_arc_rad {
        discard;
    }
    return vec4<f32>(material.color_r, material.color_g, material.color_b, material.color_a);
}

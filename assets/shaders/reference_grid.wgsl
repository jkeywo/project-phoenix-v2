// Reference grid — a faint, world-locked lattice in the y = 0 plane, drawn on
// one quad that follows the local ship.
//
// The whole point of this shader is that the lines are computed from the
// fragment's WORLD position, not from the quad's UVs. The quad is a moving
// window; the lattice underneath it does not move, so the ship visibly slides
// over the lines and the crew get a motion cue that empty space cannot give
// them. Anything here that read `in.uv` for the line maths would silently turn
// the grid into a ship-locked decal and lose the entire effect.
//
// The maths below is mirrored — deliberately, expression for expression — by
// `distance_to_nearest_line`, `line_coverage` and `radial_fade` in
// `src/reference_grid.rs`, which is where it is unit-tested. The GPU cannot
// call into the crate, so a rule written only here is a rule nothing in CI
// evaluates. Change one, change the other.
//
// Line widths are in PIXELS rather than world units. Dividing the world-space
// distance by `fwidth()` before comparing it to the width is what antialiases
// the grid for free: a line seen nearly edge-on covers a fraction of a pixel
// and dims, where a fixed world-space width would alias into a moiré pattern
// at exactly the grazing angles the viewscreen spends most of its time at.
//
// Faintness is carried in ALPHA, not in brightness. The camera is HDR with
// bloom thresholded at 1.0, so an RGB triple pushed for visibility would start
// to glow; see the calibration note on `ReferenceGridConfig`.

#import bevy_pbr::forward_io::VertexOutput

struct ReferenceGridMaterial {
    minor_r: f32,
    minor_g: f32,
    minor_b: f32,
    minor_a: f32,
    major_r: f32,
    major_g: f32,
    major_b: f32,
    major_a: f32,
    minor_spacing: f32,
    major_spacing: f32,
    minor_half_width_px: f32,
    major_half_width_px: f32,
    opacity: f32,
    patch_radius: f32,
    fade_start: f32,
    fade_span: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> material: ReferenceGridMaterial;

// Distance in world units from `coord` to the nearest line of the lattice.
// Lines sit at every integer multiple of `spacing`, including zero — that is
// the world-locked claim — so the result is in [0, spacing / 2].
fn distance_to_nearest_line(coord: f32, spacing: f32) -> f32 {
    let cells = coord / spacing;
    return abs(cells - round(cells)) * spacing;
}

// Coverage of a line whose centre is `world_distance` world units away, drawn
// `half_width_px` pixels wide either side, where one pixel spans
// `world_per_px` world units at this fragment. The parameter is not called
// `distance` because that is a WGSL builtin, and shadowing it here would be a
// trap for the next person to add a call to it.
fn line_coverage(world_distance: f32, half_width_px: f32, world_per_px: f32) -> f32 {
    if (world_per_px <= 0.0 || half_width_px <= 0.0) {
        return 0.0;
    }
    let distance_px = world_distance / world_per_px;
    return clamp(1.0 - distance_px / half_width_px, 0.0, 1.0);
}

// Coverage of the whole lattice at this fragment: the stronger of the X-facing
// and Z-facing families, so a crossing reads as one intersection rather than as
// a double-blended bright spot.
fn lattice_coverage(
    world_xz: vec2<f32>,
    derivative: vec2<f32>,
    spacing: f32,
    half_width_px: f32,
) -> f32 {
    let along_x = line_coverage(
        distance_to_nearest_line(world_xz.x, spacing),
        half_width_px,
        derivative.x,
    );
    let along_z = line_coverage(
        distance_to_nearest_line(world_xz.y, spacing),
        half_width_px,
        derivative.y,
    );
    return max(along_x, along_z);
}

// Radial fade, full strength within `fade_start` and gone by the patch edge.
// `fade_span` is floored above zero CPU-side, so this divides unconditionally
// and an authored fade band of zero reads as a hard edge.
fn radial_fade(world_distance: f32, fade_start: f32, fade_span: f32) -> f32 {
    let t = clamp((world_distance - fade_start) / fade_span, 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let world_xz = in.world_position.xz;

    // World units covered by one pixel, per axis. Taken ONCE, outside the
    // branchless coverage calls below, because a derivative is only defined
    // where the whole quad of fragments agrees to evaluate it.
    let derivative = fwidth(world_xz);

    let minor_cov = lattice_coverage(
        world_xz,
        derivative,
        material.minor_spacing,
        material.minor_half_width_px,
    );
    let major_cov = lattice_coverage(
        world_xz,
        derivative,
        material.major_spacing,
        material.major_half_width_px,
    );

    // The patch is centred on the ship, so the quad's own UV centre is the
    // fade centre — no ship position needs to reach the shader for this.
    let from_centre = length(in.uv * 2.0 - vec2<f32>(1.0)) * material.patch_radius;
    let fade = radial_fade(from_centre, material.fade_start, material.fade_span);

    let minor_alpha = minor_cov * material.minor_a;
    let major_alpha = major_cov * material.major_a;
    let alpha = max(minor_alpha, major_alpha) * material.opacity * fade;
    if (alpha <= 0.001) {
        discard;
    }

    // Major lines are the same hue, brighter — one visual language, two
    // weights. Blending on major coverage rather than switching on it keeps
    // the transition antialiased along with everything else.
    let minor_rgb = vec3<f32>(material.minor_r, material.minor_g, material.minor_b);
    let major_rgb = vec3<f32>(material.major_r, material.major_g, material.major_b);
    let rgb = mix(minor_rgb, major_rgb, major_cov);

    return vec4<f32>(rgb, alpha);
}

// Radar blip icon — samples the icon texture and optionally clips pixels to the
// inscribed circle of the radar widget.
//
// When `clip_circle` is non-zero, the pixel's position within the blip icon is
// converted back to normalised radar-space coordinates using the stored blip
// centre `(radar_nx, radar_ny)` and `size_frac` (half-blip-size / radar-radius).
// Any pixel whose radar-space distance from the origin exceeds 1.0 is discarded,
// cleanly clipping the icon at the circular radar boundary without needing a
// background colour or opaque overlay.

#import bevy_ui::ui_vertex_output::UiVertexOutput

struct RadarBlipMaterial {
    color_r: f32,
    color_g: f32,
    color_b: f32,
    color_a: f32,
    radar_nx: f32,
    radar_ny: f32,
    size_frac: f32,
    clip_circle: f32,
    highlighted: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(1) @binding(0) var icon_texture: texture_2d<f32>;
@group(1) @binding(1) var icon_sampler: sampler;
@group(1) @binding(2) var<uniform> material: RadarBlipMaterial;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    if material.clip_circle > 0.5 {
        // Convert this pixel's UV position to radar-space coords [-1, 1].
        // The blip node's bounding box spans from (nx - size_frac) to (nx + size_frac)
        // in radar X, and (ny - size_frac) to (ny + size_frac) in radar Y (Y-flipped).
        let rx = material.radar_nx + (in.uv.x - 0.5) * 2.0 * material.size_frac;
        let ry = material.radar_ny - (in.uv.y - 0.5) * 2.0 * material.size_frac;
        if rx * rx + ry * ry > 1.0 {
            discard;
        }
    }
    let tint = vec4<f32>(material.color_r, material.color_g, material.color_b, material.color_a);
    let icon_sample = textureSample(icon_texture, icon_sampler, in.uv) * tint;

    // When highlighted, draw a red ring around the blip.
    // The ring is a circular outline at a fixed fraction of the node UV space,
    // drawn on top of the icon so it's always visible regardless of icon colour.
    if material.highlighted > 0.5 {
        let uv_center = in.uv - vec2(0.5, 0.5);
        let dist = length(uv_center);
        let ring_radius: f32 = 0.42;
        let ring_half_width: f32 = 0.035;
        if abs(dist - ring_radius) < ring_half_width {
            let frac = 1.0 - abs(dist - ring_radius) / ring_half_width;
            return vec4<f32>(1.0, 0.2, 0.2, frac * 0.85);
        }
    }

    return icon_sample;
}

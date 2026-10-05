use super::*;

#[test]
fn the_payload_reports_the_level_it_is_showing() {
    let stats = SubjectStats {
        triangles: 138_790,
        meshes: 12,
        textures: 3,
        measured_textures: 3,
        texture_pixels: 786_432,
        largest_texture: 512,
    };
    let ladder = LadderState {
        distance: 87.5,
        current: Some(1),
        levels: vec![Default::default(), Default::default()],
        ..Default::default()
    };
    let json = render_stats(&stats, &ladder, LodMode::Auto, true, 8.0, None);
    assert!(json.contains(r#""triangles":138790"#), "{json}");
    assert!(json.contains(r#""distance":87.50"#), "{json}");
    assert!(json.contains(r#""mode":"auto""#), "{json}");
    assert!(json.contains(r#""level":1"#), "{json}");
    assert!(json.contains(r#""levels":2"#), "{json}");
    assert!(json.contains(r#""settled":true"#), "{json}");
    assert!(json.contains(r#""extent":8.00"#), "{json}");
}

/// The base model is "no level", and JSON has a word for that.
#[test]
fn no_level_is_null_rather_than_a_number() {
    let json = render_stats(
        &SubjectStats::default(),
        &LadderState::default(),
        LodMode::Base,
        false,
        0.0,
        None,
    );
    assert!(json.contains(r#""level":null"#), "{json}");
}

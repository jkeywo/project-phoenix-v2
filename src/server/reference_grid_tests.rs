use super::*;

// ── Gating ────────────────────────────────────────────────────────────

#[test]
fn an_authored_hull_with_a_local_ship_spawns_a_patch() {
    let action = decide_patch_action(Some(Vec2::new(12.0, -30.0)), true, false);
    assert_eq!(action, PatchAction::Spawn(Vec2::new(12.0, -30.0)));
}

#[test]
fn a_hull_that_authors_no_table_never_spawns_one() {
    assert_eq!(
        decide_patch_action(Some(Vec2::new(12.0, -30.0)), false, false),
        PatchAction::Idle
    );
}

#[test]
fn the_patch_follows_the_ship_once_it_exists() {
    assert_eq!(
        decide_patch_action(Some(Vec2::new(4.0, 5.0)), true, true),
        PatchAction::Follow(Vec2::new(4.0, 5.0))
    );
}

#[test]
fn losing_the_ship_takes_the_patch_with_it() {
    assert_eq!(decide_patch_action(None, true, true), PatchAction::Despawn);
    assert_eq!(decide_patch_action(None, true, false), PatchAction::Idle);
}

#[test]
fn a_table_that_goes_away_takes_an_existing_patch_with_it() {
    assert_eq!(
        decide_patch_action(Some(Vec2::ZERO), false, true),
        PatchAction::Despawn
    );
}

/// The structural half of "NPC hulls never carry one": the gate has no
/// input that could express a non-local ship, so there is no arrangement of
/// NPCs that reaches a second grid. Asserted by exhausting the input space.
#[test]
fn no_input_combination_produces_more_than_one_patch() {
    for authored in [true, false] {
        for patch_exists in [true, false] {
            for ship_xz in [None, Some(Vec2::new(1.0, 2.0))] {
                let action = decide_patch_action(ship_xz, authored, patch_exists);
                if patch_exists {
                    assert_ne!(
                        action,
                        PatchAction::Spawn(Vec2::new(1.0, 2.0)),
                        "a second patch was spawned for \
                             ship={ship_xz:?} authored={authored}"
                    );
                }
            }
        }
    }
}

// ── Uniform ───────────────────────────────────────────────────────────

#[test]
fn the_uniform_carries_the_authored_table() {
    let config = ReferenceGridConfig::default();
    let material = material_from_config(&config);
    assert_eq!(material.minor_spacing, 10.0);
    assert_eq!(material.major_spacing, 50.0);
    assert_eq!(material.minor_a, config.minor_colour[3]);
    assert_eq!(material.major_a, config.major_colour[3]);
    assert_eq!(material.opacity, config.opacity);
    assert_eq!(material.patch_radius, config.patch_radius);
    assert_eq!(material.fade_exponent, config.fade_exponent);
    // The three pads exist only to round the uniform to whole 16-byte rows;
    // they must carry nothing the shader could read as data.
    assert_eq!(
        (material._pad0, material._pad1, material._pad2),
        (0.0, 0.0, 0.0)
    );
}

#[test]
fn the_uniform_halves_the_authored_pixel_widths() {
    // The shader compares against a distance from the line CENTRE, so it
    // wants half widths. Doing it here means the WGSL carries no arithmetic
    // on authored values at all.
    let config = ReferenceGridConfig {
        minor_line_width_px: 3.0,
        major_line_width_px: 5.0,
        ..Default::default()
    };
    let material = material_from_config(&config);
    assert_eq!(material.minor_half_width_px, 1.5);
    assert_eq!(material.major_half_width_px, 2.5);
}

// ── Per-view height ───────────────────────────────────────────────────

#[test]
fn first_person_takes_its_own_height_when_authored() {
    let config = ReferenceGridConfig {
        plane_y: -0.5,
        first_person_plane_y: Some(-6.0),
        ..Default::default()
    };
    let first_person = ViewMode::Camera(crate::core::messages::CameraView::default());
    assert_eq!(plane_y_for_view(&config, Some(&first_person)), -6.0);
    // Every other mode — and no mode at all — keeps the shared floor.
    assert_eq!(plane_y_for_view(&config, Some(&ViewMode::Cinematic)), -0.5);
    assert_eq!(plane_y_for_view(&config, Some(&ViewMode::Radar)), -0.5);
    assert_eq!(plane_y_for_view(&config, None), -0.5);
}

#[test]
fn first_person_falls_back_to_plane_y_when_unauthored() {
    let config = ReferenceGridConfig {
        plane_y: -0.5,
        first_person_plane_y: None,
        ..Default::default()
    };
    let first_person = ViewMode::Camera(crate::core::messages::CameraView::default());
    assert_eq!(plane_y_for_view(&config, Some(&first_person)), -0.5);
}

#[test]
fn the_uniform_fade_span_is_never_zero() {
    // WGSL divides by it unconditionally.
    let config = ReferenceGridConfig {
        fade_band: 0.0,
        ..Default::default()
    };
    assert!(material_from_config(&config).fade_span > 0.0);
}

// ── Shipped content ───────────────────────────────────────────────────

fn shipped_hull(stem: &str) -> crate::entities::config::EntityConfig {
    let path = format!("assets/entities/{stem}.toml");
    crate::entities::include_resolve::load_entity_config(&path)
        .unwrap_or_else(|e| panic!("{stem}.toml must compose and parse: {e}"))
}

#[test]
fn the_player_destroyer_authors_a_grid_that_validates() {
    let config = shipped_hull("alliance_destroyer")
        .reference_grid
        .expect("alliance_destroyer.toml authors [reference_grid]");
    config
        .validate()
        .expect("the shipped table must pass the same validator a load does");
    assert_eq!(config.minor_spacing, 10.0);
    assert_eq!(config.major_spacing, 50.0);
    // [ai] The retuned floor/fade values John signed off on. Pinned so a
    // future edit to the TOML that drops one is caught here.
    assert_eq!(config.plane_y, -0.5);
    assert_eq!(config.fade_band, 250.0);
    assert_eq!(config.fade_exponent, 2.5);
    // The uniform it produces is the one the shader is calibrated against.
    let material = material_from_config(&config);
    assert!(
        material.minor_a <= 0.25 && material.major_a <= 0.25,
        "\"faint\" is carried in alpha; these are what keep it faint"
    );
}

/// The other half of "NPC hulls never carry one", checked against the
/// shipped content rather than against the gate: no hostile hull authors
/// the table, so even a future bug in the gate has nothing to act on.
#[test]
fn no_npc_hull_authors_a_reference_grid() {
    for stem in [
        "ship_harrow_destroyer",
        "ship_harrow_warhawk",
        "ship_requiem_courier",
        "alliance_courier",
    ] {
        assert!(
            shipped_hull(stem).reference_grid.is_none(),
            "{stem}.toml must not author [reference_grid] — the grid is the local \
                 player ship's alone"
        );
    }
}

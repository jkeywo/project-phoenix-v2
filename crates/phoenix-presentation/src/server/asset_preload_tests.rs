use super::*;
use crate::entities::config::*;

#[test]
fn icon_asset_path_capitalizes_first_letter() {
    assert_eq!(
        icon_asset_path("destroyer"),
        "radar_icons/Icon-Destroyer.png"
    );
    assert_eq!(icon_asset_path("Star"), "radar_icons/Icon-Star.png");
    assert_eq!(
        icon_asset_path("playerShip"),
        "radar_icons/Icon-PlayerShip.png"
    );
}

#[test]
fn icon_asset_path_empty_returns_empty_icon_name() {
    // The naming convention doesn't make sense for empty input,
    // but it should not panic.
    let path = icon_asset_path("");
    assert!(path.starts_with("radar_icons/Icon-"));
}

#[test]
fn discover_entity_config_with_model_and_icon() {
    let config = EntityConfig {
        mesh: Some(MeshConfig {
            model: Some("assets/models/test_ship.glb".into()),
            variant: None,
            shape: MeshShape::Sphere,
            colour: vec![1.0, 0.0, 0.0],
            radius: 1.0,
            size: None,
            minor_radius: 0.0,
            emissive: None,
            scale: 1.0,
            rotation: [0.0, 0.0, 0.0],
        }),
        radar_appearance: Some(RadarAppearanceConfig {
            icon: Some("testShip".into()),
            colour: Some(vec![1.0, 0.0, 0.0]),
            size: None,
            region_colour: None,
        }),
        ..Default::default()
    };

    let mut manifest = AssetManifest::default();
    discover_entity_config_assets(&config, &mut manifest);

    assert_eq!(manifest.glb_models, vec!["models/test_ship.glb"]);
    assert!(manifest
        .sidecars
        .iter()
        .any(|s| s.contains("test_ship.model.toml")));
    assert_eq!(manifest.radar_icons, vec!["radar_icons/Icon-TestShip.png"]);
}

/// A world without mote overrides still renders the built-in
/// textures, so they must be discovered even though the world file
/// never names them.
#[test]
fn discover_base_assets_preloads_builtin_dust_textures() {
    let world = WorldConfig {
        dust: Some(crate::world::config::DustPfxConfig {
            enabled: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (manifest, _, _, _, _) = discover_base_assets(&world, &HashMap::new());
    assert!(
        !manifest.pfx_textures.is_empty(),
        "built-in dust layers must contribute textures"
    );
    assert!(
        manifest.pfx_textures.iter().all(|p| p.starts_with("pfx/")),
        "got {:?}",
        manifest.pfx_textures
    );
}

/// The player-ship radar icon is injected onto the selected hull at spawn,
/// not authored in any template, so the template scan never discovers it.
/// `discover_base_assets` must add it unconditionally so clients always
/// have the player blip PNG — even for a world that references no ships.
#[test]
fn discover_base_assets_always_preloads_player_ship_icon() {
    let world = WorldConfig::default();
    let (manifest, _, _, _, _) = discover_base_assets(&world, &HashMap::new());
    let expected = icon_asset_path(PLAYER_SHIP_RADAR_ICON);
    assert!(
        manifest.radar_icons.contains(&expected),
        "player-ship radar icon must always preload; got {:?}",
        manifest.radar_icons
    );
}

#[test]
fn discover_base_assets_skips_dust_textures_when_disabled() {
    let world = WorldConfig {
        dust: Some(crate::world::config::DustPfxConfig {
            enabled: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (manifest, _, _, _, _) = discover_base_assets(&world, &HashMap::new());
    assert!(manifest.pfx_textures.is_empty());
}

#[test]
fn discover_base_assets_preloads_platform_mote_textures() {
    let mut render = crate::world::config::RenderConfig::default();
    render.native.mote_textures[0] = "pfx/native_mote.png".into();
    render.web.mote_textures[0] = "pfx/web_mote.png".into();
    let expected = render.visuals().mote_textures[0].clone();
    let world = WorldConfig {
        render: Some(render),
        ..Default::default()
    };
    let (manifest, _, _, _, _) = discover_base_assets(&world, &HashMap::new());
    assert!(
        manifest.pfx_textures.contains(&expected),
        "got {:?}",
        manifest.pfx_textures
    );
}

#[test]
fn discover_entity_config_with_model_adds_glb_and_sidecar() {
    let config = EntityConfig {
        mesh: Some(MeshConfig {
            model: Some("assets/models/alliance_cruiser.glb".into()),
            variant: None,
            shape: MeshShape::Sphere,
            colour: vec![],
            radius: 1.0,
            size: None,
            minor_radius: 0.0,
            emissive: None,
            scale: 1.0,
            rotation: [0.0, 0.0, 0.0],
        }),
        ..Default::default()
    };
    let mut manifest = AssetManifest::default();
    discover_entity_config_assets(&config, &mut manifest);
    // GLB is always added; local ship rendering is skipped at render time.
    assert!(manifest
        .glb_models
        .iter()
        .any(|s| s.contains("alliance_cruiser.glb")));
    // Sidecar must also be added for ModelMarkers.
    assert!(manifest
        .sidecars
        .iter()
        .any(|s| s.contains("alliance_cruiser.model.toml")));
}

/// A `[planet]` section contributes every declared texture path with the
/// correct sRGB flag, so the preload gate covers planet textures and they
/// load with the same settings the renderer uses.
#[test]
fn discover_entity_config_with_planet_adds_textures() {
    let config = EntityConfig {
        planet: Some(PlanetConfig {
            radius: 20.0,
            longitude_segments: 64,
            latitude_segments: 32,
            surface: PlanetSurfaceConfig {
                natural: None,
                city: None,
                albedo: "assets/planets/earth/albedo.webp".into(),
                normal: Some("assets/planets/earth/normal.webp".into()),
                roughness: None,
                emissive_colour: Some("assets/planets/earth/emissive_colour.webp".into()),
                emissive_mask: None,
                emissive_night_only: true,
                emissive_strength: 1.0,
            },
            clouds: Some(PlanetCloudsConfig {
                dynamics: None,
                smog: None,
                albedo: "assets/planets/earth/cloud_albedo.webp".into(),
                opacity: Some("assets/planets/earth/cloud_opacity.webp".into()),
                normal: None,
                scale: 1.03,
                drift_speed: 0.0,
            }),
            atmosphere: None,
        }),
        ..Default::default()
    };
    let mut manifest = AssetManifest::default();
    discover_entity_config_assets(&config, &mut manifest);
    assert_eq!(
        manifest.planet_textures,
        vec![
            ("assets/planets/earth/albedo.webp".to_string(), true),
            ("assets/planets/earth/normal.webp".to_string(), false),
            (
                "assets/planets/earth/emissive_colour.webp".to_string(),
                true
            ),
            ("assets/planets/earth/cloud_albedo.webp".to_string(), true),
            ("assets/planets/earth/cloud_opacity.webp".to_string(), false),
        ]
    );
}

#[test]
fn discover_entity_config_no_model_no_icon() {
    let config = EntityConfig::default();
    let mut manifest = AssetManifest::default();
    discover_entity_config_assets(&config, &mut manifest);
    assert!(manifest.glb_models.is_empty());
    assert!(manifest.sidecars.is_empty());
    assert!(manifest.radar_icons.is_empty());
}

// ── Sidecar-owned LOD ladders (issue #914) ────────────────────────────

/// The entity walk sees ONE model now. The far levels are behind the
/// sidecar, so claiming to have found them here would be a lie that
/// silently shrinks the loading gate.
#[test]
fn the_entity_walk_discovers_only_the_model_it_names() {
    let config = EntityConfig {
        mesh: Some(MeshConfig {
            model: Some("assets/models/rock.glb".into()),
            variant: Some("large".into()),
            shape: MeshShape::Sphere,
            colour: vec![0.5, 0.5, 0.5],
            radius: 4.0,
            size: None,
            minor_radius: 0.0,
            emissive: None,
            scale: 1.0,
            rotation: [0.0, 0.0, 0.0],
        }),
        ..Default::default()
    };
    let mut manifest = AssetManifest::default();
    discover_entity_config_assets(&config, &mut manifest);
    assert_eq!(manifest.glb_models, vec!["models/rock.glb"]);
    assert_eq!(manifest.sidecars, vec!["assets/models/rock.large.toml"]);
}

/// The second phase: the delivered sidecar contributes the rest of the
/// ladder — every far GLB and the sidecar each of those needs in turn.
#[test]
fn a_delivered_sidecar_contributes_its_whole_ladder() {
    let sidecar = r#"
[base]
scale = [1.0, 1.0, 1.0]

[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod2.glb"

[[lod]]
shape = "sphere"
"#;
    let mut manifest = AssetManifest::default();
    discover_sidecar_lod_assets(
        sidecar,
        "assets/models/rock.large.toml",
        None,
        &mut manifest,
    );

    assert_eq!(
        manifest.glb_models,
        vec!["models/rock.glb", "models/rock_lod2.glb"],
        "the procedural fallback level names no GLB"
    );
    // A level that omits `variant` inherits the sidecar's own — which is
    // the same fallback `update_mesh_lod` applies from `[mesh] variant`.
    assert_eq!(
        manifest.sidecars,
        vec![
            "assets/models/rock.large.toml",
            "assets/models/rock_lod2.large.toml"
        ]
    );
}

/// A tier that declares it ships no sidecar contributes its GLB and NOTHING
/// else. This is where the 404 John saw actually came from: the preloader
/// listed `<hull>_lod1.model.toml` for every ship in the scenario and duly
/// fetched all of them, at page load, before any LOD band had been crossed.
///
/// The GLB must still be listed — the tier is real and has to stream — so a
/// blanket "skip declared-identity tiers" would break the preload instead.
#[test]
fn a_declared_identity_tier_preloads_its_glb_but_asks_for_no_sidecar() {
    let sidecar = r#"
[base]
scale = [1.5, 1.5, 1.5]

[[lod]]
max_distance = 15.0
model = "assets/models/hull.glb"

[[lod]]
max_distance = 100.0
model = "assets/models/hull_lod1.glb"
tier_rig = "identity"

[[lod]]
shape = "sphere"
"#;
    let mut manifest = AssetManifest::default();
    discover_sidecar_lod_assets(
        sidecar,
        "assets/models/hull.model.toml",
        None,
        &mut manifest,
    );

    assert_eq!(
        manifest.glb_models,
        vec!["models/hull.glb", "models/hull_lod1.glb"],
        "both real GLB tiers still stream"
    );
    assert_eq!(
        manifest.sidecars,
        vec!["assets/models/hull.model.toml"],
        "only the PRIMARY sidecar, which exists — nothing may ask for \
             hull_lod1.model.toml, a file the pipeline deliberately never wrote"
    );
}

/// The other half: a `baked` tier's sidecar IS there and must be preloaded,
/// or the renderer stalls waiting for a fetch nobody started.
#[test]
fn a_declared_baked_tier_still_contributes_its_sidecar() {
    let sidecar = r#"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod2.glb"
tier_rig = "baked"

[[lod]]
shape = "sphere"
"#;
    let mut manifest = AssetManifest::default();
    discover_sidecar_lod_assets(
        sidecar,
        "assets/models/rock.large.toml",
        None,
        &mut manifest,
    );
    assert_eq!(
        manifest.sidecars,
        vec![
            "assets/models/rock.large.toml",
            "assets/models/rock_lod2.large.toml"
        ]
    );
}

/// Issue lod-preload-by-distance: a known distance preloads ONLY the
/// level `select_lod` would pick for it, not the whole ladder.
#[test]
fn a_known_distance_preloads_only_the_needed_level() {
    let sidecar = r#"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod1.glb"

[[lod]]
max_distance = 300.0
model = "assets/models/rock_lod2.glb"

[[lod]]
shape = "sphere"
"#;
    // 200 world units falls in the third band (150..300) -> index 2.
    let mut manifest = AssetManifest::default();
    discover_sidecar_lod_assets(
        sidecar,
        "assets/models/rock.large.toml",
        Some(200.0),
        &mut manifest,
    );
    assert_eq!(
        manifest.glb_models,
        vec!["models/rock_lod2.glb"],
        "only the level covering 200 units must preload"
    );
    assert_eq!(
        manifest.sidecars,
        vec!["assets/models/rock_lod2.large.toml"]
    );
}

/// The nearest band (distance 0) selects level 0 — the entity's own named
/// model, not a decimated step.
#[test]
fn a_known_distance_at_the_near_band_preloads_the_base_level() {
    let sidecar = r#"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod1.glb"

[[lod]]
shape = "sphere"
"#;
    let mut manifest = AssetManifest::default();
    discover_sidecar_lod_assets(
        sidecar,
        "assets/models/rock.large.toml",
        Some(5.0),
        &mut manifest,
    );
    assert_eq!(manifest.glb_models, vec!["models/rock.glb"]);
}

/// The final, unbounded level (usually the procedural-sphere fallback)
/// names no GLB — a distance that lands there must not error, just yield
/// an empty manifest.
#[test]
fn a_known_distance_past_every_glb_level_yields_no_glb() {
    let sidecar = r#"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
shape = "sphere"
"#;
    let mut manifest = AssetManifest::default();
    discover_sidecar_lod_assets(
        sidecar,
        "assets/models/rock.large.toml",
        Some(10_000.0),
        &mut manifest,
    );
    assert!(manifest.glb_models.is_empty());
    assert!(manifest.sidecars.is_empty());
}

#[test]
fn a_level_may_override_the_variant_it_inherits() {
    let sidecar = "[[lod]]\nmodel = \"assets/models/rock_lod1.glb\"\nvariant = \"weathered\"\n";
    let mut manifest = AssetManifest::default();
    discover_sidecar_lod_assets(
        sidecar,
        "assets/models/rock.large.toml",
        None,
        &mut manifest,
    );
    assert_eq!(
        manifest.sidecars,
        vec!["assets/models/rock_lod1.weathered.toml"]
    );
}

/// Absent (404 → empty push) and malformed sidecars contribute nothing and
/// must not panic: the entity still renders its flat `[mesh]`.
#[test]
fn an_absent_or_malformed_sidecar_contributes_no_ladder() {
    for body in ["", "   \n", "[[lod]\nbroken", "lods = 3\n"] {
        let mut manifest = AssetManifest::default();
        discover_sidecar_lod_assets(body, "assets/models/rock.large.toml", None, &mut manifest);
        assert!(manifest.glb_models.is_empty(), "body: {body:?}");
        assert!(manifest.sidecars.is_empty(), "body: {body:?}");
    }
}

/// A ship hull's sidecar declares no ladder at all — the common case, and
/// it must stay a no-op rather than registering phantom assets.
#[test]
fn a_sidecar_with_markers_but_no_ladder_contributes_nothing() {
    let sidecar = "[markers.fore]\nposition = [0.0, 0.0, -1.0]\ndirection = [0.0, 0.0, -1.0]\n";
    let mut manifest = AssetManifest::default();
    discover_sidecar_lod_assets(
        sidecar,
        "assets/models/ship.model.toml",
        None,
        &mut manifest,
    );
    assert!(manifest.glb_models.is_empty());
    assert!(manifest.sidecars.is_empty());
}

// ── resolve_player_start (issue lod-preload-by-distance) ─────────────────

#[test]
fn resolve_player_start_prefers_explicit_position() {
    let world = WorldConfig {
        player_spawn: Some(crate::world::config::PlayerSpawnEntry {
            anchor: None,
            position: Some([10.0, 0.0, 20.0]),
            rotation: None,
        }),
        ..Default::default()
    };
    assert_eq!(
        resolve_player_start(&world, &HashMap::new()),
        [10.0, 0.0, 20.0]
    );
}

#[test]
fn resolve_player_start_falls_back_to_player_spawn_anchor() {
    let mut anchors = HashMap::new();
    anchors.insert("dock".to_string(), [5.0, 0.0, 5.0]);
    let world = WorldConfig {
        anchors,
        player_spawn: Some(crate::world::config::PlayerSpawnEntry {
            anchor: Some("dock".to_string()),
            position: None,
            rotation: None,
        }),
        ..Default::default()
    };
    assert_eq!(
        resolve_player_start(&world, &HashMap::new()),
        [5.0, 0.0, 5.0]
    );
}

/// No `[player_spawn]` at all: falls back to wherever the player ship's
/// own `[[entity]]` instance resolves to — same precedence
/// `spawn_game_start_entities` applies when it actually places the ship.
#[test]
fn resolve_player_start_falls_back_to_the_player_ship_entity() {
    let world = WorldConfig {
        entities: vec![crate::world::config::WorldEntity {
            template_path: "assets/entities/alliance_cruiser.toml".to_string(),
            spawn_on: crate::world::config::WorldEntitySpawnOn::GameStart,
            transform: Some(crate::world::config::TransformConfig {
                position: Some([30.0, 0.0, 40.0]),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut config_cache = HashMap::new();
    config_cache.insert(
        "assets/entities/alliance_cruiser.toml".to_string(),
        EntityConfig {
            tags: vec!["ship".to_string()],
            ..Default::default()
        },
    );
    assert_eq!(
        resolve_player_start(&world, &config_cache),
        [30.0, 0.0, 40.0]
    );
}

#[test]
fn resolve_player_start_defaults_to_origin_when_nothing_resolves() {
    let world = WorldConfig::default();
    assert_eq!(
        resolve_player_start(&world, &HashMap::new()),
        [0.0, 0.0, 0.0]
    );
}

// ── distance-based LOD preload: discover_world_assets/discover_base_assets

/// A placed `[[entity]]` instance's distance from the player start is
/// tracked per the sidecar its model resolves to (issue
/// lod-preload-by-distance) — `discover_sidecar_lod_assets` reads it back
/// once that sidecar's TOML is delivered.
#[test]
fn discover_base_assets_tracks_the_closest_instance_distance_per_sidecar() {
    let world = WorldConfig {
        player_spawn: Some(crate::world::config::PlayerSpawnEntry {
            anchor: None,
            position: Some([0.0, 0.0, 0.0]),
            rotation: None,
        }),
        entities: vec![crate::world::config::WorldEntity {
            template_path: "assets/entities/outpost.toml".to_string(),
            transform: Some(crate::world::config::TransformConfig {
                position: Some([30.0, 0.0, 40.0]),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut config_cache = HashMap::new();
    config_cache.insert(
        "assets/entities/outpost.toml".to_string(),
        EntityConfig {
            mesh: Some(crate::entities::config::MeshConfig {
                model: Some("assets/models/outpost.glb".into()),
                variant: None,
                shape: crate::entities::config::MeshShape::Sphere,
                colour: vec![],
                radius: 1.0,
                size: None,
                minor_radius: 0.0,
                emissive: None,
                scale: 1.0,
                rotation: [0.0, 0.0, 0.0],
            }),
            ..Default::default()
        },
    );

    let (_manifest, _pending, _seen, sidecar_distance, player_start) =
        discover_base_assets(&world, &config_cache);

    assert_eq!(player_start, [0.0, 0.0, 0.0]);
    // 30-40-0 from the origin is a 3-4-5 triangle scaled by 10 -> 50.
    let sc = sidecar_path("assets/models/outpost.glb", None);
    assert_eq!(sidecar_distance.get(&sc).copied(), Some(50.0));
}

/// Two instances of the same template at different distances: the
/// tracked distance is the CLOSEST one — that is the instance whose LOD
/// actually needs the detail preloaded.
#[test]
fn discover_base_assets_keeps_the_minimum_distance_across_instances() {
    fn entity_at(pos: [f32; 3]) -> crate::world::config::WorldEntity {
        crate::world::config::WorldEntity {
            template_path: "assets/entities/outpost.toml".to_string(),
            transform: Some(crate::world::config::TransformConfig {
                position: Some(pos),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    let world = WorldConfig {
        player_spawn: Some(crate::world::config::PlayerSpawnEntry {
            anchor: None,
            position: Some([0.0, 0.0, 0.0]),
            rotation: None,
        }),
        entities: vec![entity_at([100.0, 0.0, 0.0]), entity_at([10.0, 0.0, 0.0])],
        ..Default::default()
    };
    let mut config_cache = HashMap::new();
    config_cache.insert(
        "assets/entities/outpost.toml".to_string(),
        EntityConfig {
            mesh: Some(crate::entities::config::MeshConfig {
                model: Some("assets/models/outpost.glb".into()),
                variant: None,
                shape: crate::entities::config::MeshShape::Sphere,
                colour: vec![],
                radius: 1.0,
                size: None,
                minor_radius: 0.0,
                emissive: None,
                scale: 1.0,
                rotation: [0.0, 0.0, 0.0],
            }),
            ..Default::default()
        },
    );

    let (_manifest, _pending, _seen, sidecar_distance, _player_start) =
        discover_base_assets(&world, &config_cache);

    let sc = sidecar_path("assets/models/outpost.glb", None);
    assert_eq!(sidecar_distance.get(&sc).copied(), Some(10.0));
}

/// `extra_worlds` entries are de-duplicated before they reach the preload
/// work list.
///
/// The duplicate used to come from two `[[trigger]]` blocks naming the same
/// `load_world` path; issue #985 deleted that walk with the front-end that
/// fed it, and a scripted `load_world` is not discoverable before its handler
/// runs (the layer applier fetches on demand instead). `extra_worlds` is the
/// surviving static source. Durable fetched bodies make duplicate reads
/// safe, but de-duplicating the authored work still prevents parsing and
/// registering every discovered asset twice.
#[test]
fn discover_base_assets_deduplicates_extra_world_paths() {
    use crate::world::config::parse_world;

    let toml = r#"
extra_worlds = [
  "assets/worlds/branch_a.toml",
  "assets/worlds/branch_a.toml",
  "assets/worlds/branch_b.toml",
]

[global]
seed = 1
title = "Test"
"#;

    let world = parse_world(toml).expect("parse must succeed");
    let config_cache = HashMap::new();
    let (_manifest, pending_worlds, _, _, _) = discover_base_assets(&world, &config_cache);

    let branch_a_count = pending_worlds
        .iter()
        .filter(|p| p.as_str() == "assets/worlds/branch_a.toml")
        .count();
    assert_eq!(
        branch_a_count, 1,
        "duplicate sub-world path causes permanent preload hang; got {branch_a_count} copies"
    );
    assert_eq!(pending_worlds.len(), 2, "expected branch_a + branch_b only");
}

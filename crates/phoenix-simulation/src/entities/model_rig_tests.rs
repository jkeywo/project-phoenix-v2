use super::*;

const EPS: f32 = 1e-6;

fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < EPS)
}

#[test]
fn parses_full_sidecar() {
    let toml = r##"
[base]
offset = [1.0, 2.0, 3.0]
rotation = [0.1, 0.2, 0.3]
scale = [2.0, 1.0, 0.5]

[extents]
min = [-4.0, -1.2, -6.0]
max = [4.0, 1.2, 6.0]
size = [8.0, 2.4, 12.0]

[markers.fore_emitter]
position = [0.0, 0.0, -6.0]
direction = [0.0, 0.0, -1.0]

[markers.aft_exhaust]
position = [0.0, 0.0, 6.0]
direction = [0.0, 0.0, 1.0]

[[target_points]]
position = [0.5, -0.1, 0.0]

[[target_points]]
position = [-0.25, -0.1, 0.25]
"##;
    let rig = parse_model_rig(toml).expect("full sidecar must parse");
    assert!(approx(rig.base.offset, [1.0, 2.0, 3.0]));
    assert!(approx(rig.base.rotation, [0.1, 0.2, 0.3]));
    assert!(approx(rig.base.scale, [2.0, 1.0, 0.5]));

    let ext = rig.extents.as_ref().expect("extents present");
    assert!(approx(ext.min, [-4.0, -1.2, -6.0]));
    assert!(approx(ext.max, [4.0, 1.2, 6.0]));
    assert!(approx(ext.size, [8.0, 2.4, 12.0]));

    assert_eq!(rig.markers.len(), 2);
    let fore = rig.marker("fore_emitter").expect("fore marker present");
    assert!(approx(fore.position, [0.0, 0.0, -6.0]));
    assert!(approx(fore.direction, [0.0, 0.0, -1.0]));
    assert_eq!(rig.target_points.len(), 2);
    assert!(approx(rig.target_points[0].position, [0.5, -0.1, 0.0]));
    assert!(approx(rig.target_points[1].position, [-0.25, -0.1, 0.25]));
}

#[test]
fn sparse_sidecar_uses_base_defaults() {
    // Only a partial [base] — missing fields default.
    let toml = r##"
[base]
offset = [5.0, 0.0, 0.0]
"##;
    let rig = parse_model_rig(toml).expect("sparse sidecar must parse");
    assert!(approx(rig.base.offset, [5.0, 0.0, 0.0]));
    // rotation defaults to zeros, scale defaults to ones.
    assert!(approx(rig.base.rotation, [0.0, 0.0, 0.0]));
    assert!(approx(rig.base.scale, [1.0, 1.0, 1.0]));
    assert!(rig.extents.is_none());
    assert!(rig.markers.is_empty());
    assert!(rig.target_points.is_empty());
}

#[test]
fn empty_sidecar_is_identity_rig() {
    let rig = parse_model_rig("").expect("empty sidecar must parse");
    assert_eq!(rig.base, BaseTransform::default());
    assert!(approx(rig.base.offset, [0.0, 0.0, 0.0]));
    assert!(approx(rig.base.scale, [1.0, 1.0, 1.0]));
    assert!(rig.extents.is_none());
    assert!(rig.markers.is_empty());
    assert!(rig.target_points.is_empty());
}

#[test]
fn markers_subtable_form_parses() {
    // The `[markers.<name>]` subtable form (what smol-toml emits) parses
    // into the flat map.
    let toml = r##"
[markers.port_bank]
position = [-4.0, 0.0, -2.0]
direction = [-1.0, 0.0, 0.0]
"##;
    let rig = parse_model_rig(toml).expect("markers subtable must parse");
    assert_eq!(rig.markers.len(), 1);
    let m = rig.marker("port_bank").expect("port_bank present");
    assert!(approx(m.position, [-4.0, 0.0, -2.0]));
    assert!(approx(m.direction, [-1.0, 0.0, 0.0]));
}

#[test]
fn sidecar_path_default_variant() {
    assert_eq!(
        sidecar_path("assets/models/dynasty_destroyer.glb", None),
        "assets/models/dynasty_destroyer.model.toml"
    );
    // `Some("model")` is treated as the reserved default name.
    assert_eq!(
        sidecar_path("assets/models/dynasty_destroyer.glb", Some("model")),
        "assets/models/dynasty_destroyer.model.toml"
    );
}

#[test]
fn sidecar_path_named_variant() {
    assert_eq!(
        sidecar_path("assets/models/dynasty_destroyer.glb", Some("weathered")),
        "assets/models/dynasty_destroyer.weathered.toml"
    );
}

#[test]
fn sidecar_path_handles_uppercase_extension() {
    assert_eq!(
        sidecar_path("assets/models/Ship.GLB", None),
        "assets/models/Ship.model.toml"
    );
}

#[test]
fn marker_resolve_hit_and_miss() {
    let toml = r##"
[markers.fore_emitter]
position = [0.0, 0.0, -6.0]
direction = [0.0, 0.0, -1.0]
"##;
    let rig = parse_model_rig(toml).unwrap();
    assert!(rig.marker("fore_emitter").is_some());
    assert!(rig.marker("nope").is_none());

    // ModelMarkers component resolves identically.
    let mm = ModelMarkers::from_rig(&rig);
    assert!(mm.get("fore_emitter").is_some());
    assert!(mm.get("missing").is_none());
}

#[test]
fn target_points_array_form_parses() {
    let toml = r##"
[[target_points]]
position = [0.5, -0.1, 0.0]

[[target_points]]
position = [-0.25, -0.1, -0.25]
"##;
    let rig = parse_model_rig(toml).expect("target points must parse");
    assert_eq!(rig.target_points.len(), 2);
    assert!(approx(rig.target_points[0].position, [0.5, -0.1, 0.0]));
    assert!(approx(rig.target_points[1].position, [-0.25, -0.1, -0.25]));

    let mm = ModelMarkers::from_rig(&rig);
    assert_eq!(mm.target_point_count(), 2);
    assert!(approx(
        mm.target_point(1).unwrap().position,
        [-0.25, -0.1, -0.25]
    ));
}

#[test]
fn base_bevy_transform_identity_for_default() {
    let rig = ModelRig::default();
    let t = rig.base_bevy_transform();
    assert!((t.translation - Vec3::ZERO).length() < EPS);
    assert!((t.scale - Vec3::ONE).length() < EPS);
    assert!((t.rotation.angle_between(Quat::IDENTITY)).abs() < EPS);
}

// ── Real-asset linkage (alliance_destroyer → its own sidecar) ─────────────

/// End-to-end (pure, native) proof of marker linkage on a realistic test
/// target: the Alliance Destroyer's `omni` phaser bank carries `marker =
/// "phasers_omni"`, its `[mesh] model` resolves to a sidecar that defines a
/// `phasers_omni` marker, and the resolver returns it.
///
/// (#892) This ran against `pirate_raider.toml` → `dynasty_destroyer.model.toml`
/// until that hull was retired as a duplicate. It was the ONLY entity
/// referencing either `dynasty_destroyer.glb` or its `fore_emitter` marker,
/// so there is no like-for-like replacement: the sidecar is now orphaned
/// content. The linkage claim itself is hull-agnostic, so it moves to a
/// shipped (hull, sidecar, marker) triple that still exists.
#[test]
fn alliance_destroyer_omni_bank_marker_resolves_in_sidecar() {
    // Through the include resolver (issue #906) so the linkage claim keeps
    // holding once the hull is composed.
    let cfg = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_destroyer.toml",
    )
    .expect("alliance_destroyer must compose and parse");

    // The omni bank links to a marker.
    let weapons = cfg
        .weapons_console
        .as_ref()
        .expect("weapons_console present");
    let omni_bank = weapons
        .phaser_banks
        .iter()
        .find(|b| b.id == "omni")
        .expect("omni bank present");
    let marker_name = omni_bank
        .marker
        .as_deref()
        .expect("omni bank carries a marker name");
    assert_eq!(marker_name, "phasers_omni");

    // The mesh model resolves to a sidecar (default variant).
    let mesh = cfg.mesh.as_ref().expect("mesh present");
    let model_path = mesh.model.as_deref().expect("model path present");
    let path = sidecar_path(model_path, mesh.variant.as_deref());
    assert_eq!(
        path,
        "assets/models/alliance_destroyer_recreated.model.toml"
    );

    // Parse the sidecar and resolve the linked marker.
    let rig_toml = crate::repo_fixtures::fs::read_to_string(&path)
        .expect("alliance_destroyer sidecar must exist");
    let rig = parse_model_rig(&rig_toml).expect("sidecar must parse");
    rig.marker(marker_name)
        .expect("phasers_omni marker must resolve in the sidecar");

    // Missing marker → None (caller falls back to origin).
    assert!(rig.marker("does_not_exist").is_none());
}

// ── Sidecar-owned LOD chains (issue #914) ────────────────────────────

#[test]
fn lod_chain_parses_from_sidecar_toml() {
    let toml = r##"
[base]
scale = [2.0, 2.0, 2.0]

[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"
variant = "small"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod2.glb"

[[lod]]
shape = "sphere"
"##;
    let rig = parse_model_rig(toml).expect("a sidecar ladder must parse");
    assert_eq!(rig.lod.len(), 3);
    assert_eq!(rig.lod[0].max_distance, Some(50.0));
    assert_eq!(rig.lod[0].model.as_deref(), Some("assets/models/rock.glb"));
    assert_eq!(rig.lod[0].variant.as_deref(), Some("small"));
    // A level may omit `variant`: it inherits the entity's `[mesh] variant`.
    assert_eq!(rig.lod[1].variant, None);
    // The last level is the procedural fallback and has no upper bound.
    assert_eq!(rig.lod[2].max_distance, None);
    assert_eq!(
        rig.lod[2].shape,
        Some(crate::entities::config::MeshShape::Sphere)
    );
}

#[test]
fn a_sidecar_without_a_ladder_has_an_empty_chain() {
    let rig = parse_model_rig("[base]\nscale = [1.0, 1.0, 1.0]\n").expect("parses");
    assert!(
        rig.lod.is_empty(),
        "no `[[lod]]` means no ladder — the entity renders its flat [mesh]"
    );
}

/// A generated level carries the parameters that produced it (issue #919).
/// The renderer ignores them; the point is that they parse, so the sidecar
/// can own its whole ladder without the engine rejecting the sidecar.
#[test]
fn a_level_carries_its_generation_parameters() {
    let toml = r##"
[[lod]]
max_distance = 50.0
model = "assets/models/rock.glb"

[[lod]]
max_distance = 150.0
model = "assets/models/rock_lod2.glb"
[lod.generate]
source = "assets/models/rock.glb"
ratio = 0.05
error = 0.1
texture_size = 256

[[lod]]
shape = "sphere"
"##;
    let rig = parse_model_rig(toml).expect("generation params must parse");
    // The near level is authored, not generated.
    assert!(rig.lod[0].generate.is_none());
    let gen = rig.lod[1].generate.as_ref().expect("level 1 is generated");
    assert_eq!(gen.source.as_deref(), Some("assets/models/rock.glb"));
    assert_eq!(gen.ratio, Some(0.05));
    assert_eq!(gen.error, Some(0.1));
    assert_eq!(gen.texture_size, Some(256));
    // Absent means "no Blender pre-pass", not "voxel size zero".
    assert_eq!(gen.remesh_voxel_size, None);
    // Selection is untouched by the added block: still four bands.
    assert_eq!(rig.lod.len(), 3);
}

/// The schema is strict, so a mistyped key fails loudly instead of
/// resolving to a rig that quietly lost a marker or a whole ladder.
#[test]
fn an_unknown_sidecar_field_is_rejected() {
    assert!(parse_model_rig("[base]\noffest = [1.0, 0.0, 0.0]\n").is_err());
    assert!(parse_model_rig("lods = []\n").is_err());
    assert!(parse_model_rig("[[lod]]\nmax_distance = 50.0\nmodell = \"a.glb\"\n").is_err());
    // …including inside the build-time block, where a typo would otherwise
    // detach the level from the generator that maintains it (issue #919).
    assert!(parse_model_rig(
        "[[lod]]\nmodel = \"a.glb\"\n[lod.generate]\nratio = 0.5\nerorr = 0.01\n"
    )
    .is_err());
    assert!(parse_model_rig(
        "[markers.fore]\nposition = [0.0, 0.0, 0.0]\ndirection = [0.0, 0.0, -1.0]\nfacing = 1.0\n"
    )
    .is_err());
}

#[test]
fn sidecar_variant_reads_the_variant_back_out_of_a_path() {
    assert_eq!(
        sidecar_variant("assets/models/asteroid_common_1.large.toml"),
        Some("large")
    );
    assert_eq!(
        sidecar_variant("assets/models/dynasty_destroyer.model.toml"),
        Some(DEFAULT_VARIANT)
    );
    // Round-trips with `sidecar_path` for both the default and a named variant.
    for variant in [None, Some("weathered")] {
        let path = sidecar_path("assets/models/ship.glb", variant);
        assert_eq!(
            sidecar_variant(&path),
            Some(variant.unwrap_or(DEFAULT_VARIANT))
        );
    }
    assert_eq!(sidecar_variant("assets/models/ship.glb"), None);
    assert_eq!(sidecar_variant("noextension"), None);
}

// ── Shipped-asset conformance ────────────────────────────────────────

/// The migration itself (issue #914), asserted on a real shipped pair: the
/// asteroid entity no longer carries a ladder, and the sidecar its `[mesh]`
/// resolves to does — with every level's GLB present on disk.
#[test]
fn a_shipped_asteroid_reads_its_ladder_from_its_sidecar() {
    let cfg = crate::entities::include_resolve::load_entity_config(
        "assets/entities/asteroid_common_1_large.toml",
    )
    .expect("asteroid template must parse");
    let mesh = cfg.mesh.as_ref().expect("mesh present");
    let path = sidecar_path(
        mesh.model.as_deref().expect("model path"),
        mesh.variant.as_deref(),
    );
    assert_eq!(path, "assets/models/asteroid_common_1.large.toml");

    let rig =
        parse_model_rig(&crate::repo_fixtures::fs::read_to_string(&path).expect("sidecar exists"))
            .expect("sidecar parses");
    assert_eq!(
        rig.lod.len(),
        4,
        "three GLB steps plus a procedural fallback"
    );

    // Ascending, exclusive bounds; only the final level is unbounded.
    let bounds: Vec<Option<f32>> = rig.lod.iter().map(|l| l.max_distance).collect();
    assert_eq!(
        bounds,
        vec![Some(15.0), Some(100.0), Some(400.0), None],
        "bands ported from asteroid_common_4's tuned ladder so all four \
             common asteroids share one visual LOD profile"
    );

    // Switching behaviour over the ported chain, at the authored distances.
    use crate::entities::config::select_lod;
    for (distance, expected) in [
        (0.0, 0),
        (14.9, 0),
        (15.0, 1),
        (99.9, 1),
        (100.0, 2),
        (399.9, 2),
        (400.0, 3),
        (10_000.0, 3),
    ] {
        assert_eq!(
            select_lod(&rig.lod, distance, None),
            expected,
            "distance {distance} must select level {expected}"
        );
    }
    // …and hysteresis still holds each band across its boundary.
    assert_eq!(select_lod(&rig.lod, 17.0, Some(0)), 0);
    assert_eq!(select_lod(&rig.lod, 13.0, Some(1)), 1);

    // Every GLB level names a file that exists, at the entity's variant.
    for level in rig.lod.iter().filter(|l| l.model.is_some()) {
        let model = level.model.as_deref().unwrap();
        assert!(
            crate::repo_fixtures::path(model).exists(),
            "LOD level model {model} must exist"
        );
        let level_sidecar =
            sidecar_path(model, level.variant.as_deref().or(mesh.variant.as_deref()));
        assert!(
            crate::repo_fixtures::path(&level_sidecar).exists(),
            "LOD level sidecar {level_sidecar} must exist"
        );
    }
    // Both decimated levels declare how they are regenerated (issue #919),
    // so deleting a `_lod*.glb` is recoverable from the sidecar alone. The
    // near level is the source and declares nothing.
    assert!(rig.lod[0].generate.is_none(), "the source is not generated");
    for level in &rig.lod[1..3] {
        let gen = level
            .generate
            .as_ref()
            .expect("every decimated level declares its parameters");
        assert_eq!(
            gen.source.as_deref(),
            Some("assets/models/asteroid_common_1.glb"),
            "both steps decimate the full model, not each other"
        );
        assert!(matches!(gen.ratio, Some(r) if r > 0.0 && r < 1.0));
        assert!(gen.error.is_some());
        assert!(gen.texture_size.is_some());
    }

    // The last level is the shared billboard that replaced the procedural
    // far sphere: a captured yaw-ring atlas of the hull, sized per variant
    // from the captured world extent by the authoring tool
    // (scripts/capture-billboards.mjs). It is a billboard, not a shape, and
    // records how it was baked in `[lod.capture]`.
    let last = rig.lod.last().unwrap();
    assert_eq!(
        last.shape, None,
        "the far level is a billboard, not a shape"
    );
    assert_eq!(
        last.billboard.as_deref(),
        Some("assets/models/asteroid_common_1_lod3.png")
    );
    assert_eq!(
        last.scale,
        Some([7.9615, 4.8775, 1.0]),
        "the large variant's billboard is sized to its captured world extent"
    );
    assert_eq!(
        last.capture.as_ref().and_then(|c| c.yaw_views),
        Some(8),
        "the billboard records its capture provenance"
    );
}

/// The huge size class's ladder (issue #947), which is the same four
/// models on bands three times further out.
///
/// A LOD switch is an angular threshold wearing a distance's clothes: the
/// bands say "swap detail when this thing gets small on screen", and a rock
/// at three times the radius is that small three times further away. Left
/// on large's bands, a huge rock would drop to its procedural far sphere
/// while still filling a fifth of the viewscreen.
///
/// The generated `.glb` files themselves are SHARED with the small and
/// large variants — no new geometry ships for this size — so the ratios
/// must be identical across the three sidecars. `generate-lods.mjs` refuses
/// two sidecars that disagree about one output, which is the check that
/// catches a retune here; asserted from the Rust side too, because that
/// script only runs in the Node job.
#[test]
fn the_huge_asteroid_variant_scales_its_bands_and_not_its_ratios() {
    for n in 1..=4 {
        let read = |variant: &str| {
            let path = format!("assets/models/asteroid_common_{n}.{variant}.toml");
            let text = crate::repo_fixtures::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{path} must exist: {e}"));
            parse_model_rig(&text).unwrap_or_else(|e| panic!("{path} must parse: {e}"))
        };
        let large = read("large");
        let huge = read("huge");

        let bounds = |rig: &ModelRig| -> Vec<Option<f32>> {
            rig.lod.iter().map(|l| l.max_distance).collect()
        };
        assert_eq!(
            bounds(&large),
            vec![Some(15.0), Some(100.0), Some(400.0), None],
            "the large bands this test is relative to"
        );
        assert_eq!(
            bounds(&huge),
            vec![Some(45.0), Some(300.0), Some(1200.0), None],
            "asteroid_common_{n}: the huge bands are large's, x3"
        );

        // Same models, same decimation, three times the authored scale.
        let models = |rig: &ModelRig| -> Vec<Option<String>> {
            rig.lod.iter().map(|l| l.model.clone()).collect()
        };
        assert_eq!(
            models(&large),
            models(&huge),
            "no new geometry for this size"
        );
        for (l, h) in large.lod.iter().zip(huge.lod.iter()) {
            let (Some(lg), Some(hg)) = (l.generate.as_ref(), h.generate.as_ref()) else {
                assert!(
                    l.generate.is_none() && h.generate.is_none(),
                    "asteroid_common_{n}: one variant generates a level the other does not"
                );
                continue;
            };
            assert_eq!(lg.source, hg.source);
            assert_eq!(lg.ratio, hg.ratio);
            assert_eq!(lg.error, hg.error);
            assert_eq!(lg.texture_size, hg.texture_size);
            assert_eq!(lg.remesh_voxel_size, hg.remesh_voxel_size);
        }
        // The rig scales the mesh so its widest horizontal half-extent is
        // the entity's collider radius — 4 for large, 12 for huge. That,
        // and not the raw scale factor, is the invariant: two of the four
        // commons ship a `large` rig whose scale drifted about a twentieth
        // of a percent (0.052% and 0.037%) from `4 / half-width` (their
        // .glb was re-exported without the sidecar being re-derived), and
        // this size class must not silently "fix" an existing size by
        // inheriting from it.
        let half_width = |rig: &ModelRig| -> f32 {
            let e = rig.extents.as_ref().expect("the rig caches its extents");
            [
                e.min[0].abs(),
                e.max[0].abs(),
                e.min[2].abs(),
                e.max[2].abs(),
            ]
            .into_iter()
            .fold(0.0f32, f32::max)
        };
        assert!(
            (half_width(&large) - 4.0).abs() < 1e-4,
            "asteroid_common_{n}: large fills a collider of radius 4"
        );
        assert!(
            (half_width(&huge) - 12.0).abs() < 1e-4,
            "asteroid_common_{n}: huge fills a collider of radius 12 — the same mesh, \
                 three times the authored scale"
        );

        // Every level the preloader may fetch has its own huge sidecar.
        for level in huge.lod.iter().filter(|l| l.model.is_some()) {
            let sidecar = sidecar_path(level.model.as_deref().unwrap(), Some("huge"));
            assert!(
                crate::repo_fixtures::path(&sidecar).exists(),
                "LOD level sidecar {sidecar} must exist"
            );
        }
    }
}

#[test]
fn base_bevy_transform_xyz_euler_correct() {
    let toml = r##"
[base]
offset = [1.0, 2.0, 3.0]
rotation = [0.5, 0.0, 0.0]
scale = [2.0, 3.0, 4.0]
"##;
    let rig = parse_model_rig(toml).unwrap();
    let t = rig.base_bevy_transform();
    assert!((t.translation - Vec3::new(1.0, 2.0, 3.0)).length() < EPS);
    assert!((t.scale - Vec3::new(2.0, 3.0, 4.0)).length() < EPS);

    let expected = Quat::from_euler(EulerRot::XYZ, 0.5, 0.0, 0.0);
    assert!(t.rotation.angle_between(expected).abs() < EPS);

    // A +0.5 rad rotation about X maps +Z toward -Y / +... sanity: rotate
    // forward (0,0,-1) and confirm it tilts in Y.
    let fwd = t.rotation * Vec3::new(0.0, 0.0, -1.0);
    assert!(fwd.y.abs() > 0.1, "X-rotation should tilt forward in Y");
}

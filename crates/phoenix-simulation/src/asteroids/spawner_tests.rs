use super::*;

#[test]
fn all_positions_within_outer_radius() {
    let result = generate_donut_field(
        100.0,
        200.0,
        0.005,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );

    for spawn in &result.spawns {
        let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
        assert!(
            dist <= 200.0,
            "Position ({}, {}) is outside outer radius, dist = {}",
            spawn.x,
            spawn.z,
            dist
        );
    }
}

#[test]
fn no_positions_inside_inner_radius() {
    let result = generate_donut_field(
        100.0,
        200.0,
        0.005,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );

    for spawn in &result.spawns {
        let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
        assert!(
            dist >= 100.0,
            "Position ({}, {}) is inside inner radius, dist = {}",
            spawn.x,
            spawn.z,
            dist
        );
    }
}

#[test]
fn count_within_tolerance_of_area_times_density() {
    let inner = 100.0;
    let outer = 200.0;
    let density = 0.005;

    let result = generate_donut_field(
        inner,
        outer,
        density,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );

    let ring_area = PI * (outer.powi(2) - inner.powi(2));
    let expected = (ring_area * density).round() as usize;
    let actual = result.count;

    // Allow 10% tolerance
    let tolerance = (expected as f32 * 0.1).ceil() as usize;
    assert!(
        (expected >= actual && expected - actual <= tolerance)
            || (actual >= expected && actual - expected <= tolerance)
            || expected == actual,
        "Count {} not within 10% of expected {}",
        actual,
        expected
    );
}

#[test]
fn same_inputs_produce_same_positions() {
    let result_a = generate_donut_field(
        100.0,
        200.0,
        0.005,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );

    let result_b = generate_donut_field(
        100.0,
        200.0,
        0.005,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );

    assert_eq!(result_a.spawns, result_b.spawns, "Determinism failed");
}

#[test]
fn different_seed_offset_produces_different_layouts() {
    let result_a = generate_donut_field(
        100.0,
        200.0,
        0.005,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );

    let result_b = generate_donut_field(
        100.0,
        200.0,
        0.005,
        43,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );

    assert_ne!(
        result_a.spawns, result_b.spawns,
        "Different seeds should produce different layouts"
    );
}

#[test]
fn gameplay_asteroids_come_first() {
    let result = generate_donut_field(
        100.0,
        200.0,
        0.01,
        42,
        &["gameplay1.toml".to_string(), "gameplay2.toml".to_string()],
        &["cosmetic1.toml".to_string()],
    );

    // Find the first cosmetic asteroid
    let first_cosmetic_idx = result
        .spawns
        .iter()
        .position(|s| s.config_path.contains("cosmetic"));

    // Find the last gameplay asteroid
    let last_gameplay_idx = result
        .spawns
        .iter()
        .rev()
        .position(|s| s.config_path.contains("gameplay"))
        .map(|pos| result.spawns.len() - 1 - pos);

    if let (Some(first_cosmetic), Some(last_gameplay)) = (first_cosmetic_idx, last_gameplay_idx) {
        assert!(
            last_gameplay < first_cosmetic,
            "Gameplay asteroids should come before cosmetic asteroids"
        );
    }
}

#[test]
fn type_assignment_selects_from_provided_list() {
    let paths = vec![
        "type1.toml".to_string(),
        "type2.toml".to_string(),
        "type3.toml".to_string(),
    ];

    let result = generate_donut_field(100.0, 200.0, 0.01, 42, &paths, &[]);

    for spawn in &result.spawns {
        assert!(
            paths.contains(&spawn.config_path),
            "Type path {} not in provided list",
            spawn.config_path
        );
    }
}

#[test]
fn empty_type_paths_produces_no_asteroids() {
    let result = generate_donut_field(100.0, 200.0, 0.01, 42, &[], &[]);

    assert_eq!(result.count, 0);
    assert!(result.spawns.is_empty());
}

#[test]
fn only_gameplay_types_produces_only_gameplay() {
    let result = generate_donut_field(100.0, 200.0, 0.01, 42, &["gameplay.toml".to_string()], &[]);

    for spawn in &result.spawns {
        assert!(
            spawn.config_path.contains("gameplay"),
            "Should only have gameplay types"
        );
    }
}

#[test]
fn only_cosmetic_types_produces_only_cosmetic() {
    let result = generate_donut_field(100.0, 200.0, 0.01, 42, &[], &["cosmetic.toml".to_string()]);

    for spawn in &result.spawns {
        assert!(
            spawn.config_path.contains("cosmetic"),
            "Should only have cosmetic types"
        );
    }
}

#[test]
fn uuids_are_deterministic() {
    let uuids_a = generate_donut_uuids(100.0, 200.0, 0.005, 42, 10);
    let uuids_b = generate_donut_uuids(100.0, 200.0, 0.005, 42, 10);
    assert_eq!(uuids_a, uuids_b);
}

#[test]
fn uuids_are_unique() {
    let uuids = generate_donut_uuids(100.0, 200.0, 0.005, 42, 50);
    let mut seen = std::collections::HashSet::new();
    for id in &uuids {
        assert!(seen.insert(id.as_str()), "Duplicate UUID: {}", id);
    }
}

#[test]
fn uuids_have_expected_format() {
    let uuids = generate_donut_uuids(100.0, 200.0, 0.005, 42, 5);
    for id in &uuids {
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 5, "UUID should have 5 parts: {}", id);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
    }
}

#[test]
fn different_seed_produces_different_uuids() {
    let uuids_a = generate_donut_uuids(100.0, 200.0, 0.005, 42, 10);
    let uuids_b = generate_donut_uuids(100.0, 200.0, 0.005, 43, 10);
    assert_ne!(uuids_a, uuids_b);
}

#[test]
fn zero_count_returns_empty() {
    let result = generate_donut_field(
        100.0,
        200.0,
        0.0,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );
    assert!(result.spawns.is_empty());
    assert_eq!(result.count, 0);
}

#[test]
fn eval_cell_deterministic() {
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.4,
        fill_cosmetic: 0.15,
        uniformity: 0.3,
        noise_freq: 0.02,
        noise_octaves: 3,
        density_noise_freq: 0.01,
        density_noise_octaves: 2,
        jitter: 10.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let a = eval_cell(
        0,
        5,
        3,
        &grid,
        100.0,
        200.0,
        &["gameplay.toml".to_string()],
        &[],
    );
    let b = eval_cell(
        0,
        5,
        3,
        &grid,
        100.0,
        200.0,
        &["gameplay.toml".to_string()],
        &[],
    );
    assert_eq!(a, b, "eval_cell must be deterministic");
}

#[test]
fn eval_cell_returns_none_for_failed_density() {
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 1.0,
        fill_cosmetic: 1.0,
        uniformity: 1.0,
        noise_freq: 0.02,
        noise_octaves: 1,
        density_noise_freq: 0.01,
        density_noise_octaves: 1,
        jitter: 10.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let result = eval_cell(
        0,
        5,
        3,
        &grid,
        100.0,
        200.0,
        &["gameplay.toml".to_string()],
        &[],
    );
    assert!(result.is_none(), "Should be None when fill is 1.0");
}

#[test]
fn eval_cell_returns_some_for_gameplay() {
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.0,
        fill_cosmetic: 0.0,
        uniformity: 0.0,
        noise_freq: 0.02,
        noise_octaves: 1,
        density_noise_freq: 0.01,
        density_noise_octaves: 1,
        jitter: 0.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let result = eval_cell(
        0,
        5,
        3,
        &grid,
        100.0,
        200.0,
        &["gameplay.toml".to_string()],
        &[],
    );
    assert!(result.is_some(), "Should be Some when fill is 0.0");
    let spawn = result.unwrap();
    assert_eq!(spawn.x, 75.0, "Cell center X = cx * resolution");
    assert_eq!(spawn.z, 45.0, "Cell center Z = cz * resolution");
    assert_eq!(spawn.y, 0.0, "Gameplay Y must be 0 when variance is 0");
    assert!(spawn.config_path.contains("gameplay"));
}

#[test]
fn grid_positions_within_torus_bounds() {
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.4,
        fill_cosmetic: 0.15,
        uniformity: 0.3,
        noise_freq: 0.02,
        noise_octaves: 3,
        density_noise_freq: 0.01,
        density_noise_octaves: 2,
        jitter: 10.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let result = generate_grid_field(
        100.0,
        200.0,
        grid,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );
    for spawn in &result.gameplay {
        let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
        assert!(
            (100.0..=200.0).contains(&dist),
            "Gameplay pos ({}, {}) dist={} outside torus [100,200]",
            spawn.x,
            spawn.z,
            dist
        );
    }
    for spawn in &result.cosmetic_upper {
        let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
        assert!(
            (100.0..=200.0).contains(&dist),
            "Cosmetic upper pos ({}, {}) dist={} outside torus [100,200]",
            spawn.x,
            spawn.z,
            dist
        );
    }
    for spawn in &result.cosmetic_lower {
        let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
        assert!(
            (100.0..=200.0).contains(&dist),
            "Cosmetic lower pos ({}, {}) dist={} outside torus [100,200]",
            spawn.x,
            spawn.z,
            dist
        );
    }
}

#[test]
fn grid_same_params_same_output() {
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.4,
        fill_cosmetic: 0.15,
        uniformity: 0.3,
        noise_freq: 0.02,
        noise_octaves: 3,
        density_noise_freq: 0.01,
        density_noise_octaves: 2,
        jitter: 10.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let result_a = generate_grid_field(
        100.0,
        200.0,
        grid.clone(),
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );
    let result_b = generate_grid_field(
        100.0,
        200.0,
        grid,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );
    assert_eq!(
        result_a.gameplay, result_b.gameplay,
        "Gameplay must be deterministic"
    );
    assert_eq!(
        result_a.cosmetic_upper, result_b.cosmetic_upper,
        "Cosmetic upper must be deterministic"
    );
    assert_eq!(
        result_a.cosmetic_lower, result_b.cosmetic_lower,
        "Cosmetic lower must be deterministic"
    );
    assert_eq!(result_a.count, result_b.count);
}

#[test]
fn grid_cosmetic_y_offsets_correct_sign() {
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.0,
        fill_cosmetic: 1.0,
        uniformity: 0.0,
        noise_freq: 0.02,
        noise_octaves: 1,
        density_noise_freq: 0.01,
        density_noise_octaves: 1,
        jitter: 0.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let result = generate_grid_field(100.0, 200.0, grid, 42, &[], &["cosmetic.toml".to_string()]);
    for spawn in &result.gameplay {
        assert_eq!(spawn.y, 0.0, "Gameplay Y must be 0");
    }
    for spawn in &result.cosmetic_upper {
        assert!(
            spawn.y > 0.0 && spawn.y <= 15.0,
            "Cosmetic upper Y={} must be in (0, 15]",
            spawn.y
        );
    }
    for spawn in &result.cosmetic_lower {
        assert!(
            spawn.y < 0.0 && spawn.y >= -15.0,
            "Cosmetic lower Y={} must be in [-15, 0)",
            spawn.y
        );
    }
}

#[test]
fn eval_cell_gameplay_y_within_variance() {
    let variance = 0.5;
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.0,
        fill_cosmetic: 0.0,
        uniformity: 0.0,
        noise_freq: 0.02,
        noise_octaves: 1,
        density_noise_freq: 0.01,
        density_noise_octaves: 1,
        jitter: 0.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: variance,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let result = eval_cell(
        0,
        5,
        3,
        &grid,
        100.0,
        200.0,
        &["gameplay.toml".to_string()],
        &[],
    );
    let spawn = result.expect("Should spawn when fill is 0.0");
    assert!(
        spawn.y.abs() <= variance,
        "Gameplay Y={} must be within variance ±{}",
        spawn.y,
        variance
    );
}

// ── Torus shape eligibility tests ──────────────────────────────────────

#[test]
fn cell_in_field_none_shape_uses_center_distance() {
    // Cell placement-point at (75, 45): distance ≈ 87.46. With inner=100,
    // the legacy centre-distance test rejects it (87 < 100).
    assert!(!cell_in_field(5, 3, 15.0, 100.0, 200.0, None));

    // Cell placement-point at (150, 150): distance ≈ 212. With outer=200,
    // the legacy centre-distance test rejects it (212 > 200).
    assert!(!cell_in_field(10, 10, 15.0, 100.0, 200.0, None));

    // Cell placement at (75, 75) → dist ≈ 106, inside the [100, 200] annulus.
    assert!(cell_in_field(5, 5, 15.0, 100.0, 200.0, None));
}

#[test]
fn cell_in_field_torus_admits_cell_straddling_inner_radius() {
    // Cell at gx=7, gz=0 with resolution=15:
    //   placement centre = (105, 0)
    //   bbox = [97.5..112.5] × [-7.5..7.5]
    //   nearest corner from origin = (97.5, 0) → dist = 97.5
    //   farthest corner = (112.5, 7.5) → dist ≈ 112.75
    // With inner_radius=100, the bbox straddles the inner boundary.
    // Torus admits it.
    assert!(cell_in_field(
        7,
        0,
        15.0,
        100.0,
        200.0,
        Some(AsteroidFieldShape::Torus)
    ));
}

#[test]
fn cell_in_field_torus_rejects_cell_fully_inside_inner_radius() {
    // Cell at gx=0, gz=0 with resolution=10:
    //   bbox centred at origin = [-5..5] × [-5..5]
    //   farthest corner = (5, 5) → dist ≈ 7.07
    // With inner_radius=50, the entire bbox is inside the inner hole.
    assert!(!cell_in_field(
        0,
        0,
        10.0,
        50.0,
        200.0,
        Some(AsteroidFieldShape::Torus)
    ));
}

#[test]
fn cell_in_field_torus_rejects_cell_fully_outside_outer_radius() {
    // Cell at gx=20, gz=20 with resolution=15:
    //   placement centre = (300, 300)
    //   bbox = [292.5..307.5] × [292.5..307.5]
    //   nearest corner = (292.5, 292.5) → dist ≈ 413.66
    // With outer_radius=200, the nearest corner is well beyond.
    assert!(!cell_in_field(
        20,
        20,
        15.0,
        100.0,
        200.0,
        Some(AsteroidFieldShape::Torus)
    ));
}

#[test]
fn cell_in_field_torus_admits_cell_straddling_outer_radius() {
    // Cell at gx=13, gz=0 with resolution=15:
    //   placement centre = (195, 0)
    //   bbox = [187.5..202.5] × [-7.5..7.5]
    //   nearest corner = (187.5, 0) → dist = 187.5 (inside outer=200)
    //   farthest = (202.5, 7.5) → dist ≈ 202.6 (outside outer=200)
    // Straddles outer boundary. Admitted because nearest is inside.
    assert!(cell_in_field(
        13,
        0,
        15.0,
        100.0,
        200.0,
        Some(AsteroidFieldShape::Torus)
    ));
}

#[test]
fn cell_in_field_torus_admits_cell_containing_origin() {
    // Cell at gx=0, gz=0 with resolution=15:
    //   bbox = [-7.5..7.5] × [-7.5..7.5] (contains origin)
    //   nearest corner distance = 0 (origin is inside the bbox)
    //   With inner_radius=10, the cell straddles the inner boundary
    //   (farthest corner at sqrt(112.5) ≈ 10.6 > 10).
    assert!(cell_in_field(
        0,
        0,
        15.0,
        10.0,
        200.0,
        Some(AsteroidFieldShape::Torus)
    ));
}

#[test]
fn cell_in_field_torus_zero_inner_radius_admits_central_cells() {
    // With inner_radius = 0, no cells are "fully inside" the inner
    // hole (the hole has no area). Admit any cell whose nearest
    // corner is inside outer_radius.
    assert!(cell_in_field(
        0,
        0,
        15.0,
        0.0,
        200.0,
        Some(AsteroidFieldShape::Torus)
    ));
}

#[test]
fn cell_in_field_torus_negative_coords_symmetric() {
    // Symmetry across quadrants: rejection of cells far in -X, -Z.
    assert!(!cell_in_field(
        -20,
        -20,
        15.0,
        100.0,
        200.0,
        Some(AsteroidFieldShape::Torus)
    ));
    // Cell whose bbox crosses the outer boundary on the -X side.
    assert!(cell_in_field(
        -13,
        0,
        15.0,
        100.0,
        200.0,
        Some(AsteroidFieldShape::Torus)
    ));
}

#[test]
fn cell_in_field_operates_in_anchor_relative_space() {
    // PRD #397 fix 5: anchor support is implemented as a pure post-seed
    // translation. `cell_in_field` is anchor-agnostic — it tests cells
    // against the annulus centred on the field-local origin. Callers
    // (the streaming spawner) must convert the player's world position
    // into anchor-relative grid coordinates BEFORE calling it.
    //
    // This test pins the contract: a cell at anchor-relative (0,0)
    // straddles the inner radius (admitted when inner=10), and a cell
    // at anchor-relative (20,20) is far outside (rejected) — regardless
    // of where the anchor lives in world space. If a refactor ever
    // pushes anchor knowledge into `cell_in_field`, this test fails.
    let res = 15.0;

    // A cell at anchor-relative origin: bbox contains (0,0), so it is
    // admitted for a torus that includes the centre (inner=0 or small).
    assert!(
        cell_in_field(0, 0, res, 0.0, 200.0, Some(AsteroidFieldShape::Torus)),
        "anchor-relative origin cell must be admitted when inner_radius=0"
    );

    // A cell at anchor-relative (20,20) (≈ 424 from anchor) is well
    // outside outer=200 — rejected.
    assert!(
        !cell_in_field(20, 20, res, 100.0, 200.0, Some(AsteroidFieldShape::Torus)),
        "anchor-relative (20,20) is ~424 units out; must be rejected by outer_radius=200"
    );

    // A cell at anchor-relative (7,0) straddles inner_radius=100 — admitted.
    assert!(
        cell_in_field(7, 0, res, 100.0, 200.0, Some(AsteroidFieldShape::Torus)),
        "anchor-relative (7,0) straddles inner_radius=100; must be admitted"
    );

    // Symmetric: anchor-relative (-7,0) likewise straddles inner — admitted.
    assert!(
        cell_in_field(-7, 0, res, 100.0, 200.0, Some(AsteroidFieldShape::Torus)),
        "cell_in_field must be symmetric in anchor-relative space"
    );
}

#[test]
fn eval_cell_is_anchor_independent_anchor_is_pure_translation() {
    // PRD #397 fix 5 / AGENTS.md rule 6: the per-cell density seed is
    // `(seed salt, gx, gz)` and MUST NOT include the anchor. The anchor
    // is a pure post-seed translation — since #913 it is applied inside
    // the composed evaluator (`covering_for_cell` translates the lattice
    // cell into field-local space; the returned position is the lattice
    // cell's world centre plus jitter), and `eval_cell` itself remains
    // anchor-free.
    //
    // This test pins three invariants:
    //   1. `eval_cell` takes no anchor parameter (compiles as-is).
    //   2. Two calls with identical (field_idx, gx, gz) return identical
    //      anchor-relative positions, regardless of what anchor a caller
    //      might add later.
    //   3. Translating the returned position by an anchor offset
    //      produces the expected world-space position — modelling the
    //      contract the composed evaluator honours.
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.0,
        fill_cosmetic: 0.0,
        uniformity: 0.0,
        noise_freq: 0.02,
        noise_octaves: 1,
        density_noise_freq: 0.01,
        density_noise_octaves: 1,
        jitter: 0.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };

    // `fill_gameplay = 0.0` reliably yields Some (see
    // `eval_cell_returns_some_for_gameplay`). `jitter = 0.0` makes the
    // returned position the exact cell centre — easy to assert against.
    let (field_idx, gx, gz) = (7_u64, 8_i32, 4_i32);
    let anchor_relative = eval_cell(
        field_idx,
        gx,
        gz,
        &grid,
        100.0,
        200.0,
        &["gameplay.toml".to_string()],
        &[],
    )
    .expect("fill_gameplay=0.0 must produce Some");

    // Invariant 1: cell centre is `(gx*res, _, gz*res)` — anchor-relative,
    // anchor is not even an input to eval_cell.
    assert_eq!(anchor_relative.x, gx as f32 * grid.resolution);
    assert_eq!(anchor_relative.z, gz as f32 * grid.resolution);

    // Invariant 2: identical inputs → identical output (anchor-independent).
    let again = eval_cell(
        field_idx,
        gx,
        gz,
        &grid,
        100.0,
        200.0,
        &["gameplay.toml".to_string()],
        &[],
    )
    .expect("second call with identical (field_idx, gx, gz) must also spawn");
    assert_eq!(
        (anchor_relative.x, anchor_relative.y, anchor_relative.z),
        (again.x, again.y, again.z),
        "eval_cell must be anchor-independent: identical (field_idx, gx, gz) → identical anchor-relative position"
    );

    // Invariant 3: post-seed translation by anchor_offset gives the
    // expected world-space position — model of the contract the
    // composed evaluator honours internally: `covering_for_cell`
    // translates the lattice cell into field-local space, and the
    // returned position is the lattice cell's world centre (anchor-
    // independent) plus jitter.
    let anchor_offset = [100.0_f32, 0.0, 100.0];
    let world_x = anchor_relative.x + anchor_offset[0];
    let world_z = anchor_relative.z + anchor_offset[2];
    assert_eq!(
        world_x,
        gx as f32 * grid.resolution + 100.0,
        "world X = anchor-relative X + anchor_offset.x"
    );
    assert_eq!(
        world_z,
        gz as f32 * grid.resolution + 100.0,
        "world Z = anchor-relative Z + anchor_offset.z"
    );
    // Y is anchor-irrelevant (anchor is XZ-only by design).
}

#[test]
fn generate_grid_field_with_shape_none_matches_legacy() {
    // Determinism / back-compat: when shape = None, the new variant
    // must produce identical output to the legacy `generate_grid_field`.
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.4,
        fill_cosmetic: 0.15,
        uniformity: 0.3,
        noise_freq: 0.02,
        noise_octaves: 3,
        density_noise_freq: 0.01,
        density_noise_octaves: 2,
        jitter: 10.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let legacy = generate_grid_field(
        100.0,
        200.0,
        grid.clone(),
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
    );
    let shaped = generate_grid_field_with_shape(
        100.0,
        200.0,
        grid,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
        None,
    );
    assert_eq!(legacy.gameplay, shaped.gameplay);
    assert_eq!(legacy.cosmetic_upper, shaped.cosmetic_upper);
    assert_eq!(legacy.cosmetic_lower, shaped.cosmetic_lower);
    assert_eq!(legacy.count, shaped.count);
}

#[test]
fn generate_grid_field_torus_positions_near_annulus() {
    // Torus eligibility admits cells whose bbox overlaps the annulus —
    // including cells whose centre lies just outside [r_min, r_max].
    // The per-cell jitter does not pull such positions back inside the
    // annulus (seed derivation unchanged from legacy). We assert a
    // looser bound: every position lies within one cell width of the
    // annulus.
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.4,
        fill_cosmetic: 0.15,
        uniformity: 0.3,
        noise_freq: 0.02,
        noise_octaves: 3,
        density_noise_freq: 0.01,
        density_noise_octaves: 2,
        jitter: 10.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let res = grid.resolution;
    let result = generate_grid_field_with_shape(
        100.0,
        200.0,
        grid,
        42,
        &["gameplay.toml".to_string()],
        &["cosmetic.toml".to_string()],
        Some(AsteroidFieldShape::Torus),
    );
    // Bounded tolerance: one cell diagonal beyond the annulus on either side.
    let tol = res * std::f32::consts::SQRT_2;
    for spawn in result
        .gameplay
        .iter()
        .chain(result.cosmetic_upper.iter())
        .chain(result.cosmetic_lower.iter())
    {
        let dist = (spawn.x * spawn.x + spawn.z * spawn.z).sqrt();
        assert!(
            dist >= 100.0 - tol && dist <= 200.0 + tol,
            "torus pos ({}, {}) dist={} outside [{}, {}]",
            spawn.x,
            spawn.z,
            dist,
            100.0 - tol,
            200.0 + tol,
        );
    }
}

#[test]
fn generate_grid_field_torus_count_ge_legacy_count() {
    // The torus (bbox-overlap) admits a superset of cells compared to
    // the legacy centre-distance test, so the gameplay+cosmetic counts
    // must be ≥ the legacy counts (the same per-cell seed produces the
    // same per-cell outcome for the cells common to both).
    let grid = GridConfig {
        resolution: 15.0,
        fill_gameplay: 0.4,
        fill_cosmetic: 0.15,
        uniformity: 0.3,
        noise_freq: 0.02,
        noise_octaves: 3,
        density_noise_freq: 0.01,
        density_noise_octaves: 2,
        jitter: 10.0,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    };
    let legacy = generate_grid_field_with_shape(
        100.0,
        200.0,
        grid.clone(),
        42,
        &["gameplay.toml".to_string()],
        &[],
        None,
    );
    let torus = generate_grid_field_with_shape(
        100.0,
        200.0,
        grid,
        42,
        &["gameplay.toml".to_string()],
        &[],
        Some(AsteroidFieldShape::Torus),
    );
    assert!(
        torus.gameplay.len() >= legacy.gameplay.len(),
        "torus admits a superset of cells: legacy={} torus={}",
        legacy.gameplay.len(),
        torus.gameplay.len(),
    );
}

// ── Composed density evaluation (#913) ─────────────────────────────────

fn cgrid(resolution: f32, fill_gameplay: f32, jitter: f32) -> GridConfig {
    GridConfig {
        resolution,
        fill_gameplay,
        fill_cosmetic: fill_gameplay,
        uniformity: 0.3,
        noise_freq: 0.02,
        noise_octaves: 3,
        density_noise_freq: 0.01,
        density_noise_octaves: 2,
        jitter,
        cosmetic_y_offset: 15.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 10,
        despawn_cells: 12,
    }
}

fn contribution(
    weight: f32,
    inner: f32,
    outer: f32,
    anchor: [f32; 3],
    grid: GridConfig,
    gameplay: &[&str],
    cosmetic: &[&str],
) -> FieldContribution {
    FieldContribution {
        weight,
        inner_radius: inner,
        outer_radius: outer,
        shape: None,
        anchor_offset: anchor,
        grid,
        gameplay_types: gameplay
            .iter()
            .copied()
            .map(AsteroidTypeRef::from)
            .collect(),
        cosmetic_types: cosmetic
            .iter()
            .copied()
            .map(AsteroidTypeRef::from)
            .collect(),
        shield_pierce: 0.0,
        random_rotation: None,
    }
}

/// Same, but with an authored rarity weight per gameplay type (#946).
fn weighted_contribution(
    inner: f32,
    outer: f32,
    grid: GridConfig,
    gameplay: &[(&str, f32)],
) -> FieldContribution {
    FieldContribution {
        weight: 1.0,
        inner_radius: inner,
        outer_radius: outer,
        shape: None,
        anchor_offset: [0.0, 0.0, 0.0],
        grid,
        gameplay_types: gameplay
            .iter()
            .map(|(path, weight)| AsteroidTypeRef::Weighted {
                path: (*path).to_string(),
                weight: *weight,
            })
            .collect(),
        cosmetic_types: Vec::new(),
        shield_pierce: 0.0,
        random_rotation: None,
    }
}

/// Bit-compat proof: a composition of ONE origin-anchored field is the
/// legacy evaluator. For every cell the streaming path would have
/// admitted (`cell_in_field`), the composed result must equal
/// `eval_cell` with the same seed salt — gameplay layer salt 0 and
/// cosmetic-upper salt 0x0001_0000_0000 are exactly the seeds the
/// per-field windows used for field 0, so single-field worlds keep
/// their pre-#913 layouts bit for bit at this test's resolution (15.0,
/// where gx*res/res round-trips exactly in f32; not guaranteed for
/// arbitrary resolutions like 0.1).
#[test]
fn composed_single_field_matches_legacy_eval_cell() {
    let grid = cgrid(15.0, 0.4, 10.0);
    let fields = [contribution(
        1.0,
        100.0,
        200.0,
        [0.0, 0.0, 0.0],
        grid.clone(),
        &["gameplay.toml"],
        &["cosmetic.toml"],
    )];
    for gx in -15..=15 {
        for gz in -15..=15 {
            let eligible = cell_in_field(gx, gz, 15.0, 100.0, 200.0, None);

            let composed_gameplay =
                eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay);
            let legacy_gameplay = if eligible {
                eval_cell(
                    0,
                    gx,
                    gz,
                    &grid,
                    100.0,
                    200.0,
                    &["gameplay.toml".to_string()],
                    &[],
                )
            } else {
                None
            };
            assert_eq!(
                composed_gameplay.clone().map(|(s, _)| s),
                legacy_gameplay,
                "gameplay parity broken at cell ({gx}, {gz})"
            );
            if let Some((_, idx)) = composed_gameplay {
                assert_eq!(idx, 0);
            }

            let composed_cosmetic =
                eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::CosmeticUpper);
            let legacy_cosmetic = if eligible {
                eval_cell(
                    0x0001_0000_0000,
                    gx,
                    gz,
                    &grid,
                    100.0,
                    200.0,
                    &[],
                    &["cosmetic.toml".to_string()],
                )
            } else {
                None
            };
            assert_eq!(
                composed_cosmetic.map(|(s, _)| s),
                legacy_cosmetic,
                "cosmetic parity broken at cell ({gx}, {gz})"
            );
        }
    }
}

/// The regression the composition exists to fix: in a cell covered by
/// two fields, the pre-#913 per-field evaluators BOTH spawned (double
/// spawn); the composed evaluator returns exactly one spawn.
#[test]
fn composed_overlap_spawns_exactly_once_where_legacy_doubled() {
    let grid = cgrid(15.0, 0.0, 0.0); // fill 0 → every covered cell spawns
    let a = contribution(
        1.0,
        0.0,
        150.0,
        [0.0, 0.0, 0.0],
        grid.clone(),
        &["a.toml"],
        &[],
    );
    let b = contribution(
        1.0,
        100.0,
        250.0,
        [0.0, 0.0, 0.0],
        grid.clone(),
        &["b.toml"],
        &[],
    );

    // Cell (8, 0) sits at distance 120 — inside both annuli.
    let (gx, gz) = (8, 0);
    assert!(cell_in_field(gx, gz, 15.0, 0.0, 150.0, None));
    assert!(cell_in_field(gx, gz, 15.0, 100.0, 250.0, None));

    // Old world: each field ran its own window and its own eval — two
    // rocks in one cell.
    let legacy_a = eval_cell(0, gx, gz, &grid, 0.0, 150.0, &["a.toml".to_string()], &[]);
    let legacy_b = eval_cell(1, gx, gz, &grid, 100.0, 250.0, &["b.toml".to_string()], &[]);
    assert!(
        legacy_a.is_some() && legacy_b.is_some(),
        "per-field evaluation double-spawns this cell — precondition for the regression"
    );

    // New world: one composed evaluation, one rock.
    let composed = eval_cell_composed(&[a, b], 15.0, gx, gz, ComposedLayer::Gameplay);
    assert!(
        composed.is_some(),
        "the overlap cell must still spawn (fill is 0)"
    );
}

/// Same contributions → identical composed field across a full sweep.
#[test]
fn composed_evaluator_is_deterministic() {
    let make = || {
        vec![
            contribution(
                2.0,
                0.0,
                150.0,
                [0.0, 0.0, 0.0],
                cgrid(15.0, 0.3, 10.0),
                &["a.toml"],
                &["ca.toml"],
            ),
            contribution(
                1.0,
                100.0,
                250.0,
                [50.0, 0.0, -25.0],
                cgrid(15.0, 0.5, 5.0),
                &["b.toml"],
                &["cb.toml"],
            ),
        ]
    };
    let fields_a = make();
    let fields_b = make();
    for layer in [
        ComposedLayer::Gameplay,
        ComposedLayer::CosmeticUpper,
        ComposedLayer::CosmeticLower,
    ] {
        for gx in -18..=18 {
            for gz in -18..=18 {
                assert_eq!(
                    eval_cell_composed(&fields_a, 15.0, gx, gz, layer),
                    eval_cell_composed(&fields_b, 15.0, gx, gz, layer),
                    "composed evaluation must be deterministic at ({gx}, {gz}, {layer:?})"
                );
            }
        }
    }
}

/// Per-type rarity weights actually bias the draw (issue #946): with the
/// tiers the shipped fields author — 1.0 / 0.1 / 0.01 — commons must
/// dominate, uncommons must be clearly rarer, and rares rarer again while
/// still appearing at all. The counts are exact, not statistical: every
/// cell is a pure function of its coordinates, so this test either always
/// passes or always fails.
#[test]
fn weighted_types_bias_selection_by_rarity_tier() {
    let fields = [weighted_contribution(
        0.0,
        900.0,
        cgrid(15.0, 0.0, 0.0),
        &[
            ("common.toml", 1.0),
            ("uncommon.toml", 0.1),
            ("rare.toml", 0.01),
        ],
    )];
    let (mut common, mut uncommon, mut rare) = (0, 0, 0);
    for gx in -40..=40 {
        for gz in -40..=40 {
            if let Some((spawn, _)) =
                eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay)
            {
                match spawn.config_path.as_str() {
                    "common.toml" => common += 1,
                    "uncommon.toml" => uncommon += 1,
                    "rare.toml" => rare += 1,
                    other => panic!("unexpected config path {other}"),
                }
            }
        }
    }
    assert!(
        common + uncommon + rare > 5000,
        "the sample needs to be big enough for a 1:100 tier to show: {common}/{uncommon}/{rare}"
    );
    assert!(rare > 0, "a 1:100 type must still appear at all");
    assert!(
        uncommon > rare * 3,
        "rares must be visibly rarer than uncommons: uncommon={uncommon} rare={rare}"
    );
    assert!(
        common > uncommon * 3,
        "uncommons must be visibly rarer than commons: common={common} uncommon={uncommon}"
    );
}

/// Weighting must not cost a draw (issue #946, AGENTS.md rule 8).
///
/// The type pick is resolved from ONE uniform draw mapped onto the
/// cumulative weights, in the same slot in the per-cell sequence the old
/// `random_range` occupied. So for any cell, a weighted list and the bare
/// (unweighted) spelling of the same paths must agree on everything the
/// draws around the pick decide: the jittered position drawn before it,
/// and — the load-bearing one — the gameplay Y drawn *after* it. A pick
/// that consumed a different amount of entropy would shift Y.
#[test]
fn weighting_a_type_list_does_not_shift_the_draw_sequence() {
    let mut grid = cgrid(15.0, 0.0, 20.0);
    grid.gameplay_y_variance = 5.0;
    let plain = [contribution(
        1.0,
        0.0,
        900.0,
        [0.0, 0.0, 0.0],
        grid.clone(),
        &["a.toml", "b.toml", "c.toml"],
        &[],
    )];
    let weighted = [weighted_contribution(
        0.0,
        900.0,
        grid,
        &[("a.toml", 1.0), ("b.toml", 0.1), ("c.toml", 0.01)],
    )];
    let mut differed = 0;
    for gx in -20..=20 {
        for gz in -20..=20 {
            let a = eval_cell_composed(&plain, 15.0, gx, gz, ComposedLayer::Gameplay);
            let b = eval_cell_composed(&weighted, 15.0, gx, gz, ComposedLayer::Gameplay);
            match (a, b) {
                (None, None) => {}
                (Some((a, _)), Some((b, _))) => {
                    assert_eq!((a.x, a.z), (b.x, b.z), "position at ({gx}, {gz})");
                    assert_eq!(a.y, b.y, "gameplay Y at ({gx}, {gz})");
                    if a.config_path != b.config_path {
                        differed += 1;
                    }
                }
                (a, b) => panic!("weighting changed whether ({gx}, {gz}) spawns: {a:?} {b:?}"),
            }
        }
    }
    assert!(
        differed > 0,
        "the weights must still change WHICH type is picked, or this proves nothing"
    );
}

/// The pick costs exactly the one draw `random_range` cost (issue #946,
/// AGENTS.md rule 8).
///
/// `weighting_a_type_list_does_not_shift_the_draw_sequence` above compares
/// two authored *spellings* — bare paths and weighted entries — and both
/// run through [`pick_weighted_type`], so it pins that re-weighting an
/// authored list is free. It cannot pin what the doc comment on
/// [`pick_weighted_type`] actually claims: that the pick sits in the same
/// slot, at the same cost, as the pre-#946 `rng.random_range(0..len)`.
/// A rewrite to rejection sampling or a draw per candidate would keep that
/// test green and silently move every gameplay Y in every shipped field.
///
/// So this reconstructs the pre-#946 consumption from a bare `StdRng` —
/// the cell seed, the density draw, the jitter magnitude, then
/// `random_range(0..len)` where the type pick goes — and asserts the Y that
/// falls out next is the Y the evaluator produced. Y is the load-bearing
/// one: it is drawn *after* the pick, so it moves the instant the pick's
/// entropy cost changes. (X and Z are drawn before it and cannot.)
#[test]
fn the_type_pick_costs_the_same_draw_the_old_random_range_did() {
    let mut grid = cgrid(15.0, 0.0, 20.0);
    grid.gameplay_y_variance = 5.0;
    let y_variance = grid.gameplay_y_variance;
    let types: &[(&str, f32)] = &[("a.toml", 1.0), ("b.toml", 0.1), ("c.toml", 0.01)];
    let fields = [weighted_contribution(0.0, 900.0, grid, types)];

    let mut checked = 0;
    for gx in -20..=20 {
        for gz in -20..=20 {
            let Some((spawn, _)) =
                eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay)
            else {
                continue;
            };

            // The cell seed, mixed exactly as `eval_covered_cell` does
            // (the gameplay layer's salt is 0).
            let seed = {
                let mut s = ComposedLayer::Gameplay.seed_salt();
                s = s.wrapping_mul(2654435761);
                s = s.wrapping_add(gx as u64);
                s = s.wrapping_mul(2654435761);
                s = s.wrapping_add(gz as u64);
                s
            };
            let mut rng = StdRng::seed_from_u64(seed);
            // 1. density gate, 2. jitter magnitude — unchanged by #946.
            let _density = rng.random::<f32>();
            let _jitter_magnitude = rng.random::<f32>();
            // 3. THE PRE-#946 TYPE PICK. A single covering field means no
            //    field-selection draw sits between these.
            let _type_index = rng.random_range(0..types.len());
            // 4. gameplay Y.
            let y = (rng.random::<f32>() * 2.0 - 1.0) * y_variance;

            assert_eq!(
                spawn.y, y,
                "gameplay Y at ({gx}, {gz}) moved: the weighted pick no longer consumes the \
                 single draw `random_range(0..len)` consumed before issue #946"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 100,
        "the sweep has to actually spawn rocks to pin anything: {checked} cells checked"
    );
}

/// Degenerate authoring guard, matching the field-level one: a type list
/// whose weights are all zero falls back to a uniform draw rather than
/// dividing by zero or erasing the field.
#[test]
fn zero_weighted_type_list_falls_back_to_uniform() {
    let fields = [weighted_contribution(
        0.0,
        900.0,
        cgrid(15.0, 0.0, 0.0),
        &[("a.toml", 0.0), ("b.toml", 0.0)],
    )];
    let mut seen: Vec<String> = Vec::new();
    for gx in -20..=20 {
        for gz in -20..=20 {
            if let Some((spawn, _)) =
                eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay)
            {
                if !seen.contains(&spawn.config_path) {
                    seen.push(spawn.config_path.clone());
                }
            }
        }
    }
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec!["a.toml".to_string(), "b.toml".to_string()],
        "all-zero weights must draw both types uniformly, not erase the list"
    );
}

/// Weight picks the contributing field: a weight-3 field should supply
/// roughly three times as many rocks as a weight-1 field across a full
/// overlap, and both must contribute.
#[test]
fn composed_weight_biases_field_selection() {
    let grid = cgrid(15.0, 0.0, 0.0);
    let fields = [
        contribution(
            3.0,
            0.0,
            300.0,
            [0.0, 0.0, 0.0],
            grid.clone(),
            &["a.toml"],
            &[],
        ),
        contribution(
            1.0,
            0.0,
            300.0,
            [0.0, 0.0, 0.0],
            grid.clone(),
            &["b.toml"],
            &[],
        ),
    ];
    let mut a_count = 0;
    let mut b_count = 0;
    for gx in -20..=20 {
        for gz in -20..=20 {
            if let Some((spawn, _)) =
                eval_cell_composed(&fields, 15.0, gx, gz, ComposedLayer::Gameplay)
            {
                match spawn.config_path.as_str() {
                    "a.toml" => a_count += 1,
                    "b.toml" => b_count += 1,
                    other => panic!("unexpected config path {other}"),
                }
            }
        }
    }
    assert!(b_count > 0, "the weight-1 field must still contribute");
    assert!(
        a_count > b_count * 2,
        "weight 3 vs 1 should skew selection heavily: a={a_count} b={b_count}"
    );
}

/// Degenerate authoring guard: if every covering field is zero-weighted
/// the blend falls back to uniform instead of dividing by zero.
#[test]
fn composed_zero_weights_fall_back_to_uniform() {
    let grid = cgrid(15.0, 0.0, 0.0);
    let fields = [
        contribution(
            0.0,
            0.0,
            300.0,
            [0.0, 0.0, 0.0],
            grid.clone(),
            &["a.toml"],
            &[],
        ),
        contribution(
            0.0,
            0.0,
            300.0,
            [0.0, 0.0, 0.0],
            grid.clone(),
            &["b.toml"],
            &[],
        ),
    ];
    let spawn = eval_cell_composed(&fields, 15.0, 3, 4, ComposedLayer::Gameplay);
    assert!(
        spawn.is_some(),
        "all-zero weights must not erase the field (uniform fallback)"
    );
}

/// A contribution anchored away from the origin covers the translated
/// region and nothing else; positions come back in world space.
#[test]
fn composed_anchor_translates_field_coverage() {
    let grid = cgrid(25.0, 0.0, 0.0);
    let fields = [contribution(
        1.0,
        0.0,
        100.0,
        [600.0, 0.0, 0.0],
        grid,
        &["a.toml"],
        &[],
    )];

    // Cell (24, 0) → world (600, 0) → field-local (0, 0): covered.
    let hit = eval_cell_composed(&fields, 25.0, 24, 0, ComposedLayer::Gameplay)
        .expect("cell at the anchor must be covered");
    assert_eq!(
        hit.0.x, 600.0,
        "jitter 0 → spawn at the cell's world-space centre"
    );
    assert_eq!(hit.0.z, 0.0);

    // Cell (0, 0) → field-local (-600, 0): far outside the disc.
    assert!(
        eval_cell_composed(&fields, 25.0, 0, 0, ComposedLayer::Gameplay).is_none(),
        "the world origin is not covered by a field anchored at x=600"
    );
}

/// The shared lattice: finest authored resolution, largest authored
/// spawn/despawn windows; empty composition has no lattice.
#[test]
fn composed_lattice_takes_finest_resolution_and_largest_windows() {
    let mut coarse = cgrid(25.0, 0.4, 0.0);
    coarse.spawn_cells = 10;
    coarse.despawn_cells = 12;
    let mut fine = cgrid(10.0, 0.4, 0.0);
    fine.spawn_cells = 30;
    fine.despawn_cells = 32;

    let fields = [
        contribution(1.0, 0.0, 100.0, [0.0; 3], coarse, &["a.toml"], &[]),
        contribution(1.0, 0.0, 100.0, [0.0; 3], fine, &["b.toml"], &[]),
    ];
    let lattice = composed_lattice(&fields).expect("non-empty composition has a lattice");
    assert_eq!(lattice.resolution, 10.0);
    assert_eq!(lattice.spawn_cells, 30);
    assert_eq!(lattice.despawn_cells, 32);

    assert!(composed_lattice(&[]).is_none());
}

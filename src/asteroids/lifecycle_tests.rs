use super::*;
use crate::entities::config::{AsteroidFieldConfig, GridConfig};
use crate::entities::spawner::AsteroidFieldSection;
use crate::lobby::OutboundMessage;
use crate::server_app::SimOutbox;

/// A rock's uuid keys its row in the headless report's `damage_by_ship`
/// ledger, so two distinct rocks sharing one is silent data corruption
/// rather than a cosmetic clash. The first packed implementation aliased
/// two ways and both are pinned here: `cell_gx` values differing only in
/// the top two bits of their low byte (the bits the v4 variant stamp
/// overwrites), and the `(field_idx, slot_x)` swap that two overlapping
/// fields hit for real.
#[test]
fn cell_uuids_are_unique_across_the_identifying_tuple() {
    let mut seen: std::collections::HashMap<String, (usize, i32, i32, usize, usize)> =
        std::collections::HashMap::new();
    for field_idx in 0..4usize {
        for cell_gx in [-193, -1, 0, 1, 64, 65, 128, 192, 256] {
            for cell_gz in [-64, 0, 3, 64, 128] {
                for slot_x in 0..4usize {
                    for slot_z in 0..4usize {
                        let key = (field_idx, cell_gx, cell_gz, slot_x, slot_z);
                        let uuid =
                            deterministic_cell_uuid(field_idx, cell_gx, cell_gz, slot_x, slot_z);
                        if let Some(other) = seen.insert(uuid.clone(), key) {
                            panic!("{key:?} and {other:?} share uuid {uuid}");
                        }
                    }
                }
            }
        }
    }

    // Same rock, same name — the identity has to be stable, not merely
    // collision-free, or a respawned asteroid changes ledger row.
    assert_eq!(
        deterministic_cell_uuid(1, 7, -9, 2, 3),
        deterministic_cell_uuid(1, 7, -9, 2, 3)
    );
    // v4 formatting survives the hashing.
    let parsed =
        uuid::Uuid::parse_str(&deterministic_cell_uuid(0, 0, 0, 0, 0)).expect("valid uuid");
    assert_eq!(parsed.get_version_num(), 4);
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.add_message::<OutboundMessage>();
    app.init_resource::<SimOutbox>();
    app.init_resource::<AsteroidWindow>();
    app.init_resource::<AsteroidEntityMap>();
    app.init_resource::<WorldResource>();
    app.init_resource::<crate::server_app::LastBroadcastEntityPositions>();
    app.init_resource::<crate::server_app::LastBroadcastEntityHealth>();
    // Spawn a fleet ship with ShipPhysics so update_asteroid_window can
    // query it. `FleetSlotOf` is what the window now centres on (issue
    // #1116); `LocalShip` rides along because `set_ship_pos` moves the ship
    // through that marker and, in production, the two sit on the same hull.
    app.world_mut().spawn((
        crate::server_app::LocalShip,
        crate::lockstep::FleetSlotOf(crate::command_admission::HostSlot::SOLO),
        bevy::prelude::Transform::default(),
        crate::ship::state::ShipPhysics::default(),
    ));
    // (#913) One composed window per world: every AsteroidFieldSection
    // entity feeds the same evaluator and the same window resource.
    app.add_systems(Update, update_asteroid_window);
    app
}

fn set_ship_pos(app: &mut App, x: f32, z: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::ship::state::ShipPhysics, With<crate::server_app::LocalShip>>(
        );
    let mut p = q
        .single_mut(app.world_mut())
        .expect("expected LocalShip with ShipPhysics");
    p.x = x;
    p.z = z;
}

fn grid(resolution: f32) -> GridConfig {
    GridConfig {
        resolution,
        fill_gameplay: 0.0,
        fill_cosmetic: 0.0,
        uniformity: 0.0,
        noise_freq: 0.02,
        noise_octaves: 3,
        density_noise_freq: 0.01,
        density_noise_octaves: 2,
        jitter: 0.0,
        cosmetic_y_offset: 0.0,
        gameplay_y_variance: 0.0,
        spawn_cells: 2,
        despawn_cells: 3,
    }
}

fn field(grid_resolution: f32) -> AsteroidFieldConfig {
    AsteroidFieldConfig {
        inner_radius: 100.0,
        outer_radius: 200.0,
        density: 0.0,
        weight: 1.0,
        spawn_distance: 150.0,
        despawn_distance: 250.0,
        asteroid_type_paths: vec!["asteroid_small.toml".into()],
        cosmetic_type_paths: vec![],
        tags: vec![],
        grid: Some(grid(grid_resolution)),
        shield_pierce: 0.0,
        shape: None,
        anchor: None,
        anchor_offset: [0.0, 0.0, 0.0],
        random_rotation: None,
    }
}

/// Helper: an annulus field with a torus shape and a dense fill, so
/// every eligible cell spawns and assertions are exact.
fn torus_field(
    inner_radius: f32,
    outer_radius: f32,
    resolution: f32,
    spawn_cells: u32,
    despawn_cells: u32,
) -> AsteroidFieldConfig {
    AsteroidFieldConfig {
        inner_radius,
        outer_radius,
        density: 0.0,
        weight: 1.0,
        spawn_distance: 150.0,
        despawn_distance: 250.0,
        asteroid_type_paths: vec!["asteroid_small.toml".into()],
        cosmetic_type_paths: vec![],
        tags: vec![],
        grid: Some(GridConfig {
            resolution,
            fill_gameplay: 0.0, // admit every covered cell
            fill_cosmetic: 1.0,
            uniformity: 0.0,
            noise_freq: 0.02,
            noise_octaves: 1,
            density_noise_freq: 0.01,
            density_noise_octaves: 1,
            jitter: 0.0,
            cosmetic_y_offset: 0.0,
            gameplay_y_variance: 0.0,
            spawn_cells,
            despawn_cells,
        }),
        shield_pierce: 0.0,
        shape: Some(crate::entities::config::AsteroidFieldShape::Torus),
        anchor: None,
        anchor_offset: [0.0, 0.0, 0.0],
        random_rotation: None,
    }
}

#[test]
fn window_initialises_from_spawned_asteroid_field_section() {
    let mut app = test_app();
    // WorldResource is init'd by test_app. The system should find the field.
    app.world_mut()
        .spawn((AsteroidFieldSection(field(15.0)), Transform::default()));
    app.update();

    let window = app.world().resource::<AsteroidWindow>();
    assert_eq!(
        window.resolution, 15.0,
        "window.resolution should be sourced from the composed lattice"
    );
    assert_eq!(window.spawn_cells, 2);
    assert_eq!(window.despawn_cells, 3);
    assert!(!window.needs_init, "first tick must run the full rebuild");
}

#[test]
fn canonical_window_is_restore_ready_before_a_paused_candidate_spends_a_tick() {
    use crate::entities::spawner::EntityUuid;

    let mut canonical = test_app();
    canonical
        .world_mut()
        .spawn((AsteroidFieldSection(field(15.0)), Transform::default()));
    canonical
        .world_mut()
        .spawn(EntityUuid("standing-entity".into()));
    canonical.update();
    let snapshot = crate::snapshot::capture(canonical.world());
    assert!(
        !snapshot
            .asteroid_window
            .as_ref()
            .expect("the canonical host captures its window")
            .needs_init
    );

    // This is the reconnect candidate immediately after its GameStart
    // topology arrived under the technical Pause: fields and roster rows
    // exist, but FixedUpdate has not run and must not run before restore.
    let mut candidate = test_app();
    candidate
        .world_mut()
        .spawn((AsteroidFieldSection(field(15.0)), Transform::default()));
    candidate
        .world_mut()
        .spawn(EntityUuid("standing-entity".into()));
    assert!(candidate.world().resource::<AsteroidWindow>().needs_init);
    assert!(
        crate::snapshot::ready_to_restore(candidate.world(), &snapshot),
        "matching live fields make the canonical window safe to install without a private tick"
    );

    let mut wrong = test_app();
    wrong
        .world_mut()
        .spawn((AsteroidFieldSection(field(30.0)), Transform::default()));
    wrong
        .world_mut()
        .spawn(EntityUuid("standing-entity".into()));
    assert!(
        !crate::snapshot::ready_to_restore(wrong.world(), &snapshot),
        "a different field composition must still fail closed"
    );
}

#[test]
fn window_does_nothing_with_no_field_entity() {
    // (#913) With no field entity there is nothing to compose; the
    // window resource stays untouched and no asteroid spawns.
    let mut app = test_app();
    app.update();
    let window = app.world().resource::<AsteroidWindow>();
    assert!(
        window.player_grid.is_none(),
        "no AsteroidFieldSection entity → the window never initialises"
    );
    let mut q = app.world_mut().query::<&Asteroid>();
    assert_eq!(q.iter(app.world()).count(), 0);
}

#[test]
fn asteroid_field_shield_pierce_defaults_to_zero_in_toml() {
    // Pre-#414 behaviour: asteroid impacts are fully absorbed by shields.
    // A TOML file that does not mention shield_pierce must continue to
    // behave that way after the field is added.
    let toml = r#"
inner_radius = 100.0
outer_radius = 200.0
density = 0.5
asteroid_type_paths = ["x.toml"]
"#;
    let cfg: AsteroidFieldConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.shield_pierce, 0.0);
}

#[test]
fn asteroid_field_shield_pierce_parses_when_present_in_toml() {
    let toml = r#"
inner_radius = 100.0
outer_radius = 200.0
density = 0.5
asteroid_type_paths = ["x.toml"]
shield_pierce = 0.4
"#;
    let cfg: AsteroidFieldConfig = toml::from_str(toml).unwrap();
    assert!((cfg.shield_pierce - 0.4).abs() < 1e-6);
}

#[test]
fn asteroid_field_weight_defaults_to_one_in_toml() {
    // (#913) A field that does not author a weight is an equal partner
    // in the composed density blend.
    let toml = r#"
inner_radius = 100.0
outer_radius = 200.0
density = 0.5
asteroid_type_paths = ["x.toml"]
"#;
    let cfg: AsteroidFieldConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.weight, 1.0);
}

#[test]
fn asteroid_field_weight_parses_when_present_in_toml() {
    let toml = r#"
inner_radius = 100.0
outer_radius = 200.0
density = 0.5
weight = 2.5
asteroid_type_paths = ["x.toml"]
"#;
    let cfg: AsteroidFieldConfig = toml::from_str(toml).unwrap();
    assert!((cfg.weight - 2.5).abs() < 1e-6);
}

#[test]
fn streaming_full_rebuild_with_torus_keeps_positions_near_annulus() {
    // Drive a full rebuild and assert that every spawned gameplay
    // asteroid sits near the annulus [inner_radius, outer_radius].
    // Torus eligibility admits cells whose bbox overlaps the annulus,
    // so positions may extend up to one cell diagonal beyond either
    // boundary. After the player departs, all asteroids must despawn.
    let mut app = test_app();
    let res = 15.0f32;
    let f = torus_field(100.0, 200.0, res, 20, 22);
    // Anchor the player on the belt so the spawn window covers it.
    set_ship_pos(&mut app, 150.0, 0.0);
    app.world_mut()
        .spawn((AsteroidFieldSection(f), Transform::default()));
    app.update();

    let tol = res * std::f32::consts::SQRT_2;
    let mut q = app.world_mut().query::<(&Transform, &Asteroid)>();
    let mut count = 0;
    for (t, _) in q.iter(app.world()) {
        let d = (t.translation.x.powi(2) + t.translation.z.powi(2)).sqrt();
        assert!(
            d >= 100.0 - tol && d <= 200.0 + tol,
            "asteroid at ({}, {}) dist={} outside [{}, {}]",
            t.translation.x,
            t.translation.z,
            d,
            100.0 - tol,
            200.0 + tol,
        );
        count += 1;
    }
    assert!(count > 0, "no asteroids spawned — test set-up is wrong");

    // Move the player far away → full rebuild should clear them.
    set_ship_pos(&mut app, 10_000.0, 10_000.0);
    app.update();
    let mut q = app.world_mut().query::<&Asteroid>();
    let remaining = q.iter(app.world()).count();
    assert_eq!(
        remaining, 0,
        "all asteroids must despawn after departing the belt"
    );
}

/// (#475 → #913) Multi-field: two fields with disjoint annuli compose
/// into one density field whose support is the union of the two bands.
/// Both bands must produce asteroids when the player sits between them.
#[test]
fn two_fields_with_disjoint_annuli_each_spawn_asteroids() {
    let mut app = test_app();

    let inner = torus_field(100.0, 150.0, 25.0, 30, 32);
    let outer = torus_field(400.0, 500.0, 25.0, 30, 32);

    // Position the player between the two annuli so the composed spawn
    // window overlaps both belts.
    set_ship_pos(&mut app, 250.0, 0.0);
    app.world_mut()
        .spawn((AsteroidFieldSection(inner), Transform::default()));
    app.world_mut()
        .spawn((AsteroidFieldSection(outer), Transform::default()));
    app.update();

    // Each spawned asteroid must lie within either the inner annulus
    // [100, 150] or the outer annulus [400, 500] (with the cell-diagonal
    // tolerance Torus eligibility introduces).
    let tol = 25.0 * std::f32::consts::SQRT_2;
    let mut asteroid_q = app.world_mut().query::<(&Transform, &Asteroid)>();
    let mut inner_count = 0;
    let mut outer_count = 0;
    for (t, _) in asteroid_q.iter(app.world()) {
        let d = (t.translation.x.powi(2) + t.translation.z.powi(2)).sqrt();
        if d >= 100.0 - tol && d <= 150.0 + tol {
            inner_count += 1;
        } else if d >= 400.0 - tol && d <= 500.0 + tol {
            outer_count += 1;
        } else {
            panic!(
                "asteroid at dist={} fell outside both annuli [100..150] and [400..500]",
                d
            );
        }
    }
    assert!(
        inner_count > 0,
        "inner belt must have spawned at least one asteroid (got 0)"
    );
    assert!(
        outer_count > 0,
        "outer belt must have spawned at least one asteroid \
             — the composed field's support is the union of the authored bands"
    );
}

/// (#913) The headline regression: two OVERLAPPING fields must not
/// double-spawn in the overlap band. With the per-field windows each
/// field evaluated the shared cells independently and both spawned;
/// the composed evaluator runs each lattice cell exactly once.
#[test]
fn overlapping_fields_spawn_each_cell_at_most_once() {
    let mut app = test_app();
    let res = 25.0f32;

    // Annuli [100, 200] and [150, 250] — the band [150, 200] is covered
    // by both. fill 0.0 + jitter 0.0 → every covered cell spawns exactly
    // at its centre, so cell occupancy is exact.
    let a = torus_field(100.0, 200.0, res, 12, 14);
    let b = torus_field(150.0, 250.0, res, 12, 14);

    set_ship_pos(&mut app, 175.0, 0.0);
    app.world_mut()
        .spawn((AsteroidFieldSection(a), Transform::default()));
    app.world_mut()
        .spawn((AsteroidFieldSection(b), Transform::default()));
    app.update();

    let mut q = app.world_mut().query::<(&Transform, &Asteroid)>();
    let mut cells = std::collections::HashSet::new();
    let mut overlap_band_count = 0;
    let mut total = 0;
    for (t, _) in q.iter(app.world()) {
        let cell = (
            (t.translation.x / res).round() as i32,
            (t.translation.z / res).round() as i32,
        );
        assert!(
            cells.insert(cell),
            "cell {cell:?} spawned more than one asteroid — overlapping \
                 fields must compose, not double-spawn"
        );
        let d = (t.translation.x.powi(2) + t.translation.z.powi(2)).sqrt();
        if (160.0..=190.0).contains(&d) {
            overlap_band_count += 1;
        }
        total += 1;
    }
    assert!(total > 0, "no asteroids spawned — test set-up is wrong");
    assert!(
        overlap_band_count > 0,
        "the overlap band [160, 190] must contain asteroids — otherwise \
             this test never exercised the composed path"
    );
}

/// (#913) Same authored fields → identical composed field, run over run:
/// every rock at the same position with the same uuid.
#[test]
fn composed_field_is_deterministic_across_runs() {
    let build = || {
        let mut app = test_app();
        let a = torus_field(100.0, 200.0, 25.0, 12, 14);
        let b = torus_field(150.0, 250.0, 25.0, 12, 14);
        set_ship_pos(&mut app, 175.0, 0.0);
        app.world_mut()
            .spawn((AsteroidFieldSection(a), Transform::default()));
        app.world_mut()
            .spawn((AsteroidFieldSection(b), Transform::default()));
        app.update();
        let mut q = app.world_mut().query::<(&Transform, &AsteroidUuid)>();
        let mut rocks: Vec<(String, [i64; 3])> = q
            .iter(app.world())
            .map(|(t, u)| {
                (
                    u.0.clone(),
                    [
                        (t.translation.x * 1000.0) as i64,
                        (t.translation.y * 1000.0) as i64,
                        (t.translation.z * 1000.0) as i64,
                    ],
                )
            })
            .collect();
        rocks.sort();
        rocks
    };
    let run_a = build();
    let run_b = build();
    assert!(!run_a.is_empty(), "no asteroids spawned");
    assert_eq!(run_a, run_b, "same fields must produce the same rocks");
}

/// (#913) Adding a field entity mid-run changes the composition key and
/// forces a full rebuild against the new composed field — with no
/// leftover duplicates from the old composition.
#[test]
fn adding_a_field_recomposes_without_duplicates() {
    let mut app = test_app();
    let res = 25.0f32;
    set_ship_pos(&mut app, 175.0, 0.0);
    app.world_mut().spawn((
        AsteroidFieldSection(torus_field(100.0, 200.0, res, 12, 14)),
        Transform::default(),
    ));
    app.update();

    let mut q = app.world_mut().query::<&Asteroid>();
    let single_field_count = q.iter(app.world()).count();
    assert!(single_field_count > 0, "first field spawned nothing");

    // A second, overlapping field arrives (e.g. a world layer loads).
    app.world_mut().spawn((
        AsteroidFieldSection(torus_field(150.0, 250.0, res, 12, 14)),
        Transform::default(),
    ));
    app.update();

    let mut q = app.world_mut().query::<(&Transform, &Asteroid)>();
    let mut cells = std::collections::HashSet::new();
    let mut new_band = 0;
    for (t, _) in q.iter(app.world()) {
        let cell = (
            (t.translation.x / res).round() as i32,
            (t.translation.z / res).round() as i32,
        );
        assert!(
            cells.insert(cell),
            "cell {cell:?} holds more than one rock after recomposition"
        );
        let d = (t.translation.x.powi(2) + t.translation.z.powi(2)).sqrt();
        // Beyond the first field's reach even with the torus cell-diagonal
        // slack (200 + 25·√2 ≈ 235) — only the new field spawns out here.
        if d > 240.0 {
            new_band += 1;
        }
    }
    assert!(
        new_band > 0,
        "the new field's outer band (dist > 240) must have spawned rocks"
    );
}

/// (#924) A single-cell crossing must spawn exactly the newly-entered
/// edge cells, despawn exactly the newly-exited trailing cells, and
/// leave every interior survivor's entity untouched. Before the fix,
/// slot addressing was keyed by offset-from-player: despawn slots were
/// computed against the OLD arena origin, the arena was updated, spawn
/// slots were computed against the NEW origin, and every surviving
/// slot's contents were left addressed by the stale offset — so a
/// one-cell move silently skipped newly-entered edge cells (their slot
/// looked occupied) and left trailing rocks stranded (never despawned).
/// Ring addressing (`cell.rem_euclid(size)`) makes a cell's slot
/// independent of the player's position, so this test would have caught
/// the bug: interior cells must keep the exact same `Entity`, not a
/// respawned one.
#[test]
fn single_cell_crossing_reindexes_only_entered_and_exited_cells() {
    let mut app = test_app();
    let res = 10.0f32;
    let spawn_cells = 4u32;
    let despawn_cells = 4u32;
    // inner_radius 0, huge outer_radius: every cell near the player is
    // eligible, so occupancy across the small area under test is exact
    // and predictable.
    let f = torus_field(0.0, 100_000.0, res, spawn_cells, despawn_cells);

    set_ship_pos(&mut app, 0.0, 0.0);
    app.world_mut()
        .spawn((AsteroidFieldSection(f), Transform::default()));
    app.update(); // full rebuild at grid cell (0, 0)

    let cell_of = |t: &Transform| -> (i32, i32) {
        (
            (t.translation.x / res).round() as i32,
            (t.translation.z / res).round() as i32,
        )
    };

    let before: std::collections::HashMap<(i32, i32), Entity> = {
        let mut q = app.world_mut().query::<(Entity, &Transform, &Asteroid)>();
        q.iter(app.world())
            .map(|(e, t, _)| (cell_of(t), e))
            .collect()
    };
    assert!(!before.is_empty(), "no asteroids spawned before the move");

    // Cross exactly one cell boundary: grid cell (0, 0) -> (1, 0).
    set_ship_pos(&mut app, res, 0.0);
    app.update();

    let after: std::collections::HashMap<(i32, i32), Entity> = {
        let mut q = app.world_mut().query::<(Entity, &Transform, &Asteroid)>();
        q.iter(app.world())
            .map(|(e, t, _)| (cell_of(t), e))
            .collect()
    };

    let dc = despawn_cells as i32;

    // Interior survivors: cells within both the old and new despawn
    // window must be the exact same Entity — no despawn/respawn churn.
    let mut interior_checked = 0;
    for (&(cx, cz), &entity) in &before {
        let in_old_window = cx.abs().max(cz.abs()) <= dc;
        let in_new_window = (cx - 1).abs().max(cz.abs()) <= dc;
        if in_old_window && in_new_window {
            interior_checked += 1;
            assert_eq!(
                after.get(&(cx, cz)),
                Some(&entity),
                "interior survivor cell {:?} churned (despawned/respawned) \
                     across a one-cell move",
                (cx, cz)
            );
        }
    }
    assert!(
        interior_checked > 0,
        "test set-up produced no interior survivor cells to check"
    );

    // Trailing despawn: the old window's leftmost column exits the new
    // window and must be gone.
    let trailing_col = -dc;
    let mut trailing_checked = 0;
    for &(cx, cz) in before.keys() {
        if cx == trailing_col {
            trailing_checked += 1;
            assert!(
                !after.contains_key(&(cx, cz)),
                "trailing cell {:?} should have despawned after the crossing",
                (cx, cz)
            );
        }
    }
    assert!(
        trailing_checked > 0,
        "test set-up produced no trailing cells to check"
    );

    // Edge spawn: the newly-entered spawn column (beyond the old spawn
    // window) must now hold asteroids that did not exist before.
    let entered_col = 1 + spawn_cells as i32;
    let mut edge_checked = 0;
    for &(cx, cz) in after.keys() {
        if cx == entered_col {
            edge_checked += 1;
            assert!(
                !before.contains_key(&(cx, cz)),
                "edge cell {:?} existed before the move — test set-up is wrong",
                (cx, cz)
            );
        }
    }
    assert!(
        edge_checked > 0,
        "no asteroids spawned in the newly-entered edge column"
    );
}

/// (#924) Several sequential one-cell moves must land in the same state
/// as a single full rebuild at the destination. Ring addressing means a
/// cell's slot never depends on the path taken to reach the current
/// player position, so a walk of individual steps and one big jump to
/// the same place must agree exactly — no drift accumulates from
/// repeated incremental deltas.
#[test]
fn multi_step_walk_matches_full_rebuild_at_destination() {
    let res = 10.0f32;
    let spawn_cells = 4u32;
    let despawn_cells = 4u32;
    let field = || torus_field(0.0, 100_000.0, res, spawn_cells, despawn_cells);

    let cells = |app: &mut App| -> std::collections::HashSet<(i32, i32)> {
        let mut q = app.world_mut().query::<(&Transform, &Asteroid)>();
        q.iter(app.world())
            .map(|(t, _)| {
                (
                    (t.translation.x / res).round() as i32,
                    (t.translation.z / res).round() as i32,
                )
            })
            .collect()
    };

    // Walk: five sequential one-cell moves along +x, each within
    // spawn_cells so none of them force a full rebuild on their own.
    let mut walked = test_app();
    set_ship_pos(&mut walked, 0.0, 0.0);
    walked
        .world_mut()
        .spawn((AsteroidFieldSection(field()), Transform::default()));
    walked.update(); // full rebuild at (0, 0)
    for step in 1..=5 {
        set_ship_pos(&mut walked, step as f32 * res, 0.0);
        walked.update();
    }

    // Direct: a single full rebuild landing at the same destination.
    let mut direct = test_app();
    set_ship_pos(&mut direct, 5.0 * res, 0.0);
    direct
        .world_mut()
        .spawn((AsteroidFieldSection(field()), Transform::default()));
    direct.update();

    let walked_cells = cells(&mut walked);
    let direct_cells = cells(&mut direct);
    assert!(!walked_cells.is_empty(), "walk produced no asteroids");
    assert_eq!(
        walked_cells.len(),
        direct_cells.len(),
        "walked population size must match a direct full rebuild at the destination"
    );
    assert_eq!(
        walked_cells, direct_cells,
        "walked cell occupancy must match a direct full rebuild at the destination"
    );
}

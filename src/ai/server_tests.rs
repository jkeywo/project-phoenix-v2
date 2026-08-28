use super::*;

// ── anchors_from_world_config (PRD #337/#338 slice 1) ──────────────────

#[test]
fn anchors_from_world_config_clones_anchor_table() {
    let mut world = crate::world::config::WorldConfig::default();
    world.anchors.insert("alpha".to_string(), [10.0, 0.0, 20.0]);
    world.anchors.insert("beta".to_string(), [-5.0, 1.5, 30.0]);

    let anchors = anchors_from_world_config(&world);
    assert_eq!(anchors.len(), 2);
    assert_eq!(anchors.get("alpha"), Some(&[10.0, 0.0, 20.0]));
    assert_eq!(anchors.get("beta"), Some(&[-5.0, 1.5, 30.0]));
}

#[test]
fn anchors_from_world_config_returns_empty_when_no_anchors() {
    let world = crate::world::config::WorldConfig::default();
    assert!(anchors_from_world_config(&world).is_empty());
}

// ── build_world_snapshot: asteroids as obstacles ───────────────────────

fn snapshot_test_app() -> App {
    let mut app = App::new();
    app.init_resource::<WorldSnapshot>()
        .add_systems(Update, build_world_snapshot);
    app
}

/// The snapshot is an authoritative AI input, so its order must not inherit
/// Bevy's archetype-creation order. `assess_hazards` sums floating-point force
/// contributions in this order; a different order can move a ship by a few
/// ULPs and then diverge the whole seeded run.
#[test]
fn world_snapshot_orders_all_entity_kinds_by_uuid() {
    const LOW: &str = "00000000-0000-4000-8000-000000000001";
    const MID: &str = "00000000-0000-4000-8000-000000000002";
    const HIGH: &str = "00000000-0000-4000-8000-000000000003";

    let mut app = snapshot_test_app();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(HIGH.into()),
        Transform::default(),
    ));
    app.world_mut().spawn((
        crate::server_app::Asteroid,
        crate::server_app::AsteroidUuid(LOW.into()),
        Transform::default(),
        crate::entities::spawner::ColliderSection(crate::entities::config::ColliderConfig {
            shape: crate::entities::config::ColliderShape::Ball,
            radius: 1.0,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
    ));
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(MID.into()),
        Transform::default(),
    ));

    app.update();

    let ids: Vec<_> = app
        .world()
        .resource::<WorldSnapshot>()
        .entities
        .iter()
        .map(|entity| entity.uuid.to_string())
        .collect();
    assert_eq!(ids, [LOW, MID, HIGH]);
}

/// Field asteroids carry `AsteroidUuid`, not `EntityUuid`, because they are
/// streamed rather than spawned through `spawn_entity`. They used to fall
/// out of the snapshot entirely, which left `avoidance_steering` blind to
/// every rock in the field.
#[test]
fn world_snapshot_includes_field_asteroids_with_their_radius() {
    let mut app = snapshot_test_app();
    app.world_mut().spawn((
        crate::server_app::Asteroid,
        crate::server_app::AsteroidUuid(uuid::Uuid::new_v4().to_string()),
        Transform::from_xyz(30.0, 0.0, -12.0),
        crate::entities::spawner::ColliderSection(crate::entities::config::ColliderConfig {
            shape: crate::entities::config::ColliderShape::Ball,
            radius: 4.0,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
    ));

    app.update();

    let snapshot = app.world().resource::<WorldSnapshot>();
    assert_eq!(
        snapshot.entities.len(),
        1,
        "asteroid must reach the snapshot"
    );
    let rock = &snapshot.entities[0];
    assert_eq!(rock.radius, 4.0, "avoidance sizes the obstacle off radius");
    assert_eq!(rock.position, [30.0, 0.0, -12.0]);
    assert_eq!(rock.faction, None, "a rock is hostile to nobody");
    assert_eq!(rock.forward_speed, 0.0);
}

// ── build_world_snapshot: direct-fire reach (issue #788) ───────────────
//
// This is genuinely new CROSS-ENTITY plumbing: before #788 nothing published
// one ship's weapon reach where another ship's AI could read it (the
// long-dead `WorldView.entity_weapons_range` was zeroed by every producer).
// The tests below drive the real system, so a regression that stops
// publishing the field fails here rather than showing up as a destroyer that
// quietly orbits at its authored margin.

/// A hull with one 200-unit phaser bank and one 320-unit blaster bank, plus
/// the components the spawner would attach for them.
fn armed_hull_components() -> (
    crate::console::weapons::PhaserCombatConfigResource,
    crate::console::weapons::BlasterSystemResource,
) {
    let cfg = crate::entities::config::EntityConfig::from_toml(
        // Each bank AUTHORS its open-fire policy: since #885b stage 5d
        // strict AI-declaration mode rejects a bank that declares neither a
        // policy nor an explicit idle, because nothing would be synthesised
        // for it and it would simply never fire.
        //
        // The ship-level `[weapons_console.ai]` doctrine below is owed for
        // the same reason since issue #956, and this fixture is exactly the
        // hull that makes the point: it is ARMED and carries no
        // `[behaviour]`, so gating the doctrine on `[behaviour]` would have
        // let it ship with no arc-bearing preference at all.
        r#"
name = "Armed"
[weapons_console]

[weapons_console.ai]

[[weapons_console.ai.rule]]
priority = 0
channel = "arc_bearing_first"
when = "true"
verb = "bring_phasers_to_bear"

[[weapons_console.ai.rule]]
priority = 0
channel = "arc_bearing_second"
when = "true"
verb = "bring_blasters_to_bear"

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0
fire_arc_deg = 90
auto_arc_deg = 90
beam_range = 200
beam_damage_per_sec = 3
beam_duration_secs = 4
cooldown_secs = 4

[[weapons_console.phaser_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "fire_phaser"

[[weapons_console.blaster_banks]]
id = "lance"
facing_deg = 0
range = 320

[[weapons_console.blaster_banks.ai.rule]]
priority = 0
channel = "blaster_fire"
when = "true"
verb = "fire_blaster"
"#,
    )
    .expect("fixture hull must parse");
    let wc = cfg
        .weapons_console
        .expect("hull declares [weapons_console]");
    (
        crate::console::weapons::PhaserCombatConfigResource(
            crate::entities::config::PhaserCombatConfig::from_weapons_console(&wc),
        ),
        crate::console::weapons::BlasterSystemResource(
            wc.blaster_banks
                .iter()
                .map(|bc| crate::weapons::blaster::BlasterSystem::new(bc.to_runtime()))
                .collect(),
        ),
    )
}

/// The snapshot publishes the LONGEST reach across the entity's direct-fire
/// banks — here the blaster, which outranges the phaser.
#[test]
fn world_snapshot_publishes_the_longest_direct_fire_reach() {
    let mut app = snapshot_test_app();
    let (phasers, blasters) = armed_hull_components();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        phasers,
        blasters,
    ));

    app.update();

    let snapshot = app.world().resource::<WorldSnapshot>();
    assert_eq!(
        snapshot.entities[0].direct_fire_range, 320.0,
        "the reach is the longest bank, not the first or the sum"
    );
}

/// **The reach fact is the authored range, not the radar-scaled one
/// (issue #955).**
///
/// `entity_direct_fire_range` used to multiply each phaser bank's authored
/// `beam_range` by `ModifierSlot::RadarRange`, so the standoff ring another
/// helm derived from this fact shrank and grew with the TARGET's sensor
/// power — a hull resting `sensors` at 1 published two thirds of the reach
/// its guns were credited with. (That power group is gone since #952; the
/// slot survives, driven by radar hull damage and region dampening, and the
/// invariant this test pins survives with it.) Blasters were never scaled, which is what
/// made the two families disagree about the same question.
///
/// The entity here carries a live `ShipModifiers` with the slot crushed to
/// ×0.667 and, in the second half, doubled. The published reach must not
/// move in either direction: the phaser bank reaches 200 and the blaster 320
/// because that is what they author.
///
/// **Both numbers are asserted, per bank, and that is load-bearing.** The
/// scalar `direct_fire_range` is a MAX, and the bug this targets only ever
/// scaled the phaser arm — so on the ×0.667 leg the old coupled code returned
/// 320 too (`200 × 0.667 = 133` loses to the unscaled blaster) and a
/// max-only assertion passed against the very bug it names. The per-bank
/// sectors on `AiWorldEntity::weapon_arcs` are the same
/// `entity_direct_fire_banks` list unreduced, so checking them makes the
/// crushed leg discriminate as sharply as the doubled one.
#[test]
fn direct_fire_reach_ignores_the_radar_range_slot() {
    use crate::core::messages::ModifierSlot;
    use crate::modifiers::cache::ModifierSource;
    use crate::modifiers::{Modifier, ShipModifiers};

    // Written under a radar-DAMAGE source since issue #952 retired the
    // `sensors` power group: the slot's producers are now hull damage and
    // region dampening, and this fixture has to move it the way something
    // real still does. The reach question is unchanged — nothing about how
    // far a gun shoots may depend on this slot, whoever wrote it.
    let radar_slot_at = |bonus: f32| -> ShipModifiers {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(Modifier {
            source: ModifierSource::SystemDamage(
                crate::ship::system_registry::tactical_radar_system_id(),
            ),
            slot: ModifierSlot::RadarRange,
            bonus,
        });
        mods
    };

    for (bonus, label) in [(-0.5f32, "radar crushed"), (1.0, "radar doubled")] {
        let mut app = snapshot_test_app();
        let (phasers, blasters) = armed_hull_components();
        let mods = radar_slot_at(bonus);
        let mult = mods.get(&ModifierSlot::RadarRange);
        assert!(
            (mult - 1.0).abs() > 0.1,
            "{label}: the fixture must actually move the slot or this proves \
             nothing (got x{mult})"
        );
        app.world_mut().spawn((
            crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            phasers,
            blasters,
            mods,
        ));

        app.update();

        let published = &app.world().resource::<WorldSnapshot>().entities[0];
        assert_eq!(
            published.direct_fire_range, 320.0,
            "{label} (RadarRange x{mult:.3}): the published reach must be the \
             authored longest bank. A ring sized off this fact has to describe \
             where the target's guns actually stop, and since #955 that is the \
             number its file authors — nothing scales it"
        );

        // The phaser's own 200, which the max above can never see: it is the
        // arm the retired coupling scaled, and the only reading that fails on
        // the crushed leg if the multiplication comes back.
        let mut per_bank: Vec<f32> = published.weapon_arcs.iter().map(|s| s.range).collect();
        per_bank.sort_by(|a, b| a.partial_cmp(b).expect("no NaN range"));
        assert_eq!(
            per_bank,
            vec![200.0f32, 320.0],
            "{label} (RadarRange x{mult:.3}): the PER-BANK reaches must be the two \
             authored numbers. 200 scaled by this slot is {:.1}, which the scalar \
             `direct_fire_range` above hides behind the unscaled blaster's 320 — so \
             this is where a phaser arm quietly scaled again shows up",
            200.0 * mult
        );
    }
}

/// An offline bank is not a threat, so it must not inflate the ring another
/// ship keeps. With the blaster shot out, the reach falls back to the
/// phaser's; with both gone it is zero.
#[test]
fn an_offline_bank_stops_counting_toward_direct_fire_reach() {
    let mut app = snapshot_test_app();
    let (phasers, blasters) = armed_hull_components();
    let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
    sources.0.set_offline(
        crate::ship::system_registry::blaster_bank_system_id("lance").unwrap(),
        true,
    );
    let entity = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            phasers,
            blasters,
            sources,
        ))
        .id();

    app.update();
    assert_eq!(
        app.world().resource::<WorldSnapshot>().entities[0].direct_fire_range,
        200.0,
        "a disabled blaster bank must drop out of the reach"
    );

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>()
        .unwrap()
        .0
        .set_offline(
            crate::ship::system_registry::phaser_bank_system_id("fore").unwrap(),
            true,
        );
    app.update();
    assert_eq!(
        app.world().resource::<WorldSnapshot>().entities[0].direct_fire_range,
        0.0,
        "a fully disarmed ship has no reach at all — the ring collapses to the \
         standing-off hull's own authored margin"
    );
}

// ── build_world_snapshot: hostile weapon-arc sectors (issue #874) ──────

/// A ship heading `yaw` radians, stated the way the SIMULATION states it
/// (issue #937).
///
/// `ShipPhysics.yaw` is the authority for anything that moves, and its
/// convention is clockwise (0 = facing −Z, so a heading θ points along
/// `(sin θ, −cos θ)` — the convention `arc_geometry::world_bearing_deg`
/// resolves bearings in). The `Transform` a ship carries is the RENDER pose
/// and holds the negation, because `sync_ship_position` writes
/// `Quat::from_euler(YXZ, -physics.yaw, …)` and Bevy's Y euler turns the
/// other way.
///
/// The fixtures below used to hand-roll the Transform alone and assert on
/// what came out, which pinned the render convention onto a field every
/// consumer reads in the simulation one. Building both here, from one yaw
/// and through the same negation the real sync applies, is what makes these
/// tests a statement about a ship rather than about a quaternion.
fn hull_pose(yaw: f32) -> (Transform, crate::ship::state::ShipPhysics) {
    (
        Transform::from_xyz(0.0, 0.0, 0.0).with_rotation(Quat::from_rotation_y(-yaw)),
        crate::ship::state::ShipPhysics {
            yaw,
            ..Default::default()
        },
    )
}

/// **The snapshot's `yaw` is the SIMULATION's heading, not the render
/// pose's (issue #937).**
///
/// Every consumer of `AiWorldEntity::yaw` reconstructs a forward vector as
/// `(sin θ, −cos θ)` — `ai::core::target_relative_motion` for both the helm's
/// `closing_rate` and the captain's hostile range, both avoidance
/// projections in `ai::core`, and `arc_geometry::weapon_arc_sectors`, whose
/// output is compared against `world_bearing_deg`'s `atan2(dx, −dz)`. That
/// is `ShipPhysics.yaw`'s convention and nothing else's.
///
/// The render `Transform` holds the NEGATION of it, because
/// `sync_ship_position` writes `Quat::from_euler(YXZ, −physics.yaw, …)` and
/// Bevy's Y euler turns the other way. Reading the euler straight back — as
/// this producer did — published every ship's heading mirrored, which is a
/// silent failure: the field is still present, still finite, still moves
/// when the ship turns, and every test that built its fixture from a
/// hand-rolled quaternion still agreed with it.
///
/// So this pin is deliberately NOT "the number equals the number". It runs
/// the ship through the REAL `sync_ship_position`, then asserts on the
/// FORWARD VECTOR the snapshot implies — against the direction the physics
/// integrator would actually carry the hull. A sign flip cannot survive
/// that, and neither can a future change of either convention that forgets
/// the other.
///
/// What it cost in play is
/// `headless_runner::the_composed_destroyer_passes_breaks_off_and_passes_again`:
/// a mirrored target velocity made `closing_rate` read "still closing" for a
/// destroyer that had already flown past its target, so the attack pass's
/// closest-approach detector never fired and the hull ground along at
/// contact range instead of breaking off.
#[test]
fn the_snapshot_publishes_headings_in_the_simulations_own_convention() {
    // Four headings rather than one: a sign flip is invisible at 0 and at
    // pi, which are exactly the two a single-case fixture reaches for.
    for yaw in [0.7_f32, -1.9, 2.6, std::f32::consts::FRAC_PI_2] {
        let mut app = snapshot_test_app();
        app.add_systems(
            Update,
            crate::ship::physics_systems::sync_ship_position.before(build_world_snapshot),
        );
        app.world_mut().spawn((
            crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
            Transform::default(),
            crate::ship::state::ShipPhysics {
                yaw,
                forward_speed: 10.0,
                ..Default::default()
            },
        ));
        app.update();

        let e = &app.world().resource::<WorldSnapshot>().entities[0];
        let published = e.yaw.expect("a ship publishes a heading");
        // The direction the integrator carries the hull, straight off
        // `ShipPhysics` (see `ship_physics::compute_physics`).
        let (truth_x, truth_z) = (crate::simmath::sin(yaw), -crate::simmath::cos(yaw));
        // The direction every consumer reconstructs from the published
        // heading.
        let (read_x, read_z) = (
            crate::simmath::sin(published),
            -crate::simmath::cos(published),
        );
        assert!(
            (read_x - truth_x).abs() < 1e-4 && (read_z - truth_z).abs() < 1e-4,
            "yaw {yaw}: the snapshot published {published}, which reads as \
             forward ({read_x:.3}, {read_z:.3}) — the hull actually travels \
             ({truth_x:.3}, {truth_z:.3}). A mirrored heading makes every \
             relative-velocity and weapon-arc reading in the AI wrong \
             without making any of them absent."
        );
    }
}

/// AC2: the arcs are published for every armed entity, with no scan gate
/// and no target involved — a hull's arcs are a fact about the hull.
#[test]
fn world_snapshot_publishes_world_bearing_weapon_arc_sectors() {
    let mut app = snapshot_test_app();
    let (phasers, blasters) = armed_hull_components();
    // Yawed 90 degrees to starboard, so a forward bank bears on +X.
    let (transform, physics) = hull_pose(std::f32::consts::FRAC_PI_2);
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        transform,
        physics,
        phasers,
        blasters,
    ));

    app.update();

    let arcs = &app.world().resource::<WorldSnapshot>().entities[0].weapon_arcs;
    assert_eq!(arcs.len(), 2, "one sector per direct-fire bank: {arcs:?}");
    for a in arcs {
        assert!(
            (a.bearing_deg - 90.0).abs() < 1e-3,
            "yaw 90 + facing 0 must bear 90: {a:?}"
        );
    }
    assert!((arcs[0].half_angle_deg - 45.0).abs() < 1e-3, "{arcs:?}");
    assert!((arcs[0].range - 200.0).abs() < 1e-3, "phaser reach");
    assert!((arcs[1].range - 320.0).abs() < 1e-3, "blaster reach");
}

/// An offline bank is not a threat: it drops out of the sectors exactly as
/// it drops out of the reach, because both are projections of one list.
#[test]
fn an_offline_bank_stops_publishing_its_arc_sector() {
    let mut app = snapshot_test_app();
    let (phasers, blasters) = armed_hull_components();
    let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
    sources.0.set_offline(
        crate::ship::system_registry::blaster_bank_system_id("lance").unwrap(),
        true,
    );
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        phasers,
        blasters,
        sources,
    ));

    app.update();

    let arcs = &app.world().resource::<WorldSnapshot>().entities[0].weapon_arcs;
    assert_eq!(arcs.len(), 1, "the disabled blaster arc must go: {arcs:?}");
    assert!((arcs[0].range - 200.0).abs() < 1e-3, "the phaser remains");
}

#[test]
fn an_unarmed_entity_and_an_asteroid_publish_no_arc_sectors() {
    let mut app = snapshot_test_app();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app.world_mut().spawn((
        crate::server_app::Asteroid,
        crate::server_app::AsteroidUuid(uuid::Uuid::new_v4().to_string()),
        Transform::from_xyz(30.0, 0.0, -12.0),
        crate::entities::spawner::ColliderSection(crate::entities::config::ColliderConfig {
            shape: crate::entities::config::ColliderShape::Ball,
            radius: 4.0,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
    ));
    app.update();
    for e in &app.world().resource::<WorldSnapshot>().entities {
        assert!(e.weapon_arcs.is_empty(), "{e:?}");
    }
}

/// AC4, Rust half: the AI fact reduction and the wire payload derive from
/// the SAME producer call.
///
/// Both consumers are exercised against one `build_world_snapshot` run:
/// the wire conversion the helm blackboard performs, and
/// `crate::ai::hostile_arc_exposure`, the reduction the helm facts are
/// seeded from. The assertion is elementwise identity — not "both look
/// plausible" — so a future change that gave either side its own geometry
/// would fail here rather than drift silently.
#[test]
fn the_wire_payload_and_the_ai_fact_reduction_read_the_same_sectors() {
    let mut app = snapshot_test_app();
    let (phasers, blasters) = armed_hull_components();
    let hostile_faction = uuid::Uuid::new_v4();
    let own_faction = uuid::Uuid::new_v4();
    let (transform, physics) = hull_pose(std::f32::consts::FRAC_PI_2);
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        transform,
        physics,
        crate::entities::spawner::FactionComponent(hostile_faction),
        phasers,
        blasters,
    ));
    app.update();

    let snapshot_entity = app.world().resource::<WorldSnapshot>().entities[0].clone();
    assert!(!snapshot_entity.weapon_arcs.is_empty());

    // (a) The wire payload the helm blackboard builds.
    let wire: Vec<crate::core::messages::HostileWeaponArc> =
        snapshot_entity.weapon_arcs.iter().map(Into::into).collect();

    // (b) The reduction the helm facts are seeded from, over the same
    //     snapshot entry. Observer 100 units to +X — inside the yawed
    //     hull's forward arcs.
    let mut registry = crate::ai::faction::FactionRegistry::new();
    registry.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: own_faction,
        name: "Own".into(),
        enemies: vec![hostile_faction],
        compliance: None,
    });
    let view = crate::ai::WorldView {
        entity_pos: [100.0, 0.0, 0.0],
        self_faction: Some(own_faction),
        entities: vec![snapshot_entity.clone()],
        ..Default::default()
    };
    let exposure = crate::ai::hostile_arc_exposure(&view, &registry);

    // Same sectors, elementwise: the wire is a verbatim copy.
    assert_eq!(wire.len(), snapshot_entity.weapon_arcs.len());
    for (w, s) in wire.iter().zip(snapshot_entity.weapon_arcs.iter()) {
        assert_eq!(w.bearing_deg, s.bearing_deg);
        assert_eq!(w.half_angle_deg, s.half_angle_deg);
        assert_eq!(w.range, s.range);
    }
    // And the reduction is a reduction of exactly those sectors: rebuilding
    // it from the WIRE arcs reproduces the fact the policy reads.
    let from_wire = crate::weapons::arc_geometry::arc_exposure(
        &wire
            .iter()
            .map(|w| crate::weapons::arc_geometry::WeaponArcSector {
                bearing_deg: w.bearing_deg,
                half_angle_deg: w.half_angle_deg,
                range: w.range,
            })
            .collect::<Vec<_>>(),
        snapshot_entity.position[0],
        snapshot_entity.position[2],
        100.0,
        0.0,
    );
    assert_eq!(from_wire, exposure);
    assert_eq!(exposure.covering_count, 2, "both banks bear: {exposure:?}");
}

/// A friendly ship's arcs are published on the snapshot (they are hull
/// facts) but must not read as exposure — the reduction is hostility-gated.
#[test]
fn a_friendly_ships_arcs_are_not_exposure() {
    let same_faction = uuid::Uuid::new_v4();
    let mut app = snapshot_test_app();
    let (phasers, blasters) = armed_hull_components();
    let (transform, physics) = hull_pose(std::f32::consts::FRAC_PI_2);
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        transform,
        physics,
        crate::entities::spawner::FactionComponent(same_faction),
        phasers,
        blasters,
    ));
    app.update();
    let entity = app.world().resource::<WorldSnapshot>().entities[0].clone();
    assert!(!entity.weapon_arcs.is_empty(), "arcs are still published");

    let mut registry = crate::ai::faction::FactionRegistry::new();
    registry.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: same_faction,
        name: "Own".into(),
        enemies: vec![],
        compliance: None,
    });
    let view = crate::ai::WorldView {
        entity_pos: [100.0, 0.0, 0.0],
        self_faction: Some(same_faction),
        entities: vec![entity],
        ..Default::default()
    };
    assert_eq!(
        crate::ai::hostile_arc_exposure(&view, &registry).covering_count,
        0
    );
}

/// An unarmed entity — and an asteroid — publish no reach, rather than a
/// default one.
#[test]
fn an_unarmed_entity_publishes_no_direct_fire_reach() {
    let mut app = snapshot_test_app();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app.update();
    assert_eq!(
        app.world().resource::<WorldSnapshot>().entities[0].direct_fire_range,
        0.0
    );
}

#[test]
fn world_snapshot_carries_both_entities_and_asteroids() {
    let mut app = snapshot_test_app();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    app.world_mut().spawn((
        crate::server_app::Asteroid,
        crate::server_app::AsteroidUuid(uuid::Uuid::new_v4().to_string()),
        Transform::from_xyz(50.0, 0.0, 0.0),
        crate::entities::spawner::ColliderSection(crate::entities::config::ColliderConfig {
            shape: crate::entities::config::ColliderShape::Ball,
            radius: 2.5,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
    ));

    app.update();

    let snapshot = app.world().resource::<WorldSnapshot>();
    assert_eq!(snapshot.entities.len(), 2);
    assert!(
        snapshot.entities.iter().any(|e| e.radius == 2.5),
        "the asteroid pass must not replace the entity pass"
    );
}

/// Issue #958: the `movable` fact the hazard rule keys on is the entity's
/// AUTHORED `[collider] movable`, not "everything spawned through
/// `spawn_entity` is a ship". A station and a planet come through the same
/// `EntityUuid` query a hull does; before this, all three published
/// `movable: true`, which put static terrain on the ignorable side of the
/// ignore-smaller rule.
#[test]
fn world_snapshot_publishes_authored_mobility_not_the_query_arm() {
    let mut app = snapshot_test_app();
    let ball = |radius: f32, movable: bool| {
        crate::entities::spawner::ColliderSection(crate::entities::config::ColliderConfig {
            shape: crate::entities::config::ColliderShape::Ball,
            radius,
            length: 0.0,
            half_height: None,
            movable,
        })
    };
    // A hull: authored mobile.
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        crate::entities::spawner::EntityName("hull".into()),
        Transform::from_xyz(0.0, 0.0, 0.0),
        ball(5.0, true),
    ));
    // A station: same query arm, authored static.
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        crate::entities::spawner::EntityName("station".into()),
        Transform::from_xyz(100.0, 0.0, 0.0),
        ball(12.0, false),
    ));
    // An entity with no collider at all falls back to the same static
    // default the TOML parser uses.
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid(uuid::Uuid::new_v4().to_string()),
        crate::entities::spawner::EntityName("bare".into()),
        Transform::from_xyz(200.0, 0.0, 0.0),
    ));

    app.update();

    let snapshot = app.world().resource::<WorldSnapshot>();
    let movable_of = |name: &str| {
        snapshot
            .entities
            .iter()
            .find(|e| e.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{name} must be in the snapshot"))
            .movable
    };
    assert!(movable_of("hull"), "an authored hull is a mobile contact");
    assert!(
        !movable_of("station"),
        "a station must publish as static terrain, not as a ship"
    );
    assert!(
        !movable_of("bare"),
        "an entity with no [collider] must not claim mobility"
    );
}

// ── AiTokenRegistry unit tests ─────────────────────────────────────────

#[test]
fn register_produces_ai_prefixed_token() {
    let mut reg = AiTokenRegistry::new();
    reg.register("abc-123");
    assert_eq!(reg.token_for_entity("abc-123"), Some("ai:abc-123"));
}

#[test]
fn register_is_idempotent() {
    let mut reg = AiTokenRegistry::new();
    reg.register("abc-123");
    reg.register("abc-123");
    assert_eq!(reg.token_for_entity("abc-123"), Some("ai:abc-123"));
    // Exactly one reverse entry
    assert_eq!(reg.entity_uuid_for_token("ai:abc-123"), Some("abc-123"));
}

#[test]
fn entity_uuid_for_token_returns_none_for_player_token() {
    let reg = AiTokenRegistry::new();
    assert!(reg.entity_uuid_for_token("some-player-uuid").is_none());
}

#[test]
fn entity_uuid_for_token_round_trips() {
    let mut reg = AiTokenRegistry::new();
    reg.register("ent-999");
    assert_eq!(reg.entity_uuid_for_token("ai:ent-999"), Some("ent-999"));
}

#[test]
fn unregister_removes_both_directions() {
    let mut reg = AiTokenRegistry::new();
    reg.register("ent-1");
    reg.unregister("ent-1");
    assert!(reg.token_for_entity("ent-1").is_none());
    assert!(reg.entity_uuid_for_token("ai:ent-1").is_none());
}

#[test]
fn unregister_unknown_entity_is_silent() {
    let mut reg = AiTokenRegistry::new();
    reg.unregister("ghost-uuid"); // must not panic
}

#[test]
fn contains_entity_returns_true_after_register() {
    let mut reg = AiTokenRegistry::new();
    reg.register("ent-x");
    assert!(reg.contains_entity("ent-x"));
}

#[test]
fn contains_entity_returns_false_after_unregister() {
    let mut reg = AiTokenRegistry::new();
    reg.register("ent-x");
    reg.unregister("ent-x");
    assert!(!reg.contains_entity("ent-x"));
}

#[test]
fn multiple_entities_registered_independently() {
    let mut reg = AiTokenRegistry::new();
    reg.register("alpha");
    reg.register("beta");
    reg.register("gamma");
    assert_eq!(reg.token_for_entity("alpha"), Some("ai:alpha"));
    assert_eq!(reg.token_for_entity("beta"), Some("ai:beta"));
    assert_eq!(reg.token_for_entity("gamma"), Some("ai:gamma"));
}

#[test]
fn unregistering_one_does_not_affect_others() {
    let mut reg = AiTokenRegistry::new();
    reg.register("alpha");
    reg.register("beta");
    reg.unregister("alpha");
    assert!(reg.token_for_entity("alpha").is_none());
    assert_eq!(reg.token_for_entity("beta"), Some("ai:beta"));
}

// ── Bevy integration tests ─────────────────────────────────────────────

use crate::core::messages::GamePhase;
use crate::entities::config::BehaviourConfig;
use crate::entities::config_cache::FactionRegistryResource;
use crate::entities::spawner::EntityUuid;
use crate::lobby::LobbyPlugin;

#[derive(Resource, Default)]
struct AttackedBox(Vec<AiEntityAttacked>);
#[derive(Resource, Default)]
struct DestroyedBox(Vec<AiEntityDestroyed>);

fn collect_attacked(mut r: MessageReader<AiEntityAttacked>, mut b: ResMut<AttackedBox>) {
    for e in r.read() {
        b.0.push(e.clone());
    }
}
fn collect_destroyed(mut r: MessageReader<AiEntityDestroyed>, mut b: ResMut<DestroyedBox>) {
    for e in r.read() {
        b.0.push(e.clone());
    }
}

fn build_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(AiPlugin)
        .insert_resource(FactionRegistryResource(
            crate::entities::config_cache::get_faction_registry(),
        ))
        .init_resource::<AttackedBox>()
        .init_resource::<DestroyedBox>()
        .add_systems(PostUpdate, (collect_attacked, collect_destroyed));
    // One fixed step per update (issue #895): AiPlugin's systems run on
    // the logical tick, and each harness tick advances it once.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    app
}

fn spawn_behaviour_entity(app: &mut App, uuid: &str) -> Entity {
    app.world_mut()
        .spawn((
            Transform::from_xyz(1.0, 0.0, 2.0),
            EntityUuid(uuid.to_string()),
            BehaviourSection(BehaviourConfig::default()),
        ))
        .id()
}

#[test]
fn token_registered_after_spawn() {
    let mut app = build_test_app();
    spawn_behaviour_entity(&mut app, "ent-003");
    app.update();
    let reg = app.world().resource::<AiTokenRegistry>();
    assert!(reg.contains_entity("ent-003"), "entity must be registered");
    assert_eq!(reg.token_for_entity("ent-003"), Some("ai:ent-003"));
}

#[test]
fn token_unregistered_after_entity_despawn() {
    let mut app = build_test_app();
    let entity = spawn_behaviour_entity(&mut app, "ent-007");
    app.update();
    // Verify registered
    assert!(app
        .world()
        .resource::<AiTokenRegistry>()
        .contains_entity("ent-007"));
    // Despawn
    app.world_mut().despawn(entity);
    app.update();
    assert!(
        !app.world()
            .resource::<AiTokenRegistry>()
            .contains_entity("ent-007"),
        "token must be unregistered after despawn"
    );
}

// ── AiEntityAttacked event ─────────────────────────────────────────────
//
// Post-#702 the rising edge lives on `LastShipAttacker`'s change detection
// rather than on a private `AiMemory.last_attacker` mirror, so these drive
// that component. They pin the *reader's* half of the exactly-once
// contract: given a writer that compares before writing, the emitter fires
// once per new attacker. The writer's half — that `tick_beams` really does
// compare rather than blind-write under a live beam — is pinned by
// `sustained_beam_marks_last_attacker_changed_exactly_once` in
// `console::weapons`. Both halves are required; neither alone
// establishes the AC.

/// Write `LastShipAttacker` the way `tick_beams` does — via `set_if_neq`,
/// not `insert`. The distinction is the whole point: an `insert` marks the
/// component changed even when the value is identical, so a fixture that
/// inserted would fake an edge production never produces and this test
/// would pass on a blind-writing `tick_beams`.
fn beam_hit(app: &mut App, entity: Entity, attacker: &str) {
    let mut e = app.world_mut().entity_mut(entity);
    let mut last = e
        .get_mut::<crate::console::weapons::LastShipAttacker>()
        .expect("ship must carry LastShipAttacker");
    last.set_if_neq(crate::console::weapons::LastShipAttacker(Some(
        attacker.to_string(),
    )));
}

fn attacked_count(app: &App, entity_uuid: &str) -> usize {
    app.world()
        .resource::<AttackedBox>()
        .0
        .iter()
        .filter(|e| e.entity_uuid == entity_uuid)
        .count()
}

#[test]
fn ai_entity_attacked_event_emitted_when_new_attacker_arrives() {
    let mut app = build_test_app();
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    let attacker_id = "aaaaaaaa-0000-0000-0000-000000000099";
    let entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-attacked-001".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            crate::console::weapons::LastShipAttacker::default(),
        ))
        .id();

    app.update(); // attach controller; no attacker yet
    beam_hit(&mut app, entity, attacker_id);
    app.update(); // the change is seen — emits AiEntityAttacked

    let events = app.world().resource::<AttackedBox>().0.clone();
    let event = events
        .iter()
        .find(|e| e.entity_uuid == "ent-attacked-001")
        .expect("AiEntityAttacked must be emitted when a new attacker arrives");
    assert_eq!(
        event.attacker_uuid,
        uuid::Uuid::parse_str(attacker_id).unwrap(),
        "the event must name the attacker LastShipAttacker records"
    );
}

/// Sustained fire: the beam keeps naming the same shooter every tick, and
/// the trigger must fire exactly once.
#[test]
fn ai_entity_attacked_not_re_emitted_for_same_attacker() {
    let mut app = build_test_app();
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    let attacker_id = "aaaaaaaa-0000-0000-0000-000000000088";
    let entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-attacked-002".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            crate::console::weapons::LastShipAttacker::default(),
        ))
        .id();

    app.update(); // attach

    // Five ticks of a live beam from one shooter.
    for _ in 0..5 {
        beam_hit(&mut app, entity, attacker_id);
        app.update();
    }

    assert_eq!(
        attacked_count(&app, "ent-attacked-002"),
        1,
        "sustained fire from one shooter must emit AiEntityAttacked exactly once"
    );
}

/// The other edge: a *different* shooter is a new attacker and must re-fire,
/// even though `LastShipAttacker` was already `Some`. Guards against a fix
/// for the test above that latches on "was ever attacked" instead of "who".
#[test]
fn ai_entity_attacked_re_emitted_for_a_different_attacker() {
    let mut app = build_test_app();
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    let first = "aaaaaaaa-0000-0000-0000-000000000077";
    let second = "bbbbbbbb-0000-0000-0000-000000000077";
    let entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-attacked-003".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            crate::console::weapons::LastShipAttacker::default(),
        ))
        .id();

    app.update();
    beam_hit(&mut app, entity, first);
    app.update();
    beam_hit(&mut app, entity, second);
    app.update();

    assert_eq!(
        attacked_count(&app, "ent-attacked-003"),
        2,
        "a second, different attacker is a new edge and must re-emit"
    );
}

/// Clearing the attacker (`clear_last_attacker_on_death` /
/// `clear_last_attacker_on_red_alert_off` both write `None`) marks the
/// component changed, but `None` names nobody and must not be reported as
/// an attack.
#[test]
fn clearing_the_attacker_emits_no_attacked_event() {
    let mut app = build_test_app();
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    let entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-attacked-004".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            crate::console::weapons::LastShipAttacker::default(),
        ))
        .id();

    app.update();
    beam_hit(&mut app, entity, "aaaaaaaa-0000-0000-0000-000000000066");
    app.update();

    // The threat passes — the attacker record is cleared.
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::console::weapons::LastShipAttacker(None));
    app.update();

    assert_eq!(
        attacked_count(&app, "ent-attacked-004"),
        1,
        "clearing LastShipAttacker to None must not count as an attack"
    );
}

// ── Issue #314: WorldView population from components ───────────────────

fn make_weapons_console_config(beam_range: f32) -> crate::entities::config::WeaponsConsoleConfig {
    crate::entities::config::WeaponsConsoleConfig {
        torpedo_arc_color: vec![],
        power_multipliers: None,
        phaser_banks: vec![crate::entities::config::PhaserBankConfig {
            id: "fore".into(),
            facing_deg: 0.0,
            fire_arc_deg: 360.0,
            auto_arc_deg: 360.0,
            beam_range,
            beam_damage_per_sec: 5.0,
            beam_duration_secs: 3.0,
            cooldown_secs: 3.0,
            beam_color: vec![],
            shield_pierce: Some(0.0),
            marker: None,
            ai: None,
            cycle_jitter: 0.0,
        }],
        blaster_banks: vec![],
        radar: None,
        selector: None,
        selector_idle: false,
        ai: None,
    }
}

#[test]
fn self_hull_fraction_reflects_entity_console_hull() {
    use crate::core::messages::SystemId;
    use crate::entities::spawner::EntitySystemHull;
    use crate::ship::damage::SystemHull;

    let mut app = build_test_app();
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    // 50 HP out of 100 HP = 0.5 fraction
    let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);
    let mut rng = crate::sim_rng::unseeded_test_rng();
    hull.apply_damage(50.0, &mut rng);

    let entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-hull-frac-001".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            EntitySystemHull(hull),
        ))
        .id();

    app.update(); // attach controller
    app.update(); // tick

    // The hull fraction should be ~0.5; we verify via the world_view that was
    // used internally by confirming the EntitySystemHull component is readable.
    let hull_comp = app.world().get::<EntitySystemHull>(entity).unwrap();
    let frac = hull_comp.0.total_current() / hull_comp.0.total_max();
    assert!(
        (frac - 0.5).abs() < 0.01,
        "hull fraction should be ~0.5, got {frac}"
    );
}

#[test]
fn npc_beam_ready_true_when_active_beam_inactive_and_no_cooldown() {
    use crate::console::weapons::{ActiveBeam, PhaserCooldown};
    use crate::entities::spawner::WeaponsConsoleSection;

    let mut app = build_test_app();
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    let entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-phaser-002".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            WeaponsConsoleSection(make_weapons_console_config(40.0)),
            ActiveBeam::default(),
            PhaserCooldown::default(),
        ))
        .id();

    app.update(); // attach controller + first tick
    app.update(); // second tick runs the world_view logic

    let beam = app.world().get::<ActiveBeam>(entity).unwrap();
    let cd = app.world().get::<PhaserCooldown>(entity).unwrap();
    assert!(!beam.is_firing(), "beam must not be active");
    assert!(!cd.is_bank_active("fore"), "cooldown must be 0");
}

#[test]
fn npc_beam_ready_false_when_cooldown_active() {
    use crate::console::weapons::{ActiveBeam, PhaserCooldown};
    use crate::entities::spawner::WeaponsConsoleSection;

    let mut app = build_test_app();
    app.world_mut()
        .insert_resource(State::new(GamePhase::InProgress));

    let mut cooldown = PhaserCooldown::default();
    cooldown.start_bank("fore", 5.0);

    let entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.0),
            EntityUuid("ent-phaser-003".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            WeaponsConsoleSection(make_weapons_console_config(40.0)),
            ActiveBeam::default(),
            cooldown,
        ))
        .id();

    app.update();

    let cd = app.world().get::<PhaserCooldown>(entity).unwrap();
    assert!(
        cd.is_bank_active("fore"),
        "phaser must not be ready when bank cooldown is active"
    );
}

#[test]
fn weapons_console_section_attached_when_config_has_weapons_console() {
    use crate::entities::config::EntityConfig;
    use crate::entities::spawner::WeaponsConsoleSection;

    let mut app = build_test_app();

    // Build a minimal EntityConfig with a weapons_console section.
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        hull: None,
        weapons_console: Some(make_weapons_console_config(80.0)),
        behaviour: None,
        helm_console: None,
        helm_capability: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        collider: None,
        appearance: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        tags: vec![],
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        radar_appearance: None,
        mesh: None,
        target: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };

    let mut commands = app.world_mut().commands();
    let entity = crate::entities::spawner::spawn_entity(
        &mut commands,
        &config,
        bevy::math::Vec3::ZERO,
        "ent-spawner-weapons-001".to_string(),
        None,
    );
    app.world_mut().flush();

    let wc = app.world().get::<WeaponsConsoleSection>(entity);
    assert!(
        wc.is_some(),
        "WeaponsConsoleSection must be attached when config has weapons_console"
    );
    assert!(
        wc.unwrap()
            .0
            .phaser_banks
            .first()
            .map(|b| (b.beam_range - 80.0).abs() < 0.01)
            .unwrap_or(false),
        "beam_range must match config"
    );
}

// ── PRD #307: FactionRegistryResource must be accessible as Res (not Option) ──

/// A minimal system that takes `Res<FactionRegistryResource>` (non-Option).
/// If the resource is not present, Bevy panics with a missing-resource error.
/// This test verifies that `build_test_app` — which calls `insert_faction_registry`
/// via the unconditional path — makes the resource available on native.
fn read_faction_registry_system(reg: Res<FactionRegistryResource>) {
    // Just accessing it is enough — the test verifies the resource exists.
    let _ = &reg.0;
}

// ── aggregate_doctrine_blackboards ────────────────────────────────────────

/// `aggregate_doctrine_blackboards` must write a `ViewscreenBlackboard` with
/// at least one `ScoredObjective` carrying `SystemAffinity::Helm` for an
/// entity whose `BehaviourSection` contains a `Patrol` doctrine entry.
/// This is the gate the per-axis helm AI checks (`has_helm_objective`);
/// without it the ship stays still even when Backfill AI is active.
#[test]
fn aggregate_doctrine_blackboards_writes_scored_helm_objective() {
    use crate::core::messages::{SystemAffinity, SystemId};
    use crate::entities::config::{BehaviourConfig, DoctrineObjective};
    use crate::entities::spawner::EntitySystemHull;
    use crate::server_app::ShipSystemBlackboards;
    use crate::ship::damage::SystemHull;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    let mut app = build_test_app();

    let behaviour = BehaviourConfig {
        doctrine: vec![DoctrineObjective {
            id: "patrol-test".into(),
            text: "Patrol test route".into(),
            directive_kind: Some("Patrol".into()),
            base_priority: 30.0,
            directive_loop: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    let hull = EntitySystemHull(SystemHull::from_config(&[(
        SystemId("captain".into()),
        100.0,
    )]));

    app.world_mut().spawn((
        BehaviourSection(behaviour),
        hull,
        ShipSystemBlackboards::default(),
    ));

    app.update();

    let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
    let bb = q
        .iter(app.world())
        .next()
        .expect("entity must have ShipSystemBlackboards");

    let viewscreen =
        bb.0.get(&crate::core::messages::SystemId(
            VIEWSCREEN_SYSTEM_ID.to_string(),
        ))
        .expect("viewscreen entry must be present after aggregate_doctrine_blackboards");

    let scored = match viewscreen {
        crate::core::messages::SystemBlackboard::Viewscreen(v) => &v.scored_objectives,
        _ => panic!("expected Viewscreen blackboard"),
    };

    assert!(
        !scored.is_empty(),
        "scored_objectives must not be empty for a Patrol doctrine entity"
    );
    assert!(
        scored
            .iter()
            .any(|o| o.score > 0.0 && o.relevance.contains(&SystemAffinity::Helm)),
        "at least one scored objective must carry SystemAffinity::Helm"
    );
}

/// A `StaticPointDefence` platform (the station) carries NO `[behaviour]`, so
/// it used to fall outside `aggregate_doctrine_blackboards`' query and never
/// got a Viewscreen blackboard — and `ai_phaser_auto_fire` aims at the
/// Viewscreen `combat_lock`, so the station's Tactical lock was consumed by
/// nothing and it never fired. It must now get a Viewscreen entry whose
/// `combat_lock` mirrors its tactical radar's `selected_target`.
#[test]
fn aggregate_publishes_combat_lock_for_a_behaviourless_point_defence() {
    use crate::core::messages::{SystemBlackboard, SystemId, TacticalRadarBlackboard};
    use crate::entities::spawner::{EntitySystemHull, StaticPointDefence};
    use crate::server_app::ShipSystemBlackboards;
    use crate::ship::damage::SystemHull;
    use crate::ship::system_registry::{tactical_radar_system_id, VIEWSCREEN_SYSTEM_ID};

    let mut app = build_test_app();

    let locked = uuid::Uuid::new_v4().to_string();
    let mut blackboards = ShipSystemBlackboards::default();
    blackboards.0.insert(
        tactical_radar_system_id(),
        SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
            selected_target: Some(locked.clone()),
            ..Default::default()
        }),
    );
    let hull = EntitySystemHull(SystemHull::from_config(&[(
        SystemId("captain".into()),
        100.0,
    )]));

    // No `BehaviourSection`: a static point-defence platform with no doctrine.
    app.world_mut()
        .spawn((StaticPointDefence, hull, blackboards));

    app.update();

    let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
    let bb = q
        .iter(app.world())
        .next()
        .expect("entity must have ShipSystemBlackboards");
    let viewscreen =
        bb.0.get(&SystemId(VIEWSCREEN_SYSTEM_ID.to_string()))
            .expect("a StaticPointDefence must get a Viewscreen entry with no behaviour");
    let combat_lock = match viewscreen {
        SystemBlackboard::Viewscreen(v) => v.combat_lock.clone(),
        _ => panic!("expected Viewscreen blackboard"),
    };
    assert_eq!(
        combat_lock.as_deref(),
        Some(locked.as_str()),
        "the station's Viewscreen combat_lock must mirror its tactical radar selected_target"
    );
}

/// Publish a viewscreen pool for one entity and hand back its
/// `scored_objectives`.
fn scored_pool_for(
    behaviour: crate::entities::config::BehaviourConfig,
    hull_current: f32,
    hull_max: f32,
) -> Vec<crate::core::messages::ScoredObjective> {
    use crate::core::messages::SystemId;
    use crate::entities::spawner::EntitySystemHull;
    use crate::server_app::ShipSystemBlackboards;
    use crate::ship::damage::SystemHull;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    let mut app = build_test_app();

    let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), hull_max)]);
    hull.set_hp(&SystemId("captain".into()), hull_current);

    app.world_mut().spawn((
        BehaviourSection(behaviour),
        EntitySystemHull(hull),
        ShipSystemBlackboards::default(),
    ));
    app.update();

    let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
    let bb = q.iter(app.world()).next().expect("blackboards").clone();
    match bb
        .0
        .get(&crate::core::messages::SystemId(
            VIEWSCREEN_SYSTEM_ID.to_string(),
        ))
        .expect("viewscreen entry")
    {
        crate::core::messages::SystemBlackboard::Viewscreen(v) => v.scored_objectives.clone(),
        _ => panic!("expected Viewscreen blackboard"),
    }
}

/// A `Retreat` doctrine entry gated on `hull_below` scores only once the
/// ship is actually hurt (issue #702).
///
/// This is the replacement for the engine's synthetic hull-triggered
/// Retreat, which `aggregate_doctrine_blackboards` used to inject below a
/// `[behaviour] retreat_hull_threshold`. That mechanism was inert in
/// production and could not have worked: it scored 0..1 against doctrine
/// priorities in the tens, so it lost every contest even at zero hull. An
/// authored entry scores on the same scale as everything else, which is the
/// bug fix hiding inside the deletion.
#[test]
fn authored_retreat_outranks_doctrine_only_once_hull_is_low() {
    let healthy = scored_pool_for(retreat_behaviour(0.3), 100.0, 100.0);
    let hurt = scored_pool_for(retreat_behaviour(0.3), 10.0, 100.0);

    let score_of = |pool: &[crate::core::messages::ScoredObjective], id: &str| {
        pool.iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("{id} must be in the pool"))
            .score
    };

    assert_eq!(
        score_of(&healthy, "retreat-when-hurt"),
        0.0,
        "at full hull the `hull_below` zero-gate must veto the Retreat \
         outright, so its high base_priority costs nothing"
    );
    assert!(
        score_of(&healthy, "loiter") > 0.0,
        "precondition: the rival objective must be live at full hull"
    );

    assert!(
        score_of(&hurt, "retreat-when-hurt") > score_of(&hurt, "loiter"),
        "below the gate's threshold the Retreat must outrank ordinary \
         doctrine — the score-scale bug the synthetic Retreat could never \
         clear (0..1 against a base_priority in the tens)"
    );
    assert_eq!(
        hurt[0].id, "retreat-when-hurt",
        "and it must lead the pool, since every consumer takes the FIRST \
         Helm-relevant entry rather than scanning for the maximum"
    );
}

/// The retreat threshold is designer-tunable per entity template — two
/// ships at identical hull must disagree about retreating purely on their
/// TOML, with no recompile.
///
/// Was `retreat_threshold_comes_from_behaviour_config`, which tuned the
/// engine's `[behaviour] retreat_hull_threshold`. The authored form is
/// strictly more expressive: the threshold, the destination anchor, the
/// urgency and the gate condition are all per-hull now, rather than one
/// hardwired hull ramp to a fixed place.
#[test]
fn retreat_threshold_is_authored_per_entity_template() {
    // Both ships sit at 40% hull; only their authored gate differs.
    let brave = scored_pool_for(retreat_behaviour(0.1), 40.0, 100.0);
    let cautious = scored_pool_for(retreat_behaviour(0.9), 40.0, 100.0);

    let retreat_score = |pool: &[crate::core::messages::ScoredObjective]| {
        pool.iter()
            .find(|o| o.id == "retreat-when-hurt")
            .expect("retreat must be in the pool")
            .score
    };

    assert_eq!(
        retreat_score(&brave),
        0.0,
        "hull 0.4 is above a 0.1 threshold — a brave ship must not retreat"
    );
    assert!(
        retreat_score(&cautious) > 0.0,
        "hull 0.4 is below a 0.9 threshold — a cautious ship must retreat"
    );
}

/// The published pool is sorted descending by score.
///
/// `operate_helm` and `resolve_helm_target_position` both take the FIRST
/// Helm-relevant entry as the top-scored directive rather than scanning for
/// the maximum, so a pool that is merely "mostly sorted" silently
/// mis-selects.
#[test]
fn published_pool_is_sorted_by_score_descending() {
    let scored = scored_pool_for(retreat_behaviour(0.5), 10.0, 100.0);

    assert!(scored.len() > 1, "precondition: need a pool to sort");
    for pair in scored.windows(2) {
        assert!(
            pair[0].score >= pair[1].score,
            "pool must stay sorted descending: {:?} ({}) preceded {:?} ({})",
            pair[0].id,
            pair[0].score,
            pair[1].id,
            pair[1].score
        );
    }
}

/// A hull carrying an authored `Retreat` gated at `threshold`, plus an
/// ordinary always-on objective to outrank. Mirrors the shape shipped in
/// `assets/worlds/patrol.toml`, which authors one on `raider_alpha` (#892 —
/// it used to ship on the retired `pirate_raider.toml`).
fn retreat_behaviour(threshold: f32) -> crate::entities::config::BehaviourConfig {
    use crate::entities::config::{BehaviourConfig, DoctrineObjective};
    BehaviourConfig {
        doctrine: vec![
            DoctrineObjective {
                id: "loiter".into(),
                text: "Loiter".into(),
                directive_kind: Some("Patrol".into()),
                base_priority: 20.0,
                directive_loop: true,
                ..Default::default()
            },
            DoctrineObjective {
                id: "retreat-when-hurt".into(),
                text: "Hull critical - run for the haven".into(),
                directive_kind: Some("Retreat".into()),
                directive_anchor: Some("pirate_haven".into()),
                base_priority: 100.0,
                zero_gates: vec![crate::objectives::ZeroGateCondition {
                    condition: "hull_below".into(),
                    threshold: Some(threshold),
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// Issue #936, retained through #1010: `WorldConditions.attacked` never
/// reports whether the ship merely CARRIES the component that would record
/// an attacker.
///
/// `entity_spawner` inserts `LastShipAttacker::default()` on every ship it
/// spawns, so the pre-#936 `last_attacker_opt.is_some()` was a constant
/// `true` from an NPC's first tick and every `attacked` / `not_attacked`
/// condition in shipped content was decided before the fight began. That
/// survived because nothing ever looked: the one test over
/// `aggregate_doctrine_blackboards` spawned an entity with NO
/// `LastShipAttacker` at all, which reads `false` on both sides of the fix.
///
/// #1010 then moved the signal off `LastShipAttacker` entirely. That field
/// is set on the first beam that connects and cleared only on death or on
/// the red-alert on→off edge, and while the shooting continues that edge
/// never comes: a Harrow's captain stand-down (`combat_window_secs = 10`)
/// folds the hull's OWN return fire into `secs_since_combat`, so a Harrow
/// fighting back keeps its own alert up and the latch stayed set for as
/// long as anything loitered nearby — the playtest's frozen raid. The
/// signal is now recency of the last LANDED HIT, which is why the middle
/// case below — a named attacker on a ship nothing has hit lately — leaves
/// the gate open where it used to veto.
///
/// The doctrine shape is `combat_test.toml`'s: an assault entry zero-gated
/// on `not_attacked`, which is what "commit to the raid unless something is
/// shooting at you" is authored as, and which scored 0 on every wave for
/// the whole life of the #936 bug.
#[test]
fn a_default_last_attacker_component_does_not_read_as_attacked() {
    let score_of = |pool: &[crate::core::messages::ScoredObjective], id: &str| {
        pool.iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("{id} must be in the pool"))
            .score
    };

    let untouched = scored_pool_for_attacker(assault_behaviour(), None, Activity::default());
    assert!(
        score_of(&untouched, "assault-starbase") > 0.0,
        "a ship carrying `LastShipAttacker::default()` has NOT been \
         attacked, so a `not_attacked` zero-gate must stay open. Reading \
         the component's presence instead of its contents vetoed this \
         entry on every wave of every run: {untouched:?}"
    );

    let stale_attacker = scored_pool_for_attacker(
        assault_behaviour(),
        Some("attacker-uuid"),
        Activity::default(),
    );
    assert!(
        score_of(&stale_attacker, "assault-starbase") > 0.0,
        "the attacker uuid is a LATCH — set on the first hit, cleared only \
         on death or on the red-alert on→off edge, which a ship still \
         returning fire never reaches. A ship carrying one but taking no \
         hits is not under attack, so the raid must stay live: \
         {stale_attacker:?}"
    );

    let under_fire = scored_pool_for_attacker(
        assault_behaviour(),
        Some("attacker-uuid"),
        Activity {
            last_damage_taken: Some(0.0),
            ..Activity::default()
        },
    );
    assert_eq!(
        score_of(&under_fire, "assault-starbase"),
        0.0,
        "with a hit inside the memory window the `not_attacked` gate must \
         veto the assault so the hull turns and fights: {under_fire:?}"
    );
    assert!(
        score_of(&under_fire, "destroy-hostiles") > 0.0,
        "precondition: the rival entry the veto hands the fight to must be \
         live: {under_fire:?}"
    );
}

/// Fire an arc ABSORBS is still being attacked (issue #1010 review).
///
/// `last_damage_taken` moves only when the hull total drops, so shield-eaten
/// fire lands in `last_hostile_fire_taken` instead. This is not a corner
/// case: `station_axiom.toml` shoots 5 dps in 4 s bursts with no
/// `shield_pierce` at a Harrow's single 90 hp arc regenerating 2/s — the
/// burst (20 dmg) barely outpaces the regen over the same cycle (16 dmg
/// over 8 s), netting only ~4 hp/cycle, so shield-absorbed fire dominates
/// the engagement and hull damage does not land until sustained pressure
/// collapses the arc (roughly three minutes of continuous fire). A gate
/// reading hull damage alone would leave a Harrow under sustained station
/// fire flying its raid into the guns for most of a short engagement —
/// exactly the AC ("a Harrow attacked by the station switches to
/// self-defence") the old per-beam `LastShipAttacker` signal did satisfy,
/// since `tick_beams` marks the target on every hit regardless of damage.
#[test]
fn shield_absorbed_hostile_fire_closes_the_assault_gate() {
    let score_of = |pool: &[crate::core::messages::ScoredObjective], id: &str| {
        pool.iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("{id} must be in the pool"))
            .score
    };

    let shields_holding = scored_pool_for_attacker(
        assault_behaviour(),
        Some("attacker-uuid"),
        Activity {
            last_hostile_fire_taken: Some(0.0),
            ..Activity::default()
        },
    );
    assert_eq!(
        score_of(&shields_holding, "assault-starbase"),
        0.0,
        "a hit the shields ate is still a hit — the raid must break off \
         even though the hull total never moved: {shields_holding:?}"
    );
}

/// Firing your own guns is NOT being attacked (issue #1010 review).
///
/// `last_weapon_fired` is the third reading on `RecentCombatActivity` and
/// the captain's `secs_since_combat` red-alert fact folds it in — which is
/// precisely why the alert never stood down mid-fight. Folding it here too
/// would make a `not_attacked` gate veto itself the instant the hull opened
/// fire and hold the veto for as long as it kept firing, reinstating the
/// permanent break-off #1010 exists to remove.
#[test]
fn a_ships_own_weapon_fire_does_not_read_as_being_attacked() {
    let score_of = |pool: &[crate::core::messages::ScoredObjective], id: &str| {
        pool.iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("{id} must be in the pool"))
            .score
    };

    let shooting = scored_pool_for_attacker(
        assault_behaviour(),
        Some("attacker-uuid"),
        Activity {
            last_weapon_fired: Some(0.0),
            ..Activity::default()
        },
    );
    assert!(
        score_of(&shooting, "assault-starbase") > 0.0,
        "a raider pressing its own attack has not been attacked — the \
         `not_attacked` gate must stay open: {shooting:?}"
    );
}

/// Issue #1010: the `attacked` window DECAYS, so the raid a hit interrupts
/// comes back once the shooting stops — and it decays over the window the
/// WORLD AUTHORS, not just over the serde default.
///
/// This is the behaviour `combat_test.toml`'s `assault-starbase` is
/// authored for and never got: a hit closes the `not_attacked` gate (the
/// base `destroy_hostiles` arm takes the fight), and a reprieve of
/// `[global] attacked_memory_secs` with no further hits reopens it. Under
/// the old `LastShipAttacker` latch the reopen needed a red-alert
/// stand-down that a ship still returning fire never reached, so the raid
/// stayed retired for as long as anything loitered nearby.
///
/// Driven over the real fixed clock rather than by poking a boolean, so the
/// decay is measured in the sim seconds the window is authored in
/// (AGENTS.md #7) — nothing here reads a wall clock.
///
/// The world here AUTHORS a short window (#889's lesson: a fixture with no
/// `WorldConfig` exercises only the serde-default fallback arm, so a
/// typo'd field path in the resolution block would pass unnoticed). Two
/// seconds is well clear of the 8 s default, so the gate reopening on
/// schedule can only be the authored value being read.
#[test]
fn the_assault_resumes_once_the_authored_attacked_window_elapses() {
    use crate::console::weapons::beam::LastShipAttacker;
    use crate::core::messages::SystemId;
    use crate::entities::spawner::EntitySystemHull;
    use crate::server_app::ShipSystemBlackboards;
    use crate::ship::damage::SystemHull;

    let window = 2.0_f32;
    let default_window = crate::entities::config::GlobalConfig::default().attacked_memory_secs;
    assert!(
        window < default_window,
        "the authored window must differ from the {default_window}s serde \
         default, or this test cannot tell the two arms apart"
    );

    let mut app = build_test_app();
    // The three clock keys are authored 1:1 so the AI cadence stays one
    // snapshot per fixed step, which is what `build_test_app` is paced for
    // — a `WorldConfig` arms `ai::cadence`'s latches from the authored
    // ratios instead of every step, and the shipped 60/30/10 defaults would
    // put the aggregator on a six-step cadence this fixture was not written
    // against. `sim_tick_hz` sits on `MIN_SIM_TICK_HZ`, the floor
    // `parse_world` enforces.
    app.insert_resource(
        crate::world::config::parse_world(&format!(
            "[global]\n\
             sim_tick_hz = 30\n\
             ai_tick_hz = 30\n\
             ai_snapshot_hz = 30\n\
             attacked_memory_secs = {window}\n"
        ))
        .expect("world TOML should parse"),
    );
    let ship = app
        .world_mut()
        .spawn((
            BehaviourSection(assault_behaviour()),
            EntitySystemHull(SystemHull::from_config(&[(
                SystemId("captain".into()),
                100.0,
            )])),
            ShipSystemBlackboards::default(),
            LastShipAttacker(Some("attacker-uuid".into())),
            Activity::default(),
        ))
        .id();

    app.update();
    assert!(
        published_score(&app, ship, "assault-starbase") > 0.0,
        "precondition: an undamaged raider flies the assault"
    );

    // A hit lands on this tick.
    let hit_at = sim_secs(&app);
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<Activity>()
        .expect("combat activity")
        .last_damage_taken = Some(hit_at);
    app.update();
    assert_eq!(
        published_score(&app, ship, "assault-starbase"),
        0.0,
        "a fresh hit must close the `not_attacked` gate"
    );
    assert!(
        published_score(&app, ship, "destroy-hostiles") > 0.0,
        "self-defence is what the veto hands the fight to"
    );

    // Halfway through the reprieve, with no further hits: still broken off.
    // A window that expired early would let the raider turn its back on a
    // ship that is still shooting it.
    drive_to_sim_secs(&mut app, hit_at + window * 0.5);
    assert_eq!(
        published_score(&app, ship, "assault-starbase"),
        0.0,
        "the gate must stay shut for the whole authored {window}s window"
    );

    // Past the AUTHORED window — and still well inside the serde default,
    // so a resolution block that silently fell back to the default would
    // leave this at 0 and fail here.
    drive_to_sim_secs(&mut app, hit_at + window + 0.5);
    assert!(
        sim_secs(&app) < hit_at + default_window,
        "precondition: still inside the {default_window}s default, so \
         reopening here can only be the authored {window}s being honoured"
    );
    assert!(
        published_score(&app, ship, "assault-starbase") > 0.0,
        "after the authored {window}s with no further hits the `attacked` \
         memory must decay and the raid resume — under the old latch this \
         stayed 0 for as long as anything loitered nearby"
    );
}

/// Simulation seconds elapsed on the fixed clock — the same value
/// `aggregate_doctrine_blackboards` reads from `Res<Time>` inside
/// `FixedUpdate`.
fn sim_secs(app: &App) -> f32 {
    app.world().resource::<Time<Fixed>>().elapsed_secs()
}

/// Drive whole fixed steps until the sim clock reaches `target` seconds.
fn drive_to_sim_secs(app: &mut App, target: f32) {
    let mut guard = 0;
    while sim_secs(app) < target {
        app.update();
        guard += 1;
        assert!(
            guard < 10_000,
            "the fixed clock is not advancing — {} s after {guard} updates",
            sim_secs(app)
        );
    }
}

/// The score `aggregate_doctrine_blackboards` last published for `id` in
/// this entity's viewscreen pool.
fn published_score(app: &App, ship: Entity, id: &str) -> f32 {
    let bb = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .expect("blackboards");
    match bb
        .0
        .get(&crate::core::messages::SystemId(
            crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID.to_string(),
        ))
        .expect("viewscreen entry")
    {
        crate::core::messages::SystemBlackboard::Viewscreen(v) => {
            v.scored_objectives
                .iter()
                .find(|o| o.id == id)
                .unwrap_or_else(|| panic!("{id} must be in the pool: {v:?}"))
                .score
        }
        _ => panic!("expected Viewscreen blackboard"),
    }
}

/// The combat-activity component the `attacked` gate reads, aliased so the
/// three-reading case tables above stay legible.
use crate::ship::combat_activity::RecentCombatActivity as Activity;

/// Publish a viewscreen pool for one entity that carries a
/// `LastShipAttacker` and a `RecentCombatActivity`, and hand back its
/// `scored_objectives`. The activity's timestamps are in sim seconds;
/// `Some(0.0)` is "at the start of the run", which the single tick this
/// drives leaves well inside the memory window.
///
/// It takes the whole component rather than one timestamp because WHICH
/// readings feed the gate is the thing under test: hull damage and
/// shield-absorbed fire both close it, own weapon fire must not.
///
/// Deliberately separate from [`scored_pool_for`]: that helper spawns
/// neither component, which is a further state (`None` vs `Some(default)`
/// vs `Some(attacker)`) and the one the pre-#936 bug hid behind.
fn scored_pool_for_attacker(
    behaviour: crate::entities::config::BehaviourConfig,
    attacker: Option<&str>,
    activity: Activity,
) -> Vec<crate::core::messages::ScoredObjective> {
    use crate::console::weapons::beam::LastShipAttacker;
    use crate::core::messages::SystemId;
    use crate::entities::spawner::EntitySystemHull;
    use crate::server_app::ShipSystemBlackboards;
    use crate::ship::damage::SystemHull;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    let mut app = build_test_app();

    app.world_mut().spawn((
        BehaviourSection(behaviour),
        EntitySystemHull(SystemHull::from_config(&[(
            SystemId("captain".into()),
            100.0,
        )])),
        ShipSystemBlackboards::default(),
        LastShipAttacker(attacker.map(str::to_string)),
        activity,
    ));
    app.update();

    let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
    let bb = q.iter(app.world()).next().expect("blackboards").clone();
    match bb
        .0
        .get(&crate::core::messages::SystemId(
            VIEWSCREEN_SYSTEM_ID.to_string(),
        ))
        .expect("viewscreen entry")
    {
        crate::core::messages::SystemBlackboard::Viewscreen(v) => v.scored_objectives.clone(),
        _ => panic!("expected Viewscreen blackboard"),
    }
}

/// A raid hull shaped like the ones `combat_test.toml` spawns: the
/// template's untargeted Destroy, plus the world's `not_attacked`-gated
/// assault on the station.
fn assault_behaviour() -> crate::entities::config::BehaviourConfig {
    use crate::entities::config::{BehaviourConfig, DoctrineObjective};
    BehaviourConfig {
        doctrine: vec![
            DoctrineObjective {
                id: "destroy-hostiles".into(),
                text: "Engage whatever is in front of you".into(),
                directive_kind: Some("Destroy".into()),
                base_priority: 38.0,
                ..Default::default()
            },
            DoctrineObjective {
                id: "assault-starbase".into(),
                text: "Press the assault on the station".into(),
                directive_kind: Some("Destroy".into()),
                directive_target: Some("world.entity.starbase_alpha.name".into()),
                base_priority: 50.0,
                zero_gates: vec![crate::objectives::ZeroGateCondition {
                    condition: "not_attacked".into(),
                    threshold: None,
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn faction_registry_resource_is_present_on_native() {
    let app = build_test_app();
    assert!(
        app.world()
            .get_resource::<FactionRegistryResource>()
            .is_some(),
        "FactionRegistryResource must be present on native without WASM preload"
    );
}

#[test]
fn faction_registry_resource_accessible_as_res_not_option() {
    let mut app = build_test_app();
    app.add_systems(bevy::app::Update, read_faction_registry_system);
    app.update(); // Must not panic
}

// ── LOD system tests ─────────────────────────────────────────────────────

use crate::server_app::{LocalShip, Ship};
use crate::ship::state::ShipPhysics;

/// Mirrors the production schedule: `simulate_low_lod_ships` (Physics)
/// steers from the cursor, then `advance_objective_cursors` (Modifiers)
/// advances it against the ship's post-movement position. The `SimSet`s
/// themselves aren't configured here, so the order is stated explicitly.
fn build_lod_test_app() -> App {
    let mut app = App::new();
    app.add_message::<AiWaypointReached>();
    app.insert_resource(Time::<()>::default()).add_systems(
        Update,
        (
            simulate_low_lod_ships.before(lod_ai_ships),
            lod_ai_ships,
            advance_objective_cursors.after(simulate_low_lod_ships),
        ),
    );
    app
}

fn tick_with_dt(app: &mut App, dt_secs: f32) {
    let mut time = app.world_mut().resource_mut::<Time>();
    time.advance_by(std::time::Duration::from_secs_f32(dt_secs));
    app.update();
}

fn spawn_player(app: &mut App, x: f32, z: f32) -> Entity {
    app.world_mut()
        .spawn((
            Ship,
            LocalShip,
            Transform::from_xyz(x, 0.0, z),
            ShipPhysics::default(),
            // Same shared set the production player-ship spawn uses.
            ai_high_fidelity_components(),
            // LOD keys on the ANCHOR's bubble radius, so these mechanic tests
            // give the player a 100-unit bubble — the threshold they were
            // written against back when it was `spawn_npc`'s `sensor_range`
            // arg — so their promote/demote distances (50 in, 200 out,
            // 110/120 hysteresis) still mean what they say.
            LodBubble { radius: 100.0 },
        ))
        .id()
}

fn spawn_npc(app: &mut App, x: f32, z: f32, sensor_range: f32) -> Entity {
    app.world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(x, 0.0, z),
            ShipPhysics {
                x,
                z,
                forward_speed: 10.0,
                yaw: 0.0,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range,
                ..Default::default()
            },
        ))
        .id()
}

/// The guard against the recurring spawn-path gap.
///
/// Every per-ship AI component that must accompany `AiHighFidelity` is
/// named ONCE, in `AiHighFidelityComponents`, and every spawn path inserts
/// that set — so a component added there reaches the player ship
/// (`server_app::spawn_game_start_entities`), every promoted NPC
/// (`lod_ai_ships`) and the test twin (`ship::test_support`) at the same
/// time. This test pins what the set contains, so the components three
/// separate issues have now silently lost on one path or the other cannot
/// quietly leave it.
#[test]
fn the_high_fidelity_component_set_carries_every_per_ship_ai_component() {
    let mut world = World::new();
    let e = world.spawn(ai_high_fidelity_components()).id();
    assert!(world.get::<AiHighFidelity>(e).is_some(), "the marker");
    assert!(
        world
            .get::<crate::console_ai::server::ShipFrequencyHintState>(e)
            .is_some(),
        "frequency-hint state (issue #692)"
    );
    assert!(world.get::<crate::ship::helm::ThrustInput>(e).is_some());
    assert!(world.get::<crate::ship::helm::SteeringInput>(e).is_some());
    assert!(world
        .get::<crate::ship::helm::LateralThrustInput>(e)
        .is_some());
    assert!(world
        .get::<crate::ship::helm::VerticalThrustInput>(e)
        .is_some());
    assert!(world.get::<crate::ship::helm::ImpulseCommand>(e).is_some());
    assert!(world.get::<crate::ship::helm::BoostCommand>(e).is_some());
    assert!(
        world
            .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(e)
            .is_some(),
        "the #882 policy runtime state — the component the player ship was \
         missing, which made `ai_policy_state_tick` skip it silently"
    );

    // Insert and remove are the same unit, so a demoted ship cannot keep
    // half of it.
    world.entity_mut(e).remove::<AiHighFidelityComponents>();
    assert!(world.get::<AiHighFidelity>(e).is_none());
    assert!(world
        .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(e)
        .is_none());
    assert!(world.get::<crate::ship::helm::BoostCommand>(e).is_none());
}

#[test]
fn local_ship_permanently_has_ai_high_fidelity() {
    let mut app = build_lod_test_app();
    let player = spawn_player(&mut app, 0.0, 0.0);
    assert!(
        app.world().get::<AiHighFidelity>(player).is_some(),
        "LocalShip must start with AiHighFidelity"
    );
    tick_with_dt(&mut app, 0.1);
    assert!(
        app.world().get::<AiHighFidelity>(player).is_some(),
        "LocalShip must retain AiHighFidelity after update"
    );
}

#[test]
fn npc_out_of_range_gets_cheap_movement() {
    let mut app = build_lod_test_app();
    spawn_player(&mut app, 0.0, 0.0);
    let npc = spawn_npc(&mut app, 500.0, 0.0, 100.0);

    let initial = *app.world().get::<ShipPhysics>(npc).unwrap();
    tick_with_dt(&mut app, 0.1);
    let physics = app.world().get::<ShipPhysics>(npc).unwrap();

    // With yaw=0, forward_speed=10, dt=0.1:
    //   x' = 500 + 10 * sin(0) * 0.1 = 500
    //   z' = 0 - 10 * cos(0) * 0.1 = -1
    assert!(
        (physics.z - (initial.z - 1.0)).abs() < 0.001,
        "NPC z should advance by forward_speed * dt: expected {}, got {}",
        initial.z - 1.0,
        physics.z,
    );
    assert!(
        (physics.x - initial.x).abs() < 0.001,
        "NPC x should not change when yaw=0: expected {}, got {}",
        initial.x,
        physics.x,
    );
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_none(),
        "NPC out of range must not have AiHighFidelity"
    );
}

#[test]
fn npc_in_range_promoted_to_high_fidelity() {
    let mut app = build_lod_test_app();
    spawn_player(&mut app, 0.0, 0.0);
    let npc = spawn_npc(&mut app, 50.0, 0.0, 100.0);

    tick_with_dt(&mut app, 0.1);
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_some(),
        "NPC within sensor_range must be promoted to AiHighFidelity"
    );
}

#[test]
fn dwell_timer_prevents_lod_thrashing() {
    let mut app = build_lod_test_app();
    spawn_player(&mut app, 0.0, 0.0);
    let npc = spawn_npc(&mut app, 50.0, 0.0, 100.0);

    // First update: promote to High (within range)
    tick_with_dt(&mut app, 0.1);
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_some(),
        "NPC must start in High after first update"
    );

    // Move far outside range + hysteresis
    app.world_mut()
        .entity_mut(npc)
        .insert(Transform::from_xyz(200.0, 0.0, 0.0));
    app.world_mut().entity_mut(npc).insert(ShipPhysics {
        x: 200.0,
        z: 0.0,
        forward_speed: 10.0,
        yaw: 0.0,
        ..Default::default()
    });

    // One more update: still within 2s dwell window (only 0.2s elapsed total)
    tick_with_dt(&mut app, 0.1);
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_some(),
        "NPC must stay High during dwell window"
    );

    // Advance well past 2-second dwell (35 * 0.1 = 3.5s more elapsed)
    for _ in 0..35 {
        tick_with_dt(&mut app, 0.1);
    }

    assert!(
        app.world().get::<AiHighFidelity>(npc).is_none(),
        "NPC must demote after dwell window elapses"
    );
}

/// A `LodBubble` carrier anchors its own zone, so it is held high-fidelity
/// unconditionally — the station must run its own guns even when the player
/// (the only other bubble) is on the far side of the map.
#[test]
fn lod_bubble_carrier_is_always_high_fidelity() {
    let mut app = build_lod_test_app();
    spawn_player(&mut app, 5000.0, 0.0);
    let station = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 0.0),
            ShipPhysics::default(),
            AiProfile::default(),
            LodBubble { radius: 250.0 },
        ))
        .id();
    tick_with_dt(&mut app, 0.1);
    assert!(
        app.world().get::<AiHighFidelity>(station).is_some(),
        "a LodBubble carrier must stay high-fidelity even with the player far away"
    );
}

/// An NPC inside a NON-player bubble (the station's) is promoted even though
/// the player is nowhere near — the raid sieging the station runs in full
/// fidelity so its guns, and the station's, actually fire.
#[test]
fn npc_inside_a_non_player_bubble_is_promoted() {
    let mut app = build_lod_test_app();
    // Player's 100-unit bubble parked far away.
    spawn_player(&mut app, 5000.0, 0.0);
    // Station anchor at the origin with a 250-unit bubble.
    app.world_mut().spawn((
        Ship,
        Transform::from_xyz(0.0, 0.0, 0.0),
        ShipPhysics::default(),
        AiProfile::default(),
        LodBubble { radius: 250.0 },
    ));
    // NPC 200 units from the station (inside its bubble), far from the player.
    let npc = spawn_npc(&mut app, 200.0, 0.0, 100.0);
    tick_with_dt(&mut app, 0.1);
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_some(),
        "an NPC inside the station's bubble must be promoted with the player far away"
    );
}

/// An NPC outside EVERY bubble stays low — the station's bubble does not
/// blanket the map, it covers its own airspace.
#[test]
fn npc_outside_every_bubble_stays_low() {
    let mut app = build_lod_test_app();
    // Player's 100-unit bubble at the origin.
    spawn_player(&mut app, 0.0, 0.0);
    // Station's 250-unit bubble a kilometre away.
    app.world_mut().spawn((
        Ship,
        Transform::from_xyz(1000.0, 0.0, 0.0),
        ShipPhysics::default(),
        AiProfile::default(),
        LodBubble { radius: 250.0 },
    ));
    // NPC at 500: 500 from the player (outside 100) and 500 from the station
    // (outside 250).
    let npc = spawn_npc(&mut app, 500.0, 0.0, 100.0);
    tick_with_dt(&mut app, 0.1);
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_none(),
        "an NPC outside every bubble must not be promoted"
    );
}

/// Promotion on re-entering the ring restores full doctrine cleanly
/// (issue #933 AC2 — existing behaviour, pinned here). This is the same
/// `ai_high_fidelity_components()` unit `the_high_fidelity_component_set_
/// carries_every_per_ship_ai_component` already pins in isolation; this
/// test pins it end-to-end through the actual demote → re-enter cycle
/// that `lod_ai_ships` drives, so a future change to either the demote or
/// promote arm can't quietly stop restoring the full set on re-entry.
#[test]
fn demoted_npc_repromoted_on_re_entry_restores_full_high_fidelity_components() {
    let mut app = build_lod_test_app();
    spawn_player(&mut app, 0.0, 0.0);
    let npc = spawn_npc(&mut app, 50.0, 0.0, 100.0);

    // Promote (within range).
    tick_with_dt(&mut app, 0.1);
    assert!(app.world().get::<AiHighFidelity>(npc).is_some());
    assert!(app
        .world()
        .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(npc)
        .is_some());

    // Demote: move far outside range + hysteresis, wait out the dwell.
    app.world_mut()
        .entity_mut(npc)
        .insert(Transform::from_xyz(500.0, 0.0, 0.0));
    app.world_mut().entity_mut(npc).insert(ShipPhysics {
        x: 500.0,
        z: 0.0,
        forward_speed: 10.0,
        yaw: 0.0,
        ..Default::default()
    });
    for _ in 0..30 {
        tick_with_dt(&mut app, 0.1);
    }
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_none(),
        "must have demoted before re-entry can be tested"
    );
    assert!(
        app.world()
            .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(npc)
            .is_none(),
        "demotion must strip the whole high-fidelity component set, not just the marker"
    );

    // Re-enter: move back within sensor range.
    app.world_mut()
        .entity_mut(npc)
        .insert(Transform::from_xyz(50.0, 0.0, 0.0));
    app.world_mut().entity_mut(npc).insert(ShipPhysics {
        x: 50.0,
        z: 0.0,
        forward_speed: 10.0,
        yaw: 0.0,
        ..Default::default()
    });
    tick_with_dt(&mut app, 0.1);

    assert!(
        app.world().get::<AiHighFidelity>(npc).is_some(),
        "NPC re-entering sensor range must be promoted back to AiHighFidelity"
    );
    assert!(
        app.world()
            .get::<crate::ship::helm_ai::HelmBoostAiPolicyState>(npc)
            .is_some(),
        "re-promotion must restore the FULL high-fidelity component set, \
         not just the AiHighFidelity marker"
    );
}

// ── Dead-reckoning fallback: decay + return-to-target (issue #933) ─────────

/// The named AC3 test: demote a ship mid-escape (moving fast, away from
/// its standing `Destroy` target) and assert it re-enters the engagement
/// envelope (comes back within `sensor_range` of its target) within a
/// bounded simulated time, rather than dead-reckoning its exit velocity
/// off into the void forever.
#[test]
fn demoted_ship_mid_escape_returns_to_engagement_envelope_within_bounded_time() {
    let mut app = build_lod_test_app();

    // Standing target the ship's Destroy directive names, resolvable via
    // WorldSnapshot exactly as the production build_world_snapshot pass
    // would publish it — parked at the origin.
    app.insert_resource(WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: uuid::Uuid::nil(),
            name: Some("target-ship".to_string()),
            position: [0.0, 0.0, 0.0],
            faction: None,
            shields: None,
            hull_fraction: None,
            yaw: None,
            radius: 5.0,
            forward_speed: 0.0,
            movable: true,
            dangerous: true,
            size_rating: 5.0,
            direct_fire_range: 0.0,
            weapon_arcs: vec![],
        }],
    });

    let sensor_range = 100.0_f32;
    // Demoted mid-escape: parked well outside the ring, at a boosted
    // speed, yaw pointed directly AWAY from the target (forward = (0, 1)
    // at yaw = PI in this sim's (sin(yaw), -cos(yaw)) convention).
    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 300.0),
            ShipPhysics {
                x: 0.0,
                z: 300.0,
                forward_speed: 80.0,
                yaw: std::f32::consts::PI,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range,
                ..Default::default()
            },
            // Marks this ship as having been through at least one High↔Low
            // transition already — it really was demoted, not just still
            // approaching for the first time (see the gating note on
            // `simulate_low_lod_ships`).
            LodTransitionTimer {
                last_state_change_secs: 0.0,
            },
            BehaviourSection(crate::entities::config::BehaviourConfig {
                doctrine: vec![crate::entities::config::DoctrineObjective {
                    id: "assault".into(),
                    text: "Destroy target-ship".into(),
                    directive_kind: Some("Destroy".into()),
                    directive_target: Some("target-ship".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            // What `aggregate_doctrine_blackboards` would have published
            // this tick for the doctrine above once scored: a single
            // Destroy entry, ungated, so it scores above 0 and qualifies
            // as the standing target `active_destroy_target` resolves.
            blackboards_with_destroy_pool(&[("assault", 1.0, "target-ship")]),
            crate::entities::spawner::HelmConsoleSection(
                crate::entities::config::EntityConfig::from_toml(
                    "[helm_console]\nmax_speed = 100.0\nmax_yaw_rate = 1.0\n",
                )
                .unwrap()
                .helm_console
                .unwrap(),
            ),
        ))
        .id();

    let distance_to_target = |app: &App| -> f32 {
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        (physics.x * physics.x + physics.z * physics.z).sqrt()
    };

    assert!(
        distance_to_target(&app) > sensor_range,
        "test setup: must start outside the engagement envelope"
    );

    // Bounded time: 60 simulated seconds at 10 Hz. If the old frozen dead-
    // reckoning fallback were still in place this ship would only ever
    // move farther away (300 + 80*t) and this loop would time out.
    let bound_ticks = 600;
    let mut re_entered = false;
    for _ in 0..bound_ticks {
        tick_with_dt(&mut app, 0.1);
        if distance_to_target(&app) <= sensor_range {
            re_entered = true;
            break;
        }
    }

    assert!(
        re_entered,
        "demoted ship must re-enter the engagement envelope (distance <= {sensor_range}) \
         within {bound_ticks} ticks; final distance was {}",
        distance_to_target(&app)
    );
}

/// Companion negative-ish check: without a standing named `Destroy`
/// target (untargeted / no doctrine at all), the fallback still decays
/// the frozen speed toward cruise rather than holding the boosted exit
/// speed forever — it just has nothing to turn toward, so it does not
/// necessarily return. Pins the decay half of #933 independently of the
/// steering half.
#[test]
fn demoted_ship_with_no_destroy_target_still_decays_frozen_speed_toward_cruise() {
    let mut app = build_lod_test_app();

    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 300.0),
            ShipPhysics {
                x: 0.0,
                z: 300.0,
                forward_speed: 80.0,
                yaw: std::f32::consts::PI,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range: 100.0,
                ..Default::default()
            },
            LodTransitionTimer {
                last_state_change_secs: 0.0,
            },
            crate::entities::spawner::HelmConsoleSection(
                crate::entities::config::EntityConfig::from_toml(
                    "[helm_console]\nmax_speed = 100.0\nmax_yaw_rate = 1.0\n",
                )
                .unwrap()
                .helm_console
                .unwrap(),
            ),
        ))
        .id();

    for _ in 0..100 {
        tick_with_dt(&mut app, 0.1);
    }

    let physics = app.world().get::<ShipPhysics>(npc).unwrap();
    // cruise_fraction defaults to 0.5 -> cruise speed = 100.0 * 0.5 = 50.0
    assert!(
        (physics.forward_speed - 50.0).abs() < 0.5,
        "frozen speed must have decayed to the authored cruise fraction of max_speed, got {}",
        physics.forward_speed
    );
}

/// Review follow-up on issue #933: the low-LOD return-steer must resolve
/// its Destroy target through the *scored* pool, honoring `zero_gates`,
/// not just grab the first `Destroy` entry in authoring order.
///
/// Shipped counter-example this pins: `combat_test.toml`'s wave ships
/// author `assault-starbase` (Destroy "Starbase Alpha") gated on
/// `zero_gates = [{condition = "not_attacked"}]`. Once the ship has been
/// attacked, `score_doctrine_pool` scores that directive at 0 — exactly
/// like the high-LOD `plan_helm_travel`, which filters `score > 0.0` and
/// so stops steering at the starbase. A demoted, attacked ship must agree:
/// with its only Destroy directive scored at 0 (the gate having fired),
/// `active_destroy_target` must find nothing to steer toward and the
/// dead-reckoning fallback must fall back to decay-only, never turning
/// the frozen exit heading back toward that target.
#[test]
fn demoted_attacked_ship_does_not_steer_toward_a_zero_gated_destroy_target() {
    let mut app = build_lod_test_app();

    app.insert_resource(WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: uuid::Uuid::nil(),
            name: Some("target-ship".to_string()),
            position: [0.0, 0.0, 0.0],
            faction: None,
            shields: None,
            hull_fraction: None,
            yaw: None,
            radius: 5.0,
            forward_speed: 0.0,
            movable: true,
            dangerous: true,
            size_rating: 5.0,
            direct_fire_range: 0.0,
            weapon_arcs: vec![],
        }],
    });

    // Same mid-escape setup as the positive case: parked outside sensor
    // range, boosted speed, yaw pointed directly away from the target.
    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 300.0),
            ShipPhysics {
                x: 0.0,
                z: 300.0,
                forward_speed: 80.0,
                yaw: std::f32::consts::PI,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range: 100.0,
                ..Default::default()
            },
            LodTransitionTimer {
                last_state_change_secs: 0.0,
            },
            BehaviourSection(crate::entities::config::BehaviourConfig {
                doctrine: vec![crate::entities::config::DoctrineObjective {
                    id: "assault-starbase".into(),
                    text: "Destroy target-ship".into(),
                    directive_kind: Some("Destroy".into()),
                    directive_target: Some("target-ship".into()),
                    base_priority: 100.0,
                    zero_gates: vec![crate::objectives::ZeroGateCondition {
                        condition: "not_attacked".into(),
                        threshold: None,
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }),
            // The gate has already fired: this ship has been attacked, so
            // `score_doctrine_pool` would score `assault-starbase` at 0 —
            // reflected here exactly as `aggregate_doctrine_blackboards`
            // would publish it for an attacked ship.
            blackboards_with_destroy_pool(&[("assault-starbase", 0.0, "target-ship")]),
            crate::entities::spawner::HelmConsoleSection(
                crate::entities::config::EntityConfig::from_toml(
                    "[helm_console]\nmax_speed = 100.0\nmax_yaw_rate = 1.0\n",
                )
                .unwrap()
                .helm_console
                .unwrap(),
            ),
        ))
        .id();

    let distance_to_target = |app: &App| -> f32 {
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        (physics.x * physics.x + physics.z * physics.z).sqrt()
    };

    let start_distance = distance_to_target(&app);

    for _ in 0..600 {
        tick_with_dt(&mut app, 0.1);
    }

    assert!(
        distance_to_target(&app) >= start_distance,
        "a zero-gated (score == 0) Destroy directive must not steer the ship back — \
         distance to target must not have decreased, started at {start_distance}, \
         ended at {}",
        distance_to_target(&app)
    );
}

/// Companion to the zero-gate test above: resolution must pick the
/// TOP-SCORING Destroy directive, not the first one in authoring order.
/// A low-scoring (here, zero-scored/gated) decoy entry authored first
/// must be skipped in favor of a higher-scoring entry authored after it.
#[test]
fn demoted_ship_resolves_destroy_target_by_score_not_authoring_order() {
    let mut app = build_lod_test_app();

    app.insert_resource(WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::nil(),
                name: Some("decoy-ship".to_string()),
                position: [0.0, 0.0, 5_000.0],
                faction: None,
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 5.0,
                forward_speed: 0.0,
                movable: true,
                dangerous: true,
                size_rating: 5.0,
                direct_fire_range: 0.0,
                weapon_arcs: vec![],
            },
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::nil(),
                name: Some("target-ship".to_string()),
                position: [0.0, 0.0, 0.0],
                faction: None,
                shields: None,
                hull_fraction: None,
                yaw: None,
                radius: 5.0,
                forward_speed: 0.0,
                movable: true,
                dangerous: true,
                size_rating: 5.0,
                direct_fire_range: 0.0,
                weapon_arcs: vec![],
            },
        ],
    });

    let sensor_range = 100.0_f32;
    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 300.0),
            ShipPhysics {
                x: 0.0,
                z: 300.0,
                forward_speed: 80.0,
                yaw: std::f32::consts::PI,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range,
                ..Default::default()
            },
            LodTransitionTimer {
                last_state_change_secs: 0.0,
            },
            // First in authoring order is the zero-scored decoy; the
            // real, higher-scoring target is authored second. Resolution
            // must still pick "target-ship".
            blackboards_with_destroy_pool(&[
                ("decoy", 0.0, "decoy-ship"),
                ("assault", 1.0, "target-ship"),
            ]),
            crate::entities::spawner::HelmConsoleSection(
                crate::entities::config::EntityConfig::from_toml(
                    "[helm_console]\nmax_speed = 100.0\nmax_yaw_rate = 1.0\n",
                )
                .unwrap()
                .helm_console
                .unwrap(),
            ),
        ))
        .id();

    let distance_to_target = |app: &App| -> f32 {
        let physics = app.world().get::<ShipPhysics>(npc).unwrap();
        (physics.x * physics.x + physics.z * physics.z).sqrt()
    };

    assert!(
        distance_to_target(&app) > sensor_range,
        "test setup: must start outside the engagement envelope"
    );

    let bound_ticks = 600;
    let mut re_entered = false;
    for _ in 0..bound_ticks {
        tick_with_dt(&mut app, 0.1);
        if distance_to_target(&app) <= sensor_range {
            re_entered = true;
            break;
        }
    }

    assert!(
        re_entered,
        "demoted ship must steer toward the top-scoring Destroy target \
         (target-ship at the origin), not the zero-scored decoy at z=5000; \
         final distance to origin was {}",
        distance_to_target(&app)
    );
}

// ── Low-LOD ships advance on their destroy objectives (issue #1012) ───────
//
// Two gates used to strand a Harrow assault wave short of the station:
//
//   1. a ship still in its FIRST Low stretch since spawn (no
//      `LodTransitionTimer`) was exempt from both dead-reckoning
//      corrections, on the premise that its heading was authored content.
//      `spawner.rs` seeds every ship at `yaw: 0.0`, `forward_speed: 0.0`,
//      so the premise was false and the exemption froze it in place;
//   2. a wave that flew its `close-on-starbase` Reach to the run-in anchor
//      hit `route_completed`, which is true forever after on a non-looping
//      route, and parked — `continue`ing before the destroy-steer could be
//      reached at all, timer or no timer.
//
// Both now yield to a *scored* Destroy. Neither yields to anything else.

/// The engagement geometry these tests share, mirroring `combat_test.toml`:
/// a starbase the wave was told to kill, at a bearing the spawn heading
/// (yaw 0, facing -Z) does not point at.
///
/// Two fields are set explicitly because their defaults are traps:
///
/// * **`uuid`** — a fixture ship whose `EntityUuid` is unparseable (or
///   absent) resolves its `self_uuid` to `Uuid::nil()` via `unwrap_or_default`,
///   and `avoidance_steering` skips `entity.uuid == excluded_uuid`. A nil
///   uuid here would therefore delete the objective from every fixture
///   ship's hazard picture — a silent exemption these tests never asked for.
/// * **`dangerous`** — `AiWorldEntity`'s hand-written `Default` sets it
///   `true`, which is right for an obstacle and wrong for a destination.
///   The low-LOD scan does not read the flag today (see "What this path
///   does NOT share with `assess_hazards`" on `low_lod_avoid_yaw`), so what
///   actually keeps avoidance out of these tests is distance — the ship
///   never closes to within a buffer of the target. Stating it anyway
///   records the intent for the day the filters do reach this path.
fn snapshot_with_named_entity(name: &str, position: [f32; 3]) -> WorldSnapshot {
    WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: uuid::Uuid::from_u128(0x51a7),
            name: Some(name.to_string()),
            position,
            dangerous: false,
            ..Default::default()
        }],
    }
}

/// A `HelmConsoleSection` with the authored limits the low-LOD corrections
/// scale off: cruise is `max_speed * low_lod_cruise_fraction` (0.5 by
/// parse default → 50) and the turn ceiling is `max_yaw_rate *
/// low_lod_turn_rate_fraction` (→ 0.5 rad/s).
fn test_helm_section(
    max_speed: f32,
    max_yaw_rate: f32,
) -> crate::entities::spawner::HelmConsoleSection {
    crate::entities::spawner::HelmConsoleSection(
        crate::entities::config::EntityConfig::from_toml(&format!(
            "[helm_console]\nmax_speed = {max_speed}\nmax_yaw_rate = {max_yaw_rate}\n"
        ))
        .unwrap()
        .helm_console
        .unwrap(),
    )
}

/// AC1/AC2: a Harrow spawned straight into low LOD, carrying a scored
/// `Destroy`, steers at the objective rather than sitting on the canned
/// spawn heading.
///
/// The pre-fix baseline is exact and this test is not vacuous against it:
/// the ship spawns exactly as `spawner.rs` seeds one (`yaw: 0.0`,
/// `forward_speed: 0.0`) and has no `LodTransitionTimer`, so under the old
/// `(ai_profile, lod_timer.is_some())` gate NEITHER correction applied —
/// yaw stayed 0.0, speed stayed 0.0, and the distance to the objective
/// stayed 1500.0 for every tick of the run. That is the loiter.
#[test]
fn first_low_ship_with_a_scored_destroy_steers_at_it_instead_of_its_spawn_heading() {
    const TARGET: [f32; 3] = [1500.0, 0.0, 0.0];
    let mut app = build_lod_test_app();
    app.insert_resource(snapshot_with_named_entity("starbase-alpha", TARGET));
    // Far enough that `lod_ai_ships` runs and still never promotes: the ship
    // under test spawns at the origin, so a player there would put it inside
    // its own sensor range on tick 1 and take it off this path entirely.
    spawn_player(&mut app, 0.0, 50_000.0);

    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 0.0),
            // Exactly what `spawner.rs` seeds: facing -Z, stationary.
            ShipPhysics::default(),
            AiProfile {
                aggression: 0.5,
                // Small enough that the ship never promotes over the run.
                sensor_range: 10.0,
                ..Default::default()
            },
            BehaviourSection(BehaviourConfig::default()),
            blackboards_with_destroy_pool(&[("assault-starbase", 50.0, "starbase-alpha")]),
            test_helm_section(100.0, 1.0),
        ))
        .id();

    assert!(
        app.world().get::<LodTransitionTimer>(npc).is_none(),
        "test setup: this ship must never have been promoted — the whole \
         point is the FIRST Low stretch since spawn"
    );

    let distance_to_target = |app: &App| -> f32 {
        let p = app.world().get::<ShipPhysics>(npc).unwrap();
        ((TARGET[0] - p.x).powi(2) + (TARGET[2] - p.z).powi(2)).sqrt()
    };
    let start_distance = distance_to_target(&app);
    assert!((start_distance - 1500.0).abs() < 0.001);

    // 10 simulated seconds: ~3.1 s to swing 90° at 0.5 rad/s, then cruise.
    for _ in 0..100 {
        tick_with_dt(&mut app, 0.1);
    }

    let physics = *app.world().get::<ShipPhysics>(npc).unwrap();
    // Target bearing from ~the origin to (1500, 0, 0) is π/2.
    assert!(
        (physics.yaw - std::f32::consts::FRAC_PI_2).abs() < 0.05,
        "yaw must converge on the objective's bearing (π/2), not hold the \
         spawn heading of 0.0; got {}",
        physics.yaw
    );
    assert!(
        physics.forward_speed > 1.0,
        "a ship with an order to carry out must get under way — the \
         cruise ramp is the other half of the correction; got {}",
        physics.forward_speed
    );
    assert!(
        distance_to_target(&app) < start_distance - 300.0,
        "the ship must have closed meaningfully on its objective; started \
         at {start_distance}, ended at {} (pre-fix it never moved at all)",
        distance_to_target(&app)
    );
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_none(),
        "this must be the cheap low-LOD path throughout, never a promotion"
    );
}

/// AC1 for the shape `combat_test.toml` actually ships: a wave authoring
/// BOTH `assault-starbase` (Destroy @50) and `close-on-starbase` (Reach
/// @35) flies the run-in, and then *diverts to the Destroy* instead of
/// parking on the anchor.
///
/// This is the gate a relaxed `LodTransitionTimer` check alone would not
/// have opened: `route_completed` is true forever once a non-looping route
/// runs past its end, and the coast-to-stop branch `continue`d before any
/// destroy-steering code could run.
#[test]
fn low_lod_ship_diverts_to_its_scored_destroy_when_the_run_in_route_completes() {
    const RUN_IN: [f32; 3] = [600.0, 0.0, 100.0];
    const TARGET: [f32; 3] = [2600.0, 0.0, 100.0];
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    world
        .anchors
        .insert("harrow_assault_point".to_string(), RUN_IN);
    app.insert_resource(world);
    app.insert_resource(snapshot_with_named_entity("starbase-alpha", TARGET));
    spawn_player(&mut app, 0.0, 0.0);

    // Spawned 300 units +Z of the run-in point, so the approach itself is
    // flown on the spawn heading (yaw 0 = -Z) and the divert is a clean 90°
    // turn onto the objective's bearing.
    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(600.0, 0.0, 400.0),
            ShipPhysics {
                x: 600.0,
                z: 400.0,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range: 10.0,
                ..Default::default()
            },
            EntityUuid("wave_1".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            ObjectiveCursors::default(),
            with_reach_objective(
                blackboards_with_destroy_pool(&[("assault-starbase", 50.0, "starbase-alpha")]),
                "close-on-starbase",
                35.0,
                "harrow_assault_point",
            ),
            test_helm_section(100.0, 1.0),
        ))
        .id();

    let distance_to_target = |app: &App| -> f32 {
        let p = app.world().get::<ShipPhysics>(npc).unwrap();
        ((TARGET[0] - p.x).powi(2) + (TARGET[2] - p.z).powi(2)).sqrt()
    };

    // Fly the run-in until the cursor runs past the end of the one-waypoint
    // Reach — from here on `route_completed` is permanently true.
    let mut completed_after = None;
    for tick in 0..300 {
        tick_with_dt(&mut app, 0.1);
        if cursor_state(&app, npc) == vec![("close-on-starbase".to_string(), 1)] {
            completed_after = Some(tick);
            break;
        }
    }
    assert!(
        completed_after.is_some(),
        "test setup: the ship must actually reach the run-in anchor first"
    );
    let distance_on_arrival = distance_to_target(&app);

    // 15 more seconds, well past the ~4 s the old coast-to-stop needed to
    // bring a 40 u/s run-in to a dead halt.
    for _ in 0..150 {
        tick_with_dt(&mut app, 0.1);
    }

    let physics = *app.world().get::<ShipPhysics>(npc).unwrap();
    assert!(
        physics.forward_speed > 1.0,
        "a completed run-in must NOT park a ship that still has a scored \
         Destroy to serve; forward_speed was {}",
        physics.forward_speed
    );
    // 0.1 rad, not the 0.05 the geometry happens to land inside: the ship
    // overshoots the target's z during its ~3 s turn and settles a hair
    // PAST π/2 aiming back, ~0.04 rad off. That margin is deterministic but
    // coincidental — the claim is "turned from 0.0 onto the objective's
    // bearing", not "landed within 0.04 of it".
    assert!(
        (physics.yaw - std::f32::consts::FRAC_PI_2).abs() < 0.1,
        "yaw must turn from the run-in heading (0.0) onto the objective's \
         bearing (π/2); got {}",
        physics.yaw
    );
    assert!(
        distance_to_target(&app) < distance_on_arrival - 300.0,
        "the ship must close on the objective after the run-in completes; \
         was {distance_on_arrival} on arrival, now {}",
        distance_to_target(&app)
    );
}

/// The divert is not a hole in collision avoidance. Issue #1012 added a
/// SECOND `low_lod_objective_steer` call site, and the `low_lod_avoid_yaw`
/// bend that follows it has to still apply there — otherwise a wave that
/// diverts onto its Destroy flies through whatever lies between it and the
/// station, which is the failure issue #968 fixed on the other branches.
///
/// Run twice on identical geometry, once with a rock parked on the divert
/// bearing, and compare. An absolute yaw assertion could not carry this
/// claim: the ~3 s slew onto the objective's bearing puts yaw far from π/2
/// all by itself. The two runs must also be bit-identical over the run-in,
/// which pins the divergence to the divert rather than to the route branch.
#[test]
fn low_lod_divert_still_avoids_a_hazard_on_the_objective_bearing() {
    const RUN_IN: [f32; 3] = [600.0, 0.0, 100.0];
    const TARGET: [f32; 3] = [2600.0, 0.0, 100.0];
    // The run-in takes ~9 s; sample well inside it for the "same route,
    // either way" half of the comparison.
    const RUN_IN_TICKS: usize = 80;

    // Yaw after every tick, so the comparison sees the whole manoeuvre
    // rather than one instant the ship may already have flown past.
    let run = |rock: Option<crate::ai::AiWorldEntity>| -> Vec<f32> {
        let mut app = build_lod_test_app();
        let mut world = crate::world::config::WorldConfig::default();
        world
            .anchors
            .insert("harrow_assault_point".to_string(), RUN_IN);
        app.insert_resource(world);
        let mut snapshot = snapshot_with_named_entity("starbase-alpha", TARGET);
        snapshot.entities.extend(rock);
        app.insert_resource(snapshot);
        spawn_player(&mut app, 0.0, 0.0);

        let npc = app
            .world_mut()
            .spawn((
                Ship,
                Transform::from_xyz(600.0, 0.0, 400.0),
                ShipPhysics {
                    x: 600.0,
                    z: 400.0,
                    ..Default::default()
                },
                AiProfile {
                    aggression: 0.5,
                    sensor_range: 10.0,
                    ..Default::default()
                },
                EntityUuid("wave_1".to_string()),
                BehaviourSection(BehaviourConfig::default()),
                ObjectiveCursors::default(),
                with_reach_objective(
                    blackboards_with_destroy_pool(&[("assault-starbase", 50.0, "starbase-alpha")]),
                    "close-on-starbase",
                    35.0,
                    "harrow_assault_point",
                ),
                test_helm_section(100.0, 1.0),
            ))
            .id();

        (0..200)
            .map(|_| {
                tick_with_dt(&mut app, 0.1);
                app.world().get::<ShipPhysics>(npc).unwrap().yaw
            })
            .collect()
    };

    let clear = run(None);
    // Downrange of the divert and 300 units clear of the run-in leg, so it
    // is out of reach until the ship turns onto the objective's bearing.
    // Its own uuid must differ from the fixture ship's (which resolves to
    // nil — `EntityUuid("wave_1")` does not parse) or `avoidance_steering`
    // would skip it as the ship's own snapshot entry.
    let obstructed = run(Some(crate::ai::AiWorldEntity {
        uuid: uuid::Uuid::from_u128(0x0c1a),
        position: [900.0, 0.0, 20.0],
        radius: 40.0,
        ..Default::default()
    }));

    assert_eq!(
        clear[..RUN_IN_TICKS],
        obstructed[..RUN_IN_TICKS],
        "test setup: the rock must be irrelevant until the divert — a \
         difference during the run-in would mean this test pins the route \
         branch's avoidance, which is already covered"
    );
    let max_divergence = clear
        .iter()
        .zip(&obstructed)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0_f32, f32::max);
    assert!(
        max_divergence > 0.1,
        "a hazard on the divert bearing must bend the heading off it — the \
         avoidance pass has to run on the #1012 branch too; the largest \
         difference from the clear run was {max_divergence} rad"
    );
}

/// The other half of the divert: with nothing *scored* in the Destroy pool,
/// a completed route still parks the ship exactly as it did before #1012.
/// This is the Requiem Courier case — a hull whose whole behaviour is one
/// `Reach` — and the regression #1012's change could most easily cause.
///
/// The pool holds a zero-scored `Destroy` naming an entity the snapshot
/// really does carry, rather than being empty: an empty pool cannot tell
/// "`active_destroy_target`'s `score > 0.0` filter held" from "there was
/// nothing to filter", and the divert's brand-new call site would never see
/// a zero-scored Destroy at all. Post-#1010 that is the live case — an
/// assault wave under fire has its `assault-starbase` gated to 0 — so a
/// wave that has flown its run-in and then taken a hit must park, not
/// charge on at a target its own doctrine has given up on.
#[test]
fn low_lod_ship_still_parks_on_a_completed_route_with_a_zero_gated_destroy() {
    const RUN_IN: [f32; 3] = [600.0, 0.0, 100.0];
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    world
        .anchors
        .insert("harrow_assault_point".to_string(), RUN_IN);
    app.insert_resource(world);
    // The Destroy's target IS in the snapshot and 2000 units away: the park
    // must come from the zero gate alone, not from a target that fails to
    // resolve or a snapshot with nothing in it.
    app.insert_resource(snapshot_with_named_entity(
        "starbase-alpha",
        [2600.0, 0.0, 100.0],
    ));
    spawn_player(&mut app, 0.0, 0.0);

    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(600.0, 0.0, 400.0),
            ShipPhysics {
                x: 600.0,
                z: 400.0,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range: 10.0,
                ..Default::default()
            },
            EntityUuid("courier".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            ObjectiveCursors::default(),
            with_reach_objective(
                blackboards_with_destroy_pool(&[("assault-starbase", 0.0, "starbase-alpha")]),
                "close-on-starbase",
                35.0,
                "harrow_assault_point",
            ),
            test_helm_section(100.0, 1.0),
        ))
        .id();

    for _ in 0..450 {
        tick_with_dt(&mut app, 0.1);
    }

    let arrived = *app.world().get::<ShipPhysics>(npc).unwrap();
    assert_eq!(
        arrived.forward_speed, 0.0,
        "with nothing scored to fly at, a finished route must still coast \
         to a stop"
    );
    for _ in 0..50 {
        tick_with_dt(&mut app, 0.1);
    }
    let later = *app.world().get::<ShipPhysics>(npc).unwrap();
    assert_eq!(
        (later.x, later.z),
        (arrived.x, arrived.z),
        "a parked ship must hold station, not resume drifting"
    );
    // The 20-unit arrival radius plus the coast: 40 u/s (max_speed * the
    // 0.4 route fraction) shed at LOW_LOD_ACCEL_PER_SEC = 10 u/s² is 80
    // units. Unchanged by #1012 — the bound is "parked at the anchor",
    // not "sailed across the map at cruise".
    let drift = ((later.x - RUN_IN[0]).powi(2) + (later.z - RUN_IN[2]).powi(2)).sqrt();
    assert!(
        drift < 120.0,
        "the ship must park near the anchor it arrived at, {drift} units off"
    );
}

/// AC: the zero-gate interaction survives the relaxed first-Low gate.
///
/// The no-timer twin of
/// `demoted_attacked_ship_does_not_steer_toward_a_zero_gated_destroy_target`:
/// post-#1010 an attacked wave's `assault-starbase` scores 0, and
/// `active_destroy_target` filters on `score > 0.0`. A first-Low ship under
/// fire must therefore resolve nothing, keep the pre-#933 exemption, and
/// drift on untouched — neither the turn nor the cruise ramp may fire.
#[test]
fn first_low_ship_does_not_steer_toward_a_zero_gated_destroy_target() {
    let mut app = build_lod_test_app();
    app.insert_resource(snapshot_with_named_entity(
        "starbase-alpha",
        [1500.0, 0.0, 0.0],
    ));
    // Far away, so the ship under test is never promoted off this path —
    // a promoted ship would hold yaw and speed for want of being simulated
    // at all, and pass this test vacuously.
    spawn_player(&mut app, 0.0, 50_000.0);

    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 0.0),
            ShipPhysics {
                forward_speed: 10.0,
                yaw: 0.0,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range: 10.0,
                ..Default::default()
            },
            BehaviourSection(BehaviourConfig::default()),
            // The gate has already fired: this ship has taken a hit.
            blackboards_with_destroy_pool(&[("assault-starbase", 0.0, "starbase-alpha")]),
            test_helm_section(100.0, 1.0),
        ))
        .id();

    for _ in 0..100 {
        tick_with_dt(&mut app, 0.1);
    }

    let physics = *app.world().get::<ShipPhysics>(npc).unwrap();
    assert!(
        physics.yaw.abs() < 1e-6,
        "a zero-scored Destroy is not authored intent and must not turn a \
         first-Low ship; yaw was {}",
        physics.yaw
    );
    assert_eq!(
        physics.forward_speed, 10.0,
        "with nothing resolving, the first-Low exemption stands and the \
         cruise ramp must not fire either"
    );
    assert!(
        physics.x.abs() < 1e-3,
        "the ship must still be drifting straight down -Z, x was {}",
        physics.x
    );
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_none(),
        "guard against a vacuous pass: a promoted ship is not simulated by \
         this path at all and would hold yaw and speed for the wrong reason"
    );
}

/// A ship with no `AiProfile` at all is outside low-LOD authoring entirely
/// and keeps its unmodified drift, scored `Destroy` or not. Pins the one
/// arm of the gate #1012 deliberately did not widen.
#[test]
fn first_low_ship_without_an_ai_profile_keeps_its_unmodified_drift() {
    let mut app = build_lod_test_app();
    app.insert_resource(snapshot_with_named_entity(
        "starbase-alpha",
        [1500.0, 0.0, 0.0],
    ));

    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(0.0, 0.0, 0.0),
            ShipPhysics {
                forward_speed: 10.0,
                yaw: 0.0,
                ..Default::default()
            },
            BehaviourSection(BehaviourConfig::default()),
            blackboards_with_destroy_pool(&[("assault-starbase", 50.0, "starbase-alpha")]),
            test_helm_section(100.0, 1.0),
        ))
        .id();

    for _ in 0..100 {
        tick_with_dt(&mut app, 0.1);
    }

    let physics = *app.world().get::<ShipPhysics>(npc).unwrap();
    assert!(
        physics.yaw.abs() < 1e-6,
        "no AiProfile means no authored low-LOD tuning to steer by; yaw \
         was {}",
        physics.yaw
    );
    assert_eq!(
        physics.forward_speed, 10.0,
        "no AiProfile means no cruise fraction to ramp toward either"
    );
    assert!(
        (physics.z + 100.0).abs() < 0.1,
        "10 u/s down -Z for 10 s is z = -100; got {}",
        physics.z
    );
}

// ── Low-LOD patrol wiring (ObjectiveCursors / advance_cursor) ───────────────

/// Build a `ShipSystemBlackboards` carrying a single Helm-relevant Patrol
/// objective under the viewscreen entry (mirrors what
/// `aggregate_doctrine_blackboards` publishes for a patrolling ship).
fn blackboards_with_patrol(
    id: &str,
    waypoints: &[&str],
    loop_path: bool,
) -> crate::server_app::ShipSystemBlackboards {
    let mut bb = crate::server_app::ShipSystemBlackboards::default();
    bb.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        crate::core::messages::SystemBlackboard::Viewscreen(
            crate::core::messages::ViewscreenBlackboard {
                scored_objectives: vec![crate::core::messages::ScoredObjective {
                    id: id.to_string(),
                    score: 1.0,
                    directive: crate::core::messages::AiDirective::Patrol {
                        anchors: waypoints.iter().map(|w| w.to_string()).collect(),
                        loop_path,
                    },
                    source: crate::core::messages::ObjectiveSource::Doctrine,
                    relevance: vec![crate::core::messages::SystemAffinity::Helm],
                    snapshot: crate::core::messages::ObjectiveSnapshot {
                        id: id.to_string(),
                        text: "Patrol".to_string(),
                        text_params: Default::default(),
                        mandatory: false,
                        status: crate::core::messages::ObjectiveStatus::Active,
                        targets: vec![],
                        source: crate::core::messages::ObjectiveSource::Doctrine,
                    },
                }],
                ..Default::default()
            },
        ),
    );
    bb
}

/// Build a `ShipSystemBlackboards` carrying an already-scored Destroy pool
/// (mirrors what `aggregate_doctrine_blackboards` + `score_doctrine_pool`
/// publish for a ship's standing Destroy doctrine, `zero_gates` already
/// applied). Entries are given in authoring order so a test can put a
/// low-/zero-scoring entry first and a higher-scoring one after it —
/// exactly the shape the #933 review follow-up caught: the low-LOD Destroy
/// steer must resolve by score, not by position in this slice.
fn blackboards_with_destroy_pool(
    entries: &[(&str, f32, &str)],
) -> crate::server_app::ShipSystemBlackboards {
    let mut bb = crate::server_app::ShipSystemBlackboards::default();
    bb.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        crate::core::messages::SystemBlackboard::Viewscreen(
            crate::core::messages::ViewscreenBlackboard {
                scored_objectives: entries
                    .iter()
                    .map(
                        |(id, score, target)| crate::core::messages::ScoredObjective {
                            id: id.to_string(),
                            score: *score,
                            directive: crate::core::messages::AiDirective::Destroy {
                                target: target.to_string(),
                            },
                            source: crate::core::messages::ObjectiveSource::Doctrine,
                            relevance: vec![crate::core::messages::SystemAffinity::Helm],
                            snapshot: crate::core::messages::ObjectiveSnapshot {
                                id: id.to_string(),
                                text: "Destroy".to_string(),
                                text_params: Default::default(),
                                mandatory: false,
                                status: crate::core::messages::ObjectiveStatus::Active,
                                targets: vec![],
                                source: crate::core::messages::ObjectiveSource::Doctrine,
                            },
                        },
                    )
                    .collect(),
                ..Default::default()
            },
        ),
    );
    bb
}

/// Push a Helm-relevant `Reach` entry onto an existing fixture blackboard.
///
/// Composes with [`blackboards_with_destroy_pool`] to author the MIXED
/// doctrine `combat_test.toml`'s Harrow assault waves carry — a
/// high-scoring `Destroy` on the starbase plus a lower-scoring `Reach` on
/// the run-in anchor. That mix is the whole of issue #1012:
/// `active_waypoint_route` filters to Patrol/Reach and so only ever sees
/// the Reach, which used to let a completed run-in park the wave with its
/// top-scoring directive unserved.
fn with_reach_objective(
    mut bb: crate::server_app::ShipSystemBlackboards,
    id: &str,
    score: f32,
    anchor: &str,
) -> crate::server_app::ShipSystemBlackboards {
    if let Some(crate::core::messages::SystemBlackboard::Viewscreen(v)) =
        bb.0.get_mut(&crate::ship::system_registry::viewscreen_system_id())
    {
        v.scored_objectives
            .push(crate::core::messages::ScoredObjective {
                id: id.to_string(),
                score,
                directive: crate::core::messages::AiDirective::Reach {
                    anchor: anchor.to_string(),
                },
                source: crate::core::messages::ObjectiveSource::Doctrine,
                relevance: vec![crate::core::messages::SystemAffinity::Helm],
                snapshot: crate::core::messages::ObjectiveSnapshot {
                    id: id.to_string(),
                    text: "Reach".to_string(),
                    text_params: Default::default(),
                    mandatory: false,
                    status: crate::core::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::core::messages::ObjectiveSource::Doctrine,
                },
            });
    }
    bb
}

/// Spawn a low-LOD patrolling NPC at `(x, z)` carrying the TOML-authored
/// `BehaviourSection` the cursor evaluator reads its arrival radius from.
fn spawn_patrolling_npc(
    app: &mut App,
    x: f32,
    z: f32,
    uuid: &str,
    objective_id: &str,
    waypoints: &[&str],
    loop_path: bool,
) -> Entity {
    app.world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(x, 0.0, z),
            ShipPhysics {
                x,
                z,
                forward_speed: 10.0,
                yaw: 0.0,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range: 100.0,
                ..Default::default()
            },
            EntityUuid(uuid.to_string()),
            BehaviourSection(BehaviourConfig::default()),
            ObjectiveCursors::default(),
            blackboards_with_patrol(objective_id, waypoints, loop_path),
        ))
        .id()
}

/// Every cursor on `entity` as `(objective_id, waypoint_index)`.
fn cursor_state(app: &App, entity: Entity) -> Vec<(String, usize)> {
    app.world()
        .get::<ObjectiveCursors>(entity)
        .unwrap()
        .0
        .iter()
        .map(|c| (c.objective_id.clone(), c.index()))
        .collect()
}

#[test]
fn cursor_advances_when_ship_arrives_at_its_waypoint() {
    let mut app = build_lod_test_app();
    // Ship starts AT wp0, so it arrives immediately; wp1 is 200 units away.
    let mut world = crate::world::config::WorldConfig::default();
    world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    let npc = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-1",
        "patrol",
        &["wp0", "wp1"],
        true,
    );

    tick_with_dt(&mut app, 0.1);

    assert_eq!(
        cursor_state(&app, npc),
        vec![("patrol".to_string(), 1)],
        "cursor must advance to waypoint 1 after arriving at waypoint 0"
    );
}

#[test]
fn cursor_does_not_advance_while_ship_is_short_of_its_waypoint() {
    let mut app = build_lod_test_app();
    // wp0 sits 200 units away — far outside the default arrival radius.
    let mut world = crate::world::config::WorldConfig::default();
    world.anchors.insert("wp0".to_string(), [700.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [900.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    let npc = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-1",
        "patrol",
        &["wp0", "wp1"],
        true,
    );

    tick_with_dt(&mut app, 0.1);

    assert_eq!(
        cursor_state(&app, npc),
        vec![("patrol".to_string(), 0)],
        "cursor must stay on waypoint 0 until the ship reaches it"
    );
}

/// The arrival radius is designer-tunable via `[behaviour]
/// waypoint_arrival_radius` in entity TOML — a ship with a wide radius
/// counts as arrived from a distance that a default-radius ship does not.
#[test]
fn arrival_radius_comes_from_the_entity_behaviour_config() {
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    world.anchors.insert("wp0".to_string(), [600.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [900.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    // Both ships sit 100 units from wp0, but only the wide-radius ship
    // is close enough to count as arrived.
    let narrow = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-narrow",
        "patrol",
        &["wp0", "wp1"],
        true,
    );
    let wide = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-wide",
        "patrol",
        &["wp0", "wp1"],
        true,
    );
    app.world_mut()
        .entity_mut(wide)
        .insert(BehaviourSection(BehaviourConfig {
            waypoint_arrival_radius: 150.0,
            ..Default::default()
        }));

    tick_with_dt(&mut app, 0.1);

    assert_eq!(
        cursor_state(&app, narrow),
        vec![("patrol".to_string(), 0)],
        "default arrival radius must not count 100 units away as arrived"
    );
    assert_eq!(
        cursor_state(&app, wide),
        vec![("patrol".to_string(), 1)],
        "a TOML-widened arrival radius must count 100 units away as arrived"
    );
}

#[test]
fn reach_objective_cursor_advances_to_terminal_on_arrival() {
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    world
        .anchors
        .insert("dock".to_string(), [500.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    let mut bb = crate::server_app::ShipSystemBlackboards::default();
    bb.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        crate::core::messages::SystemBlackboard::Viewscreen(
            crate::core::messages::ViewscreenBlackboard {
                scored_objectives: vec![crate::core::messages::ScoredObjective {
                    id: "reach-dock".to_string(),
                    score: 1.0,
                    directive: crate::core::messages::AiDirective::Reach {
                        anchor: "dock".to_string(),
                    },
                    source: crate::core::messages::ObjectiveSource::Mission,
                    relevance: vec![crate::core::messages::SystemAffinity::Helm],
                    snapshot: crate::core::messages::ObjectiveSnapshot {
                        id: "reach-dock".to_string(),
                        text: "Reach the dock".to_string(),
                        text_params: Default::default(),
                        mandatory: false,
                        status: crate::core::messages::ObjectiveStatus::Active,
                        targets: vec![],
                        source: crate::core::messages::ObjectiveSource::Mission,
                    },
                }],
                ..Default::default()
            },
        ),
    );

    // Ship sits on the dock anchor → arrived.
    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(500.0, 0.0, 500.0),
            ShipPhysics {
                x: 500.0,
                z: 500.0,
                forward_speed: 10.0,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range: 100.0,
                ..Default::default()
            },
            EntityUuid("npc-reach".to_string()),
            BehaviourSection(BehaviourConfig::default()),
            ObjectiveCursors::default(),
            bb,
        ))
        .id();

    tick_with_dt(&mut app, 0.1);

    assert_eq!(
        cursor_state(&app, npc),
        vec![("reach-dock".to_string(), 1)],
        "a Reach cursor is a one-waypoint route: arriving moves it to the terminal index"
    );

    // Having arrived, the low-LOD ship coasts to a stop and stays put. It
    // used to fall through to the dumb forward-drift the moment its route
    // went terminal, which sailed the Requiem Courier — a hull whose whole
    // behaviour is one Reach — clean through its destination and out of the
    // scenario at cruise speed.
    for _ in 0..20 {
        tick_with_dt(&mut app, 0.1);
    }
    let arrived = *app.world().get::<ShipPhysics>(npc).unwrap();
    assert_eq!(
        arrived.forward_speed, 0.0,
        "a ship that has flown its route to the end must come to rest"
    );

    for _ in 0..20 {
        tick_with_dt(&mut app, 0.1);
    }
    let later = *app.world().get::<ShipPhysics>(npc).unwrap();
    assert_eq!(
        (later.x, later.z),
        (arrived.x, arrived.z),
        "a stopped ship must hold station, not resume drifting"
    );
    // And it stopped near where it arrived rather than crossing the map.
    let drift = ((later.x - 500.0).powi(2) + (later.z - 500.0).powi(2)).sqrt();
    assert!(
        drift < 10.0,
        "the ship coasted {drift} units past the anchor it arrived at"
    );
}

#[test]
fn low_lod_npc_follows_patrol_route_between_waypoints() {
    let mut app = build_lod_test_app();
    // Ship starts AT wp0; wp1 is offset in +x so the steer is observable
    // in both yaw and position.
    let mut world = crate::world::config::WorldConfig::default();
    world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    let npc = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-1",
        "patrol",
        &["wp0", "wp1"],
        true,
    );

    // Tick 1 arrives at wp0 and advances the cursor to wp1; tick 2 is the
    // first tick that steers toward wp1.
    tick_with_dt(&mut app, 0.1);
    tick_with_dt(&mut app, 0.1);

    // Steering toward wp1 (700,0,500): dx=+200, dz=0 → bearing = π/2.
    let physics = app.world().get::<ShipPhysics>(npc).unwrap();
    assert!(
        (physics.yaw - std::f32::consts::FRAC_PI_2).abs() < 0.01,
        "yaw should steer toward wp1 bearing (π/2), got {}",
        physics.yaw,
    );
    assert!(
        physics.x > 500.0,
        "ship should advance toward wp1 (+x), got x={}",
        physics.x,
    );

    // Must remain low-LOD (never promoted), proving this is the cheap path.
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_none(),
        "patrolling NPC out of range must stay low-LOD"
    );
}

/// End-to-end route following: a low-LOD NPC placed on a two-waypoint
/// looping route drives itself to the far waypoint, wraps back to the
/// first, and returns — without ever being promoted to high LOD.
#[test]
fn low_lod_npc_patrol_route_wraps_around_and_returns_to_first_waypoint() {
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    let npc = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-1",
        "patrol",
        &["wp0", "wp1"],
        true,
    );

    // forward_speed 10 → ~1 unit/tick at dt=0.1, so the 200-unit leg out
    // to wp1 takes ~180 ticks to close to within the 20-unit arrival
    // radius. Run until the cursor wraps back to wp0 (bounded well above
    // that), sampling the cursor each tick to prove the whole cycle.
    let mut seen_indices = Vec::new();
    let mut reached_max_x: f32 = 500.0;
    for _ in 0..600 {
        tick_with_dt(&mut app, 0.1);
        let idx = cursor_state(&app, npc)[0].1;
        if seen_indices.last() != Some(&idx) {
            seen_indices.push(idx);
        }
        reached_max_x = reached_max_x.max(app.world().get::<ShipPhysics>(npc).unwrap().x);
        // Stop on the first wraparound: 0 → 1 → back to 0.
        if seen_indices.len() == 2 {
            break;
        }
    }

    assert_eq!(
        seen_indices,
        vec![1, 0],
        "cursor must advance to wp1, then wrap back to wp0 on a looping route"
    );
    assert!(
        reached_max_x > 680.0,
        "ship must actually travel the leg out to wp1 (x≈700), got max x={}",
        reached_max_x,
    );

    // Steering reads the cursor before the evaluator advances it, so the
    // turn toward wp0 happens on the tick *after* the wrap.
    tick_with_dt(&mut app, 0.1);

    // Having wrapped, it is heading back toward wp0 (-x) → bearing = -π/2.
    let physics = app.world().get::<ShipPhysics>(npc).unwrap();
    assert!(
        (physics.yaw + std::f32::consts::FRAC_PI_2).abs() < 0.01,
        "after wraparound the ship should steer back toward wp0 (-π/2), got {}",
        physics.yaw,
    );
    assert!(
        physics.x < reached_max_x,
        "ship must be travelling back toward wp0 after the wrap"
    );
    assert!(
        app.world().get::<AiHighFidelity>(npc).is_none(),
        "patrolling NPC out of range must stay low-LOD for the whole route"
    );
}

/// The arrival that advances the cursor is announced as an
/// `AiWaypointReached` message — the bridge the world plugin turns into a
/// `WorldEvent::WaypointReached` for `on_waypoint_reached` triggers.
#[test]
fn reaching_a_waypoint_emits_ai_waypoint_reached() {
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [700.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-1",
        "patrol",
        &["wp0", "wp1"],
        true,
    );

    tick_with_dt(&mut app, 0.1);

    let messages = app
        .world()
        .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
    let mut cursor = messages.get_cursor();
    let emitted: Vec<_> = cursor.read(messages).collect();

    assert_eq!(
        emitted.len(),
        1,
        "arriving at wp0 must announce exactly once"
    );
    assert_eq!(emitted[0].entity_uuid, "npc-1");
    assert_eq!(emitted[0].objective_id, "patrol");
    assert_eq!(
        emitted[0].waypoint, "wp0",
        "the announced waypoint must be the one arrived at, not the next one"
    );
}

/// Read every `AiWaypointReached` emitted so far, as `(uuid, waypoint)`.
fn reached_waypoints(app: &App) -> Vec<(String, String)> {
    let messages = app
        .world()
        .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
    let mut cursor = messages.get_cursor();
    cursor
        .read(messages)
        .map(|m| (m.entity_uuid.clone(), m.waypoint.clone()))
        .collect()
}

/// Regression: a tick that carries the cursor past several waypoints at
/// once must announce every one of them. With `wp0` and `wp1` spaced
/// closer than the arrival radius, the cursor jumps 0 → 2 in a single
/// tick; announcing only `wp0` would leave an `on_waypoint_reached`
/// trigger keyed to `wp1` silently dead.
#[test]
fn one_message_per_waypoint_consumed_when_a_tick_skips_several() {
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    // wp0 and wp1 are 5 units apart — well inside the 20-unit default
    // arrival radius — while wp2 is a long leg away.
    world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [505.0, 0.0, 500.0]);
    world.anchors.insert("wp2".to_string(), [700.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    let npc = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-1",
        "patrol",
        &["wp0", "wp1", "wp2"],
        true,
    );

    tick_with_dt(&mut app, 0.1);

    assert_eq!(
        reached_waypoints(&app),
        vec![
            ("npc-1".to_string(), "wp0".to_string()),
            ("npc-1".to_string(), "wp1".to_string()),
        ],
        "both waypoints consumed this tick must be announced, in route order"
    );
    assert_eq!(
        cursor_state(&app, npc),
        vec![("patrol".to_string(), 2)],
        "cursor must land on the far wp2 after skipping wp0 and wp1"
    );
}

/// Regression: a looping route whose every waypoint sits inside the
/// arrival radius closes its lap immediately. Any route with legs shorter
/// than the authored `waypoint_arrival_radius` does this — a designer
/// widening the radius for a station-keeping patrol, not just a
/// pathological case.
///
/// The contract has three parts, and the second and third are what the
/// original permanent-retirement design broke: the ship announced its lap
/// and then lost its cursor entirely, fell through to the dumb
/// forward-move, and flew out of the cluster in a straight line forever
/// with no way back.
#[test]
fn looping_route_entirely_inside_arrival_radius_announces_once_then_holds_station() {
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    // All three anchors are within the 20-unit default arrival radius of
    // the ship's spawn point.
    world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [505.0, 0.0, 500.0]);
    world.anchors.insert("wp2".to_string(), [500.0, 0.0, 505.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    let npc = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-1",
        "patrol",
        &["wp0", "wp1", "wp2"],
        true,
    );

    // First tick: the lap closes — each waypoint announced exactly once.
    tick_with_dt(&mut app, 0.1);
    assert_eq!(
        reached_waypoints(&app),
        vec![
            ("npc-1".to_string(), "wp0".to_string()),
            ("npc-1".to_string(), "wp1".to_string()),
            ("npc-1".to_string(), "wp2".to_string()),
        ],
        "the closing lap must announce each waypoint exactly once"
    );

    // ── 2. No per-tick spam, and the ship holds station ────────────────
    // Drain, then tick long enough that a ship flying off at
    // forward_speed (1 unit/tick here) would be 200 units clear of the
    // cluster.
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<AiWaypointReached>>()
        .clear();
    for _ in 0..200 {
        tick_with_dt(&mut app, 0.1);
    }

    assert!(
        reached_waypoints(&app).is_empty(),
        "a settled degenerate route must not re-announce its waypoints every tick"
    );
    assert_eq!(
        cursor_state(&app, npc),
        vec![("patrol".to_string(), 0)],
        "the cursor must stay on a real waypoint index, not a sentinel"
    );
    let physics = app.world().get::<ShipPhysics>(npc).unwrap();
    let drift = ((physics.x - 500.0).powi(2) + (physics.z - 500.0).powi(2)).sqrt();
    assert!(
        drift < 20.0,
        "the ship must keep station on its route, not fly out of the cluster: \
         drifted {} units to ({}, {})",
        drift,
        physics.x,
        physics.z,
    );

    // ── 3. Moved out of the radius, the route resumes ──────────────────
    // Shove the ship 2000 units clear (a knockback, tow or scenario
    // teleport does the same thing).
    {
        let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
        physics.x = 2500.0;
        physics.z = 500.0;
    }
    tick_with_dt(&mut app, 0.1);

    assert!(
        reached_waypoints(&app).is_empty(),
        "nothing was arrived at 2000 units out"
    );
    let physics = app.world().get::<ShipPhysics>(npc).unwrap();
    assert!(
        (physics.yaw + std::f32::consts::FRAC_PI_2).abs() < 0.01,
        "the resumed route must steer back toward wp0 (-π/2), got {}",
        physics.yaw,
    );
    assert!(
        physics.x < 2500.0,
        "the ship must fly back toward its route, got x={}",
        physics.x,
    );

    // Back in the cluster, the lap is flown and announced afresh — the
    // route is alive, not permanently dead.
    {
        let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
        physics.x = 500.0;
        physics.z = 500.0;
    }
    tick_with_dt(&mut app, 0.1);

    assert_eq!(
        reached_waypoints(&app),
        vec![
            ("npc-1".to_string(), "wp0".to_string()),
            ("npc-1".to_string(), "wp1".to_string()),
            ("npc-1".to_string(), "wp2".to_string()),
        ],
        "a route resumed after leaving the arrival radius must announce again"
    );
}

/// Regression (issue #696 review): the shipped-content shape of the bug.
/// `waypoint_arrival_radius` is designer-tunable per entity, so a route
/// whose legs are shorter than the authored radius is ordinary content —
/// a station-keeping patrol. It must not silently become "fly off the map
/// in a straight line, forever".
#[test]
fn route_with_legs_shorter_than_the_authored_radius_does_not_die() {
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    // 100-unit legs against a 150-unit authored radius.
    world.anchors.insert("wp0".to_string(), [500.0, 0.0, 500.0]);
    world.anchors.insert("wp1".to_string(), [600.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    let npc = spawn_patrolling_npc(
        &mut app,
        500.0,
        500.0,
        "npc-1",
        "patrol",
        &["wp0", "wp1"],
        true,
    );
    app.world_mut()
        .entity_mut(npc)
        .insert(BehaviourSection(BehaviourConfig {
            waypoint_arrival_radius: 150.0,
            ..Default::default()
        }));

    // The lap closes on tick 1: both waypoints are inside the radius.
    tick_with_dt(&mut app, 0.1);
    assert_eq!(
        reached_waypoints(&app),
        vec![
            ("npc-1".to_string(), "wp0".to_string()),
            ("npc-1".to_string(), "wp1".to_string()),
        ],
        "the closing lap announces each waypoint once"
    );

    // 400 ticks at 1 unit/tick: a ship that lost its cursor would be 400
    // units clear by now. This one is still on its route.
    for _ in 0..400 {
        tick_with_dt(&mut app, 0.1);
    }
    let physics = app.world().get::<ShipPhysics>(npc).unwrap();
    let dist_to_wp0 = ((physics.x - 500.0).powi(2) + (physics.z - 500.0).powi(2)).sqrt();
    assert!(
        dist_to_wp0 < 150.0,
        "the ship must hold its route, not fly off at forward_speed: {} units out",
        dist_to_wp0,
    );
    assert_eq!(
        cursor_state(&app, npc),
        vec![("patrol".to_string(), 0)],
        "the cursor must still name a real waypoint"
    );

    // And the route is resumable: shoved clear, it steers back.
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<AiWaypointReached>>()
        .clear();
    {
        let mut physics = app.world_mut().get_mut::<ShipPhysics>(npc).unwrap();
        physics.x = 3000.0;
        physics.z = 500.0;
    }
    tick_with_dt(&mut app, 0.1);
    let physics = app.world().get::<ShipPhysics>(npc).unwrap();
    assert!(
        physics.x < 3000.0,
        "a resumed route must fly the ship back toward wp0, got x={}",
        physics.x,
    );
    assert!(
        !app.world().get::<ObjectiveCursors>(npc).unwrap().0[0].settled(),
        "leaving the arrival radius must un-settle the cursor"
    );
}

#[test]
fn no_waypoint_reached_message_while_ship_is_short_of_its_waypoint() {
    let mut app = build_lod_test_app();
    let mut world = crate::world::config::WorldConfig::default();
    world.anchors.insert("wp0".to_string(), [700.0, 0.0, 500.0]);
    app.insert_resource(world);
    spawn_player(&mut app, 0.0, 0.0);

    spawn_patrolling_npc(&mut app, 500.0, 500.0, "npc-1", "patrol", &["wp0"], false);

    tick_with_dt(&mut app, 0.1);

    let messages = app
        .world()
        .resource::<bevy::ecs::message::Messages<AiWaypointReached>>();
    let mut cursor = messages.get_cursor();
    assert_eq!(
        cursor.read(messages).count(),
        0,
        "no arrival must be announced while the ship is still 200 units out"
    );
}

/// Issue #968: the low-LOD hazard assessment reasons about the ship's OWN
/// hull and its OWN authored standoff, not about a point with the module
/// default.
///
/// `low_lod_avoid_yaw` used to build its `WorldView` from
/// `WorldView::default()`, which zeroes `self_radius`, and to pass the
/// parse-default `AVOIDANCE_BUFFER` whatever the hull authored. Both push a
/// demoted ship's reaction late by exactly the margin it needed: the two
/// Harrow pickets in `combat_test` ground 3.2 units into radius-4 rocks,
/// were kicked back to the surface once a second by the collision response,
/// and drove straight back in.
///
/// The obstacle here sits 10 units off the ship's 3-second projection, so it
/// is outside a point-model's `0 + 4 + 5` avoidance radius and inside the
/// radius a 3-unit hull (or a hull authoring a wider buffer) actually needs.
#[test]
fn low_lod_avoidance_uses_the_hulls_own_radius_and_authored_buffer() {
    const ROCK_RADIUS: f32 = 4.0;
    const DEFAULT_BUFFER: f32 = 5.0;

    let snapshot = WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: uuid::Uuid::from_u128(1),
            // 10 units from the projected position (0, 0, -9): dx = 6, dz = -8.
            position: [6.0, 0.0, -17.0],
            radius: ROCK_RADIUS,
            ..Default::default()
        }],
    };
    // Facing -Z at 3 u/s, so the 3-second look-ahead projects to z = -9.
    let avoid = |self_radius: f32, buffer: f32| {
        let behaviour = crate::entities::config::BehaviourConfig {
            avoidance_buffer: buffer,
            ..Default::default()
        };
        low_lod_avoid_yaw(
            0.0,
            [0.0, 0.0, 0.0],
            3.0,
            self_radius,
            uuid::Uuid::from_u128(99),
            &behaviour,
            Some(&snapshot),
        )
    };

    assert_eq!(
        avoid(0.0, DEFAULT_BUFFER),
        0.0,
        "control: as a point with the default buffer, this obstacle is out of \
         range and the ship holds its heading"
    );
    assert_ne!(
        avoid(3.0, DEFAULT_BUFFER),
        0.0,
        "a 3-unit hull needs 3 more units of clearance than a point, and must \
         bend its heading for an obstacle a point would ignore"
    );
    assert_ne!(
        avoid(0.0, 20.0),
        0.0,
        "a hull authoring a wider avoidance buffer must react earlier at low \
         fidelity too — the authored value has to reach this path"
    );
}

/// Issue #968: the dead-reckoned deviation ceiling is AUTHORED, and the
/// deviation actually applied scales with threat up to it.
///
/// The 90° default is geometry (the tangent at contact), but the ramp to it
/// is not — `threat × ceiling` is a proportional bend, not the tangent angle
/// for the distance in hand — so a hull may author its own. This also pins
/// what the constant's note warns about: the magnitude no longer passes
/// through the hull's `max_yaw_rate` at all.
#[test]
fn the_low_lod_deviation_ceiling_is_authored_per_hull() {
    // Directly ahead and close enough to saturate: the ship's projected
    // point (0, 0, -9) is well inside a radius-4 rock's skin at z = -10,
    // so the threat fraction is 1.0 and the deviation is the whole ceiling.
    let snapshot = WorldSnapshot {
        entities: vec![crate::ai::AiWorldEntity {
            uuid: uuid::Uuid::from_u128(1),
            position: [0.5, 0.0, -10.0],
            radius: 4.0,
            ..Default::default()
        }],
    };
    let avoid = |ceiling: f32| {
        let behaviour = crate::entities::config::BehaviourConfig {
            low_lod_avoidance_deviation_rad: ceiling,
            ..Default::default()
        };
        low_lod_avoid_yaw(
            0.0,
            [0.0, 0.0, 0.0],
            3.0,
            1.0,
            uuid::Uuid::from_u128(99),
            &behaviour,
            Some(&snapshot),
        )
    };

    let default_turn = avoid(crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD);
    assert!(
        (default_turn.abs() - crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD).abs() < 1e-4,
        "a saturated hazard must bend the heading by the whole authored \
         ceiling, got {default_turn}"
    );

    // Half the ceiling, same geometry: the authored value is what scales it.
    let half = avoid(crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD / 2.0);
    assert!(
        (half.abs() - crate::ai::LOW_LOD_AVOIDANCE_DEVIATION_RAD / 2.0).abs() < 1e-4,
        "a hull authoring half the ceiling must deviate half as far, got {half}"
    );
    assert_eq!(
        half.signum(),
        default_turn.signum(),
        "the authored ceiling must scale the turn, not reverse it"
    );

    // A hull that authors no deviation at all holds its route bearing.
    assert_eq!(
        avoid(0.0),
        0.0,
        "a zero ceiling means a hull that never leaves its route"
    );
}

#[test]
fn low_lod_without_patrol_objective_keeps_dumb_forward_move() {
    let mut app = build_lod_test_app();
    spawn_player(&mut app, 0.0, 0.0);

    // NPC carries ObjectiveCursors + an (empty) blackboard but NO patrol
    // objective — it must fall back to the pre-existing dumb forward-move.
    let npc = app
        .world_mut()
        .spawn((
            Ship,
            Transform::from_xyz(500.0, 0.0, 0.0),
            ShipPhysics {
                x: 500.0,
                z: 0.0,
                forward_speed: 10.0,
                yaw: 0.0,
                ..Default::default()
            },
            AiProfile {
                aggression: 0.5,
                sensor_range: 100.0,
                ..Default::default()
            },
            ObjectiveCursors::default(),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();

    let initial = *app.world().get::<ShipPhysics>(npc).unwrap();
    tick_with_dt(&mut app, 0.1);
    let physics = app.world().get::<ShipPhysics>(npc).unwrap();

    // yaw=0, forward_speed=10, dt=0.1 → z advances by -1, x unchanged.
    assert!(
        (physics.z - (initial.z - 1.0)).abs() < 0.001,
        "no-patrol NPC z should advance by forward_speed * dt: expected {}, got {}",
        initial.z - 1.0,
        physics.z,
    );
    assert!(
        (physics.x - initial.x).abs() < 0.001,
        "no-patrol NPC x should not change when yaw=0: expected {}, got {}",
        initial.x,
        physics.x,
    );
    // Cursor stays empty — nothing was advanced.
    assert!(
        app.world()
            .get::<ObjectiveCursors>(npc)
            .unwrap()
            .0
            .is_empty(),
        "cursor must stay empty when there is no patrol objective"
    );
}

use super::*;
use crate::core::messages::ModifierSlot;
use crate::entities::config::EntityConfig;
use crate::entities::spawner::spawn_entity;
use crate::modifiers::ShipModifiers;
use crate::regions::effects::{BlocksImpulseEffect, RadarDampeningEffect, SlowZoneEffect};
use crate::regions::shape::RegionShape;
use crate::server_app::ShipImpulse;
use crate::ship::damage::SystemHull;
use crate::ship::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
use crate::ship::physics::ShipPhysicsConfig;

/// Build a minimal Bevy app with the real `RegionPlugin`, one fixed
/// simulation step per `update()` (issue #895).
fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .add_plugins(RegionPlugin);
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        crate::ship::test_support::TEST_TICK,
    );
    // Spawn the ship entity with ShipPhysics and ShipModifiers so region systems can query it.
    app.world_mut().spawn((
        LocalShip,
        crate::server_app::Ship,
        Transform::default(),
        crate::ship::state::ShipPhysics::default(),
        ShipModifiers::new(),
    ));
    app
}

/// Spawn a region entity at the given position with the given shape.
///
/// Does NOT call `app.update()` - the caller drives the membership
/// system itself. It DOES flush the world command queue, which matters
/// since issue #895: `spawn_entity` queues through `Commands`, and that
/// queue is applied later in the frame than `FixedUpdate`, so without
/// the flush the region would not exist yet on the fixed step the
/// caller's next `update()` runs.
fn spawn_region(app: &mut App, x: f32, z: f32, shape: RegionShape) -> Entity {
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
        tags: vec!["region".to_string()],
        shape: Some(shape),
        effects: None,
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
        target: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let mut commands = app.world_mut().commands();
    let entity = spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None);
    app.world_mut().flush();
    entity
}

fn ship_entity(app: &mut App) -> Entity {
    let mut query = QueryState::<Entity, With<LocalShip>>::new(app.world_mut());
    query.iter(app.world()).next().unwrap()
}

fn is_inside(app: &mut App, region: Entity) -> bool {
    let ship = ship_entity(app);
    app.world()
        .resource::<RegionMembership>()
        .inside
        .get(&ship)
        .is_some_and(|set| set.contains(&region))
}

fn set_ship_pos(app: &mut App, x: f32, z: f32) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::ship::state::ShipPhysics, With<LocalShip>>();
    let mut physics = q
        .single_mut(app.world_mut())
        .expect("expected LocalShip with ShipPhysics");
    physics.x = x;
    physics.z = z;
}

// ── Entry tests ───────────────────────────────────────────────────

#[test]
fn ship_enters_region_when_moving_inside() {
    let mut app = test_app();
    let region = spawn_region(&mut app, 100.0, 0.0, RegionShape::Sphere { radius: 50.0 });
    // Flush so region entity is queryable + system runs once
    app.update();
    // Ship at (0,0) is outside region at (100,0) with radius 50 → no entry
    assert!(
        !is_inside(&mut app, region),
        "ship should start outside region"
    );

    // Move ship inside the region
    set_ship_pos(&mut app, 120.0, 0.0); // 20 units from centre, well inside radius 50
    app.update();

    assert!(
        is_inside(&mut app, region),
        "ship should enter region when moving inside"
    );
}

// ── Exit tests ────────────────────────────────────────────────────

#[test]
fn ship_exits_region_when_moving_outside() {
    let mut app = test_app();
    let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
    set_ship_pos(&mut app, 20.0, 0.0); // inside
    app.update(); // flush + system run → enters
    assert!(
        is_inside(&mut app, region),
        "ship should be inside after moving in"
    );

    // Move ship outside
    set_ship_pos(&mut app, 100.0, 0.0); // far outside radius 50
    app.update();

    assert!(
        !is_inside(&mut app, region),
        "ship should exit region when moving outside"
    );
}

// ── No-duplicate-while-inside test ────────────────────────────────

#[test]
fn no_duplicate_entered_while_staying_inside() {
    let mut app = test_app();
    let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    app.update(); // flush + system run → enters
    assert!(
        is_inside(&mut app, region),
        "ship should be inside after first tick"
    );

    // Stay inside — tick again; membership should remain stable
    app.update();
    assert!(
        is_inside(&mut app, region),
        "ship should remain inside without duplicate entry"
    );
}

// ── Despawn-implicit-exit test ────────────────────────────────────

#[test]
fn region_despawn_while_inside_emits_implicit_exit() {
    let mut app = test_app();
    let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    app.update(); // flush + system run → enters
    assert!(
        is_inside(&mut app, region),
        "ship should be inside before despawn"
    );

    // Despawn the region entity
    app.world_mut().despawn(region);
    app.update();

    assert!(
        !is_inside(&mut app, region),
        "ship should exit region when region is despawned"
    );
}

// ── Edge: ship outside from start ─────────────────────────────────

#[test]
fn ship_outside_from_start_does_not_enter() {
    let mut app = test_app();
    let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
    set_ship_pos(&mut app, 200.0, 0.0); // far outside
    app.update();

    assert!(
        !is_inside(&mut app, region),
        "ship outside region should not enter"
    );
}

// ── Enter and exit across multiple regions ────────────────────────

#[test]
fn ship_enters_and_exits_two_regions_independently() {
    let mut app = test_app();
    let r1 = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 30.0 });
    let r2 = spawn_region(&mut app, 100.0, 0.0, RegionShape::Sphere { radius: 30.0 });

    // Start ship outside both regions
    set_ship_pos(&mut app, 200.0, 0.0);
    // Flush so both region entities are queryable + first system run
    app.update();
    assert!(!is_inside(&mut app, r1), "should not start in r1");
    assert!(!is_inside(&mut app, r2), "should not start in r2");

    // Ship inside r1, outside r2
    set_ship_pos(&mut app, 10.0, 0.0);
    app.update();

    assert!(is_inside(&mut app, r1), "should enter r1");
    assert!(!is_inside(&mut app, r2), "should NOT enter r2");

    // Move to r2 — should exit r1, enter r2
    set_ship_pos(&mut app, 110.0, 0.0);
    app.update();

    assert!(is_inside(&mut app, r2), "should enter r2");
    assert!(!is_inside(&mut app, r1), "should exit r1");
}

// ── Damage Zone tests ────────────────────────────────────────────────

use crate::regions::effects::{DamageZoneEffect, RegionEffectsConfig as EffectsCfg};
use std::time::Duration;

fn ship_hull_hp(app: &mut App) -> f32 {
    let hull = app
        .world_mut()
        .query_filtered::<&crate::entities::spawner::EntitySystemHull, With<LocalShip>>()
        .single(app.world())
        .unwrap()
        .0
        .total_current();
    hull
}

fn damage_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .add_plugins(RegionPlugin);
    // AiEntityDestroyed / WorldResource are needed by apply_damage_zone_damage
    // for the NPC-destruction path (PRD #597 PR 9). They're written only
    // when a non-LocalShip ship dies inside a damage zone, but the
    // MessageWriter and Resource must be registered up-front.
    app.add_message::<crate::ai::server::AiEntityDestroyed>();
    app.init_resource::<crate::lobby::WorldResource>();
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let hull_config = &[
        (crate::core::messages::SystemId("helm".into()), 25.0),
        (crate::core::messages::SystemId("tactical".into()), 25.0),
        (crate::core::messages::SystemId("power".into()), 25.0),
        (crate::core::messages::SystemId("shields".into()), 25.0),
    ];
    app.world_mut().spawn((
        LocalShip,
        crate::server_app::Ship,
        Transform::default(),
        crate::ship::state::ShipPhysics::default(),
        crate::server_app::ShipShields(ShieldSystem::new(&ShieldConfig::default()), 0.5),
        crate::entities::spawner::EntitySystemHull(SystemHull::from_config(hull_config)),
        ShipModifiers::new(),
    ));
    app
}

fn blocks_impulse_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .add_plugins(RegionPlugin);
    app.world_mut().spawn((
        LocalShip,
        crate::server_app::Ship,
        Transform::default(),
        crate::ship::state::ShipPhysics::default(),
        ShipImpulse::default(),
        ShipModifiers::new(),
    ));
    app
}

/// Advance the fixed clock by exactly `dt_secs` and run one `app.update()`.
///
/// Routes through `test_support::drive_one_fixed_step_per_update`
/// (issue #895 re-review), which lets every fixture in this module build
/// with the REAL `RegionPlugin` instead of a hand-rolled `Time<()>` copy
/// of its registration: `apply_damage_zone_damage` and
/// `update_region_membership` read the generic `Res<Time>`, which
/// resolves to `Time<Fixed>` inside `FixedUpdate` and reports exactly the
/// `dt_secs` this function just pinned the timestep to. A caller passing
/// 0.1, then 1.0, then 0.016 across successive calls sees exactly those
/// deltas — the arbitrary-precision behaviour these tests were written
/// against — because the corrected helper discards stale overstep and
/// skips its fresh-app preload once `app.update()` has run at least once,
/// so re-pacing mid-test can never double a step.
fn tick_with_dt(app: &mut App, dt_secs: f32) {
    crate::ship::test_support::drive_one_fixed_step_per_update(
        app,
        Duration::from_secs_f32(dt_secs),
    );
    app.update();
}

fn spawn_blocks_impulse_region(app: &mut App, x: f32, z: f32, radius: f32) -> Entity {
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
        tags: vec!["region".to_string()],
        shape: Some(RegionShape::Sphere { radius }),
        effects: Some(EffectsCfg {
            blocks_impulse: Some(BlocksImpulseEffect {}),
            ..Default::default()
        }),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
        target: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let mut commands = app.world_mut().commands();
    let entity = spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None);
    // Flush now (issue #895): these fixtures drive the REAL `RegionPlugin`,
    // which runs in `FixedUpdate` — earlier in the frame than the point a
    // command queued outside any system would otherwise be applied. Without
    // this the region entity does not exist yet on the fixed step the
    // caller's next `tick_with_dt` runs, exactly like `spawn_region` above.
    app.world_mut().flush();
    entity
}

fn spawn_damage_zone(app: &mut App, x: f32, z: f32, radius: f32, dps: f32) -> Entity {
    spawn_damage_zone_with_pierce(app, x, z, radius, dps, 1.0)
}

fn spawn_damage_zone_with_pierce(
    app: &mut App,
    x: f32,
    z: f32,
    radius: f32,
    dps: f32,
    shield_pierce: f32,
) -> Entity {
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
        tags: vec!["region".to_string()],
        shape: Some(RegionShape::Sphere { radius }),
        effects: Some(EffectsCfg {
            damage_zone: Some(DamageZoneEffect {
                damage_per_second: dps,
                shield_pierce,
            }),
            ..Default::default()
        }),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
        target: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let mut commands = app.world_mut().commands();
    let entity = spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None);
    // Flush now (issue #895): these fixtures drive the REAL `RegionPlugin`,
    // which runs in `FixedUpdate` — earlier in the frame than the point a
    // command queued outside any system would otherwise be applied. Without
    // this the region entity does not exist yet on the fixed step the
    // caller's next `tick_with_dt` runs, exactly like `spawn_region` above.
    app.world_mut().flush();
    entity
}

#[test]
fn ship_in_damage_zone_takes_damage() {
    let mut app = damage_test_app();
    spawn_damage_zone(&mut app, 0.0, 0.0, 50.0, 50.0);
    tick_with_dt(&mut app, 0.1);

    let hull_hp = ship_hull_hp(&mut app);
    assert!(
        (hull_hp - 95.0).abs() < 1e-6,
        "hull should be ~95 after 0.1s at 50 dps, got {}",
        hull_hp
    );
}

#[test]
fn ship_outside_damage_zone_takes_no_damage() {
    let mut app = damage_test_app();
    spawn_damage_zone(&mut app, 200.0, 0.0, 50.0, 50.0);
    // Ship stays at origin, far outside the zone at (200, 0)
    set_ship_pos(&mut app, 0.0, 0.0);
    tick_with_dt(&mut app, 0.1);
    tick_with_dt(&mut app, 0.1);

    let hull_hp = ship_hull_hp(&mut app);
    assert!(
        (hull_hp - 100.0).abs() < 1e-6,
        "hull should remain at 100 when outside damage zone, got {}",
        hull_hp
    );
}

#[test]
fn damage_zone_bypasses_shields() {
    let mut app = damage_test_app();
    // Override shields with custom config via entity component
    use crate::server_app::ShipShields;
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(ship).insert(ShipShields(
        ShieldSystem::new(&ShieldConfig {
            max_hp: 100,
            ..Default::default()
        }),
        0.5,
    ));

    spawn_damage_zone(&mut app, 0.0, 0.0, 50.0, 50.0);
    tick_with_dt(&mut app, 0.1);

    // Hull should have taken damage (bypassing shields)
    let hull_hp = ship_hull_hp(&mut app);
    assert!(
        (hull_hp - 95.0).abs() < 1e-6,
        "hull should be ~95 (damage bypassed shields), got {}",
        hull_hp
    );

    // Shields should be untouched (full HP)
    let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
    for facing in &shields.0.facings {
        assert_eq!(facing.hp, 100, "shield facing should be undamaged");
    }
}

#[test]
fn damage_zone_partial_pierce_splits_70_30() {
    let mut app = damage_test_app();
    use crate::server_app::ShipShields;
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(ship).insert(ShipShields(
        ShieldSystem::new(&ShieldConfig {
            max_hp: 1000,
            ..Default::default()
        }),
        0.5,
    ));

    // 100 dps for 1s = 100 damage. shield_pierce = 0.3 →
    // pierced = 30 (to hull), absorbed = 70 (to fore shield).
    spawn_damage_zone_with_pierce(&mut app, 0.0, 0.0, 50.0, 100.0, 0.3);
    tick_with_dt(&mut app, 1.0);

    let hull_hp = ship_hull_hp(&mut app);
    assert!(
        (hull_hp - 70.0).abs() < 0.5,
        "hull should be ~70 after 30 pierced damage on 100hp, got {}",
        hull_hp
    );
    // 70 absorbed ÷ 4 facings = 17 rem 2. Fore and Port get 18, Aft and Starboard get 17.
    let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
    assert_eq!(shields.0.facings[0].hp, 982, "fore should get 18 of 70");
    assert_eq!(shields.0.facings[1].hp, 982, "port should get 18 of 70");
    assert_eq!(shields.0.facings[2].hp, 983, "aft should get 17 of 70");
    assert_eq!(
        shields.0.facings[3].hp, 983,
        "starboard should get 17 of 70"
    );
}

#[test]
fn damage_zone_zero_pierce_routes_all_to_shields() {
    let mut app = damage_test_app();
    use crate::server_app::ShipShields;
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut().entity_mut(ship).insert(ShipShields(
        ShieldSystem::new(&ShieldConfig {
            max_hp: 1000,
            ..Default::default()
        }),
        0.5,
    ));

    // shield_pierce = 0.0: all damage absorbed by fore shield, hull untouched.
    spawn_damage_zone_with_pierce(&mut app, 0.0, 0.0, 50.0, 50.0, 0.0);
    tick_with_dt(&mut app, 1.0);

    let hull_hp = ship_hull_hp(&mut app);
    assert!(
        (hull_hp - 100.0).abs() < 1e-6,
        "hull should be untouched at zero pierce, got {}",
        hull_hp
    );
    // 50 absorbed ÷ 4 facings = 12 rem 2. Fore and Port get 13, Aft and Starboard get 12.
    let shields = app.world().entity(ship).get::<ShipShields>().unwrap();
    assert_eq!(shields.0.facings[0].hp, 987, "fore should get 13 of 50");
    assert_eq!(shields.0.facings[1].hp, 987, "port should get 13 of 50");
    assert_eq!(shields.0.facings[2].hp, 988, "aft should get 12 of 50");
    assert_eq!(
        shields.0.facings[3].hp, 988,
        "starboard should get 12 of 50"
    );
}

#[test]
fn fractional_dps_accumulates_over_multiple_ticks() {
    let mut app = damage_test_app();
    // Low DPS so each tick does fractional damage
    spawn_damage_zone(&mut app, 0.0, 0.0, 50.0, 3.0);
    // Three ticks at 0.1s each = 0.3s total, damage = 3 * 0.3 = 0.9
    tick_with_dt(&mut app, 0.1);
    tick_with_dt(&mut app, 0.1);
    tick_with_dt(&mut app, 0.1);

    let hull_hp = ship_hull_hp(&mut app);
    assert!(
        (hull_hp - 99.1).abs() < 0.001,
        "hull should be ~99.1 after 0.3s at 3 dps, got {}",
        hull_hp
    );
}

/// PRD #597 PR 9: region effects (including damage zones) must apply to
/// every ship (player + NPCs), not just the LocalShip. This test spawns
/// an NPC ship (with the `Ship` marker but no `LocalShip`) inside a
/// damage zone while the player ship sits outside; only the NPC's hull
/// should decrease.
#[test]
fn npc_ship_in_damage_zone_takes_hull_damage() {
    use crate::entities::spawner::{EntitySystemHull, EntityUuid};
    use crate::ship::damage::SystemHull;

    let mut app = damage_test_app();

    // Move the player (LocalShip) far outside the damage zone.
    set_ship_pos(&mut app, 500.0, 0.0);
    let player_hull_before = ship_hull_hp(&mut app);

    // Spawn an NPC ship at the origin with the Ship marker but no
    // LocalShip. Its EntitySystemHull starts at 100 HP.
    let npc_hull_config = &[(crate::core::messages::SystemId("captain".into()), 100.0)];
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid("npc-damage-zone".to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            crate::ship::state::ShipPhysics {
                x: 0.0,
                z: 0.0,
                ..Default::default()
            },
            EntitySystemHull(SystemHull::from_config(npc_hull_config)),
            ShipModifiers::new(),
        ))
        .id();

    // Damage zone at origin with 50 dps. NPC is inside; player is outside.
    spawn_damage_zone(&mut app, 0.0, 0.0, 50.0, 50.0);
    tick_with_dt(&mut app, 0.1);

    // NPC hull must decrease.
    let npc_hull_after = app
        .world()
        .get::<EntitySystemHull>(npc)
        .expect("NPC must retain EntitySystemHull")
        .0
        .total_current();
    assert!(
        npc_hull_after < 100.0,
        "NPC hull must decrease from damage zone, got {} (max 100)",
        npc_hull_after
    );
    // At 50 dps for 0.1s = 5 damage → 95 HP.
    assert!(
        (npc_hull_after - 95.0).abs() < 1e-6,
        "NPC hull should be ~95 after 0.1s at 50 dps, got {}",
        npc_hull_after
    );

    // Player hull must be unaffected (player is outside the zone).
    let player_hull_after = ship_hull_hp(&mut app);
    assert!(
        (player_hull_after - player_hull_before).abs() < 1e-6,
        "player hull must be unchanged (player is outside zone), before={} after={}",
        player_hull_before,
        player_hull_after,
    );
}

// -- BlocksImpulse tests ------------------------------------------------

// ── BlocksImpulse tests ─────────────────────────────────────────────

fn set_impulse_charging(app: &mut App) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipImpulse, With<LocalShip>>();
    if let Ok(mut imp) = q.single_mut(app.world_mut()) {
        imp.0.start_charge();
    }
}

fn set_impulse_active(app: &mut App) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipImpulse, With<LocalShip>>();
    if let Ok(mut imp) = q.single_mut(app.world_mut()) {
        imp.0.start_charge();
        imp.0.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
    }
}

fn assert_impulse_phase(app: &mut App, expected: ImpulsePhase) {
    let phase = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipImpulse, With<LocalShip>>();
        q.single(app.world())
            .map(|i| i.0.phase)
            .expect("LocalShip must have ShipImpulse component")
    };
    assert_eq!(
        phase, expected,
        "expected impulse {:?}, got {:?}",
        expected, phase
    );
}

#[test]
fn entering_blocks_impulse_region_cancels_charging_impulse() {
    let mut app = blocks_impulse_test_app();
    let _region = spawn_blocks_impulse_region(&mut app, 100.0, 0.0, 50.0);
    set_ship_pos(&mut app, 0.0, 0.0); // outside region at (100,0) radius 50
    tick_with_dt(&mut app, 0.016); // initialise membership

    // Move ship inside the region
    set_ship_pos(&mut app, 80.0, 0.0);
    set_impulse_charging(&mut app);
    assert_impulse_phase(&mut app, ImpulsePhase::Charging);

    // Tick — should trigger RegionEntered and cancel impulse
    tick_with_dt(&mut app, 0.016);

    assert_impulse_phase(&mut app, ImpulsePhase::Idle);
}

#[test]
fn entering_blocks_impulse_region_cancels_active_impulse() {
    let mut app = blocks_impulse_test_app();
    let _region = spawn_blocks_impulse_region(&mut app, 100.0, 0.0, 50.0);
    set_ship_pos(&mut app, 0.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    // Move ship inside
    set_ship_pos(&mut app, 80.0, 0.0);
    set_impulse_active(&mut app);
    assert_impulse_phase(&mut app, ImpulsePhase::Active);

    tick_with_dt(&mut app, 0.016);

    assert_impulse_phase(&mut app, ImpulsePhase::Idle);
}

#[test]
fn staying_outside_blocks_impulse_region_leaves_impulse_unchanged() {
    let mut app = blocks_impulse_test_app();
    let _region = spawn_blocks_impulse_region(&mut app, 200.0, 0.0, 50.0);
    set_ship_pos(&mut app, 0.0, 0.0); // far outside
    tick_with_dt(&mut app, 0.016);

    set_impulse_charging(&mut app);
    tick_with_dt(&mut app, 0.016);

    assert_impulse_phase(&mut app, ImpulsePhase::Charging);
}

#[test]
fn npc_entering_blocks_impulse_region_does_not_cancel_players_impulse() {
    // Regression for the audit-report bug where an NPC entering a
    // BlocksImpulse region silently cancelled the player's impulse
    // because the observer wrote to the global ShipImpulse Resource
    // without a LocalShip gate.
    let mut app = blocks_impulse_test_app();
    let _region = spawn_blocks_impulse_region(&mut app, 100.0, 0.0, 50.0);
    // Player at (0,0,0) — far outside the region at (100,0,0)r=50.
    set_ship_pos(&mut app, 0.0, 0.0);
    // Spawn an NPC ship at (80,0,0) — inside the region.
    let _npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            Transform::default(),
            crate::ship::state::ShipPhysics {
                x: 80.0,
                z: 0.0,
                ..Default::default()
            },
        ))
        .id();
    // Charge the player's impulse.
    set_impulse_charging(&mut app);
    assert_impulse_phase(&mut app, ImpulsePhase::Charging);
    // Tick — NPC crosses into the region, RegionEntered fires with
    // subject = NPC. The observer must NOT touch the player's impulse.
    tick_with_dt(&mut app, 0.016);
    assert_impulse_phase(&mut app, ImpulsePhase::Charging);
}

// ── Radar Dampening tests ───────────────────────────────────────────

fn radar_dampening_test_app() -> App {
    let mut app = App::new();
    // Region observers first, then the modifier plugin's — matching the
    // production registration order (`RegionPlugin` before
    // `ModifierCoordinationPlugin`), which decides which observer sees a
    // `RegionEntered` first.
    app.add_plugins(bevy::time::TimePlugin)
        .add_plugins(RegionPlugin)
        .add_plugins(crate::modifiers::coordination::ModifierCoordinationPlugin);
    app.world_mut().spawn((
        LocalShip,
        crate::server_app::Ship,
        Transform::default(),
        crate::ship::state::ShipPhysics::default(),
        ShipModifiers::new(),
    ));
    app
}

fn get_ship_modifiers(app: &mut App) -> ShipModifiers {
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipModifiers, With<LocalShip>>();
    q.single(app.world()).unwrap().clone()
}

fn spawn_radar_dampening_region(
    app: &mut App,
    x: f32,
    z: f32,
    radius: f32,
    multiplier: f32,
) -> Entity {
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
        tags: vec!["region".to_string()],
        shape: Some(RegionShape::Sphere { radius }),
        effects: Some(EffectsCfg {
            radar_dampening: Some(RadarDampeningEffect {
                range_modifier: multiplier,
            }),
            ..Default::default()
        }),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
        target: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let mut commands = app.world_mut().commands();
    let entity = spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None);
    // Flush now (issue #895): these fixtures drive the REAL `RegionPlugin`,
    // which runs in `FixedUpdate` — earlier in the frame than the point a
    // command queued outside any system would otherwise be applied. Without
    // this the region entity does not exist yet on the fixed step the
    // caller's next `tick_with_dt` runs, exactly like `spawn_region` above.
    app.world_mut().flush();
    entity
}

#[test]
fn entering_radar_dampening_region_adds_modifier() {
    let mut app = radar_dampening_test_app();
    spawn_radar_dampening_region(&mut app, 0.0, 0.0, 50.0, -0.3);
    set_ship_pos(&mut app, 0.0, 0.0); // inside region at origin
    tick_with_dt(&mut app, 0.016);

    let modifiers = get_ship_modifiers(&mut app);
    let expected = 1.0 / (1.0 + 0.3); // PRD #117 negative-bonus formula
    assert!(
        (modifiers.get(&ModifierSlot::RadarRange) - expected).abs() < 1e-6,
        "expected radar range multiplier ~{}, got {}",
        expected,
        modifiers.get(&ModifierSlot::RadarRange)
    );
}

#[test]
fn exiting_radar_dampening_region_removes_modifier() {
    let mut app = radar_dampening_test_app();
    let _region = spawn_radar_dampening_region(&mut app, 0.0, 0.0, 50.0, -0.3);
    set_ship_pos(&mut app, 0.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);

    // Verify modifier is present
    let modifiers_before = get_ship_modifiers(&mut app);
    assert!(
        (modifiers_before.get(&ModifierSlot::RadarRange) - 1.0 / 1.3).abs() < 1e-6,
        "modifier should be present while inside region"
    );

    // Move ship outside
    set_ship_pos(&mut app, 200.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    let modifiers_after = get_ship_modifiers(&mut app);
    assert!(
        (modifiers_after.get(&ModifierSlot::RadarRange) - 1.0).abs() < 1e-6,
        "modifier should be removed after exiting region, got {}",
        modifiers_after.get(&ModifierSlot::RadarRange)
    );
}

#[test]
fn overlapping_radar_dampening_regions_stack_additively() {
    let mut app = radar_dampening_test_app();
    // Region A at (0,0) radius 80, bonus -0.3
    // Region B at (60,0) radius 80, bonus -0.5
    // Ship at (0,0) is inside both
    spawn_radar_dampening_region(&mut app, 0.0, 0.0, 80.0, -0.3);
    spawn_radar_dampening_region(&mut app, 60.0, 0.0, 80.0, -0.5);
    set_ship_pos(&mut app, 0.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    // Both bonuses sum to -0.8 → 1/(1+0.8) = 0.5556
    let modifiers = get_ship_modifiers(&mut app);
    let expected_both = 1.0 / (1.0 + 0.3 + 0.5);
    assert!(
        (modifiers.get(&ModifierSlot::RadarRange) - expected_both).abs() < 1e-6,
        "expected stacked multiplier ~{}, got {}",
        expected_both,
        modifiers.get(&ModifierSlot::RadarRange)
    );

    // Move to (-40,0): still inside A (dist 40 < 80), outside B (dist 100 > 80)

    set_ship_pos(&mut app, -40.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    let modifiers = get_ship_modifiers(&mut app);
    let expected_a = 1.0 / (1.0 + 0.3);
    assert!(
        (modifiers.get(&ModifierSlot::RadarRange) - expected_a).abs() < 1e-6,
        "expected only region A multiplier ~{}, got {}",
        expected_a,
        modifiers.get(&ModifierSlot::RadarRange)
    );
}

// ── Slow Zone tests ─────────────────────────────────────────────────

fn slow_zone_test_app() -> App {
    let mut app = App::new();
    // Region observers first, then the modifier plugin's — matching the
    // production registration order (`RegionPlugin` before
    // `ModifierCoordinationPlugin`), which decides which observer sees a
    // `RegionEntered` first.
    app.add_plugins(bevy::time::TimePlugin)
        .add_plugins(RegionPlugin)
        .add_plugins(crate::modifiers::coordination::ModifierCoordinationPlugin);
    app.world_mut().spawn((
        LocalShip,
        crate::server_app::Ship,
        Transform::default(),
        crate::ship::state::ShipPhysics::default(),
        ShipModifiers::new(),
    ));
    app
}

fn spawn_slow_zone(
    app: &mut App,
    x: f32,
    z: f32,
    radius: f32,
    thrust_modifier: Option<f32>,
    yaw_rate_modifier: Option<f32>,
) -> Entity {
    use crate::regions::effects::RegionEffectsConfig as EffectsCfg;
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
        tags: vec!["region".to_string()],
        shape: Some(RegionShape::Sphere { radius }),
        effects: Some(EffectsCfg {
            slow_zone: Some(SlowZoneEffect {
                thrust_modifier,
                yaw_rate_modifier,
            }),
            ..Default::default()
        }),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
        target: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let mut commands = app.world_mut().commands();
    let entity = spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None);
    // Flush now (issue #895): these fixtures drive the REAL `RegionPlugin`,
    // which runs in `FixedUpdate` — earlier in the frame than the point a
    // command queued outside any system would otherwise be applied. Without
    // this the region entity does not exist yet on the fixed step the
    // caller's next `tick_with_dt` runs, exactly like `spawn_region` above.
    app.world_mut().flush();
    entity
}

fn check_modifier(app: &mut App, slot: ModifierSlot, expected: f32) {
    let modifiers = get_ship_modifiers(app);
    assert!(
        (modifiers.get(&slot) - expected).abs() < 1e-6,
        "expected modifier multiplier {} for {:?}, got {}",
        expected,
        slot,
        modifiers.get(&slot)
    );
}

/// RED 1: entering slow zone with thrust_modifier registers MaxSpeed modifier
#[test]
fn entering_slow_zone_with_thrust_modifier_registers_maxspeed_modifier() {
    let mut app = slow_zone_test_app();
    spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);

    // -0.5 bonus → 1/(1+0.5) = 0.6667
    check_modifier(&mut app, ModifierSlot::MaxSpeed, 1.0 / 1.5);
}

/// RED 2: entering slow zone with yaw_rate_modifier registers MaxYawRate modifier
#[test]
fn entering_slow_zone_with_yaw_rate_modifier_registers_maxyawrate_modifier() {
    let mut app = slow_zone_test_app();
    spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, None, Some(-0.3));
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);

    // -0.3 bonus → 1/(1+0.3) = 0.7692
    check_modifier(&mut app, ModifierSlot::MaxYawRate, 1.0 / 1.3);
}

/// RED 3: entering slow zone with both fields registers both slots
#[test]
fn entering_slow_zone_with_both_fields_registers_both_slots() {
    let mut app = slow_zone_test_app();
    spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), Some(-0.3));
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);

    check_modifier(&mut app, ModifierSlot::MaxSpeed, 1.0 / 1.5);
    check_modifier(&mut app, ModifierSlot::MaxYawRate, 1.0 / 1.3);
}

/// RED 4: entering slow zone with both fields omitted registers nothing
#[test]
fn entering_slow_zone_with_both_fields_omitted_registers_nothing() {
    let mut app = slow_zone_test_app();
    spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, None, None);
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);

    check_modifier(&mut app, ModifierSlot::MaxSpeed, 1.0);
    check_modifier(&mut app, ModifierSlot::MaxYawRate, 1.0);
}

fn get_ship_physics(app: &mut App) -> crate::ship::state::ShipPhysics {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::ship::state::ShipPhysics, With<LocalShip>>();
    *q.single(app.world())
        .expect("expected LocalShip with ShipPhysics")
}

fn set_physics(app: &mut App, f: impl FnOnce(&mut crate::ship::state::ShipPhysics)) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::ship::state::ShipPhysics, With<LocalShip>>();
    let mut p = q
        .single_mut(app.world_mut())
        .expect("expected LocalShip with ShipPhysics");
    f(&mut p);
}

/// RED 5: entry clamps forward_speed to new effective max
#[test]
fn entering_slow_zone_clamps_forward_speed() {
    let mut app = slow_zone_test_app();
    spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);

    // Set ship speed above the clamped limit
    set_physics(&mut app, |p| {
        p.forward_speed = 50.0;
        p.x = 10.0;
    });

    tick_with_dt(&mut app, 0.016);

    // After clamping: base max speed = 25.0, modifier = 0.6667, effective max = 16.667
    let expected_clamped = ShipPhysicsConfig::new().max_speed * (1.0 / 1.5);
    let physics = get_ship_physics(&mut app);
    assert!(
        (physics.forward_speed - expected_clamped).abs() < 0.001,
        "expected forward_speed clamped to ~{}, got {}",
        expected_clamped,
        physics.forward_speed
    );
}

/// RED 6: entering slow zone does not clamp speed when already below limit
#[test]
fn entering_slow_zone_does_not_clamp_when_already_below_limit() {
    let mut app = slow_zone_test_app();
    spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);

    set_physics(&mut app, |p| {
        p.forward_speed = 5.0;
        p.x = 10.0;
    });

    tick_with_dt(&mut app, 0.016);

    // 5.0 is already below effective max (16.667), should remain 5.0
    let physics = get_ship_physics(&mut app);
    assert!(
        (physics.forward_speed - 5.0).abs() < 0.001,
        "forward_speed should remain 5.0, got {}",
        physics.forward_speed
    );
}

/// RED 7: exit removes MaxSpeed modifier, does NOT restore velocity
#[test]
fn exiting_slow_zone_removes_maxspeed_modifier() {
    let mut app = slow_zone_test_app();
    let _region = spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), Some(-0.3));
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);
    check_modifier(&mut app, ModifierSlot::MaxSpeed, 1.0 / 1.5);

    // Exit the region
    set_ship_pos(&mut app, 200.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    check_modifier(&mut app, ModifierSlot::MaxSpeed, 1.0);
    check_modifier(&mut app, ModifierSlot::MaxYawRate, 1.0);
}

/// RED 8: exit does NOT restore previously-clamped velocity
#[test]
fn exiting_slow_zone_does_not_restore_velocity() {
    let mut app = slow_zone_test_app();
    let _region = spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);

    // Start with 50 speed, enter → clamped to ~16.667
    set_physics(&mut app, |p| {
        p.forward_speed = 50.0;
        p.x = 10.0;
    });

    tick_with_dt(&mut app, 0.016);

    // Confirm speed was clamped
    let physics = get_ship_physics(&mut app);
    assert!(
        (physics.forward_speed - 16.667).abs() < 0.001,
        "speed should be clamped to ~16.667, got {}",
        physics.forward_speed
    );

    // Exit the region
    set_ship_pos(&mut app, 200.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    // Speed should REMAIN clamped (not restored to 50)
    let physics = get_ship_physics(&mut app);
    assert!(
        (physics.forward_speed - 16.667).abs() < 0.001,
        "speed should remain clamped after exit (not restored), got {}",
        physics.forward_speed
    );
}

#[test]
fn slow_zone_still_clamps_player_when_npcs_exist() {
    // Regression test for PRD #597 PR-1: handle_slow_zone_speed_clamp used
    // ship_query.single_mut() on With<Ship>. With NPCs having Ship marker,
    // single_mut() returns Err and the clamp silently no-ops for the player.
    // After fix: uses trigger.event().subject so the entering entity is always clamped.
    use crate::ship::state::ShipPhysics;

    let mut app = slow_zone_test_app();

    // Give the LocalShip high speed and place it inside the upcoming slow zone.
    set_physics(&mut app, |p| {
        p.forward_speed = 50.0;
        p.x = 10.0;
    });

    // Spawn a second NPC ship (now has Ship marker). Before the fix this made
    // single_mut() return Err and the player would not be clamped.
    app.world_mut().spawn((
        crate::server_app::Ship,
        Transform::from_xyz(200.0, 0.0, 0.0), // outside the zone
        ShipPhysics {
            forward_speed: 50.0,
            ..Default::default()
        },
    ));

    // Spawn slow zone around origin — the LocalShip is inside.
    let _region = spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);
    tick_with_dt(&mut app, 0.016);

    let player_speed = get_ship_physics(&mut app).forward_speed;
    assert!(
        player_speed < 50.0,
        "Player entering slow zone must still be clamped even when NPC ships exist (got {player_speed})"
    );
}

/// PRD #597 PR 9: region membership is tracked for every ship (player +
/// NPCs), and the slow-zone speed clamp applies to whichever ship
/// crossed the boundary. Player is far outside the zone; an NPC enters
/// the zone at high speed and must be clamped by its own
/// `ShipModifiers` component — while the player's speed is untouched.
#[test]
fn slow_zone_slows_npc_ship() {
    use crate::ship::state::ShipPhysics;

    let mut app = slow_zone_test_app();

    // Player is far outside the zone; give it a high speed too so we can
    // prove the clamp acts on the NPC, not the player.
    set_physics(&mut app, |p| {
        p.forward_speed = 50.0;
        p.x = 500.0;
    });

    // Spawn the NPC inside the upcoming slow zone. It needs its own
    // ShipPhysics (region membership queries `With<Ship>` + &ShipPhysics)
    // and its own ShipModifiers (the slow-zone modifier is applied
    // per-entity via the coordination observer).
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            Transform::from_xyz(10.0, 0.0, 0.0),
            ShipPhysics {
                x: 10.0,
                z: 0.0,
                forward_speed: 50.0,
                ..Default::default()
            },
            ShipModifiers::new(),
        ))
        .id();

    // Slow zone around origin (thrust_modifier -0.5 → 1/(1+0.5) = 0.667 mult).
    let _region = spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);
    tick_with_dt(&mut app, 0.016);

    // NPC forward speed must be clamped to the effective max
    // (base_max * 0.667 = 25.0 * 0.667 = ~16.667).
    let npc_speed = app
        .world()
        .get::<ShipPhysics>(npc)
        .expect("NPC must retain ShipPhysics")
        .forward_speed;
    let expected_clamped = crate::ship::physics::ShipPhysicsConfig::new().max_speed * (1.0 / 1.5);
    assert!(
        (npc_speed - expected_clamped).abs() < 0.5,
        "NPC entering slow zone must be clamped to ~{}, got {}",
        expected_clamped,
        npc_speed,
    );

    // Player is outside the zone and must be unaffected.
    let player_speed = get_ship_physics(&mut app).forward_speed;
    assert!(
        (player_speed - 50.0).abs() < 1e-6,
        "player outside slow zone must retain its speed 50.0, got {}",
        player_speed,
    );
}

// ── Flag effect tests (CommsJam / SensorBlind) ─────────────────────────

use crate::core::messages::FlagKind;
use crate::regions::effects::{CommsJamEffect, SensorBlindEffect};

fn flag_test_app() -> App {
    let mut app = App::new();
    // Region observers first, then the modifier plugin's — matching the
    // production registration order (`RegionPlugin` before
    // `ModifierCoordinationPlugin`), which decides which observer sees a
    // `RegionEntered` first.
    app.add_plugins(bevy::time::TimePlugin)
        .add_plugins(RegionPlugin)
        .add_plugins(crate::modifiers::coordination::ModifierCoordinationPlugin);
    app.world_mut().spawn((
        LocalShip,
        crate::server_app::Ship,
        Transform::default(),
        crate::ship::state::ShipPhysics::default(),
        ShipModifiers::new(),
    ));
    app
}

/// Spawn a region with the CommsJam effect at the given position.
fn spawn_comms_jam_region(app: &mut App, x: f32, z: f32, radius: f32) -> Entity {
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
        tags: vec!["region".to_string()],
        shape: Some(RegionShape::Sphere { radius }),
        effects: Some(EffectsCfg {
            comms_jammed: Some(CommsJamEffect {}),
            ..Default::default()
        }),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
        target: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let mut commands = app.world_mut().commands();
    let entity = spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None);
    // Flush now (issue #895): these fixtures drive the REAL `RegionPlugin`,
    // which runs in `FixedUpdate` — earlier in the frame than the point a
    // command queued outside any system would otherwise be applied. Without
    // this the region entity does not exist yet on the fixed step the
    // caller's next `tick_with_dt` runs, exactly like `spawn_region` above.
    app.world_mut().flush();
    entity
}

/// Spawn a region with the SensorBlind effect at the given position.
fn spawn_sensor_blind_region(app: &mut App, x: f32, z: f32, radius: f32) -> Entity {
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
        tags: vec!["region".to_string()],
        shape: Some(RegionShape::Sphere { radius }),
        effects: Some(EffectsCfg {
            sensor_blind: Some(SensorBlindEffect {}),
            ..Default::default()
        }),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        mesh: None,
        target: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let mut commands = app.world_mut().commands();
    let entity = spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None);
    // Flush now (issue #895): these fixtures drive the REAL `RegionPlugin`,
    // which runs in `FixedUpdate` — earlier in the frame than the point a
    // command queued outside any system would otherwise be applied. Without
    // this the region entity does not exist yet on the fixed step the
    // caller's next `tick_with_dt` runs, exactly like `spawn_region` above.
    app.world_mut().flush();
    entity
}

fn assert_flag(app: &mut App, flag: FlagKind, expected: bool) {
    let modifiers = get_ship_modifiers(app);
    assert_eq!(
        modifiers.has_flag(&flag),
        expected,
        "expected flag {:?} to be {}, but got {}",
        flag,
        expected,
        !expected
    );
}

/// RED 1: entering a comms_jam region sets the CommsJammed flag
#[test]
fn entering_comms_jam_region_sets_flag() {
    let mut app = flag_test_app();
    spawn_comms_jam_region(&mut app, 0.0, 0.0, 50.0);
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);
    assert_flag(&mut app, FlagKind::CommsJammed, true);
}

/// RED 2: entering a sensor_blind region sets the SensorBlind flag
#[test]
fn entering_sensor_blind_region_sets_flag() {
    let mut app = flag_test_app();
    spawn_sensor_blind_region(&mut app, 0.0, 0.0, 50.0);
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);
    assert_flag(&mut app, FlagKind::SensorBlind, true);
}

/// RED 3: exiting a comms_jam region clears the flag
#[test]
fn exiting_comms_jam_region_clears_flag() {
    let mut app = flag_test_app();
    let _region = spawn_comms_jam_region(&mut app, 0.0, 0.0, 50.0);
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);
    assert_flag(&mut app, FlagKind::CommsJammed, true);

    // Exit the region
    set_ship_pos(&mut app, 200.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    assert_flag(&mut app, FlagKind::CommsJammed, false);
}

/// RED 4: two overlapping comms_jam regions OR-aggregate; flag clears only
/// when the last source exits.
#[test]
fn overlapping_comms_jam_regions_or_aggregate() {
    let mut app = flag_test_app();
    // Region A at (0,0) radius 80
    // Region B at (60,0) radius 80
    // Ship at (0,0) is inside both
    spawn_comms_jam_region(&mut app, 0.0, 0.0, 80.0);
    spawn_comms_jam_region(&mut app, 60.0, 0.0, 80.0);
    set_ship_pos(&mut app, 0.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    assert_flag(&mut app, FlagKind::CommsJammed, true);

    // Exit B: move to (-40,0) — still inside A (dist 40 < 80), outside B (dist 100 > 80)
    set_ship_pos(&mut app, -40.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    assert_flag(&mut app, FlagKind::CommsJammed, true);

    // Exit A: move far away — outside both
    set_ship_pos(&mut app, -200.0, 0.0);
    tick_with_dt(&mut app, 0.016);

    assert_flag(&mut app, FlagKind::CommsJammed, false);
}

/// RED 5: region despawn while inside clears the flag
#[test]
fn region_despawn_while_inside_clears_flag() {
    let mut app = flag_test_app();
    let region = spawn_comms_jam_region(&mut app, 0.0, 0.0, 50.0);
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);
    assert_flag(&mut app, FlagKind::CommsJammed, true);

    // Despawn the region entity
    app.world_mut().despawn(region);
    tick_with_dt(&mut app, 0.016);

    assert_flag(&mut app, FlagKind::CommsJammed, false);
}

/// RED 9: region despawn while inside removes modifiers
#[test]
fn region_despawn_while_inside_removes_slow_zone_modifiers() {
    let mut app = slow_zone_test_app();
    let region = spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), Some(-0.3));
    set_ship_pos(&mut app, 10.0, 0.0); // inside
    tick_with_dt(&mut app, 0.016);
    check_modifier(&mut app, ModifierSlot::MaxSpeed, 1.0 / 1.5);

    // Despawn the region (implicit exit)
    app.world_mut().despawn(region);
    tick_with_dt(&mut app, 0.016);

    check_modifier(&mut app, ModifierSlot::MaxSpeed, 1.0);
    check_modifier(&mut app, ModifierSlot::MaxYawRate, 1.0);
}

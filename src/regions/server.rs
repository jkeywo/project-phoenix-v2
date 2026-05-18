use bevy::prelude::*;
use rand::SeedableRng as _;
use std::collections::{HashMap, HashSet};

use crate::entity_spawner::{RegionShapeSection, RegionEffectsSection, EntityUuid};
use crate::simulation::{Ship, ShipHullIntegrity, ShipImpulse};
use crate::ship_state::ShipState;
use crate::region_effects::RegionEffectKind;
use crate::modifiers::ShipModifiers;
use crate::messages::{GamePhase, ModifierSlot, ServerMessage};
use crate::lobby::Target;
use crate::server_app::SimOutbox;
use crate::simulation::GameOverReason;

/// Resource tracking which entities are inside which regions.
#[derive(Resource, Default)]
pub struct RegionMembership {
    /// Maps ship entity → set of region entities the ship is currently inside.
    pub inside: HashMap<Entity, HashSet<Entity>>,
    /// Cached UUIDs for region entities (persists after entity despawn).
    pub region_uuids: HashMap<Entity, String>,
}

/// Fired when a subject entity enters a region.
#[derive(Event, Clone, Debug)]
pub struct RegionEntered {
    pub subject: Entity,
    pub region_entity: Entity,
}

/// Fired when a subject entity exits a region (or the region is despawned).
#[derive(Event, Clone, Debug)]
pub struct RegionExited {
    pub subject: Entity,
    pub region_entity: Entity,
}

pub struct RegionPlugin;

impl Plugin for RegionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RegionMembership>()
            .add_systems(Update, (
                update_region_membership.in_set(crate::sim_sets::SimSet::Physics),
                apply_damage_zone_damage.in_set(crate::sim_sets::SimSet::Physics).after(update_region_membership),
            ))
            .add_observer(handle_blocks_impulse_region_enter)
            .add_observer(handle_slow_zone_speed_clamp);
    }
}

/// Per-tick system: checks ship position against every region shape and
/// emits `RegionEntered` / `RegionExited` on boundary crossings.
///
/// Automatically handles region despawn: if a region was previously occupied
/// and is no longer in the ECS, an implicit `RegionExited` is emitted.
pub(crate) fn update_region_membership(
    mut commands: Commands,
    mut membership: ResMut<RegionMembership>,
    region_query: Query<(Entity, &Transform, &RegionShapeSection)>,
    uuid_query: Query<&EntityUuid>,
    ship_state: Res<ShipState>,
    ship_query: Query<Entity, With<Ship>>,
) {
    let Ok(ship_entity) = ship_query.single() else {
        return;
    };

    let ship_pos = glam::Vec3::new(ship_state.x, 0.0, ship_state.z);

    // Cache UUIDs for all current region entities (survives despawn)
    for (entity, _, _) in region_query.iter() {
        if let Ok(uuid) = uuid_query.get(entity) {
            membership.region_uuids.insert(entity, uuid.0.clone());
        }
    }

    // Determine current region occupancy
    let current_inside: HashSet<Entity> = region_query
        .iter()
        .filter(|(_, transform, shape)| {
            let region_origin = transform.translation;
            shape.0.contains(ship_pos, region_origin)
        })
        .map(|(entity, _, _)| entity)
        .collect();

    // Get previous frame's inside set for this ship
    let prev_inside = membership.inside.get(&ship_entity).cloned().unwrap_or_default();

    // Detect exits: were in prev_inside but not in current_inside
    // (also catches despawned regions — despawned entities don't appear in region_query)
    for entity in prev_inside.difference(&current_inside) {
        commands.trigger(RegionExited {
            subject: ship_entity,
            region_entity: *entity,
        });
    }

    // Detect enters: in current_inside but not in prev_inside
    for entity in current_inside.difference(&prev_inside) {
        commands.trigger(RegionEntered {
            subject: ship_entity,
            region_entity: *entity,
        });
    }

    membership.inside.insert(ship_entity, current_inside);
}

/// Applies continuous damage from `DamageZone` regions to the ship each tick.
/// Damage bypasses shields — it goes directly to the hull via `apply_hull_damage`.
/// Damaged regions are tracked via `RegionMembership`.
fn apply_damage_zone_damage(
    time: Res<Time>,
    membership: Res<RegionMembership>,
    region_query: Query<&RegionEffectsSection>,
    ship_query: Query<Entity, With<Ship>>,
    mut hull: Option<ResMut<ShipHullIntegrity>>,
    mut outbox: Option<ResMut<SimOutbox>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
) {
    let Some(mut hull) = hull else {
        return;
    };

    let Ok(ship_entity) = ship_query.single() else {
        return;
    };

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let Some(region_set) = membership.inside.get(&ship_entity) else {
        return;
    };

    for &region_entity in region_set.iter() {
        let Ok(effects) = region_query.get(region_entity) else {
            continue;
        };
        for effect in &effects.0 {
            if let crate::region_effects::RegionEffectKind::DamageZone { dps } = effect {
                let total_damage = dps * dt;
                let mut rng = rand::rngs::SmallRng::from_os_rng();
                let (hull_applied, ship_destroyed) = crate::damage::apply_hull_damage(
                    &mut hull.0,
                    total_damage,
                    &mut rng,
                );
                if let Some(ref mut ob) = outbox {
                    ob.0.push((Target::All, ServerMessage::DamageTaken {
                        hull: hull_applied,
                        shield: 0.0,
                    }));
                }
                if ship_destroyed {
                    if let Some(ref mut ob) = outbox {
                        ob.0.push((Target::All, ServerMessage::ShipDestroyed));
                    }
                    if let Some(ref mut reason) = game_over_reason {
                        if reason.0.is_none() {
                            reason.0 = Some("All consoles destroyed".into());
                        }
                    }
                    if let Some(ref mut ns) = next_state {
                        ns.set(GamePhase::GameOver);
                    }
                }
            }
        }
    }
}

/// Cancels the ship's impulse drive (charging or active) when the ship enters
/// a region with the `BlocksImpulse` effect.
fn handle_blocks_impulse_region_enter(
    trigger: On<RegionEntered>,
    region_query: Query<&RegionEffectsSection>,
    impulse: Option<ResMut<ShipImpulse>>,
) {
    let Some(mut impulse) = impulse else {
        return;
    };
    let ev = trigger.event();
    let Ok(effects) = region_query.get(ev.region_entity) else {
        return;
    };
    if effects.0.iter().any(|e| *e == RegionEffectKind::BlocksImpulse) {
        impulse.0.cancel_charge();
    }
}







/// Clamps the ship's forward speed to the effective maximum when entering a
/// slow zone region. The modifier registration is handled by the coordinator's
/// `translate_region_modifiers` system — this system only clamps speed.
///
/// This is a non-modifier side effect that must run after the coordinator so
/// the effective max reflects the updated modifier state.
pub(crate) fn handle_slow_zone_speed_clamp(
    trigger: On<RegionEntered>,
    region_query: Query<&RegionEffectsSection>,
    modifiers: Res<ShipModifiers>,
    mut ship: ResMut<ShipState>,
) {
    let ev = trigger.event();
    let Ok(effects) = region_query.get(ev.region_entity) else {
        return;
    };
    let has_slow = effects.0.iter().any(|e| matches!(e, RegionEffectKind::SlowZone { .. }));
    if !has_slow {
        return;
    }
    let base_max = crate::ship_physics::ShipPhysicsConfig::new().max_speed;
    let effective_max = base_max * modifiers.get(&ModifierSlot::MaxSpeed);
    if ship.forward_speed.abs() > effective_max {
        ship.forward_speed = ship.forward_speed.signum() * effective_max;
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_spawner::spawn_entity;
    use crate::entity_config::EntityConfig;
    use crate::region_shape::RegionShape;
    use crate::damage::ConsoleHull;
    use crate::simulation::{ShipHullIntegrity, ShipImpulse};
    use crate::impulse::{ImpulseState, ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::region_effects::{BlocksImpulseEffect, RadarDampeningEffect, SlowZoneEffect};
    use crate::ship_physics::ShipPhysicsConfig;
    use crate::modifiers::ShipModifiers;
    use crate::messages::ModifierSlot;

    /// Build a minimal Bevy app with the region plugin.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(RegionPlugin)
            .insert_resource(ShipState::new())
            .insert_resource(ShipModifiers::new());
        // Spawn the ship entity
        app.world_mut().spawn((
            Ship,
            Transform::default(),
        ));
        app
    }

    /// Spawn a region entity at the given position with the given shape.
    /// Does NOT call app.update() internally — caller must flush afterwards.
    fn spawn_region(app: &mut App, x: f32, z: f32, shape: RegionShape) -> Entity {
        let config = EntityConfig {
            tags: vec!["region".to_string()],
            shape: Some(shape),
            effects: None,
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            science_console: None,
            sensors_console: None,
            shields_console: None,
            star: None,
            planet: None,
            asteroid_field: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    fn ship_entity(app: &mut App) -> Entity {
        let mut query = QueryState::<Entity, With<Ship>>::new(app.world_mut());
        query.iter(app.world()).next().unwrap()
    }

    fn is_inside(app: &mut App, region: Entity) -> bool {
        let ship = ship_entity(app);
        app.world().resource::<RegionMembership>().inside.get(&ship).map_or(false, |set| set.contains(&region))
    }

    fn set_ship_pos(app: &mut App, x: f32, z: f32) {
        let mut ship = app.world_mut().resource_mut::<ShipState>();
        ship.x = x;
        ship.z = z;
    }

    // ── Entry tests ───────────────────────────────────────────────────

    #[test]
    fn ship_enters_region_when_moving_inside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 100.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        // Flush so region entity is queryable + system runs once
        app.update();
        // Ship at (0,0) is outside region at (100,0) with radius 50 → no entry
        assert!(!is_inside(&mut app, region), "ship should start outside region");

        // Move ship inside the region
        set_ship_pos(&mut app, 120.0, 0.0); // 20 units from centre, well inside radius 50
        app.update();

        assert!(is_inside(&mut app, region),
            "ship should enter region when moving inside");
    }

    // ── Exit tests ────────────────────────────────────────────────────

    #[test]
    fn ship_exits_region_when_moving_outside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 20.0, 0.0); // inside
        app.update(); // flush + system run → enters
        assert!(is_inside(&mut app, region), "ship should be inside after moving in");

        // Move ship outside
        set_ship_pos(&mut app, 100.0, 0.0); // far outside radius 50
        app.update();

        assert!(!is_inside(&mut app, region),
            "ship should exit region when moving outside");
    }

    // ── No-duplicate-while-inside test ────────────────────────────────

    #[test]
    fn no_duplicate_entered_while_staying_inside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        app.update(); // flush + system run → enters
        assert!(is_inside(&mut app, region), "ship should be inside after first tick");

        // Stay inside — tick again; membership should remain stable
        app.update();
        assert!(is_inside(&mut app, region),
            "ship should remain inside without duplicate entry");
    }

    // ── Despawn-implicit-exit test ────────────────────────────────────

    #[test]
    fn region_despawn_while_inside_emits_implicit_exit() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        app.update(); // flush + system run → enters
        assert!(is_inside(&mut app, region), "ship should be inside before despawn");

        // Despawn the region entity
        app.world_mut().despawn(region);
        app.update();

        assert!(!is_inside(&mut app, region),
            "ship should exit region when region is despawned");
    }

    // ── Edge: ship outside from start ─────────────────────────────────

    #[test]
    fn ship_outside_from_start_does_not_enter() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 200.0, 0.0); // far outside
        app.update();

        assert!(!is_inside(&mut app, region),
            "ship outside region should not enter");
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

    use std::time::Duration;
    use crate::region_effects::{DamageZoneEffect, RegionEffectsConfig as EffectsCfg};

    fn damage_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(RegionPlugin);
        // Manually control time — no TimePlugin. Bevy 0.18 Time is generic;
        // we insert Time<()> ourselves and use advance_by() before each update.
        app.insert_resource(Time::<()>::default());
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
            (crate::messages::Console::Helm, 25.0),
            (crate::messages::Console::Tactical, 25.0),
            (crate::messages::Console::Power, 25.0),
            (crate::messages::Console::Shields, 25.0),
        ])));
        app.insert_resource(ShipModifiers::new());
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    fn blocks_impulse_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(RegionPlugin);
        app.insert_resource(Time::<()>::default());
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipImpulse(ImpulseState::new()));
        app.insert_resource(ShipModifiers::new());
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    fn tick_with_dt(app: &mut App, dt_secs: f32) {
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(Duration::from_secs_f32(dt_secs));
        app.update();
    }

    fn spawn_blocks_impulse_region(app: &mut App, x: f32, z: f32, radius: f32) -> Entity {
        let config = EntityConfig {
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
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            science_console: None,
            sensors_console: None,
            shields_console: None,
            star: None,
            planet: None,
            asteroid_field: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    fn spawn_damage_zone(app: &mut App, x: f32, z: f32, radius: f32, dps: f32) -> Entity {
        let config = EntityConfig {
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: Some(EffectsCfg {
                damage_zone: Some(DamageZoneEffect { damage_per_second: dps }),
                ..Default::default()
            }),
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            science_console: None,
            sensors_console: None,
            shields_console: None,
            star: None,
            planet: None,
            asteroid_field: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    #[test]
    fn ship_in_damage_zone_takes_damage() {
        let mut app = damage_test_app();
        spawn_damage_zone(&mut app, 0.0, 0.0, 50.0, 50.0);
        tick_with_dt(&mut app, 0.1);

        let hull_hp = app.world().resource::<ShipHullIntegrity>().0.total_current();
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

        let hull_hp = app.world().resource::<ShipHullIntegrity>().0.total_current();
        assert!(
            (hull_hp - 100.0).abs() < 1e-6,
            "hull should remain at 100 when outside damage zone, got {}",
            hull_hp
        );
    }

    #[test]
    fn damage_zone_bypasses_shields() {
        let mut app = damage_test_app();
        // Add shields with known state
        use crate::simulation::ShipShields;
        use crate::shield::{ShieldSystem, ShieldConfig};
        app.insert_resource(ShipShields(ShieldSystem::new(&ShieldConfig {
            max_hp: 100,
            ..Default::default()
        })));

        spawn_damage_zone(&mut app, 0.0, 0.0, 50.0, 50.0);
        tick_with_dt(&mut app, 0.1);

        // Hull should have taken damage (bypassing shields)
        let hull_hp = app.world().resource::<ShipHullIntegrity>().0.total_current();
        assert!(
            (hull_hp - 95.0).abs() < 1e-6,
            "hull should be ~95 (damage bypassed shields), got {}",
            hull_hp
        );

        // Shields should be untouched (full HP)
        let shields = app.world().resource::<ShipShields>();
        for facing in &shields.0.facings {
            assert_eq!(facing.hp, 100, "shield facing should be undamaged");
        }
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

        let hull_hp = app.world().resource::<ShipHullIntegrity>().0.total_current();
        assert!(
            (hull_hp - 99.1).abs() < 0.001,
            "hull should be ~99.1 after 0.3s at 3 dps, got {}",
            hull_hp
        );
    }

    // ── BlocksImpulse tests ─────────────────────────────────────────────

    fn set_impulse_charging(app: &mut App) {
        let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
        imp.0.start_charge();
    }

    fn set_impulse_active(app: &mut App) {
        let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
        imp.0.start_charge();
        imp.0.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
    }

    fn assert_impulse_phase(app: &App, expected: ImpulsePhase) {
        let phase = app.world().resource::<ShipImpulse>().0.phase;
        assert_eq!(phase, expected, "expected impulse {:?}, got {:?}", expected, phase);
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
        assert_impulse_phase(&app, ImpulsePhase::Charging);

        // Tick — should trigger RegionEntered and cancel impulse
        tick_with_dt(&mut app, 0.016);

        assert_impulse_phase(&app, ImpulsePhase::Idle);
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
        assert_impulse_phase(&app, ImpulsePhase::Active);

        tick_with_dt(&mut app, 0.016);

        assert_impulse_phase(&app, ImpulsePhase::Idle);
    }

    #[test]
    fn staying_outside_blocks_impulse_region_leaves_impulse_unchanged() {
        let mut app = blocks_impulse_test_app();
        let _region = spawn_blocks_impulse_region(&mut app, 200.0, 0.0, 50.0);
        set_ship_pos(&mut app, 0.0, 0.0); // far outside
        tick_with_dt(&mut app, 0.016);


        set_impulse_charging(&mut app);
        tick_with_dt(&mut app, 0.016);

        assert_impulse_phase(&app, ImpulsePhase::Charging);
    }

    // ── Radar Dampening tests ───────────────────────────────────────────

    fn radar_dampening_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(RegionPlugin)
            .add_plugins(crate::modifier_coordination::ModifierCoordinationPlugin)
            .insert_resource(Time::<()>::default())
            .insert_resource(ShipState::new());
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    fn spawn_radar_dampening_region(app: &mut App, x: f32, z: f32, radius: f32, multiplier: f32) -> Entity {
        let config = EntityConfig {
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: Some(EffectsCfg {
                radar_dampening: Some(RadarDampeningEffect { range_modifier: multiplier }),
                ..Default::default()
            }),
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            science_console: None,
            sensors_console: None,
            shields_console: None,
            star: None,
            planet: None,
            asteroid_field: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    #[test]
    fn entering_radar_dampening_region_adds_modifier() {
        let mut app = radar_dampening_test_app();
        spawn_radar_dampening_region(&mut app, 0.0, 0.0, 50.0, -0.3);
        set_ship_pos(&mut app, 0.0, 0.0); // inside region at origin
        tick_with_dt(&mut app, 0.016);

        let modifiers = app.world().resource::<ShipModifiers>();
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
        let modifiers_before = app.world().resource::<ShipModifiers>();
        assert!(
            (modifiers_before.get(&ModifierSlot::RadarRange) - 1.0 / 1.3).abs() < 1e-6,
            "modifier should be present while inside region"
        );

        // Move ship outside
        set_ship_pos(&mut app, 200.0, 0.0);
        tick_with_dt(&mut app, 0.016);

        let modifiers_after = app.world().resource::<ShipModifiers>();
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
        let modifiers = app.world().resource::<ShipModifiers>();
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

        let modifiers = app.world().resource::<ShipModifiers>();
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
        app.add_plugins(RegionPlugin)
            .add_plugins(crate::modifier_coordination::ModifierCoordinationPlugin)
            .insert_resource(Time::<()>::default())
            .insert_resource(ShipState::new());
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    fn spawn_slow_zone(app: &mut App, x: f32, z: f32, radius: f32, thrust_modifier: Option<f32>, yaw_rate_modifier: Option<f32>) -> Entity {
        use crate::region_effects::RegionEffectsConfig as EffectsCfg;
        let config = EntityConfig {
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: Some(EffectsCfg {
                slow_zone: Some(SlowZoneEffect { thrust_modifier, yaw_rate_modifier }),
                ..Default::default()
            }),
            hull: None,
            collider: None,
            appearance: None,
            helm_console: None,
            weapons_console: None,
            engineering_console: None,
            captain_console: None,
            power: None,
            science_console: None,
            sensors_console: None,
            shields_console: None,
            star: None,
            planet: None,
            asteroid_field: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    fn check_modifier(app: &App, slot: ModifierSlot, expected: f32) {
        let modifiers = app.world().resource::<ShipModifiers>();
        assert!(
            (modifiers.get(&slot) - expected).abs() < 1e-6,
            "expected modifier multiplier {} for {:?}, got {}",
            expected, slot, modifiers.get(&slot)
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
        check_modifier(&app, ModifierSlot::MaxSpeed, 1.0 / 1.5);
    }

    /// RED 2: entering slow zone with yaw_rate_modifier registers MaxYawRate modifier
    #[test]
    fn entering_slow_zone_with_yaw_rate_modifier_registers_maxyawrate_modifier() {
        let mut app = slow_zone_test_app();
        spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, None, Some(-0.3));
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);

        // -0.3 bonus → 1/(1+0.3) = 0.7692
        check_modifier(&app, ModifierSlot::MaxYawRate, 1.0 / 1.3);
    }

    /// RED 3: entering slow zone with both fields registers both slots
    #[test]
    fn entering_slow_zone_with_both_fields_registers_both_slots() {
        let mut app = slow_zone_test_app();
        spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), Some(-0.3));
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);

        check_modifier(&app, ModifierSlot::MaxSpeed, 1.0 / 1.5);
        check_modifier(&app, ModifierSlot::MaxYawRate, 1.0 / 1.3);
    }

    /// RED 4: entering slow zone with both fields omitted registers nothing
    #[test]
    fn entering_slow_zone_with_both_fields_omitted_registers_nothing() {
        let mut app = slow_zone_test_app();
        spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, None, None);
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);

        check_modifier(&app, ModifierSlot::MaxSpeed, 1.0);
        check_modifier(&app, ModifierSlot::MaxYawRate, 1.0);
    }

    /// RED 5: entry clamps forward_speed to new effective max
    #[test]
    fn entering_slow_zone_clamps_forward_speed() {
        let mut app = slow_zone_test_app();
        spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);

        // Set ship speed above the clamped limit
        let mut ship = app.world_mut().resource_mut::<ShipState>();
        ship.forward_speed = 50.0;
        ship.x = 10.0; // inside region
        drop(ship);

        tick_with_dt(&mut app, 0.016);

        // After clamping: base max speed = 25.0, modifier = 0.6667, effective max = 16.667
        let ship = app.world().resource::<ShipState>();
        let expected_clamped = ShipPhysicsConfig::new().max_speed * (1.0 / 1.5);
        assert!(
            (ship.forward_speed - expected_clamped).abs() < 0.001,
            "expected forward_speed clamped to ~{}, got {}",
            expected_clamped, ship.forward_speed
        );
    }

    /// RED 6: entering slow zone does not clamp speed when already below limit
    #[test]
    fn entering_slow_zone_does_not_clamp_when_already_below_limit() {
        let mut app = slow_zone_test_app();
        spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);

        let mut ship = app.world_mut().resource_mut::<ShipState>();
        ship.forward_speed = 5.0;
        ship.x = 10.0; // inside region
        drop(ship);

        tick_with_dt(&mut app, 0.016);

        // 5.0 is already below effective max (16.667), should remain 5.0
        let ship = app.world().resource::<ShipState>();
        assert!(
            (ship.forward_speed - 5.0).abs() < 0.001,
            "forward_speed should remain 5.0, got {}",
            ship.forward_speed
        );
    }

    /// RED 7: exit removes MaxSpeed modifier, does NOT restore velocity
    #[test]
    fn exiting_slow_zone_removes_maxspeed_modifier() {
        let mut app = slow_zone_test_app();
        let _region = spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), Some(-0.3));
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);
        check_modifier(&app, ModifierSlot::MaxSpeed, 1.0 / 1.5);



        // Exit the region
        set_ship_pos(&mut app, 200.0, 0.0);
        tick_with_dt(&mut app, 0.016);

        check_modifier(&app, ModifierSlot::MaxSpeed, 1.0);
        check_modifier(&app, ModifierSlot::MaxYawRate, 1.0);
    }

    /// RED 8: exit does NOT restore previously-clamped velocity
    #[test]
    fn exiting_slow_zone_does_not_restore_velocity() {
        let mut app = slow_zone_test_app();
        let _region = spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), None);

        // Start with 50 speed, enter → clamped to ~16.667
        let mut ship = app.world_mut().resource_mut::<ShipState>();
        ship.forward_speed = 50.0;
        ship.x = 10.0; // inside region
        drop(ship);

        tick_with_dt(&mut app, 0.016);



        // Confirm speed was clamped
        let ship = app.world().resource::<ShipState>();
        assert!(
            (ship.forward_speed - 16.667).abs() < 0.001,
            "speed should be clamped to ~16.667, got {}",
            ship.forward_speed
        );

        // Exit the region
        set_ship_pos(&mut app, 200.0, 0.0);
        tick_with_dt(&mut app, 0.016);

        // Speed should REMAIN clamped (not restored to 50)
        let ship = app.world().resource::<ShipState>();
        assert!(
            (ship.forward_speed - 16.667).abs() < 0.001,
            "speed should remain clamped after exit (not restored), got {}",
            ship.forward_speed
        );
    }

    // ── Flag effect tests (CommsJam / SensorBlind) ─────────────────────────

    use crate::flag_kind::FlagKind;
    use crate::region_effects::{CommsJamEffect, SensorBlindEffect};

    fn flag_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(RegionPlugin)
            .add_plugins(crate::modifier_coordination::ModifierCoordinationPlugin)
            .insert_resource(Time::<()>::default())
            .insert_resource(ShipState::new());
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    /// Spawn a region with the CommsJam effect at the given position.
    fn spawn_comms_jam_region(app: &mut App, x: f32, z: f32, radius: f32) -> Entity {
        let config = EntityConfig {
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: Some(EffectsCfg {
                comms_jammed: Some(CommsJamEffect {}),
                ..Default::default()
            }),
            hull: None, collider: None, appearance: None,
            helm_console: None, weapons_console: None, engineering_console: None,
            captain_console: None, power: None, science_console: None,
            sensors_console: None,
            shields_console: None,
            star: None, planet: None, asteroid_field: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    /// Spawn a region with the SensorBlind effect at the given position.
    fn spawn_sensor_blind_region(app: &mut App, x: f32, z: f32, radius: f32) -> Entity {
        let config = EntityConfig {
            tags: vec!["region".to_string()],
            shape: Some(RegionShape::Sphere { radius }),
            effects: Some(EffectsCfg {
                sensor_blind: Some(SensorBlindEffect {}),
                ..Default::default()
            }),
            hull: None, collider: None, appearance: None,
            helm_console: None, weapons_console: None, engineering_console: None,
            captain_console: None, power: None, science_console: None,
            sensors_console: None,
            shields_console: None,
            star: None, planet: None, asteroid_field: None,
            station: None,
            faction: None,
            behaviour: None,
            radar_appearance: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    fn assert_flag(app: &App, flag: FlagKind, expected: bool) {
        let modifiers = app.world().resource::<ShipModifiers>();
        assert_eq!(modifiers.has_flag(&flag), expected,
            "expected flag {:?} to be {}, but got {}",
            flag, expected, !expected);
    }

    /// RED 1: entering a comms_jam region sets the CommsJammed flag
    #[test]
    fn entering_comms_jam_region_sets_flag() {
        let mut app = flag_test_app();
        spawn_comms_jam_region(&mut app, 0.0, 0.0, 50.0);
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);
        assert_flag(&app, FlagKind::CommsJammed, true);
    }

    /// RED 2: entering a sensor_blind region sets the SensorBlind flag
    #[test]
    fn entering_sensor_blind_region_sets_flag() {
        let mut app = flag_test_app();
        spawn_sensor_blind_region(&mut app, 0.0, 0.0, 50.0);
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);
        assert_flag(&app, FlagKind::SensorBlind, true);
    }

    /// RED 3: exiting a comms_jam region clears the flag
    #[test]
    fn exiting_comms_jam_region_clears_flag() {
        let mut app = flag_test_app();
        let _region = spawn_comms_jam_region(&mut app, 0.0, 0.0, 50.0);
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);
        assert_flag(&app, FlagKind::CommsJammed, true);



        // Exit the region
        set_ship_pos(&mut app, 200.0, 0.0);
        tick_with_dt(&mut app, 0.016);

        assert_flag(&app, FlagKind::CommsJammed, false);
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

        assert_flag(&app, FlagKind::CommsJammed, true);




        // Exit B: move to (-40,0) — still inside A (dist 40 < 80), outside B (dist 100 > 80)
        set_ship_pos(&mut app, -40.0, 0.0);
        tick_with_dt(&mut app, 0.016);

        assert_flag(&app, FlagKind::CommsJammed, true);




        // Exit A: move far away — outside both
        set_ship_pos(&mut app, -200.0, 0.0);
        tick_with_dt(&mut app, 0.016);

        assert_flag(&app, FlagKind::CommsJammed, false);
    }

    /// RED 5: region despawn while inside clears the flag
    #[test]
    fn region_despawn_while_inside_clears_flag() {
        let mut app = flag_test_app();
        let region = spawn_comms_jam_region(&mut app, 0.0, 0.0, 50.0);
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);
        assert_flag(&app, FlagKind::CommsJammed, true);



        // Despawn the region entity
        app.world_mut().despawn(region);
        tick_with_dt(&mut app, 0.016);

        assert_flag(&app, FlagKind::CommsJammed, false);
    }

    /// RED 9: region despawn while inside removes modifiers
    #[test]
    fn region_despawn_while_inside_removes_slow_zone_modifiers() {
        let mut app = slow_zone_test_app();
        let region = spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, Some(-0.5), Some(-0.3));
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);
        check_modifier(&app, ModifierSlot::MaxSpeed, 1.0 / 1.5);



        // Despawn the region (implicit exit)
        app.world_mut().despawn(region);
        tick_with_dt(&mut app, 0.016);

        check_modifier(&app, ModifierSlot::MaxSpeed, 1.0);
        check_modifier(&app, ModifierSlot::MaxYawRate, 1.0);
    }
}

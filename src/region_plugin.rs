use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::entity_spawner::{RegionShapeSection, RegionEffectsSection};
use crate::simulation::{Ship, ShipHullIntegrity, ShipImpulse, BreakdownQueueResource};
use crate::ship_state::ShipState;
use crate::region_effects::RegionEffectKind;

/// Resource tracking which entities are inside which regions.
#[derive(Resource, Default)]
pub struct RegionMembership {
    /// Maps ship entity → set of region entities the ship is currently inside.
    pub inside: HashMap<Entity, HashSet<Entity>>,
}

/// Fired when a subject entity enters a region.
#[derive(Message, Clone, Debug)]
pub struct RegionEntered {
    pub subject: Entity,
    pub region_entity: Entity,
}

/// Fired when a subject entity exits a region (or the region is despawned).
#[derive(Message, Clone, Debug)]
pub struct RegionExited {
    pub subject: Entity,
    pub region_entity: Entity,
}

pub struct RegionPlugin;

impl Plugin for RegionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RegionMembership>()
            .add_message::<RegionEntered>()
            .add_message::<RegionExited>()
            .add_systems(Update, (
                update_region_membership,
                apply_damage_zone_damage.after(update_region_membership),
                handle_blocks_impulse_region_enter.after(update_region_membership),
            ));
    }
}

/// Per-tick system: checks ship position against every region shape and
/// emits `RegionEntered` / `RegionExited` on boundary crossings.
///
/// Automatically handles region despawn: if a region was previously occupied
/// and is no longer in the ECS, an implicit `RegionExited` is emitted.
fn update_region_membership(
    mut membership: ResMut<RegionMembership>,
    mut entered: MessageWriter<RegionEntered>,
    mut exited: MessageWriter<RegionExited>,
    region_query: Query<(Entity, &Transform, &RegionShapeSection)>,
    ship_state: Res<ShipState>,
    ship_query: Query<Entity, With<Ship>>,
) {
    let Ok(ship_entity) = ship_query.single() else {
        return;
    };

    let ship_pos = glam::Vec3::new(ship_state.x, 0.0, ship_state.z);

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
        exited.write(RegionExited {
            subject: ship_entity,
            region_entity: *entity,
        });
    }

    // Detect enters: in current_inside but not in prev_inside
    for entity in current_inside.difference(&prev_inside) {
        entered.write(RegionEntered {
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
    hull: Option<ResMut<ShipHullIntegrity>>,
    breakdowns: Option<ResMut<BreakdownQueueResource>>,
) {
    let (Some(mut hull), Some(mut breakdowns)) = (hull, breakdowns) else {
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
                let (_, new_cumulative, new_count) = crate::damage::apply_hull_damage(
                    &mut hull.0,
                    total_damage,
                    breakdowns.cumulative_damage,
                );
                breakdowns.cumulative_damage = new_cumulative;
                let BreakdownQueueResource { queue, rng, .. } = &mut *breakdowns;
                for _ in 0..new_count {
                    queue.push_random(rng);
                }
            }
        }
    }
}

/// Cancels the ship's impulse drive (charging or active) when the ship enters
/// a region with the `BlocksImpulse` effect.
fn handle_blocks_impulse_region_enter(
    mut entered: MessageReader<RegionEntered>,
    region_query: Query<&RegionEffectsSection>,
    impulse: Option<ResMut<ShipImpulse>>,
) {
    let Some(mut impulse) = impulse else {
        return;
    };
    for ev in entered.read() {
        let Ok(effects) = region_query.get(ev.region_entity) else {
            continue;
        };
        if effects.0.iter().any(|e| *e == RegionEffectKind::BlocksImpulse) {
            impulse.0.cancel_charge();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_spawner::spawn_entity;
    use crate::entity_config::EntityConfig;
    use crate::region_shape::RegionShape;
    use crate::damage::HullIntegrity;
    use crate::simulation::{ShipHullIntegrity, ShipImpulse, BreakdownQueueResource};
    use crate::impulse::{ImpulseState, ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::region_effects::BlocksImpulseEffect;

    /// Build a minimal Bevy app with the region plugin.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(RegionPlugin)
            .insert_resource(ShipState::new());
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
            star: None,
            planet: None,
            asteroid_field: None,
        };
        let uuid = uuid::Uuid::new_v4().to_string();
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, &config, Vec3::new(x, 0.0, z), uuid, None)
    }

    /// Drain all pending `RegionEntered` messages (discard).
    fn drain_entered(app: &mut App) {
        app.world_mut().resource_mut::<Messages<RegionEntered>>().drain();
    }

    /// Drain all pending `RegionExited` messages (discard).
    fn drain_exited(app: &mut App) {
        app.world_mut().resource_mut::<Messages<RegionExited>>().drain();
    }

    fn set_ship_pos(app: &mut App, x: f32, z: f32) {
        let mut ship = app.world_mut().resource_mut::<ShipState>();
        ship.x = x;
        ship.z = z;
    }

    fn has_entered(out: &[RegionEntered], region: Entity) -> bool {
        out.iter().any(|e| e.region_entity == region)
    }

    fn has_exited(out: &[RegionExited], region: Entity) -> bool {
        out.iter().any(|e| e.region_entity == region)
    }

    // ── Entry tests ───────────────────────────────────────────────────

    #[test]
    fn ship_enters_region_when_moving_inside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 100.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        // Flush so region entity is queryable + system runs once
        app.update();
        // Ship at (0,0) is outside region at (100,0) with radius 50 → no entry
        drain_entered(&mut app);

        // Move ship inside the region
        set_ship_pos(&mut app, 120.0, 0.0); // 20 units from centre, well inside radius 50
        app.update();

        let entered_events: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionEntered>>()
            .drain()
            .collect();
        assert!(has_entered(&entered_events, region),
            "ship should enter region when moving inside");
    }

    // ── Exit tests ────────────────────────────────────────────────────

    #[test]
    fn ship_exits_region_when_moving_outside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 20.0, 0.0); // inside
        app.update(); // flush + system run → enters
        drain_entered(&mut app);

        // Move ship outside
        set_ship_pos(&mut app, 100.0, 0.0); // far outside radius 50
        app.update();

        let exited_events: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionExited>>()
            .drain()
            .collect();
        assert!(has_exited(&exited_events, region),
            "ship should exit region when moving outside");
    }

    // ── No-duplicate-while-inside test ────────────────────────────────

    #[test]
    fn no_duplicate_entered_while_staying_inside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        app.update(); // flush + system run → enters
        drain_entered(&mut app);
        drain_exited(&mut app);

        // Stay inside — tick again
        app.update();
        let entered_events: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionEntered>>()
            .drain()
            .collect();
        assert!(!has_entered(&entered_events, region),
            "should NOT emit RegionEntered again while ship stays inside");

        let exited_events: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionExited>>()
            .drain()
            .collect();
        assert!(!has_exited(&exited_events, region),
            "should NOT emit RegionExited while ship stays inside");
    }

    // ── Despawn-implicit-exit test ────────────────────────────────────

    #[test]
    fn region_despawn_while_inside_emits_implicit_exit() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        app.update(); // flush + system run → enters
        drain_entered(&mut app);
        drain_exited(&mut app);

        // Despawn the region entity
        app.world_mut().despawn(region);
        app.update();

        let exited_events: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionExited>>()
            .drain()
            .collect();
        assert!(has_exited(&exited_events, region),
            "ship should receive RegionExited when region is despawned while ship is inside");
    }

    // ── Edge: ship outside from start ─────────────────────────────────

    #[test]
    fn ship_outside_from_start_does_not_enter() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 200.0, 0.0); // far outside
        app.update();

        let entered_events: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionEntered>>()
            .drain()
            .collect();
        assert!(!has_entered(&entered_events, region),
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
        drain_entered(&mut app);
        drain_exited(&mut app);

        // Ship inside r1, outside r2
        set_ship_pos(&mut app, 10.0, 0.0);
        app.update();

        let entered: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionEntered>>()
            .drain()
            .collect();
        assert!(has_entered(&entered, r1), "should enter r1");
        assert!(!has_entered(&entered, r2), "should NOT enter r2");

        // Move to r2 — should exit r1, enter r2
        set_ship_pos(&mut app, 110.0, 0.0);
        app.update();

        let entered2: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionEntered>>()
            .drain()
            .collect();
        let exited2: Vec<_> = app.world_mut()
            .resource_mut::<Messages<RegionExited>>()
            .drain()
            .collect();
        assert!(has_entered(&entered2, r2), "should enter r2");
        assert!(has_exited(&exited2, r1), "should exit r1");
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
        app.insert_resource(ShipHullIntegrity(HullIntegrity::new()));
        app.init_resource::<BreakdownQueueResource>();
        app.world_mut().spawn((Ship, Transform::default()));
        app
    }

    fn blocks_impulse_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(RegionPlugin);
        app.insert_resource(Time::<()>::default());
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipImpulse(ImpulseState::new()));
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
            star: None,
            planet: None,
            asteroid_field: None,
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
                damage_zone: Some(DamageZoneEffect { dps }),
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
            star: None,
            planet: None,
            asteroid_field: None,
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

        let hull_hp = app.world().resource::<ShipHullIntegrity>().0.current();
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

        let hull_hp = app.world().resource::<ShipHullIntegrity>().0.current();
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
        let hull_hp = app.world().resource::<ShipHullIntegrity>().0.current();
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

        let hull_hp = app.world().resource::<ShipHullIntegrity>().0.current();
        assert!(
            (hull_hp - 99.1).abs() < 0.001,
            "hull should be ~99.1 after 0.3s at 3 dps, got {}",
            hull_hp
        );
    }

    #[test]
    fn damage_zone_feeds_breakdown_queue() {
        let mut app = damage_test_app();
        // High DPS to cross 10-HP threshold
        spawn_damage_zone(&mut app, 0.0, 0.0, 50.0, 100.0);
        // 0.15s at 100 dps = 15 HP → crosses 1 bucket (10 HP)
        tick_with_dt(&mut app, 0.15);

        let bd = app.world().resource::<BreakdownQueueResource>();
        assert_eq!(bd.queue.len(), 1, "should have 1 breakdown after crossing 10-HP threshold");
        assert!(
            (bd.cumulative_damage - 15.0).abs() < 1e-6,
            "cumulative_damage should be ~15, got {}",
            bd.cumulative_damage
        );

        // Second tick: total 30 HP → crosses 2 more buckets = 3 total
        tick_with_dt(&mut app, 0.15);
        let bd2 = app.world().resource::<BreakdownQueueResource>();
        assert_eq!(bd2.queue.len(), 3, "should have 3 breakdowns after 30 damage");
    }

    // ── BlocksImpulse tests ─────────────────────────────────────────────

    fn set_impulse_charging(app: &mut App) {
        let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
        imp.0.start_charge();
    }

    fn set_impulse_active(app: &mut App) {
        let mut imp = app.world_mut().resource_mut::<ShipImpulse>();
        imp.0.start_charge();
        imp.0.tick(IMPULSE_CHARGE_DURATION);
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
        drain_entered(&mut app);

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
        drain_entered(&mut app);

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
        drain_entered(&mut app);

        set_impulse_charging(&mut app);
        tick_with_dt(&mut app, 0.016);

        assert_impulse_phase(&app, ImpulsePhase::Charging);
    }
}

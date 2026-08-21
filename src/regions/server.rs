use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::debug_overlay::{DamageLog, DamageLogEntry};
use crate::entity_spawner::{EntityUuid, RegionEffectsSection, RegionShapeSection};
use crate::lobby::Target;
use crate::messages::{GamePhase, ModifierSlot, ServerMessage};
use crate::modifiers::ShipModifiers;
use crate::region_effects::RegionEffectKind;
use crate::server_app::{Ship, SimOutbox};
use crate::ship_state::ShipPhysics;
use crate::simulation::GameOverReason;
use crate::simulation::{LocalShip, ShipImpulse};

/// Resource tracking which entities are inside which regions.
#[derive(Resource, Default)]
pub struct RegionMembership {
    /// Maps ship entity Ã¢â€ â€™ set of region entities the ship is currently inside.
    /// A `BTreeSet` of regions, not a `HashSet` (issue #965). The set
    /// differences in `update_region_membership` are what emit
    /// `RegionEntered`/`RegionExited`, and a ship that crosses two boundaries
    /// on one tick emits one event per region — so the set's iteration order
    /// IS the event order. Those events queue `WorldEvent::EnteredRegion` for
    /// the world-trigger pipeline and queue `ModifierEvent`s for broadcast,
    /// neither of which may depend on a hash seed. Ordering by `Entity` costs
    /// nothing at these sizes (a ship is inside a handful of regions at most)
    /// and is stable across processes because ECS entity allocation in a
    /// seeded run is.
    pub inside: HashMap<Entity, std::collections::BTreeSet<Entity>>,
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
            .add_systems(
                FixedUpdate,
                (
                    update_region_membership.in_set(crate::sim_sets::SimSet::Physics),
                    apply_damage_zone_damage
                        .in_set(crate::sim_sets::SimSet::Physics)
                        .after(update_region_membership),
                ),
            )
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
    ship_query: Query<(Entity, &ShipPhysics), With<Ship>>,
) {
    // Cache UUIDs for all current region entities (survives despawn).
    for (entity, _, _) in region_query.iter() {
        if let Ok(uuid) = uuid_query.get(entity) {
            membership.region_uuids.insert(entity, uuid.0.clone());
        }
    }

    // Collect region positions/shapes into a Vec so we can reuse them across
    // every ship without re-iterating the query per ship.
    let regions: Vec<(Entity, Vec3)> = region_query
        .iter()
        .map(|(entity, transform, _)| (entity, transform.translation))
        .collect();
    let region_shapes: HashMap<Entity, &RegionShapeSection> = region_query
        .iter()
        .map(|(entity, _, shape)| (entity, shape))
        .collect();

    // Track which ship entities are still present so stale membership entries
    // for despawned ships can be cleaned up at the end.
    let mut seen_ships: HashSet<Entity> = HashSet::new();

    for (ship_entity, physics) in ship_query.iter() {
        seen_ships.insert(ship_entity);
        let ship_pos = glam::Vec3::new(physics.x, 0.0, physics.z);

        // Determine current region occupancy for this ship.
        let current_inside: std::collections::BTreeSet<Entity> = regions
            .iter()
            .filter(|(entity, origin)| {
                region_shapes
                    .get(entity)
                    .is_some_and(|shape| shape.0.contains(ship_pos, *origin))
            })
            .map(|(entity, _)| *entity)
            .collect();

        let prev_inside = membership
            .inside
            .get(&ship_entity)
            .cloned()
            .unwrap_or_default();

        for entity in prev_inside.difference(&current_inside) {
            commands.trigger(RegionExited {
                subject: ship_entity,
                region_entity: *entity,
            });
        }

        for entity in current_inside.difference(&prev_inside) {
            commands.trigger(RegionEntered {
                subject: ship_entity,
                region_entity: *entity,
            });
        }

        membership.inside.insert(ship_entity, current_inside);
    }

    // Clean up membership entries for ships that no longer exist. Emit
    // implicit exit events so downstream systems (modifier caches, etc.)
    // can clear per-region state tied to the vanished ship.
    // Sorted, because these keys come from a `HashMap` and each one below
    // emits an implicit exit event per region the vanished ship was in.
    let mut stale_ships: Vec<Entity> = membership
        .inside
        .keys()
        .copied()
        .filter(|e| !seen_ships.contains(e))
        .collect();
    stale_ships.sort();
    for ship_entity in stale_ships {
        if let Some(prev_inside) = membership.inside.remove(&ship_entity) {
            for region_entity in prev_inside {
                commands.trigger(RegionExited {
                    subject: ship_entity,
                    region_entity,
                });
            }
        }
    }
}

/// Applies continuous damage from `DamageZone` regions to every ship each tick
/// (player + NPCs). Damage is split via the zone's `shield_pierce` field: the
/// pierced fraction goes straight to the hull, and the absorbed fraction is
/// distributed uniformly across all shield facings (since regions have no
/// bearing). Damaged regions are tracked per-ship via `RegionMembership`.
///
/// Player-only side effects (`DamageTaken` UI messages, `ShipDestroyed`,
/// `GameOver` transition, debug damage log) are gated on `Has<LocalShip>`.
/// NPCs that die inside a damage zone follow the same path as beam-kill:
/// emit `AiEntityDestroyed` + `EntityDespawned`, remove from `WorldResource`,
/// then despawn the entity.
fn apply_damage_zone_damage(
    time: Res<Time>,
    membership: Res<RegionMembership>,
    region_query: Query<(&RegionEffectsSection, Option<&EntityUuid>)>,
    mut ship_query: Query<
        (
            Entity,
            &mut crate::entity_spawner::EntitySystemHull,
            Option<&mut crate::simulation::ShipShields>,
            Option<&EntityUuid>,
            Has<LocalShip>,
            Option<&mut crate::entity_spawner::EntityShipArcHull>,
        ),
        With<Ship>,
    >,
    mut outbox: Option<ResMut<SimOutbox>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
    mut damage_log: Option<ResMut<DamageLog>>,
    mut destroyed_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::ai_plugin::AiEntityDestroyed>>,
    >,
    mut world: Option<ResMut<crate::lobby::WorldResource>>,
    mut commands: Commands,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
    // See `tick_beams_apply_damage` (issue #838): forget the killed uuid from
    // the registry so the reconcile sweep does not re-emit `EntityDespawned`.
    mut tracked: Option<ResMut<crate::server_app::TrackedEntities>>,
    // `Option<Res<_>>` so bare-`App` fixtures with no `LogFilterConfig`
    // inserted still pass parameter validation (see logging macro docs).
    log_cfg: Option<Res<crate::logging::LogFilterConfig>>,
    // God Mode (issue #900): `Option<Res<_>>` for the same reason as
    // `log_cfg` — bare-`App` region fixtures never insert it.
    god_mode: Option<Res<crate::server_app::GodMode>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    // Stable iteration order (issue #1052), the same mechanism
    // `server_app::handle_collisions` has used since #896 and for the same
    // reason. `ship_query.iter_mut()` walks the archetypes, which is an
    // artefact of how entities were spawned, moved and despawned rather than
    // anything the simulation authored — and the order is load-bearing here
    // because every ship in a damage zone draws from `SimStream::RegionDamage`
    // to pick which system absorbs the hit. Two ships swapping places swap
    // their draws, so the same total damage lands on different systems.
    // Measured (issue #1051): that is exactly what moved `rng_coverage` when a
    // debug-only component insert displaced the archetype order.
    let mut ship_order: Vec<((String, bevy::ecs::entity::EntityIndex), Entity)> = ship_query
        .iter()
        // Position 3 of the tuple below is the ship's `Option<&EntityUuid>`.
        .map(|(entity, _, _, uuid, _, _)| {
            (
                (
                    uuid.map(|u| u.0.clone()).unwrap_or_default(),
                    entity.index(),
                ),
                entity,
            )
        })
        .collect();
    ship_order.sort();

    for ship_entity in ship_order.into_iter().map(|(_, entity)| entity) {
        let Ok((ship_entity, mut hull, mut shields_opt, ship_uuid, is_local, mut arc_hull_opt)) =
            ship_query.get_mut(ship_entity)
        else {
            continue;
        };
        let Some(region_set) = membership.inside.get(&ship_entity) else {
            continue;
        };
        if region_set.is_empty() {
            continue;
        }

        for &region_entity in region_set.iter() {
            let Ok((effects, uuid_opt)) = region_query.get(region_entity) else {
                continue;
            };
            for effect in &effects.0 {
                let crate::region_effects::RegionEffectKind::DamageZone { dps, shield_pierce } =
                    effect
                else {
                    continue;
                };
                let total_damage = dps * dt;
                let (pierced, absorbed) =
                    crate::damage::split_damage_for_pierce(total_damage, *shield_pierce);

                // Absorbed portion: distribute uniformly across all shield
                // facings (regions have no bearing). Any shield leak adds
                // to the pierced hull damage.
                let mut hull_amount = pierced;
                let mut shield_amount = 0.0;
                if absorbed > 0.0 {
                    if let Some(shields) = shields_opt.as_deref_mut() {
                        let leak = shields.0.apply_uniform_damage(absorbed.round() as i32);
                        shield_amount = (absorbed - leak as f32).max(0.0);
                        hull_amount += leak as f32;
                    } else {
                        // No shields on this ship — treat absorbed as hull.
                        hull_amount += absorbed;
                    }
                }

                // God mode: local ship takes no damage.
                if is_local && god_mode.as_ref().is_some_and(|g| g.0) {
                    hull_amount = 0.0;
                }

                let (hull_applied, ship_destroyed) = if hull_amount > 0.0 {
                    crate::sim_rng::with_stream(
                        sim_rng.as_deref(),
                        crate::sim_rng::SimStream::RegionDamage,
                        |rng| {
                            let result =
                                crate::damage::apply_hull_damage(&mut hull.0, hull_amount, rng);
                            // Distribute the same absorbed amount across
                            // per-arc hull (issue #514). Skipped for NPCs (no
                            // `EntityShipArcHull`).
                            if let Some(ref mut arc_hull) = arc_hull_opt {
                                arc_hull.0.apply_damage(result.0, rng);
                            }
                            result
                        },
                    )
                } else {
                    (0.0, false)
                };

                // Balance tracer. A damage zone has no shooter, so `attacker`
                // is `None`; emitted for every ship in the zone, not just the
                // LocalShip. Skipped for a ship with no `EntityUuid` — there
                // is no identity to key a ledger on.
                if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
                    msgs.write(crate::balance::BalanceEvent::DamageApplied {
                        attacker: None,
                        victim: uuid.0.clone(),
                        // A damage zone only ever ticks ships; asteroids carry
                        // no hull the zone could touch.
                        victim_kind: crate::balance::VictimKind::Ship,
                        weapon: crate::balance::WEAPON_KIND_REGION.to_string(),
                        amount: total_damage,
                        shield_absorbed: shield_amount,
                        hull_damage: hull_applied,
                        system_hit: None,
                    });
                }

                // Human-readable logging alongside the structured BalanceEvent
                // (does NOT replace it). Same level discipline as the beam
                // site: a damage zone ticks *every* frame a ship is inside it,
                // so the per-tick line is `trace`; the one `info` edge is
                // destruction. Both entity-scoped to the victim so
                // `--log-entity` narrows to one hull.
                let source_label = uuid_opt
                    .map(|u| format!("region:{}", u.0))
                    .unwrap_or_else(|| "region:damage_zone".to_string());
                crate::ptrace!(
                    log_cfg,
                    crate::logging::LogCat::Damage,
                    entity = ship_entity,
                    "took {:.1} (shield {:.0}/hull {:.0}) from {}",
                    total_damage,
                    shield_amount,
                    hull_applied,
                    source_label
                );
                if ship_destroyed {
                    crate::pinfo!(
                        log_cfg,
                        crate::logging::LogCat::Damage,
                        entity = ship_entity,
                        "destroyed by {}",
                        source_label
                    );
                }

                // Debug damage log is a single-player developer overlay.
                if is_local {
                    if let Some(ref mut log) = damage_log {
                        log.push(DamageLogEntry {
                            source: source_label.clone(),
                            shield_arc: None,
                            amount: total_damage,
                        });
                    }
                }

                // DamageTaken / ShipDestroyed / GameOver are player-facing UI
                // events — only emit for the LocalShip.
                if is_local {
                    if let Some(ref mut ob) = outbox {
                        ob.0.push((
                            Target::All,
                            ServerMessage::DamageTaken {
                                hull: hull_applied,
                                shield: shield_amount,
                            },
                        ));
                    }
                    if ship_destroyed {
                        if let Some(ref mut ob) = outbox {
                            ob.0.push((Target::All, ServerMessage::ShipDestroyed));
                        }
                        if let Some(ref mut reason) = game_over_reason {
                            if reason.0.is_none() {
                                // Player-visible via the game-over overlays, so a
                                // `strings.csv` id, not English (issue #977); the
                                // HUD/GameOver paths resolve it client-side. All
                                // built-in ship-death sites latch the same id.
                                reason.0 = Some("server.game_over.ship_destroyed".into());
                                // The LocalShip died → defeat (#843), latched
                                // under the same first-write guard as the reason.
                                reason.1 = Some(crate::balance::Outcome::Defeat);
                                // EntityDestroyed for the player death, once
                                // (guarded by the first reason write). A damage
                                // zone has no shooter → no killer. Shares the
                                // `GameOverReason` latch with a scenario's
                                // `SetGameOverReason`; see the beam death site
                                // for why that coupling is accepted.
                                if let (Some(msgs), Some(uuid)) =
                                    (balance_events.as_mut(), ship_uuid)
                                {
                                    msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                                        victim: uuid.0.clone(),
                                        killer: None,
                                    });
                                }
                            }
                        }
                        if let Some(ref mut ns) = next_state {
                            ns.set(GamePhase::GameOver);
                        }
                    }
                } else if ship_destroyed {
                    // NPC destruction: mirror the beam-kill path so downstream
                    // world triggers and clients update consistently.
                    if let Some(uuid) = ship_uuid {
                        if let Some(ref mut world) = world {
                            world.0.entities.retain(|e| e.uuid != uuid.0);
                        }
                        if let Some(ref mut msgs) = destroyed_events {
                            msgs.write(crate::ai_plugin::AiEntityDestroyed {
                                entity_uuid: uuid.0.clone(),
                            });
                        }
                        if let Some(ref mut ob) = outbox {
                            ob.0.push((
                                Target::All,
                                ServerMessage::EntityDespawned {
                                    uuid: uuid.0.clone(),
                                },
                            ));
                        }
                        if let Some(t) = tracked.as_mut() {
                            t.forget(&uuid.0);
                        }
                        // EntityDestroyed for the NPC death, co-located with the
                        // AiEntityDestroyed write. Damage zone → no killer.
                        if let Some(msgs) = balance_events.as_mut() {
                            msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                                victim: uuid.0.clone(),
                                killer: None,
                            });
                        }
                        commands.entity(ship_entity).try_despawn();
                    }
                }
            }
        }
    }
}

/// Cancels the ship's impulse drive (charging or active) when the ship enters
/// a region with the `BlocksImpulse` effect.
///
/// Per-subject: writes to `ev.subject`'s own `ShipImpulse` component so an
/// NPC entering a `BlocksImpulse` region cancels its own impulse (a no-op
/// under current AI which never charges) without touching the player's.
fn handle_blocks_impulse_region_enter(
    trigger: On<RegionEntered>,
    region_query: Query<&RegionEffectsSection>,
    mut impulse_q: Query<&mut ShipImpulse>,
) {
    let ev = trigger.event();
    let Ok(mut impulse) = impulse_q.get_mut(ev.subject) else {
        return;
    };
    let Ok(effects) = region_query.get(ev.region_entity) else {
        return;
    };
    if effects.0.contains(&RegionEffectKind::BlocksImpulse) {
        impulse.0.cancel_charge();
    }
}

/// Clamps the ship's forward speed to the effective maximum when entering a
/// slow zone region. The modifier registration is handled by the coordinator's
/// `on_region_entered` observer — this system only clamps speed.
///
/// This is a non-modifier side effect that must run after the coordinator so
/// the effective max reflects the updated modifier state.
///
/// # Sanctioned out-of-band `ShipPhysics` writer (issue #699)
///
/// `integrate_ship_physics` is the sole *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`. This clamps
/// `forward_speed` directly and is an intentional exception: it is an
/// **observer** (`trigger: On<RegionEntered>`), not a scheduled system, so it
/// can fire at any point and cannot be sequenced relative to the helm
/// integrator inside a `SimSet` window. It deliberately does not opt into the
/// debug `HelmPhysicsWriteGuard`. See the writer-policy table on `ShipPhysics`
/// (`src/ship/state.rs`).
pub(crate) fn handle_slow_zone_speed_clamp(
    trigger: On<RegionEntered>,
    region_query: Query<&RegionEffectsSection>,
    modifiers_q: Query<&ShipModifiers>,
    mut ship_query: Query<&mut ShipPhysics>,
) {
    let ev = trigger.event();
    let Ok(effects) = region_query.get(ev.region_entity) else {
        return;
    };
    let has_slow = effects
        .0
        .iter()
        .any(|e| matches!(e, RegionEffectKind::SlowZone { .. }));
    if !has_slow {
        return;
    }
    let base_max = crate::ship_physics::ShipPhysicsConfig::new().max_speed;
    let default_modifiers;
    let modifiers: &ShipModifiers = match modifiers_q.get(ev.subject) {
        Ok(m) => m,
        Err(_) => {
            default_modifiers = ShipModifiers::new();
            &default_modifiers
        }
    };
    let effective_max = base_max * modifiers.get(&ModifierSlot::MaxSpeed);
    // Apply to the specific entity that entered the region, not all ships.
    if let Ok(mut physics) = ship_query.get_mut(ev.subject) {
        if physics.forward_speed.abs() > effective_max {
            physics.forward_speed = physics.forward_speed.signum() * effective_max;
        }
    }
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::damage::SystemHull;
    use crate::entity_config::EntityConfig;
    use crate::entity_spawner::spawn_entity;
    use crate::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::messages::ModifierSlot;
    use crate::modifiers::ShipModifiers;
    use crate::region_effects::{BlocksImpulseEffect, RadarDampeningEffect, SlowZoneEffect};
    use crate::region_shape::RegionShape;
    use crate::ship_physics::ShipPhysicsConfig;
    use crate::simulation::ShipImpulse;

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
            crate::simulation::Ship,
            Transform::default(),
            crate::ship_state::ShipPhysics::default(),
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
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
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
            operations: None,
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
            .query_filtered::<&mut crate::ship_state::ShipPhysics, With<LocalShip>>();
        let mut physics = q
            .single_mut(app.world_mut())
            .expect("expected LocalShip with ShipPhysics");
        physics.x = x;
        physics.z = z;
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Entry tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn ship_enters_region_when_moving_inside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 100.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        // Flush so region entity is queryable + system runs once
        app.update();
        // Ship at (0,0) is outside region at (100,0) with radius 50 Ã¢â€ â€™ no entry
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Exit tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn ship_exits_region_when_moving_outside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 20.0, 0.0); // inside
        app.update(); // flush + system run Ã¢â€ â€™ enters
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ No-duplicate-while-inside test Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn no_duplicate_entered_while_staying_inside() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        app.update(); // flush + system run Ã¢â€ â€™ enters
        assert!(
            is_inside(&mut app, region),
            "ship should be inside after first tick"
        );

        // Stay inside Ã¢â‚¬â€ tick again; membership should remain stable
        app.update();
        assert!(
            is_inside(&mut app, region),
            "ship should remain inside without duplicate entry"
        );
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Despawn-implicit-exit test Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn region_despawn_while_inside_emits_implicit_exit() {
        let mut app = test_app();
        let region = spawn_region(&mut app, 0.0, 0.0, RegionShape::Sphere { radius: 50.0 });
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        app.update(); // flush + system run Ã¢â€ â€™ enters
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Edge: ship outside from start Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Enter and exit across multiple regions Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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

        // Move to r2 Ã¢â‚¬â€ should exit r1, enter r2
        set_ship_pos(&mut app, 110.0, 0.0);
        app.update();

        assert!(is_inside(&mut app, r2), "should enter r2");
        assert!(!is_inside(&mut app, r1), "should exit r1");
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Damage Zone tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    use crate::region_effects::{DamageZoneEffect, RegionEffectsConfig as EffectsCfg};
    use std::time::Duration;

    fn ship_hull_hp(app: &mut App) -> f32 {
        let hull = app
            .world_mut()
            .query_filtered::<&crate::entity_spawner::EntitySystemHull, With<LocalShip>>()
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
        app.add_message::<crate::ai_plugin::AiEntityDestroyed>();
        app.init_resource::<crate::lobby::WorldResource>();
        use crate::shield::{ShieldConfig, ShieldSystem};
        let hull_config = &[
            (crate::messages::SystemId("helm".into()), 25.0),
            (crate::messages::SystemId("tactical".into()), 25.0),
            (crate::messages::SystemId("power".into()), 25.0),
            (crate::messages::SystemId("shields".into()), 25.0),
        ];
        app.world_mut().spawn((
            LocalShip,
            crate::simulation::Ship,
            Transform::default(),
            crate::ship_state::ShipPhysics::default(),
            crate::simulation::ShipShields(ShieldSystem::new(&ShieldConfig::default()), 0.5),
            crate::entity_spawner::EntitySystemHull(SystemHull::from_config(hull_config)),
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
            crate::simulation::Ship,
            Transform::default(),
            crate::ship_state::ShipPhysics::default(),
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
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
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
            operations: None,
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
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
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
            operations: None,
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
        use crate::shield::{ShieldConfig, ShieldSystem};
        use crate::simulation::ShipShields;
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
        use crate::shield::{ShieldConfig, ShieldSystem};
        use crate::simulation::ShipShields;
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
        use crate::shield::{ShieldConfig, ShieldSystem};
        use crate::simulation::ShipShields;
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
        use crate::damage::SystemHull;
        use crate::entity_spawner::{EntitySystemHull, EntityUuid};

        let mut app = damage_test_app();

        // Move the player (LocalShip) far outside the damage zone.
        set_ship_pos(&mut app, 500.0, 0.0);
        let player_hull_before = ship_hull_hp(&mut app);

        // Spawn an NPC ship at the origin with the Ship marker but no
        // LocalShip. Its EntitySystemHull starts at 100 HP.
        let npc_hull_config = &[(crate::messages::SystemId("captain".into()), 100.0)];
        let npc = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                EntityUuid("npc-damage-zone".to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                crate::ship_state::ShipPhysics {
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ BlocksImpulse tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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

        // Tick Ã¢â‚¬â€ should trigger RegionEntered and cancel impulse
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
                crate::simulation::Ship,
                Transform::default(),
                crate::ship_state::ShipPhysics {
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Radar Dampening tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn radar_dampening_test_app() -> App {
        let mut app = App::new();
        // Region observers first, then the modifier plugin's — matching the
        // production registration order (`RegionPlugin` before
        // `ModifierCoordinationPlugin`), which decides which observer sees a
        // `RegionEntered` first.
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(RegionPlugin)
            .add_plugins(crate::modifier_coordination::ModifierCoordinationPlugin);
        app.world_mut().spawn((
            LocalShip,
            crate::simulation::Ship,
            Transform::default(),
            crate::ship_state::ShipPhysics::default(),
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
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
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
            operations: None,
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Slow Zone tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    fn slow_zone_test_app() -> App {
        let mut app = App::new();
        // Region observers first, then the modifier plugin's — matching the
        // production registration order (`RegionPlugin` before
        // `ModifierCoordinationPlugin`), which decides which observer sees a
        // `RegionEntered` first.
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(RegionPlugin)
            .add_plugins(crate::modifier_coordination::ModifierCoordinationPlugin);
        app.world_mut().spawn((
            LocalShip,
            crate::simulation::Ship,
            Transform::default(),
            crate::ship_state::ShipPhysics::default(),
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
        use crate::region_effects::RegionEffectsConfig as EffectsCfg;
        let config = EntityConfig {
            reference_grid: None,
            name: None,
            display_name: None,
            star: None,
            planet: None,
            class: None,
            hull_id: None,
            power_rating: None,
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
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
            operations: None,
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

        // -0.5 bonus Ã¢â€ â€™ 1/(1+0.5) = 0.6667
        check_modifier(&mut app, ModifierSlot::MaxSpeed, 1.0 / 1.5);
    }

    /// RED 2: entering slow zone with yaw_rate_modifier registers MaxYawRate modifier
    #[test]
    fn entering_slow_zone_with_yaw_rate_modifier_registers_maxyawrate_modifier() {
        let mut app = slow_zone_test_app();
        spawn_slow_zone(&mut app, 0.0, 0.0, 50.0, None, Some(-0.3));
        set_ship_pos(&mut app, 10.0, 0.0); // inside
        tick_with_dt(&mut app, 0.016);

        // -0.3 bonus Ã¢â€ â€™ 1/(1+0.3) = 0.7692
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

    fn get_ship_physics(app: &mut App) -> crate::ship_state::ShipPhysics {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipPhysics, With<LocalShip>>();
        *q.single(app.world())
            .expect("expected LocalShip with ShipPhysics")
    }

    fn set_physics(app: &mut App, f: impl FnOnce(&mut crate::ship_state::ShipPhysics)) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_state::ShipPhysics, With<LocalShip>>();
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
        use crate::ship_state::ShipPhysics;

        let mut app = slow_zone_test_app();

        // Give the LocalShip high speed and place it inside the upcoming slow zone.
        set_physics(&mut app, |p| {
            p.forward_speed = 50.0;
            p.x = 10.0;
        });

        // Spawn a second NPC ship (now has Ship marker). Before the fix this made
        // single_mut() return Err and the player would not be clamped.
        app.world_mut().spawn((
            crate::simulation::Ship,
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
        use crate::ship_state::ShipPhysics;

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
                crate::simulation::Ship,
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
        let expected_clamped =
            crate::ship_physics::ShipPhysicsConfig::new().max_speed * (1.0 / 1.5);
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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Flag effect tests (CommsJam / SensorBlind) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    use crate::messages::FlagKind;
    use crate::region_effects::{CommsJamEffect, SensorBlindEffect};

    fn flag_test_app() -> App {
        let mut app = App::new();
        // Region observers first, then the modifier plugin's — matching the
        // production registration order (`RegionPlugin` before
        // `ModifierCoordinationPlugin`), which decides which observer sees a
        // `RegionEntered` first.
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(RegionPlugin)
            .add_plugins(crate::modifier_coordination::ModifierCoordinationPlugin);
        app.world_mut().spawn((
            LocalShip,
            crate::simulation::Ship,
            Transform::default(),
            crate::ship_state::ShipPhysics::default(),
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
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
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
            operations: None,
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
            mass: crate::entity_config::DEFAULT_ENTITY_MASS,
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
            operations: None,
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

        // Exit B: move to (-40,0) Ã¢â‚¬â€ still inside A (dist 40 < 80), outside B (dist 100 > 80)
        set_ship_pos(&mut app, -40.0, 0.0);
        tick_with_dt(&mut app, 0.016);

        assert_flag(&mut app, FlagKind::CommsJammed, true);

        // Exit A: move far away Ã¢â‚¬â€ outside both
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
}

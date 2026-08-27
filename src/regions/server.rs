use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::core::messages::{GamePhase, ModifierSlot, ServerMessage};
use crate::debug_overlay::{DamageLog, DamageLogEntry};
use crate::entities::spawner::{EntityUuid, RegionEffectsSection, RegionShapeSection};
use crate::lobby::Target;
use crate::modifiers::ShipModifiers;
use crate::regions::effects::RegionEffectKind;
use crate::server_app::GameOverReason;
use crate::server_app::{LocalShip, ShipImpulse};
use crate::server_app::{Ship, SimOutbox};
use crate::ship::state::ShipPhysics;

/// Resource tracking which entities are inside which regions.
#[derive(Resource, Default)]
pub struct RegionMembership {
    /// Maps ship entity → set of region entities the ship is currently inside.
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
            &mut crate::entities::spawner::EntitySystemHull,
            Option<&mut crate::server_app::ShipShields>,
            Option<&EntityUuid>,
            Has<LocalShip>,
            Option<&mut crate::entities::spawner::EntityShipArcHull>,
        ),
        With<Ship>,
    >,
    mut outbox: Option<ResMut<SimOutbox>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
    mut damage_log: Option<ResMut<DamageLog>>,
    mut destroyed_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::ai::server::AiEntityDestroyed>>,
    >,
    mut world: Option<ResMut<crate::lobby::WorldResource>>,
    mut commands: Commands,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
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
                let crate::regions::effects::RegionEffectKind::DamageZone { dps, shield_pierce } =
                    effect
                else {
                    continue;
                };
                let total_damage = dps * dt;
                let (pierced, absorbed) =
                    crate::ship::damage::split_damage_for_pierce(total_damage, *shield_pierce);

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
                            let result = crate::ship::damage::apply_hull_damage(
                                &mut hull.0,
                                hull_amount,
                                rng,
                            );
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
                    msgs.write(crate::core::balance::BalanceEvent::DamageApplied {
                        attacker: None,
                        victim: uuid.0.clone(),
                        // A damage zone only ever ticks ships; asteroids carry
                        // no hull the zone could touch.
                        victim_kind: crate::core::balance::VictimKind::Ship,
                        weapon: crate::core::balance::WEAPON_KIND_REGION.to_string(),
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
                        ob.push_reliable((
                            Target::All,
                            ServerMessage::DamageTaken {
                                hull: hull_applied,
                                shield: shield_amount,
                            },
                        ));
                    }
                    if ship_destroyed {
                        if let Some(ref mut ob) = outbox {
                            ob.push_reliable((Target::All, ServerMessage::ShipDestroyed));
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
                                reason.1 = Some(crate::core::balance::Outcome::Defeat);
                                // EntityDestroyed for the player death, once
                                // (guarded by the first reason write). A damage
                                // zone has no shooter → no killer. Shares the
                                // `GameOverReason` latch with a scenario's
                                // `SetGameOverReason`; see the beam death site
                                // for why that coupling is accepted.
                                if let (Some(msgs), Some(uuid)) =
                                    (balance_events.as_mut(), ship_uuid)
                                {
                                    msgs.write(
                                        crate::core::balance::BalanceEvent::EntityDestroyed {
                                            victim: uuid.0.clone(),
                                            killer: None,
                                        },
                                    );
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
                            msgs.write(crate::ai::server::AiEntityDestroyed {
                                entity_uuid: uuid.0.clone(),
                            });
                        }
                        if let Some(ref mut ob) = outbox {
                            ob.push_reliable((
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
                            msgs.write(crate::core::balance::BalanceEvent::EntityDestroyed {
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
    let base_max = crate::ship::physics::ShipPhysicsConfig::new().max_speed;
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
#[path = "server_tests.rs"]
mod tests;

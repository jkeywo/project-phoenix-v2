use bevy::prelude::*;
use std::collections::HashSet;

use crate::core::messages::{GamePhase, ModifierSlot, ServerMessage};
use crate::debug_overlay::{DamageLog, DamageLogEntry};
use crate::entities::spawner::{EntityUuid, RegionEffectsSection, RegionShapeSection};
use crate::lobby::Target;
use crate::modifiers::ShipModifiers;
use crate::regions::effects::RegionEffectKind;
use crate::server_app::{LocalShip, ShipImpulse};
use crate::server_app::{Ship, SimOutbox};
use crate::ship::state::ShipPhysics;

pub struct RegionPlugin;

/// Rebuild only read caches after snapshot adoption, before Input can consult
/// BlocksImpulse. No boundary events, modifier updates or actuator commands.
pub fn rebuild_region_read_cache(world: &mut World) {
    use crate::regions::field::{
        local_exposure, FieldFalloff, FieldPose, RegionFieldState, SampledRegionFieldPose,
    };
    let clock = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map(|tick| tick.0)
        .zip(
            world
                .get_resource::<Time<Fixed>>()
                .map(|fixed| fixed.timestep().as_secs_f32()),
        );
    let regions: Vec<_> = world
        .query::<(
            Entity,
            &Transform,
            &RegionShapeSection,
            Option<&RegionFieldState>,
            Option<&RegionEffectsSection>,
            Option<&EntityUuid>,
            Option<&crate::regions::signal_path::SignalMaterialConfig>,
        )>()
        .iter(world)
        .filter(|(_, _, _, field, effects, _, material)| {
            !(material.is_some() && field.is_none() && effects.is_none())
        })
        .filter_map(|(entity, transform, shape, field, _, uuid, _)| {
            let pose = if let Some(field) = field {
                clock.and_then(|(tick, dt)| field.sample_pose(tick, dt).ok())
            } else {
                Some(FieldPose::stationary(transform.translation))
            }?;
            Some((
                entity,
                shape.0.clone(),
                field.map(|field| field.profile.falloff),
                uuid.map(|uuid| uuid.0.clone()),
                pose,
            ))
        })
        .collect();
    let ships: Vec<_> = world
        .query_filtered::<(Entity, &ShipPhysics), With<Ship>>()
        .iter(world)
        .map(|(entity, physics)| (entity, Vec3::new(physics.x, 0.0, physics.z)))
        .collect();
    let mut membership = RegionMembership::default();
    for (entity, _, field, uuid, pose) in &regions {
        if let Some(uuid) = uuid {
            membership.region_uuids.insert(*entity, uuid.clone());
        }
        if field.is_some() {
            world
                .entity_mut(*entity)
                .insert(SampledRegionFieldPose(*pose));
        }
    }
    for (ship, position) in ships {
        let mut inside = std::collections::BTreeSet::new();
        for (region, shape, falloff, _, pose) in &regions {
            let contains = if falloff.is_some() {
                local_exposure(
                    shape,
                    FieldPose {
                        strength: 1.0,
                        ..*pose
                    },
                    FieldFalloff::Step,
                    position,
                ) > 0.0
            } else {
                shape.contains(position, pose.origin)
            };
            if contains {
                inside.insert(*region);
                if let Some(falloff) = falloff {
                    membership.exposure.insert(
                        (ship, *region),
                        local_exposure(shape, *pose, *falloff, position),
                    );
                }
            }
        }
        membership.inside.insert(ship, inside);
    }
    world.insert_resource(membership);
}

/// Reconcile scene-derived consumers after the full snapshot is adopted.
/// This invokes no boundary events, paid work or machinery commands.
pub fn rebuild_region_projections(world: &mut World) {
    use crate::regions::field::RegionFieldState;
    rebuild_region_read_cache(world);
    let mut regions: Vec<_> = world
        .query::<(
            Entity,
            &EntityUuid,
            &RegionEffectsSection,
            Option<&RegionFieldState>,
        )>()
        .iter(world)
        .filter_map(|(entity, uuid, effects, field)| {
            uuid::Uuid::parse_str(&uuid.0)
                .ok()
                .map(|uuid| (entity, uuid, effects.0.clone(), field.is_some()))
        })
        .collect();
    regions.sort_by_key(|(_, uuid, ..)| *uuid);
    let membership = world.resource::<RegionMembership>();
    let inside = membership.inside.clone();
    let exposures = membership.exposure.clone();
    for (entity, mut modifiers) in world
        .query_filtered::<(Entity, &mut ShipModifiers), With<Ship>>()
        .iter_mut(world)
    {
        let pending = std::mem::take(&mut modifiers.pending_events);
        modifiers.clear_region_sources();
        for (region, uuid, effects, field) in &regions {
            if inside
                .get(&entity)
                .is_some_and(|inside| inside.contains(region))
            {
                let exposure = if *field {
                    exposures.get(&(entity, *region)).copied().unwrap_or(0.0)
                } else {
                    1.0
                };
                crate::modifiers::coordination::apply_weighted_region_effects(
                    &mut modifiers,
                    *uuid,
                    effects,
                    exposure,
                );
            }
        }
        modifiers.pending_events = pending;
    }
    // Reachability gates Hail/Respond in Input, before the usual Broadcast
    // refresh. It is a read projection, not part of captured Comms intent.
    if world.contains_resource::<crate::comms::server::CommsRuntime>() {
        use bevy::ecs::system::RunSystemOnce;
        // The normal Broadcast updater also retires open hails. Preserve that
        // captured intent: restore is only rebuilding its derived reach views.
        let open_hails = world
            .resource::<crate::comms::server::CommsRuntime>()
            .open_hails
            .clone();
        let _ = world.run_system_once(crate::comms::server::update_comms_range_flags);
        world
            .resource_mut::<crate::comms::server::CommsRuntime>()
            .open_hails = open_hails;
    }
}

impl Plugin for RegionPlugin {
    fn build(&self, app: &mut App) {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<crate::regions::field::RegionFieldState>(
            StateClass::Folded,
            "spatial-fields",
        )
        .declare_state::<crate::regions::signal_path::SignalMaterialConfig>(
            StateClass::Derived,
            "spatial-fields",
        )
        .declare_state::<crate::regions::field::SampledRegionFieldPose>(
            StateClass::Derived,
            "spatial-fields",
        );
        app.init_resource::<RegionMembership>()
            .add_systems(
                FixedUpdate,
                (
                    update_region_membership
                        .in_set(crate::sim_sets::FixedStep::UpdateRegionMembership)
                        .in_set(crate::sim_sets::SimSet::Physics),
                    apply_damage_zone_damage
                        .in_set(crate::sim_sets::FixedStep::ApplyDamageZoneDamage)
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
    region_query: Query<(
        Entity,
        &Transform,
        &RegionShapeSection,
        Option<&crate::regions::field::RegionFieldState>,
        Option<&RegionEffectsSection>,
        Option<&EntityUuid>,
        Option<&crate::regions::signal_path::SignalMaterialConfig>,
    )>,
    mut ship_query: Query<
        (
            Entity,
            &ShipPhysics,
            Option<&mut ShipModifiers>,
            Option<&mut ShipImpulse>,
        ),
        With<Ship>,
    >,
    tick: Option<Res<crate::sim_tick::SimTick>>,
    fixed: Option<Res<Time<Fixed>>>,
) {
    use crate::regions::field::{local_exposure, FieldFalloff, FieldPose};
    let mut regions = Vec::new();
    for (entity, transform, shape, field, effects, uuid, material) in &region_query {
        if material.is_some() && field.is_none() && effects.is_none() {
            continue;
        }
        if let Some(uuid) = uuid {
            membership.region_uuids.insert(entity, uuid.0.clone());
        }
        let pose = if let Some(field) = field {
            tick.as_ref().zip(fixed.as_ref()).and_then(|(tick, fixed)| {
                field
                    .sample_pose(tick.0, fixed.timestep().as_secs_f32())
                    .ok()
            })
        } else {
            Some(FieldPose::stationary(transform.translation))
        };
        if let Some(pose) = pose {
            if field.is_some() {
                commands
                    .entity(entity)
                    .insert(crate::regions::field::SampledRegionFieldPose(pose));
            }
            regions.push((entity, shape, field, effects, uuid, pose));
        }
    }
    membership.exposure.clear();
    let mut seen_ships = HashSet::new();
    for (ship_entity, physics, mut modifiers, mut impulse) in &mut ship_query {
        seen_ships.insert(ship_entity);
        let ship_pos = glam::Vec3::new(physics.x, 0.0, physics.z);
        let mut current_inside = std::collections::BTreeSet::new();
        for (entity, shape, field, effects, uuid, pose) in &regions {
            let inside = if field.is_some() {
                local_exposure(
                    &shape.0,
                    FieldPose {
                        strength: 1.0,
                        ..*pose
                    },
                    FieldFalloff::Step,
                    ship_pos,
                ) > 0.0
            } else {
                shape.0.contains(ship_pos, pose.origin)
            };
            if !inside {
                continue;
            }
            current_inside.insert(*entity);
            if let Some(field) = field {
                let exposure = local_exposure(&shape.0, *pose, field.profile.falloff, ship_pos);
                membership.exposure.insert((ship_entity, *entity), exposure);
                if let Some(effects) = effects {
                    if let (Some(modifiers), Some(uuid)) = (modifiers.as_deref_mut(), uuid) {
                        if let Ok(uuid) = uuid::Uuid::parse_str(&uuid.0) {
                            crate::modifiers::coordination::apply_weighted_region_effects(
                                modifiers, uuid, &effects.0, exposure,
                            );
                        }
                    }
                    if exposure > 0.0 && effects.0.contains(&RegionEffectKind::BlocksImpulse) {
                        if let Some(impulse) = impulse.as_deref_mut() {
                            impulse.0.cancel_charge();
                        }
                    }
                }
            }
        }
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
/// Fleet-authoritative destruction (`ShipDestroyed` and the `GameOver`
/// transition) is gated on [`crate::lockstep::FleetSlotOf`], never
/// [`LocalShip`]. `LocalShip` is only this host's presentation projection: a
/// stationless GM peer has no local ship, but must retain the same destroyed
/// fleet hull and enter the same phase as the crew host. Local-only damage UI,
/// debug logging and God Mode still follow `LocalShip`.
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
            Option<&mut crate::entities::spawner::EntitySystemHull>,
            Option<&mut crate::server_app::ShipShields>,
            Option<&EntityUuid>,
            Has<LocalShip>,
            Has<crate::lockstep::FleetSlotOf>,
            Option<&mut crate::ship::compartment_damage::ShipDamage>,
        ),
        With<Ship>,
    >,
    mut outbox: Option<ResMut<SimOutbox>>,
    death_latch: crate::server_app::PlayerDeathLatch,
    mut damage_log: Option<ResMut<DamageLog>>,
    mut destroyed_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::ai::server::AiEntityDestroyed>>,
    >,
    mut world: Option<ResMut<crate::lobby::WorldResource>>,
    mut commands: Commands,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
    sim_rng: crate::sim_rng::LiveStream<'_, { crate::sim_rng::SimStream::RegionDamage as usize }>,
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
    let crate::server_app::PlayerDeathLatch {
        mission,
        mut next_state,
        reason: mut game_over_reason,
    } = death_latch;
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
        .map(|(entity, _, _, uuid, _, _, _)| {
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
        let Ok((
            ship_entity,
            mut hull,
            mut shields_opt,
            ship_uuid,
            is_local,
            is_fleet_ship,
            mut physical_damage,
        )) = ship_query.get_mut(ship_entity)
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
                let exposure = membership
                    .exposure
                    .get(&(ship_entity, region_entity))
                    .copied()
                    .unwrap_or(1.0);
                let total_damage =
                    (*dps as f64 * dt as f64 * exposure as f64).min(f32::MAX as f64) as f32;
                if total_damage <= 0.0 {
                    continue;
                }
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
                    crate::sim_rng::with_live_stream(sim_rng.as_deref(), |rng| {
                        if let Some(damage) = physical_damage.as_deref_mut() {
                            // A region has no projectile azimuth. Four equal,
                            // local cardinal exposures preserve the total field
                            // budget without inventing a moving attacker.
                            let mut applied = 0.0;
                            for incoming in [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]] {
                                let hit = phoenix_model::damage::DamageHit {
                                    incoming,
                                    profile: phoenix_model::damage::DamageProfile {
                                        damage_type: phoenix_model::damage::DamageType::Laser,
                                        penetration: 0.0,
                                    },
                                    payload: hull_amount as f64 * 0.25,
                                    aim: None,
                                    source: uuid_opt
                                        .map(|u| format!("region:{}", u.0))
                                        .unwrap_or_else(|| "region:damage-zone".into()),
                                    sample: None,
                                };
                                match damage.apply_hit(0.0, hit, rng) {
                                    Ok(result) => applied += result.structural_loss as f32,
                                    Err(error) => {
                                        warn!("physical region exposure refused: {error}")
                                    }
                                }
                            }
                            (applied, damage.catastrophic())
                        } else if let Some(hull) = hull.as_deref_mut() {
                            crate::ship::damage::apply_hull_damage(&mut hull.0, hull_amount, rng)
                        } else {
                            (0.0, false)
                        }
                    })
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

                // The continuous damage projection is local UI only.
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
                }

                // Fleet membership is the authoritative player-ship marker on
                // every simulation peer. A remote GM has no `LocalShip`, so
                // branching on that projection here would despawn the fleet
                // hull as an NPC on one peer while the crew host retained it.
                if is_fleet_ship && ship_destroyed {
                    if mission.continues_after_ship_loss() {
                        if hull_applied > 0.0 {
                            if let Some(uuid) = ship_uuid {
                                if let Some(ref mut events) = destroyed_events {
                                    events.write(crate::ai::server::AiEntityDestroyed {
                                        entity_uuid: uuid.0.clone(),
                                    });
                                }
                                if let Some(ref mut events) = balance_events {
                                    events.write(
                                        crate::core::balance::BalanceEvent::EntityDestroyed {
                                            victim: uuid.0.clone(),
                                            killer: None,
                                        },
                                    );
                                }
                            }
                        }
                    } else {
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
                                reason.1 = Some(crate::core::balance::Outcome::Defeat);
                                // EntityDestroyed for the fleet ship death, once
                                // (guarded by the first reason write). A damage zone
                                // has no shooter → no killer.
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
    membership: Res<RegionMembership>,
    mut impulse_q: Query<&mut ShipImpulse>,
) {
    let ev = trigger.event();
    let Ok(mut impulse) = impulse_q.get_mut(ev.subject) else {
        return;
    };
    let Ok(effects) = region_query.get(ev.region_entity) else {
        return;
    };
    if effects.0.contains(&RegionEffectKind::BlocksImpulse)
        && membership
            .exposure
            .get(&(ev.subject, ev.region_entity))
            .copied()
            .unwrap_or(1.0)
            > 0.0
    {
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
/// (`crates/phoenix-sim-gameplay/src/ship/state.rs`).
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

pub use phoenix_sim_gameplay::regions::server::*;

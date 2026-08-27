//! Collision handling for the simulation app assembly (issue #1199).
//!
//! Public surface: the `handle_collisions` system (registered in
//! `SimSet::Damage`) and `separate_ship_from_collision`, re-exported through
//! `crate::server_app`. Its wide raw parameter list is gathered into two named
//! `#[derive(SystemParam)]` bundles — [`CollisionBodies`] (the ship / asteroid /
//! body queries) and [`CollisionSinks`] (the outbox, phase/game-over latch,
//! damage log, despawn/balance message writers, world resource, commands and
//! tracked-entity registry) — the same idiom #1185 used; the bundles preserve
//! the exact access set, and the system destructures them back to its original
//! locals at entry so the body is byte-for-byte unchanged.
//!
//! Role: the per-tick collision response — hull/shield damage, the hard stop,
//! and de-overlap — for every ship (player + NPC), uniformly.
//!
//! Load-bearing invariant: ships and their contacts are resolved in a stable
//! world-id order (`collision_order_key`), never archetype/broadphase order, so
//! which ship in a mutual impact dies first is identical on every instance —
//! this is what keeps the collision digest deterministic (issue #896).

use super::*;

/// Everything `handle_collisions` needs to know about the other body in a
/// contact: where it is, how big it is, and — since issue #896 — what it is
/// called.
type CollisionBodyQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Transform,
        Option<&'static ColliderSection>,
        Option<&'static EntityUuid>,
        Option<&'static AsteroidUuid>,
    ),
>;

/// The sort key that puts collision handling in a stable world-ID order
/// (issue #896).
///
/// Authored uuid first — that is the identity two instances of the simulation
/// share, and the one the AC asks for — with the entity index behind it as the
/// tiebreak for anything the world file never named (bare test spawns, and
/// bodies carrying no uuid at all). Deliberately NOT the entity index alone:
/// two hosts agree on it only for as long as they agree on spawn order, which
/// is a weaker promise than the uuid already makes.
fn collision_order_key(
    entity: Entity,
    bodies: &CollisionBodyQuery,
) -> (String, bevy::ecs::entity::EntityIndex) {
    let uuid = bodies
        .get(entity)
        .ok()
        .and_then(|(_, _, entity_uuid, asteroid_uuid)| {
            entity_uuid
                .map(|u| u.0.clone())
                .or_else(|| asteroid_uuid.map(|u| u.0.clone()))
        })
        .unwrap_or_default();
    (uuid, entity.index())
}

/// The three queries `handle_collisions` reads bodies from: the ship query it
/// mutates, the asteroids it resolves damage against, and the generic
/// [`CollisionBodyQuery`] used to look a contacted body's transform, radius and
/// world id up. Gathered into a `SystemParam` bundle (issue #1199, the #1185
/// idiom); the access set is exactly the three queries' union, and the system
/// destructures it back to its original `asteroid_query` / `ship_query` /
/// `body_query` locals at entry so the body is unchanged.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CollisionBodies<'w, 's> {
    pub asteroid_query: Query<
        'w,
        's,
        (
            &'static Transform,
            &'static AsteroidUuid,
            Option<&'static AsteroidShieldPierce>,
        ),
        With<Asteroid>,
    >,
    pub ship_query: Query<
        'w,
        's,
        (
            Entity,
            &'static mut ShipPhysicsComponent,
            &'static mut CollisionCooldown,
            &'static mut crate::entities::spawner::EntitySystemHull,
            Option<&'static mut ShipShields>,
            Option<&'static ShipModifiers>,
            Option<&'static EntityUuid>,
            Option<&'static ColliderSection>,
            Has<LocalShip>,
            Option<&'static mut ShipImpulse>,
            Option<&'static mut crate::entities::spawner::EntityShipArcHull>,
        ),
        With<Ship>,
    >,
    pub body_query: CollisionBodyQuery<'w, 's>,
}

/// The sinks `handle_collisions` writes collision outcomes into: the outbound
/// message queue, the phase / game-over latch, the debug damage log, the
/// NPC-despawn and balance message streams, the world resource, deferred
/// commands, and the tracked-entity registry. Gathered into a `SystemParam`
/// bundle (issue #1199); every field keeps its exact `ResMut` / `Option<ResMut>`
/// / `MessageWriter` / `Commands` type, so the access set is unchanged, and the
/// system destructures it back to its original locals at entry.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CollisionSinks<'w, 's> {
    pub outbox: ResMut<'w, SimOutbox>,
    pub next_state: ResMut<'w, NextState<GamePhase>>,
    pub game_over_reason: ResMut<'w, GameOverReason>,
    pub damage_log: ResMut<'w, DamageLog>,
    pub destroyed_events: MessageWriter<'w, crate::ai::server::AiEntityDestroyed>,
    pub world: ResMut<'w, WorldResource>,
    pub commands: Commands<'w, 's>,
    // `Option<ResMut<Messages<_>>>` so bare-`App` fixtures that never
    // registered the message still pass Bevy's parameter validation.
    pub balance_events:
        Option<ResMut<'w, bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>>,
    // See `tick_beams_apply_damage` (issue #838): forget the killed uuid from
    // the registry so the reconcile sweep does not re-emit `EntityDespawned`.
    pub tracked: Option<ResMut<'w, TrackedEntities>>,
}

pub(crate) fn handle_collisions(
    time: Res<Time>,
    context: ReadRapierContext,
    bodies: CollisionBodies,
    sinks: CollisionSinks,
    // Seeded RNG + log filter + God Mode (issue #900), bundled: separately
    // they put this system one over Bevy's 16-parameter ceiling.
    ambient: SimRngAndLog,
) {
    // Destructure the bundles back to the pre-#1199 locals so the body below is
    // byte-for-byte unchanged.
    let CollisionBodies {
        asteroid_query,
        mut ship_query,
        body_query,
    } = bodies;
    let CollisionSinks {
        mut outbox,
        mut next_state,
        mut game_over_reason,
        mut damage_log,
        mut destroyed_events,
        mut world,
        mut commands,
        mut balance_events,
        mut tracked,
    } = sinks;

    let dt = time.delta_secs();

    let Ok(ctx) = context.single() else { return };

    // Stable iteration order (issue #896). `ship_query.iter_mut()` walks the
    // archetypes, which is an artefact of how entities were spawned, moved and
    // despawned rather than anything the simulation authored — and the order
    // is load-bearing: a collision can destroy a ship, and which of two ships
    // in a mutual impact is resolved (and so which one dies) first decides the
    // outcome. Sorted by world id, every instance resolves them in the same
    // order.
    let mut ship_order: Vec<((String, bevy::ecs::entity::EntityIndex), Entity)> = ship_query
        .iter()
        // Position 6 of the tuple below is the ship's `Option<&EntityUuid>` —
        // read straight off this query rather than looked up again.
        .map(|(entity, _, _, _, _, _, uuid, ..)| {
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

    // Handle every ship (player + NPCs) uniformly. Per-entity CollisionCooldown,
    // ShipModifiers, ShipShields, EntitySystemHull, ShipImpulse. Player-only side
    // effects (damage messages, GameOver, debug log) are gated on `is_local`.
    for (_, ship_entity) in ship_order {
        let Ok((
            ship_entity,
            mut physics,
            mut cooldown,
            mut hull_comp,
            shields_opt,
            modifiers_comp,
            ship_uuid,
            ship_collider,
            is_local,
            mut impulse_opt,
            mut arc_hull_opt,
        )) = ship_query.get_mut(ship_entity)
        else {
            // NOT reachable via an earlier iteration of this same loop
            // despawning `ship_entity`: despawns in this system go through
            // `Commands`, which are deferred until the next `ApplyDeferred`
            // sync point, so an entity queued for despawn earlier in this
            // very call is still present and still queryable here. This arm
            // exists only because `Query::get_mut` returns a `Result` by
            // API — any entity in `ship_order` genuinely missing from
            // `ship_query` (a stale id from a prior tick, a test fixture
            // gap) falls back to skipping it rather than panicking.
            continue;
        };
        cooldown.remaining_secs = (cooldown.remaining_secs - dt).max(0.0);

        let default_modifiers;
        let modifiers: &ShipModifiers = match modifiers_comp {
            Some(m) => m,
            None => {
                default_modifiers = ShipModifiers::new();
                &default_modifiers
            }
        };

        // One collision per ship per tick, and *which* one must not be
        // rapier's business (issue #896). `contact_pairs_with(..).next()` took
        // whatever the narrow phase happened to hand back first — an order
        // that follows the broadphase's internal bookkeeping, and one a
        // parallel broadphase would not even produce consistently between
        // builds. The choice is the lowest world id instead: with a ship
        // wedged between two rocks, every instance of the simulation picks the
        // same rock, and so deals the same damage from the same bearing into
        // the same shield arc.
        let mut contacts: Vec<Entity> = ctx
            .contact_pairs_with(ship_entity)
            // `contact_pairs_with` yields every pair whose *bounding volumes*
            // overlap, not just the ones actually touching (see the method's
            // own doc pointer to `has_any_active_contact`). Filtering to real
            // contacts before the deterministic pick matters because two rocks
            // can have overlapping AABBs without their shapes touching, and a
            // lower-uuid rock merely near the ship must not out-rank a rock the
            // ship is actually embedded in.
            .filter(|pair| pair.has_any_active_contact())
            .filter_map(|pair| {
                if pair.collider1() == Some(ship_entity) {
                    pair.collider2()
                } else {
                    pair.collider1()
                }
            })
            .collect();
        contacts.sort_by_key(|other| collision_order_key(*other, &body_query));

        // DAMAGE is still one contact per ship per tick, and still the lowest
        // world id — the #896 guarantee above is unchanged.
        let Some(&attacker_entity) = contacts.first() else {
            continue;
        };

        // GEOMETRY, by contrast, is resolved against every contact, every tick,
        // ahead of the damage cooldown (issue #968).
        //
        // Separation used to sit behind the `remaining_secs` gate with the
        // damage, so a hull that drove into something was pushed back to its
        // surface once and then had a full second to bury itself again before
        // anything corrected it — measured at up to 6.5 units INSIDE a radius-12
        // `huge` asteroid on a `combat_test` run, with the hull grinding through
        // the rock rather than around it. Geometry is not a rate-limited event:
        // the ship may not take damage more than once a second, but it must never
        // be allowed to occupy the same space as a rock in between. Damage, the
        // hard stop and the impulse cancel all stay on the cooldown below, so the
        // hit rate is unchanged.
        //
        // Resolving only the damage contact would not survive that rate change.
        // `separate_ship_from_collision` snaps the hull onto ONE body's surface
        // with no regard for any other, so a ship touching two bodies closer
        // together than `r_ship + r_body + slop` gets pushed out of the
        // lowest-keyed one and into its neighbour. At 1 Hz that was a transient
        // nuisance the ship's own motion worked itself out of; at 60 Hz it would
        // be a steady state, because the pick is deterministic — the same body
        // wins every tick, so the hull would be pushed into the same neighbour
        // for ever and never corrected against it. Rocks that close together are
        // reachable wherever `[asteroid_field]` streaming puts them.
        //
        // So every contact is resolved, in the same world-id order, as a
        // Gauss-Seidel pass: each correction applies on top of the last, and the
        // order is a function of world ids rather than of rapier's internal
        // bookkeeping, so every instance of the simulation lands the hull in the
        // same place.
        for &other in &contacts {
            let body = body_query.get(other).ok();
            separate_ship_from_collision(
                &mut physics,
                collider_radius(ship_collider),
                body.map(|(transform, ..)| transform),
                collider_radius(body.and_then(|(_, collider, ..)| collider)),
            );
        }

        if cooldown.remaining_secs > 0.0 {
            continue;
        }

        // Cancel impulse charge on any ship that takes a collision hit.
        if let Some(ref mut impulse) = impulse_opt {
            impulse.0.cancel_charge();
        }

        let speed_at_impact = physics.forward_speed;
        physics.forward_speed = 0.0;
        let damage = collision_damage(speed_at_impact) as f32
            * modifiers.get(&ModifierSlot::HullDamageTaken);

        let asteroid_info = asteroid_query.get(attacker_entity).ok();
        let bearing = asteroid_info
            .map(|(t, _, _)| {
                attacker_bearing_relative(
                    t.translation.x,
                    t.translation.z,
                    physics.x,
                    physics.z,
                    physics.yaw,
                )
            })
            .unwrap_or(0.0);

        let source_label = asteroid_info
            .map(|(_, uuid, _)| format!("asteroid:{}", uuid.0))
            .unwrap_or_else(|| "collision".to_string());

        // Resolve the colliding asteroid's `shield_pierce` (missing → 0.0,
        // matching pre-#414 behaviour where all collision damage was first
        // absorbed by shields).
        let shield_pierce = asteroid_info
            .and_then(|(_, _, sp)| sp.map(|c| c.0))
            .unwrap_or(0.0);

        // Split impact damage by the asteroid's `shield_pierce`: the
        // pierced fraction goes straight to hull; the absorbed fraction
        // is mitigated by the facing shield quadrant (any leak adds to
        // hull damage).
        let (pierced, absorbed) =
            crate::ship::damage::split_damage_for_pierce(damage, shield_pierce);
        let mut total_hull = pierced;
        let mut shield_amount = 0.0;

        // Shields are optional per-ship. Absorb through them when present;
        // otherwise all absorbed damage leaks straight to hull.
        let arc_label = if let Some(mut shields) = shields_opt {
            let arc_idx = shields.0.facing_index_for_bearing(bearing);
            let label = shields.0.facings.get(arc_idx).map(|f| f.label.clone());
            if absorbed > 0.0 {
                let leak =
                    apply_damage_with_shields(absorbed.round() as i32, bearing, &mut shields.0);
                shield_amount = (absorbed - leak as f32).max(0.0);
                total_hull += leak as f32;
            }
            label
        } else {
            // No shields → the "absorbed" portion also lands on hull.
            total_hull += absorbed;
            None
        };

        // Entity-scoped trace covering *every* ship. The `DamageLog` below is
        // player-only and capped at 10 entries because it backs the damage
        // debug overlay; this is the channel that survives a headless run and
        // can be narrowed to one ship with `--log-entity`.
        crate::pdebug!(
            ambient.log,
            crate::logging::LogCat::Damage,
            entity = ship_entity,
            "collision: source={} amount={:.1} arc={:?} pierced={:.1} absorbed={:.1}",
            source_label,
            damage,
            arc_label,
            pierced,
            absorbed
        );

        // Debug damage log: player-only (single-player debug overlay).
        if is_local {
            damage_log.push(DamageLogEntry {
                source: source_label.clone(),
                shield_arc: arc_label,
                amount: damage,
            });
        }

        // What the shields actually lost, captured before the god-mode clamp
        // below. The shield hit was already written into `ShipShields` above,
        // and god mode does not put it back — so the balance tracer has to
        // report the real figure even when the wire message reports zero.
        let shield_absorbed_for_balance = shield_amount;

        // God mode: local ship takes no damage.
        if is_local && ambient.god_mode_active() {
            total_hull = 0.0;
            shield_amount = 0.0;
        }

        let mut ship_destroyed = false;
        let hull_applied = if total_hull > 0.0 {
            crate::sim_rng::with_stream(
                ambient.rng.as_deref(),
                crate::sim_rng::SimStream::CollisionDamage,
                |rng| {
                    let (applied, destroyed) = apply_hull_damage(&mut hull_comp.0, total_hull, rng);
                    // Distribute the same absorbed amount across the per-arc
                    // hull pool (issue #514) so arc tier tracking follows
                    // overall hull damage. Skipped when the ship has no
                    // `EntityShipArcHull` (NPCs).
                    if let Some(ref mut arc_hull) = arc_hull_opt {
                        arc_hull.0.apply_damage(applied, rng);
                    }
                    ship_destroyed = destroyed;
                    applied
                },
            )
        } else {
            0.0
        };

        // The `info` half of the collision damage logging: the per-hit line
        // above is `debug`/`trace` detail, but destruction is a state edge a
        // balancer reads as a headline. Same discipline as the beam, blaster,
        // torpedo, and region kill sites.
        if ship_destroyed {
            crate::pinfo!(
                ambient.log,
                crate::logging::LogCat::Damage,
                entity = ship_entity,
                "destroyed by {}",
                source_label
            );
        }

        // Balance tracer. Environmental damage has no attacker — the asteroid
        // that hit us is identified by the `collision` weapon kind, not by a
        // shooter uuid. Emitted for every ship, not just the LocalShip.
        // Skipped for a ship with no `EntityUuid`, which has no identity the
        // report could key a ledger on.
        if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
            msgs.write(crate::core::balance::BalanceEvent::DamageApplied {
                attacker: None,
                victim: uuid.0.clone(),
                // Only ships run this collision path; the asteroid is the
                // thing collided *with*, and takes no damage from it.
                victim_kind: crate::core::balance::VictimKind::Ship,
                weapon: crate::core::balance::WEAPON_KIND_COLLISION.to_string(),
                amount: damage,
                shield_absorbed: shield_absorbed_for_balance,
                hull_damage: hull_applied,
                system_hit: None,
            });
        }

        // DamageTaken / ShipDestroyed / GameOver are player-facing UI events.
        // Only emit for the LocalShip. NPCs use the AiEntityDestroyed +
        // EntityDespawned path (same as beam-kill).
        if is_local {
            outbox.push_reliable((
                Target::All,
                ServerMessage::DamageTaken {
                    hull: hull_applied,
                    shield: shield_amount,
                },
            ));
            if ship_destroyed {
                outbox.push_reliable((Target::All, ServerMessage::ShipDestroyed));
                if game_over_reason.0.is_none() {
                    // Player-visible via the game-over overlays, so a
                    // `strings.csv` id, not English (issue #977); the HUD and
                    // GameOver paths resolve it client-side. Every built-in
                    // ship-death site latches this same id.
                    game_over_reason.0 = Some("server.game_over.ship_destroyed".into());
                    // The LocalShip died → this run is a defeat (#843). Latched
                    // alongside the reason under the same first-write guard.
                    game_over_reason.1 = Some(crate::core::balance::Outcome::Defeat);
                    // EntityDestroyed for the player death, once (guarded by the
                    // first reason write). Environmental death → no killer.
                    // Shares the `GameOverReason` latch with a scenario's
                    // `SetGameOverReason`; see the beam death site (console/
                    // weapons/beam.rs) for why that coupling is accepted.
                    if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
                        msgs.write(crate::core::balance::BalanceEvent::EntityDestroyed {
                            victim: uuid.0.clone(),
                            killer: None,
                        });
                    }
                }
                next_state.set(GamePhase::GameOver);
            }
        } else if ship_destroyed {
            // NPC destruction: mirror the beam-kill path so downstream world
            // triggers and clients update consistently.
            if let Some(uuid) = ship_uuid {
                world.0.entities.retain(|e| e.uuid != uuid.0);
                destroyed_events.write(crate::ai::server::AiEntityDestroyed {
                    entity_uuid: uuid.0.clone(),
                });
                outbox.push_reliable((
                    Target::All,
                    ServerMessage::EntityDespawned {
                        uuid: uuid.0.clone(),
                    },
                ));
                if let Some(t) = tracked.as_mut() {
                    t.forget(&uuid.0);
                }
                // EntityDestroyed for the NPC death, co-located with the
                // AiEntityDestroyed write. Environmental death → no killer.
                if let Some(msgs) = balance_events.as_mut() {
                    msgs.write(crate::core::balance::BalanceEvent::EntityDestroyed {
                        victim: uuid.0.clone(),
                        killer: None,
                    });
                }
            }
            commands.entity(ship_entity).try_despawn();
        }
        cooldown.remaining_secs = 1.0;
    }
}

pub(crate) const COLLISION_SEPARATION_SLOP: f32 = 0.05;

fn collider_radius(collider: Option<&ColliderSection>) -> f32 {
    collider.map(|c| c.0.radius.max(0.0)).unwrap_or(0.0)
}

/// Pushes `physics.x`/`z` out along the contact normal so the ship no longer
/// overlaps what it hit. Sanctioned out-of-band `ShipPhysics` writer — see
/// `handle_collisions` and the writer-policy table on `ShipPhysics`.
pub(crate) fn separate_ship_from_collision(
    physics: &mut ShipPhysicsComponent,
    ship_radius: f32,
    attacker_transform: Option<&Transform>,
    attacker_radius: f32,
) {
    let Some(attacker_transform) = attacker_transform else {
        return;
    };
    let min_dist = ship_radius + attacker_radius + COLLISION_SEPARATION_SLOP;
    if min_dist <= 0.0 {
        return;
    }

    let dx = physics.x - attacker_transform.translation.x;
    let dz = physics.z - attacker_transform.translation.z;
    let dist_sq = dx * dx + dz * dz;
    let (nx, nz, dist) = if dist_sq > 1e-6 {
        let dist = dist_sq.sqrt();
        (dx / dist, dz / dist, dist)
    } else {
        // Degenerate overlap: step back opposite the ship's current forward.
        (-simmath::sin(physics.yaw), simmath::cos(physics.yaw), 0.0)
    };

    if dist < min_dist {
        physics.x = attacker_transform.translation.x + nx * min_dist;
        physics.z = attacker_transform.translation.z + nz * min_dist;
    }
}

//! Snapshot broadcasters and publish/HUD systems (issue #1199).
//!
//! Public surface: the `SimBroadcaster` factories [`sim_state_broadcaster`],
//! [`modifier_events_broadcaster`], [`sim_outbox_broadcaster`]; the per-audience
//! snapshot builders (`build_sim_state_entity_states`, the `build_station_*` /
//! `build_control_source_snapshots` family); [`StationImportanceRes`] and its
//! ingest/drain; and the publish systems (`publish_viewscreen_blackboard`,
//! `broadcast_blackboard_updates`, `broadcast_shield_status`, the game-over /
//! world-setup / cache-reset / reconnect broadcasts, and `reconcile_runtime_entities`).
//! All re-exported through `crate::server_app`.
//!
//! Role: turns authoritative tick state into the outbound wire messages, gated
//! by the delta caches so only changed detail is sent.
//!
//! Load-bearing invariant: anything collected from a `HashMap` before it reaches
//! the wire is sorted (blackboard updates, spawned ids, boost pairs) so two
//! seeded runs emit byte-identical broadcasts; the delta caches are digest
//! exclusions (`cache_registry`), so the suppression logic here never feeds the
//! digest.

use super::*;

/// Returns a [`SimBroadcaster`] pre-configured with the `SimState` producer.
///
/// Broadcasts `SimState` at 10 Hz to all players (`Audience::All`).
/// Registered by [`add_simulation_plugins`] and the test harness in `test_app()`.
pub fn sim_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::Hz(10.0), |world: &mut World| {
        let entity_states = build_sim_state_entity_states(world);
        let station_hosts = build_station_host_snapshots(world);
        let station_health = build_station_health_snapshots(world);
        // Fold this tick's host state (objectives + Red Alert) into the
        // authoritative importance projection before reading it, so the snapshot
        // reflects the same tick's events (issue #1101). The drain of visits runs
        // frame-driven elsewhere; this ingest only ever raises flags on their
        // edge, so it can never resurrect a visit-cleared unread.
        ingest_station_importance(world);
        let station_importance = build_station_importance_snapshots(world);
        let control_sources = build_control_source_snapshots(world);

        // ── Emit SystemHullUpdate per recipient, only when that recipient's
        // *visible* detail changed (issue #737).
        //
        // Post issue #618 `SystemHullStatus` carries the authoritative
        // `SystemId`, display_name and tier. Post #737 the entry list is a
        // role-scoped projection instead of the whole ship, so the send is a
        // per-token fan-out rather than one `Target::All` push — see
        // `crate::console::repair::visibility`.
        crate::console::repair::visibility::push_hull_updates(world);

        let snapshot = crate::core::messages::SimSnapshot {
            entity_states,
            station_hosts,
            station_health,
            station_importance,
            control_sources,
        };
        vec![ServerMessage::SimState { snapshot }]
    })
}

pub(crate) fn build_control_source_snapshots(
    world: &mut World,
) -> BTreeMap<crate::core::messages::SystemId, String> {
    let mut query =
        world.query_filtered::<&crate::ship_plugin::ShipSystemControlSources, With<LocalShip>>();
    query
        .iter(world)
        .next()
        .map(|sources| {
            sources
                .0
                .entries()
                .map(|(system, source)| {
                    let effective = if sources.0.is_offline(system) {
                        crate::ship::control_source::ControlSource::Offline
                    } else {
                        *source
                    };
                    let label = match effective {
                        crate::ship::control_source::ControlSource::Human => "Human",
                        crate::ship::control_source::ControlSource::Ai => "Ai",
                        crate::ship::control_source::ControlSource::Offline => "Offline",
                    };
                    (system.clone(), label.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Compute this tick's authoritative per-Station health for the `SimState`
/// broadcast (issue #1100).
///
/// Reads the `LocalShip`'s [`HullVisibility`] and reduces it to one scalar per
/// owning Station via `station_fractions`. Mirrors [`build_station_host_snapshots`]:
/// a station-level figure that names no system, so it is safe for `Target::All`
/// — the same privacy argument as the ship-wide aggregate. Empty before the
/// ship spawns.
fn build_station_health_snapshots(
    world: &mut World,
) -> Vec<crate::core::messages::StationHealthSnapshot> {
    crate::console::repair::visibility::hull_visibility(world)
        .map(|vis| {
            vis.station_fractions()
                .into_iter()
                .map(
                    |(station, health)| crate::core::messages::StationHealthSnapshot {
                        station,
                        health,
                    },
                )
                .collect()
        })
        .unwrap_or_default()
}

/// Authoritative host-side per-Station importance (issue #1101).
///
/// The SAME structure the visit-clear ([`drain_station_visited`]) mutates and
/// the snapshot builder ([`build_station_importance_snapshots`]) reads. Held as
/// a persistent resource because `unread` is sticky across ticks until a visit
/// clears it — unlike health, which is recomputed wholesale every tick.
#[derive(Resource, Default)]
pub struct StationImportanceRes(pub crate::station_importance::StationImportance);

/// Fold this tick's authoritative host state into [`StationImportanceRes`]
/// (issue #1101).
///
/// Host-derived, tracer-bullet sourcing from state that already exists:
///
/// - Each mission objective's terminal edge (Completed/Failed) is a one-off
///   `unread` event for the Station it is attributed to. Attribution reuses the
///   repair `owner_of` bucketing (`HullVisibility::owned_station`): an objective
///   whose target names a Station-owned System is attributed to that Station,
///   otherwise it falls to the ship-wide core bucket — the same bucket ownerless
///   hull sums into.
/// - A raised Red Alert on the local ship is a continuing `critical` condition,
///   attributed to that ship-wide core bucket.
///
/// Authored-TOML importance is a deliberate future seam and is NOT built here.
pub(crate) fn ingest_station_importance(world: &mut World) {
    use crate::console::repair::visibility::CORE_BUCKET_ID;
    use crate::core::messages::{StationId, SystemId};

    // Objectives as (id, targets, status) — empty on an `App` with no manager.
    let objectives: Vec<(String, Vec<String>, crate::core::messages::ObjectiveStatus)> = world
        .get_resource::<crate::world::server::ObjectiveManagerRes>()
        .map(|m| {
            m.0.sorted_snapshots()
                .into_iter()
                .map(|o| (o.id, o.targets, o.status))
                .collect()
        })
        .unwrap_or_default();

    // Red Alert on the local ship, if it has spawned.
    let red_alert = {
        let mut q = world.query_filtered::<&crate::ship::state::ShipRedAlert, With<LocalShip>>();
        q.iter(world).next().map(|r| r.0).unwrap_or(false)
    };

    // Owned as a value (not borrowing the world), so the resource can be
    // borrowed mutably below.
    let vis = crate::console::repair::visibility::hull_visibility(world);
    let core = StationId(CORE_BUCKET_ID.to_string());

    let attributed: Vec<(String, StationId, crate::core::messages::ObjectiveStatus)> = objectives
        .into_iter()
        .map(|(id, targets, status)| {
            let station = vis
                .as_ref()
                .and_then(|v| {
                    targets
                        .iter()
                        .find_map(|t| v.owned_station(&SystemId(t.clone())))
                })
                .unwrap_or_else(|| core.clone());
            (id, station, status)
        })
        .collect();

    let critical_stations: Vec<StationId> = if red_alert {
        vec![core.clone()]
    } else {
        Vec::new()
    };

    if let Some(mut res) = world.get_resource_mut::<StationImportanceRes>() {
        res.0.ingest(attributed, critical_stations);
    }
}

/// Read this tick's authoritative per-Station importance for the `SimState`
/// broadcast (issue #1101).
///
/// A pure read of [`StationImportanceRes`] — [`ingest_station_importance`] has
/// already folded the tick's events in. Station-level and recipient-independent,
/// exactly like [`build_station_health_snapshots`], and safe for `Target::All`.
/// Empty before the resource is registered.
pub(crate) fn build_station_importance_snapshots(
    world: &mut World,
) -> Vec<crate::core::messages::StationImportanceSnapshot> {
    world
        .get_resource::<StationImportanceRes>()
        .map(|res| res.0.snapshots())
        .unwrap_or_default()
}

/// Drain `ClientMessage::StationVisited` from connected phones and clear the
/// visited Station's one-off `unread` importance flag (issue #1101 AC2).
///
/// Reads raw `InboundMessage` rather than `AdmittedCommands` — like the debug
/// drains, a visit changes no simulation outcome, only a presentation-attention
/// flag, so it never crosses command admission. The clear mutates host state
/// only; it reaches clients solely through the next `SimState` broadcast, which
/// is the authoritative lifecycle AC2 requires (never an optimistic client
/// clear). A continuing `critical` condition is untouched — [`StationImportance::visit`](crate::station_importance::StationImportance::visit)
/// clears only `unread` (AC3).
pub(crate) fn drain_station_visited(
    mut reader: MessageReader<crate::lobby::InboundMessage>,
    sessions: Res<crate::lobby::Sessions>,
    importance: Option<ResMut<StationImportanceRes>>,
) {
    let Some(mut importance) = importance else {
        return;
    };
    for ev in reader.read() {
        if let crate::core::messages::ClientMessage::StationVisited { station } = &ev.msg {
            // Only a connected player's visit counts — same authority gate the
            // debug drains apply.
            if sessions.0.players().iter().any(|p| p.token == ev.token) {
                importance.0.visit(station);
            }
        }
    }
}

pub(crate) fn build_station_host_snapshots(
    world: &mut World,
) -> Vec<crate::core::messages::StationHostSnapshot> {
    let mut query =
        world.query_filtered::<&crate::ship_plugin::VisitingStationHosts, With<LocalShip>>();
    query
        .iter(world)
        .next()
        .map(|hosts| {
            hosts
                .0
                .iter()
                .map(|assignment| crate::core::messages::StationHostSnapshot {
                    station: assignment.station.clone(),
                    host: assignment.host.clone(),
                    rating: assignment.rating.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Compute this tick's `EntityStateSnapshot` list for the `SimState` broadcast.
///
/// Extracted from [`sim_state_broadcaster`]'s producer closure (issue #927)
/// so it can be called directly in tests without going through the
/// Broadcaster/cadence machinery — see the `sim_state_entity_states` test
/// module below, which pins the shield-detail payload population directly
/// (target with shields -> `shields`/`shield_freq` present; entity with none
/// -> absent) without needing a full multi-tick cadence fixture.
pub(crate) fn build_sim_state_entity_states(
    world: &mut World,
) -> Vec<crate::core::messages::EntityStateSnapshot> {
    // ── Asteroids: position/yaw never changes — omit from per-tick payload.
    // The client already has asteroid positions from WorldSetup/AsteroidSpawned.
    // Health fields are delta-compressed: only emitted when changed since last tick.
    type AsteroidRaw = (
        String,
        Option<f32>,
        Option<f32>,
        Option<Vec<crate::core::messages::ShieldFacingStatus>>,
        Option<f32>,
    );
    let asteroid_raw: Vec<AsteroidRaw> = {
        let mut q = world.query::<(
            &AsteroidUuid,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::ship::shields::ShipShields>,
        )>();
        q.iter(world)
            .filter_map(|(uuid, hull_comp, shield_comp)| {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                // Per-facing detail + generator frequency (issue #927): the
                // SAME producer this ship's own `ShieldsBlackboard.facings`
                // uses (`ship::shields::shield_facing_statuses`) and the
                // same `ShipShields::frequency()`
                // `tick_frequency_hint_high_fidelity` reads for
                // `FrequencyHint` — one producer, no parallel derivation.
                // These were always sent as `None` before #927, which is
                // why `target_shields`/`target_shield_freq` were always
                // empty on the wire regardless of which console rendered them.
                let shields_wire = shield_comp
                    .map(|s| crate::ship::shields::shield_facing_statuses(&s.0.snapshot()));
                let shield_freq = shield_comp.map(|s| s.frequency());
                // Skip entirely when there are no health components (unbreakable asteroids).
                if hull_fraction.is_none() && shield_fraction.is_none() {
                    return None;
                }
                Some((
                    uuid.0.clone(),
                    hull_fraction,
                    shield_fraction,
                    shields_wire,
                    shield_freq,
                ))
            })
            .collect()
    };
    let asteroid_states: Vec<crate::core::messages::EntityStateSnapshot> = {
        let mut health_cache = world.resource_mut::<LastBroadcastEntityHealth>();
        asteroid_raw
            .into_iter()
            .filter_map(
                |(uuid, hull_fraction, shield_fraction, shields_wire, shield_freq)| {
                    let prev = health_cache
                        .0
                        .get(&uuid)
                        .cloned()
                        .unwrap_or((None, None, None, None));
                    let hull_changed = hull_fraction != prev.0;
                    let shield_changed = shield_fraction != prev.1;
                    // Bucketed projection (issue #927 gap-fill review): a
                    // raw `shields_wire != prev.2` compares `offline_remaining`
                    // at full precision, which `tick_shields` decrements every
                    // tick through a ~30s recovery — that re-triggered this
                    // gate on effectively every 10 Hz tick while any facing
                    // was offline. See `ship::shields::shields_delta_projection`.
                    let shields_changed =
                        crate::ship::shields::shields_delta_projection(&shields_wire)
                            != crate::ship::shields::shields_delta_projection(&prev.2);
                    let freq_changed = shield_freq != prev.3;
                    if !hull_changed && !shield_changed && !shields_changed && !freq_changed {
                        return None;
                    }
                    health_cache.0.insert(
                        uuid.clone(),
                        (
                            hull_fraction,
                            shield_fraction,
                            shields_wire.clone(),
                            shield_freq,
                        ),
                    );
                    Some(crate::core::messages::EntityStateSnapshot {
                        uuid,
                        position: None,
                        yaw: None,
                        hull_fraction,
                        shield_fraction,
                        flags: vec![],
                        shields: shields_wire,
                        shield_freq,
                        warp_out_remaining_secs: None,
                    })
                },
            )
            .collect()
    };

    // ── Non-asteroid entities (NPCs, stations): collect raw data first so
    // we can drop the ECS borrow before mutating the LastBroadcast* resources.
    type NpcRaw = (
        String,
        bevy::math::Vec3,
        f32,
        Option<f32>,
        Option<f32>,
        Option<Vec<crate::core::messages::ShieldFacingStatus>>,
        Option<f32>,
    );
    let npc_raw: Vec<NpcRaw> = {
        let mut q = world.query_filtered::<(
            &Transform,
            &EntityUuid,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::ship::shields::ShipShields>,
        ), Without<Asteroid>>();
        q.iter(world)
            .map(|(transform, uuid, hull_comp, shield_comp)| {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                // Per-facing detail + generator frequency (issue #927) —
                // same producer as the asteroid branch above; see the
                // comment there for why this closes the Sensors-panel gap.
                let shields_wire = shield_comp
                    .map(|s| crate::ship::shields::shield_facing_statuses(&s.0.snapshot()));
                let shield_freq = shield_comp.map(|s| s.frequency());
                let yaw = transform.rotation.to_euler(bevy::math::EulerRot::YXZ).0;
                (
                    uuid.0.clone(),
                    transform.translation,
                    yaw,
                    hull_fraction,
                    shield_fraction,
                    shields_wire,
                    shield_freq,
                )
            })
            .collect()
    };

    // Compare against last-broadcast positions and health; skip entities
    // where nothing changed.  Position/yaw suppressed below ~1 cm movement;
    // hull/shield suppressed when the f32 value is identical to last tick.
    const POS_THRESHOLD_SQ: f32 = 0.0001; // 0.01 world-unit radius
    const YAW_THRESHOLD: f32 = 0.001; // ~0.057 degrees
    let npc_states: Vec<crate::core::messages::EntityStateSnapshot> = {
        // Borrow position cache, then health cache separately (both mut).
        // Collect diffs first to avoid holding multiple mut borrows.
        type NpcDiff = (
            String,
            Option<[f32; 3]>,
            Option<f32>,
            Option<f32>,
            Option<f32>,
            Option<Vec<crate::core::messages::ShieldFacingStatus>>,
            Option<f32>,
        );
        let diffs: Vec<NpcDiff> = {
            let mut pos_cache = world.resource_mut::<LastBroadcastEntityPositions>();
            npc_raw
                .iter()
                .map(
                    |(
                        uuid,
                        pos,
                        yaw,
                        hull_fraction,
                        shield_fraction,
                        shields_wire,
                        shield_freq,
                    )| {
                        let moved = match pos_cache.0.get(uuid) {
                            Some(&(prev_pos, prev_yaw)) => {
                                (*pos - prev_pos).length_squared() > POS_THRESHOLD_SQ
                                    || (*yaw - prev_yaw).abs() > YAW_THRESHOLD
                            }
                            None => true,
                        };
                        if moved {
                            pos_cache.0.insert(uuid.clone(), (*pos, *yaw));
                        }
                        let out_pos = if moved {
                            Some([pos.x, pos.y, pos.z])
                        } else {
                            None
                        };
                        let out_yaw = if moved { Some(*yaw) } else { None };
                        (
                            uuid.clone(),
                            out_pos,
                            out_yaw,
                            *hull_fraction,
                            *shield_fraction,
                            shields_wire.clone(),
                            *shield_freq,
                        )
                    },
                )
                .collect()
        };
        let mut health_cache = world.resource_mut::<LastBroadcastEntityHealth>();
        diffs
            .into_iter()
            .filter_map(
                |(
                    uuid,
                    out_pos,
                    out_yaw,
                    hull_fraction,
                    shield_fraction,
                    shields_wire,
                    shield_freq,
                )| {
                    let prev = health_cache
                        .0
                        .get(&uuid)
                        .cloned()
                        .unwrap_or((None, None, None, None));
                    let hull_changed = hull_fraction != prev.0;
                    let shield_changed = shield_fraction != prev.1;
                    // Bucketed projection — see the asteroid branch above and
                    // `ship::shields::shields_delta_projection`'s doc comment.
                    let shields_changed =
                        crate::ship::shields::shields_delta_projection(&shields_wire)
                            != crate::ship::shields::shields_delta_projection(&prev.2);
                    let freq_changed = shield_freq != prev.3;
                    // Skip the entity entirely when nothing at all changed.
                    if out_pos.is_none()
                        && out_yaw.is_none()
                        && !hull_changed
                        && !shield_changed
                        && !shields_changed
                        && !freq_changed
                    {
                        return None;
                    }
                    if hull_changed || shield_changed || shields_changed || freq_changed {
                        health_cache.0.insert(
                            uuid.clone(),
                            (
                                hull_fraction,
                                shield_fraction,
                                shields_wire.clone(),
                                shield_freq,
                            ),
                        );
                    }
                    Some(crate::core::messages::EntityStateSnapshot {
                        uuid,
                        position: out_pos,
                        yaw: out_yaw,
                        hull_fraction: if hull_changed { hull_fraction } else { None },
                        shield_fraction: if shield_changed {
                            shield_fraction
                        } else {
                            None
                        },
                        flags: vec![],
                        shields: if shields_changed { shields_wire } else { None },
                        shield_freq: if freq_changed { shield_freq } else { None },
                        warp_out_remaining_secs: None,
                    })
                },
            )
            .collect()
    };

    asteroid_states.into_iter().chain(npc_states).collect()
}

/// Returns a [`SimBroadcaster`] pre-configured with the `ModifierAdded` and
/// `ModifierRemoved` producers.
///
/// Drains pending modifier events from [`ShipModifiers`] once per frame and
/// broadcasts each as a separate `ServerMessage` to all players (`Audience::All`).
/// Uses `Cadence::OnEvent` so the producer is called every frame regardless of
/// any Hz timer; an empty drain produces no outbound messages.
/// Registered by [`add_simulation_plugins`] and the test harness in `test_app()`.
///
/// After PR 6 (PRD #597): prefers the per-entity `ShipModifiers` component on
/// the LocalShip entity, falling back to the global Resource for tests that
/// only insert the Resource form.
pub fn modifier_events_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        use crate::modifiers::ModifierEvent;
        let events: Vec<ModifierEvent> = {
            let mut q =
                world.query_filtered::<&mut crate::modifiers::ShipModifiers, With<LocalShip>>();
            if let Some(mut mods_comp) = q.iter_mut(world).next() {
                std::mem::take(&mut mods_comp.pending_events)
            } else {
                Vec::new()
            }
        };
        events
            .into_iter()
            .map(|event| match event {
                ModifierEvent::Added {
                    source,
                    slot,
                    bonus,
                } => ServerMessage::ModifierAdded {
                    source,
                    slot,
                    bonus,
                },
                ModifierEvent::Removed { source, slot } => {
                    ServerMessage::ModifierRemoved { source, slot }
                }
            })
            .collect()
    })
}

/// Returns a [`SimBroadcaster`] that drains [`SimOutbox`] each frame and writes
/// each entry as an `OutboundMessage` with per-message target routing.
///
/// Uses `Cadence::OnEvent` so the producer fires every frame.  When the outbox
/// is empty the producer returns an empty `Vec` and no messages are emitted.
/// When populated (by any simulation system) the queued entries are flushed
/// directly to `OutboundMessage` with their original `Target` routing.
pub fn sim_outbox_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(Audience::All, Cadence::OnEvent, |world: &mut World| {
        let mut outbox = world.resource_mut::<SimOutbox>();
        let entries = std::mem::take(&mut outbox.0);
        for (target, msg) in entries {
            world.write_message(OutboundMessage {
                target,
                msg: msg.clone(),
                delivery: delivery_class_for_msg(&msg),
            });
        }
        vec![]
    })
}

/// Derive the delivery class for a `ServerMessage`.
///
/// Snapshot-class messages ride the unordered/no-retransmit DataChannel;
/// everything else (commands, lobby messages, Welcome, etc.) is reliable.
/// This is the single place where delivery class is decided server-side
/// (AC 1). The function is not exported — everything routes through
/// `sim_outbox_broadcaster` or `broadcast::dispatch::<Sim>`.
fn delivery_class_for_msg(msg: &ServerMessage) -> DeliveryClass {
    match msg {
        ServerMessage::SimState { .. }
        | ServerMessage::BlackboardUpdate { .. }
        | ServerMessage::ShieldStatus { .. }
        | ServerMessage::RepairState { .. }
        | ServerMessage::PowerState { .. }
        | ServerMessage::WeaponsUpdate { .. }
        | ServerMessage::SystemHullUpdate { .. } => DeliveryClass::Snapshot,
        _ => DeliveryClass::Reliable,
    }
}

// -- Systems -------------------------------------------------------------------

/// When the entity identified by `LastShipAttacker` no longer exists in the
/// world, clear the attacker record so stale references are not published.
pub(crate) fn clear_last_attacker_on_death(
    mut attacker_q: Query<&mut LastShipAttacker>,
    entity_uuids: Query<&EntityUuid>,
) {
    for mut attacker in attacker_q.iter_mut() {
        let uuid = match &attacker.0 {
            Some(u) => u.clone(),
            None => continue,
        };
        let still_alive = entity_uuids.iter().any(|eu| eu.0.as_str() == uuid.as_str());
        if !still_alive {
            attacker.0 = None;
        }
    }
}

/// When a ship's red alert transitions from on to off, clear the attacker
/// record — the threat has passed and the old attacker is no longer relevant.
///
/// Covers every ship (player + NPC), not just `LocalShip`: NPC captain-AI can
/// set its own `ShipRedAlert` (`handle_set_red_alert` in
/// `console::captain::server` dispatches `SetRedAlert` per-ship), and an
/// NPC that stands down should stop retaliating just like the player does.
///
/// `ShipRedAlert` only changes via an explicit assignment (never a
/// same-value rewrite in production), so `Changed<ShipRedAlert>` combined
/// with a boolean component reduces to exactly the on→off edge: the only way
/// a two-valued component both changes and reads `false` is if it was `true`
/// the instant before. This also sidesteps needing a per-entity "previous
/// value" store — a single shared `Local<bool>` (the pre-#685-followup
/// version) does not work once more than one ship is in the query, since it
/// would still only remember one entity's last state.
pub(crate) fn clear_last_attacker_on_red_alert_off(
    mut attacker_q: Query<
        (&mut LastShipAttacker, &crate::ship::state::ShipRedAlert),
        Changed<crate::ship::state::ShipRedAlert>,
    >,
) {
    for (mut attacker, ra) in &mut attacker_q {
        if !ra.0 {
            attacker.0 = None;
        }
    }
}

/// Publish the `LocalShip` viewscreen blackboard: hull/alert status plus the
/// scored objective pool the player ship's per-system AI (weapons, helm,
/// navigation) reads to pick a directive to serve.
///
/// # Why this MERGES rather than clobbers (issue #842)
///
/// After #842 the game-start player hull carries a default `[behaviour]`
/// doctrine, so the player ship holds BOTH `LocalShip` and `BehaviourSection`.
/// `aggregate_doctrine_blackboards` (`With<BehaviourSection>`) also writes the
/// same `VIEWSCREEN_SYSTEM_ID` entry, from the *template* doctrine. If this
/// system simply overwrote that entry — or vice versa — one objective pool
/// would silently erase the other: the doctrine writer clobbering here dropped
/// the player's scenario objectives entirely, so a shipped defence scenario
/// (`combat_test`) stopped developing combat and violated AC3 (scenario
/// objectives must outrank template doctrine).
///
/// Instead this system, pinned to run `.after(aggregate_doctrine_blackboards)`,
/// combines both sources into one scored pool: the global `ObjectiveManager`
/// scenario objectives (e.g. targeted `Destroy wave_N` @80) UNIONED with the
/// hull's template doctrine (untargeted `Destroy` @45 + `Hold` @20), re-sorted
/// descending by score. Scenario objectives coexist with and outrank the
/// standing default, so the player pursues the mission (restoring `combat_test`)
/// while the untargeted @45 remains a fallback that licenses proactive
/// engagement whenever no scenario objective is in play (the probe worlds).
///
/// The doctrine pool is scored fresh from the `BehaviourSection` component here
/// — NOT read back out of the blackboard entry the doctrine writer left. Those
/// two writers run at different cadences (the doctrine writer is gated to the
/// 10 Hz AI snapshot; this one runs every tick), so reading the published entry
/// and re-merging would re-consume this system's own prior output on the ticks
/// the doctrine writer skipped and duplicate the pool without bound. Rescoring
/// from the component is the one source that stays correct every tick.
///
/// A `LocalShip` with no `BehaviourSection` (pre-#842 shape) merges an empty
/// doctrine pool — i.e. behaves exactly as before.
pub(crate) fn publish_viewscreen_blackboard(
    time: Res<Time>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    hull_q: Query<&crate::entities::spawner::EntitySystemHull, With<LocalShip>>,
    local_uuid_q: Query<&crate::entities::spawner::EntityUuid, With<LocalShip>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    boost: Option<Res<CaptainPriorityBoost>>,
    mut ship_blackboards_q: Query<
        (
            &mut ShipSystemBlackboards,
            Option<&crate::ship::state::ShipRedAlert>,
            Option<&crate::ship::combat_activity::RecentCombatActivity>,
            Option<&crate::console::weapons::LastShipAttacker>,
            Option<&crate::entities::spawner::BehaviourSection>,
        ),
        With<LocalShip>,
    >,
) {
    use crate::core::messages::{SystemBlackboard, SystemId, ViewscreenBlackboard};
    use crate::objectives::WorldConditions;
    use crate::ship::system_registry::VIEWSCREEN_SYSTEM_ID;

    let entity_state = ship_blackboards_q.single().ok();
    // Lift Combat Lock + Science Target from the local ship's own radar
    // blackboards (issue #829), published this tick in `SimSet::Publish`.
    let combat_lock = entity_state.as_ref().and_then(|(bbs, _, _, _, _)| {
        match bbs
            .0
            .get(&crate::ship::system_registry::tactical_radar_system_id())
        {
            Some(SystemBlackboard::TacticalRadar(bb)) => bb.selected_target.clone(),
            _ => None,
        }
    });
    let science_target = entity_state.as_ref().and_then(|(bbs, _, _, _, _)| {
        match bbs
            .0
            .get(&crate::ship::system_registry::sensor_radar_system_id())
        {
            Some(SystemBlackboard::SensorRadar(bb)) => bb.selected_target.clone(),
            _ => None,
        }
    });
    let red_alert = entity_state
        .as_ref()
        .and_then(|(_, ra, _, _, _)| ra.map(|r| r.0))
        .unwrap_or(false);
    let last_damage_taken_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _, _)| act.and_then(|a| a.last_damage_taken));
    let last_hostile_fire_taken_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _, _)| act.and_then(|a| a.last_hostile_fire_taken));
    let last_weapon_fired_secs = entity_state
        .as_ref()
        .and_then(|(_, _, act, _, _)| act.and_then(|a| a.last_weapon_fired));
    let last_attacker_uuid = entity_state
        .as_ref()
        .and_then(|(_, _, _, la, _)| la.and_then(|l| l.0.clone()));

    let hull_integrity_pct = hull_q
        .single()
        .map(|h| {
            let max = h.0.total_max();
            let cur = h.0.total_current();
            if max > 0.0 {
                (cur / max * 100.0).clamp(0.0, 100.0)
            } else {
                100.0
            }
        })
        .unwrap_or(100.0);

    let conditions = WorldConditions {
        red_alert,
        hull_fraction: hull_integrity_pct / 100.0,
        attacked: false,
    };
    // Scope the captain boost to this (local) ship, so a boost only ever
    // reorders this ship's own objective consumers (issue #752).
    let local_uuid = local_uuid_q.single().ok().map(|u| u.0.clone());
    let scope = CaptainPriorityBoost::scope_key(local_uuid.as_deref());
    let captain_boost = boost.as_ref().and_then(|b| b.boost_arg(scope));
    let mut scored_objectives = objectives
        .as_ref()
        .map(|o| o.0.scored_pool_with_boost(&conditions, captain_boost))
        .unwrap_or_default();

    // Merge the hull's standing template doctrine into the scenario pool (see
    // the "why this MERGES" note above). Score the doctrine with the same
    // `attacked` signal the NPC path (`aggregate_doctrine_blackboards`) uses, so
    // a backfilled player and a world-spawned copy of the same hull evaluate
    // their identical doctrine identically (#842 AC4 symmetry). Both sites run
    // the one `objectives::last_landed_hit_secs` fold into the one
    // `objectives::attacked_recently` predicate (issue #1010) — a decaying
    // recency window over the last hit that CONNECTED, shields or hull, not the
    // `LastShipAttacker` latch — so the symmetry holds by construction rather
    // than by two copies of the rule staying in step. The scenario pool keeps
    // its own conditions (unchanged), so existing player-objective scoring is
    // untouched.
    if let Some((_, _, _, _, Some(behaviour))) = entity_state.as_ref() {
        // Sim seconds off the fixed clock (`Res<Time>` is `Time<Fixed>` inside
        // `FixedUpdate`), never a wall clock — AGENTS.md #7.
        let attacked_memory_secs = world_config
            .as_deref()
            .map(|wc| wc.global.attacked_memory_secs)
            .unwrap_or_else(|| {
                crate::entities::config::GlobalConfig::default().attacked_memory_secs
            });
        let doctrine_conditions = WorldConditions {
            red_alert,
            hull_fraction: hull_integrity_pct / 100.0,
            attacked: crate::objectives::attacked_recently(
                crate::objectives::last_landed_hit_secs(
                    last_damage_taken_secs,
                    last_hostile_fire_taken_secs,
                ),
                time.elapsed_secs(),
                attacked_memory_secs,
            ),
        };
        let doctrine_pool =
            crate::ai::score_doctrine_pool(&behaviour.0.doctrine, &doctrine_conditions);
        scored_objectives.extend(doctrine_pool);
    }

    // Re-sort the unioned pool descending by score. `sort_by` is stable, so
    // ties keep concatenation order (scenario objectives before doctrine ones —
    // a deterministic tiebreak the `top_destroy_objective_target` / helm
    // consumers rely on to read the highest-scored directive first). `total_cmp`
    // gives a total, deterministic order the rng-determinism guard depends on.
    scored_objectives.sort_by(|a, b| b.score.total_cmp(&a.score));

    let bb = ViewscreenBlackboard {
        red_alert,
        hull_integrity_pct,
        last_damage_taken_secs,
        last_weapon_fired_secs,
        last_attacker_uuid,
        scored_objectives,
        combat_lock,
        science_target,
    };

    // Write directly to the per-entity component.
    if let Some((mut entity_bbs, _, _, _, _)) = ship_blackboards_q.iter_mut().next() {
        entity_bbs.0.insert(
            SystemId(VIEWSCREEN_SYSTEM_ID.to_string()),
            SystemBlackboard::Viewscreen(bb),
        );
    }
}

/// Collision response for ships in contact: applies hull damage, brings the
/// ship to a hard stop, and de-overlaps it from the collider it hit.
///
/// # Sanctioned out-of-band `ShipPhysics` writer (issue #699)
///
/// `integrate_ship_physics` is the sole *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`. This system (with
/// `separate_ship_from_collision`) writes `forward_speed`/`x`/`z` directly and
/// is an intentional exception: collision response is a correction layered on
/// top of the helm integration, not a competing integrator. Routing it through
/// helm intent would let the ship integrate *into* geometry for a frame before
/// responding. It deliberately does not opt into the debug
/// `HelmPhysicsWriteGuard`. See the writer-policy table on `ShipPhysics`
/// (`src/ship/state.rs`).
/// Balance tracer: emit a [`BalanceEvent::PhaseChanged`] for every game-phase
/// transition. Reads the global `StateTransitionEvent<GamePhase>` stream, so it
/// fires exactly once per real transition without tapping each `next_state.set`
/// call site. Same-state "transitions" are skipped. `Option<ResMut<Messages>>`
/// so bare-`App` fixtures without the message registered still validate.
pub(crate) fn emit_phase_change_balance_events(
    mut reader: MessageReader<bevy::state::state::StateTransitionEvent<GamePhase>>,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
) {
    let Some(msgs) = balance_events.as_mut() else {
        return;
    };
    for ev in reader.read() {
        if ev.exited == ev.entered {
            continue;
        }
        let fmt = |s: &Option<GamePhase>| match s {
            Some(p) => format!("{p:?}"),
            None => "None".to_string(),
        };
        msgs.write(crate::core::balance::BalanceEvent::PhaseChanged {
            from: fmt(&ev.exited),
            to: fmt(&ev.entered),
        });
    }
}

/// Broadcast `ShieldStatus` at 10 Hz.
/// Sends to all players only when shield state changed; always sends to the
/// Shields console holder so their panel stays smooth during regeneration.
pub(crate) fn broadcast_shield_status(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    mut outbox: ResMut<SimOutbox>,
    sessions: Res<Sessions>,
    ship_query: Query<&ShipShields, With<LocalShip>>,
    mut last: ResMut<LastBroadcastShields>,
) {
    let Some(shields) = ship_query.iter().next() else {
        return;
    };
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let facings: Vec<ShieldFacingStatus> =
        crate::ship::shields::shield_facing_statuses(&shields.0.snapshot());

    let frequency = shields.frequency();
    if facings != last.0 {
        // State changed — broadcast to everyone.
        last.0 = facings.clone();
        outbox.0.push((
            Target::All,
            ServerMessage::ShieldStatus { facings, frequency },
        ));
    } else if let Some(token) = sessions.0.holder_for_station(&StationId("shields".into())) {
        // Nothing changed but the Shields holder still gets a periodic refresh
        // so regenerating HP stays smooth on their panel.
        outbox.0.push((
            Target::Token(token.to_string()),
            ServerMessage::ShieldStatus { facings, frequency },
        ));
    }
}

/// Tracks whether the initial WorldSetup broadcast has fired, so it only
/// goes out once per game.
#[derive(Resource, Default)]
pub(crate) struct WorldSetupBroadcast {
    sent: bool,
}

/// Broadcast `GameOver { reason, outcome }` to all players when the game enters
/// the GameOver phase. Reads both halves of the `GameOverReason` resource and
/// resets the REASON to `None` after broadcast.
///
/// Only the reason is taken. `.1` is read and left in place, for two separate
/// reasons that happen to agree: the headless exit report reads it after the
/// run to classify victory vs defeat (`src/bin/phoenix_headless.rs`), and
/// `state_digest` folds BOTH halves — clearing the outcome here would move
/// every digest of a run that reaches `GameOver` inside its window, for no
/// gain. `Outcome` is `Copy`, so reading it needs no mutation at all.
pub(crate) fn on_game_over_enter(
    mut game_over_reason: ResMut<GameOverReason>,
    mut outbox: ResMut<SimOutbox>,
) {
    let outcome = game_over_reason.1.map(|o| o.as_str().to_string());
    let reason = game_over_reason.0.take().unwrap_or_default();
    outbox
        .0
        .push((Target::All, ServerMessage::GameOver { reason, outcome }));
}

/// Reset all change-detection caches when entering InProgress so the first
/// broadcast tick always sends a full state to all players. Also covers the
/// multi-game restart case where stale cache from a previous game would
/// otherwise suppress initial updates.
///
/// Delegates to [`crate::core::broadcast::cache_registry::reset_all`] (issue
/// #613), the single place that knows about all six broadcast delta caches.
pub(crate) fn reset_broadcast_caches_on_start(
    mut hull: ResMut<LastBroadcastHull>,
    mut shields: ResMut<LastBroadcastShields>,
    mut positions: ResMut<LastBroadcastEntityPositions>,
    mut health: ResMut<LastBroadcastEntityHealth>,
    mut weapons: ResMut<LastWeaponsUpdate>,
    mut last_bb: ResMut<LastBroadcastBlackboards>,
    last_repair_bb: Option<ResMut<crate::console::repair::visibility::LastVisibleRepairBlackboard>>,
) {
    // Per-token repair-blackboard projections (issue #737) are a seventh delta
    // cache; clear them alongside the shared six so a restarted game re-sends.
    if let Some(mut last_repair_bb) = last_repair_bb {
        last_repair_bb.clear();
    }
    crate::core::broadcast::cache_registry::reset_all(
        &mut hull,
        &mut shields,
        &mut positions,
        &mut health,
        &mut weapons,
        &mut last_bb,
    );
}

/// Emit `BlackboardUpdate` for any system whose blackboard has changed since
/// the last broadcast. Reads from the `LocalShip` entity's per-entity component
/// (populated by `dual_publish_blackboards`). Runs in `SimSet::PublishAggregate`
/// (before `SimSet::Broadcast` so `broadcast::dispatch::<Sim>` sees the outbox entries).
///
/// Since issue #737 the *repair* blackboard is fanned out per session token
/// rather than broadcast to all: it carries exact per-system hull detail, and
/// who may see which system is a host-side decision. Every other blackboard
/// still goes out unprojected at `Target::All`.
/// The `Local` caches the `QueryState` across ticks. `World::query_filtered`
/// builds a *fresh* one on every call, and constructing it walks every archetype
/// in the world to work out which ones match — per tick, for a query that
/// resolves to a single `LocalShip` entity. Bevy asserts the state's world id on
/// use, so a cached state cannot silently be applied to the wrong world.
pub fn broadcast_blackboard_updates(
    world: &mut World,
    mut bb_query: Local<Option<QueryState<&'static ShipSystemBlackboards, With<LocalShip>>>>,
) {
    use crate::console::repair::visibility;

    world.init_resource::<visibility::LastVisibleRepairBlackboard>();

    let mut updates: Vec<(
        crate::core::messages::SystemId,
        crate::core::messages::SystemBlackboard,
    )> = {
        let q = bb_query.get_or_insert_with(|| {
            world.query_filtered::<&ShipSystemBlackboards, With<LocalShip>>()
        });
        let Some(bb) = q.iter(world).next() else {
            return;
        };
        let last = world.resource::<LastBroadcastBlackboards>();
        let mut changed: Vec<(
            crate::core::messages::SystemId,
            crate::core::messages::SystemBlackboard,
        )> =
            bb.0.iter()
                .filter(|(id, bb)| last.0.get(*id) != Some(*bb))
                .map(|(id, bb)| (id.clone(), bb.clone()))
                .collect();
        // Sorted because this vec becomes the `BlackboardUpdate` payload, and
        // it was collected from a `HashMap` whose order follows `RandomState`'s
        // per-process seed — two `--seed` runs emitted the same updates in a
        // different order every time. Sorting the changed set (a handful of
        // entries) rather than ordering the map itself keeps the per-tick
        // publish writes on cheap hash lookups.
        changed.sort_by(|a, b| a.0.cmp(&b.0));
        changed
    };

    let viewers = visibility::connected_viewers(world);

    // A token's station is an input to its repair-blackboard projection, so a
    // player changing station mid-game invalidates it even though nothing the
    // `LastBroadcastBlackboards` diff can see has changed. Without this, an
    // idle undamaged ship would leave the previous station's detail on that
    // phone until the internal blackboard next changed — possibly never.
    let stations_changed = {
        let cache = world.resource::<visibility::LastVisibleRepairBlackboard>();
        cache.stations_changed(&viewers)
    };

    // Prune first so a disconnected token cannot keep suppressing a resend if
    // it reconnects into the same station later in the same game.
    {
        let mut cache = world.resource_mut::<visibility::LastVisibleRepairBlackboard>();
        visibility::prune_repair_blackboard_cache(&mut cache, &viewers);
        cache.record_stations(&viewers);
    }

    if updates.is_empty() && !stations_changed {
        return;
    }

    // Station change with an otherwise-unchanged blackboard: re-feed the
    // current repair blackboard so the per-token projection is recomputed. The
    // per-token cache inside `project_repair_blackboards` still suppresses the
    // send for every token whose *view* did not actually change.
    if stations_changed
        && !updates
            .iter()
            .any(|(_, bb)| matches!(bb, crate::core::messages::SystemBlackboard::Repair(_)))
    {
        let mut q = world.query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        if let Some(bb) = q.iter(world).next() {
            if let Some((id, repair)) = bb
                .0
                .iter()
                .find(|(_, bb)| matches!(bb, crate::core::messages::SystemBlackboard::Repair(_)))
            {
                updates.push((id.clone(), repair.clone()));
            }
        }
    }

    // One `resource_mut` for the whole batch, not one per entry: each call is a
    // resource lookup plus a change-tick bump, and this loop runs every tick
    // that anything changed.
    {
        let mut last = world.resource_mut::<LastBroadcastBlackboards>();
        for (id, bb) in &updates {
            last.0.insert(id.clone(), bb.clone());
        }
    }

    let vis = visibility::hull_visibility(world);
    let mut cache = world
        .remove_resource::<visibility::LastVisibleRepairBlackboard>()
        .unwrap_or_default();
    let pending =
        visibility::project_repair_blackboards(updates, vis.as_ref(), &viewers, &mut cache);
    world.insert_resource(cache);
    world.resource_mut::<SimOutbox>().0.extend(pending);
}

/// When a player reconnects mid-game (Identify during InProgress),
/// `handle_identify_system` (in `LobbySystemSet`) queues a `Welcome { .. }` into
/// `LobbyOutbox` targeted at that player's
/// token. Detect this and push a full-state resync to *just that token* via
/// [`crate::core::broadcast::cache_registry::resync_for_token`] (issue #613).
///
/// This replaces the #599 quick fix, which reset all six shared broadcast
/// delta caches — correct for the reconnecting player, but it also forced
/// the *next* 10 Hz tick to broadcast full state to *every other* connected
/// client, since those caches are shared across all `Audience::All`
/// producers. The targeted resync leaves the shared caches untouched, so
/// every other client's next tick remains a normal delta.
pub(crate) fn refresh_caches_on_midgame_reconnect(world: &mut World) {
    let state = world.resource::<State<GamePhase>>();
    if *state.get() != GamePhase::InProgress {
        return;
    }
    let reconnecting_tokens: Vec<String> = {
        let lobby_outbox = world.resource::<LobbyOutbox>();
        lobby_outbox
            .0
            .iter()
            .filter_map(|(target, msg)| match (target, msg) {
                (Target::Token(token), ServerMessage::Welcome { .. }) => Some(token.clone()),
                _ => None,
            })
            .collect()
    };
    for token in reconnecting_tokens {
        crate::core::broadcast::cache_registry::resync_for_token(world, &token);
    }
}

/// Emit a single `WorldSetup` broadcast when the game enters `InProgress`.
/// Uses `State<GamePhase>` + sentry to fire exactly once.
pub(crate) fn broadcast_world_setup_on_start(
    state: Res<State<GamePhase>>,
    world: Res<WorldResource>,
    mut sent: ResMut<WorldSetupBroadcast>,
    mut outbox: ResMut<SimOutbox>,
) {
    if sent.sent || state.get() != &GamePhase::InProgress {
        return;
    }
    outbox.0.push((
        Target::All,
        ServerMessage::WorldSetup {
            world: world.0.clone(),
        },
    ));
    sent.sent = true;
}

/// For non-asteroid entities carrying `EntityUuid`:
/// - New entities (present in ECS, absent from `reported`) emit `EntitySpawned`
///   and are added to `WorldResource.entities` so they appear on reconnect `Welcome`.
/// - Missing entities (absent from ECS, present in `reported`) emit
///   `EntityDespawned` and are removed from `WorldResource.entities`.
///
/// Asteroids are excluded (they use `AsteroidSpawned` / `AsteroidDestroyed`).
///
/// On the very first `InProgress` tick, seeds `reported` from the initial
/// `WorldResource` entities so those are not re-broadcast.
pub(crate) fn reconcile_runtime_entities(
    mut registry: ResMut<TrackedEntities>,
    mut world: ResMut<WorldResource>,
    query: Query<
        (
            Entity,
            &EntityUuid,
            Option<&EntityId>,
            Option<&EntityName>,
            &Transform,
            Option<&RegionShapeSection>,
            Option<&EntityTagsSection>,
            Option<&RadarAppearanceSection>,
            Option<&AsteroidFieldSection>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::entities::spawner::EntityTarget>,
            Option<&crate::ship::shields::ShipShields>,
            Option<&crate::infrastructure::InfrastructureCondition>,
        ),
        Without<Asteroid>,
    >,
    mut outbox: ResMut<SimOutbox>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    mut positions_cache: ResMut<LastBroadcastEntityPositions>,
    mut health_cache: ResMut<LastBroadcastEntityHealth>,
) {
    // Build set of entity names referenced by active mission objectives.
    let active_objective_names: std::collections::HashSet<String> = objectives
        .as_ref()
        .map(|obj| {
            obj.0
                .sorted_snapshots()
                .into_iter()
                .filter(|s| s.status == crate::core::messages::ObjectiveStatus::Active)
                .flat_map(|s| s.targets)
                .collect()
        })
        .unwrap_or_default();
    // Build the current set of ECS entity UUIDs.
    let current: HashMap<String, Entity> = query
        .iter()
        .map(|(e, u, _, _, _, _, _, _, _, _, _, _, _)| (u.0.clone(), e))
        .collect();

    /// Serialise a `RegionShape` to the wire string (snake_case variant name).
    fn shape_to_wire(shape: &RegionShapeSection) -> String {
        use crate::regions::shape::RegionShape;
        match &shape.0 {
            RegionShape::Sphere { .. } => "sphere",
            RegionShape::Box { .. } => "box",
            RegionShape::Torus { .. } => "torus",
        }
        .to_string()
    }

    // Seed reported set from ECS on first in-progress frame so that initial
    // world entities (stars, planets, ships, fields) are not re-reported.
    // Also populate WorldData.entities so the reconnect Welcome includes them.
    if !registry.seeded {
        for (uuid, entity) in &current {
            registry.reported.insert(uuid.clone());
            if let Ok((
                _,
                _,
                id,
                name,
                transform,
                region_shape,
                entity_tags,
                radar_appearance,
                asteroid_field,
                hull_comp,
                entity_target,
                shield_comp,
                infrastructure,
            )) = query.get(*entity)
            {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    name: name.as_ref().map(|n| n.0.clone()),
                    hull_fraction,
                    shield_fraction,
                    // Issue #1025: minted from the LIVE track, so a structure
                    // reported after it degraded is reported as it now is.
                    infrastructure: infrastructure.and_then(|i| {
                        crate::core::messages::InfrastructureSnapshot::from_state(&i.0)
                    }),
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                    if snapshot.radius.is_none() {
                        match &shape.0 {
                            crate::regions::shape::RegionShape::Sphere { radius } => {
                                snapshot.radius = Some(*radius);
                            }
                            crate::regions::shape::RegionShape::Box { half_extents, .. } => {
                                let max_he = half_extents[0].max(half_extents[2]);
                                snapshot.radius = Some(max_he);
                                snapshot.half_extents = Some(*half_extents);
                            }
                            crate::regions::shape::RegionShape::Torus {
                                inner_radius,
                                outer_radius,
                            } => {
                                snapshot.radius = Some(*outer_radius);
                                snapshot.inner_radius = Some(*inner_radius);
                            }
                        }
                    }
                }
                if snapshot.shape.is_none() {
                    if let Some(field) = asteroid_field {
                        snapshot.shape = Some("torus".to_string());
                        snapshot.radius = Some(field.0.outer_radius);
                        snapshot.inner_radius = Some(field.0.inner_radius);
                    }
                }
                if let Some(ra) = radar_appearance {
                    if let Some(colour) = &ra.0.colour {
                        if colour.len() >= 3 {
                            snapshot.colour = Some([colour[0], colour[1], colour[2]]);
                        }
                    }
                    if let Some(region_colour) = &ra.0.region_colour {
                        if region_colour.len() >= 3 {
                            snapshot.region_colour =
                                Some([region_colour[0], region_colour[1], region_colour[2]]);
                        }
                    }
                    snapshot.radar_size = ra.0.size;
                    snapshot.radar_icon = ra.0.icon.clone();
                }
                if let Some(ref id) = snapshot.id {
                    snapshot.objective_target = active_objective_names.contains(id);
                }
                // Target info
                if let Some(t) = entity_target {
                    snapshot.target_tags = t.0.tags.clone();
                    snapshot.threat_level = Some(t.0.threat_level.as_str().to_string());
                    snapshot.target_description = t.0.description.clone();
                }
                upsert_world_entity(&mut world, snapshot);
            }
        }
        registry.seeded = true;
        return;
    }

    // Emit EntitySpawned for new entities, in UUID order.
    //
    // Iterating `current` directly announced the same ships in a different
    // order on every run, because `HashMap` order follows `RandomState`'s
    // per-process seed. Only the *newly seen* ids are sorted — almost always
    // none, and a handful on the tick a wave spawns — so this stays off the
    // per-tick cost of walking every entity.
    let mut newly_seen: Vec<(&String, &Entity)> = current
        .iter()
        .filter(|(uuid, _)| !registry.reported.contains(*uuid))
        .collect();
    newly_seen.sort_by(|a, b| a.0.cmp(b.0));
    for (uuid, entity) in newly_seen {
        if registry.reported.insert(uuid.clone()) {
            if let Ok((
                _,
                _,
                id,
                name,
                transform,
                region_shape,
                entity_tags,
                radar_appearance,
                asteroid_field,
                hull_comp,
                entity_target,
                shield_comp,
                infrastructure,
            )) = query.get(*entity)
            {
                let hull_fraction = hull_comp.map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        h.0.total_current() / max
                    } else {
                        1.0
                    }
                });
                let shield_fraction = shield_comp.map(|s| {
                    let total_hp: i32 = s.0.facings.iter().map(|f| f.hp).sum();
                    let total_max: i32 = s.0.facings.iter().map(|f| f.max_hp).sum();
                    if total_max > 0 {
                        total_hp as f32 / total_max as f32
                    } else {
                        0.0
                    }
                });
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    name: name.as_ref().map(|n| n.0.clone()),
                    hull_fraction,
                    shield_fraction,
                    // Issue #1025: minted from the LIVE track, so a structure
                    // reported after it degraded is reported as it now is.
                    infrastructure: infrastructure.and_then(|i| {
                        crate::core::messages::InfrastructureSnapshot::from_state(&i.0)
                    }),
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                    if snapshot.radius.is_none() {
                        match &shape.0 {
                            crate::regions::shape::RegionShape::Sphere { radius } => {
                                snapshot.radius = Some(*radius);
                            }
                            crate::regions::shape::RegionShape::Box { half_extents, .. } => {
                                let max_he = half_extents[0].max(half_extents[2]);
                                snapshot.radius = Some(max_he);
                                snapshot.half_extents = Some(*half_extents);
                            }
                            crate::regions::shape::RegionShape::Torus {
                                inner_radius,
                                outer_radius,
                            } => {
                                snapshot.radius = Some(*outer_radius);
                                snapshot.inner_radius = Some(*inner_radius);
                            }
                        }
                    }
                }
                if snapshot.shape.is_none() {
                    if let Some(field) = asteroid_field {
                        snapshot.shape = Some("torus".to_string());
                        snapshot.radius = Some(field.0.outer_radius);
                        snapshot.inner_radius = Some(field.0.inner_radius);
                    }
                }
                if let Some(ra) = radar_appearance {
                    if let Some(colour) = &ra.0.colour {
                        if colour.len() >= 3 {
                            snapshot.colour = Some([colour[0], colour[1], colour[2]]);
                        }
                    }
                    if let Some(region_colour) = &ra.0.region_colour {
                        if region_colour.len() >= 3 {
                            snapshot.region_colour =
                                Some([region_colour[0], region_colour[1], region_colour[2]]);
                        }
                    }
                    snapshot.radar_size = ra.0.size;
                    snapshot.radar_icon = ra.0.icon.clone();
                }
                if let Some(ref id) = snapshot.id {
                    snapshot.objective_target = active_objective_names.contains(id);
                }
                // Target info
                if let Some(t) = entity_target {
                    snapshot.target_tags = t.0.tags.clone();
                    snapshot.threat_level = Some(t.0.threat_level.as_str().to_string());
                    snapshot.target_description = t.0.description.clone();
                }
                upsert_world_entity(&mut world, snapshot.clone());
                outbox
                    .0
                    .push((Target::All, ServerMessage::EntitySpawned { snapshot }));
            }
        }
    }

    // Emit EntityDespawned for entities no longer in the ECS.
    let reported_snapshot: Vec<String> = registry.reported.iter().cloned().collect();
    for uuid in &reported_snapshot {
        if !current.contains_key(uuid) {
            registry.reported.remove(uuid);
            world.0.entities.retain(|e| e.uuid != *uuid);
            // Prune the despawned UUID from the delta caches (issue #613) —
            // runtime-spawned entities (e.g. scenario-triggered NPCs) can
            // despawn and respawn with fresh UUIDs just like asteroids.
            crate::core::broadcast::cache_registry::prune(
                &mut positions_cache,
                &mut health_cache,
                std::slice::from_ref(uuid),
            );
            outbox.0.push((
                Target::All,
                ServerMessage::EntityDespawned { uuid: uuid.clone() },
            ));
        }
    }
}

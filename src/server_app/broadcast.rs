//! Sim-state snapshot builders (issue #1199; split further from the sibling
//! `broadcast_publish` module — issue #1241, peeling the publish/HUD systems
//! out once the combined file ran 2% over the #1199 split's ~1500-line
//! target).
//!
//! Public surface: the `SimBroadcaster` factory [`sim_state_broadcaster`];
//! the per-audience snapshot builders (`build_sim_state_entity_states`, the
//! `build_station_*` / `build_control_source_snapshots` family); and
//! [`StationImportanceRes`] and its ingest/drain. All re-exported through
//! `crate::server_app`.
//!
//! Role: turns authoritative tick state into the per-audience `SimState`
//! snapshot data. The sibling [`super::broadcast_publish`] module is
//! everything downstream of that split: the other `SimBroadcaster`
//! factories, plus the publish/HUD systems that react to state rather than
//! building it. Neither module calls into the other's functions — checked
//! before splitting — so this is a real seam, not an arbitrary line cut.

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

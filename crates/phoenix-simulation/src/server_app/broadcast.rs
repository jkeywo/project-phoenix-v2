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
use crate::core::broadcast::sim::SimProducer;
use crate::core::messages::SystemId;

/// Stable lifecycle key for the `SimState` entity position/health delta pair.
pub(crate) const ENTITY_STATE_REPLICATION_KEY: &str = "entity-state";

/// Last-broadcast positions for non-asteroid entities (NPCs, stations).
///
/// Keyed by UUID string; the live [`sim_state_broadcaster`] uses this cache to
/// suppress unchanged position/yaw fields. Reset and UUID pruning deliberately
/// live here with that producer rather than in the generic lifecycle runner.
#[derive(Resource, Default)]
pub struct LastBroadcastEntityPositions(pub HashMap<String, (bevy::math::Vec3, f32)>);

/// Last-broadcast health and shield detail for every entity.
///
/// Keyed by UUID string; the tuple mirrors the four independently optional
/// health fields in `EntityStateSnapshot`. This is transport bookkeeping, not
/// authoritative simulation state.
#[derive(Resource, Default)]
pub struct LastBroadcastEntityHealth(
    pub  HashMap<
        String,
        (
            Option<f32>,
            Option<f32>,
            Option<Vec<crate::core::messages::ShieldFacingStatus>>,
            Option<f32>,
        ),
    >,
);

/// Register the entity-state producer's reset-only lifecycle behavior.
///
/// Entity positions and health need no reconnect projection because the
/// targeted `Welcome` already carries the current world snapshot. They do need
/// a run-boundary reset so the first live delta contains every field.
pub(crate) fn register_entity_state_replication_lifecycle(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::core::broadcast::{RegisterReplicationLifecycle, ReplicationLifecycleAdapter};

    app.init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<LastBroadcastEntityHealth>()
        .declare_state::<LastBroadcastEntityPositions>(
            StateClass::Cache,
            "digest-exclusion-classes",
        )
        .declare_state::<LastBroadcastEntityHealth>(StateClass::Cache, "digest-exclusion-classes")
        .register_replication_lifecycle(
            ReplicationLifecycleAdapter::new(ENTITY_STATE_REPLICATION_KEY)
                .with_reset(reset_entity_state_replication),
        );
}

fn reset_entity_state_replication(world: &mut World) {
    *world.resource_mut::<LastBroadcastEntityPositions>() = LastBroadcastEntityPositions::default();
    *world.resource_mut::<LastBroadcastEntityHealth>() = LastBroadcastEntityHealth::default();
}

/// Remove despawned UUIDs from the only replication caches keyed by entity.
///
/// Other owner-local caches are ship-, System-, or recipient-keyed and must
/// not acquire a synthetic pruning contract. Asteroid and runtime-entity
/// despawn paths call this explicit operation with their known UUIDs.
pub(crate) fn prune_entity_replication_caches(
    positions: &mut LastBroadcastEntityPositions,
    health: &mut LastBroadcastEntityHealth,
    uuids: &[String],
) {
    for uuid in uuids {
        positions.0.remove(uuid);
        health.0.remove(uuid);
    }
}

/// Returns a [`SimBroadcaster`] pre-configured with the `SimState` producer.
///
/// Broadcasts `SimState` at 10 Hz to all players (`Audience::All`).
/// Registered by [`add_simulation_plugins`] and the test harness in `test_app()`.
pub fn sim_state_broadcaster() -> SimBroadcaster {
    let broadcaster = SimBroadcaster::for_producer(SimProducer::SimState);
    broadcaster.register(Audience::All, Cadence::Hz(10.0), |world: &mut World| {
        let entity_states = build_sim_state_entity_states(world);
        let station_health = build_station_health_snapshots(world);
        let console_health = build_console_health_snapshots(world);
        let console_hosts = build_console_host_snapshots(world);
        // These two fields are removed from the reserved local packet and from
        // the unhosted All packet below. Only construct them when the existing
        // connected-recipient branch can actually publish their projection.
        let recipient_context = !console_hosts.is_empty()
            && world
                .resource::<crate::lobby::Sessions>()
                .0
                .players()
                .iter()
                .any(|player| player.connected);
        let (system_status, status_owners) = if recipient_context {
            build_system_status(world)
        } else {
            Default::default()
        };
        // Fold this tick's host state (objectives + Red Alert) into the
        // authoritative importance projection before reading it, so the snapshot
        // reflects the same tick's events (issue #1101). The drain of visits runs
        // frame-driven elsewhere; this ingest only ever raises flags on their
        // edge, so it can never resurrect a visit-cleared unread.
        ingest_station_importance(world);
        let station_importance = build_station_importance_snapshots(world);
        let console_importance = build_console_importance_snapshots(world);
        let control_sources = build_control_source_snapshots(world);
        let system_depths = build_system_depth_snapshots(world);
        let station_puppets = build_station_puppet_snapshots(world);

        // ── Emit SystemHullUpdate per recipient, only when that recipient's
        // *visible* detail changed (issue #737).
        //
        // Post issue #618 `SystemHullStatus` carries the authoritative
        // `SystemId`, display_name and tier. Post #737 the entry list is a
        // role-scoped projection instead of the whole ship, so the send is a
        // per-token fan-out rather than one `Target::All` push — see
        // `crate::console::repair::visibility`.
        crate::console::repair::visibility::push_hull_updates(world);

        let mut snapshot = crate::core::messages::SimSnapshot {
            observation_tick: world
                .get_resource::<crate::sim_tick::SimTick>()
                .map(|tick| tick.0),
            observation_generation: Some(
                world
                    .get_resource::<crate::audio_lifecycle::RoomAudioLifecycle>()
                    .map_or(0, |owner| owner.state.generation),
            ),
            system_status,
            console_views: if recipient_context {
                build_console_working_views(world)
            } else {
                Default::default()
            },
            console_hosts,
            entity_states,
            station_health,
            console_health,
            station_importance,
            console_importance,
            control_sources,
            system_depths,
            station_puppets,
        };
        if snapshot.console_hosts.is_empty() {
            snapshot.console_views.clear();
            snapshot.system_status.clear();
            return vec![ServerMessage::SimState { snapshot }];
        }
        // Working context can name records unknown to other Consoles. Project
        // it at the host boundary, not merely in the receiving GUI.
        let pending: Vec<_> = world
            .resource::<crate::lobby::Sessions>()
            .0
            .players()
            .iter()
            .filter(|player| player.connected)
            .map(|player| {
                let mut projected = snapshot.clone();
                projected.system_status.retain(|id, _| {
                    super::system_status::visible_to(
                        id,
                        &status_owners,
                        &snapshot.console_hosts,
                        player.station.as_ref(),
                    )
                });
                projected.console_views.retain(|id, _| {
                    snapshot.console_hosts.iter().any(|assignment| {
                        &assignment.console == id
                            && assignment.host.is_some()
                            && assignment.host == player.station
                    })
                });
                (
                    crate::lobby::Target::Token(player.token.clone()),
                    ServerMessage::SimState {
                        snapshot: projected,
                    },
                )
            })
            .collect();
        world.resource_mut::<SimOutbox>().extend_snapshot(pending);
        let mut local = snapshot;
        local.console_views.clear();
        local.system_status.clear();
        world.resource_mut::<SimOutbox>().push_snapshot((
            crate::lobby::Target::Token(crate::console_bridge::LOCAL_CONSOLE_TOKEN.to_string()),
            ServerMessage::SimState { snapshot: local },
        ));
        Vec::new()
    })
}

pub(crate) fn build_station_puppet_snapshots(
    world: &mut World,
) -> Vec<crate::core::messages::StationPuppetSnapshot> {
    let local_ship = {
        let mut query =
            world.query_filtered::<&crate::entities::spawner::EntityUuid, With<LocalShip>>();
        query.iter(world).next().map(|uuid| uuid.0.clone())
    };
    let Some(local_ship) = local_ship else {
        return Vec::new();
    };
    let puppets = world
        .get_resource::<crate::gm::operations::puppet::StationPuppets>()
        .cloned()
        .unwrap_or_default();
    let activity = world
        .get_resource::<crate::gm::operations::puppet::StationPuppetActivity>()
        .cloned()
        .unwrap_or_default();

    puppets
        .entries()
        .iter()
        .filter(|entry| entry.target.ship.0 == local_ship)
        .map(|entry| {
            let latest_activity = activity
                .entries()
                .iter()
                .rev()
                .find(|activity| {
                    activity.ship.0 == local_ship && activity.station == entry.target.station
                })
                .map(
                    |activity| crate::core::messages::StationPuppetActivitySnapshot {
                        tick: activity.tick,
                        operator_id: activity.operator_id.clone(),
                        target: activity.target.clone(),
                        action: activity.action.clone(),
                    },
                );
            crate::core::messages::StationPuppetSnapshot {
                station: entry.target.station.clone(),
                operators: entry.operators.clone(),
                latest_activity,
            }
        })
        .collect()
}

pub(crate) fn build_system_depth_snapshots(
    world: &mut World,
) -> BTreeMap<crate::core::messages::SystemId, crate::ship::rating::SystemDepth> {
    use crate::ship::{
        components::{ActiveStationRatings, ShipConfigComponent},
        rating,
    };
    let mut query =
        world.query_filtered::<(&ShipConfigComponent, &ActiveStationRatings), With<LocalShip>>();
    let Some((config, ratings)) = query.iter(world).next() else {
        return BTreeMap::new();
    };
    ratings
        .0
        .iter()
        .flat_map(|(station, name)| {
            rating::resolve_system_depths(
                &config.0,
                station,
                name,
                &std::collections::HashMap::new(),
            )
            .unwrap_or_default()
        })
        .collect()
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
                        crate::ship::control_source::ControlSource::Simplified => "Simplified",
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
/// owning Station via `station_fractions`. Mirrors [`build_console_host_snapshots`]:
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
/// - A latest-live Critical Comms thread is a continuing `critical` condition
///   on the logical `comms` station. Read/visit edges do not clear it; the inbox
///   lifecycle does so on response, supersession, or invalidation.
///
/// Authored-TOML importance is a deliberate future seam and is NOT built here.
pub(crate) fn ingest_station_importance(world: &mut World) {
    use crate::core::messages::SystemId;
    use crate::station_importance::AttentionOwner;

    let local_uuid = {
        let mut q =
            world.query_filtered::<&crate::entities::spawner::EntityUuid, With<LocalShip>>();
        q.iter(world)
            .next()
            .map(|u| u.0.clone())
            .unwrap_or_default()
    };
    // Objectives as (id, targets, status) for the local recipient ship.
    let legacy_objectives = world
        .get_resource::<crate::world::server::ObjectiveManagerRes>()
        .map(|m| m.0.snapshots_for(&local_uuid))
        .unwrap_or_default();
    let effective_objectives = world
        .get_resource::<crate::world::server::ObjectiveInstanceManagerRes>()
        .map(|instances| {
            instances
                .0
                .project_snapshots_for_ship(&local_uuid, legacy_objectives.clone())
        })
        .unwrap_or(legacy_objectives);
    let objectives: Vec<(String, Vec<String>, crate::core::messages::ObjectiveStatus)> =
        effective_objectives
            .into_iter()
            .map(|o| (o.id, o.targets, o.status))
            .collect();

    // Red Alert on the local ship, if it has spawned.
    let red_alert = {
        let mut q = world.query_filtered::<&crate::ship::state::ShipRedAlert, With<LocalShip>>();
        q.iter(world).next().map(|r| r.0).unwrap_or(false)
    };

    let local_ship_uuid = {
        let mut query =
            world.query_filtered::<&crate::entities::spawner::EntityUuid, With<LocalShip>>();
        query.iter(world).next().map(|uuid| uuid.0.clone())
    };
    // Retain only the attributed owners, not a copy of the ship's authored
    // topology. This immutable borrow ends before importance is updated below.
    let mut config_query =
        world.query_filtered::<&crate::ship::components::ShipConfigComponent, With<LocalShip>>();
    let config = config_query.iter(world).next().map(|config| &config.0);
    let attributed: Vec<_> = objectives
        .into_iter()
        .map(|(id, targets, status)| {
            let owner = config
                .as_ref()
                .and_then(|config| {
                    targets.iter().find_map(|target| {
                        config
                            .system(&SystemId(target.clone()))
                            .and_then(|system| system.console.clone())
                    })
                })
                .map(AttentionOwner::Console)
                .unwrap_or(AttentionOwner::Core);
            (id, owner, status)
        })
        .collect();
    let mut critical_stations = Vec::new();
    if red_alert {
        critical_stations.push(AttentionOwner::Core);
    }
    if world
        .get_resource::<crate::comms::server::CommsInboxRes>()
        .is_some_and(|inbox| {
            inbox
                .0
                .has_live_critical_thread_for(local_ship_uuid.as_deref())
        })
    {
        if let Some(config) = &config {
            critical_stations.extend(
                config
                    .systems
                    .iter()
                    .filter(|system| system.kind == "comms")
                    .filter_map(|system| system.console.clone())
                    .map(AttentionOwner::Console),
            );
        }
    }

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
    let homes = {
        let mut query = world
            .query_filtered::<&crate::ship::components::ShipConfigComponent, With<LocalShip>>();
        query
            .iter(world)
            .next()
            .map(|config| {
                config
                    .0
                    .consoles
                    .iter()
                    .filter_map(|console| {
                        console
                            .home_station
                            .clone()
                            .map(|station| (console.id.clone(), station))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    world
        .get_resource::<StationImportanceRes>()
        .map(|res| res.0.station_snapshots(&homes))
        .unwrap_or_default()
}

pub(crate) fn build_console_importance_snapshots(
    world: &mut World,
) -> Vec<phoenix_model::console::ConsoleImportanceSnapshot> {
    world
        .get_resource::<StationImportanceRes>()
        .map(|res| res.0.snapshots())
        .unwrap_or_default()
}

#[path = "console_visits.rs"]
mod console_visits;
pub(crate) use console_visits::drain_station_visited;

pub(crate) fn build_console_host_snapshots(
    world: &mut World,
) -> Vec<phoenix_model::console::ConsoleHostSnapshot> {
    let mut query = world.query_filtered::<&crate::ship_plugin::ConsoleHosts, With<LocalShip>>();
    query
        .iter(world)
        .next()
        .map(|hosts| hosts.0.clone())
        .unwrap_or_default()
}

fn build_console_working_views(
    world: &mut World,
) -> std::collections::BTreeMap<
    crate::core::messages::ConsoleId,
    phoenix_model::console::ConsoleWorkingView,
> {
    let mut query =
        world.query_filtered::<&crate::ship::components::ConsoleWorkingViews, With<LocalShip>>();
    query
        .iter(world)
        .next()
        .map(|views| views.0.clone())
        .unwrap_or_default()
}

/// Project the common live hull scalar carried by both ordinary `SimState` and
/// the rendererless GM's absolute entity lane.
pub(crate) fn project_entity_hull_fraction(
    hull: Option<&crate::entities::spawner::EntitySystemHull>,
    damage: Option<&crate::ship::compartment_damage::ShipDamage>,
) -> Option<f32> {
    crate::ship::compartment_damage::structural_fraction(damage, hull)
}

/// Project the common shield fields carried by both ordinary `SimState` and
/// the rendererless GM's absolute entity lane. Keeping this producer shared is
/// what prevents the local GM view from becoming a second sensor truth path.
pub(crate) fn project_entity_shield_state(
    shields: Option<&crate::ship::shields::ShipShields>,
) -> (
    Option<f32>,
    Option<Vec<crate::core::messages::ShieldFacingStatus>>,
    Option<f32>,
) {
    let fraction = shields.map(|shields| {
        let total_hp: i32 = shields.0.facings.iter().map(|facing| facing.hp).sum();
        let total_max: i32 = shields.0.facings.iter().map(|facing| facing.max_hp).sum();
        if total_max > 0 {
            total_hp as f32 / total_max as f32
        } else {
            0.0
        }
    });
    let facings =
        shields.map(|shields| crate::ship::shields::shield_facing_statuses(&shields.0.snapshot()));
    let frequency = shields.map(|shields| shields.frequency());
    (fraction, facings, frequency)
}

/// Compute this tick's `EntityStateSnapshot` list for the `SimState` broadcast.
///
/// Extracted from [`sim_state_broadcaster`]'s producer closure (issue #927)
/// so it can be called directly in tests without going through the
/// Broadcaster/cadence machinery — see the `sim_state_entity_states` test
/// module below, which pins the shield-detail payload population directly
/// (target with shields -> `shields`/`shield_freq` present; entity with none
/// -> absent) without needing a full multi-tick cadence fixture.
pub fn build_sim_state_entity_states(
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
            Option<&crate::ship::compartment_damage::ShipDamage>,
            Option<&crate::ship::shields::ShipShields>,
        )>();
        q.iter(world)
            .filter_map(|(uuid, hull_comp, damage, shield_comp)| {
                let hull_fraction = project_entity_hull_fraction(hull_comp, damage);
                // Per-facing detail + generator frequency (issue #927): the
                // SAME producer this ship's own `ShieldsBlackboard.facings`
                // uses (`ship::shields::shield_facing_statuses`) and the
                // same `ShipShields::frequency()`
                // `tick_frequency_hint_high_fidelity` reads for
                // `FrequencyHint` — one producer, no parallel derivation.
                // These were always sent as `None` before #927, which is
                // why `target_shields`/`target_shield_freq` were always
                // empty on the wire regardless of which console rendered them.
                let (shield_fraction, shields_wire, shield_freq) =
                    project_entity_shield_state(shield_comp);
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
            Option<&crate::ship::compartment_damage::ShipDamage>,
            Option<&crate::ship::shields::ShipShields>,
        ), Without<Asteroid>>();
        q.iter(world)
            .map(|(transform, uuid, hull_comp, damage, shield_comp)| {
                let hull_fraction = project_entity_hull_fraction(hull_comp, damage);
                // Per-facing detail + generator frequency (issue #927) —
                // same producer as the asteroid branch above; see the
                // comment there for why this closes the Sensors-panel gap.
                let (shield_fraction, shields_wire, shield_freq) =
                    project_entity_shield_state(shield_comp);
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

#[cfg(test)]
#[path = "broadcast_replication_tests.rs"]
mod replication_tests;

/// Reduce each Console's own modeled capacity without retaining a health store.
fn build_console_health_snapshots(
    world: &mut World,
) -> Vec<crate::core::messages::ConsoleHealthSnapshot> {
    let mut query = world.query_filtered::<(
        &crate::ship::components::ShipConfigComponent,
        Option<&crate::entities::spawner::EntitySystemHull>,
        Option<&crate::ship::compartment_damage::ShipDamage>,
    ), With<LocalShip>>();
    query
        .iter(world)
        .next()
        .map(|(config, hull, damage)| {
            console_health_from_parts(&config.0, hull.map(|h| &h.0), damage)
        })
        .unwrap_or_default()
}

fn console_health_from_parts(
    config: &crate::ship::config::ShipConfig,
    hull: Option<&crate::ship::damage::SystemHull>,
    damage: Option<&crate::ship::compartment_damage::ShipDamage>,
) -> Vec<crate::core::messages::ConsoleHealthSnapshot> {
    config
        .consoles
        .iter()
        .map(|console| {
            if let Some(damage) = damage {
                let factors: Vec<_> = config
                    .systems
                    .iter()
                    .filter(|system| system.console.as_ref() == Some(&console.id))
                    .map(|system| {
                        damage.factor(&system.id, phoenix_model::damage::DamageBinding::Operation)
                    })
                    .collect();
                return crate::core::messages::ConsoleHealthSnapshot {
                    console: console.id.clone(),
                    health: (!factors.is_empty())
                        .then(|| factors.iter().sum::<f32>() / factors.len() as f32),
                };
            }
            let (current, max) = config
                .systems
                .iter()
                .filter(|system| system.console.as_ref() == Some(&console.id))
                .filter_map(|system| hull.and_then(|h| h.get(&system.id)))
                .filter(|entry| entry.max > 0.0)
                .fold((0.0_f32, 0.0_f32), |(current, max), entry| {
                    (current + entry.current, max + entry.max)
                });
            crate::core::messages::ConsoleHealthSnapshot {
                console: console.id.clone(),
                health: (max > 0.0).then(|| (current / max).clamp(0.0, 1.0)),
            }
        })
        .collect()
}

fn build_system_status(
    world: &mut World,
) -> (
    std::collections::BTreeMap<SystemId, phoenix_model::system_status::SystemStatus>,
    std::collections::BTreeMap<SystemId, phoenix_model::identity::ConsoleId>,
) {
    let mut query = world.query_filtered::<(
        Entity,
        &crate::ship::components::ShipConfigComponent,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::entities::spawner::EntitySystemHull>,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        Option<&crate::dock::server::DockControl>,
        Option<&crate::ship::power::ShipPowerSystem>,
        Option<&crate::dock::server::DockMarkers>,
        Option<&crate::ship::shields::ShipShields>,
        Option<&crate::resources::fitting_server::CompiledShipFitting>,
        Option<&crate::resources::fitting_server::ShipFitting>,
        Option<&crate::console::repair::server::ShipRepairTeams>,
        Option<&crate::resources::fitting_server::FittingPlannerCatalog>,
        Option<&crate::entities::spawner::EntityTemplatePath>,
    ), With<LocalShip>>();
    let Some((
        entity,
        config,
        sources,
        hull,
        damage,
        dock,
        power,
        markers,
        shields,
        fitting_profile,
        fitting_state,
        teams,
        planner,
        hull_path,
    )) = query.iter(world).next()
    else {
        return Default::default();
    };
    let mut status = super::system_status::project(
        &config.0,
        hull.map(|h| &h.0),
        damage,
        &sources.0,
        dock,
        power,
        markers,
    );
    super::system_status::apply_shields(&mut status, shields);
    if let Some((profile, state)) = fitting_profile.zip(fitting_state) {
        crate::resources::fitting_server::project(
            profile,
            state,
            &config.0,
            &sources.0,
            teams,
            &mut status,
        );
    }
    if let Some((planner, path)) = planner.zip(hull_path) {
        let mut context = planner.0.clone();
        context.hull_path = path.0.clone();
        for status in status
            .values_mut()
            .filter(|status| !status.fitting.is_empty())
        {
            status.fitting_planner = Some(context.clone());
        }
    }
    if let Some(mut view) = crate::resources::equipment_recovery_server::project(world, entity) {
        if let Some(owner) =
            world.get::<crate::resources::equipment_recovery_server::ShipEquipmentRecovery>(entity)
        {
            let permitted = sources.0.policy_for(&owner.owner).accept_human_input;
            view.can_start &= permitted;
            view.can_cancel &= permitted;
            if let Some(status) = status.get_mut(&owner.owner) {
                status.equipment_recovery = Some(view);
            }
        }
    }
    for (owner, row) in &mut status {
        row.guidance = crate::console::weapons::guidance_ui::project(world, entity, owner);
        row.illumination = crate::console::weapons::illumination::project(world, entity, owner);
        row.jamming = crate::comms::jamming::project(world, entity, owner);
        row.signal_protection = crate::science::protection::project(world, entity, owner);
        row.identity = crate::science::identity::project(world, entity, owner);
        row.concealment = crate::science::concealment::project(world, entity, owner);
        row.bay = crate::entities::craft::project(world, entity, owner);
        // SimDispatch runs after all current paid work and before FixedLast.
        row.emission = crate::console::command::emission::advisory(world, entity, owner);
    }
    let owners = config
        .0
        .systems
        .iter()
        .filter_map(|s| Some((s.id.clone(), s.console.clone()?)))
        .collect();
    (status, owners)
}

use bevy::prelude::*;

use crate::core::messages::{
    CoordinationPayload, HelmBlackboard, HelmEngineBlackboard, HelmLateralThrustBlackboard,
    InterSystemPayload, InterSystemQueue, ModifierSlot, SystemBlackboard, SystemId,
};
use crate::server_app::{ShipBoost, ShipImpulse};
use crate::ship::components::{
    CoordinationDelivery, DeliveredCoordination, HelmWaypointClearance, PendingArcBearingRequest,
    ShipConfigComponent, ShipSystemControlSources,
};
use crate::ship::damage::DamageTier;
use crate::ship::state::ShipPhysics;
use crate::ship::system_registry::{
    helm_engine_port_system_id, helm_engine_starboard_system_id, helm_station_key,
    helm_steering_system_id, lateral_thrust_system_id,
};
use crate::ship_plugin::BoostConfigResource;

pub struct HelmPlugin;

impl Plugin for HelmPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DeliveredCoordination>()
            .add_systems(
                FixedUpdate,
                receive_helm_coordination
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(crate::ship_plugin::process_coordination_lag),
            )
            .add_systems(
                FixedUpdate,
                publish_helm_blackboard
                    .in_set(crate::sim_sets::FixedStep::PublishHelmBlackboard)
                    .in_set(crate::sim_sets::SimSet::Publish),
            );
    }
}

/// Helm-owned receiver for delayed Coordination deliveries (issue #1256).
///
/// The generic lag router owns the delay and resolves whether the addressed
/// Station is AI-operated. Once it emits [`DeliveredCoordination`], Helm owns
/// the meaning of its typed payloads: arc requests and withdrawals mutate the
/// pending facing request, while `NavigateTo` latches the waypoint generation.
///
/// The address and live steering-axis policy are deliberately re-checked at
/// consumption. Steering is the lag router's representative Helm axis when
/// Helm axes diverge, so a damaged or human-held thrust axis must not discard a
/// delivery that the AI steering axis can still act on. The live check keeps a
/// delivery from crossing a late human steering claim and preserves custom
/// hull topology without baking the `helm` Station id into the consumer. The
/// receiver runs after the router in the same `Modifiers` phase, so Helm's
/// `Physics` readers still observe the result on the following logical tick,
/// exactly as they did when the router performed these writes.
pub(crate) fn receive_helm_coordination(
    mut delivered: MessageReader<DeliveredCoordination>,
    mut ships: Query<
        (
            &ShipConfigComponent,
            &ShipSystemControlSources,
            Option<&mut PendingArcBearingRequest>,
            Option<&mut HelmWaypointClearance>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for message in delivered.read() {
        let Ok((ship_config, control_sources, mut pending_bearing, mut waypoint_clearance)) =
            ships.get_mut(message.source_entity)
        else {
            continue;
        };
        let helm_address = crate::ship::coordination::address_for_system(
            &ship_config.0,
            &helm_steering_system_id(),
        );
        let steering_operates_ai = control_sources
            .0
            .policy_for(&helm_steering_system_id())
            .operate_ai;
        if !matches!(&message.delivery, CoordinationDelivery::Ai)
            || helm_address.as_ref() != Some(&message.address)
            || !steering_operates_ai
        {
            continue;
        }

        match &message.payload {
            CoordinationPayload::ArcBearingRequest { uuid, arcs, .. } => {
                if let Some(pending) = pending_bearing.as_deref_mut() {
                    pending.target = uuid::Uuid::parse_str(uuid).ok();
                    pending.arcs = arcs.clone();
                }
            }
            CoordinationPayload::ArcBearingWithdraw { .. } => {
                if let Some(pending) = pending_bearing.as_deref_mut() {
                    pending.target = None;
                    pending.arcs.clear();
                }
            }
            CoordinationPayload::NavigateTo { generation, .. } => {
                if let Some(clearance) = waypoint_clearance.as_deref_mut() {
                    clearance.0 = Some(*generation);
                }
            }
            _ => {}
        }
    }
}

/// Publish every ship's Helm blackboard from current sim state.
/// Runs in `SimSet::Publish` (phase 1a) so downstream Broadcast systems
/// see fully-updated values. The component-change dirty-tracking is done
/// globally by `broadcast_blackboard_updates` in `SimSet::Broadcast`.
///
/// Also publishes per-engine `HelmEngine` blackboard entries (issue #511).
///
/// Per-entity for every `Ship` carrying `ShipSystemBlackboards` (issue #824),
/// following the `publish_weapons_core_blackboard` pattern (issue #697):
/// NPC helm AI reads `radar_range` from its own ship's Helm entry
/// (`ship::helm_ai::helm_ai_radar_range`), so NPCs need a live,
/// damage-scaled value rather than the static `HelmConsoleSection` fallback.
///
/// Two tiers of field, split by `Has<LocalShip>` in the loop:
///
/// - **Ship state** — position/yaw/speeds, impulse charge, boost state,
///   `radar_range` (base range × the `HelmRadarRange` modifier, which
///   `apply_radar_damage_modifiers` keeps in sync with the `helm-radar`
///   damage tier for every ship), engine and lateral entries. Computed for
///   every ship with the weapons missing-component default idiom.
/// - **Player-resource-derived data** — the base radar range for the
///   LocalShip comes from the player-only `ShipClientConfigResource`
///   (unchanged), and the engine entries' joystick fan-out is read from the
///   `InterSystemQueue` only for the LocalShip (the queue carries the player
///   joystick's channel-1 messages; an NPC has no joystick). An NPC's base
///   radar range comes from its own `HelmConsoleSection`.
///
/// None of this reaches the wire for NPCs: `broadcast_blackboard_updates`
/// is `With<LocalShip>`-filtered, so NPC blackboards add zero bandwidth.
fn publish_helm_blackboard(
    ship_client_config: Res<crate::lobby::server::ShipClientConfigResource>,
    queue: Res<InterSystemQueue>,
    // Issue #874: the hostile weapon-arc overlay. `build_world_snapshot` runs
    // under `run_if(ai_snapshot_ready)` (the derived ~10 Hz snapshot cadence,
    // `crates/phoenix-simulation/src/ai/server.rs`) while this system publishes every frame, so the
    // sectors and anchor positions read here are the MOST RECENT SNAPSHOT
    // TICK's, not this frame's: they can be up to ~100 ms stale, and the wedges
    // therefore lag the live blips slightly. Parity is unaffected — these are
    // the SAME sectors the helm AI's exposure fact is reduced from, off the same
    // snapshot, never a second computation of them.
    //
    // One asymmetry worth recording before #877 leans on "identical
    // information": the AI fact reduces over the merged `WorldView` (everything
    // in AI view range), while the overlay below is ADDITIONALLY filtered to
    // helm radar range. Same producer, so AC4 holds on the sectors themselves,
    // but a courier policy can react to a hostile whose arcs the human is never
    // shown. Deliberate for now; it is a #877 design question, not a defect.
    world_snapshot: Option<Res<crate::ai::server::WorldSnapshot>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    mut ship_q: Query<
        (
            Option<&ShipPhysics>,
            Option<&BoostConfigResource>,
            Option<&ShipImpulse>,
            Option<&ShipBoost>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::ship_plugin::LastHelmInput>,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&crate::entities::spawner::HelmConsoleSection>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
            &mut crate::server_app::ShipSystemBlackboards,
            Has<crate::server_app::LocalShip>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&crate::ship::state::ShipRedAlert>,
            (
                Option<&crate::ship::helm::ThrustInput>,
                Option<&crate::ship::helm::LateralThrustInput>,
            ),
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let default_registry = crate::ai::faction::FactionRegistry::default();
    let registry = faction_registry
        .as_deref()
        .map(|r| &r.0)
        .unwrap_or(&default_registry);

    for (
        physics,
        boost_config,
        impulse,
        boost,
        hull,
        last_input,
        modifiers,
        helm_section,
        sources,
        mut bbs,
        is_local,
        faction,
        red_alert,
        (thrust_input, lateral_input),
    ) in ship_q.iter_mut()
    {
        // Per-entity component path. Each fallback mirrors the pre-#824
        // `.single()` error arm, so a ship (or test fixture) missing a
        // component publishes exactly what it published before.
        let physics = physics.copied().unwrap_or_default();
        let boost_enabled = boost_config.map(|c| c.enabled).unwrap_or(false);
        let impulse_charge = impulse.map(|i| i.0.charge_progress).unwrap_or(0.0);
        let boost_state = boost.map(|b| b.0);
        let boost_battery = boost_state.as_ref().map(|b| b.battery).unwrap_or(0.0);
        let boost_active = boost_state.as_ref().map(|b| b.is_active()).unwrap_or(false);
        // view_mode is not raw sim truth; helm blackboard omits it

        // Live helm radar range: base config range scaled by the dedicated
        // `HelmRadarRange` modifier, which `apply_radar_damage_modifiers`
        // keeps in sync with the `helm-radar` system's damage tier each tick
        // — for every ship, not just the player.
        //
        // The base range is the ship's OWN authored `[helm_console.radar]
        // range`, whoever is looking at it (issue #1116). It used to be the
        // player-only client config for the `LocalShip` and the section for
        // everything else — which is the same number spelled twice, since
        // `lobby::server` builds that resource's `helm_radar_range` from the
        // very same `hc.effective_radar_range()`. The two spellings stopped
        // being interchangeable the moment there was a second host: each tags a
        // DIFFERENT ship `LocalShip`, so each would hand its own hull the
        // client-config number and the peer's hull the section number — and the
        // helm AI reads this field (`ship::helm_ai::helm_ai_radar_range`), so
        // the two hosts ran one ship's AI on two different sensor horizons.
        //
        // The client config stays as the fallback for a `LocalShip` whose hull
        // authors no `[helm_console.radar]` at all, which is the bare-fixture
        // shape and the only case where the two ever disagreed.
        let radar_mult = modifiers
            .map(|m| m.get(&ModifierSlot::HelmRadarRange))
            .unwrap_or(1.0);
        let base_radar_range = helm_section
            .map(|hc| hc.0.effective_radar_range())
            .filter(|range| *range > 0.0)
            .unwrap_or(if is_local {
                ship_client_config.0.helm_radar_range
            } else {
                0.0
            });
        let radar_range = base_radar_range * radar_mult;

        // ── Hostile weapon arcs (issue #874) ────────────────────────────────
        //
        // Two gates, both here on the server rather than on the client:
        //
        // - LOCAL SHIP ONLY, like `TacticalRadarBlackboard::blips`. An NPC
        //   renders no radar, so it would be pure bandwidth.
        // - RED ALERT ONLY. Gating client-side would still put the intel on the
        //   wire; gating here means a helm not at red alert is never sent it.
        //
        // The sectors are copied verbatim off the world snapshot — the SAME
        // producer output `crate::ai::hostile_arc_exposure` reduces into the
        // helm AI's facts. Nothing here recomputes an arc.
        let at_red_alert = red_alert.map(|r| r.0).unwrap_or(false);
        let hostile_weapon_arcs = if is_local && at_red_alert {
            let self_faction = faction.map(|f| f.0);
            world_snapshot
                .as_deref()
                .map(|snap| {
                    snap.entities
                        .iter()
                        .filter(|e| !e.weapon_arcs.is_empty())
                        .filter(|e| {
                            e.faction
                                .map(|ef| {
                                    crate::ai::faction::is_enemy(self_faction, Some(ef), registry)
                                })
                                .unwrap_or(false)
                        })
                        // Only contacts the helm radar is actually showing: an
                        // overlay anchored off the edge of the scope is noise.
                        .filter(|e| {
                            let dx = e.position[0] - physics.x;
                            let dz = e.position[2] - physics.z;
                            dx * dx + dz * dz <= radar_range * radar_range
                        })
                        .map(|e| crate::core::messages::HostileWeaponArcContact {
                            uuid: e.uuid.to_string(),
                            x: e.position[0],
                            z: e.position[2],
                            arcs: e.weapon_arcs.iter().map(Into::into).collect(),
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let bb = HelmBlackboard {
            yaw: physics.yaw,
            forward_speed: physics.forward_speed,
            x: physics.x,
            z: physics.z,
            impulse_charge,
            boost_battery,
            boost_active,
            boost_enabled,
            radar_range,
            lateral_speed: physics.lateral_speed,
            hostile_weapon_arcs,
        };

        // Read last helm input for engine thrust fraction.
        let mut last_input = last_input.copied().unwrap_or_default();
        if !is_local {
            // NPC interfaces read the same per-axis actuator intent that the
            // ordinary consumer writes. LastHelmInput is the player's HUD
            // mirror; it is not updated by NPC commands and would show zero
            // throughout a real GM takeover.
            if let Some(input) = thrust_input {
                last_input.thrust = input.0;
            }
            if let Some(input) = lateral_input {
                last_input.lateral = input.0;
            }
        }

        // Per-engine blackboard (issue #511): one entry per fine engine system.
        let engine_entries = [
            (
                helm_engine_port_system_id(),
                SystemId("helm-engine-port".into()),
            ),
            (
                helm_engine_starboard_system_id(),
                SystemId("helm-engine-starboard".into()),
            ),
        ];

        // Console-level blackboard: keyed by the Helm STATION id (issue #801).
        // The wire string is unchanged — the client still reads
        // `blackboards['helm']` — but the key names the console, not a system.
        bbs.0.insert(helm_station_key(), SystemBlackboard::Helm(bb));

        // Publish per-engine entries.
        for (system_id, engine_sid) in engine_entries {
            let tier = hull
                .map(|h| h.0.tier_for(&engine_sid))
                .unwrap_or(DamageTier::Operational);
            let is_online = !matches!(tier, DamageTier::Disabled | DamageTier::Destroyed);
            // Prefer the JoystickState from the InterSystemQueue (written by
            // `publish_joystick_to_engines` in SimSet::Physics, which runs
            // before SimSet::Publish). LocalShip only: the queue's engine
            // messages are the player joystick's fan-out, keyed by target
            // system id, and must not bleed into NPC entries. Fall back to
            // this ship's LastHelmInput otherwise.
            let last_input_thrust = last_input.thrust;
            let joystick_thrust = if is_local {
                queue
                    .0
                    .iter()
                    .filter(|m| m.target == system_id)
                    .filter_map(|m| {
                        if let InterSystemPayload::JoystickState { thrust, .. } = &m.payload {
                            Some(*thrust)
                        } else {
                            None
                        }
                    })
                    .next_back()
                    .unwrap_or(last_input_thrust)
            } else {
                last_input_thrust
            };
            let thrust_fraction = if is_online {
                joystick_thrust.abs()
            } else {
                0.0
            };
            bbs.0.insert(
                system_id,
                SystemBlackboard::HelmEngine(HelmEngineBlackboard {
                    thrust_fraction,
                    is_online,
                }),
            );
        }

        // ── Lateral thrust blackboard ───────────────────────────────────────
        let lt_sid = lateral_thrust_system_id();
        let lt_tier = hull
            .map(|h| h.0.tier_for(&SystemId(lt_sid.0.clone())))
            .unwrap_or(DamageTier::Operational);
        let lt_is_online = !matches!(lt_tier, DamageTier::Disabled | DamageTier::Destroyed);
        let lt_auto = sources
            .map(|s| s.0.policy_for(&lt_sid).operate_ai)
            .unwrap_or(false);
        bbs.0.insert(
            lt_sid,
            SystemBlackboard::HelmLateralThrust(HelmLateralThrustBlackboard {
                lateral_input: last_input.lateral,
                is_online: lt_is_online,
                auto: lt_auto,
            }),
        );
    }
}

#[cfg(test)]
// Fixture ids only (issue #907): these tests need distinct opaque targets, not
// reproducible production identity. Production ids are minted by `world_id`.
#[allow(clippy::disallowed_methods)]
#[path = "server_tests.rs"]
mod tests;

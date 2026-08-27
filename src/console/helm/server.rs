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
                publish_helm_blackboard.in_set(crate::sim_sets::SimSet::Publish),
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
    // `src/ai/server.rs`) while this system publishes every frame, so the
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
        // — for every ship, not just the player. The base range is the
        // player-only client config for the LocalShip (unchanged) and the
        // ship's own authored `[helm_console.radar] range` for an NPC.
        let radar_mult = modifiers
            .map(|m| m.get(&ModifierSlot::HelmRadarRange))
            .unwrap_or(1.0);
        let base_radar_range = if is_local {
            ship_client_config.0.helm_radar_range
        } else {
            helm_section
                .map(|hc| hc.0.effective_radar_range())
                .unwrap_or(0.0)
        };
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
        let last_input = last_input.copied().unwrap_or_default();

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
mod tests {
    use super::*;
    use crate::core::messages::{
        CoordinationAddress, CoordinationPayload, SystemBlackboard, WeaponEmitterArc, WeaponFamily,
    };
    use crate::server_app::ShipSystemBlackboards;
    use crate::ship::boost::BoostState;

    fn base_app() -> App {
        let mut app = App::new();
        app.add_systems(Update, publish_helm_blackboard);
        // Initialise InterSystemQueue so the system parameter is satisfied.
        app.init_resource::<InterSystemQueue>();
        app.insert_resource(crate::lobby::server::ShipClientConfigResource::default());
        // Spawn a LocalShip entity with components so the system can query it.
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            ShipPhysics::default(),
            ShipSystemBlackboards::default(),
            ShipImpulse::default(),
            ShipBoost::default(),
            crate::modifiers::ShipModifiers::new(),
            crate::ship_plugin::LastHelmInput::default(),
        ));
        app
    }

    fn helm_coordination_app() -> (App, Entity, CoordinationAddress) {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<InterSystemQueue>()
            .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
            .add_plugins(HelmPlugin);
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            crate::ship::test_support::TEST_TICK,
        );

        let config = ShipConfigComponent::default();
        let address =
            crate::ship::coordination::address_for_system(&config.0, &helm_steering_system_id())
                .expect("shipped test hull assigns Helm steering to a Station");
        let ship = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                config,
                ShipSystemControlSources::default(),
                PendingArcBearingRequest::default(),
                HelmWaypointClearance::default(),
            ))
            .id();
        crate::ship::test_support::set_helm_control_source(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
        );
        (app, ship, address)
    }

    fn deliver_to_helm(
        app: &mut App,
        ship: Entity,
        address: CoordinationAddress,
        payload: CoordinationPayload,
    ) {
        app.world_mut()
            .resource_mut::<Messages<DeliveredCoordination>>()
            .write(DeliveredCoordination {
                source_entity: ship,
                address,
                payload,
                presentation: crate::core::messages::CoordinationPresentation::new(
                    "test.coordination.title",
                    "test.coordination.body",
                ),
                delivery: CoordinationDelivery::Ai,
            });
    }

    #[test]
    fn helm_coordination_receiver_preserves_payload_values_and_delivery_order() {
        let (mut app, ship, address) = helm_coordination_app();
        let target = uuid::Uuid::new_v4();
        let arcs = vec![WeaponEmitterArc {
            facing_deg: 37.0,
            arc_deg: 83.0,
            range: 412.5,
        }];

        deliver_to_helm(
            &mut app,
            ship,
            address.clone(),
            CoordinationPayload::ArcBearingRequest {
                uuid: target.to_string(),
                label: "test target".into(),
                family: WeaponFamily::Blasters,
                arcs: arcs.clone(),
            },
        );
        deliver_to_helm(
            &mut app,
            ship,
            address.clone(),
            CoordinationPayload::ArcBearingWithdraw {
                family: WeaponFamily::Blasters,
            },
        );
        crate::ship::test_support::tick(&mut app);

        let pending = app
            .world()
            .entity(ship)
            .get::<PendingArcBearingRequest>()
            .expect("test ship carries pending arc state");
        assert_eq!(pending.target, None, "later withdrawal wins in bus order");
        assert!(
            pending.arcs.is_empty(),
            "withdrawal clears carried geometry"
        );

        deliver_to_helm(
            &mut app,
            ship,
            address.clone(),
            CoordinationPayload::NavigateTo {
                generation: 73,
                x: 900.0,
                z: -450.0,
            },
        );
        deliver_to_helm(
            &mut app,
            ship,
            address,
            CoordinationPayload::ArcBearingRequest {
                uuid: target.to_string(),
                label: "test target".into(),
                family: WeaponFamily::Blasters,
                arcs: arcs.clone(),
            },
        );
        crate::ship::test_support::tick(&mut app);

        let pending = app
            .world()
            .entity(ship)
            .get::<PendingArcBearingRequest>()
            .unwrap();
        assert_eq!(pending.target, Some(target));
        assert_eq!(
            pending.arcs, arcs,
            "arc geometry is copied without reduction"
        );
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<HelmWaypointClearance>()
                .expect("test ship carries waypoint clearance")
                .0,
            Some(73),
            "NavigateTo latches only its exact generation"
        );
    }

    #[test]
    fn helm_coordination_receiver_rechecks_address_and_live_ai_ownership() {
        let (mut app, ship, address) = helm_coordination_app();
        let payload = CoordinationPayload::ArcBearingRequest {
            uuid: uuid::Uuid::new_v4().to_string(),
            label: "test target".into(),
            family: WeaponFamily::Phasers,
            arcs: vec![WeaponEmitterArc {
                facing_deg: 0.0,
                arc_deg: 90.0,
                range: 300.0,
            }],
        };

        crate::ship::test_support::set_helm_control_source(
            &mut app,
            crate::ship::control_source::ControlSource::Human,
        );
        deliver_to_helm(&mut app, ship, address.clone(), payload.clone());
        crate::ship::test_support::tick(&mut app);
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<PendingArcBearingRequest>()
                .unwrap()
                .target,
            None,
            "a late human claim invalidates an already-emitted AI delivery"
        );

        crate::ship::test_support::set_helm_control_source(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
        );

        app.world_mut()
            .resource_mut::<Messages<DeliveredCoordination>>()
            .write(DeliveredCoordination {
                source_entity: ship,
                address: address.clone(),
                payload: payload.clone(),
                presentation: crate::core::messages::CoordinationPresentation::new(
                    "test.coordination.title",
                    "test.coordination.body",
                ),
                delivery: CoordinationDelivery::HumanPopup {
                    token: "test-token".into(),
                    sender_label: "test-sender".into(),
                },
            });
        crate::ship::test_support::tick(&mut app);
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<PendingArcBearingRequest>()
                .unwrap()
                .target,
            None,
            "Helm's AI receiver must reject a human-popup delivery"
        );

        deliver_to_helm(&mut app, ship, CoordinationAddress::Ship, payload);
        crate::ship::test_support::tick(&mut app);
        assert_eq!(
            app.world()
                .entity(ship)
                .get::<PendingArcBearingRequest>()
                .unwrap()
                .target,
            None,
            "a Ship broadcast is not a Helm Station delivery"
        );
    }

    #[test]
    fn helm_coordination_receiver_uses_ai_steering_when_thrust_is_offline() {
        let (mut app, ship, address) = helm_coordination_app();
        let target = uuid::Uuid::new_v4();
        let arcs = vec![WeaponEmitterArc {
            facing_deg: -42.0,
            arc_deg: 67.0,
            range: 318.0,
        }];

        let mut ship_entity = app.world_mut().entity_mut(ship);
        let mut control_sources = ship_entity
            .get_mut::<ShipSystemControlSources>()
            .expect("test ship carries control sources");
        control_sources
            .0
            .set_offline(crate::ship::system_registry::helm_thrust_system_id(), true);
        assert!(
            control_sources
                .0
                .policy_for(&helm_steering_system_id())
                .operate_ai,
            "steering remains AI-operated"
        );
        assert!(
            !crate::ship_plugin::helm_axes_operate_ai(&control_sources),
            "offline thrust makes the old composite receiver gate false"
        );
        drop(control_sources);

        deliver_to_helm(
            &mut app,
            ship,
            address.clone(),
            CoordinationPayload::ArcBearingRequest {
                uuid: target.to_string(),
                label: "test target".into(),
                family: WeaponFamily::Phasers,
                arcs: arcs.clone(),
            },
        );
        deliver_to_helm(
            &mut app,
            ship,
            address.clone(),
            CoordinationPayload::NavigateTo {
                generation: 91,
                x: -25.0,
                z: 640.0,
            },
        );
        crate::ship::test_support::tick(&mut app);

        let ship_state = app.world().entity(ship);
        let pending = ship_state
            .get::<PendingArcBearingRequest>()
            .expect("test ship carries pending arc state");
        assert_eq!(pending.target, Some(target));
        assert_eq!(pending.arcs, arcs);
        assert_eq!(
            ship_state
                .get::<HelmWaypointClearance>()
                .expect("test ship carries waypoint clearance")
                .0,
            Some(91),
            "offline thrust does not swallow steering-owned clearance"
        );

        deliver_to_helm(
            &mut app,
            ship,
            address,
            CoordinationPayload::ArcBearingWithdraw {
                family: WeaponFamily::Blasters,
            },
        );
        crate::ship::test_support::tick(&mut app);

        let pending = app
            .world()
            .entity(ship)
            .get::<PendingArcBearingRequest>()
            .unwrap();
        assert_eq!(pending.target, None);
        assert!(
            pending.arcs.is_empty(),
            "withdrawal remains unconditional across weapon families"
        );
    }

    /// Spawn an NPC ship (no `LocalShip`) carrying the components the
    /// entity spawner gives every behaviour-bearing NPC, plus an authored
    /// helm radar range. Returns its entity id.
    fn spawn_npc_ship(app: &mut App, radar_range: f32) -> Entity {
        let toml_str = format!(
            "[helm_console]\nmax_speed = 30.0\n\n[helm_console.radar]\nrange = {radar_range}\nshows = [\"ship\"]\n"
        );
        let helm_config = crate::entities::config::EntityConfig::from_toml(&toml_str)
            .expect("helm_console TOML must parse")
            .helm_console
            .expect("helm_console section must be present");
        app.world_mut()
            .spawn((
                crate::server_app::Ship,
                ShipPhysics {
                    x: 42.0,
                    z: -17.0,
                    ..Default::default()
                },
                ShipSystemBlackboards::default(),
                crate::modifiers::ShipModifiers::new(),
                crate::entities::spawner::HelmConsoleSection(helm_config),
            ))
            .id()
    }

    /// Helper: read the helm blackboard from the LocalShip entity's ShipSystemBlackboards component.
    fn get_helm_blackboard(app: &mut App) -> crate::core::messages::HelmBlackboard {
        let key = helm_station_key();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::Helm(bb) = bbs
            .0
            .get(&key)
            .expect("expected helm entry in blackboards")
            .clone()
        else {
            panic!("expected Helm blackboard")
        };
        bb
    }

    // ── Hostile weapon-arc overlay (issue #874) ───────────────────────────

    const OWN_FACTION: uuid::Uuid = uuid::Uuid::from_u128(0x0874_0001);
    const ENEMY_FACTION: uuid::Uuid = uuid::Uuid::from_u128(0x0874_0002);

    /// A snapshot carrying one armed hostile 100 units off the bow, plus a
    /// faction registry that makes it an enemy.
    fn arc_overlay_app(red_alert: bool, local: bool) -> App {
        let mut app = App::new();
        app.add_systems(Update, publish_helm_blackboard);
        app.init_resource::<InterSystemQueue>();
        app.insert_resource(crate::lobby::server::ShipClientConfigResource::default());

        let mut registry = crate::ai::faction::FactionRegistry::new();
        registry.insert(crate::ai::faction::FactionConfig {
            display_name: None,
            uuid: OWN_FACTION,
            name: "Own".into(),
            enemies: vec![ENEMY_FACTION],
            compliance: None,
        });
        registry.insert(crate::ai::faction::FactionConfig {
            display_name: None,
            uuid: ENEMY_FACTION,
            name: "Enemy".into(),
            enemies: vec![OWN_FACTION],
            compliance: None,
        });
        app.insert_resource(crate::entities::config_cache::FactionRegistryResource(
            registry,
        ));

        app.insert_resource(crate::ai::server::WorldSnapshot {
            entities: vec![
                crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::from_u128(0x0874_1111),
                    position: [0.0, 0.0, -100.0],
                    faction: Some(ENEMY_FACTION),
                    weapon_arcs: crate::weapons::arc_geometry::weapon_arc_sectors(
                        0.0,
                        &[crate::weapons::arc_geometry::WeaponArcBank {
                            facing_deg: 180.0,
                            fire_arc_deg: 90.0,
                            range: 400.0,
                        }],
                    ),
                    ..Default::default()
                },
                // A friendly ship with arcs of its own — must never appear.
                crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::from_u128(0x0874_2222),
                    position: [50.0, 0.0, 0.0],
                    faction: Some(OWN_FACTION),
                    weapon_arcs: crate::weapons::arc_geometry::weapon_arc_sectors(
                        0.0,
                        &[crate::weapons::arc_geometry::WeaponArcBank {
                            facing_deg: 0.0,
                            fire_arc_deg: 60.0,
                            range: 400.0,
                        }],
                    ),
                    ..Default::default()
                },
                // A hostile far outside the helm radar horizon.
                crate::ai::AiWorldEntity {
                    uuid: uuid::Uuid::from_u128(0x0874_3333),
                    position: [0.0, 0.0, -9000.0],
                    faction: Some(ENEMY_FACTION),
                    weapon_arcs: crate::weapons::arc_geometry::weapon_arc_sectors(
                        0.0,
                        &[crate::weapons::arc_geometry::WeaponArcBank {
                            facing_deg: 180.0,
                            fire_arc_deg: 90.0,
                            range: 400.0,
                        }],
                    ),
                    ..Default::default()
                },
            ],
        });

        let mut ship = app.world_mut().spawn((
            crate::server_app::Ship,
            ShipPhysics::default(),
            ShipSystemBlackboards::default(),
            ShipImpulse::default(),
            ShipBoost::default(),
            crate::modifiers::ShipModifiers::new(),
            crate::ship_plugin::LastHelmInput::default(),
            crate::entities::spawner::FactionComponent(OWN_FACTION),
            crate::ship::state::ShipRedAlert(red_alert),
        ));
        if local {
            ship.insert(crate::server_app::LocalShip);
        }
        app
    }

    fn helm_bb_only(app: &mut App) -> crate::core::messages::HelmBlackboard {
        let key = helm_station_key();
        let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::Helm(bb) = bbs.0.get(&key).expect("helm entry").clone() else {
            panic!("expected Helm blackboard")
        };
        bb
    }

    /// AC3: at red alert the local helm gets the hostile's sectors — and only
    /// the hostile's, and only the ones on the scope.
    #[test]
    fn red_alert_publishes_in_range_hostile_arcs_only() {
        let mut app = arc_overlay_app(true, true);
        app.update();
        let bb = helm_bb_only(&mut app);
        assert_eq!(
            bb.hostile_weapon_arcs.len(),
            1,
            "the friendly and the over-the-horizon hostile must not appear: {:?}",
            bb.hostile_weapon_arcs
        );
        let contact = &bb.hostile_weapon_arcs[0];
        assert_eq!(contact.uuid, uuid::Uuid::from_u128(0x0874_1111).to_string());
        assert!((contact.x - 0.0).abs() < 1e-3);
        assert!((contact.z + 100.0).abs() < 1e-3);
        assert_eq!(contact.arcs.len(), 1);
        assert!((contact.arcs[0].bearing_deg - 180.0).abs() < 1e-3);
        assert!((contact.arcs[0].half_angle_deg - 45.0).abs() < 1e-3);
        assert!((contact.arcs[0].range - 400.0).abs() < 1e-3);
    }

    /// AC3, the other half: no red alert, no arcs — and the gate is server
    /// side, so the intel never reaches the wire at all.
    #[test]
    fn without_red_alert_no_hostile_arcs_are_published() {
        let mut app = arc_overlay_app(false, true);
        app.update();
        assert!(
            helm_bb_only(&mut app).hostile_weapon_arcs.is_empty(),
            "arcs must be red-alert gated"
        );
    }

    /// An NPC renders no radar, so it pays no bandwidth for one — the same
    /// posture `TacticalRadarBlackboard::blips` takes.
    #[test]
    fn a_non_local_ship_publishes_no_hostile_arcs_even_at_red_alert() {
        let mut app = arc_overlay_app(true, false);
        app.update();
        assert!(helm_bb_only(&mut app).hostile_weapon_arcs.is_empty());
    }

    // ── Publish tests ──────────────────────────────────────────────────────

    #[test]
    fn publish_writes_helm_entry_to_blackboards() {
        let mut app = base_app();
        app.update();

        let key = helm_station_key();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        assert!(
            bbs.0.contains_key(&key),
            "expected helm entry in blackboards"
        );
    }

    #[test]
    fn publish_reflects_ship_position_and_yaw() {
        let mut app = base_app();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
            let mut physics = q.single_mut(app.world_mut()).unwrap();
            physics.x = 100.0;
            physics.z = -200.0;
            physics.yaw = std::f32::consts::FRAC_PI_4;
            physics.forward_speed = 50.0;
        }
        app.update();

        let bb = get_helm_blackboard(&mut app);
        assert!((bb.x - 100.0).abs() < 0.001);
        assert!((bb.z - (-200.0)).abs() < 0.001);
        assert!((bb.forward_speed - 50.0).abs() < 0.001);
        assert!((bb.yaw - std::f32::consts::FRAC_PI_4).abs() < 0.001);
    }

    #[test]
    fn publish_reflects_impulse_charge() {
        let mut app = base_app();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipImpulse, With<crate::server_app::LocalShip>>();
            let mut imp = q
                .single_mut(app.world_mut())
                .expect("LocalShip must have ShipImpulse");
            imp.0.charge_progress = 0.5;
        }
        app.update();

        let bb = get_helm_blackboard(&mut app);
        assert!((bb.impulse_charge - 0.5).abs() < 0.001);
    }

    #[test]
    fn publish_reflects_boost_state() {
        let mut app = base_app();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            let ship = q.single_mut(app.world_mut()).unwrap();
            app.world_mut()
                .entity_mut(ship)
                .insert(BoostConfigResource {
                    enabled: true,
                    multiplier: 3.0,
                    steering_multiplier: 2.0,
                    active_duration: 4.0,
                    recharge_duration: 20.0,
                });
        }
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipBoost, With<crate::server_app::LocalShip>>();
            let mut boost = q
                .single_mut(app.world_mut())
                .expect("LocalShip must have ShipBoost");
            boost.0 = BoostState {
                active: true,
                battery: 0.75,
            };
        }
        app.update();

        let bb = get_helm_blackboard(&mut app);
        assert!(bb.boost_enabled);
        assert!(bb.boost_active);
        assert!((bb.boost_battery - 0.75).abs() < 0.001);
    }

    #[test]
    fn publish_boost_disabled_when_no_config() {
        let mut app = base_app();
        app.update();

        let bb = get_helm_blackboard(&mut app);
        assert!(!bb.boost_enabled);
        assert!(!bb.boost_active);
    }

    // ── Per-engine blackboard tests (issue #511) ───────────────────────────

    #[test]
    fn publish_writes_engine_port_entry_to_blackboards() {
        let mut app = base_app();
        app.update();

        let key = helm_engine_port_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        assert!(
            bbs.0.contains_key(&key),
            "expected helm-engine-port in blackboards"
        );
    }

    #[test]
    fn publish_writes_engine_starboard_entry_to_blackboards() {
        let mut app = base_app();
        app.update();

        let key = helm_engine_starboard_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        assert!(
            bbs.0.contains_key(&key),
            "expected helm-engine-starboard in blackboards"
        );
    }

    #[test]
    fn engine_is_online_when_no_hull_damage() {
        let mut app = base_app();
        app.update();

        let key = helm_engine_port_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::HelmEngine(engine_bb) = bbs
            .0
            .get(&key)
            .expect("expected helm-engine-port in blackboards")
            .clone()
        else {
            panic!("expected HelmEngine blackboard");
        };
        assert!(
            engine_bb.is_online,
            "engine should be online when no hull damage"
        );
    }

    #[test]
    fn engine_thrust_fraction_reflects_last_input() {
        let mut app = base_app();
        // Set helm input to 0.8 thrust.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
                .single(app.world())
                .unwrap();
            app.world_mut()
                .entity_mut(ship)
                .insert(crate::ship_plugin::LastHelmInput {
                    thrust: 0.8,
                    steering: 0.0,
                    lateral: 0.0,
                });
        }
        app.update();

        let key = helm_engine_port_system_id();
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
        let SystemBlackboard::HelmEngine(engine_bb) = bbs
            .0
            .get(&key)
            .expect("expected helm-engine-port in blackboards")
            .clone()
        else {
            panic!("expected HelmEngine blackboard");
        };
        assert!(
            (engine_bb.thrust_fraction - 0.8).abs() < 0.001,
            "thrust_fraction should match last helm input"
        );
    }

    // ── Per-entity publish tests (issue #824) ──────────────────────────────

    fn helm_bb_of(app: &mut App, entity: Entity) -> crate::core::messages::HelmBlackboard {
        let bbs = app
            .world()
            .entity(entity)
            .get::<ShipSystemBlackboards>()
            .expect("ship must carry ShipSystemBlackboards");
        let SystemBlackboard::Helm(bb) = bbs
            .0
            .get(&helm_station_key())
            .expect("expected helm entry in blackboards")
            .clone()
        else {
            panic!("expected Helm blackboard")
        };
        bb
    }

    /// AC (issue #824): an NPC ship gets a Helm blackboard entry of its own,
    /// with ship-state fields derived from its own components.
    #[test]
    fn publish_writes_helm_entry_for_npc_ship() {
        let mut app = base_app();
        let npc = spawn_npc_ship(&mut app, 750.0);
        app.update();

        let bb = helm_bb_of(&mut app, npc);
        assert!((bb.x - 42.0).abs() < 0.001, "NPC x must be its own physics");
        assert!(
            (bb.z - (-17.0)).abs() < 0.001,
            "NPC z must be its own physics"
        );
        assert!(
            (bb.radar_range - 750.0).abs() < 0.001,
            "NPC radar_range must come from its own HelmConsoleSection, got {}",
            bb.radar_range
        );
    }

    /// AC (issue #824): the NPC's `radar_range` is live — scaled by the
    /// `HelmRadarRange` modifier `apply_radar_damage_modifiers` maintains —
    /// not the static config fallback.
    #[test]
    fn npc_radar_range_is_scaled_by_the_damage_modifier() {
        let mut app = base_app();
        let npc = spawn_npc_ship(&mut app, 800.0);
        {
            let mut entity = app.world_mut().entity_mut(npc);
            let mut modifiers = entity.get_mut::<crate::modifiers::ShipModifiers>().unwrap();
            // The same shape `apply_radar_damage_modifiers` writes for a
            // damaged helm-radar: a -0.5 bonus is a 0.5 multiplier.
            modifiers.add_or_update(crate::modifiers::Modifier {
                source: crate::modifiers::cache::ModifierSource::SystemDamage(
                    crate::ship::system_registry::helm_radar_system_id(),
                ),
                slot: ModifierSlot::HelmRadarRange,
                bonus: -0.5,
            });
        }
        app.update();

        // Whatever multiplier the cache computes for a -0.5 bonus, the
        // published range must be the base range scaled by it — and it must
        // actually be a reduction, or the modifier did nothing.
        let mult = app
            .world()
            .entity(npc)
            .get::<crate::modifiers::ShipModifiers>()
            .unwrap()
            .get(&ModifierSlot::HelmRadarRange);
        assert!(
            mult < 0.999,
            "precondition: the damage modifier must reduce the multiplier, got {mult}"
        );
        let bb = helm_bb_of(&mut app, npc);
        assert!(
            (bb.radar_range - 800.0 * mult).abs() < 0.01,
            "NPC radar_range must be damage-scaled (800 * {mult}), got {}",
            bb.radar_range
        );
    }

    /// The is_local gating: the LocalShip's base radar range still comes from
    /// the player-only `ShipClientConfigResource`, never from a
    /// `HelmConsoleSection`, and both tiers publish in the same tick.
    #[test]
    fn local_ship_radar_range_still_comes_from_client_config() {
        let mut app = base_app();
        app.insert_resource(crate::lobby::server::ShipClientConfigResource(
            crate::core::messages::ShipClientConfig {
                helm_radar_range: 123.0,
                ..Default::default()
            },
        ));
        let npc = spawn_npc_ship(&mut app, 750.0);
        app.update();

        let local = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        let local_bb = helm_bb_of(&mut app, local);
        assert!(
            (local_bb.radar_range - 123.0).abs() < 0.001,
            "LocalShip radar_range must come from ShipClientConfigResource, got {}",
            local_bb.radar_range
        );
        let npc_bb = helm_bb_of(&mut app, npc);
        assert!(
            (npc_bb.radar_range - 750.0).abs() < 0.001,
            "NPC radar_range must ignore the player-only client config, got {}",
            npc_bb.radar_range
        );
    }

    /// NPC ships get engine + lateral entries too (ship-state tier), derived
    /// from their own components rather than the player's joystick queue.
    #[test]
    fn publish_writes_engine_and_lateral_entries_for_npc_ship() {
        let mut app = base_app();
        let npc = spawn_npc_ship(&mut app, 750.0);
        app.update();

        let bbs = app
            .world()
            .entity(npc)
            .get::<ShipSystemBlackboards>()
            .unwrap();
        assert!(
            bbs.0.contains_key(&helm_engine_port_system_id()),
            "expected NPC helm-engine-port entry"
        );
        assert!(
            bbs.0.contains_key(&helm_engine_starboard_system_id()),
            "expected NPC helm-engine-starboard entry"
        );
        assert!(
            bbs.0.contains_key(&lateral_thrust_system_id()),
            "expected NPC helm-lateral-thrust entry"
        );
    }
}

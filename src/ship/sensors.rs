use bevy::prelude::*;

use crate::messages::{
    CoordinationPayload, ModifierSlot, SensorsBlackboard, SystemBlackboard, SystemControlPayload,
    SystemId,
};
use crate::ship_plugin::CoordinationEnqueue;

// ── Resources ──────────────────────────────────────────────────────────────────

/// The currently selected science target on the Sensors console. `None` means
/// no target is selected. Broadcast to all clients via SensorsBlackboard so
/// every radar can render a blue science-target marker.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own sensors target.
#[derive(Component, Default, Clone, Debug)]
pub struct SensorsTarget(pub Option<String>);

/// Tracks the last frequency value sent for a given target so we avoid
/// re-emitting when nothing has changed.
///
/// Per-ship `Component` so NPC ships track their own Sensors→Tactical
/// frequency hints independently of the player's.
#[derive(Component, Default, Clone)]
pub struct SensorsFrequencyState {
    pub last_sent_target: Option<String>,
    pub last_sent_frequency: Option<f32>,
}

/// Tracks the last threat warning emitted per ship to debounce against
/// bus spam (issue #683). Sensors emits a `ThreatBearing` coordination
/// message to Shields only when a *new* threat appears or an existing
/// threat's bearing changes by more than the configured epsilon.
#[derive(Component, Default, Clone)]
pub struct SensorsThreatState {
    pub last_threat_uuid: Option<String>,
    pub last_bearing_rad: Option<f32>,
    pub last_label: Option<String>,
    pub last_distance: Option<f32>,
}

/// TOML-loaded configuration for the Sensors AI controller
/// (`console_ai::server::ai_frequency_hint`, issue #692).
///
/// Loaded from `[sensors_console.ai]` in the ship entity TOML. Defaults are
/// used when the section is absent.
///
/// Dual `Resource + Component`, mirroring `ShieldsAiConfigResource`:
/// production reads use the Resource form (ship-wide default), with the
/// Component form available for per-ship overrides.
#[derive(Resource, Component, Clone, Debug)]
pub struct SensorsAiConfigResource {
    /// Delay (seconds) between a target lock and the AI-driven Sensors
    /// operator emitting a `FrequencyHint` coordination message to Tactical.
    pub frequency_hint_delay_secs: f32,
}

impl Default for SensorsAiConfigResource {
    fn default() -> Self {
        Self {
            frequency_hint_delay_secs: 3.0,
        }
    }
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipSensorsPlugin;

impl Plugin for ShipSensorsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<CoordinationEnqueue>()
            .init_resource::<SensorsAiConfigResource>()
            .add_systems(
                Update,
                (
                    handle_sensors_messages.in_set(crate::sim_sets::SimSet::Input),
                    operate_sensors_ai.in_set(crate::sim_sets::SimSet::Input),
                    tick_sensors_frequency_hint.in_set(crate::sim_sets::SimSet::Input),
                    tick_sensors_threat_warning.in_set(crate::sim_sets::SimSet::Input),
                    publish_sensors_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                ),
            );
    }
}

// ── Systems ────────────────────────────────────────────────────────────────────

/// Handle `SetScienceTarget` messages from the Sensors console.
///
/// Validates: sender holds the Sensors station and `accept_human_input` is
/// true. Stores the target in [`SensorsTarget`] for blackboard broadcast, and
/// emits a `CoordinationPayload::TargetDesignation` on the channel-3 bus for
/// Tactical (issue #676 — replaces the old direct `SensorsTargetSuggestion`).
/// Enqueued unconditionally for every ship (player + NPC), matching how
/// `tick_sensors_frequency_hint` already handles both.
pub fn handle_sensors_messages(
    mut ship_query: Query<
        (
            Entity,
            &crate::messages::AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
            &mut SensorsTarget,
            &crate::ship_plugin::ShipSystemControlSources,
        ),
        With<crate::server_app::Ship>,
    >,
    entity_name_q: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, admitted, _ship_config, mut entity_target, control_sources) in
        ship_query.iter_mut()
    {
        for cmd in admitted.for_target(crate::system_registry::SENSORS_SYSTEM_ID) {
            let SystemControlPayload::SetScienceTarget { uuid } = &cmd.payload else {
                continue;
            };

            // Write to this ship's own SensorsTarget component (player or NPC).
            entity_target.0 = Some(uuid.clone());

            // Resolve a human-readable label for the target, falling back to
            // the raw uuid if no matching EntityName is found (e.g. asteroids
            // don't carry EntityName).
            let label = entity_name_q
                .iter()
                .find_map(|(u, n)| (u.0 == *uuid).then(|| n.0.clone()))
                .unwrap_or_else(|| uuid.clone());

            let sender_origin = control_sources
                .0
                .source_for(&crate::system_registry::sensors_system_id());

            writer.write(CoordinationEnqueue {
                source_entity: entity,
                sender_origin,
                target: crate::system_registry::tactical_station_key(),
                payload: CoordinationPayload::TargetDesignation {
                    uuid: uuid.clone(),
                    label,
                },
                sender_label: "Sensors".to_string(),
            });
        }
    }
}

/// Emit a channel-3 `FrequencyHint` coordination message to Tactical whenever
/// each ship's locked target changes.
///
/// Iterates every ship (player + NPC) so NPC Sensors→Tactical hints flow
/// through the coordination bus alongside the player's. Each emission
/// stamps its source ship so the enqueue handler routes it correctly.
///
/// Skips ships whose Sensors is fully AI-operated (`operate_ai` policy) AND
/// which carry `AiHighFidelity` (issue #692) — those ships hand off to
/// `console_ai::server::ai_frequency_hint`, which replicates a Low-complexity
/// operator's reaction delay via `console_ai::tick_frequency_hint` instead of
/// this system's immediate readout. Human-held Sensors (the overwhelmingly
/// common case for the player ship) is unaffected.
pub fn tick_sensors_frequency_hint(
    mut ship_q: Query<
        (
            Entity,
            &crate::simulation::WeaponsTarget,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut SensorsFrequencyState,
            Has<crate::ai_plugin::AiHighFidelity>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut writer: MessageWriter<CoordinationEnqueue>,
    target_shields_q: Query<(
        &crate::entity_spawner::EntityUuid,
        &crate::ship::shields::ShipShields,
    )>,
) {
    for (entity, weapons_target, control_sources, mut state, is_high_fidelity) in ship_q.iter_mut()
    {
        if is_high_fidelity
            && control_sources
                .0
                .policy_for(&crate::system_registry::sensors_system_id())
                .operate_ai
        {
            continue;
        }

        let current_target = match weapons_target.0.clone() {
            Some(uuid) => uuid,
            None => {
                state.last_sent_target = None;
                state.last_sent_frequency = None;
                continue;
            }
        };

        // Look up the target entity's shield frequency; fall back to 0.5.
        let frequency = target_shields_q
            .iter()
            .find_map(|(uuid, shields)| {
                if uuid.0 == current_target {
                    Some(shields.frequency())
                } else {
                    None
                }
            })
            .unwrap_or(0.5);

        let target_changed = state.last_sent_target.as_deref() != Some(&current_target);
        let frequency_changed = state.last_sent_frequency != Some(frequency);

        if !target_changed && !frequency_changed {
            continue;
        }

        state.last_sent_target = Some(current_target);
        state.last_sent_frequency = Some(frequency);

        let sender_origin = control_sources
            .0
            .source_for(&crate::system_registry::sensors_system_id());

        writer.write(CoordinationEnqueue {
            source_entity: entity,
            sender_origin,
            target: crate::system_registry::tactical_station_key(),
            payload: CoordinationPayload::FrequencyHint { frequency },
            sender_label: "Sensors".to_string(),
        });
    }
}

/// Emit a channel-3 `ThreatBearing` coordination message to Shields whenever
/// each ship's sensors detect an in-range closing hostile (or incoming torpedo).
///
/// Debounced: only fires on a *new* threat or a materially changed bearing
/// (> configured `threat_bearing_epsilon_rad`, default ~10°). Iterates every ship
/// (player + NPC) so AI sensors feed AI shields through the coordination bus.
pub fn tick_sensors_threat_warning(
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    entity_positions: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::entity_spawner::FactionComponent>,
        ),
        Without<crate::server_app::Ship>,
    >,
    ship_positions: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &crate::ship_state::ShipPhysics,
            &crate::entity_spawner::FactionComponent,
        ),
        With<crate::server_app::Ship>,
    >,
    mut ships: Query<
        (
            Entity,
            &crate::entity_spawner::EntityUuid,
            &crate::ship_state::ShipPhysics,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut SensorsThreatState,
            &crate::modifiers::ShipModifiers,
            Option<&crate::entity_spawner::FactionComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    let cfg = &ship_config.0;
    let Some(faction_registry) = faction_registry else {
        return; // No faction registry available (e.g. in tests without world setup)
    };
    let reg = &faction_registry.0;

    // Build a list of all potential threat entities with their world positions
    // and factions. Collected upfront to avoid ECS borrow conflicts with the
    // mutable ship query below.
    let mut candidates: Vec<(String, f32, f32, Option<uuid::Uuid>)> = Vec::new();
    for (uuid, physics, faction) in &ship_positions {
        candidates.push((uuid.0.clone(), physics.x, physics.z, Some(faction.0)));
    }
    for (uuid, tf, faction_opt) in &entity_positions {
        if let Some(faction) = faction_opt {
            candidates.push((
                uuid.0.clone(),
                tf.translation.x,
                tf.translation.z,
                Some(faction.0),
            ));
        }
    }

    for (entity, self_uuid, physics, control_sources, mut state, modifiers, self_faction) in
        ships.iter_mut()
    {
        let radar_mult = modifiers.get(&ModifierSlot::SensorRadarRange);
        let sensor_range = cfg.sensors_radar_range * radar_mult;
        if sensor_range <= 0.0 {
            continue;
        }

        let range_sq = sensor_range * sensor_range;
        let sx = physics.x;
        let sz = physics.z;
        let yaw = physics.yaw;
        let self_f_uuid = self_faction.map(|f| f.0);

        // Find the closest enemy within sensor range.
        let mut closest: Option<(String, f32, f32, f32)> = None; // uuid, dx, dz, dist_sq
        for (other_uuid, ox, oz, other_faction) in &candidates {
            if other_uuid == &self_uuid.0 {
                continue;
            }
            let Some(other_f) = other_faction else {
                continue;
            };
            if !crate::faction::is_enemy(self_f_uuid, Some(*other_f), reg) {
                continue;
            }
            let dx = ox - sx;
            let dz = oz - sz;
            let dsq = dx * dx + dz * dz;
            if dsq > range_sq {
                continue;
            }
            if closest.as_ref().is_none_or(|(_, _, _, d)| dsq < *d) {
                closest = Some((other_uuid.clone(), dx, dz, dsq));
            }
        }

        // No threat in range — clear state.
        let Some((threat_uuid, dx, dz, dist_sq)) = closest else {
            if state.last_threat_uuid.is_some() {
                state.last_threat_uuid = None;
                state.last_bearing_rad = None;
                state.last_label = None;
                state.last_distance = None;
            }
            continue;
        };

        let distance = dist_sq.sqrt();

        // Compute relative bearing (0 = dead ahead, positive = to starboard).
        let absolute_bearing = dx.atan2(-dz);
        let mut relative_bearing = absolute_bearing - yaw;
        if relative_bearing > std::f32::consts::PI {
            relative_bearing -= std::f32::consts::TAU;
        } else if relative_bearing < -std::f32::consts::PI {
            relative_bearing += std::f32::consts::TAU;
        }

        let is_new_threat = state.last_threat_uuid.as_deref() != Some(&threat_uuid);
        let bearing_changed = state
            .last_bearing_rad
            .is_none_or(|last| (relative_bearing - last).abs() > cfg.threat_bearing_epsilon_rad);

        if !is_new_threat && !bearing_changed {
            continue;
        }

        let bearing_deg = (relative_bearing.to_degrees() + 360.0) % 360.0;
        let label = format!("Hostile closing, range {distance:.0}m, bearing {bearing_deg:.0}°");

        state.last_threat_uuid = Some(threat_uuid.clone());
        state.last_bearing_rad = Some(relative_bearing);
        state.last_label = Some(label.clone());
        state.last_distance = Some(distance);

        let sender_origin = control_sources
            .0
            .source_for(&crate::system_registry::sensors_system_id());

        writer.write(CoordinationEnqueue {
            source_entity: entity,
            sender_origin,
            target: crate::system_registry::shields_system_id(),
            payload: CoordinationPayload::ThreatBearing {
                bearing_rad: relative_bearing,
                label,
            },
            sender_label: "Sensors".to_string(),
        });
    }
}

// ── Blackboard publish ────────────────────────────────────────────────────────

pub fn publish_sensors_blackboard(
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    sensors_target_q: Query<&SensorsTarget, With<crate::server_app::LocalShip>>,
    modifiers_q: Query<&crate::modifiers::ShipModifiers, With<crate::server_app::LocalShip>>,
    mut ship_bbs_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    let cfg = &ship_config.0;
    let science_target_uuid = sensors_target_q.single().ok().and_then(|st| st.0.clone());
    // Live sensor radar range: base config range scaled by the dedicated
    // `SensorRadarRange` modifier, which `apply_radar_damage_modifiers` keeps
    // in sync with the `sensor-radar` system's damage tier each tick.
    let radar_mult = modifiers_q
        .single()
        .map(|m| m.get(&ModifierSlot::SensorRadarRange))
        .unwrap_or(1.0);
    let bb = SensorsBlackboard {
        radar_range: cfg.sensors_radar_range * radar_mult,
        radar_shows: cfg.sensors_radar_shows.clone(),
        radar_selects: cfg.sensors_radar_selects.clone(),
        science_target_uuid,
    };

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(crate::system_registry::SENSORS_SYSTEM_ID.to_string()),
            SystemBlackboard::Sensors(bb),
        );
    }
}

/// Per-entity AI loop for the Sensors system. Loops over all ship entities
/// where the Sensors system is `ControlSource::Ai`.
///
/// Selection priority:
///   1. Combat target — mirror the ship's `WeaponsTarget` (set by
///      `ai_target_selection`) so the Sensors console shows what Tactical is
///      engaging.
///   2. Objective entity — scan scored objectives for a `Destroy` directive
///      with a named target (not the `""` engage-any sentinel), resolve the
///      name to an entity UUID, and select it on the Sensors console.
pub fn operate_sensors_ai(
    mut ships: Query<
        (
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &mut SensorsTarget,
            &crate::simulation::WeaponsTarget,
        ),
        With<crate::server_app::Ship>,
    >,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    entity_q: Query<(
        &crate::entity_spawner::EntityUuid,
        Option<&crate::entities::spawner::EntityName>,
    )>,
) {
    for (sources, blackboards, mut sensors_target, weapons_target) in &mut ships {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::sensors_system_id());
        if !policy.operate_ai {
            continue;
        }

        // Priority 1: mirror the combat target.
        if let Some(target_uuid) = &weapons_target.0 {
            if entity_q.iter().any(|(u, _)| u.0 == *target_uuid) {
                sensors_target.0 = Some(target_uuid.clone());
                continue;
            }
        }

        // Priority 2: scan scored objectives for a named Destroy target.
        let viewscreen_bb = blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id());
        if let Some(crate::messages::SystemBlackboard::Viewscreen(bb)) = viewscreen_bb {
            let mut selected: Option<String> = None;
            for objective in bb.scored_objectives.iter().filter(|o| o.score > 0.0) {
                if let crate::messages::AiDirective::Destroy { target } = &objective.directive {
                    if target.is_empty() {
                        continue;
                    }
                    let uuid = runtime
                        .as_ref()
                        .and_then(|rt| rt.name_to_uuid.get(target).cloned())
                        .or_else(|| {
                            entity_q.iter().find_map(|(u, name)| {
                                (u.0 == *target || name.is_some_and(|n| n.0 == *target))
                                    .then(|| u.0.clone())
                            })
                        });
                    if let Some(uuid) = uuid {
                        selected = Some(uuid);
                        break;
                    }
                }
            }
            sensors_target.0 = selected;
        } else {
            sensors_target.0 = None;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::ship::control_source::ControlSource;
    use crate::simulation::{ShipImpulse, SimOutbox};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    #[derive(Resource, Default)]
    struct EnqueueLog(Vec<CoordinationEnqueue>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn collect_enqueues(
        mut reader: MessageReader<CoordinationEnqueue>,
        mut log: ResMut<EnqueueLog>,
    ) {
        for m in reader.read() {
            log.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .init_resource::<EnqueueLog>()
            .init_resource::<crate::lobby::server::ShipClientConfigResource>()
            .add_plugins(ShipSensorsPlugin)
            .add_systems(PostUpdate, (collect, collect_enqueues));
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            SensorsTarget::default(),
            // PR 7 (issue #597) — WeaponsTarget is now per-entity Component.
            crate::simulation::WeaponsTarget::default(),
            SensorsFrequencyState::default(),
            ShipImpulse(crate::impulse::ImpulseState::new()),
        ));
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage {
                target,
                msg,
                delivery: crate::messages::DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game_with_sensors_and_tactical(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(app);
        push(
            app,
            "tactical",
            ClientMessage::Identify {
                token: "tactical".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "tactical",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "sensors", ClientMessage::SetReady { ready: true });
        push(app, "tactical", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    #[test]
    fn sensors_set_science_target_enqueues_target_designation_for_tactical() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        push(
            &mut app,
            "sensors",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId(
                    crate::system_registry::SENSORS_SYSTEM_ID.to_string(),
                ),
                payload: SystemControlPayload::SetScienceTarget {
                    uuid: "asteroid-42".into(),
                },
            },
        );
        tick(&mut app);

        let log = app.world().resource::<EnqueueLog>();
        let enqueued = log
            .0
            .iter()
            .find(|e| matches!(&e.payload, CoordinationPayload::TargetDesignation { .. }))
            .expect("expected a TargetDesignation CoordinationEnqueue event");

        assert_eq!(
            enqueued.target,
            crate::system_registry::tactical_station_key(),
            "TargetDesignation should be enqueued for the Tactical system"
        );
        match &enqueued.payload {
            CoordinationPayload::TargetDesignation { uuid, label } => {
                assert_eq!(uuid, "asteroid-42");
                // No EntityUuid/EntityName in this test world, so label falls
                // back to the raw uuid.
                assert_eq!(label, "asteroid-42");
            }
            other => panic!("expected TargetDesignation, got {other:?}"),
        }
    }

    #[test]
    fn non_sensors_player_cannot_send_science_target() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId(
                    crate::system_registry::SENSORS_SYSTEM_ID.to_string(),
                ),
                payload: SystemControlPayload::SetScienceTarget {
                    uuid: "asteroid-42".into(),
                },
            },
        );
        tick(&mut app);

        let log = app.world().resource::<EnqueueLog>();
        assert!(
            !log.0
                .iter()
                .any(|e| matches!(&e.payload, CoordinationPayload::TargetDesignation { .. })),
            "non-Sensors player should not be able to enqueue a TargetDesignation"
        );
    }

    /// Set the LocalShip's per-entity `WeaponsTarget` for tests.
    fn set_local_weapons_target(app: &mut App, uuid: Option<String>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::simulation::WeaponsTarget, With<crate::server_app::LocalShip>>();
        if let Ok(mut wt) = q.single_mut(app.world_mut()) {
            wt.0 = uuid;
        }
    }

    #[test]
    fn frequency_hint_emitted_when_target_changes() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        set_local_weapons_target(&mut app, Some("asteroid-1".into()));
        tick(&mut app); // emits first hint

        set_local_weapons_target(&mut app, Some("asteroid-2".into()));
        let enqueue_count = {
            // Tick and count CoordinationEnqueue events written
            app.update();
            // We verify indirectly — state should update to new target
            let mut q = app
                .world_mut()
                .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
            q.single(app.world())
                .expect("LocalShip must carry SensorsFrequencyState")
                .last_sent_target
                .clone()
        };

        assert_eq!(
            enqueue_count.as_deref(),
            Some("asteroid-2"),
            "state should track the new target after it changes"
        );
    }

    #[test]
    fn frequency_hint_not_re_emitted_for_same_target() {
        let mut app = test_app();
        start_game_with_sensors_and_tactical(&mut app);

        set_local_weapons_target(&mut app, Some("asteroid-1".into()));
        tick(&mut app); // first emit

        let state_before = {
            let mut q = app
                .world_mut()
                .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap().last_sent_frequency
        };

        tick(&mut app); // second tick, same target

        let state_after = {
            let mut q = app
                .world_mut()
                .query_filtered::<&SensorsFrequencyState, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap().last_sent_frequency
        };

        assert_eq!(
            state_before, state_after,
            "state should not change when target is unchanged"
        );
    }

    /// Verifies that operate_sensors_ai skips entities where Sensors is Human,
    /// and runs (without panic) for entities where Sensors is Ai (issue #589 AC).
    #[test]
    fn operate_sensors_ai_runs_per_entity_for_ai_controlled_ships() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        // Human-controlled: operate_sensors_ai must do nothing.
        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Human,
        );
        let human_sources = crate::ship_plugin::ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::sensors_system_id());
        assert!(
            !human_policy.operate_ai,
            "human Sensors should not operate AI"
        );

        // AI-controlled: operate_sensors_ai must gate and proceed.
        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = crate::ship_plugin::ShipSystemControlSources(ai_resolver);
        let ai_policy = ai_sources
            .0
            .policy_for(&crate::system_registry::sensors_system_id());
        assert!(
            ai_policy.operate_ai,
            "AI Sensors must gate through operate_ai"
        );
    }

    // ── tick_sensors_threat_warning tests ──────────────────────────────────────

    /// Helper: initialise a faction registry with Federation (self) and Harrow
    /// (enemy) factions, register the sensor range, and spawn the local ship.
    fn test_app_with_factions() -> (App, uuid::Uuid, uuid::Uuid) {
        let mut app = test_app();

        // Seed the faction registry so is_enemy works.
        let fed_uuid = uuid::Uuid::new_v4();
        let harrow_uuid = uuid::Uuid::new_v4();
        let mut reg = crate::faction::FactionRegistry::new();
        reg.insert(crate::faction::FactionConfig {
            uuid: fed_uuid,
            name: "Federation".into(),
            enemies: vec![harrow_uuid],
        });
        reg.insert(crate::faction::FactionConfig {
            uuid: harrow_uuid,
            name: "Harrow".into(),
            enemies: vec![fed_uuid],
        });
        app.insert_resource(crate::entities::config_cache::FactionRegistryResource(reg));

        // Add ShipPhysics, EntityUuid, SensorsThreatState, ShipModifiers,
        // and FactionComponent to the existing test ship entity.
        let ship_uuid = uuid::Uuid::new_v4().to_string();
        let mut ship_q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        let ship = ship_q.single_mut(app.world_mut()).unwrap();
        app.world_mut().entity_mut(ship).insert((
            crate::entity_spawner::EntityUuid(ship_uuid.clone()),
            SensorsThreatState::default(),
            crate::modifiers::ShipModifiers::new(),
            crate::entity_spawner::FactionComponent(fed_uuid),
            crate::ship_state::ShipPhysics::default(),
        ));

        (app, fed_uuid, harrow_uuid)
    }

    /// Spawn a hostile entity at the given position.
    fn spawn_hostile(app: &mut App, uuid: &str, x: f32, z: f32, faction: uuid::Uuid) {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.to_string()),
            crate::entities::spawner::EntityName(format!("Hostile-{uuid}")),
            Transform::from_xyz(x, 0.0, z),
            crate::entity_spawner::FactionComponent(faction),
        ));
    }

    #[test]
    fn threat_warning_emitted_for_hostile_in_range() {
        let (mut app, _fed, harrow) = test_app_with_factions();
        spawn_hostile(&mut app, "h-1", 0.0, -200.0, harrow); // directly ahead, 200m

        tick(&mut app);

        let log = app.world().resource::<EnqueueLog>();
        let threat = log
            .0
            .iter()
            .find(|e| matches!(&e.payload, CoordinationPayload::ThreatBearing { .. }))
            .expect("expected a ThreatBearing CoordinationEnqueue");

        assert_eq!(
            threat.target,
            crate::system_registry::shields_system_id(),
            "ThreatBearing should target the Shields system"
        );
        match &threat.payload {
            CoordinationPayload::ThreatBearing { bearing_rad, label } => {
                // Hostile at (0, -200) directly ahead → bearing ≈ 0 rad
                assert!(
                    bearing_rad.abs() < 0.1,
                    "bearing should be near 0 for target ahead, got {bearing_rad}"
                );
                assert!(
                    label.contains("Hostile closing"),
                    "label should contain threat description, got {label}"
                );
            }
            other => panic!("expected ThreatBearing, got {other:?}"),
        }
    }

    #[test]
    fn threat_warning_debounced_for_same_threat_and_bearing() {
        let (mut app, _fed, harrow) = test_app_with_factions();
        spawn_hostile(&mut app, "h-1", 0.0, -200.0, harrow);

        tick(&mut app); // first emission

        let state = {
            let mut q = app
                .world_mut()
                .query_filtered::<&SensorsThreatState, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap().last_threat_uuid.clone()
        };
        assert_eq!(
            state.as_deref(),
            Some("h-1"),
            "state should track the threat uuid"
        );

        // Clear logged events
        app.world_mut().resource_mut::<EnqueueLog>().0.clear();

        tick(&mut app); // second tick, same hostile, same bearing

        let log = app.world().resource::<EnqueueLog>();
        let new_threats = log
            .0
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::ThreatBearing { .. }))
            .count();
        assert_eq!(
            new_threats, 0,
            "should not re-emit ThreatBearing for the same threat and bearing"
        );
    }

    #[test]
    fn threat_warning_not_emitted_for_out_of_range_hostile() {
        let (mut app, _fed, harrow) = test_app_with_factions();
        // Default sensor range is 500; place hostile at 1000m
        spawn_hostile(&mut app, "far-1", 0.0, -1000.0, harrow);

        tick(&mut app);

        let log = app.world().resource::<EnqueueLog>();
        let threat = log
            .0
            .iter()
            .find(|e| matches!(&e.payload, CoordinationPayload::ThreatBearing { .. }));
        assert!(
            threat.is_none(),
            "should not emit ThreatBearing for out-of-range hostile"
        );
    }

    #[test]
    fn threat_warning_re_emitted_on_bearing_change() {
        let (mut app, _fed, harrow) = test_app_with_factions();
        spawn_hostile(&mut app, "h-1", 0.0, -200.0, harrow); // directly ahead

        tick(&mut app); // first emission, bearing ≈ 0

        // Clear logged events
        app.world_mut().resource_mut::<EnqueueLog>().0.clear();

        // Move hostile to starboard (~45°)
        let mut hostile_q = app
            .world_mut()
            .query_filtered::<&mut Transform, With<crate::entity_spawner::EntityUuid>>();
        for mut tf in hostile_q.iter_mut(app.world_mut()) {
            tf.translation.x = 200.0;
            tf.translation.z = -200.0;
        }

        tick(&mut app); // second emission — bearing changed enough

        let log = app.world().resource::<EnqueueLog>();
        let re_emitted = log
            .0
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::ThreatBearing { .. }))
            .count();
        assert_eq!(
            re_emitted, 1,
            "should re-emit ThreatBearing when bearing changes materially"
        );
    }

    #[test]
    fn threat_warning_state_cleared_when_no_threat() {
        let (mut app, _fed, harrow) = test_app_with_factions();
        spawn_hostile(&mut app, "h-1", 0.0, -200.0, harrow);

        tick(&mut app); // first emission — threat detected

        // Despawn the hostile (exclude the LocalShip)
        let mut hostile_q = app.world_mut().query_filtered::<Entity, (
            With<crate::entity_spawner::EntityUuid>,
            Without<crate::server_app::LocalShip>,
        )>();
        if let Some(hostile) = hostile_q.iter_mut(app.world_mut()).next() {
            app.world_mut().entity_mut(hostile).despawn();
        }

        // Clear logged events
        app.world_mut().resource_mut::<EnqueueLog>().0.clear();

        tick(&mut app); // tick without threat

        let state = {
            let mut q = app
                .world_mut()
                .query_filtered::<&SensorsThreatState, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap().last_threat_uuid.clone()
        };
        assert_eq!(
            state, None,
            "state should be cleared when no threat remains"
        );
    }

    // ── operate_sensors_ai tests ────────────────────────────────────────────

    fn sensors_ai_test_app() -> App {
        let mut app = App::new();
        app.insert_resource(bevy::time::Time::<()>::default())
            .init_resource::<crate::world::server::WorldContentRuntime>()
            .add_systems(Update, operate_sensors_ai);

        let mut control_sources = crate::ship_plugin::ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Ai,
        );

        app.world_mut().spawn((
            crate::server_app::Ship,
            control_sources,
            crate::server_app::ShipSystemBlackboards::default(),
            SensorsTarget::default(),
            crate::simulation::WeaponsTarget::default(),
        ));

        app
    }

    fn insert_viewscreen_objective(app: &mut App, target_name: &str, score: f32) {
        let viewscreen = crate::messages::ViewscreenBlackboard {
            scored_objectives: vec![crate::messages::ScoredObjective {
                id: format!("obj-destroy-{target_name}"),
                score,
                directive: crate::messages::AiDirective::Destroy {
                    target: target_name.into(),
                },
                source: crate::messages::ObjectiveSource::Mission,
                relevance: vec![
                    crate::messages::SystemAffinity::Helm,
                    crate::messages::SystemAffinity::Weapons,
                    crate::messages::SystemAffinity::Captain,
                ],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: format!("obj-destroy-{target_name}"),
                    text: format!("Destroy {target_name}"),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![target_name.into()],
                    source: crate::messages::ObjectiveSource::Mission,
                },
            }],
            ..Default::default()
        };
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<crate::server_app::Ship>>();
        let mut bbs = q
            .single_mut(app.world_mut())
            .expect("Ship must have ShipSystemBlackboards");
        bbs.0.insert(
            crate::system_registry::viewscreen_system_id(),
            crate::messages::SystemBlackboard::Viewscreen(viewscreen),
        );
    }

    fn get_sensors_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&SensorsTarget, With<crate::server_app::Ship>>();
        q.single(app.world()).unwrap().0.clone()
    }

    fn tick_sensors_ai(app: &mut App) {
        let mut time = app.world_mut().resource_mut::<bevy::time::Time>();
        time.advance_by(std::time::Duration::from_secs_f32(0.1));
        app.update();
    }

    #[test]
    fn ai_sensors_mirrors_weapons_target() {
        let mut app = sensors_ai_test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();

        app.world_mut()
            .spawn((crate::entity_spawner::EntityUuid(target_uuid.clone()),));
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::WeaponsTarget, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
        }

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "sensors AI should mirror WeaponsTarget"
        );
    }

    #[test]
    fn ai_sensors_selects_destroy_objective_when_no_weapons_target() {
        let mut app = sensors_ai_test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();

        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_viewscreen_objective(&mut app, "wave_1", 80.0);

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "sensors AI should select named Destroy objective target"
        );
    }

    #[test]
    fn ai_sensors_skips_untargeted_destroy() {
        let mut app = sensors_ai_test_app();

        let viewscreen = crate::messages::ViewscreenBlackboard {
            scored_objectives: vec![crate::messages::ScoredObjective {
                id: "obj-destroy-any".into(),
                score: 80.0,
                directive: crate::messages::AiDirective::Destroy { target: "".into() },
                source: crate::messages::ObjectiveSource::Doctrine,
                relevance: vec![
                    crate::messages::SystemAffinity::Helm,
                    crate::messages::SystemAffinity::Weapons,
                    crate::messages::SystemAffinity::Captain,
                ],
                snapshot: crate::messages::ObjectiveSnapshot {
                    id: "obj-destroy-any".into(),
                    text: "Engage hostiles".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Doctrine,
                },
            }],
            ..Default::default()
        };
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::server_app::ShipSystemBlackboards, With<crate::server_app::Ship>>();
            let mut bbs = q
                .single_mut(app.world_mut())
                .expect("Ship must have ShipSystemBlackboards");
            bbs.0.insert(
                crate::system_registry::viewscreen_system_id(),
                crate::messages::SystemBlackboard::Viewscreen(viewscreen),
            );
        }

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app),
            None,
            "sensors AI should skip untargeted Destroy directives"
        );
    }

    #[test]
    fn ai_sensors_prefers_weapons_target_over_objective() {
        let mut app = sensors_ai_test_app();
        let objective_uuid = uuid::Uuid::new_v4().to_string();
        let combat_uuid = uuid::Uuid::new_v4().to_string();

        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), objective_uuid.clone());
        insert_viewscreen_objective(&mut app, "wave_1", 80.0);

        app.world_mut()
            .spawn((crate::entity_spawner::EntityUuid(combat_uuid.clone()),));
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::WeaponsTarget, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(combat_uuid.clone());
        }

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(combat_uuid.as_str()),
            "sensors AI should prefer WeaponsTarget over objective target"
        );
    }

    #[test]
    fn ai_sensors_does_not_select_objective_when_weapons_target_is_some_but_entity_gone() {
        let mut app = sensors_ai_test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();

        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_viewscreen_objective(&mut app, "wave_1", 80.0);

        // WeaponsTarget names a UUID that no entity carries → existence check fails
        let dead_uuid = uuid::Uuid::new_v4().to_string();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::WeaponsTarget, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(dead_uuid);
        }

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "sensors AI should fall through to objective when WeaponsTarget entity is gone"
        );
    }
}

use crate::simmath;
use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
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
pub struct SensorRadarSelection(pub Option<String>);

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
/// (`console_ai::server::tick_frequency_hint_high_fidelity`, issue #692).
///
/// Loaded from `[sensors_console.ai]` in the ship entity TOML. Defaults are
/// used when the section is absent.
///
/// Dual `Resource + Component`, mirroring `ShieldsAiConfigResource` — but the
/// Resource half is **structural symmetry only, and has never been seeded**.
/// `ShipSensorsPlugin::build` registers it with `init_resource` and nothing
/// anywhere writes it (there is no sensors equivalent of the shields dual-write
/// in `server_app::spawn_game_start_entities`), so it has only ever held
/// `Self::default()`. Every read goes through the per-entity Component, which
/// the spawner and `spawn_game_start_entities` both attach; see
/// `console_ai::server::tick_frequency_hint_high_fidelity`. Do not reintroduce a `Res<_>` read
/// here: it applies one ship's tuning to every ship.
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

/// Per-ship resolved Sensors target selector (issue #776).
///
/// Holds the ship's data-driven [`crate::ai::selector::TargetSelector`], decoded
/// from the authored `[sensors_console.selector]` block, plus the authored ship
/// `power_rating`, which `operate_sensors_ai` exposes to the selector's
/// expressions as `self_fact(power_rating)`. Attached at spawn alongside
/// `SensorsAiConfigResource` / `CaptainAiPolicy`.
///
/// Since #885b stage 5d there is no Rust-side synthesised default behind it: a
/// ship without the component ranks nothing and `operate_sensors_ai` skips it.
#[derive(Component, Clone, Debug)]
pub struct SensorsTargetSelector {
    /// The resolved ranking policy.
    pub selector: crate::ai::selector::TargetSelector,
    /// Authored ship power rating, seeded from `EntityConfig.power_rating`.
    pub power_rating: Option<f32>,
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipSensorsPlugin;

impl Plugin for ShipSensorsPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumer (issue #833): `handle_sensors_messages`
        // reads the `sensors` system's admitted commands.
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::system_registry::SENSORS_SYSTEM_ID,
        ));
        // The ONE shared AI decision cadence (issue #889), which also derives
        // the slower snapshot latch `operate_sensors_ai` gates on.
        crate::ai::cadence::register_ai_cadence(app);
        app.add_message::<CoordinationEnqueue>()
            .init_resource::<SensorsAiConfigResource>()
            .add_systems(
                FixedUpdate,
                (
                    // In `SimSet::Physics`, not Input (issue #828, the #826
                    // shields shape): `admit_system_commands` clears every
                    // ship's `AdmittedCommands` before Input each tick, and
                    // the AI decide system (`operate_sensors_ai`, Input)
                    // refills it same-tick via `validate_and_admit` — so the
                    // applier must consume *after* the AI emit or AI commands
                    // would be silently lost. The `.before` edge on the decide
                    // system below is the one explicit ordering between them.
                    handle_sensors_messages.in_set(crate::sim_sets::SimSet::Physics),
                    // Decide only (issue #828): emits admitted
                    // SetScienceTarget / ClearScienceTarget payloads; the
                    // single applier is `handle_sensors_messages` above.
                    // Gated by `run_if` on the derived slower snapshot cadence
                    // (issue #889) — the same rate the retired in-body
                    // `Option<Res<AiSnapshotReady>>` check enforced in
                    // production, minus its evaluate-every-tick fallback.
                    operate_sensors_ai
                        .in_set(crate::sim_sets::SimSet::Input)
                        .before(handle_sensors_messages)
                        .run_if(crate::ai::cadence::ai_snapshot_ready),
                    tick_sensors_frequency_hint.in_set(crate::sim_sets::SimSet::Input),
                    tick_sensors_threat_warning.in_set(crate::sim_sets::SimSet::Input),
                    publish_sensors_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                    publish_sensor_radar_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                ),
            );
    }
}

// ── Systems ────────────────────────────────────────────────────────────────────

/// Handle admitted `SetScienceTarget` / `ClearScienceTarget` commands — the
/// single applier for human and AI Sensors commands alike (issue #828).
///
/// Admission already validated the sender (station tenure for humans,
/// `operate_ai` for `ai:` tokens), so nothing here branches on origin. Stores
/// the target in [`SensorRadarSelection`] for blackboard broadcast, and — for a set,
/// never a clear — emits a `CoordinationPayload::TargetDesignation` on the
/// channel-3 bus for Tactical (issue #676 — replaces the old direct
/// `SensorsTargetSuggestion`; it advises Tactical, it does not replace
/// Tactical target authority). Enqueued unconditionally for every ship
/// (player + NPC), matching how `tick_sensors_frequency_hint` already
/// handles both.
pub fn handle_sensors_messages(
    mut ship_query: Query<
        (
            Entity,
            &crate::messages::AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
            &mut SensorRadarSelection,
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
            let uuid = match &cmd.payload {
                SystemControlPayload::SetScienceTarget { uuid } => uuid,
                SystemControlPayload::ClearScienceTarget => {
                    // A clear deselects — there is no contact to designate,
                    // so no channel-3 advisory is emitted.
                    entity_target.0 = None;
                    continue;
                }
                _ => continue,
            };

            // Write to this ship's own SensorRadarSelection component (player or NPC).
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
                sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
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
/// # Which ships this system serves
///
/// Ships that do NOT carry `AiHighFidelity`. High-fidelity ships hand off to
/// `console_ai::server::tick_frequency_hint_high_fidelity`, which routes the
/// same authoritative reading through `console_ai::tick_frequency_hint`'s
/// operator reaction-delay model instead of this system's immediate readout.
///
/// That split is a **level-of-detail** split, and since issue #873 it is
/// nothing else. It used to also require the ship's Sensors to be
/// `operate_ai`, which made the hint's existence, content and timing depend on
/// *who was holding the console* — the exact human/AI branch downstream of
/// admission that AGENTS.md rule 6 forbids, and the reason a human on Sensors
/// fed a different bus than the AI did. Whoever holds Sensors, the ship emits
/// the same fact at the same moment from the same authoritative state; only the
/// simulation fidelity of the hull decides which of the two models produces it,
/// and `sender_origin` below is a routing tag stamped after that decision, never
/// an input to it.
pub fn tick_sensors_frequency_hint(
    mut ship_q: Query<
        (
            Entity,
            &crate::server_app::ShipSystemBlackboards,
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
    for (entity, blackboards, control_sources, mut state, is_high_fidelity) in ship_q.iter_mut() {
        // LOD split only (issue #873) — see this system's doc comment. Do NOT
        // re-add an `operate_ai` conjunct here: it would put the emission of a
        // coordination fact back under the control of who holds the console.
        if is_high_fidelity {
            continue;
        }

        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3).
        let combat_lock = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
            _ => None,
        };
        let current_target = match combat_lock {
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
            sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
        });
    }
}

/// This ship's own sensor horizon, in world units.
///
/// `ShipClientConfigResource` describes the **local player's** hull, but every
/// sensors system here iterates all ships, NPCs included. Reading the player's
/// `sensors_radar_range` for a Harrow destroyer gave it the player's reach —
/// which is how AI ships came to lock targets from far outside their own range.
/// An NPC carries its own `AiProfile.sensor_range` (`[ai_profile] sensor_range`
/// in the entity TOML), so prefer that and fall back to the console config only
/// for ships that have no AI profile at all.
fn effective_sensor_range(
    profile: Option<&crate::ai::server::AiProfile>,
    console_range: f32,
    modifiers: &crate::modifiers::ShipModifiers,
) -> f32 {
    let base = profile
        .map(|p| p.sensor_range)
        .filter(|r| r.is_finite() && *r > 0.0)
        .unwrap_or(console_range);
    base * modifiers.get(&ModifierSlot::SensorRadarRange)
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
            Option<&crate::ai::server::AiProfile>,
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

    for (
        entity,
        self_uuid,
        physics,
        control_sources,
        mut state,
        modifiers,
        self_faction,
        ai_profile,
    ) in ships.iter_mut()
    {
        let sensor_range = effective_sensor_range(ai_profile, cfg.sensors_radar_range, modifiers);
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
        let absolute_bearing = simmath::atan2(dx, -dz);
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
            sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
        });
    }
}

// ── Blackboard publish ────────────────────────────────────────────────────────

/// Publish every ship's own Sensors blackboard into that ship's
/// `ShipSystemBlackboards` (issue #828 — was LocalShip-only; per-Ship
/// following the #824 helm / #826 shields precedent), split on
/// `Has<LocalShip>`:
///
/// - `science_target_uuid` — each ship's own [`SensorRadarSelection`], player and
///   NPC alike.
/// - `radar_range` — live sensor radar range: the ship's own base range
///   scaled by its own `SensorRadarRange` modifier, which
///   `apply_radar_damage_modifiers` keeps in sync with the `sensor-radar`
///   system's damage tier each tick. The local ship's base is the console
///   config (`cfg.sensors_radar_range`, as before); an NPC's base follows
///   [`effective_sensor_range`]'s preference order — its own
///   `AiProfile.sensor_range`, falling back to the console config only for
///   hulls with no AI profile at all.
/// - `radar_shows` / `radar_selects` — authored presentation filters from
///   `ShipClientConfigResource`, which describes the **local player's** hull
///   only, so they are gated on `is_local`; NPCs don't render a radar and
///   get empty filters.
pub fn publish_sensors_blackboard(
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    mut ships_q: Query<
        (
            Option<&SensorRadarSelection>,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&crate::ai::server::AiProfile>,
            &mut crate::server_app::ShipSystemBlackboards,
            Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let cfg = &ship_config.0;
    for (sensors_target, modifiers, ai_profile, mut bbs, is_local) in ships_q.iter_mut() {
        let radar_mult = modifiers
            .map(|m| m.get(&ModifierSlot::SensorRadarRange))
            .unwrap_or(1.0);
        // The local ship keeps the console-config base; NPCs use the same
        // per-entity preference order as `effective_sensor_range`.
        let base_range = if is_local {
            cfg.sensors_radar_range
        } else {
            ai_profile
                .map(|p| p.sensor_range)
                .filter(|r| r.is_finite() && *r > 0.0)
                .unwrap_or(cfg.sensors_radar_range)
        };
        let (radar_shows, radar_selects) = if is_local {
            (
                cfg.sensors_radar_shows.clone(),
                cfg.sensors_radar_selects.clone(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let bb = SensorsBlackboard {
            radar_range: base_range * radar_mult,
            radar_shows,
            radar_selects,
            science_target_uuid: sensors_target.and_then(|st| st.0.clone()),
        };
        bbs.0.insert(
            SystemId(crate::system_registry::SENSORS_SYSTEM_ID.to_string()),
            SystemBlackboard::Sensors(bb),
        );
    }
}

/// Publish each ship's Sensor Radar blackboard (issue #829). Runs in
/// `SimSet::Publish`. The sensor radar owns the **Science Target**:
/// `selected_target` mirrors this ship's `SensorRadarSelection` component so the
/// viewscreen aggregator can lift it into `ViewscreenBlackboard::science_target`.
/// Reading the ship's own selection here is the sensor-radar authority, not a
/// cross-system read (spec §3).
pub fn publish_sensor_radar_blackboard(
    mut ships_q: Query<
        (
            Option<&SensorRadarSelection>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
    // Read-only lookup of Red-Alert-capable ships by uuid. `ShipRedAlert` is a
    // ship-only capability (attached inside the `[behaviour]`/player spawn gate),
    // so non-ship contacts (asteroid/star/planet/region) never appear here and a
    // selection that names one resolves to `None` → no alert field (issue #749).
    alert_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &crate::ship_state::ShipRedAlert,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (sensors_target, mut bbs) in ships_q.iter_mut() {
        let selected_target = sensors_target.and_then(|st| st.0.clone());
        // Resolve the selected target's authoritative Red Alert state. `Some(..)`
        // only when the selection names a Red-Alert-capable ship; `None` for no
        // selection, a non-ship contact, or an incapable target.
        let selected_target_alert = selected_target.as_deref().and_then(|selected| {
            alert_q
                .iter()
                .find(|(uuid, _)| uuid.0 == selected)
                .map(|(_, red_alert)| red_alert.0)
        });
        bbs.0.insert(
            crate::system_registry::sensor_radar_system_id(),
            SystemBlackboard::SensorRadar(crate::messages::SensorRadarBlackboard {
                selected_target,
                selected_target_alert,
            }),
        );
    }
}

/// Validate-and-enqueue one Sensors AI decision into this ship's own
/// `AdmittedCommands` (issue #828, mirroring `console_ai::server::
/// emit_shield_ai_command` from #826 / `ship::helm_ai::emit_helm_ai_command`
/// from #824): the AI's `ai:` token flows through the same
/// `validate_and_admit` seam network commands do, checked against this
/// entity's own `ControlSourceResolver` (`operate_ai` must hold). The write
/// happens in the same tick — `handle_sensors_messages` applies it later
/// this frame — so there is no one-tick queue lag on the AI sensors path.
fn emit_sensors_ai_command(
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    payload: crate::messages::SystemControlPayload,
    sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
    admitted: &mut crate::messages::AdmittedCommands,
) -> bool {
    emit_ai_command(
        entity_uuid,
        crate::system_registry::sensors_system_id(),
        payload,
        sources,
        sessions,
        ship_config,
        admitted,
    )
}

/// Build a [`crate::ai::selector::SelectorCandidate`] for a detectable,
/// hostile contact surfaced by one Sensors source (issue #776).
///
/// Every source the Sensors host feeds is a target the ship is meant to engage,
/// so each candidate carries `detectable = 1` and `hostile = 1` (the default
/// selector's eligibility guard) plus the `source_*` marker its score term
/// keys on. When the same UUID comes from more than one source the selector's
/// dedup folds the markers together, so a combat lock that is also the nearest
/// radar hostile scores on both terms.
fn detectable_candidate(
    uuid: &str,
    position: [f32; 3],
    source_fact: &str,
) -> crate::ai::selector::SelectorCandidate {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("detectable", 1.0);
    facts.set("hostile", 1.0);
    facts.set(source_fact, 1.0);
    crate::ai::selector::SelectorCandidate {
        uuid: uuid.to_string(),
        position,
        facts,
    }
}

/// Per-entity AI decide loop for the Sensors system. Loops over all ship
/// entities where the Sensors system is `ControlSource::Ai`.
///
/// Selection priority:
///   1. Combat target — mirror the ship's `TacticalRadarSelection` (set by
///      `ai_target_selection`) so the Sensors console shows what Tactical is
///      engaging.
///   2. Objective entity — scan scored objectives for a `Destroy` directive
///      with a named target (not the `""` engage-any sentinel), resolve the
///      name to an entity UUID, and select it on the Sensors console.
///   3. Nearest hostile — independent horizon-limited hostile selection
///      (issue #746). When neither Tactical's combat lock nor an objective
///      names a detectable target, Sensors picks the nearest faction-hostile
///      contact ([`crate::ai::find_nearest_hostile`]) inside this ship's own
///      live sensor horizon and designates it to Tactical as advisory
///      intelligence. This is *advice*, not authority: the designation flows
///      through the same [`emit_sensors_ai_command`] → `handle_sensors_messages`
///      applier a human selection does, which never mutates
///      `TacticalRadarSelection`. Tactical keeps final firing-target authority.
///
/// All three tiers are gated on this ship's own sensor horizon
/// ([`effective_sensor_range`]), which collapses as the sensor-radar hull
/// system takes damage (`apply_radar_damage_modifiers` shrinks the
/// `SensorRadarRange` modifier). A target that falls outside the shrunken
/// horizon — moved away, or the horizon itself damaged inward — yields no tier
/// and is dropped via an admitted `ClearScienceTarget`. Tier 1 inherited a
/// range check from `ai_target_selection` upstream, but tier 2 had none at all:
/// it resolved a name straight to a UUID and locked it at any distance, which
/// is why AI ships tracked contacts far outside their range. Naming a target in
/// an objective says who to engage, not that the ship can already see them.
///
/// Decide-and-emit (issue #828): instead of writing [`SensorRadarSelection`]
/// directly, the decision is emitted as an admitted `SetScienceTarget` /
/// `ClearScienceTarget` through [`emit_sensors_ai_command`], for
/// `handle_sensors_messages` to apply later this tick. Emission happens only
/// when the decided value differs from the current [`SensorRadarSelection`]: the old
/// direct writes were idempotent assignments, so no-change ticks produce no
/// admitted command (and therefore no channel-3 `TargetDesignation` spam) —
/// an AI selection now designates its target to Tactical exactly once, on
/// change, the same as a human selection through the applier.
/// # Cadence
/// Gated by `run_if(ai_snapshot_ready)` at registration (issue #889), not by an
/// `Option<Res<_>>` check inside the body. The in-body form fell back to
/// evaluating EVERY tick whenever the resource was absent — which is every
/// bare-`App` fixture in the crate — so the shipped cadence was not exercised
/// by a single unit test. The rate is unchanged: the derived slower snapshot
/// cadence, `[global] ai_tick_hz / ai_snapshot_hz` base ticks apart.
pub fn operate_sensors_ai(
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &SensorRadarSelection,
            &crate::ship_state::ShipPhysics,
            &crate::modifiers::ShipModifiers,
            Option<&crate::ai::server::AiProfile>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::entity_spawner::FactionComponent>,
            Option<&SensorsTargetSelector>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
    // Plain `Res` (issue #828): `LobbyPlugin` always inserts
    // `ShipClientConfigResource` and `tick_sensors_threat_warning` already
    // requires it, so the old `Option<Res<..>>` arm was dead optionality.
    // It remains the *fallback* of the per-entity preference order inside
    // `effective_sensor_range` — the legitimate local-player/profile-less arm.
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    // Loaded sub-world layers (issue #891 stage 2): the selector's flag chain
    // is anchored at the layer that spawned each ship.
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    entity_q: Query<(
        &crate::entity_spawner::EntityUuid,
        Option<&crate::entities::spawner::EntityName>,
        &Transform,
    )>,
    // Independent nearest-hostile tier (issue #746). Faction verdicts need the
    // registry; absent (tests without world setup), tier 3 is simply skipped.
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    // Candidate contacts for the hostile scan, split the same way
    // `tick_sensors_threat_warning` splits them: ships carry their live
    // position on `ShipPhysics` (the spawn `Transform` goes stale), non-ship
    // entities carry it on `Transform`. Both read-only, disjoint from the
    // mutable `AdmittedCommands` in `ships`, so no borrow conflict.
    hostile_ship_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &crate::ship_state::ShipPhysics,
            &crate::entity_spawner::FactionComponent,
        ),
        With<crate::server_app::Ship>,
    >,
    hostile_entity_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::entity_spawner::FactionComponent>,
        ),
        Without<crate::server_app::Ship>,
    >,
) {
    let console_range = ship_config.0.sensors_radar_range;

    // Build the shared candidate snapshot once (world state is the same for
    // every ship this tick). Each entry: (uuid, [x, y, z], faction).
    let hostile_candidates: Vec<(String, [f32; 3], Option<uuid::Uuid>)> = hostile_ship_q
        .iter()
        .map(|(uuid, physics, faction)| {
            (uuid.0.clone(), [physics.x, 0.0, physics.z], Some(faction.0))
        })
        .chain(hostile_entity_q.iter().map(|(uuid, tf, faction)| {
            (
                uuid.0.clone(),
                [tf.translation.x, tf.translation.y, tf.translation.z],
                faction.map(|f| f.0),
            )
        }))
        .collect();
    for (
        ship_entity,
        entity_uuid,
        sources,
        blackboards,
        sensors_target,
        physics,
        modifiers,
        ai_profile,
        ship_config_comp,
        self_faction,
        target_selector,
        mut admitted,
    ) in &mut ships
    {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::sensors_system_id());
        if !policy.operate_ai {
            continue;
        }
        // No authored `[sensors_console.selector]` ⇒ no component ⇒ no science
        // ranking. Since #885b stage 5d there is no synthesised stand-in.
        let Some(selector_comp) = target_selector else {
            continue;
        };

        let range = effective_sensor_range(ai_profile, console_range, modifiers);
        let range_sq = range * range;
        let in_range = |tf: &Transform| {
            let dx = tf.translation.x - physics.x;
            let dz = tf.translation.z - physics.z;
            dx * dx + dz * dz <= range_sq
        };
        // Same horizon test over a bare `[x, y, z]` (the nearest-hostile tier
        // works from position tuples, not `Transform`s).
        let in_range_pos = |pos: [f32; 3]| {
            let dx = pos[0] - physics.x;
            let dz = pos[2] - physics.z;
            dx * dx + dz * dz <= range_sq
        };

        // ── Build candidate sources for the data-driven target selector (#776) ──
        // Each source contributes contacts already pre-filtered to this ship's
        // live, damage-scaled horizon (the host owns the live gate — AC5). The
        // selector then unions + dedups them by identity, applies the authored
        // eligibility + additive utility, and retains the current target within
        // the authored switch margin. The three tiers of the retired hardcoded
        // decide (combat-lock mirror ≫ named objective ≫ nearest hostile) are
        // now the three registered sources, ordered by additive score weight.
        use crate::ai::selector::{SelectorCandidate, SelfContext};
        let mut candidates: Vec<SelectorCandidate> = Vec::new();

        // Combat Lock read from the frozen viewscreen blackboard (#829).
        let viewscreen_bb = blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id());
        if let Some(crate::messages::SystemBlackboard::Viewscreen(bb)) = viewscreen_bb {
            // Source: combat-lock — mirror Tactical's designated firing target.
            if let Some(target_uuid) = bb.combat_lock.as_deref() {
                if let Some(pos) = entity_q.iter().find_map(|(u, _, tf)| {
                    (u.0 == *target_uuid && in_range(tf)).then(|| tf.translation.to_array())
                }) {
                    candidates.push(detectable_candidate(target_uuid, pos, "source_combat_lock"));
                }
            }

            // Source: objective-destroy — named Destroy targets in the pool.
            // Resolving a name is not the same as seeing the ship, so each
            // candidate is still gated on the live horizon.
            for objective in bb.scored_objectives.iter().filter(|o| o.score > 0.0) {
                if let crate::messages::AiDirective::Destroy { target } = &objective.directive {
                    if target.is_empty() {
                        continue;
                    }
                    let uuid = runtime
                        .as_ref()
                        .and_then(|rt| rt.name_to_uuid.get(target).cloned())
                        .or_else(|| {
                            entity_q.iter().find_map(|(u, name, _)| {
                                (u.0 == *target || name.is_some_and(|n| n.0 == *target))
                                    .then(|| u.0.clone())
                            })
                        });
                    if let Some(uuid) = uuid {
                        if let Some(pos) = entity_q.iter().find_map(|(u, _, tf)| {
                            (u.0 == uuid && in_range(tf)).then(|| tf.translation.to_array())
                        }) {
                            candidates.push(detectable_candidate(&uuid, pos, "source_objective"));
                        }
                    }
                }
            }
        }

        // Source: radar-contacts — this ship's own nearest faction-hostile
        // (issue #746). Needs this ship's faction and a registry to judge
        // hostility; `find_nearest_hostile` returns the globally-nearest enemy,
        // so a single horizon check on its result suffices.
        if let (Some(self_faction), Some(registry)) = (self_faction, faction_registry.as_ref()) {
            let self_faction_uuid = self_faction.0;
            let self_uuid = entity_uuid.map(|u| u.0.as_str()).unwrap_or("");
            let entities: Vec<crate::ai::AiWorldEntity> = hostile_candidates
                .iter()
                .filter(|(u, _, _)| u != self_uuid)
                .filter_map(|(u, pos, faction)| {
                    Some(crate::ai::AiWorldEntity {
                        uuid: uuid::Uuid::parse_str(u).ok()?,
                        position: *pos,
                        faction: *faction,
                        ..Default::default()
                    })
                })
                .collect();
            let world_view = crate::ai::WorldView {
                entity_pos: [physics.x, 0.0, physics.z],
                entity_yaw: physics.yaw,
                entities,
                self_faction: Some(self_faction_uuid),
                ..crate::ai::WorldView::default()
            };
            if let Some(found) = crate::ai::find_nearest_hostile(&world_view, &registry.0) {
                if let Some((u, pos, _)) = hostile_candidates.iter().find(|(u, pos, _)| {
                    uuid::Uuid::parse_str(u).ok() == Some(found) && in_range_pos(*pos)
                }) {
                    candidates.push(detectable_candidate(u, *pos, "source_radar"));
                }
            }
        }

        // Self context: position (horizon filter) + authored power rating,
        // exposed to the selector expressions as `self_fact(power_rating)` (AC2).
        let mut self_facts = crate::world::flags::AiFacts::new();
        if let Some(pr) = selector_comp.power_rating {
            self_facts.set("power_rating", pr as f64);
        }
        let self_ctx = SelfContext {
            position: [physics.x, 0.0, physics.z],
            facts: self_facts,
        };

        // Retain the current selection through the authored switch margin (AC3);
        // an invalid current target fails eligibility and is replaced this same
        // tick (AC4). The scenario flag chain is anchored at the layer that
        // spawned this ship (issue #891 stage 2).
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(ship_entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );
        let decided = selector_comp.selector.select(
            &self_ctx,
            &candidates,
            sensors_target.0.as_deref(),
            &flag_chain,
        );

        // ── Emit on change only ────────────────────────────────────────────
        if decided == sensors_target.0 {
            continue;
        }
        let payload = match decided {
            Some(uuid) => crate::messages::SystemControlPayload::SetScienceTarget { uuid },
            None => crate::messages::SystemControlPayload::ClearScienceTarget,
        };
        emit_sensors_ai_command(
            entity_uuid,
            payload,
            sources,
            &sessions,
            ship_config_comp,
            &mut admitted,
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
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

    /// Test-only glue (issue #829): seed each ship's viewscreen combat_lock /
    /// science_target from its `TacticalRadarSelection` / `SensorRadarSelection`
    /// components before the consumers run, standing in for the radar publishers
    /// + viewscreen aggregators the full app runs.
    fn seed_viewscreen_from_selection(
        mut q: Query<
            (
                Option<&crate::weapons_plugin::TacticalRadarSelection>,
                Option<&SensorRadarSelection>,
                &mut crate::server_app::ShipSystemBlackboards,
            ),
            With<crate::server_app::Ship>,
        >,
    ) {
        for (tac, sci, mut bbs) in q.iter_mut() {
            let combat_lock = tac.and_then(|t| t.0.clone());
            let science_target = sci.and_then(|s| s.0.clone());
            let mut vbb = match bbs.0.get(&crate::system_registry::viewscreen_system_id()) {
                Some(SystemBlackboard::Viewscreen(v)) => v.clone(),
                _ => crate::messages::ViewscreenBlackboard::default(),
            };
            vbb.combat_lock = combat_lock;
            vbb.science_target = science_target;
            bbs.0.insert(
                crate::system_registry::viewscreen_system_id(),
                SystemBlackboard::Viewscreen(vbb),
            );
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        // The applier (`handle_sensors_messages`) moved to SimSet::Physics
        // (issue #828), so the harness needs the production set chain for
        // AdmissionSet → Input → Physics ordering to hold.
        app.configure_sets(
            FixedUpdate,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
                crate::sim_sets::SimSet::Damage,
                crate::sim_sets::SimSet::Modifiers,
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        );
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .init_resource::<EnqueueLog>()
            .init_resource::<crate::lobby::server::ShipClientConfigResource>()
            .add_plugins(ShipSensorsPlugin)
            .add_systems(
                FixedUpdate,
                seed_viewscreen_from_selection.before(crate::sim_sets::SimSet::Input),
            )
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
            SensorRadarSelection::default(),
            // PR 7 (issue #597) — TacticalRadarSelection is now per-entity Component.
            crate::simulation::TacticalRadarSelection::default(),
            SensorsFrequencyState::default(),
            ShipImpulse(crate::impulse::ImpulseState::new()),
        ));
        // One fixed step per update (issue #895): the plugin's systems run on
        // the logical tick, and each harness tick advances it once.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );
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

    /// Set the LocalShip's per-entity `TacticalRadarSelection` for tests.
    fn set_local_weapons_target(app: &mut App, uuid: Option<String>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::LocalShip>>();
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

    /// Issue #873: the hand-off to the high-fidelity emitter is a
    /// level-of-detail split and NOTHING else.
    ///
    /// It used to be `AiHighFidelity && policy_for(sensors).operate_ai`, so on a
    /// high-fidelity hull the ship's frequency advisory changed shape — timing,
    /// and across the delivery lag its content — according to who was holding
    /// the console, and could be silenced outright by the `auto_hint` rating
    /// gate on the other side. Now both origins take the same path.
    ///
    /// Asserted in both directions across two ticks (the second is where the
    /// on-change debounce would let a late emit through) and for both control
    /// sources, because a one-sided assertion would still pass with the
    /// `operate_ai` conjunct restored.
    #[test]
    fn high_fidelity_ships_hand_off_regardless_of_who_holds_sensors() {
        for source in [ControlSource::Human, ControlSource::Ai] {
            let mut app = test_app();
            start_game_with_sensors_and_tactical(&mut app);
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
                .single(app.world())
                .unwrap();
            app.world_mut()
                .entity_mut(ship)
                .insert(crate::ai_plugin::AiHighFidelity);
            {
                let mut cs = app
                    .world_mut()
                    .entity_mut(ship)
                    .take::<crate::ship_plugin::ShipSystemControlSources>()
                    .unwrap();
                cs.0.set(crate::system_registry::sensors_system_id(), source);
                app.world_mut().entity_mut(ship).insert(cs);
            }
            app.world_mut().resource_mut::<EnqueueLog>().0.clear();

            set_local_weapons_target(&mut app, Some("asteroid-1".into()));
            tick(&mut app);
            tick(&mut app);

            let log = app.world().resource::<EnqueueLog>();
            assert!(
                !log.0
                    .iter()
                    .any(|e| matches!(&e.payload, CoordinationPayload::FrequencyHint { .. })),
                "a high-fidelity hull's frequency hint belongs to \
                 `tick_frequency_hint_high_fidelity` whoever holds Sensors ({source:?}); \
                 emitting here too would double-send, and gating this skip on \
                 `operate_ai` is the origin branch issue #873 removed"
            );
        }
    }

    /// The other side of the same split: a hull with no `AiHighFidelity` marker
    /// is served HERE, and again regardless of origin — the immediate readout is
    /// not "the human path".
    #[test]
    fn low_fidelity_ships_emit_here_regardless_of_who_holds_sensors() {
        for source in [ControlSource::Human, ControlSource::Ai] {
            let mut app = test_app();
            start_game_with_sensors_and_tactical(&mut app);
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
                .single(app.world())
                .unwrap();
            {
                let mut cs = app
                    .world_mut()
                    .entity_mut(ship)
                    .take::<crate::ship_plugin::ShipSystemControlSources>()
                    .unwrap();
                cs.0.set(crate::system_registry::sensors_system_id(), source);
                app.world_mut().entity_mut(ship).insert(cs);
            }
            app.world_mut().resource_mut::<EnqueueLog>().0.clear();

            set_local_weapons_target(&mut app, Some("asteroid-1".into()));
            tick(&mut app);

            let log = app.world().resource::<EnqueueLog>();
            let hint = log
                .0
                .iter()
                .find(|e| matches!(&e.payload, CoordinationPayload::FrequencyHint { .. }))
                .unwrap_or_else(|| panic!("expected a FrequencyHint with Sensors on {source:?}"));
            assert_eq!(
                hint.sender_origin, source,
                "sender_origin must report the live control source and be used only as a \
                 delivery-routing tag"
            );
            assert_eq!(
                hint.target,
                crate::system_registry::tactical_station_key(),
                "the hint is addressed to Tactical either way"
            );
        }
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
            display_name: None,
            uuid: fed_uuid,
            name: "Federation".into(),
            enemies: vec![harrow_uuid],
            compliance: None,
        });
        reg.insert(crate::faction::FactionConfig {
            display_name: None,
            uuid: harrow_uuid,
            name: "Harrow".into(),
            enemies: vec![fed_uuid],
            compliance: None,
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
            // Always present in production (LobbyPlugin inserts it); the
            // default carries the same default_sensors_radar_range() base the
            // old Option fallback supplied, so test ranges are unchanged.
            .init_resource::<crate::lobby::server::ShipClientConfigResource>()
            // `emit_sensors_ai_command` validates through the shared
            // admission seam, which consults Sessions for human tokens; the
            // `ai:` path only needs the resource present.
            .insert_resource(crate::lobby::Sessions(
                crate::lobby::session::SessionManager::new(),
            ))
            // The applier emits the channel-3 TargetDesignation advisory.
            .add_message::<CoordinationEnqueue>()
            // Decide-and-emit (issue #828): the decision lands in
            // `AdmittedCommands` and `handle_sensors_messages` applies it —
            // chained so the same-tick emit→apply shape of production
            // (Input → Physics) holds in the harness.
            .add_systems(
                Update,
                (
                    seed_viewscreen_from_selection,
                    operate_sensors_ai,
                    handle_sensors_messages,
                )
                    .chain(),
            );

        let mut control_sources = crate::ship_plugin::ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Ai,
        );

        app.world_mut().spawn((
            crate::server_app::Ship,
            control_sources,
            crate::server_app::ShipSystemBlackboards::default(),
            SensorRadarSelection::default(),
            crate::simulation::TacticalRadarSelection::default(),
            // Sensors range-gate on the ship's own position and radar modifier,
            // so both must be present or the query silently matches nothing and
            // every assertion below passes vacuously.
            crate::ship_state::ShipPhysics::default(),
            crate::modifiers::ShipModifiers::default(),
            // Issue #828: the AI decision flows through this ship's own
            // AdmittedCommands, applied by handle_sensors_messages.
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            // The AUTHORED Sensors selector every shipped hull carries. Since
            // #885b stage 5d there is no synthesised stand-in inside
            // `operate_sensors_ai`, so a fixture that wants a ranking has to
            // attach the declaration a real hull authors — which is also what
            // makes these tests exercise shipped content rather than a Rust
            // default nobody wrote.
            SensorsTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("sensors")
                    .to_selector()
                    .expect("the shipped Sensors selector decodes"),
                power_rating: None,
            },
        ));

        app
    }

    /// Admitted sensors commands currently queued on the single test ship.
    fn admitted_sensors_payloads(app: &mut App) -> Vec<SystemControlPayload> {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::messages::AdmittedCommands, With<crate::server_app::Ship>>();
        q.single(app.world())
            .unwrap()
            .for_target(crate::system_registry::SENSORS_SYSTEM_ID)
            .map(|c| c.payload.clone())
            .collect()
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
                    text_params: Default::default(),
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

    /// Issue #891 stage 2, per-host both-directions proof for the Sensors
    /// target selector: an authored eligibility gated on a world flag selects
    /// nothing while the flag is clear and mirrors the combat lock once it is
    /// set.
    #[test]
    fn operate_sensors_ai_flag_guard_reads_the_world_in_both_directions() {
        let mut app = sensors_ai_test_app();
        let target = "cc000000-0000-0000-0000-0000000891aa";
        spawn_target_at(&mut app, target, 0.0, -30.0);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(target.to_string());
        }

        // Swap in a selector whose eligibility ALSO requires the world flag.
        let cfg = crate::entities::config::FineSystemAiSelectorToml {
            param: Default::default(),
            sources: vec!["combat-lock".into()],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(detectable) > 0 and flag(sensors_cleared)".into(),
            score: vec![crate::entities::config::ScoreTermToml {
                when: "candidate_fact(source_combat_lock) > 0".into(),
                weight: 1.0,
            }],
        };
        let ship = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::Ship>>();
            q.single(app.world()).unwrap()
        };
        app.world_mut()
            .entity_mut(ship)
            .insert(SensorsTargetSelector {
                selector: cfg
                    .to_selector()
                    .expect("flag-gated sensors selector decodes"),
                power_rating: None,
            });

        // Flag CLEAR → nothing is eligible, no science target.
        tick_sensors_ai(&mut app);
        assert_eq!(
            get_sensors_target(&mut app),
            None,
            "with the world flag clear the eligibility must admit no candidate"
        );

        // Flag SET → the SAME eligibility admits the combat-lock mirror.
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .flags
            .set_flag("sensors_cleared");
        tick_sensors_ai(&mut app);
        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target),
            "with the world flag set the same eligibility must select the target"
        );
    }

    /// Spawn a bare targetable entity at a world position. `operate_sensors_ai`
    /// range-gates on `Transform`, so a target without one is not detectable.
    fn spawn_target_at(app: &mut App, uuid: &str, x: f32, z: f32) {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.to_string()),
            Transform::from_xyz(x, 0.0, z),
        ));
    }

    fn get_sensors_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&SensorRadarSelection, With<crate::server_app::Ship>>();
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

        spawn_target_at(&mut app, &target_uuid, 20.0, 0.0);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
        }

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "sensors AI should mirror TacticalRadarSelection"
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
        spawn_target_at(&mut app, &target_uuid, 40.0, 0.0);

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "sensors AI should select named Destroy objective target"
        );
    }

    /// Naming a target in an objective says who to engage, not that the ship can
    /// see them. Before this gate the objective path resolved a name to a UUID
    /// and locked it at any distance, so AI ships tracked contacts thousands of
    /// units outside their own sensor range.
    #[test]
    fn ai_sensors_ignores_objective_target_beyond_sensor_range() {
        let mut app = sensors_ai_test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();

        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_viewscreen_objective(&mut app, "wave_1", 80.0);

        // Well beyond the default 500-unit console range the test ship uses.
        spawn_target_at(&mut app, &target_uuid, 5000.0, 0.0);

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app),
            None,
            "a named objective target outside sensor range must not be locked"
        );
    }

    /// An NPC must range-gate on its own `[ai_profile] sensor_range`, not on the
    /// local player's console config. Borrowing the player's reach is what let
    /// short-ranged Harrow hulls (sensor_range 120) see as far as the flagship.
    #[test]
    fn ai_sensors_uses_the_ships_own_ai_profile_range() {
        let mut app = sensors_ai_test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();

        {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::Ship>>();
            let ship = q.single(app.world()).unwrap();
            app.world_mut()
                .entity_mut(ship)
                .insert(crate::ai::server::AiProfile {
                    aggression: 0.8,
                    sensor_range: 120.0,
                    ..Default::default()
                });
        }

        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_viewscreen_objective(&mut app, "wave_1", 80.0);

        // Inside the player's 500 console range but outside this hull's 120.
        spawn_target_at(&mut app, &target_uuid, 300.0, 0.0);

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app),
            None,
            "NPC must use its own sensor_range, not the player's console range"
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
                    text_params: Default::default(),
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

        spawn_target_at(&mut app, &combat_uuid, 20.0, 0.0);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(combat_uuid.clone());
        }

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(combat_uuid.as_str()),
            "sensors AI should prefer TacticalRadarSelection over objective target"
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
        spawn_target_at(&mut app, &target_uuid, 30.0, 0.0);

        // TacticalRadarSelection names a UUID that no entity carries → existence check fails
        let dead_uuid = uuid::Uuid::new_v4().to_string();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(dead_uuid);
        }

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "sensors AI should fall through to objective when TacticalRadarSelection entity is gone"
        );
    }

    // ── Issue #828 tests: decide-and-emit through Admission ─────────────────

    /// The AI decision must land as an admitted `SetScienceTarget` in the
    /// ship's own `AdmittedCommands` (not a direct `SensorRadarSelection` write),
    /// and only on change — an unchanged decision emits nothing.
    #[test]
    fn ai_sensors_emits_admitted_set_science_target_on_change_only() {
        let mut app = sensors_ai_test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();

        spawn_target_at(&mut app, &target_uuid, 20.0, 0.0);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
        }

        tick_sensors_ai(&mut app);

        let payloads = admitted_sensors_payloads(&mut app);
        assert_eq!(
            payloads,
            vec![SystemControlPayload::SetScienceTarget {
                uuid: target_uuid.clone()
            }],
            "the AI decision must flow through AdmittedCommands"
        );
        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target_uuid.as_str()),
            "the applier must have applied the admitted command same-tick"
        );

        // Second tick, same decision: emit-on-change means no new command.
        // (This harness has no AdmissionPlugin, so AdmittedCommands is never
        // cleared — a re-emission would grow the queue.)
        tick_sensors_ai(&mut app);
        assert_eq!(
            admitted_sensors_payloads(&mut app).len(),
            1,
            "an unchanged decision must not re-emit an admitted command"
        );
    }

    /// When the decision changes from Some to None (target moved out of
    /// range, no objective fallback), the AI emits an admitted
    /// `ClearScienceTarget` and the applier clears the selection — matching
    /// the old direct `sensors_target.0 = None` write.
    #[test]
    fn ai_sensors_clears_selection_via_admitted_clear() {
        let mut app = sensors_ai_test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();

        spawn_target_at(&mut app, &target_uuid, 20.0, 0.0);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
        }
        tick_sensors_ai(&mut app);
        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(target_uuid.as_str())
        );

        // Move the target far beyond sensor range; the weapons mirror tier
        // fails its range gate and there is no objective fallback → None.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut Transform, With<crate::entity_spawner::EntityUuid>>();
            for mut tf in q.iter_mut(app.world_mut()) {
                tf.translation.x = 50_000.0;
            }
        }
        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app),
            None,
            "the admitted ClearScienceTarget must clear the selection"
        );
        assert!(
            admitted_sensors_payloads(&mut app)
                .iter()
                .any(|p| matches!(p, SystemControlPayload::ClearScienceTarget)),
            "the clear must flow through AdmittedCommands too"
        );
    }

    /// Human-held Sensors refuses the `ai:` emission at admission: the
    /// operate gate skips the ship, and even a direct emission attempt is
    /// rejected by `validate_and_admit` (operate_ai does not hold).
    #[test]
    fn human_held_sensors_refuses_the_ai_emission() {
        let mut app = sensors_ai_test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();

        // Flip Sensors to Human on the test ship.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0.set(
                crate::system_registry::sensors_system_id(),
                ControlSource::Human,
            );
        }
        spawn_target_at(&mut app, &target_uuid, 20.0, 0.0);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(target_uuid.clone());
        }

        tick_sensors_ai(&mut app);

        assert!(
            admitted_sensors_payloads(&mut app).is_empty(),
            "no ai: command may be admitted while a human holds Sensors"
        );
        assert_eq!(get_sensors_target(&mut app), None);

        // Belt and braces: the emit helper itself must be refused by the
        // admission predicate under Human control.
        let mut human_sources = crate::ship_plugin::ShipSystemControlSources::default();
        human_sources.0.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Human,
        );
        let sessions = crate::lobby::Sessions(crate::lobby::session::SessionManager::new());
        let mut admitted = crate::messages::AdmittedCommands::default();
        assert!(
            !emit_sensors_ai_command(
                None,
                SystemControlPayload::SetScienceTarget {
                    uuid: target_uuid.clone()
                },
                &human_sources,
                &sessions,
                None,
                &mut admitted,
            ),
            "validate_and_admit must reject the ai: token when Sensors is Human"
        );
        assert!(admitted.0.is_empty());
    }

    // ── Issue #828 tests: per-Ship publish ──────────────────────────────────

    /// Fetch a ship's published Sensors blackboard.
    fn sensors_bb_of(app: &App, entity: Entity) -> SensorsBlackboard {
        let bbs = app
            .world()
            .entity(entity)
            .get::<crate::server_app::ShipSystemBlackboards>()
            .expect("ShipSystemBlackboards");
        let key = SystemId(crate::system_registry::SENSORS_SYSTEM_ID.to_string());
        match bbs.0.get(&key).expect("Sensors blackboard") {
            SystemBlackboard::Sensors(bb) => bb.clone(),
            other => panic!("expected Sensors blackboard, got {other:?}"),
        }
    }

    /// Per-Ship publish (issue #828): an NPC gets its own Sensors blackboard —
    /// its own science target, its own AiProfile-derived radar range — while
    /// the player-only authored show/select filters stay gated on LocalShip.
    #[test]
    fn publish_writes_sensors_blackboards_for_every_ship_not_just_local() {
        let mut app = test_app();
        // Give the local config distinctive filters so the gating is visible.
        {
            let mut cfg = app
                .world_mut()
                .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
            cfg.0.sensors_radar_shows = vec!["ship".into()];
            cfg.0.sensors_radar_selects = vec!["hostile".into()];
        }
        let npc = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::server_app::ShipSystemBlackboards::default(),
                SensorRadarSelection(Some("npc-science-target".into())),
                crate::ai::server::AiProfile {
                    aggression: 0.5,
                    sensor_range: 120.0,
                    ..Default::default()
                },
            ))
            .id();
        app.update();

        let npc_bb = sensors_bb_of(&app, npc);
        assert_eq!(
            npc_bb.science_target_uuid.as_deref(),
            Some("npc-science-target"),
            "NPC blackboard must carry the NPC's own SensorRadarSelection"
        );
        assert!(
            (npc_bb.radar_range - 120.0).abs() < f32::EPSILON,
            "NPC radar_range must come from its own AiProfile.sensor_range, got {}",
            npc_bb.radar_range
        );
        assert!(
            npc_bb.radar_shows.is_empty() && npc_bb.radar_selects.is_empty(),
            "player-only authored filters must not leak onto NPC blackboards"
        );

        let local = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap()
        };
        let local_bb = sensors_bb_of(&app, local);
        assert_eq!(local_bb.radar_shows, vec!["ship".to_string()]);
        assert_eq!(local_bb.radar_selects, vec!["hostile".to_string()]);
        assert_eq!(
            local_bb.radar_range,
            crate::messages::default_sensors_radar_range(),
            "local ship keeps the console-config range"
        );
        assert_eq!(local_bb.science_target_uuid, None);
    }

    /// An NPC with no AiProfile falls back to the console-config base range
    /// (the same preference order as `effective_sensor_range`), scaled by its
    /// own SensorRadarRange modifier when present.
    #[test]
    fn publish_npc_without_profile_falls_back_to_console_range() {
        let mut app = test_app();
        let npc = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();
        app.update();

        let npc_bb = sensors_bb_of(&app, npc);
        assert_eq!(
            npc_bb.radar_range,
            crate::messages::default_sensors_radar_range(),
            "profile-less NPC falls back to the console config base range"
        );
        assert_eq!(
            npc_bb.science_target_uuid, None,
            "missing SensorRadarSelection publishes as no selection"
        );
    }

    // ── Issue #746 tests: independent horizon-limited hostile selection ──────

    /// Build on `sensors_ai_test_app` with a Federation/Harrow faction registry
    /// and give the single test ship the Federation faction, so the tier-3
    /// nearest-hostile selector can judge who is an enemy.
    fn sensors_ai_test_app_with_factions() -> (App, uuid::Uuid, uuid::Uuid) {
        let mut app = sensors_ai_test_app();

        let fed = uuid::Uuid::new_v4();
        let harrow = uuid::Uuid::new_v4();
        let mut reg = crate::faction::FactionRegistry::new();
        reg.insert(crate::faction::FactionConfig {
            display_name: None,
            uuid: fed,
            name: "Federation".into(),
            enemies: vec![harrow],
            compliance: None,
        });
        reg.insert(crate::faction::FactionConfig {
            display_name: None,
            uuid: harrow,
            name: "Harrow".into(),
            enemies: vec![fed],
            compliance: None,
        });
        app.insert_resource(crate::entities::config_cache::FactionRegistryResource(reg));

        let ship = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::Ship>>();
            q.single(app.world()).unwrap()
        };
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::entity_spawner::FactionComponent(fed));

        (app, fed, harrow)
    }

    /// Spawn a faction-bearing, targetable contact with a *parseable* UUID (the
    /// nearest-hostile scan filters out ids that are not canonical UUIDs).
    fn spawn_faction_contact(app: &mut App, uuid: &str, x: f32, z: f32, faction: uuid::Uuid) {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.to_string()),
            Transform::from_xyz(x, 0.0, z),
            crate::entity_spawner::FactionComponent(faction),
        ));
    }

    /// Fetch a specific ship's science selection by entity UUID (the single-ship
    /// `get_sensors_target` helper cannot disambiguate a two-ship world).
    fn selection_of(app: &mut App, uuid: &str) -> Option<String> {
        let mut q = app
            .world_mut()
            .query::<(&crate::entity_spawner::EntityUuid, &SensorRadarSelection)>();
        q.iter(app.world())
            .find(|(u, _)| u.0 == uuid)
            .and_then(|(_, s)| s.0.clone())
    }

    /// Tier 3 selects a hostile only — a closer ally or neutral contact is never
    /// designated. AC: hostility.
    #[test]
    fn ai_sensors_independently_selects_nearest_hostile_only() {
        let (mut app, fed, harrow) = sensors_ai_test_app_with_factions();
        let neutral_faction = uuid::Uuid::new_v4(); // not an enemy of Federation

        let ally = uuid::Uuid::new_v4().to_string();
        let neutral = uuid::Uuid::new_v4().to_string();
        let enemy = uuid::Uuid::new_v4().to_string();
        // Ally and neutral are *closer* than the enemy: proximity must not win
        // over hostility.
        spawn_faction_contact(&mut app, &ally, 10.0, 0.0, fed);
        spawn_faction_contact(&mut app, &neutral, 20.0, 0.0, neutral_faction);
        spawn_faction_contact(&mut app, &enemy, 60.0, 0.0, harrow);

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(enemy.as_str()),
            "sensors AI must independently pick the hostile, not the closer ally/neutral"
        );
        // And the selection reaches Tactical through the normal admitted path.
        assert!(
            admitted_sensors_payloads(&mut app).iter().any(|p| matches!(
                p,
                SystemControlPayload::SetScienceTarget { uuid } if uuid == &enemy
            )),
            "the independent selection must flow through an admitted SetScienceTarget"
        );
    }

    /// The independent tier is the *fallback*: an in-range combat lock (tier 1)
    /// still wins over a nearest hostile. Tactical authority is not displaced —
    /// Sensors mirrors what Tactical designates.
    #[test]
    fn ai_sensors_combat_lock_outranks_independent_hostile() {
        let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
        let locked = uuid::Uuid::new_v4().to_string();
        let nearer_hostile = uuid::Uuid::new_v4().to_string();

        // A nearer hostile the independent tier would otherwise choose…
        spawn_faction_contact(&mut app, &nearer_hostile, 15.0, 0.0, harrow);
        // …and a farther hostile that Tactical has locked.
        spawn_faction_contact(&mut app, &locked, 80.0, 0.0, harrow);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::simulation::TacticalRadarSelection, With<crate::server_app::Ship>>();
            q.single_mut(app.world_mut()).unwrap().0 = Some(locked.clone());
        }

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(locked.as_str()),
            "combat-lock mirror must outrank the independent nearest-hostile fallback"
        );
    }

    /// Damaging the sensor-radar hull system shrinks the `SensorRadarRange`
    /// modifier, collapsing the horizon inward until a previously-visible
    /// hostile falls outside it and is dropped. AC: horizon damage scaling.
    #[test]
    fn ai_sensors_drops_hostile_when_sensor_radar_damage_shrinks_horizon() {
        use crate::damage::{ConsoleTierConfig, SystemHull};
        use crate::entity_spawner::EntitySystemHull;
        use crate::system_registry::sensor_radar_system_id;
        use bevy::ecs::system::RunSystemOnce;

        let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
        let enemy = uuid::Uuid::new_v4().to_string();
        // Inside the healthy ~500 console horizon, but beyond the ~417 horizon a
        // Damaged sensor-radar leaves (500 × 1/1.2 ≈ 417).
        spawn_faction_contact(&mut app, &enemy, 450.0, 0.0, harrow);

        tick_sensors_ai(&mut app);
        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(enemy.as_str()),
            "a hostile at 450 is inside the undamaged horizon"
        );

        // Damage the sensor-radar to the Damaged tier (10/20 HP, below the 75%
        // threshold) and re-run the real damage→modifier translator.
        let tier_config = ConsoleTierConfig {
            damaged_threshold_pct: 0.75,
            disabled_threshold_pct: 0.25,
            debuff_magnitude: 0.20,
        };
        let ship = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::Ship>>();
            q.single(app.world()).unwrap()
        };
        let mut hull =
            SystemHull::from_config_with_tiers(&[(sensor_radar_system_id(), 20.0, tier_config)]);
        hull.set_hp(&sensor_radar_system_id(), 10.0);
        app.world_mut()
            .entity_mut(ship)
            .insert(EntitySystemHull(hull));
        app.world_mut()
            .run_system_once(crate::modifiers::coordination::apply_radar_damage_modifiers)
            .unwrap();

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app),
            None,
            "the damage-shrunken horizon must drop the now-out-of-range hostile"
        );
        assert!(
            admitted_sensors_payloads(&mut app)
                .iter()
                .any(|p| matches!(p, SystemControlPayload::ClearScienceTarget)),
            "the drop must flow through an admitted ClearScienceTarget"
        );
    }

    /// A designated hostile that despawns is dropped via an admitted
    /// `ClearScienceTarget`. AC: target loss.
    #[test]
    fn ai_sensors_clears_when_selected_hostile_despawns() {
        let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
        let enemy = uuid::Uuid::new_v4().to_string();
        spawn_faction_contact(&mut app, &enemy, 100.0, 0.0, harrow);

        tick_sensors_ai(&mut app);
        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(enemy.as_str())
        );

        // Despawn the hostile.
        let hostile_entity = {
            let mut q = app
                .world_mut()
                .query::<(Entity, &crate::entity_spawner::EntityUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == enemy)
                .map(|(e, _)| e)
                .unwrap()
        };
        app.world_mut().entity_mut(hostile_entity).despawn();

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app),
            None,
            "a despawned hostile must be cleared"
        );
        assert!(
            admitted_sensors_payloads(&mut app)
                .iter()
                .any(|p| matches!(p, SystemControlPayload::ClearScienceTarget)),
            "target loss must flow through an admitted ClearScienceTarget"
        );
    }

    /// Two AI Sensors ships each select their own nearest hostile, gated by
    /// their own position and horizon — no cross-ship leakage. AC: per-ship
    /// isolation.
    #[test]
    fn ai_sensors_two_ships_select_their_own_hostiles() {
        let (mut app, fed, harrow) = sensors_ai_test_app_with_factions();

        // Ship A is the helper's ship, sitting at the origin. Give it an id.
        let ship_a_uuid = uuid::Uuid::new_v4().to_string();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::Ship>>();
            let ship_a = q.single(app.world()).unwrap();
            app.world_mut()
                .entity_mut(ship_a)
                .insert(crate::entity_spawner::EntityUuid(ship_a_uuid.clone()));
        }

        // Ship B, same faction, 1000 units away on +X.
        let ship_b_uuid = uuid::Uuid::new_v4().to_string();
        let mut control_sources = crate::ship_plugin::ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Ai,
        );
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::entity_spawner::EntityUuid(ship_b_uuid.clone()),
            control_sources,
            crate::server_app::ShipSystemBlackboards::default(),
            SensorRadarSelection::default(),
            crate::simulation::TacticalRadarSelection::default(),
            crate::ship_state::ShipPhysics {
                x: 1000.0,
                ..Default::default()
            },
            crate::modifiers::ShipModifiers::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::entity_spawner::FactionComponent(fed),
            // Ship B needs its own authored selector, same as ship A: the
            // declaration is per-entity and there is no synthesised fallback.
            SensorsTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("sensors")
                    .to_selector()
                    .expect("the shipped Sensors selector decodes"),
                power_rating: None,
            },
        ));

        // A hostile beside each ship; each lies far outside the other's horizon.
        let enemy_a = uuid::Uuid::new_v4().to_string();
        let enemy_b = uuid::Uuid::new_v4().to_string();
        spawn_faction_contact(&mut app, &enemy_a, 50.0, 0.0, harrow);
        spawn_faction_contact(&mut app, &enemy_b, 1050.0, 0.0, harrow);

        tick_sensors_ai(&mut app);

        assert_eq!(
            selection_of(&mut app, &ship_a_uuid).as_deref(),
            Some(enemy_a.as_str()),
            "ship A must pick the hostile inside its own horizon"
        );
        assert_eq!(
            selection_of(&mut app, &ship_b_uuid).as_deref(),
            Some(enemy_b.as_str()),
            "ship B must pick its own hostile — no cross-ship leakage"
        );
    }

    // ── Issue #776 tests: data-driven selector + authored power rating ───────

    /// AC2/AC7: an authored selector gates eligibility on the ship's own
    /// authored power rating via `self_fact(power_rating)`. An under-rated ship
    /// selects nothing even with a hostile in horizon; raising the rating makes
    /// the same contact eligible and the pick flows through an admitted
    /// `SetScienceTarget` (an observable output).
    #[test]
    fn ai_sensors_selector_gates_on_authored_power_rating() {
        let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
        let enemy = uuid::Uuid::new_v4().to_string();
        spawn_faction_contact(&mut app, &enemy, 60.0, 0.0, harrow);

        let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
        cfg.param.insert("min_rating".into(), 5.0);
        cfg.eligibility = "candidate_fact(detectable) > 0 and candidate_fact(hostile) > 0 \
             and self_fact(power_rating) >= param(min_rating)"
            .into();
        let selector = cfg.to_selector().unwrap();

        let ship = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::Ship>>();
            q.single(app.world()).unwrap()
        };

        // Under-rated: nothing eligible.
        app.world_mut()
            .entity_mut(ship)
            .insert(SensorsTargetSelector {
                selector: selector.clone(),
                power_rating: Some(3.0),
            });
        tick_sensors_ai(&mut app);
        assert_eq!(
            get_sensors_target(&mut app),
            None,
            "an under-rated ship must select nothing under the authored gate"
        );

        // Sufficiently rated: the same contact is now eligible and selected.
        app.world_mut()
            .entity_mut(ship)
            .insert(SensorsTargetSelector {
                selector,
                power_rating: Some(6.0),
            });
        tick_sensors_ai(&mut app);
        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(enemy.as_str()),
            "raising the rating above the floor makes the contact eligible"
        );
        assert!(
            admitted_sensors_payloads(&mut app).iter().any(|p| matches!(
                p,
                SystemControlPayload::SetScienceTarget { uuid } if uuid == &enemy
            )),
            "the selection must flow through an admitted SetScienceTarget"
        );
    }

    /// AC6: a selected contact drives the existing advisory `TargetDesignation`
    /// on the channel-3 bus for Tactical — the same applier a human selection
    /// uses. Here an AI ship independently designates its nearest hostile.
    #[test]
    fn ai_sensors_selection_drives_target_designation_advisory() {
        use crate::messages::CoordinationPayload;
        let (mut app, _fed, harrow) = sensors_ai_test_app_with_factions();
        // The advisory is emitted by `handle_sensors_messages`; add a sink.
        app.init_resource::<EnqueueLog>().add_systems(
            bevy::app::Update,
            collect_enqueues.after(handle_sensors_messages),
        );
        let enemy = uuid::Uuid::new_v4().to_string();
        spawn_faction_contact(&mut app, &enemy, 60.0, 0.0, harrow);

        tick_sensors_ai(&mut app);

        assert_eq!(
            get_sensors_target(&mut app).as_deref(),
            Some(enemy.as_str())
        );
        let log = app.world().resource::<EnqueueLog>();
        assert!(
            log.0.iter().any(|e| matches!(
                &e.payload,
                CoordinationPayload::TargetDesignation { uuid, .. } if uuid == &enemy
            )),
            "the AI selection must designate its contact to Tactical (channel-3 advisory)"
        );
    }

    // ── publish_sensor_radar_blackboard: selected-target alert (issue #749) ──────

    /// Minimal app that runs only `publish_sensor_radar_blackboard`, plus a
    /// scanning ship whose `SensorRadarSelection` we drive directly. Returns the
    /// scanning ship's `Entity` so the caller can read back its blackboard.
    fn alert_publisher_app() -> (App, Entity) {
        let mut app = App::new();
        app.add_systems(Update, publish_sensor_radar_blackboard);
        let scanner = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::server_app::ShipSystemBlackboards::default(),
                SensorRadarSelection::default(),
            ))
            .id();
        (app, scanner)
    }

    /// Read the `selected_target_alert` replica off a ship's sensor-radar blackboard.
    fn published_alert(app: &App, ship: Entity) -> Option<bool> {
        match app
            .world()
            .entity(ship)
            .get::<crate::server_app::ShipSystemBlackboards>()
            .and_then(|bbs| {
                bbs.0
                    .get(&crate::system_registry::sensor_radar_system_id())
                    .cloned()
            }) {
            Some(SystemBlackboard::SensorRadar(bb)) => bb.selected_target_alert,
            _ => panic!("sensor-radar blackboard missing"),
        }
    }

    fn set_selection(app: &mut App, ship: Entity, uuid: Option<&str>) {
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<SensorRadarSelection>()
            .unwrap()
            .0 = uuid.map(|s| s.to_string());
    }

    #[test]
    fn sensor_radar_alert_none_when_no_selection() {
        let (mut app, scanner) = alert_publisher_app();
        app.update();
        assert_eq!(
            published_alert(&app, scanner),
            None,
            "no selection → no alert field"
        );
    }

    #[test]
    fn sensor_radar_alert_reports_selected_ship_red_alert() {
        let (mut app, scanner) = alert_publisher_app();
        // A capable ship target, currently at red alert.
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::entity_spawner::EntityUuid("enemy-1".into()),
            crate::ship_state::ShipRedAlert(true),
        ));
        set_selection(&mut app, scanner, Some("enemy-1"));
        app.update();
        assert_eq!(
            published_alert(&app, scanner),
            Some(true),
            "selected capable ship at red alert → Some(true)"
        );
    }

    #[test]
    fn sensor_radar_alert_reports_capable_but_calm_ship() {
        let (mut app, scanner) = alert_publisher_app();
        // Capable but not alerted — the distinct Some(false) case.
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::entity_spawner::EntityUuid("enemy-2".into()),
            crate::ship_state::ShipRedAlert(false),
        ));
        set_selection(&mut app, scanner, Some("enemy-2"));
        app.update();
        assert_eq!(
            published_alert(&app, scanner),
            Some(false),
            "selected capable ship not at red alert → Some(false)"
        );
    }

    #[test]
    fn sensor_radar_alert_none_for_non_ship_target() {
        let (mut app, scanner) = alert_publisher_app();
        // An asteroid: carries a uuid but NOT ShipRedAlert and NOT the Ship
        // marker → no capability → no alert field (the no-leak boundary).
        app.world_mut()
            .spawn(crate::entity_spawner::EntityUuid("asteroid-9".into()));
        set_selection(&mut app, scanner, Some("asteroid-9"));
        app.update();
        assert_eq!(
            published_alert(&app, scanner),
            None,
            "non-ship contact has no red-alert capability → no alert field"
        );
    }
}

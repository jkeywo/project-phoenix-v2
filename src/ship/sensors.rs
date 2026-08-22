use crate::simmath;
use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::core::messages::{
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
            crate::ship::system_registry::SENSORS_SYSTEM_ID,
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
            &crate::core::messages::AdmittedCommands,
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
        for cmd in admitted.for_target(crate::ship::system_registry::SENSORS_SYSTEM_ID) {
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
                .source_for(&crate::ship::system_registry::sensors_system_id());

            writer.write(CoordinationEnqueue {
                source_entity: entity,
                sender_origin,
                target: crate::ship::system_registry::tactical_station_key(),
                payload: CoordinationPayload::TargetDesignation {
                    uuid: uuid.clone(),
                    label,
                },
                sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
                sender_system: crate::ship::system_registry::sensors_system_id(),
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
            Has<crate::ai::server::AiHighFidelity>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut writer: MessageWriter<CoordinationEnqueue>,
    target_shields_q: Query<(
        &crate::entities::spawner::EntityUuid,
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
            .get(&crate::ship::system_registry::viewscreen_system_id())
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
            .source_for(&crate::ship::system_registry::sensors_system_id());

        writer.write(CoordinationEnqueue {
            source_entity: entity,
            sender_origin,
            target: crate::ship::system_registry::tactical_station_key(),
            payload: CoordinationPayload::FrequencyHint { frequency },
            sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
            sender_system: crate::ship::system_registry::sensors_system_id(),
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
            &crate::entities::spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        Without<crate::server_app::Ship>,
    >,
    ship_positions: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &crate::ship::state::ShipPhysics,
            &crate::entities::spawner::FactionComponent,
        ),
        With<crate::server_app::Ship>,
    >,
    mut ships: Query<
        (
            Entity,
            &crate::entities::spawner::EntityUuid,
            &crate::ship::state::ShipPhysics,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut SensorsThreatState,
            &crate::modifiers::ShipModifiers,
            Option<&crate::entities::spawner::FactionComponent>,
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
            if !crate::ai::faction::is_enemy(self_f_uuid, Some(*other_f), reg) {
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
            .source_for(&crate::ship::system_registry::sensors_system_id());

        writer.write(CoordinationEnqueue {
            source_entity: entity,
            sender_origin,
            target: crate::ship::system_registry::shields_system_id(),
            payload: CoordinationPayload::ThreatBearing {
                bearing_rad: relative_bearing,
                label,
            },
            sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
            sender_system: crate::ship::system_registry::sensors_system_id(),
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
            SystemId(crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string()),
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
            &crate::entities::spawner::EntityUuid,
            &crate::ship::state::ShipRedAlert,
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
            crate::ship::system_registry::sensor_radar_system_id(),
            SystemBlackboard::SensorRadar(crate::core::messages::SensorRadarBlackboard {
                selected_target,
                selected_target_alert,
            }),
        );
    }
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
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::DETECTABLE, 1.0);
    facts.set_fact(fid::HOSTILE, 1.0);
    // `source_fact` is a `SOURCE_*` catalogue constant's `.name()` (issue #1210).
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
///      through the same [`crate::command_admission::ai_emit::emit_ai_command`]
///      → `handle_sensors_messages` applier a human selection does, which never mutates
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
/// `ClearScienceTarget` through [`crate::command_admission::ai_emit::emit_ai_command`], for
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
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &SensorRadarSelection,
            &crate::ship::state::ShipPhysics,
            &crate::modifiers::ShipModifiers,
            Option<&crate::ai::server::AiProfile>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::entities::spawner::FactionComponent>,
            Option<&SensorsTargetSelector>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
    // Plain `Res` (issue #828): `LobbyPlugin` always inserts
    // `ShipClientConfigResource` and `tick_sensors_threat_warning` already
    // requires it, so the old `Option<Res<..>>` arm was dead optionality.
    // It remains the *fallback* of the per-entity preference order inside
    // `effective_sensor_range` — the legitimate local-player/profile-less arm.
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    entity_q: Query<(
        &crate::entities::spawner::EntityUuid,
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
            &crate::entities::spawner::EntityUuid,
            &crate::ship::state::ShipPhysics,
            &crate::entities::spawner::FactionComponent,
        ),
        With<crate::server_app::Ship>,
    >,
    hostile_entity_q: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::FactionComponent>,
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
        // Control-Source gate through the shared AI host spine (issue #1208): a
        // human holder (or an offline system) stands the selector down. Sensors
        // resolves a data-driven SELECTOR the spine does not model, so only its
        // gate — the one step it shares with the policy hosts — routes here.
        if !crate::ai::host::ai_operates(
            &sources.0,
            crate::ship::system_registry::sensors_system_id(),
        ) {
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
            .get(&crate::ship::system_registry::viewscreen_system_id());
        if let Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) = viewscreen_bb {
            // Source: combat-lock — mirror Tactical's designated firing target.
            if let Some(target_uuid) = bb.combat_lock.as_deref() {
                if let Some(pos) = entity_q.iter().find_map(|(u, _, tf)| {
                    (u.0 == *target_uuid && in_range(tf)).then(|| tf.translation.to_array())
                }) {
                    candidates.push(detectable_candidate(
                        target_uuid,
                        pos,
                        crate::entities::ai_flag_hosts::SOURCE_COMBAT_LOCK.name(),
                    ));
                }
            }

            // Source: objective-destroy — named Destroy targets in the pool.
            // Resolving a name is not the same as seeing the ship, so each
            // candidate is still gated on the live horizon.
            for objective in bb.scored_objectives.iter().filter(|o| o.score > 0.0) {
                if let crate::core::messages::AiDirective::Destroy { target } = &objective.directive
                {
                    if target.is_empty() {
                        continue;
                    }
                    let uuid = Some(ai_env.content_runtime())
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
                            candidates.push(detectable_candidate(
                                &uuid,
                                pos,
                                crate::entities::ai_flag_hosts::SOURCE_OBJECTIVE.name(),
                            ));
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
                    candidates.push(detectable_candidate(
                        u,
                        *pos,
                        crate::entities::ai_flag_hosts::SOURCE_RADAR.name(),
                    ));
                }
            }
        }

        // Self context: position (horizon filter) + authored power rating,
        // exposed to the selector expressions as `self_fact(power_rating)` (AC2).
        let mut self_facts = crate::world::flags::AiFacts::new();
        if let Some(pr) = selector_comp.power_rating {
            self_facts.set_fact(crate::entities::ai_flag_hosts::POWER_RATING, pr as f64);
        }
        let self_ctx = SelfContext {
            position: [physics.x, 0.0, physics.z],
            facts: self_facts,
        };

        // Retain the current selection through the authored switch margin (AC3);
        // an invalid current target fails eligibility and is replaced this same
        // tick (AC4). The scenario flag chain is anchored at the layer that
        // spawned this ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(ship_entity);
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
            Some(uuid) => crate::core::messages::SystemControlPayload::SetScienceTarget { uuid },
            None => crate::core::messages::SystemControlPayload::ClearScienceTarget,
        };
        emit_ai_command(
            entity_uuid,
            crate::ship::system_registry::sensors_system_id(),
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
#[path = "sensors_tests.rs"]
mod tests;

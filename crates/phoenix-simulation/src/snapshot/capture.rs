//! Capture.
use super::*;

/// Copy one live dialogue into a [`DialogueState`].
///
/// Infallible since issue #985: the only node shape left is the one
/// `project_node` builds, whose responses are `(text, important)`. It used to
/// return `None` for a declarative node whose responses carried `TriggerAction`s
/// or a nested `follow_up` - authored-config trees whose serialisation is the
/// commitment [`ScenarioState`] refuses for `pending_delayed_actions` - and the
/// front-end that could author one is gone.
pub(super) fn reduce_dialogue_node(message_id: &str, dialogue: &ActiveDialogue) -> DialogueState {
    let node = &dialogue.current_node;
    DialogueState {
        message_id: message_id.to_string(),
        thread_id: dialogue.thread_id.clone(),
        body: node.body.clone(),
        body_params: node.body_params.clone(),
        responses: node
            .responses
            .iter()
            .map(|r| (r.text.clone(), r.important))
            .collect(),
        // Skipped entirely for a node that authors none, so no existing world's
        // payload gains a field it has no use for.
        response_ai: if node.responses.iter().any(|r| r.ai != Default::default()) {
            node.responses.iter().map(|r| r.ai).collect()
        } else {
            Vec::new()
        },
        script: dialogue.script.clone(),
    }
}

// ── Capture ──────────────────────────────────────────────────────────────────

/// Walk a live world and take its authoritative state.
///
/// Takes `&World`, not `&mut World`, for the same reason `world_digest` does:
/// capturing must not perturb the run it is capturing. Every read goes through
/// `get_resource`/`try_query`, so a bare-`App` fixture with half the world
/// unregistered produces a partial payload rather than a panic.
///
/// Call this between `App::update()` calls — outside `SimSet`, at a tick
/// boundary. `SimRng::state`'s own docs say why: mid-tick, some systems for the
/// step have drawn and others have not, so "all six streams right now" is not a
/// point any system agrees on.
pub fn capture(world: &World) -> PhoenixSnapshot {
    super::assert_ports_settled(world);
    let paused = world
        .get_resource::<crate::gm::actions::action::SimulationPaused>()
        .is_some_and(|paused| paused.0);
    let gm_actions = world
        .get_resource::<crate::gm::actions::action::GmActionJournal>()
        .cloned()
        .unwrap_or_default();
    PhoenixSnapshot {
        personnel: world
            .get_resource::<phoenix_sim_gameplay::personnel::PersonnelRoster>()
            .cloned()
            .unwrap_or_default(),
        tick: world.get_resource::<SimTick>().map_or(0, |t| t.0),
        // The ROSTER is the identity: it is replicated and every peer agrees on
        // it, while `SelectedShipResource` is this peer's own local choice and
        // is absent by design on a stationless GM peer (issue #1445). Requiring
        // both would have meant a GM's own bookmark carried no boot identity at
        // all, and so came straight back out of its catalogue as unreadable.
        boot_identity: world
            .get_resource::<crate::lockstep::FleetRoster>()
            .map(|fleet| BootIdentity {
                selected_ship: world
                    .get_resource::<crate::lobby::SelectedShipResource>()
                    .map(|selected| selected.0.clone())
                    .unwrap_or_default(),
                fleet: fleet.clone(),
                game_start_entity_uuids: world
                    .get_resource::<crate::server_app::GameStartEntityUuids>()
                    .map(|uuids| uuids.0.clone())
                    .unwrap_or_default(),
            }),
        fixed_overstep_nanos: world
            .get_resource::<Time<bevy::time::Fixed>>()
            .and_then(|time| u64::try_from(time.overstep().as_nanos()).ok()),
        paused,
        gm_actions,
        gm_puppets: world
            .get_resource::<crate::gm::operations::puppet::StationPuppets>()
            .cloned()
            .unwrap_or_default(),
        gm_station_commands: world
            .get_resource::<crate::gm::operations::puppet::PendingGmStationCommands>()
            .cloned()
            .unwrap_or_default(),
        gm_direct_effects: world
            .get_resource::<crate::gm::operations::effect::PendingGmDirectEffects>()
            .cloned()
            .unwrap_or_default(),
        gm_faction_overrides: world
            .get_resource::<crate::gm::operations::faction::GmFactionOverrides>()
            .cloned()
            .unwrap_or_default(),
        gm_spawn_exposure: world
            .get_resource::<crate::gm::operations::exposure::GmSpawnExposure>()
            .cloned()
            .unwrap_or_default(),
        gm_idle_npcs: world
            .get_resource::<crate::gm::observation::attention::GmIdleNpcWatch>()
            .cloned()
            .unwrap_or_default(),
        gm_station_workload: world
            .get_resource::<crate::gm::observation::workload::GmWorkloadWatch>()
            .cloned()
            .unwrap_or_default(),
        rng: world.get_resource::<SimRng>().map(SimRng::state),
        objective_records: world
            .get_resource::<crate::world::server::ObjectiveManagerRes>()
            .map(|manager| manager.0.records().to_vec())
            .unwrap_or_default(),
        objective_instances: world
            .get_resource::<crate::world::server::ObjectiveInstanceManagerRes>()
            .map(|manager| manager.0.clone())
            .unwrap_or_default(),
        frozen_ship_slots: world
            .get_resource::<crate::ship_slots::FrozenShipSlots>()
            .cloned(),
        mint: world.get_resource::<WorldIdMint>().map(WorldIdMint::state),
        phase: world
            .get_resource::<State<GamePhase>>()
            .map(|s| s.get().clone()),
        game_over: world
            .get_resource::<GameOverReason>()
            .map(|reason| (reason.0.clone(), reason.1.map(|o| o.as_str().to_string()))),
        mission_report: world
            .get_resource::<crate::core::report::MissionReport>()
            .map(|report| {
                report
                    .rows()
                    .iter()
                    .map(|row| SnapshotReportRow {
                        id: row.id.clone(),
                        heading: row.heading_id.clone(),
                        outcome: row.outcome_id.clone(),
                        state: row.state.as_str().to_string(),
                        score: row.score,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ai_world: world
            .get_resource::<crate::ai::server::WorldSnapshot>()
            .map(|snapshot| snapshot.entities.clone()),
        captain_boosts: world
            .get_resource::<CaptainPriorityBoost>()
            .map(|boosts| {
                boosts
                    .boosts_sorted()
                    .into_iter()
                    .map(|(scope, objective)| (scope.to_string(), objective.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        world: world.get_resource::<WorldResource>().map(|w| w.0.clone()),
        tracked_entities: world
            .get_resource::<crate::server_app::TrackedEntities>()
            .map(|tracked| {
                let mut reported: Vec<String> = tracked.reported.iter().cloned().collect();
                reported.sort();
                TrackedEntitiesState {
                    reported,
                    seeded: tracked.seeded,
                }
            }),
        flags: world
            .get_resource::<WorldContentRuntime>()
            .map(|rt| rt.flags.clone()),
        layer_flags: capture_layer_flags(world),
        scenario: capture_scenario(world),
        comms: capture_comms(world),
        ai_policy_clock: world
            .get_resource::<crate::ship::helm_ai::AiPolicyTickClock>()
            .map(|clock| clock.0),
        npc_frequency_matches: capture_npc_frequency_matches(world),
        phaser_mode: world
            .get_resource::<crate::console::weapons::CurrentPhaserMode>()
            .map(|mode| mode.0),
        coordination_staging: capture_coordination_staging(world),
        entities: capture_entities(world),
        asteroids: capture_asteroids(world),
        asteroid_window: capture_asteroid_window(world),
        collisions: capture_collisions(world),
    }
}

pub(super) fn capture_layer_flags(world: &World) -> Vec<LayerFlags> {
    let Some(layers) = world.get_resource::<crate::world::server::WorldLayerMap>() else {
        return Vec::new();
    };
    let mut active: Vec<_> = layers
        .0
        .iter()
        .filter(|(_, runtime)| runtime.is_active)
        .collect();
    active.sort_by(|(path_a, runtime_a), (path_b, runtime_b)| {
        runtime_a
            .activation_order
            .cmp(&runtime_b.activation_order)
            .then_with(|| path_a.cmp(path_b))
    });
    active
        .into_iter()
        .map(|(path, runtime)| LayerFlags {
            path: path.clone(),
            loader_path: runtime.loader_path.clone(),
            declared_entity_uuids: runtime
                .spawned_entities
                .iter()
                .map(|entity| {
                    if world
                        .get::<crate::entities::spawner::EntitySpawnOrigin>(*entity)
                        .is_some()
                    {
                        None
                    } else {
                        world.get::<EntityUuid>(*entity).map(|uuid| uuid.0.clone())
                    }
                })
                .collect(),
            flags: runtime.flags.clone(),
            owned_objective_ids: runtime.owned_objective_ids.clone(),
        })
        .collect()
}

/// The fixed-step clock the mission anchor is a reading of.
///
/// `Time<Fixed>` explicitly, not the context-sensitive `Time`: `anchor_mission_clock`
/// runs inside `FixedUpdate`, where `Time` *is* `Time<Fixed>`, and a capture taken
/// between `App::update()` calls would read `Time<Virtual>` instead — two clocks
/// that disagree by up to a timestep, which is exactly the drift issue #960 spent
/// a whole system removing from this anchor.
pub(super) fn fixed_elapsed_secs(world: &World) -> Option<f32> {
    world
        .get_resource::<Time<bevy::time::Fixed>>()
        .map(|t| t.elapsed_secs())
}

/// Project the NPC frequency-match resource through stable ship identities.
///
/// Returning `Some` whenever the resource exists is deliberate: an empty map
/// is authoritative continuation state and must clear a fresh bootstrap's
/// newly-started timers rather than inherit them.
pub(super) fn capture_npc_frequency_matches(world: &World) -> Option<NpcFrequencyMatchStatesState> {
    let runtime = world.get_resource::<crate::console::weapons::NpcFrequencyMatchStates>()?;
    let mut states: Vec<_> = runtime
        .continuation()
        .iter()
        .filter_map(|(entity, state)| {
            let uuid = world.get::<EntityUuid>(*entity)?;
            let (current_target, elapsed_secs, match_sent) = state.continuation();
            Some((
                uuid.0.clone(),
                NpcFrequencyMatchState {
                    current_target: current_target.map(str::to_owned),
                    elapsed_secs,
                    match_sent,
                },
            ))
        })
        .collect();
    states.sort_by(|a, b| a.0.cmp(&b.0));
    Some(NpcFrequencyMatchStatesState { states })
}

/// Project the exact unread suffix of the Coordination enqueue channel.
///
/// Cloning the authoritative cursor is load-bearing: reading a fresh cursor
/// would include already-consumed events still retained in Bevy's older
/// buffer, while reading the live cursor would mutate the run being captured.
pub(super) fn capture_coordination_staging(world: &World) -> Vec<CoordinationEnqueueState> {
    let Some(messages) =
        world.get_resource::<Messages<crate::ship::components::CoordinationEnqueue>>()
    else {
        return Vec::new();
    };
    let Some(cursor) = world.get_resource::<crate::ship::components::CoordinationEnqueueCursor>()
    else {
        return Vec::new();
    };
    let mut unread = cursor.0.clone();
    unread
        .read(messages)
        .filter_map(|message| {
            let source_uuid = world.get::<EntityUuid>(message.source_entity)?.0.clone();
            Some(CoordinationEnqueueState {
                source_uuid,
                sender_origin: match message.sender_origin {
                    crate::ship::control_source::ControlSource::Human => 0,
                    crate::ship::control_source::ControlSource::Ai => 1,
                    crate::ship::control_source::ControlSource::Offline => 2,
                    crate::ship::control_source::ControlSource::Simplified => 3,
                },
                address: message.address.clone(),
                payload: message.payload.clone(),
                presentation: message.presentation.clone(),
                sender_label: message.sender_label.clone(),
                sender_system: message.sender_system.0.clone(),
            })
        })
        .collect()
}

/// Walk the scenario's progression state — see [`ScenarioState`].
pub(super) fn capture_scenario(world: &World) -> Option<ScenarioState> {
    let runtime = world.get_resource::<WorldContentRuntime>()?;

    let mission_elapsed = runtime
        .mission_clock
        .zip(world.get_resource::<Time<Fixed>>())
        .map(|(clock, now)| clock.elapsed(now.elapsed()));

    let triggers = runtime.triggers.capture();

    let pending_gm_event_fires: Vec<String> =
        runtime.pending_gm_event_fires.iter().cloned().collect();
    let paused_gm_events: Vec<String> = runtime.paused_gm_events.iter().cloned().collect();
    let pending_gm_event_skips: Vec<String> =
        runtime.pending_gm_event_skips.iter().cloned().collect();

    let mut entity_groups: Vec<(String, Vec<String>)> = runtime
        .entity_groups
        .iter()
        .map(|(group, members)| {
            let mut names: Vec<String> = members.iter().cloned().collect();
            names.sort();
            (group.clone(), names)
        })
        .collect();
    entity_groups.sort_by(|a, b| a.0.cmp(&b.0));

    let mut name_to_uuid: Vec<(String, String)> = runtime
        .name_to_uuid
        .iter()
        .map(|(name, uuid)| (name.clone(), uuid.clone()))
        .collect();
    name_to_uuid.sort_by(|a, b| a.0.cmp(&b.0));

    let mut observed_hull_fractions: Vec<(String, f32)> = runtime
        .observed_hull_fractions
        .iter()
        .map(|(uuid, fraction)| (uuid.clone(), *fraction))
        .collect();
    observed_hull_fractions.sort_by(|a, b| a.0.cmp(&b.0));

    Some(ScenarioState {
        opportunities: runtime.opportunities.clone(),
        plan_choice: runtime.plan_choice.clone(),
        situations: runtime.situations.clone(),
        mission_elapsed,
        triggers,
        entity_groups,
        // Which entity each authored name means (issue #863) — see
        // `ScenarioState::name_to_uuid`.
        name_to_uuid,
        observed_hull_fractions,
        pending_events: runtime
            .pending_world_events
            .iter()
            .map(world_event_record)
            .collect(),
        // The scripted half. Absent for every script-free world, which is what
        // keeps their payloads shaped exactly as they were before this issue.
        script_callbacks: world
            .get_resource::<WorldScriptRuntime>()
            .map(|script| script.pending_callbacks.0.clone())
            .unwrap_or_default(),
        // The named half (issue #1024). Empty — and so absent from the payload —
        // for every world that authors no `[[deadline]]`, which is every shipped
        // world today.
        deadlines: runtime.deadlines.clone(),
        // The promises (issue #1029). Empty — and so absent from the payload —
        // for every run that never reached a beat where the captain gave their
        // word, which is every shipped world today.
        commitments: runtime.commitments.clone(),
        // What the crew found out (issue #1031). Empty — and so absent from the
        // payload — for every run whose scenario never said the crew learned
        // anything, which is every shipped world today.
        evidence: runtime.evidence.clone(),
        // The labour dispute (issue #1035). Empty — and so absent from the
        // payload — for every world that authors no `[[workforce]]`, which is
        // every shipped world but Falling Skyway.
        workforce: runtime.workforce.clone(),
        // The GM's armed Fires (issue #1301). Empty — and so absent from the
        // payload — for every world that authors no GM-operable event, which is
        // every shipped world today.
        pending_gm_event_fires,
        // The GM's armed placements (issue #1305). Empty — and so absent from
        // the payload — for every world that authors no `[[gm_palette]]`, which
        // is every shipped world today.
        pending_gm_spawns: runtime.pending_gm_spawns.clone(),
        pending_gm_despawns: runtime.pending_gm_despawns.clone(),
        gm_despawn_captures: runtime.gm_despawn_captures.clone(),
        contact_overrides: runtime.contact_overrides.clone(),
        contact_classifications: runtime.contact_classifications.clone(),
        presentation: runtime.presentation.clone(),
        contact_information: runtime.contact_information.clone(),
        // The GM's paused events (issue #1303). Empty — and so absent from the
        // payload — for every world that authors no pausable event, which is
        // every shipped world today.
        paused_gm_events,
        // And the armed Skips (issue #1304), on identical terms.
        pending_gm_event_skips,
    })
}

/// Walk the comms state a mid-conversation save resumes from — see
/// [`CommsState`].
///
/// Read-only, like every other `capture_*` here, so a save-free run is
/// digest-neutral by construction: nothing in this function takes a `&mut` on
/// any resource, so no change-detection tick flips and no system downstream sees
/// a different world because a capture was taken.
pub(super) fn capture_comms(world: &World) -> Option<CommsState> {
    let comms = world.get_resource::<CommsRuntime>()?;

    // Sorted by message id — the map's own key. See `CommsState::dialogues`.
    let mut dialogues: Vec<DialogueState> = comms
        .active_dialogues
        .iter()
        .map(|(message_id, dialogue)| reduce_dialogue_node(message_id, dialogue))
        .collect();
    dialogues.sort_by(|a, b| a.message_id.cmp(&b.message_id));

    Some(CommsState {
        inbox: world
            .get_resource::<CommsInboxRes>()
            .map(|inbox| inbox.0.messages())
            .unwrap_or_default(),
        dialogues,
        open_hails: comms.open_hails.clone(),
        // The scripted half, absent for every script-free world — the same
        // shape-preserving property `ScenarioState::script_callbacks` has.
        pending_opens: world
            .get_resource::<WorldScriptRuntime>()
            .map(|script| script.pending_comms_opens.clone())
            .unwrap_or_default(),
        // Already in fleet-slot-then-message-id order — it is a `BTreeMap`.
        pending_ai_responses: comms
            .pending_ai_responses
            .iter()
            .map(|(key, record)| (key.clone(), *record))
            .collect(),
    })
}

/// Tag one queued `WorldEvent` — see [`WorldEventRecord`].
pub(super) fn world_event_record(event: &WorldEvent) -> WorldEventRecord {
    let mut row = WorldEventRecord::default();
    match event {
        WorldEvent::Activity(fact) => {
            row.kind = 13;
            row.activity = Some(fact.clone());
        }
        WorldEvent::Enquiry {
            target_uuid,
            observer_uuid,
            system,
            aim,
            intrusive,
            phase,
        } => {
            row.kind = 12;
            row.subject = target_uuid.clone();
            row.other = observer_uuid.clone();
            row.system = system.0.clone();
            row.aim = aim.clone();
            row.intrusive = *intrusive;
            row.enquiry_phase = Some(*phase);
        }
        WorldEvent::Disabled { uuid, disabled } => {
            row.kind = 11;
            row.subject = uuid.clone();
            row.disabled = *disabled;
        }
        WorldEvent::Destroyed { uuid } => {
            row.kind = 0;
            row.subject = uuid.clone();
        }
        WorldEvent::Attacked {
            uuid,
            attacker_uuid,
        } => {
            row.kind = 1;
            row.subject = uuid.clone();
            row.other = attacker_uuid.clone();
        }
        WorldEvent::HullDroppedBelow {
            uuid,
            previous_fraction,
            current_fraction,
        } => {
            row.kind = 2;
            row.subject = uuid.clone();
            row.numbers = [*previous_fraction, *current_fraction];
        }
        WorldEvent::TimerElapsed { elapsed_secs } => {
            row.kind = 3;
            row.numbers = [*elapsed_secs, 0.0];
        }
        WorldEvent::Hailed { target_uuid } => {
            row.kind = 4;
            row.subject = target_uuid.clone();
        }
        WorldEvent::FlagSet { name, origin_layer } => {
            row.kind = 5;
            row.subject = name.clone();
            row.origin_layer = origin_layer.clone();
        }
        WorldEvent::FlagCleared { name, origin_layer } => {
            row.kind = 6;
            row.subject = name.clone();
            row.origin_layer = origin_layer.clone();
        }
        WorldEvent::WorldLoaded => row.kind = 7,
        WorldEvent::EnteredRegion { uuid } => {
            row.kind = 8;
            row.subject = uuid.clone();
        }
        WorldEvent::ExitedRegion { uuid } => {
            row.kind = 9;
            row.subject = uuid.clone();
        }
        WorldEvent::WaypointReached { uuid, waypoint } => {
            row.kind = 10;
            row.subject = uuid.clone();
            row.other = waypoint.clone();
        }
    }
    row
}

pub(super) fn hull_rows(hull: &crate::ship::damage::SystemHull) -> Vec<(String, f32, f32)> {
    hull.iter()
        .map(|(id, entry)| (id.0.clone(), entry.current, entry.max))
        .collect()
}

/// Project the AI LOD lifecycle through stable entity identities.
///
/// A row is emitted for every ship. That includes a never-promoted Low NPC (no
/// marker, no timer), a demoted NPC (timer but no marker), and the permanently
/// High local ship. Querying the domain marker instead of inferring ship-ness
/// from whichever lifecycle component happens to be present is what makes the
/// boolean exhaustive: a ship carrying neither half is explicitly Low rather
/// than silently absent from the format-14 payload.
pub(super) fn capture_ai_fidelity(entity: EntityRef<'_>) -> Option<AiFidelityState> {
    entity
        .contains::<crate::server_app::Ship>()
        .then(|| AiFidelityState {
            high_fidelity: entity.contains::<crate::ai::server::AiHighFidelity>(),
            last_transition_secs: entity
                .get::<crate::ai::server::LodTransitionTimer>()
                .map(|timer| timer.last_state_change_secs),
        })
}

/// Capture the helm axes through their owning continuation adapter.
pub(super) fn capture_controls(entity: EntityRef<'_>) -> Option<ControlState> {
    ControlState::capture_from(entity)
}

pub(super) fn capture_drives(entity: EntityRef<'_>) -> Option<DriveState> {
    let impulse = entity.get::<ShipImpulse>();
    let boost = entity.get::<ShipBoost>();
    let history = entity.get::<crate::server_app::ImpulseHullHistory>();
    (impulse.is_some() || boost.is_some()).then(|| DriveState {
        impulse_phase: impulse.map_or(0, |state| match state.0.phase {
            ImpulsePhase::Idle => 0,
            ImpulsePhase::Charging => 1,
            ImpulsePhase::Active => 2,
        }),
        impulse_charge_progress: impulse.map_or(0.0, |state| state.0.charge_progress),
        impulse_previous_hull_hp: history.and_then(|history| history.0),
        boost_active: boost.is_some_and(|state| state.0.active),
        boost_battery: boost.map_or(0.0, |state| state.0.battery),
    })
}

/// Independent weapon/repair presence and the sorted blackboard/patrol continuation.
pub(super) type WeaponRepairRow = (
    Option<WeaponState>,
    Option<RepairState>,
    Vec<(String, crate::core::messages::SystemBlackboard)>,
    Vec<(String, u32, bool)>,
);

pub(super) fn capture_weapons_and_repair(entity: EntityRef<'_>) -> WeaponRepairRow {
    let beam = entity.get::<ActiveBeam>();
    let cooldown = entity.get::<PhaserCooldown>();
    let torpedoes = entity.get::<TorpedoSystemResource>();
    let arcs = entity.get::<EntityShipArcHull>();
    let teams = entity.get::<ShipRepairTeams>();
    let queue = entity.get::<RepairRequestQueue>();
    let alerted = entity.get::<RepairHumanAlerted>();
    let blackboards = entity.get::<crate::server_app::ShipSystemBlackboards>();
    let cursors = entity.get::<crate::ai::server::ObjectiveCursors>();
    let blasters = entity.get::<crate::console::weapons::blaster::BlasterSystemResource>();
    let shields = entity.get::<crate::ship::shields::ShipShields>();
    // A row is emitted only for an entity that carries at least one
    // of these, for `capture_controls`' reason: an all-defaults
    // `WeaponState` and a genuinely idle one are the same bytes, so
    // storing one for every entity would make an asteroid look like
    // a ship with its weapons cold.
    let weapons = (beam.is_some()
        || cooldown.is_some()
        || torpedoes.is_some()
        || arcs.is_some()
        || blasters.is_some()
        || shields.is_some())
    .then(|| weapon_state(beam, cooldown, torpedoes, arcs, blasters, shields));
    let repair = (teams.is_some() || queue.is_some() || alerted.is_some())
        .then(|| repair_state(teams, queue, alerted));
    let mut boards: Vec<(String, crate::core::messages::SystemBlackboard)> = blackboards
        .map(|b| {
            b.0.iter()
                .map(|(id, board)| (id.0.clone(), board.clone()))
                .collect()
        })
        .unwrap_or_default();
    // The component is a `HashMap` on purpose (see its own docs);
    // a payload may not inherit that order.
    boards.sort_by(|a, b| a.0.cmp(&b.0));
    let cursors = cursors
        .map(|c| {
            c.0.iter()
                .map(|cursor| {
                    (
                        cursor.objective_id.clone(),
                        cursor.index() as u32,
                        cursor.settled(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    (weapons, repair, boards, cursors)
}

/// Component presence remains explicit even when both arc-request halves are default.
pub(super) fn capture_arc_requests(entity: EntityRef<'_>) -> Option<ArcRequestState> {
    let weapons_state = entity.get::<crate::console::weapons::WeaponsArcRequestState>();
    let pending = entity.get::<crate::ship_plugin::PendingArcBearingRequest>();
    (weapons_state.is_some() || pending.is_some()).then(|| ArcRequestState {
        last: weapons_state.and_then(|s| {
            s.last
                .as_ref()
                .map(|(family, target, arcs)| (*family, target.clone(), arcs.clone()))
        }),
        pending_target: pending.and_then(|p| p.target.map(|t| t.to_string())),
        pending_arcs: pending.map(|p| p.arcs.clone()).unwrap_or_default(),
    })
}

/// The reactor allocation, captured directly from its entity.
///
/// A row is emitted for every ship carrying a [`ShipPowerSystem`] — unlike the
/// weapon and arc rows there is no "all defaults look idle" ambiguity to guard
/// against, because a ship either has a reactor or it does not, and a defaulted
/// reactor (every group at 2, full battery, unlocked) is a genuinely different
/// state from the boosted one this restore exists to reinstate.
pub(super) fn capture_power(entity: EntityRef<'_>) -> Option<PowerState> {
    Some(
        entity
            .get::<crate::ship::power::ShipPowerSystem>()?
            .0
            .capture_continuation(),
    )
}

/// The shared motion plan, looked up by this entity — see
/// [`EntityState::motion_plan`]. Reads the `HelmMotionPlan` resource (keyed by
/// `Entity`) and re-keys it by the stable `EntityUuid`, which is what lets it
/// survive a restore into a world whose entity ids are different.
pub(super) fn capture_motion_plans(
    entity: EntityRef<'_>,
    plan: Option<&crate::ship::helm_planner::HelmMotionPlan>,
) -> Option<MotionPlanState> {
    let ship = plan?.ships.get(&entity.id())?;
    Some(MotionPlanState {
        desired_velocity_local: ship.motion.desired_velocity_local.to_array(),
        desired_facing_local: ship.motion.desired_facing_local.to_array(),
        hazard_forces: ship.hazard.hazard_forces.to_array(),
        urgency: ship.hazard.urgency,
        primary_hazard: ship.hazard.primary_hazard.map(|u| u.to_string()),
        moving_hazard_threat: ship.hazard.moving_hazard_threat,
        docking_active: ship.docking_active,
        navigation_generation: ship.navigation_generation,
    })
}

/// The helm pass surface, captured directly from its entity — see
/// [`EntityState::pass_surface`]. A row is emitted for every ship carrying one;
/// the planner reads it every tick, so there is no "idle looks default"
/// ambiguity to guard against.
pub(super) fn capture_pass_surfaces(
    entity: EntityRef<'_>,
) -> Option<crate::ship::helm_ai::HelmPassSurface> {
    Some(*entity.get::<crate::ship::helm_ai::HelmPassSurface>()?)
}

pub(super) fn weapon_state(
    beam: Option<&ActiveBeam>,
    cooldown: Option<&PhaserCooldown>,
    torpedoes: Option<&TorpedoSystemResource>,
    arcs: Option<&EntityShipArcHull>,
    blasters: Option<&crate::console::weapons::blaster::BlasterSystemResource>,
    shields: Option<&crate::ship::shields::ShipShields>,
) -> WeaponState {
    let system = torpedoes.map(|t| &t.0);
    WeaponState {
        beam_impact_samples: beam
            .map(|beam| {
                beam.live_banks()
                    .filter_map(|(bank, slot)| {
                        Some((
                            bank.clone(),
                            (
                                slot.impact_sample_target.clone()?,
                                slot.impact_sample.clone()?,
                            ),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        beam_strike_bonuses: beam
            .map(|b| {
                b.live_banks()
                    .filter(|(_, slot)| slot.strike_damage_bonus != 0.0)
                    .map(|(bank, slot)| (bank.clone(), slot.strike_damage_bonus))
                    .collect()
            })
            .unwrap_or_default(),
        beams: beam
            .map(|b| {
                b.live_banks()
                    .map(|(bank, slot)| {
                        (
                            bank.clone(),
                            slot.target_uuid.clone(),
                            slot.remaining_secs,
                            slot.damage_accumulator,
                            slot.pending_cooldown_secs,
                            slot.operation,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        phaser_cooldowns: cooldown
            .map(PhaserCooldown::active_banks_sorted)
            .unwrap_or_default(),
        tubes: system
            .map(|s| {
                s.tubes
                    .iter()
                    .map(|tube| {
                        let (load_phase, load_timer) = match &tube.load_state {
                            TubeLoadState::Unloaded => (0, [0.0, 0.0]),
                            TubeLoadState::Loading { remaining, total } => {
                                (1, [*remaining, *total])
                            }
                            TubeLoadState::Loaded => (2, [0.0, 0.0]),
                            TubeLoadState::Unloading { remaining, total } => {
                                (3, [*remaining, *total])
                            }
                        };
                        TubeState {
                            load_priority: s.loader_group(&tube.id).map(|_| tube.load_priority),
                            id: tube.id.clone(),
                            load_phase,
                            load_timer,
                            loaded_count: tube.loaded_count,
                            target_count: tube.target_count,
                            active_barrels: tube.active_barrels.clone(),
                            pattern_step: tube.pattern_step,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        magazine: system.map(|s| s.magazine.clone()),
        torpedoes_in_flight: system
            .map(|s| {
                s.in_flight
                    .iter()
                    .map(|t| TorpedoInFlight {
                        last_guidance: t.last_guidance.clone(),
                        profile: t.profile,
                        durability_remaining: t.durability_remaining,
                        strike_damage_bonus: t.strike_damage_bonus,
                        uuid: t.uuid.clone(),
                        position: [t.x, t.y, t.z],
                        heading: t.heading,
                        pitch: t.pitch,
                        lifespan_remaining: t.lifespan_remaining,
                        target_uuid: t.target_uuid.clone(),
                        source_uuid: t.source_uuid.clone(),
                        tube_id: t.tube_id.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        bursts: system
            .map(|s| {
                s.burst_states
                    .iter()
                    .map(|b| BurstState {
                        launch_uuid: b.launch_uuid.clone(),
                        profile: b.profile,
                        burst_interval_secs: b.burst_interval_secs,
                        tube_id: b.tube_id.clone(),
                        pending: b.pending,
                        timer: b.timer,
                        launch: [b.launch_x, b.launch_y, b.launch_z],
                        launch_heading: b.launch_heading,
                        target_uuid: b.target_uuid.clone(),
                        source_uuid: b.source_uuid.clone(),
                        barrel_origins: b
                            .barrel_origins
                            .iter()
                            .map(|(x, y, z)| [*x, *y, *z])
                            .collect(),
                        barrel_sequence: b.barrel_sequence.clone(),
                        next_shot_index: b.next_shot_index,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        arc_hull: arcs
            .map(|a| {
                a.0.iter()
                    .map(|(id, entry)| (id.to_string(), entry.current, entry.max))
                    .collect()
            })
            .unwrap_or_default(),
        shield_operations: shields
            .map(|shields| shields.0.capture_operations())
            .unwrap_or_default(),
        shield_charge: shields
            .map(|s| {
                s.0.facings
                    .iter()
                    .map(|f| {
                        (
                            f.id.clone(),
                            f.hp,
                            f.hp_frac(),
                            f.offline_remaining,
                            f.is_focused,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        blasters: blasters
            .map(|b| {
                b.0.iter()
                    .map(|bank| {
                        let v = &bank.volley;
                        BlasterRuntime {
                            pending_volley: v.pending_volley,
                            schedule: v.schedule.clone(),
                            next_step: v.next_step as u32,
                            volley_elapsed: v.volley_elapsed,
                            active_barrels: v.active_barrels.clone(),
                            current_step: v.current_step,
                            on_cooldown: v.on_cooldown,
                            cooldown_remaining: v.cooldown_remaining,
                            charging: v.charging,
                            charge_elapsed: v.charge_elapsed,
                            in_flight: bank
                                .in_flight
                                .iter()
                                .map(|p| BlasterBolt {
                                    impact: p.impact,
                                    id: p.id.clone(),
                                    x: p.x,
                                    y: p.y,
                                    pitch: p.pitch,
                                    visual_scale: p.visual_scale,
                                    z: p.z,
                                    heading: p.heading,
                                    speed: p.speed,
                                    lifespan_remaining: p.lifespan_remaining,
                                    collision_radius: p.collision_radius,
                                    damage: p.damage,
                                    shield_pierce: p.shield_pierce,
                                    source_uuid: p.source_uuid.clone(),
                                })
                                .collect(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub(super) fn repair_state(
    teams: Option<&ShipRepairTeams>,
    queue: Option<&RepairRequestQueue>,
    alerted: Option<&RepairHumanAlerted>,
) -> RepairState {
    let mut alerted_rows: Vec<(
        crate::core::messages::RepairGroup,
        crate::ship::damage::DamageTier,
    )> = alerted
        .map(|a| {
            a.0.iter()
                .map(|(system, tier)| (system.clone(), *tier))
                .collect()
        })
        .unwrap_or_default();
    alerted_rows.sort_by(|a, b| a.0.cmp(&b.0));
    RepairState {
        personnel_destinations: queue
            .map(|queue| queue.personnel_destinations.clone())
            .unwrap_or_default(),
        jobs: teams.map(|t| t.0.save_state()),
        readings: queue
            .map(|q| {
                q.entries
                    .iter()
                    .map(|entry| (entry.group.clone(), entry.readings.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        queue: queue
            .map(|q| {
                q.entries
                    .iter()
                    .map(|e| {
                        (
                            e.group.clone(),
                            e.station_label.clone(),
                            e.tier,
                            e.deficit,
                            e.observations.clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        recoveries: queue.map(|q| q.recoveries.clone()).unwrap_or_default(),
        alerted: alerted_rows,
    }
}

/// The infrastructure condition tracks, captured directly from its entity
/// — see [`EntityState::infrastructure`]. Only entities that authored
/// `[infrastructure]` carry one, so most worlds capture an empty list.
pub(super) fn capture_infrastructure(
    entity: EntityRef<'_>,
) -> Option<crate::infrastructure::InfrastructureState> {
    Some(
        entity
            .get::<crate::infrastructure::InfrastructureCondition>()?
            .0
            .clone(),
    )
}

/// The tractor-beam states, captured directly from its entity — see
/// [`EntityState::tractor`]. Only hulls that authored `[tractor]` carry one, so
/// most worlds capture an empty list.
///
/// A beam that authored a tractor and is engaging nothing captures nothing — the
/// same reading `fold_tractor_namespace` takes: an idle beam is byte-for-byte the
/// state of a hull built before tractors existed, so writing it would charge
/// every world fielding a tractor-capable hull for a feature its crew never used.
pub(super) fn capture_tractors(entity: EntityRef<'_>) -> Option<crate::tractor::TractorSaveState> {
    let state = entity.get::<crate::tractor::TractorBeam>()?.save_state();
    (state != crate::tractor::TractorSaveState::default()).then_some(state)
}

/// The dock control states, captured directly from its entity — see
/// [`EntityState::dock`]. Only hulls that authored a `kind = "dock"` system carry
/// one. Idle controls retain their authored channel keys and default values,
/// matching the complete intent folded by `fold_dock_namespace`.
pub(super) fn capture_docks(entity: EntityRef<'_>) -> Option<crate::dock::DockSaveState> {
    Some(entity.get::<crate::dock::DockControl>()?.save_state())
}

/// The transfer-umbilical states, captured directly from its entity — see
/// [`EntityState::umbilical`]. Only hulls that authored a `kind = "umbilical"`
/// system carry one, so most worlds capture an empty list. An idle umbilical (not
/// running) captures nothing, the same reading `fold_umbilical_namespace` takes.
pub(super) fn capture_umbilicals(
    entity: EntityRef<'_>,
) -> Option<crate::umbilical::UmbilicalSaveState> {
    let state = entity
        .get::<crate::umbilical::TransferUmbilical>()?
        .save_state();
    state
        .port_order
        .validate_settlement()
        .expect("port order receipts must settle before snapshot capture");
    if let Some(flow) = &state.port_flow {
        flow.validate_settlement()
            .expect("free port receipts must settle before snapshot capture");
    }
    (state != crate::umbilical::UmbilicalSaveState::default()).then_some(state)
}

/// The Security-team assignments, captured directly from its entity — see
/// [`EntityState::security`]. Only hulls that authored a `[security]` table carry
/// one. A never-used muster omits the payload; consumed JobIds and terminal
/// outcomes remain captured after the last team returns.
pub(super) fn capture_security(
    entity: EntityRef<'_>,
) -> Option<crate::security::SecuritySaveState> {
    let state = entity
        .get::<crate::security::ShipSecurityTeams>()?
        .save_state();
    state.has_history().then_some(state)
}

/// The scan records, captured directly from its entity — see
/// [`EntityState::scan`]. Only hulls that authored `[scan]` carry one, so most
/// worlds capture an empty list.
pub(super) fn capture_scans(entity: EntityRef<'_>) -> Option<crate::science::ScanSaveState> {
    Some(entity.get::<crate::science::ShipScanRecord>()?.save_state())
}

/// Read the optional spawn recipe directly; an unregistered origin never gates other state.
pub(super) fn capture_spawn_origins(
    entity: EntityRef<'_>,
) -> Option<crate::world::spawn_origin::SpawnOrigin> {
    Some(
        entity
            .get::<crate::entities::spawner::EntitySpawnOrigin>()?
            .0
            .clone(),
    )
}

/// The civilian traffic states, captured directly from its entity — see
/// [`EntityState::civilian`]. Only entities that authored `[civilian]` carry
/// one, so most worlds capture an empty list.
pub(super) fn capture_civilians(entity: EntityRef<'_>) -> Option<crate::civilian::CivilianState> {
    Some(entity.get::<crate::civilian::CivilianTraffic>()?.0.clone())
}

/// The debris contacts, captured directly from its entity — see
/// [`EntityState::debris`]. Only entities that authored `[debris]` carry one, so
/// every world but the one that sheds a corridor captures an empty list.
///
/// Unlike its siblings there is no "idle row captures nothing" filter, and the
/// reason is the `Transform` this row carries: an unread mass that has drifted
/// two hundred units is not in its default state, and dropping the row because
/// no latch had risen yet would put it back where it was shed.
pub(super) fn capture_debris(entity: EntityRef<'_>) -> Option<crate::debris::DebrisSaveState> {
    let transform = entity.get::<Transform>()?;
    Some(
        entity
            .get::<crate::debris::DebrisThreat>()?
            .save_state(transform.translation),
    )
}

/// Shields AI continuation state, captured directly from its entity.
///
/// Damage records are stamped with the authoritative logical tick. Persisting
/// that tick verbatim preserves the strict integer expiry boundary exactly.
pub(super) fn capture_shield_ai(entity: EntityRef<'_>) -> Option<ShieldsAiState> {
    let history = entity.get::<crate::ship::shields::ShieldsDamageHistory>();
    let pending = entity.get::<crate::ship::shields::PendingShieldsThreatBearing>();
    if history.is_none() && pending.is_none() {
        return None;
    }
    let damage_by_arc = history
        .map(|history| {
            history
                .arcs
                .iter()
                .map(|records| {
                    records
                        .iter()
                        .map(|record| (record.recorded_tick, record.amount))
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    let last_hp = history
        .map(|history| history.last_hp.clone())
        .unwrap_or_default();

    Some(ShieldsAiState {
        damage_by_arc,
        last_hp,
        pending_threat_bearing_rad: pending.and_then(|bearing| bearing.0),
    })
}

/// Sensors threat-warning debounce memory on this entity.
pub(super) fn capture_sensors_threat(entity: EntityRef<'_>) -> Option<SensorsThreatMemoryState> {
    let state = entity.get::<crate::ship::sensors::SensorsThreatState>()?;
    Some(SensorsThreatMemoryState {
        last_threat_uuid: state.last_threat_uuid.clone(),
        last_bearing_rad: state.last_bearing_rad,
        last_label: state.last_label.clone(),
        last_distance: state.last_distance,
    })
}

pub(super) fn capture_intent_narration(entity: EntityRef<'_>) -> Option<IntentNarrationState> {
    let narration = entity.get::<crate::ship::intent_narration_systems::ShipIntentNarration>()?;
    let (last, generation) = narration.continuation();
    let mut last: Vec<_> = last
        .iter()
        .map(|(station, snapshot)| {
            (
                station.0.clone(),
                IntentSnapshotState {
                    target_label: snapshot.target_label.clone(),
                    combat_posture: snapshot.combat_posture,
                    hull_fraction: snapshot.hull_fraction,
                    shield_focus: snapshot.shield_focus.clone(),
                    brownout_groups: snapshot.brownout_groups.clone(),
                    manoeuvre: snapshot.manoeuvre.clone(),
                },
            )
        })
        .collect();
    last.sort_by(|a, b| a.0.cmp(&b.0));
    Some(IntentNarrationState { last, generation })
}

pub(super) fn capture_recent_combat(entity: EntityRef<'_>) -> Option<RecentCombatActivityState> {
    let activity = entity.get::<crate::ship::combat_activity::RecentCombatActivity>()?;
    let (damage, hostile_fire, weapon_fired, prev_hull) = activity.continuation();
    Some(RecentCombatActivityState {
        last_damage_taken_secs: damage,
        last_hostile_fire_taken_secs: hostile_fire,
        last_weapon_fired_secs: weapon_fired,
        prev_hull,
    })
}

pub(super) fn capture_frequency_hints(
    entity: EntityRef<'_>,
) -> Option<FrequencyHintContinuationState> {
    let state = entity.get::<crate::console_ai::server::ShipFrequencyHintState>()?;
    let (current_target, elapsed_secs, hint_sent) = state.continuation();
    Some(FrequencyHintContinuationState {
        current_target: current_target.map(str::to_owned),
        elapsed_secs,
        hint_sent,
    })
}

pub(super) fn capture_ship_phaser_frequencies(
    entity: EntityRef<'_>,
) -> Option<ShipPhaserFrequencyState> {
    let frequency = entity.get::<crate::ship::state::ShipPhaserFrequency>()?;
    Some(ShipPhaserFrequencyState {
        frequency: frequency.continuation(),
    })
}

pub(super) fn capture_tactical_frequency_hints(
    entity: EntityRef<'_>,
) -> Option<TacticalFrequencyHintInboxState> {
    let pending = entity.get::<crate::ship::components::PendingTacticalFrequencyHint>()?;
    Some(TacticalFrequencyHintInboxState {
        frequency: pending.continuation(),
    })
}

pub(super) fn damage_tier_tag(tier: crate::ship::damage::DamageTier) -> u8 {
    match tier {
        crate::ship::damage::DamageTier::Operational => 0,
        crate::ship::damage::DamageTier::Damaged => 1,
        crate::ship::damage::DamageTier::Disabled => 2,
        crate::ship::damage::DamageTier::Destroyed => 3,
    }
}

pub(super) fn capture_last_system_tiers(entity: EntityRef<'_>) -> Option<LastSystemTiersState> {
    let state = entity.get::<crate::ship::components::LastSystemTiers>()?;
    let (tiers, hp) = state.continuation();
    let mut tiers: Vec<_> = tiers
        .iter()
        .map(|(system, tier)| (system.0.clone(), damage_tier_tag(*tier)))
        .collect();
    let mut hp: Vec<_> = hp
        .iter()
        .map(|(system, value)| (system.0.clone(), *value))
        .collect();
    tiers.sort_by(|a, b| a.0.cmp(&b.0));
    hp.sort_by(|a, b| a.0.cmp(&b.0));
    Some(LastSystemTiersState { tiers, hp })
}

pub(super) fn capture_power_brownout(
    entity: EntityRef<'_>,
) -> Option<PowerBrownoutContinuationState> {
    let state = entity.get::<crate::ship::power::PowerBrownoutState>()?;
    let (notified_groups, locked_changed) = state.continuation();
    let mut notified_groups: Vec<_> = notified_groups.iter().cloned().collect();
    notified_groups.sort();
    Some(PowerBrownoutContinuationState {
        notified_groups,
        locked_changed,
    })
}

pub(super) fn capture_shields_coordination(
    entity: EntityRef<'_>,
) -> Option<ShieldsCoordinationContinuationState> {
    let state = entity.get::<crate::ship::shields::ShieldsCoordinationState>()?;
    let (down_notified, restore_notified) = state.continuation();
    Some(ShieldsCoordinationContinuationState {
        down_notified: down_notified.to_vec(),
        restore_notified: restore_notified.to_vec(),
    })
}

pub(super) fn capture_coordination_queues(
    entity: EntityRef<'_>,
    captured_tick: u64,
) -> Option<CoordinationQueueState> {
    let queue = entity.get::<crate::ship::components::CoordinationQueue>()?;
    let pending = queue
        .0
        .pending()
        .iter()
        .map(|message| QueuedCoordinationState {
            sender_origin: match message.sender_origin {
                crate::ship::control_source::ControlSource::Human => 0,
                crate::ship::control_source::ControlSource::Ai => 1,
                crate::ship::control_source::ControlSource::Offline => 2,
                crate::ship::control_source::ControlSource::Simplified => 3,
            },
            address: message.address.clone(),
            payload: message.payload.clone(),
            presentation: message.presentation.clone(),
            sender_label: message.sender_label.clone(),
            due_after_fixed_ticks: message
                .due_tick
                .saturating_sub(captured_tick)
                .saturating_add(1),
        })
        .collect();
    Some(CoordinationQueueState { pending })
}

#[derive(Default)]
pub(super) struct CapturedNavigationContinuation {
    pub(super) waypoint: Option<NavigationWaypointState>,
    pub(super) clearance_issue: Option<NavigationClearanceIssueState>,
    pub(super) helm_clearance: Option<HelmWaypointClearanceState>,
}

/// Capture all three pieces of the Navigation→Helm waypoint frontier.
///
/// Component presence is independent and explicit. In particular, a default
/// component is still `Some(default)` so restore can clear a value produced by
/// the fresh bootstrap rather than silently inheriting it.
pub(super) fn capture_navigation_continuation(
    entity: EntityRef<'_>,
) -> CapturedNavigationContinuation {
    let waypoint = entity.get::<crate::console::navigation::NavigationWaypoint>();
    let clearance_issue = entity.get::<crate::console::navigation::NavClearanceIssueState>();
    let helm_clearance = entity.get::<crate::ship_plugin::HelmWaypointClearance>();
    let waypoint = waypoint.map(|waypoint| {
        let snapshot = waypoint.canonical_snapshot();
        NavigationWaypointState {
            formation: match waypoint.mode() {
                Some(crate::console::navigation::WaypointMode::Formation { course }) => {
                    Some(course.clone())
                }
                _ => None,
            },
            accepted_requests: waypoint.accepted_requests.clone(),
            position: snapshot.as_ref().map(|point| [point.x, point.z]),
            source_uuid: snapshot.and_then(|point| point.source_uuid),
            generation: waypoint.generation(),
        }
    });
    let clearance_issue = clearance_issue.map(|state| {
        let (issued_generation, helm_axes_were_ai) = state.continuation();
        NavigationClearanceIssueState {
            issued_generation,
            helm_axes_were_ai,
        }
    });
    let helm_clearance = helm_clearance.map(|state| HelmWaypointClearanceState {
        generation: state.0,
    });

    CapturedNavigationContinuation {
        waypoint,
        clearance_issue,
        helm_clearance,
    }
}

/// The complete captured row for ONE live entity, taken through the same walk a
/// save takes (issue #1444).
///
/// Deliberately the whole walk filtered, not a narrowed per-entity capture: the
/// GM despawn inverse restores through [`apply_entity_state`], and a row built
/// by a different function would be missing exactly the continuations that
/// function had not been taught about. A removal is rare enough — a handful in a
/// live event — that one save-sized walk is the right price for that guarantee.
pub(crate) fn capture_entity_state(world: &World, uuid: &str) -> Option<EntityState> {
    capture_entities(world)
        .into_iter()
        .find(|row| row.uuid == uuid)
}

/// Fixed capture inputs and the registration gates of the former optional queries.
pub(super) struct EntityCaptureContext<'w> {
    pub(super) captured_tick: u64,
    pub(super) motion_plan: Option<&'w crate::ship::helm_planner::HelmMotionPlan>,
    pub(super) ai_fidelity: bool,
    pub(super) drives: bool,
    pub(super) navigation: bool,
    pub(super) machines: bool,
    pub(super) shield_ai: bool,
    pub(super) arc_requests: bool,
}
impl<'w> EntityCaptureContext<'w> {
    pub(super) fn new(world: &'w World) -> Self {
        Self {
            captured_tick: world.get_resource::<SimTick>().map_or(0, |tick| tick.0),
            motion_plan: world.get_resource(),
            ai_fidelity: world
                .try_query_filtered::<(
                    &EntityUuid,
                    Has<crate::ai::server::AiHighFidelity>,
                    Option<&crate::ai::server::LodTransitionTimer>,
                ), With<crate::server_app::Ship>>()
                .is_some(),
            drives: world
                .try_query::<(
                    Option<&ShipImpulse>,
                    Option<&ShipBoost>,
                    Option<&crate::server_app::ImpulseHullHistory>,
                )>()
                .is_some(),
            navigation: world
                .try_query::<(
                    Option<&crate::console::navigation::NavigationWaypoint>,
                    Option<&crate::console::navigation::NavClearanceIssueState>,
                    Option<&crate::ship_plugin::HelmWaypointClearance>,
                )>()
                .is_some(),
            machines: world
                .try_query::<(
                    Option<&ActiveBeam>,
                    Option<&PhaserCooldown>,
                    Option<&TorpedoSystemResource>,
                    Option<&EntityShipArcHull>,
                    Option<&ShipRepairTeams>,
                    Option<&RepairRequestQueue>,
                    Option<&RepairHumanAlerted>,
                    Option<&crate::server_app::ShipSystemBlackboards>,
                    Option<&crate::ai::server::ObjectiveCursors>,
                    Option<&crate::console::weapons::blaster::BlasterSystemResource>,
                    Option<&crate::ship::shields::ShipShields>,
                )>()
                .is_some(),
            shield_ai: world
                .try_query::<(
                    Option<&crate::ship::shields::ShieldsDamageHistory>,
                    Option<&crate::ship::shields::PendingShieldsThreatBearing>,
                )>()
                .is_some(),
            arc_requests: world
                .try_query::<(
                    Option<&crate::console::weapons::WeaponsArcRequestState>,
                    Option<&crate::ship_plugin::PendingArcBearingRequest>,
                )>()
                .is_some(),
        }
    }
}

pub(super) fn capture_entities(world: &World) -> Vec<EntityState> {
    let Some(mut query) = world.try_query_filtered::<(
        &EntityUuid,
        EntityRef,
        Option<&ShipPhysics>,
        Option<&EntitySystemHull>,
        Option<&ShipRedAlert>,
        Option<&crate::console::command::server::ShipStationStances>,
        Option<&crate::ship_plugin::ShipSystemControlSources>,
    ), Allow<bevy::ecs::entity_disabling::Disabled>>() else {
        return Vec::new();
    };
    let context = EntityCaptureContext::new(world);
    let mut rows: Vec<EntityState> = query
        .iter(world)
        .map(
            |(uuid, entity, physics, hull, alert, stances, control_sources)| {
                let navigation = if context.navigation {
                    capture_navigation_continuation(entity)
                } else {
                    Default::default()
                };
                let (weapons, repair, blackboards, patrol_cursors) = if context.machines || entity.contains::<crate::console::weapons::flight_carrier::WeaponFlightCarrier>() {
                    capture_weapons_and_repair(entity)
                } else {
                    Default::default()
                };
                EntityState {
                    weapon_flight_carrier: entity.contains::<crate::console::weapons::flight_carrier::WeaponFlightCarrier>(),
                    carrier_bolt_banks: if entity.contains::<crate::console::weapons::flight_carrier::WeaponFlightCarrier>() {
                        entity.get::<crate::console::weapons::blaster::BlasterSystemResource>()
                            .map(|owner| owner.0.iter().map(|bank| bank.config.id.clone()).collect()).unwrap_or_default()
                    } else {Vec::new()},
                    mesh_crew: capture_mesh_crew(entity),
                    operational_owner: entity.get::<phoenix_sim_session::vessel_ownership::VesselOperationalOwner>().map(|owner| owner.0.clone()),
                    npc_doctrine: entity
                        .get::<crate::gm::operations::npc::NpcDoctrineState>()
                        .and_then(|state| state.0.clone()),
                    ai_fidelity: context
                        .ai_fidelity
                        .then(|| capture_ai_fidelity(entity))
                        .flatten(),
                    control: capture_controls(entity),
                    drive: context.drives.then(|| capture_drives(entity)).flatten(),
                    navigation_waypoint: navigation.waypoint,
                    navigation_clearance_issue: navigation.clearance_issue,
                    helm_waypoint_clearance: navigation.helm_clearance,
                    weapons,
                    repair,
                    blackboards,
                    patrol_cursors,
                    shield_ai: context
                        .shield_ai
                        .then(|| capture_shield_ai(entity))
                        .flatten(),
                    sensors_threat: capture_sensors_threat(entity),
                    intent_narration: capture_intent_narration(entity),
                    recent_combat: capture_recent_combat(entity),
                    frequency_hint: capture_frequency_hints(entity),
                    phaser_frequency: capture_ship_phaser_frequencies(entity),
                    tactical_frequency_hint: capture_tactical_frequency_hints(entity),
                    last_system_tiers: capture_last_system_tiers(entity),
                    power_brownout: capture_power_brownout(entity),
                    shields_coordination: capture_shields_coordination(entity),
                    coordination_queue: capture_coordination_queues(entity, context.captured_tick),
                    arc_request: context
                        .arc_requests
                        .then(|| capture_arc_requests(entity))
                        .flatten(),
                    power: capture_power(entity),
                    mounts: entity.get::<crate::weapons::mounts::ShipMounts>().cloned(),
                    guided_source: entity
                        .get::<crate::console::weapons::guidance_link::ShipGuidanceKnowledge>()
                        .map(|state| state.0.clone()),
                    helm_movement: entity.get::<phoenix_sim_gameplay::ship::helm_control::HelmControlState>().cloned(),
                    transit: entity.get::<crate::console::helm::transit::ShipTransitDrives>().cloned(),
                    probe_custody: entity.get::<crate::entities::craft::ProbeCustody>().cloned(),
                    bay_jobs: entity.get::<crate::entities::craft::ShipBayJobs>().cloned(),
                    illumination: entity.get::<crate::console::weapons::illumination::ShipIllumination>().cloned(),
                    jamming: entity.get::<crate::comms::jamming::ShipJamming>().cloned(),
                    signal_protection: entity.get::<crate::science::protection::ShipSignalProtection>().cloned(),
                    comms_media: entity.get::<crate::comms::media_control::ShipCommsMediaIntent>().cloned(),
                    emission_intent: entity.get::<crate::console::command::emission::ShipEmissionIntent>().map(|state| state.0.clone()),
                    comms_capture: entity.get::<crate::comms::capture::ShipCommsCapture>().cloned(),
                    comms_traffic: entity.get::<crate::comms::traffic::ShipCommsTraffic>().map(|owner| owner.state.clone()),
                    identity_emitters: entity.get::<crate::science::identity::ShipIdentityEmitters>().cloned(),
                    concealment_emitters: entity.get::<crate::science::concealment::ShipConcealmentEmitters>().cloned(),
                    weapon_targets: entity
                        .get::<crate::console::weapons::assignments::ShipWeaponTargets>()
                        .map(|state| state.0.clone()),
                    port_provider: entity
                        .get::<crate::resources::ports_server::PortProviderState>()
                        .map(|owner| owner.0.clone()),
                    shield_frequency: super::shield_enquiry::capture(entity),
                    region_field: entity
                        .get::<crate::regions::field::RegionFieldState>()
                        .cloned(),
                    passive: entity
                        .get::<crate::science::passive::PassiveState>()
                        .cloned(),
                    broad: entity
                        .get::<crate::science::broad::BroadTracks>()
                        .map(|tracks| tracks.0.clone()),
                    navigation_sharing: entity
                        .get::<crate::console::navigation::sharing::NavigationSharing>()
                        .map(|owner| owner.0.clone()),
                    fitting: entity
                        .get::<crate::resources::fitting_server::ShipFitting>()
                        .cloned(),
                    equipment_recovery: entity
                        .get::<crate::resources::equipment_recovery_server::ShipEquipmentRecovery>()
                        .cloned(),
                    recovery: entity
                        .get::<crate::resources::recovery_server::ShipRecovery>()
                        .map(|owner| {
                            owner
                                .0
                                .validate_settlement()
                                .expect("recovery receipts must settle before snapshot capture");
                            owner.0.clone()
                        }),
                    production: entity
                        .get::<crate::resources::production_server::ShipProduction>()
                        .map(|owner| owner.0.clone()),
                    logistics: entity
                        .get::<crate::resources::logistics_server::ShipLogistics>()
                        .map(|owner| owner.0.clone()),
                    access: entity
                        .get::<crate::ship::access::InteriorAccess>()
                        .map(|access| access.statuses()),
                    pass_surface: capture_pass_surfaces(entity),
                    motion_plan: capture_motion_plans(entity, context.motion_plan),
                    infrastructure: capture_infrastructure(entity),
                    tractor: capture_tractors(entity),
                    dock: capture_docks(entity),
                    ai_trees: entity
                        .get::<crate::ai::state_tree_runtime::ShipAiTreeRuntime>()
                        .cloned(),
                    umbilical: capture_umbilicals(entity),
                    security: capture_security(entity),
                    scan: capture_scans(entity),
                    civilian: capture_civilians(entity),
                    debris: capture_debris(entity),
                    spawn: capture_spawn_origins(entity),
                    uuid: uuid.0.clone(),
                    physics: physics.map(|p| {
                        [
                            p.x,
                            p.y,
                            p.z,
                            p.yaw,
                            p.forward_speed,
                            p.roll,
                            p.lateral_speed,
                            p.vertical_speed,
                        ]
                    }),
                    hull: if entity.contains::<crate::ship::compartment_damage::ShipDamage>() {
                        None
                    } else {
                        hull.map(|h| hull_rows(&h.0))
                    },
                    disablement: entity
                        .get::<crate::ship::disablement::DisablementObservation>()
                        .cloned(),
                    physical_damage: entity
                        .get::<crate::ship::compartment_damage::ShipDamage>()
                        .map(|damage| damage.state.clone()),
                    red_alert: alert.map(|a| a.0),
                    // Sorted by station id, the same walk `fold_station_stances_namespace`
                    // takes, so the capture is byte-identical whatever order the map's
                    // entries were inserted in. An empty map yields an empty vec, which
                    // `skip_serializing_if` drops — a never-commanded hull carries no row.
                    console_views: entity
                        .get::<crate::ship::components::ConsoleWorkingViews>()
                        .map(|views| views.0.clone())
                        .unwrap_or_default(),
                    station_stances: stances
                        .map(|s| {
                            let mut pairs: Vec<(String, String)> =
                                s.0.iter()
                                    .map(|(station, stance)| (station.0.clone(), stance.clone()))
                                    .collect();
                            pairs.sort_by(|a, b| a.0.cmp(&b.0));
                            pairs
                        })
                        .unwrap_or_default(),
                    damage_offline_systems: control_sources
                        .map(|sources| {
                            let mut ids: Vec<String> =
                                sources.0.offline_entries().map(|id| id.0.clone()).collect();
                            ids.sort();
                            ids
                        })
                        .unwrap_or_default(),
                    gm_disabled_systems: control_sources
                        .map(|sources| {
                            sources
                                .0
                                .gm_disabled_entries()
                                .map(|id| id.0.clone())
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            },
        )
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

pub(super) fn capture_asteroids(world: &World) -> Vec<AsteroidState> {
    // The streaming window is the only place a live rock's config path
    // survives — nothing on the entity carries it — and [`restore`] needs it to
    // rebuild a rock the target world never streamed.
    let config_paths: Vec<(String, String)> = world
        .get_resource::<AsteroidWindow>()
        .map(|window| {
            window
                .slots
                .iter()
                .flatten()
                .flatten()
                .map(|data| (data.uuid.clone(), data.config_path.clone()))
                .collect()
        })
        .unwrap_or_default();

    let Some(mut query) = world.try_query::<(
        &AsteroidUuid,
        Option<&Transform>,
        Option<&EntitySystemHull>,
        Option<&crate::server_app::AsteroidShieldPierce>,
    )>() else {
        return Vec::new();
    };
    let mut rows: Vec<AsteroidState> = query
        .iter(world)
        .map(|(uuid, transform, hull, pierce)| {
            let t = transform.map(|t| t.translation).unwrap_or(Vec3::ZERO);
            let r = transform.map(|t| t.rotation).unwrap_or(Quat::IDENTITY);
            AsteroidState {
                config_path: config_paths
                    .iter()
                    .find(|(id, _)| id == &uuid.0)
                    .map(|(_, path)| path.clone()),
                uuid: uuid.0.clone(),
                translation: [t.x, t.y, t.z],
                rotation: [r.x, r.y, r.z, r.w],
                hull: hull.map(|h| hull_rows(&h.0)),
                shield_pierce: pierce.map_or(0.0, |p| p.0),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    rows
}

pub(super) fn capture_asteroid_window(world: &World) -> Option<AsteroidWindowState> {
    let window = world.get_resource::<AsteroidWindow>()?;
    let mut slots = Vec::new();
    for (z, row) in window.slots.iter().enumerate() {
        for (x, slot) in row.iter().enumerate() {
            let Some(data) = slot else { continue };
            slots.push(WindowSlot {
                x: x as u32,
                z: z as u32,
                uuid: data.uuid.clone(),
                config_path: data.config_path.clone(),
                hp: data.hp,
                max_hp: data.max_hp,
                y: data.y,
            });
        }
    }
    Some(AsteroidWindowState {
        arena_gx: window.arena_gx,
        arena_gz: window.arena_gz,
        despawn_cells: window.despawn_cells,
        spawn_cells: window.spawn_cells,
        resolution: window.resolution,
        player_grid: window.player_grid,
        composition_key: window.composition_key,
        needs_init: window.needs_init,
        slots,
    })
}

pub(super) fn capture_collisions(world: &World) -> Vec<CollisionRecord> {
    world
        .get_resource::<crate::core::collision_history::CollisionHistory>()
        .map(|history| history.records().to_vec())
        .unwrap_or_default()
}

pub(super) fn capture_mesh_crew(entity: EntityRef<'_>) -> Option<MeshCrewState> {
    use crate::ship::control_source::ControlSource;
    let ratings = entity.get::<crate::ship_plugin::ActiveStationRatings>()?;
    let sources = entity.get::<crate::ship_plugin::ShipSystemControlSources>()?;
    let mut ratings: Vec<_> = ratings
        .0
        .iter()
        .map(|(station, rating)| (station.0.clone(), rating.clone()))
        .collect();
    ratings.sort();
    let mut sources: Vec<_> = sources
        .0
        .entries()
        .map(|(system, source)| {
            (
                system.0.clone(),
                match source {
                    ControlSource::Human => MeshControlSource::Human,
                    ControlSource::Ai => MeshControlSource::Ai,
                    ControlSource::Offline => MeshControlSource::Offline,
                    ControlSource::Simplified => MeshControlSource::Simplified,
                },
            )
        })
        .collect();
    sources.sort_by(|a, b| a.0.cmp(&b.0));

    let occupied = entity
        .get::<crate::lobby::crew_replication::ShipCrewPresence>()
        .map(|presence| presence.0.iter().map(|station| station.0.clone()).collect())
        .unwrap_or_default();
    Some(MeshCrewState {
        ratings,
        sources,
        occupied,
    })
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;

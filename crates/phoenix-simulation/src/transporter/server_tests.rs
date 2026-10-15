use super::*;
use crate::core::messages::{AdmittedCommand, AdmittedCommands};
use bevy::ecs::system::RunSystemOnce;

fn transporter() -> Transporter {
    Transporter::new(
        TransporterConfig {
            range: 500.0,
            seconds_per_civilian: 2.0,
            min_power_level: 0,
        },
        PowerGroupId("helm".into()),
    )
}

fn admitted(payload: SystemControlPayload) -> AdmittedCommands {
    AdmittedCommands(vec![AdmittedCommand {
        target: transporter_system_id(),
        payload,
        response_token: None,
        feedback_correlation: None,
    }])
}

fn queued(world: &mut World) -> Vec<TaskLifecycleRequest> {
    std::mem::take(&mut world.resource_mut::<EffectQueue<TaskLifecycleRequest>>().0)
}

/// One operator with a discovered, in-range contact carrying `count`
/// civilians, and the lifecycle queue the systems push onto. Time advances
/// one whole `seconds_per_civilian` per fixed tick, so exactly one civilian
/// is recovered per `tick_transport`.
fn rescue_world(count: u32) -> (World, Entity, Entity) {
    let mut world = World::new();
    world.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    // A fixed clock whose delta is exactly one civilian's worth of time.
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_secs(2));
    world.insert_resource(time);
    let operator = world
        .spawn((
            EntityUuid("uuid-destroyer".into()),
            transporter(),
            Transform::default(),
            admitted(SystemControlPayload::StartTransport),
        ))
        .id();
    let mut rescue = CivilianRescue::new(count);
    rescue.revealed = true;
    let contact = world
        .spawn((
            EntityUuid("uuid-lighter".into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            rescue,
        ))
        .id();
    (world, operator, contact)
}

fn select_and_start(world: &mut World) {
    let op = world_operator(world);
    world
        .entity_mut(op)
        .insert(admitted(SystemControlPayload::TransportSelectContact {
            uuid: "uuid-lighter".into(),
        }));
    world
        .run_system_once(handle_transporter_commands)
        .expect("select applies");
    world
        .entity_mut(op)
        .insert(admitted(SystemControlPayload::StartTransport));
    world
        .run_system_once(handle_transporter_commands)
        .expect("start applies");
}

fn world_operator(world: &mut World) -> Entity {
    let mut q = world.query_filtered::<Entity, With<Transporter>>();
    q.iter(world).next().expect("one operator")
}

fn measured_rescue_picture(world: &mut World, operator: Entity) -> String {
    use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTrackState, BroadTracks};
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_destroyer.toml",
    )
    .unwrap();
    let config = authored.ship_config.clone().unwrap();
    let power = crate::modifiers::power_system::PowerSystem::from_energy(
        authored.energy.clone().unwrap(),
        config.clone(),
    )
    .unwrap();
    let profiles = BroadProfile::from_config(&authored).unwrap().unwrap();
    let owner = crate::ship::system_registry::sensor_radar_system_id();
    let profile = &profiles.0[&owner].profile;
    let mut state = BroadTrackState::new(profile);
    state
        .advance(
            profile,
            0,
            0.5,
            1.0,
            &[BroadMeasurement {
                source: "uuid-lighter".into(),
                selectable: true,
                observation: None,
                position: [100.0, 0.0],
                support: 1.0,
                identity: None,
                classification: None,
                assessment: None,
            }],
        )
        .unwrap();
    let public = state.projection(profile, 0, 0.5).unwrap();
    let alias = public[0].contact_id.clone();
    let mut boards = world
        .get::<crate::server_app::ShipSystemBlackboards>(operator)
        .cloned()
        .unwrap_or_default();
    boards.0.insert(
        owner.clone(),
        SystemBlackboard::SensorRadar(crate::core::messages::SensorRadarBlackboard {
            tracks: Some(public),
            ..Default::default()
        }),
    );
    world.entity_mut(operator).insert((
        crate::ship_plugin::ShipConfigComponent(config),
        boards,
        BroadTracks(BTreeMap::from([(owner, state)])),
        crate::science::broad::SensingControls(authored.sensing.unwrap().control),
        crate::ship::power::ShipPowerSystem(power),
    ));
    alias
}

#[test]
fn measured_selection_resolves_alias_refuses_guessed_uuid_and_keeps_stop_after_evidence_loss() {
    let (mut world, operator, contact) = rescue_world(3);
    let alias = measured_rescue_picture(&mut world, operator);
    world
        .entity_mut(operator)
        .insert(admitted(SystemControlPayload::TransportSelectContact {
            uuid: "uuid-lighter".into(),
        }));
    world.run_system_once(handle_transporter_commands).unwrap();
    assert!(world
        .get::<Transporter>(operator)
        .unwrap()
        .selected_contact
        .is_none());
    world
        .entity_mut(operator)
        .insert(admitted(SystemControlPayload::TransportSelectContact {
            uuid: alias.clone(),
        }));
    world.run_system_once(handle_transporter_commands).unwrap();
    assert_eq!(
        world
            .get::<Transporter>(operator)
            .unwrap()
            .selected_contact
            .as_deref(),
        Some("uuid-lighter")
    );
    world
        .entity_mut(operator)
        .insert(admitted(SystemControlPayload::StartTransport));
    world.run_system_once(handle_transporter_commands).unwrap();
    world.run_system_once(tick_transport).unwrap();
    assert!(world.get::<Transporter>(operator).unwrap().transporting);
    assert!(
        world.get::<CivilianRescue>(contact).unwrap().recovered > 0
            || world.get::<Transporter>(operator).unwrap().progress > 0.0,
        "actual finite consumer work must progress"
    );
    // Held physical work remains real even when the public association goes.
    world
        .get_mut::<crate::science::broad::BroadTracks>(operator)
        .unwrap()
        .0
        .values_mut()
        .for_each(|state| state.current.clear());
    world
        .run_system_once(publish_transporter_blackboard)
        .unwrap();
    let SystemBlackboard::Transporter(board) = &world
        .get::<crate::server_app::ShipSystemBlackboards>(operator)
        .unwrap()
        .0[&transporter_system_id()]
    else {
        panic!()
    };
    assert!(board.transporting);
    assert!(board.selected_contact_name.is_none());
    assert!(!board.discovered);
    assert_eq!(board.civilians_remaining, 0);
    world
        .entity_mut(operator)
        .insert(admitted(SystemControlPayload::StopTransport));
    world.run_system_once(handle_transporter_commands).unwrap();
    assert!(!world.get::<Transporter>(operator).unwrap().transporting);
}

#[test]
fn measured_rescue_ai_uses_recorded_request_handle_not_world_name_to_uuid() {
    let (mut world, operator) = backfill_world(
        vec![scored(
            "rescue",
            5.0,
            AiDirective::Rescue {
                target: "mission-rescue".into(),
            },
        )],
        false,
        false,
    );
    let alias = measured_rescue_picture(&mut world, operator);
    let mut runtime = WorldContentRuntime::default();
    runtime
        .name_to_uuid
        .insert("mission-rescue".into(), "uuid-lighter".into());
    world.insert_resource(runtime);
    world.run_system_once(operate_transporter_ai).unwrap();
    assert!(
        admitted_payloads(&mut world, operator).is_empty(),
        "mission lookup is not observation"
    );
    let owner = crate::ship::system_registry::sensor_radar_system_id();
    let mut boards = world.get_mut::<ShipSystemBlackboards>(operator).unwrap();
    let Some(SystemBlackboard::SensorRadar(board)) = boards.0.get_mut(&owner) else {
        panic!()
    };
    board.tracks.as_mut().unwrap()[0].identity =
        Some(phoenix_model::sensing_tracks::TrackIdentity {
            name: "Held rescue identity".into(),
            faction: None,
            references: vec!["mission-rescue".into()],
        });
    drop(boards);
    world.run_system_once(operate_transporter_ai).unwrap();
    assert_eq!(
        admitted_payloads(&mut world, operator),
        vec![
            SystemControlPayload::TransportSelectContact { uuid: alias },
            SystemControlPayload::StartTransport
        ]
    );
}

#[test]
fn a_discovered_in_range_contact_is_recovered_over_ticks_and_opens_one_activation() {
    let (mut world, operator, contact) = rescue_world(2);
    select_and_start(&mut world);
    // Draining the select's own nothing.
    let _ = queued(&mut world);

    // Tick 1: the transport begins running — a Start beat — and recovers the
    // first civilian.
    world
        .run_system_once(tick_transport)
        .expect("verdict applies");
    assert_eq!(
        queued(&mut world),
        vec![TaskLifecycleRequest::Start {
            slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
            target: Some("uuid-lighter".into()),
        }],
    );
    assert_eq!(world.get::<CivilianRescue>(contact).unwrap().recovered, 1);
    assert!(world.get::<Transporter>(operator).unwrap().transporting);

    // Tick 2: the last civilian is recovered and the activation completes.
    world
        .run_system_once(tick_transport)
        .expect("verdict applies");
    assert_eq!(
        queued(&mut world),
        vec![TaskLifecycleRequest::End {
            slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
            reason: TaskTerminalReason::Completed,
        }],
    );
    let rescue = world.get::<CivilianRescue>(contact).unwrap();
    assert_eq!(rescue.recovered, 2);
    assert_eq!(rescue.remaining(), 0);
    assert!(!world.get::<Transporter>(operator).unwrap().transporting);
}

#[test]
fn an_undiscovered_contact_is_refused_not_discovered() {
    let (mut world, operator, contact) = rescue_world(2);
    world.get_mut::<CivilianRescue>(contact).unwrap().revealed = false;
    select_and_start(&mut world);
    let _ = queued(&mut world);

    world
        .run_system_once(tick_transport)
        .expect("verdict applies");
    // A refused start is a beat — a Start and its own terminal.
    assert_eq!(
        queued(&mut world),
        vec![
            TaskLifecycleRequest::Start {
                slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
                target: Some("uuid-lighter".into()),
            },
            TaskLifecycleRequest::End {
                slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
                reason: TaskTerminalReason::Unreadable,
            },
        ],
    );
    assert_eq!(world.get::<CivilianRescue>(contact).unwrap().recovered, 0);
    assert_eq!(
        world.get::<Transporter>(operator).unwrap().last_refusal,
        Some(TransportRefusal::NotDiscovered)
    );
}

#[test]
fn a_contact_out_of_range_is_refused_out_of_range() {
    let (mut world, operator, contact) = rescue_world(2);
    world
        .entity_mut(contact)
        .insert(Transform::from_xyz(5_000.0, 0.0, 0.0));
    select_and_start(&mut world);
    let _ = queued(&mut world);

    world
        .run_system_once(tick_transport)
        .expect("verdict applies");
    assert_eq!(
        world.get::<Transporter>(operator).unwrap().last_refusal,
        Some(TransportRefusal::OutOfRange)
    );
    assert_eq!(world.get::<CivilianRescue>(contact).unwrap().recovered, 0);
}

#[test]
fn stopping_a_running_transport_reports_a_release() {
    let (mut world, _operator, _contact) = rescue_world(3);
    select_and_start(&mut world);
    let _ = queued(&mut world);
    world.run_system_once(tick_transport).expect("runs");
    let _ = queued(&mut world);

    // Stop it mid-recovery.
    let op = world_operator(&mut world);
    world
        .entity_mut(op)
        .insert(admitted(SystemControlPayload::StopTransport));
    world
        .run_system_once(handle_transporter_commands)
        .expect("stop applies");
    assert_eq!(
        queued(&mut world),
        vec![TaskLifecycleRequest::End {
            slot: transport_slot(Some(&EntityUuid("uuid-destroyer".into()))),
            reason: TaskTerminalReason::Released,
        }],
    );
}

#[test]
fn destroying_a_carrier_with_civilians_aboard_raises_the_lost_flag() {
    use crate::ai::server::AiEntityDestroyed;
    use bevy::ecs::message::Messages;

    let mut world = World::new();
    world.init_resource::<CivilianRescueLedger>();
    world.init_resource::<WorldContentRuntime>();
    world.init_resource::<Messages<AiEntityDestroyed>>();
    // A carrier with two of its four civilians still aboard.
    let mut rescue = CivilianRescue::new(4);
    rescue.recovered = 2;
    world.spawn((
        EntityUuid("uuid-lighter".into()),
        EntityId("lighter".into()),
        rescue,
    ));
    // Fire destroys it — the balance/AI death event both combat paths write.
    world
        .resource_mut::<Messages<AiEntityDestroyed>>()
        .write(AiEntityDestroyed {
            entity_uuid: "uuid-lighter".into(),
        });

    world
        .run_system_once(record_civilian_casualties)
        .expect("the casualty system runs");

    assert!(
        world
            .resource::<WorldContentRuntime>()
            .flags
            .counter(&rescue_lost_flag("lighter"))
            > 0,
        "a carrier lost with civilians aboard raises rescue.<id>.lost"
    );
}

#[test]
fn destroying_a_fully_recovered_carrier_raises_no_casualty() {
    use crate::ai::server::AiEntityDestroyed;
    use bevy::ecs::message::Messages;

    let mut world = World::new();
    world.init_resource::<CivilianRescueLedger>();
    world.init_resource::<WorldContentRuntime>();
    world.init_resource::<Messages<AiEntityDestroyed>>();
    // Everyone already aboard.
    let mut rescue = CivilianRescue::new(3);
    rescue.recovered = 3;
    world.spawn((
        EntityUuid("uuid-lighter".into()),
        EntityId("lighter".into()),
        rescue,
    ));
    world
        .resource_mut::<Messages<AiEntityDestroyed>>()
        .write(AiEntityDestroyed {
            entity_uuid: "uuid-lighter".into(),
        });

    world
        .run_system_once(record_civilian_casualties)
        .expect("the casualty system runs");

    assert_eq!(
        world
            .resource::<WorldContentRuntime>()
            .flags
            .counter(&rescue_lost_flag("lighter")),
        0,
        "an empty hulk is not a casualty — everyone was already recovered"
    );
}

// ── Backfill rescue AI (host-level) ──────────────────────────────────────

use crate::core::messages::{
    AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
    ViewscreenBlackboard,
};
use crate::server_app::ShipSystemBlackboards;
use crate::ship::control_source::{ControlSource, ControlSourceResolver};
use crate::ship::system_registry::viewscreen_system_id;
use crate::ship_plugin::ShipSystemControlSources;

/// A scored Engineering-affinity objective at `score`, wrapping `directive`.
fn scored(id: &str, score: f32, directive: AiDirective) -> ScoredObjective {
    ScoredObjective {
        id: id.into(),
        score,
        directive,
        source: ObjectiveSource::Mission,
        relevance: vec![SystemAffinity::Engineering],
        snapshot: ObjectiveSnapshot {
            progress: None,
            unassigned: false,
            id: id.into(),
            text: String::new(),
            text_params: BTreeMap::new(),
            mandatory: false,
            status: ObjectiveStatus::Active,
            targets: vec![],
            source: ObjectiveSource::Mission,
        },
    }
}

/// An AI-operated transporter host whose viewscreen blackboard carries
/// `scored`, its transport already running iff `transporting`, and the
/// [`TransporterAiEngaged`] marker present iff `engaged`. Returns the world
/// and the operator entity; drive it with `operate_transporter_ai`.
fn backfill_world(
    scored_objectives: Vec<ScoredObjective>,
    transporting: bool,
    engaged: bool,
) -> (World, Entity) {
    let mut world = World::new();
    world.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::default(),
    ));

    let mut sources = ControlSourceResolver::new();
    sources.set(transporter_system_id(), ControlSource::Ai);

    let mut blackboards = ShipSystemBlackboards::default();
    blackboards.0.insert(
        viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            scored_objectives,
            ..Default::default()
        }),
    );

    let mut transporter = transporter();
    transporter.transporting = transporting;
    if transporting {
        transporter.selected_contact = Some("uuid-lighter".into());
    }

    let mut entity = world.spawn((
        EntityUuid("uuid-destroyer".into()),
        ShipSystemControlSources(sources),
        crate::ship_plugin::ShipConfigComponent(
            crate::ship::config::ShipConfig::from_toml(
                r#"
[[station]]
id = "engineering"
name = "Engineering"
description = "Operations"
rank = "Ens."
[[console]]
id = "transport-screen"
home_station = "engineering"
[[console.feed]]
source = "viewscreen"
kind = "Viewscreen"
[[system]]
id = "transporter"
kind = "transporter"
console = "transport-screen"
ai_only = true
"#,
                &["transporter"],
            )
            .unwrap(),
        ),
        transporter,
        blackboards,
        AdmittedCommands(vec![]),
    ));
    if engaged {
        entity.insert(TransporterAiEngaged);
    }
    let operator = entity.id();
    (world, operator)
}

/// The payloads the host admitted this run, in order.
fn admitted_payloads(world: &mut World, operator: Entity) -> Vec<SystemControlPayload> {
    world
        .get::<AdmittedCommands>(operator)
        .unwrap()
        .0
        .iter()
        .map(|c| c.payload.clone())
        .collect()
}

#[test]
fn backfill_selects_and_starts_off_a_rescue_directive() {
    let (mut world, operator) = backfill_world(
        vec![scored(
            "rescue",
            5.0,
            AiDirective::Rescue {
                target: "uuid-lighter".into(),
            },
        )],
        false,
        false,
    );
    world
        .run_system_once(operate_transporter_ai)
        .expect("the backfill host runs");

    assert_eq!(
        admitted_payloads(&mut world, operator),
        vec![
            SystemControlPayload::TransportSelectContact {
                uuid: "uuid-lighter".into(),
            },
            SystemControlPayload::StartTransport,
        ],
        "a Rescue directive makes Backfill select the contact and start the transport",
    );
    assert!(
        world.get::<TransporterAiEngaged>(operator).is_some(),
        "the host claims the seat it just engaged",
    );
}

#[test]
fn backfill_cannot_read_a_rescue_from_an_unbound_console() {
    let (mut world, operator) = backfill_world(
        vec![scored(
            "rescue",
            5.0,
            AiDirective::Rescue {
                target: "uuid-lighter".into(),
            },
        )],
        false,
        false,
    );
    world
        .get_mut::<crate::ship_plugin::ShipConfigComponent>(operator)
        .unwrap()
        .0
        .consoles[0]
        .feeds
        .clear();
    world.run_system_once(operate_transporter_ai).unwrap();
    assert!(admitted_payloads(&mut world, operator).is_empty());
    assert!(world.get::<TransporterAiEngaged>(operator).is_none());
}

#[test]
fn backfill_stops_when_the_rescue_directive_is_withdrawn() {
    // A transport this host started (engaged marker present) with no directive
    // left in the pool: the host lets go.
    let (mut world, operator) = backfill_world(vec![], true, true);
    world
        .run_system_once(operate_transporter_ai)
        .expect("the backfill host runs");

    assert_eq!(
        admitted_payloads(&mut world, operator),
        vec![SystemControlPayload::StopTransport],
        "a withdrawn Rescue directive stops the transport this host started",
    );
    assert!(
        world.get::<TransporterAiEngaged>(operator).is_none(),
        "the host releases the seat it stood down from",
    );
}

#[test]
fn backfill_defers_a_rescue_to_a_higher_scored_tractor_obligation() {
    // Both orders sit on the one Engineering seat; the life-saving Stabilise
    // outscores the rescue, so the transporter host must stand down and admit
    // nothing — the tractor keeps the seat (acceptance criterion #4).
    let (mut world, operator) = backfill_world(
        vec![
            scored(
                "stabilise",
                9.0,
                AiDirective::Stabilise {
                    target: "uuid-tender".into(),
                },
            ),
            scored(
                "rescue",
                4.0,
                AiDirective::Rescue {
                    target: "uuid-lighter".into(),
                },
            ),
        ],
        false,
        false,
    );
    world
        .run_system_once(operate_transporter_ai)
        .expect("the backfill host runs");

    assert!(
        admitted_payloads(&mut world, operator).is_empty(),
        "a higher-scored tractor obligation defers the rescue — the host admits nothing",
    );
    assert!(
        world.get::<TransporterAiEngaged>(operator).is_none(),
        "the deferring host never claims the seat",
    );
}

#[test]
fn selecting_a_contact_resets_progress_and_intent() {
    let mut world = World::new();
    world.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    let operator = world
        .spawn((
            EntityUuid("uuid-destroyer".into()),
            transporter(),
            Transform::default(),
            admitted(SystemControlPayload::TransportSelectContact {
                uuid: "uuid-lighter".into(),
            }),
        ))
        .id();
    world
        .run_system_once(handle_transporter_commands)
        .expect("select applies");
    let t = world.get::<Transporter>(operator).unwrap();
    assert_eq!(t.selected_contact.as_deref(), Some("uuid-lighter"));
    assert!(!t.transporting);
    assert_eq!(t.progress, 0.0);
}

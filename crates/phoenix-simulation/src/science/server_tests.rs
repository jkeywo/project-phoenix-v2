use super::*;
use crate::science::scan::ScanBandConfig;
use bevy::ecs::system::RunSystemOnce;

const SHIP: &str = "ship-1";
/// The depot's private physical route; admitted commands use its observed alias.
const DEPOT: &str = "depot-1";
/// The depot's authored `[[entity]] id` — deliberately NOT its UUID, so the
/// mirror-flag tests below fail if the key ever slips back onto the UUID no
/// scenario author can type.
const DEPOT_ID: &str = "skyway_depot";
/// The depot's authored mass (issue #1154) — a distinctive number so a test
/// asserting on it cannot pass by accident against
/// `entity_config::DEFAULT_ENTITY_MASS`.
const DEPOT_MASS: f32 = 180_000.0;

fn suite() -> ScanConfig {
    ScanConfig {
        power_group: "shields".into(),
        min_power_level: 1,
        bands: vec![
            ScanBandConfig {
                id: "detailed".into(),
                label: "world.probe.band.detailed.label".into(),
                max_range: 500.0,
                condition_step: 0.01,
                report_thresholds: true,
                report_capacities: true,
            },
            ScanBandConfig {
                id: "coarse".into(),
                label: "world.probe.band.coarse.label".into(),
                max_range: 3000.0,
                condition_step: 0.25,
                report_thresholds: true,
                report_capacities: false,
            },
        ],
        degraded_by: Vec::new(),
        interference_bands: 1,
        mass_classes: Vec::new(),
    }
}

fn depot_condition(condition: f32) -> InfrastructureCondition {
    use crate::infrastructure::{
        CapacityConfig, InfrastructureConfig, InfrastructureState, ThresholdConfig,
    };
    InfrastructureCondition(InfrastructureState::from_config(&InfrastructureConfig {
        condition_max: 100.0,
        condition: Some(condition),
        capacities: vec![CapacityConfig {
            id: "depot_berths".into(),
            amount: 4,
            label: Some("world.probe.capacity.berths.label".into()),
            ceiling: None,
        }],
        thresholds: vec![ThresholdConfig {
            flag: "depot_transfer_capable".into(),
            capacity: None,
            fails_below: 0.4,
            restores_above: None,
            label: Some("world.probe.threshold.transfer.label".into()),
        }],
        ..InfrastructureConfig::default()
    }))
}
fn observable_subject(
    tag: &str,
) -> (
    crate::entities::spawner::EntityTagsSection,
    crate::entities::spawner::EntityTarget,
) {
    (
        crate::entities::spawner::EntityTagsSection(vec![tag.into()]),
        crate::entities::spawner::EntityTarget(crate::entities::target::TargetSection {
            tags: vec![tag.into()],
            threat_level: Default::default(),
            description: None,
        }),
    )
}

/// A minimal app: the two systems, one scanning ship at the origin and one
/// depot 200 units away.
fn app_with(config: ScanConfig, condition: f32, depot_x: f32) -> (App, Entity) {
    let mut app = App::new();
    app.add_message::<crate::lobby::server::OutboundMessage>()
        .add_systems(Update, (tick_scans, publish_scan_blackboard).chain());
    // The world's flag store, so the mirror (issue #1038) has somewhere to
    // land. Every test in this module reads it or ignores it; the one below
    // that builds its own bare `App` deliberately leaves it out, which is
    // the `None` arm every fixture in the crate takes.
    app.insert_resource(WorldContentRuntime::default());
    let mut authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    // The independent observation rig extends beyond the unchanged focused
    // bands, so observation does not erase the focused OutOfRange refusal.
    // Its explicit 100ms cadence matches this fixture's fixed step; enquiry
    // timing/range/fidelity still comes from the actual ScanConfig above.
    authored
        .sensors_console
        .as_mut()
        .unwrap()
        .long_range_radar
        .range = config
        .bands
        .iter()
        .map(|band| band.max_range)
        .fold(0.0, f32::max)
        * 2.0;
    authored.scan = Some(config);
    let sensing = authored.sensing.as_mut().unwrap();
    sensing
        .broad
        .retain(|profile| profile.system.0 == "sensor-radar");
    sensing.broad[0].sample_interval_secs = 0.1;
    sensing.focused[0].work_secs = 0.1;
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    app.insert_resource(time)
        .insert_resource(Time::<Fixed>::from_seconds(0.1))
        .insert_resource(crate::sim_tick::SimTick(0));
    let ship = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &authored,
        Vec3::ZERO,
        SHIP.into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut().spawn((
        EntityUuid(DEPOT.to_string()),
        observable_subject("station"),
        crate::entities::spawner::EntityId(DEPOT_ID.to_string()),
        Transform::from_xyz(depot_x, 0.0, 0.0),
        EntityName("world.probe.entity.depot.name".to_string()),
        depot_condition(condition),
        crate::entities::spawner::EntityMass(DEPOT_MASS),
    ));
    (app, ship)
}

/// Set up a real paid current observation without stepping the focused owner.
/// The command boundary then sees only the instrument-issued contact handle.
fn observed_handle(app: &mut App, ship: Entity, source: &str) -> String {
    let radar = SystemId("sensor-radar".into());
    let tick = app.world().resource::<crate::sim_tick::SimTick>().0;
    let due = app
        .world()
        .get::<crate::science::broad::BroadTracks>(ship)
        .unwrap()
        .0[&radar]
        .last_tick
        != Some(tick);
    if due {
        let paid = app
            .world_mut()
            .get_mut::<crate::ship::power::ShipPowerSystem>(ship)
            .unwrap()
            .0
            .work_continuous(&radar, 0.1, 1.0)
            .unwrap()
            .fraction;
        assert!(
            paid > 0.0,
            "finite actual radar reserve funds this fixture's observation"
        );
        app.world_mut()
            .run_system_once(crate::science::broad::tick)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
            .unwrap();
    }
    let handle = app
        .world()
        .get::<crate::science::broad::BroadTracks>(ship)
        .unwrap()
        .0[&radar]
        .contact_for_source(source)
        .map(str::to_owned);
    let exists = app
        .world_mut()
        .query::<&EntityUuid>()
        .iter(app.world())
        .any(|id| id.0 == source);
    if exists {
        handle.expect("the real fixture target must have an actual permitted measured return")
    } else {
        "12:sensor-radar:contact-999999".into()
    }
}

fn ask_for_scan(app: &mut App, ship: Entity, uuid: &str) {
    let handle = observed_handle(app, ship, uuid);
    app.world_mut()
        .get_mut::<AdmittedCommands>(ship)
        .expect("the ship has an admitted set")
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: SystemId(crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string()),
            payload: SystemControlPayload::ScanTarget {
                uuid: handle,
                aim: Default::default(),
                intrusive: false,
            },
            response_token: None,
            feedback_correlation: None,
        });
}

fn ask_for_correlated_scan(
    app: &mut App,
    ship: Entity,
    uuid: &str,
    token: &str,
    correlation: &str,
) {
    let handle = observed_handle(app, ship, uuid);
    app.world_mut()
        .get_mut::<AdmittedCommands>(ship)
        .expect("the ship has an admitted set")
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: SystemId(crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string()),
            payload: SystemControlPayload::ScanTarget {
                uuid: handle,
                aim: Default::default(),
                intrusive: false,
            },
            response_token: Some(token.to_string()),
            feedback_correlation: Some(
                crate::core::messages::ActionCorrelationId::new(correlation)
                    .expect("valid test correlation"),
            ),
        });
}

fn has_feedback(
    messages: &[crate::lobby::server::OutboundMessage],
    token: &str,
    correlation: &str,
    outcome: ActionFeedbackOutcome,
) -> bool {
    messages.iter().any(|message| {
        message.target == crate::lobby::Target::Token(token.to_string())
            && message.delivery == crate::core::messages::DeliveryClass::Reliable
            && matches!(
                &message.msg,
                crate::core::messages::ServerMessage::ActionFeedback {
                    correlation: actual,
                    outcome: actual_outcome,
                } if actual.as_str() == correlation && *actual_outcome == outcome
            )
    })
}

fn record(app: &App, ship: Entity) -> FocusedScanRecord {
    app.world()
        .get::<ShipScanRecord>(ship)
        .expect("the ship keeps a scan record")
        .0[&SystemId("sensors".into())]
        .clone()
}

fn step(app: &mut App) {
    app.update();
    let mut admitted = app.world_mut().query::<&mut AdmittedCommands>();
    for mut commands in admitted.iter_mut(app.world_mut()) {
        commands.0.clear();
    }
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 += 1;
}
fn scan_key() -> SystemId {
    scan_blackboard_key_for(&SystemId("sensors".into()))
}

/// **AC1.** The command arrives through `AdmittedCommands` on the `sensors`
/// system and produces a reading off the target's real condition.
#[test]
fn an_admitted_scan_command_reads_the_targets_live_condition_track() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    let mut cursor = app
        .world()
        .resource::<Messages<crate::lobby::server::OutboundMessage>>()
        .get_cursor();
    ask_for_correlated_scan(&mut app, ship, DEPOT, "science", "science-scan-applied");
    step(&mut app);

    let feedback: Vec<_> = cursor
        .read(
            app.world()
                .resource::<Messages<crate::lobby::server::OutboundMessage>>(),
        )
        .cloned()
        .collect();
    assert!(has_feedback(
        &feedback,
        "science",
        "science-scan-applied",
        ActionFeedbackOutcome::Applied,
    ));

    let reading = record(&app, ship).last.expect("a reading came back");
    assert_eq!(reading.subject_uuid, DEPOT);
    assert_ne!(reading.contact_id.as_deref(), Some(DEPOT));
    assert!(reading.contact_id.is_some());
    assert_eq!(reading.subject_name, "world.probe.entity.depot.name");
    assert_eq!(reading.band, "detailed");
    assert!(
        (reading.condition_fraction.unwrap() - 0.62).abs() < 1e-6,
        "62 of 100 points, read at whole-percent fidelity: {}",
        reading.condition_fraction.unwrap()
    );
    assert_eq!(
        reading.flags,
        vec![("world.probe.threshold.transfer.label".to_string(), true)]
    );
    assert_eq!(
        reading.capacities,
        vec![("world.probe.capacity.berths.label".to_string(), 4)]
    );
}

#[test]
fn same_tick_requests_share_one_paid_observation_and_do_not_replay_after_apply() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    let paid = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .capture_continuation();
    ask_for_scan(&mut app, ship, DEPOT);
    assert_eq!(
        app.world()
            .get::<crate::ship::power::ShipPowerSystem>(ship)
            .unwrap()
            .0
            .capture_continuation(),
        paid
    );
    step(&mut app);
    let first = record(&app, ship).save_state();
    assert_eq!(first.history.len(), 1);
    step(&mut app);
    assert_eq!(record(&app, ship).save_state(), first);
}

/// Issue #1154: the reading carries the subject's authored mass, off its
/// `EntityMass` component — end to end, from the spawned entity through
/// `AdmittedCommands` to the stored `ShipScanRecord`.
#[test]
fn a_scan_reading_reports_the_subjects_authored_mass() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);

    let reading = record(&app, ship).last.expect("a reading came back");
    assert_eq!(reading.mass.unwrap(), DEPOT_MASS);
}

/// A subject spawned without an `EntityMass` component (every real hull has
/// one — this only happens to a hand-built test fixture) still reads out a
/// real, positive mass rather than zero: [`subject_mass`] falls back to the
/// same documented default an unauthored TOML gets.
#[test]
fn a_subject_with_no_entity_mass_component_falls_back_to_the_documented_default() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    let depot = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find_map(|(entity, id)| (id.0 == DEPOT).then_some(entity))
        .unwrap();
    app.world_mut()
        .entity_mut(depot)
        .remove::<crate::entities::spawner::EntityMass>();
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);

    let reading = record(&app, ship).last.expect("a reading came back");
    assert_eq!(
        reading.mass.unwrap(),
        crate::entities::config::DEFAULT_ENTITY_MASS
    );
    assert!(
        reading.mass.unwrap() > 0.0,
        "the fallback must never be zero"
    );
}

/// **AC2 through the ECS.** The SAME command, on a depot whose condition
/// something moved in between, reads out differently — with no content edit
/// and no change to the scanning hull.
#[test]
fn mutating_the_subjects_condition_changes_what_the_next_scan_says() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);
    let before = record(&app, ship).last.expect("a reading");

    // The storm takes thirty points off it. Nothing else in the world moves.
    {
        let mut q = app
            .world_mut()
            .query::<(&EntityUuid, &mut InfrastructureCondition)>();
        let mut condition = q
            .iter_mut(app.world_mut())
            .find_map(|(id, condition)| (id.0 == DEPOT).then_some(condition))
            .expect("the depot carries a track");
        condition.0.degrade(30.0);
    }
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);
    let after = record(&app, ship).last.expect("a second reading");

    assert!(
        (before.condition_fraction.unwrap() - 0.62).abs() < 1e-6
            && (after.condition_fraction.unwrap() - 0.32).abs() < 1e-6,
        "the two readings are {} and {} — the readout is the track",
        before.condition_fraction.unwrap(),
        after.condition_fraction.unwrap()
    );
    assert_eq!(
        after.flags,
        vec![("world.probe.threshold.transfer.label".to_string(), false)],
        "…and the operational flag the drop knocked out reads out as down"
    );
}

/// **AC5 through the ECS.** Fly out and the same command comes back
/// coarser, off the hull's own authored bands.
#[test]
fn the_same_scan_from_further_out_comes_back_at_a_coarser_band() {
    let (mut app, ship) = app_with(suite(), 62.0, 2_400.0);
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);

    let reading = record(&app, ship).last.expect("a reading");
    assert_eq!(reading.band, "coarse");
    assert_eq!(
        reading.condition_fraction.unwrap(),
        0.5,
        "0.62 to the nearest quarter"
    );
    assert!(
        reading.capacities.is_empty(),
        "the coarse band does not claim to count berths"
    );
}

#[test]
fn an_observed_contact_beyond_the_focused_bands_is_refused_as_out_of_range() {
    let (mut app, ship) = app_with(suite(), 62.0, 3500.0);
    ask_for_scan(&mut app, ship, DEPOT);
    let before = app
        .world()
        .get::<crate::ship::power::ShipPowerSystem>(ship)
        .unwrap()
        .0
        .capture_continuation();
    step(&mut app);
    let record = record(&app, ship);
    assert_eq!(record.refusal, Some(ScanRefusal::OutOfRange));
    assert!(record.last.is_none());
    assert!(queued_flag_sets(&app).is_empty());
    assert_eq!(
        app.world()
            .get::<crate::ship::power::ShipPowerSystem>(ship)
            .unwrap()
            .0
            .capture_continuation(),
        before,
        "a correctly observed but unreachable focused target buys no enquiry work"
    );
}

/// **AC1's refusal half, through the ECS.** A target with no condition
/// track at all is refused, and the refusal — not a stale reading — is what
/// the console gets.
#[test]
fn scanning_something_with_no_condition_track_is_refused_with_a_reason() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    let rock = app
        .world_mut()
        .spawn((
            EntityUuid("rock-9".to_string()),
            observable_subject("asteroid"),
            Transform::from_xyz(50.0, 0.0, 0.0),
        ))
        .id();
    assert!(app.world().get::<InfrastructureCondition>(rock).is_none());

    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);
    assert!(record(&app, ship).last.is_some(), "precondition: a reading");

    ask_for_scan(&mut app, ship, "rock-9");
    step(&mut app);
    let record = record(&app, ship);
    assert_eq!(record.refusal, Some(ScanRefusal::NoReadableCondition));
    assert!(
        record.last.is_none(),
        "a refusal clears the previous reading rather than leaving one on \
             screen beside a complaint about a different target"
    );
}

/// A uuid nothing answers to is its own refusal, distinct from a target
/// that exists and has nothing to read.
#[test]
fn scanning_a_uuid_no_entity_answers_to_is_refused_as_no_such_target() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    let mut cursor = app
        .world()
        .resource::<Messages<crate::lobby::server::OutboundMessage>>()
        .get_cursor();
    ask_for_correlated_scan(
        &mut app,
        ship,
        "not-in-this-world",
        "science",
        "science-scan-refused",
    );
    step(&mut app);
    let feedback: Vec<_> = cursor
        .read(
            app.world()
                .resource::<Messages<crate::lobby::server::OutboundMessage>>(),
        )
        .cloned()
        .collect();
    assert!(has_feedback(
        &feedback,
        "science",
        "science-scan-refused",
        ActionFeedbackOutcome::Refused,
    ));
    assert_eq!(record(&app, ship).refusal, Some(ScanRefusal::NoSuchTarget));
}

/// **The leak rule through the ECS.** A `publish = false` structure is
/// refused with the identical reason a bare rock gets, and nothing about
/// its real 31 points reaches the record or the wire.
#[test]
fn a_structure_the_scenario_keeps_off_the_wire_cannot_be_scanned() {
    use crate::infrastructure::{InfrastructureConfig, InfrastructureState};

    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    app.world_mut().spawn((
        EntityUuid("sealed-1".to_string()),
        observable_subject("station"),
        Transform::from_xyz(100.0, 0.0, 0.0),
        EntityName("world.probe.entity.sealed.name".to_string()),
        InfrastructureCondition(InfrastructureState::from_config(&InfrastructureConfig {
            condition_max: 100.0,
            condition: Some(31.0),
            publish: false,
            ..InfrastructureConfig::default()
        })),
    ));

    ask_for_scan(&mut app, ship, "sealed-1");
    step(&mut app);
    let record = record(&app, ship);
    assert_eq!(
        record.refusal,
        Some(ScanRefusal::NoReadableCondition),
        "the same answer an unreadable rock gets"
    );
    assert!(record.last.is_none(), "and the withheld 0.31 is on nothing");
}

/// A hull with no `[scan]` table still answers when it is asked.
#[test]
fn a_hull_that_cannot_scan_is_refused_by_name_rather_than_ignored() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    app.world_mut().entity_mut(ship).remove::<ShipScanRecord>();
    let mut cursor = app
        .world()
        .resource::<Messages<crate::lobby::server::OutboundMessage>>()
        .get_cursor();
    ask_for_correlated_scan(&mut app, ship, DEPOT, "science", "missing-instrument");
    step(&mut app);
    assert!(
        app.world().get::<ShipScanRecord>(ship).is_none(),
        "a refusal must not manufacture an observation owner"
    );
    let feedback: Vec<_> = cursor
        .read(
            app.world()
                .resource::<Messages<crate::lobby::server::OutboundMessage>>(),
        )
        .cloned()
        .collect();
    assert!(has_feedback(
        &feedback,
        "science",
        "missing-instrument",
        ActionFeedbackOutcome::Refused
    ));
}

/// The reading reaches the wire under the `scan` channel, with the string
/// ids the console resolves and no English on any of them.
#[test]
fn the_reading_is_published_under_the_scan_blackboard_channel() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);

    let boards = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .expect("the ship publishes");
    let bb = match boards.0.get(&scan_key()) {
        Some(SystemBlackboard::Scan(bb)) => bb.clone(),
        other => panic!("expected a scan blackboard, got {other:?}"),
    };
    assert!(bb.capable);
    assert!(bb.refusal.is_none());
    let reading = bb.reading.expect("the reading is on the wire");
    assert_ne!(reading.subject_uuid, DEPOT);
    assert_eq!(
        app.world()
            .get::<crate::science::broad::BroadTracks>(ship)
            .unwrap()
            .0[&SystemId("sensor-radar".into())]
            .resolve_current(&reading.subject_uuid),
        Some(DEPOT)
    );
    assert_eq!(reading.band_label, "world.probe.band.detailed.label");
    assert_eq!(reading.subject_name, "world.probe.entity.depot.name");
    assert_eq!(reading.condition_step.unwrap(), 0.01);
}

#[test]
fn forecasts_use_only_retained_readings_and_rebuild_after_derived_board_loss() {
    let (mut app, ship) = app_with(suite(), 80.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);
    let first = app.world().get::<ShipScanRecord>(ship).unwrap().0[&SystemId("sensors".into())]
        .last
        .clone()
        .unwrap();
    let depot = app
        .world_mut()
        .query::<(Entity, &EntityUuid)>()
        .iter(app.world())
        .find_map(|(entity, id)| (id.0 == DEPOT).then_some(entity))
        .unwrap();
    app.world_mut()
        .entity_mut(depot)
        .insert(depot_condition(60.0));
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);
    let board = |app: &App| match &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0[&scan_key()]
    {
        SystemBlackboard::Scan(board) => board.clone(),
        _ => panic!("actual scan projection"),
    };
    let observed = board(&app);
    let answer = observed
        .reading
        .as_ref()
        .unwrap()
        .forecast
        .as_ref()
        .unwrap();
    assert_eq!(answer.sample_ticks[0], first.taken_at_tick);
    assert_eq!(
        answer.sample_ticks[1],
        observed.reading.as_ref().unwrap().taken_at_tick
    );
    assert_eq!(answer.horizon_secs, 120.0);
    // Changing actual state without another enquiry cannot improve/change this
    // forecast, and losing only the derived projection must rebuild identically.
    app.world_mut()
        .entity_mut(depot)
        .insert(depot_condition(99.0));
    app.world_mut()
        .get_mut::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .remove(&scan_key());
    app.world_mut()
        .run_system_once(publish_scan_blackboard)
        .unwrap();
    assert_eq!(board(&app), observed);
    // The issued projection is canonical evidence, even if its input sample is
    // later pruned from bounded history. It is not recomputed after the fact.
    let saved = app.world().get::<ShipScanRecord>(ship).unwrap().0[&SystemId("sensors".into())]
        .save_state();
    let mut pruned = saved.clone();
    pruned.history.remove(0);
    app.world_mut()
        .get_mut::<ShipScanRecord>(ship)
        .unwrap()
        .0
        .get_mut(&SystemId("sensors".into()))
        .unwrap()
        .restore(&pruned);
    app.world_mut()
        .run_system_once(publish_scan_blackboard)
        .unwrap();
    assert_eq!(board(&app).reading, observed.reading);
    let mut invalid = pruned;
    invalid.history[0].forecast.as_mut().unwrap().ranges[0].upper = f64::INFINITY;
    invalid.last = invalid.history.last().cloned();
    assert!(
        app.world().get::<ShipScanRecord>(ship).unwrap().0[&SystemId("sensors".into())]
            .validate_state(&invalid, u64::MAX)
            .is_err()
    );
}

/// A refusal reaches the wire as its own `strings.csv` id.
#[test]
fn a_refusal_is_published_as_a_string_id_rather_than_as_prose() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, "not-in-this-world");
    step(&mut app);

    let boards = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .expect("the ship publishes");
    let bb = match boards.0.get(&scan_key()) {
        Some(SystemBlackboard::Scan(bb)) => bb.clone(),
        other => panic!("expected a scan blackboard, got {other:?}"),
    };
    assert_eq!(bb.refusal.as_deref(), Some("scan.refusal.no_such_target"));
    assert!(bb.reading.is_none());
}

// ── The mirror flag (issue #1038) ───────────────────────────────────────

fn runtime(app: &App) -> &WorldContentRuntime {
    app.world().resource::<WorldContentRuntime>()
}

/// Every `FlagSet` this run has queued, in order.
fn queued_flag_sets(app: &App) -> Vec<String> {
    runtime(app)
        .pending_world_events
        .iter()
        .filter_map(|e| match e {
            WorldEvent::FlagSet { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// **Issue #1038's engine seam.** A reading that comes back raises the
/// subject's own `scan.<id>.taken` in the world flag store and queues the
/// `FlagSet` a scenario's `on_flag_set` hook fires from — keyed on the
/// world's authored id, which is the only spelling an author can write.
#[test]
fn a_reading_that_comes_back_raises_the_subjects_scanned_flag() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    assert_eq!(runtime(&app).flags.counter(&scanned_flag(DEPOT_ID)), 0);

    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);

    assert_eq!(
        runtime(&app).flags.counter(&scanned_flag(DEPOT_ID)),
        1,
        "the crew have now read this structure, and a script can ask so"
    );
    assert_eq!(queued_flag_sets(&app), vec![scanned_flag(DEPOT_ID)]);
    assert_eq!(
        runtime(&app).flags.counter(&scanned_flag(DEPOT)),
        0,
        "and nothing is keyed on the minted UUID, which no scenario can type"
    );
}

/// A structure the world authored no `id` for is one no scenario can name,
/// so it is read and nothing is mirrored. Never a `scan..taken`.
#[test]
fn a_structure_with_no_authored_id_is_scannable_and_mirrors_nothing() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    app.world_mut().spawn((
        EntityUuid("anonymous-1".to_string()),
        observable_subject("station"),
        Transform::from_xyz(150.0, 0.0, 0.0),
        EntityName("world.probe.entity.anonymous.name".to_string()),
        depot_condition(44.0),
    ));

    ask_for_scan(&mut app, ship, "anonymous-1");
    step(&mut app);

    assert!(
        record(&app, ship).last.is_some(),
        "the reading still comes back — the console is owed an answer"
    );
    assert!(queued_flag_sets(&app).is_empty());
    assert_eq!(runtime(&app).flags.counter("scan..taken"), 0);
}

/// It LATCHES. A second reading of the same structure does not queue a
/// second event — an ordinary re-scan must not re-fire a beat — and reading
/// something else afterwards does not unlearn the first.
#[test]
fn re_scanning_raises_nothing_twice_and_scanning_elsewhere_unlearns_nothing() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    app.world_mut().spawn((
        EntityUuid("depot-2".to_string()),
        observable_subject("station"),
        crate::entities::spawner::EntityId("ladder_depot".to_string()),
        Transform::from_xyz(120.0, 0.0, 0.0),
        EntityName("world.probe.entity.other.name".to_string()),
        depot_condition(55.0),
    ));

    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);
    ask_for_scan(&mut app, ship, "depot-2");
    step(&mut app);

    assert_eq!(
        queued_flag_sets(&app),
        vec![scanned_flag(DEPOT_ID), scanned_flag("ladder_depot")],
        "one event per structure the crew have read, however often they read it"
    );
    assert_eq!(
        runtime(&app).flags.counter(&scanned_flag(DEPOT_ID)),
        1,
        "the console's `last` reading has moved on to the other depot; what \
             the crew KNOW they have looked at has not"
    );
}

/// A refusal raises nothing. Being told there is nothing to read is not
/// having read it, and a scenario that fired its comparison off a refused
/// scan would be firing off "the player pressed the button".
#[test]
fn a_refused_scan_raises_no_flag_for_the_thing_it_could_not_read() {
    use crate::infrastructure::{InfrastructureConfig, InfrastructureState};

    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    app.world_mut().spawn((
        EntityUuid("sealed-1".to_string()),
        observable_subject("station"),
        crate::entities::spawner::EntityId("sealed_depot".to_string()),
        Transform::from_xyz(100.0, 0.0, 0.0),
        EntityName("world.probe.entity.sealed.name".to_string()),
        InfrastructureCondition(InfrastructureState::from_config(&InfrastructureConfig {
            condition_max: 100.0,
            condition: Some(31.0),
            publish: false,
            ..InfrastructureConfig::default()
        })),
    ));

    ask_for_scan(&mut app, ship, "sealed-1");
    step(&mut app);
    ask_for_scan(&mut app, ship, "not-in-this-world");
    step(&mut app);

    assert_eq!(
        runtime(&app).flags.counter(&scanned_flag("sealed_depot")),
        0
    );
    assert!(
        queued_flag_sets(&app).is_empty(),
        "neither refusal is an act of reading"
    );
}

/// The save projection carries the reading and the refusal and leaves the
/// authored table behind.
#[test]
fn the_save_projection_carries_the_reading_and_not_the_authored_table() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);

    let live = record(&app, ship);
    let saved = live.save_state();
    assert_eq!(saved.last, live.last);
    assert!(saved.refusal.is_none());

    let mut restored = live.clone();
    restored.last = None;
    restored.history.clear();
    restored.restore(&saved);
    assert_eq!(restored, live, "the whole record comes back");
}

#[test]
fn held_scan_metadata_keeps_its_sample_and_precision_after_the_clock_advances() {
    use phoenix_model::observation::{ObservationSource, ObservationUncertainty};
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    app.insert_resource(crate::sim_tick::SimTick(17));
    ask_for_scan(&mut app, ship, DEPOT);
    step(&mut app);
    let first = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0[&scan_key()]
        .clone();
    app.world_mut()
        .get_mut::<AdmittedCommands>(ship)
        .unwrap()
        .0
        .clear();
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 90;
    step(&mut app);
    let current = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .unwrap()
        .0[&scan_key()];
    assert_eq!(
        *current, first,
        "publishing/forwarding does not renew a held reading"
    );
    let SystemBlackboard::Scan(board) = current else {
        panic!("Scan publication");
    };
    let reading = board.reading.as_ref().unwrap();
    let metadata = reading.observation.as_ref().unwrap();
    assert_eq!(metadata.observed_tick, Some(17));
    assert_eq!(metadata.age_ticks(90), Some(73));
    assert_eq!(metadata.provenance.kind, ObservationSource::Scan);
    assert_eq!(
        metadata.uncertainty,
        ObservationUncertainty::Quantized {
            field: "condition_fraction".into(),
            step: reading.condition_step.unwrap()
        }
    );
}

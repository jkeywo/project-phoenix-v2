use super::*;
use crate::science::scan::ScanBandConfig;

const SHIP: &str = "ship-1";
/// The depot's minted UUID — what a command and a reading join on.
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
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(SHIP.to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            AdmittedCommands::default(),
            ShipScanRecord {
                config,
                ..Default::default()
            },
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();
    app.world_mut().spawn((
        EntityUuid(DEPOT.to_string()),
        crate::entities::spawner::EntityId(DEPOT_ID.to_string()),
        Transform::from_xyz(depot_x, 0.0, 0.0),
        EntityName("world.probe.entity.depot.name".to_string()),
        depot_condition(condition),
        crate::entities::spawner::EntityMass(DEPOT_MASS),
    ));
    (app, ship)
}

fn ask_for_scan(app: &mut App, ship: Entity, uuid: &str) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(ship)
        .expect("the ship has an admitted set")
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: SystemId(crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string()),
            payload: SystemControlPayload::ScanTarget {
                uuid: uuid.to_string(),
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
    app.world_mut()
        .get_mut::<AdmittedCommands>(ship)
        .expect("the ship has an admitted set")
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: SystemId(crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string()),
            payload: SystemControlPayload::ScanTarget {
                uuid: uuid.to_string(),
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

fn record(app: &App, ship: Entity) -> ShipScanRecord {
    app.world()
        .get::<ShipScanRecord>(ship)
        .expect("the ship keeps a scan record")
        .clone()
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
    app.update();

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
    assert_eq!(reading.subject_name, "world.probe.entity.depot.name");
    assert_eq!(reading.band, "detailed");
    assert!(
        (reading.condition_fraction - 0.62).abs() < 1e-6,
        "62 of 100 points, read at whole-percent fidelity: {}",
        reading.condition_fraction
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

/// Issue #1154: the reading carries the subject's authored mass, off its
/// `EntityMass` component — end to end, from the spawned entity through
/// `AdmittedCommands` to the stored `ShipScanRecord`.
#[test]
fn a_scan_reading_reports_the_subjects_authored_mass() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    app.update();

    let reading = record(&app, ship).last.expect("a reading came back");
    assert_eq!(reading.mass, DEPOT_MASS);
}

/// A subject spawned without an `EntityMass` component (every real hull has
/// one — this only happens to a hand-built test fixture) still reads out a
/// real, positive mass rather than zero: [`subject_mass`] falls back to the
/// same documented default an unauthored TOML gets.
#[test]
fn a_subject_with_no_entity_mass_component_falls_back_to_the_documented_default() {
    let mut app = App::new();
    app.add_systems(Update, (tick_scans, publish_scan_blackboard).chain());
    app.insert_resource(WorldContentRuntime::default());
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(SHIP.to_string()),
            Transform::from_xyz(0.0, 0.0, 0.0),
            AdmittedCommands::default(),
            ShipScanRecord {
                config: suite(),
                ..Default::default()
            },
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id();
    // No EntityMass component — the case the fallback in `subject_mass`
    // exists for.
    app.world_mut().spawn((
        EntityUuid(DEPOT.to_string()),
        Transform::from_xyz(200.0, 0.0, 0.0),
        EntityName("world.probe.entity.depot.name".to_string()),
        depot_condition(62.0),
    ));
    ask_for_scan(&mut app, ship, DEPOT);
    app.update();

    let reading = record(&app, ship).last.expect("a reading came back");
    assert_eq!(reading.mass, crate::entities::config::DEFAULT_ENTITY_MASS);
    assert!(reading.mass > 0.0, "the fallback must never be zero");
}

/// **AC2 through the ECS.** The SAME command, on a depot whose condition
/// something moved in between, reads out differently — with no content edit
/// and no change to the scanning hull.
#[test]
fn mutating_the_subjects_condition_changes_what_the_next_scan_says() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    app.update();
    let before = record(&app, ship).last.expect("a reading");

    // The storm takes thirty points off it. Nothing else in the world moves.
    {
        let mut q = app.world_mut().query::<&mut InfrastructureCondition>();
        let mut condition = q
            .iter_mut(app.world_mut())
            .next()
            .expect("the depot carries a track");
        condition.0.degrade(30.0);
    }
    ask_for_scan(&mut app, ship, DEPOT);
    app.update();
    let after = record(&app, ship).last.expect("a second reading");

    assert!(
        (before.condition_fraction - 0.62).abs() < 1e-6
            && (after.condition_fraction - 0.32).abs() < 1e-6,
        "the two readings are {} and {} — the readout is the track",
        before.condition_fraction,
        after.condition_fraction
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
    app.update();

    let reading = record(&app, ship).last.expect("a reading");
    assert_eq!(reading.band, "coarse");
    assert_eq!(
        reading.condition_fraction, 0.5,
        "0.62 to the nearest quarter"
    );
    assert!(
        reading.capacities.is_empty(),
        "the coarse band does not claim to count berths"
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
            Transform::from_xyz(50.0, 0.0, 0.0),
        ))
        .id();
    assert!(app.world().get::<InfrastructureCondition>(rock).is_none());

    ask_for_scan(&mut app, ship, DEPOT);
    app.update();
    assert!(record(&app, ship).last.is_some(), "precondition: a reading");

    ask_for_scan(&mut app, ship, "rock-9");
    app.update();
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
    app.update();
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
    app.update();
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
    let mut app = App::new();
    app.add_systems(Update, tick_scans);
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            EntityUuid(SHIP.to_string()),
            Transform::default(),
            AdmittedCommands::default(),
        ))
        .id();
    ask_for_scan(&mut app, ship, DEPOT);
    app.update();
    assert_eq!(
        app.world()
            .get::<ShipScanRecord>(ship)
            .expect("the refusal record is inserted")
            .refusal,
        Some(ScanRefusal::NotCapable)
    );
}

/// The reading reaches the wire under the `scan` channel, with the string
/// ids the console resolves and no English on any of them.
#[test]
fn the_reading_is_published_under_the_scan_blackboard_channel() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, DEPOT);
    app.update();

    let boards = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .expect("the ship publishes");
    let bb = match boards.0.get(&scan_blackboard_key()) {
        Some(SystemBlackboard::Scan(bb)) => bb.clone(),
        other => panic!("expected a scan blackboard, got {other:?}"),
    };
    assert!(bb.capable);
    assert!(bb.refusal.is_none());
    let reading = bb.reading.expect("the reading is on the wire");
    assert_eq!(reading.band_label, "world.probe.band.detailed.label");
    assert_eq!(reading.subject_name, "world.probe.entity.depot.name");
    assert_eq!(reading.condition_step, 0.01);
}

/// A refusal reaches the wire as its own `strings.csv` id.
#[test]
fn a_refusal_is_published_as_a_string_id_rather_than_as_prose() {
    let (mut app, ship) = app_with(suite(), 62.0, 200.0);
    ask_for_scan(&mut app, ship, "not-in-this-world");
    app.update();

    let boards = app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(ship)
        .expect("the ship publishes");
    let bb = match boards.0.get(&scan_blackboard_key()) {
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
    app.update();

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
        Transform::from_xyz(150.0, 0.0, 0.0),
        EntityName("world.probe.entity.anonymous.name".to_string()),
        depot_condition(44.0),
    ));

    ask_for_scan(&mut app, ship, "anonymous-1");
    app.update();

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
        crate::entities::spawner::EntityId("ladder_depot".to_string()),
        Transform::from_xyz(120.0, 0.0, 0.0),
        EntityName("world.probe.entity.other.name".to_string()),
        depot_condition(55.0),
    ));

    ask_for_scan(&mut app, ship, DEPOT);
    app.update();
    ask_for_scan(&mut app, ship, DEPOT);
    app.update();
    ask_for_scan(&mut app, ship, "depot-2");
    app.update();

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
    app.update();
    ask_for_scan(&mut app, ship, "not-in-this-world");
    app.update();

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
    app.update();

    let live = record(&app, ship);
    let saved = live.save_state();
    assert_eq!(saved.last, live.last);
    assert!(saved.refusal.is_none());

    let mut restored = ShipScanRecord {
        config: suite(),
        ..Default::default()
    };
    restored.restore(&saved);
    assert_eq!(restored, live, "the whole record comes back");
}

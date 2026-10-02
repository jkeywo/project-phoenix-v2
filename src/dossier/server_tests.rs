use super::*;
use crate::core::messages::{CommsContact, DossierSnapshot, DossierValue};
use crate::dossier::projection::{FACT_COMMITMENT_OPEN, FACT_COMMS, FACT_CONDITION};
use crate::infrastructure::{InfrastructureConfig, InfrastructureState};
use crate::server_app::{LocalShip, ShipSystemBlackboards};

/// A bare app with the publisher and one local ship to publish onto.
fn app() -> App {
    let mut app = App::new();
    app.add_systems(Update, publish_dossier_blackboard);
    app.world_mut()
        .spawn((LocalShip, ShipSystemBlackboards::default()));
    app
}

fn published(app: &App) -> DossierBlackboard {
    let mut q = app
        .world()
        .try_query_filtered::<&ShipSystemBlackboards, With<LocalShip>>()
        .expect("query builds");
    let blackboards = q.iter(app.world()).next().expect("the local ship exists");
    match blackboards.0.get(&dossier_blackboard_key()) {
        Some(SystemBlackboard::Dossiers(bb)) => bb.clone(),
        other => panic!("expected a dossier blackboard, got {other:?}"),
    }
}

fn labels(dossier: &DossierSnapshot) -> Vec<&str> {
    dossier.facts.iter().map(|f| f.label.as_str()).collect()
}

fn hailable(app: &mut App, uuid: &str, id: &str) -> Entity {
    app.world_mut()
        .spawn((
            EntityUuid(uuid.into()),
            EntityId(id.into()),
            crate::comms::CommsHailable { display_name: None },
        ))
        .id()
}

fn structure(app: &mut App, uuid: &str, config: InfrastructureConfig) -> Entity {
    app.world_mut()
        .spawn((
            EntityUuid(uuid.into()),
            InfrastructureCondition(InfrastructureState::from_config(&config)),
        ))
        .id()
}

/// **AC2, both doors.** A hailable hull and a publishing structure are each
/// subjects; an entity through neither door is not one. This is the whole
/// roster rule, and it is why nothing had to author a `[dossier]` block.
#[test]
fn the_roster_is_every_hailable_entity_and_every_published_structure() {
    let mut app = app();
    hailable(&mut app, "ship-1", "claimant");
    structure(
        &mut app,
        "skyhook-1",
        InfrastructureConfig {
            condition: Some(60.0),
            ..InfrastructureConfig::default()
        },
    );
    // Neither hailable nor publishing: an asteroid, a marker, most of a world.
    app.world_mut().spawn(EntityUuid("rock-1".into()));
    app.update();

    let subjects = published(&app).subjects;
    assert_eq!(
        subjects.iter().map(|d| d.uuid.as_str()).collect::<Vec<_>>(),
        vec!["ship-1", "skyhook-1"],
        "two doors in, UUID-ordered, and nothing else on the list"
    );
}

/// **AC1 at the adapter.** `publish = false` is #1025's gate and this
/// system is downstream of it: the structure is still a subject (it exists,
/// and the crew can see it out of the window) but it has no condition row,
/// and the withheld number is on no fact of any kind.
#[test]
fn a_structure_kept_off_the_wire_publishes_a_dossier_with_no_condition_on_it() {
    let mut app = app();
    // Hailable, so it is on the roster through the OTHER door — this test is
    // about the fact, not about the subject vanishing.
    let entity = hailable(&mut app, "depot-1", "depot");
    app.world_mut()
        .entity_mut(entity)
        .insert(InfrastructureCondition(InfrastructureState::from_config(
            &InfrastructureConfig {
                condition_max: 100.0,
                condition: Some(31.0),
                publish: false,
                ..InfrastructureConfig::default()
            },
        )));
    app.update();

    let subjects = published(&app).subjects;
    assert_eq!(subjects.len(), 1, "it is still a subject");
    assert!(
        !labels(&subjects[0]).contains(&FACT_CONDITION),
        "but its condition is not on the sheet"
    );
    assert!(
        !subjects[0]
            .facts
            .iter()
            .any(|f| matches!(f.value, DossierValue::Fraction(_))),
        "and 0.31 rides on nothing at all"
    );
}

/// The second gate, at the adapter: a published flag reaches the fact sheet
/// only where the scenario authored a crew-facing label beside it.
#[test]
fn only_labelled_flags_and_capacities_become_rows() {
    use crate::infrastructure::{CapacityConfig, ThresholdConfig};

    let mut app = app();
    structure(
        &mut app,
        "skyhook-1",
        InfrastructureConfig {
            condition: Some(100.0),
            capacities: vec![
                CapacityConfig {
                    ceiling: None,
                    id: "berths".into(),
                    amount: 4,
                    label: Some("world.skyhook.berths.label".into()),
                },
                CapacityConfig {
                    ceiling: None,
                    id: "throughput".into(),
                    amount: 900,
                    label: None,
                },
            ],
            thresholds: vec![ThresholdConfig {
                flag: "transfer_capable".into(),
                capacity: None,
                fails_below: 0.4,
                restores_above: None,
                label: Some("world.skyhook.transfer.label".into()),
            }],
            ..InfrastructureConfig::default()
        },
    );
    app.update();

    let subjects = published(&app).subjects;
    assert_eq!(
        labels(&subjects[0]),
        vec![
            FACT_CONDITION,
            "world.skyhook.transfer.label",
            "world.skyhook.berths.label",
        ],
        "the unlabelled capacity stays a machine number"
    );
}

/// The comms standing is read off the roster the officer is already
/// looking at, so a dossier and the contact list cannot disagree about
/// whether somebody can be called.
#[test]
fn comms_standing_comes_from_the_live_roster() {
    let mut app = app();
    hailable(&mut app, "ship-1", "claimant");
    app.update();
    assert_eq!(
        published(&app).subjects[0].facts[0],
        crate::core::messages::DossierFactSnapshot {
            label: FACT_COMMS.to_string(),
            value: DossierValue::Flag(false),
        },
        "no roster yet: hailable, and not reachable"
    );

    let mut runtime = CommsRuntime::default();
    runtime.contacts.push(CommsContact {
        uuid: "ship-1".into(),
        name: "world.entity.claimant.name".into(),
        in_range: true,
        is_urgent: false,
    });
    app.insert_resource(runtime);
    app.update();
    assert_eq!(
        published(&app).subjects[0].facts[0].value,
        DossierValue::Flag(true),
        "and it follows the roster when the roster moves"
    );
}

/// A promise reaches the party's own sheet, matched on the world's
/// `[[entity]] id` — the way #1029's ledger records a party.
#[test]
fn a_promise_lands_on_the_dossier_of_the_party_it_was_made_to() {
    let mut app = app();
    hailable(&mut app, "ship-1", "skyway_strike_committee");
    hailable(&mut app, "ship-2", "corporate_security");

    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime
        .commitments
        .record(
            "safe_passage",
            "skyway_strike_committee",
            "world.probe.terms",
            "world.probe.resolves",
            12,
        )
        .expect("a fresh id");
    app.insert_resource(runtime);
    app.update();

    let subjects = published(&app).subjects;
    assert_eq!(
        labels(&subjects[0]),
        vec![FACT_COMMS, FACT_COMMITMENT_OPEN],
        "the committee were promised something and it is still owed"
    );
    assert_eq!(
        subjects[0].facts[1].value,
        DossierValue::Text("world.probe.terms".into()),
        "the row carries the TERMS the crew gave, by string id"
    );
    assert_eq!(
        labels(&subjects[1]),
        vec![FACT_COMMS],
        "and nobody else's sheet grew a promise that was not made to them"
    );
}

/// The empty arm every world in the repository is in today: no hailable
/// entities, no published structures, and therefore an empty list — which
/// still publishes, so the panel can render its own empty state rather than
/// the console guessing.
#[test]
fn a_world_with_no_subjects_publishes_an_empty_list() {
    let mut app = app();
    app.world_mut().spawn(EntityUuid("rock-1".into()));
    app.update();
    assert!(published(&app).subjects.is_empty());
}

/// No local ship, nothing to publish onto, and no panic — the arm every
/// lobby tick and every crewless headless run takes.
#[test]
fn a_run_with_no_local_ship_publishes_nothing() {
    let mut app = App::new();
    app.add_systems(Update, publish_dossier_blackboard);
    hailable(&mut app, "ship-1", "claimant");
    app.update();
}

// ── Gathered evidence (issue #1031) ──────────────────────────────────────

/// **AC4 at the adapter.** A finding reaches the subject it was gathered on,
/// matched by UUID, in gather order — and nobody else's file grew a finding
/// about somebody else.
#[test]
fn a_finding_lands_on_the_file_of_the_subject_it_was_gathered_on() {
    use crate::dossier::evidence::EvidenceProvenance;

    let mut app = app();
    hailable(&mut app, "ship-1", "strike_committee");
    hailable(&mut app, "ship-2", "corporate_security");

    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.evidence.append(
        "ship-1",
        "world.probe.evidence.manifest",
        EvidenceProvenance::Records,
        120,
    );
    runtime.evidence.append(
        "ship-1",
        "world.probe.evidence.admission",
        EvidenceProvenance::Dialogue,
        300,
    );
    app.insert_resource(runtime);
    app.update();

    let subjects = published(&app).subjects;
    assert_eq!(
        subjects[0]
            .evidence
            .iter()
            .map(|e| (e.text.as_str(), e.provenance.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("world.probe.evidence.manifest", "records"),
            ("world.probe.evidence.admission", "dialogue"),
        ],
        "both findings, in the order the crew made them"
    );
    assert!(
        subjects[1].evidence.is_empty(),
        "and the other hull's file is untouched"
    );
}

/// Evidence is not a third door onto the roster: a finding written about
/// something the crew have no other surface on has nowhere to be shown, and
/// the subject list is exactly what it was.
#[test]
fn a_finding_about_a_non_subject_does_not_make_it_one() {
    use crate::dossier::evidence::EvidenceProvenance;

    let mut app = app();
    hailable(&mut app, "ship-1", "claimant");
    app.world_mut().spawn(EntityUuid("rock-1".into()));

    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.evidence.append(
        "rock-1",
        "world.probe.evidence.ore",
        EvidenceProvenance::Scan,
        60,
    );
    app.insert_resource(runtime);
    app.update();

    assert_eq!(
        published(&app)
            .subjects
            .iter()
            .map(|d| d.uuid.as_str())
            .collect::<Vec<_>>(),
        vec!["ship-1"],
        "the two doors are still the whole roster rule"
    );
}

//! Bevy adapter for the dossier projection (issue #1030).
//!
//! One system, no state, and no decisions of its own. Which facts a crew may see
//! is [`super::projection`]'s to say; everything here gathers the live inputs
//! that pure sibling cannot reach and publishes what it hands back.
//!
//! # The subject roster is DERIVED, not authored
//!
//! An entity is a dossier subject when the crew *already* has an authoritative
//! surface on it:
//!
//! * it is on the hail roster — `[comms] hailable = true` (#985), the same opt-in
//!   that puts it in front of a Comms officer; or
//! * it publishes an infrastructure condition track — `[infrastructure] publish
//!   = true` (#1025), the same opt-in that puts its condition on the entity
//!   snapshot.
//!
//! There is deliberately no third, dossier-only opt-in. A `[dossier] subject =
//! true` would be a way to declare that the crew hold a file on something they
//! have no other means of observing, which is precisely the shape of leak this
//! slice exists to make impossible. It also means the roster needs no new
//! component and no save field: a dossier is a *view*.
//!
//! **Gathered evidence is not a third door either** (issue #1031). Appending a
//! finding does not make something a subject: the entry lands in
//! [`WorldContentRuntime::evidence`](crate::world::server::WorldContentRuntime)
//! keyed by uuid whatever it was written about, and it reaches a fact sheet only
//! where that uuid is already through one of the two doors above. A scenario
//! that appends a finding to a rock has written it down honestly and has nowhere
//! to show it — which is the same answer #1029 gives a promise made to a party
//! that is not an entity in this world.
//!
//! Ships, stations and structures all reach it through those two doors — a
//! hailable hull, a hailable starbase, a published skyhook — which is the
//! coverage the issue asks for without a per-kind list anywhere in Rust.
//!
//! # Determinism
//!
//! Subjects are walked in UUID order, never archetype order, for the reason
//! `civilian_traffic_rows` and every other authoritative walk sorts: Bevy's
//! iteration order is not part of the simulation's contract, and a list that
//! reordered itself between ticks would move rows under the operator's finger.

use bevy::prelude::*;

use crate::comms::server::CommsRuntime;
use crate::core::messages::{
    DossierBlackboard, InfrastructureSnapshot, SystemBlackboard, SystemId,
};
use crate::entities::spawner::{EntityId, EntityName, EntityTarget, EntityUuid, FactionComponent};
use crate::infrastructure::InfrastructureCondition;

use super::projection::{project, DossierSubject, SubjectCondition};

/// The blackboard channel key dossiers are published under.
///
/// **Not a system id.** No `[[system]]` block declares it, no station owns it,
/// it registers no `ControlSource` and no `ControlSystem` message may target it
/// — a dossier is something the crew *knows*, not a thing aboard the ship. It is
/// carried inside a [`SystemId`] value for `operations`' reason: the blackboard
/// map and the `BlackboardUpdate` wire message are typed that way.
pub const DOSSIER_BLACKBOARD_KEY: &str = "dossiers";

/// The blackboard channel key as a [`SystemId`].
pub fn dossier_blackboard_key() -> SystemId {
    SystemId(DOSSIER_BLACKBOARD_KEY.to_string())
}

/// Registers the publisher. Holds no resource and no component: everything this
/// module produces is derived from state other subsystems already own.
pub struct DossierPlugin;

impl Plugin for DossierPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            publish_dossier_blackboard.in_set(crate::sim_sets::SimSet::Publish),
        );
    }
}

/// Project every subject in the world onto the local ship's dossier channel.
///
/// Read-only over the world; the only thing it writes is the blackboard entry,
/// and only when the picture actually changed — `ShipSystemBlackboards` feeds
/// the diffed `BlackboardUpdate` broadcast, so an unchanged intelligence picture
/// costs nothing on the wire.
///
/// Only the **local** ship carries the channel. An NPC's dossiers would be a
/// second copy of the same world picture with no console to render it on.
pub fn publish_dossier_blackboard(
    subjects_q: Query<(
        &EntityUuid,
        Option<&EntityId>,
        Option<&EntityName>,
        Option<&EntityTarget>,
        Option<&FactionComponent>,
        Option<&crate::comms::CommsHailable>,
        Option<&InfrastructureCondition>,
    )>,
    factions: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    comms: Option<Res<CommsRuntime>>,
    world_runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut ships: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    // Nothing to publish to. Every headless run with no local ship, and every
    // lobby tick before one is spawned, takes this arm.
    if ships.is_empty() {
        return;
    }

    let subjects = dossier_subjects(
        &subjects_q,
        factions.as_deref(),
        comms.as_deref(),
        world_runtime.as_deref(),
    );
    let blackboard = SystemBlackboard::Dossiers(DossierBlackboard {
        subjects: subjects.iter().map(project).collect(),
    });
    let key = dossier_blackboard_key();
    for mut blackboards in ships.iter_mut() {
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key.clone(), blackboard.clone());
        }
    }
}

/// Gather one [`DossierSubject`] per subject, in UUID order.
///
/// Every `Option` here is a real world state, not defensive plumbing: a bare
/// `App` fixture has no faction registry, a world with no comms content has no
/// `CommsRuntime`, and a lobby tick has no content runtime. Each missing
/// resource costs exactly the facts it sources and nothing else.
fn dossier_subjects(
    subjects_q: &Query<(
        &EntityUuid,
        Option<&EntityId>,
        Option<&EntityName>,
        Option<&EntityTarget>,
        Option<&FactionComponent>,
        Option<&crate::comms::CommsHailable>,
        Option<&InfrastructureCondition>,
    )>,
    factions: Option<&crate::entities::config_cache::FactionRegistryResource>,
    comms: Option<&CommsRuntime>,
    world_runtime: Option<&crate::world::server::WorldContentRuntime>,
) -> Vec<DossierSubject> {
    let mut subjects: Vec<DossierSubject> = subjects_q
        .iter()
        .filter_map(
            |(uuid, id, name, target, faction, hailable, infrastructure)| {
                // The published condition track, and the ONLY way one reaches a
                // dossier: `from_state` is #1025's publish gate, so a structure
                // the scenario kept off the wire yields `None` here and the
                // projection never holds its condition at all.
                let condition = infrastructure.and_then(|infra| {
                    let published = InfrastructureSnapshot::from_state(&infra.0)?;
                    Some(SubjectCondition::from_published(
                        &published,
                        |flag| {
                            infra
                                .0
                                .thresholds()
                                .iter()
                                .find(|t| t.flag == flag)
                                .and_then(|t| t.label.clone())
                        },
                        |capacity| {
                            infra
                                .0
                                .capacities()
                                .iter()
                                .find(|c| c.id == capacity)
                                .and_then(|c| c.label.clone())
                        },
                    ))
                });

                // The two doors. An entity through neither is not a subject —
                // see the module docs on why there is no third.
                if hailable.is_none() && condition.is_none() {
                    return None;
                }

                // A promise names its party the way a world names its entities:
                // by the `[[entity]] id`. Not by UUID — #1029 refuses to resolve
                // one, because a promise outlives the hull it was made to — and
                // not by display name, which is a string id for a translator.
                // A promise made to a party that is NOT an entity in this world
                // ("the Skyway strike committee" as an abstraction) lands on no
                // dossier, which is honest: there is no sheet to put it on.
                let party = id.map(|i| i.0.as_str()).unwrap_or_default();
                Some(DossierSubject {
                    faction_label: faction
                        .and_then(|f| factions.and_then(|reg| reg.0.get(&f.0)))
                        .and_then(|config| config.display_name.clone()),
                    comms_in_range: hailable.map(|_| {
                        comms
                            .map(|runtime| contact_in_range(runtime, &uuid.0))
                            .unwrap_or(false)
                    }),
                    condition,
                    commitments: world_runtime
                        .filter(|_| !party.is_empty())
                        .map(|runtime| {
                            runtime
                                .commitments
                                .records
                                .iter()
                                .filter(|c| c.made_to == party)
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default(),
                    // What the crew found out about THIS hull (issue #1031),
                    // matched by UUID where a promise is matched by name — a
                    // finding is about the specific thing that was examined,
                    // and the applier resolved the script's name to this uuid
                    // when it was written. Gather order is the log's own; the
                    // filter preserves it.
                    evidence: world_runtime
                        .map(|runtime| runtime.evidence.for_subject(&uuid.0).cloned().collect())
                        .unwrap_or_default(),
                    summary: target
                        .and_then(|t| t.0.description.clone())
                        .unwrap_or_default(),
                    uuid: uuid.0.clone(),
                    name: name.map(|n| n.0.clone()).unwrap_or_default(),
                })
            },
        )
        .collect();
    subjects.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    subjects
}

/// Whether a hailable subject is currently reachable.
///
/// Read off the roster the Comms officer is already looking at rather than
/// recomputed from transforms — two readouts of the same range check on two
/// cadences is how a console and a dossier come to disagree about whether
/// somebody can be called.
fn contact_in_range(runtime: &CommsRuntime, uuid: &str) -> bool {
    runtime
        .contacts
        .iter()
        .find(|c| c.uuid == uuid)
        .map(|c| c.in_range)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
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
}

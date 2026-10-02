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
            publish_dossier_blackboard
                .in_set(crate::sim_sets::FixedStep::PublishDossierBlackboard)
                .in_set(crate::sim_sets::SimSet::Publish),
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
#[path = "server_tests.rs"]
mod tests;

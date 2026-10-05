//! Bevy adapter for the science scan (issue #1032).
//!
//! One component, two systems, and no decisions of its own. What a scan says is
//! [`super::scan`]'s to derive; everything here gathers the live inputs that
//! pure sibling cannot reach — where the ship is, what the grid is holding,
//! which hazard band it is sitting in, what the subject's condition track
//! currently says — stores what comes back, and publishes it.
//!
//! # The command rides a real, station-owned system
//!
//! [`tick_scans`] reads `AdmittedCommands` for the **`sensors`** system, which
//! is a genuine `[[system]]` a station owns, a console can be damaged out of,
//! and admission already gates on station tenure. So a scan takes exactly the
//! path `SetScienceTarget` takes, and admission — not this file — decides who
//! may ask. Nothing here branches on whether a human or the ship's own AI sent
//! it (AGENTS.md rule 6); by the time a command is in `AdmittedCommands` there
//! is nothing left on it that could say.
//!
//! `sensors` is also the system the destroyer's **captain** station owns
//! (`gui/destroyer/captain.html` resolves its sensor panels from it), which is
//! why the readout lands on that console without a second routing rule.
//!
//! # Why this runs in `SimSet::Modifiers` rather than `SimSet::Input`
//!
//! A scan is instantaneous — there is no hold to open on one tick and advance
//! on the next — and every input it takes is a *this tick* reading:
//!
//! * the ship's `Transform`, which `sync_ship_position` mirrors out of
//!   `ShipPhysics` back in `SimSet::Physics`, exactly as the tractor tick
//!   reads it;
//! * `RegionMembership`, recomputed in `SimSet::Physics`;
//! * and the subject's condition track, which
//!   [`tick_infrastructure_condition`](crate::infrastructure::tick_infrastructure_condition)
//!   advances in this same set — so this system is explicitly ordered
//!   `.after` it. A scan taken on the tick a repair team finishes reads the
//!   repaired number, which is the only answer a crew watching both consoles
//!   would accept.
//!
//! `AdmittedCommands` is cleared and refilled once per tick, before
//! `SimSet::Input`, so a `Modifiers` reader sees the whole tick's set.
//!
//! # The one place a reading becomes something a scenario can see (issue #1038)
//!
//! [`tick_scans`] is also where [`scanned_flag`] is mirrored into the base-world
//! flag store, for [`tick_infrastructure_condition`]'s reason spelled out at
//! length in `infrastructure/server.rs`: a fact is only observable if the code
//! that produces it is also the code that mirrors it, so there is **one write
//! site** rather than a second system re-deriving "has this been scanned" from
//! the record a tick later. Like a threshold crossing, the flag is written here
//! and a `FlagSet` is pushed onto `WorldContentRuntime::pending_world_events`,
//! which `collect_world_events` drains at the top of the next tick's
//! `SimSet::Physics` — so an `on_flag_set` hook fires one tick after the reading
//! lands, on the same one-tick bridge #1025's crossings ride.
//!
//! Nothing new is registered by that: the flag store is state the world plugin
//! already owns, snapshots and censuses. See [`scanned_flag`] for why the flag
//! exists when the reading is already stored, and why it latches.
//!
//! # Determinism
//!
//! Ships are walked in UUID order, never archetype order, and the subject is
//! looked up by UUID rather than by whichever entity a query happened to yield
//! first — the same rule [`crate::sim_digest`], #1025 and #1026 apply to their
//! own walks.

use bevy::prelude::*;

use crate::core::messages::{
    ActionFeedbackOutcome, AdmittedCommand, AdmittedCommands, PowerGroupId, ScanBlackboard,
    SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::core::task_lifecycle::{
    TaskLifecycleRequest, TaskSlot, TaskTerminalReason, TASK_VERB_SCAN,
};
use crate::dossier::SubjectCondition;
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::infrastructure::InfrastructureCondition;
use crate::logging::LogFilterConfig;
use crate::regions::effects::RegionEffectName;
use crate::science::scan::{
    derive, scanned_flag, ScanConditions, ScanConfig, ScanReading, ScanRefusal, ScanSubject,
};
use crate::ship::power::ShipPowerSystem;
use crate::world::content::WorldEvent;
use crate::world::server::WorldContentRuntime;

/// The blackboard channel key as a [`SystemId`].
pub fn scan_blackboard_key() -> SystemId {
    SystemId(SCAN_BLACKBOARD_KEY.to_string())
}

/// Everything one ship knows about scanning.
///
/// Authoritative per-ship simulation state. The **reading** is the part that
/// cannot be re-derived: it is what the crew saw when they looked, at the
/// fidelity their range and power bought them at that tick, and no amount of
/// re-folding the world afterwards recovers it — the structure has moved on
/// since, which is the entire point of taking a reading rather than watching a
/// gauge. A run that scanned the skyhook and a run that did not are different
/// runs, exactly as #1031's evidence log is.
///
/// One component rather than two because the three fields move together and are
/// read together: the publisher wants all of them and a save has to restore all
/// of them.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ShipScanRecord {
    /// The hull's authored `[scan]` table, as spawned.
    pub config: ScanConfig,
    /// The last reading this ship took, retained until the next one replaces
    /// it. Retained past the moment of the scan because a console that emptied
    /// the instant the sweep finished would be a console nobody could read.
    pub last: Option<ScanReading>,
    /// Why the most recent scan returned nothing, if it did. Cleared by the
    /// next scan that succeeds, so a refusal and a reading are never both
    /// current.
    pub refusal: Option<ScanRefusal>,
}

/// The mutable part of a ship's scan record, as a save carries it.
///
/// The authored `config` is deliberately **not** in here: it is re-derived
/// from the hull's template on the tick the ship spawns, and a
/// save whose hull's `[scan]` table has since changed is refused as
/// content-moved long before this is read — so writing it would put content
/// into a save that `content_digest` is the thing answerable for.
///
/// The two fields that ARE here have to come back **together**: restore the
/// reading without the refusal and a resumed console shows an answer beside a
/// stale complaint; restore the refusal without the reading and a crew who had
/// just scanned the skyhook resume having apparently failed to.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScanSaveState {
    /// The last reading taken, whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<ScanReading>,
    /// Why the last scan returned nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<ScanRefusal>,
}

impl ShipScanRecord {
    /// Project the record onto what a save carries.
    pub fn save_state(&self) -> ScanSaveState {
        ScanSaveState {
            last: self.last.clone(),
            refusal: self.refusal,
        }
    }

    /// Take a save's state back, leaving the spawned `[scan]` table alone.
    pub fn restore(&mut self, state: &ScanSaveState) {
        self.last = state.last.clone();
        self.refusal = state.refusal;
    }
}

/// Registers the scan systems. Added by `WorldPlugin` alongside
/// `InfrastructurePlugin`, because what it reads is its state.
pub struct SciencePlugin;

impl Plugin for SciencePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                // After the condition tick, so a scan taken on the tick a
                // repair lands reads the repaired number. See the module doc.
                tick_scans
                    .in_set(crate::sim_sets::FixedStep::TickScans)
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(crate::infrastructure::tick_infrastructure_condition),
                publish_scan_blackboard
                    .in_set(crate::sim_sets::FixedStep::PublishScanBlackboard)
                    .in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

/// Take every scan asked for this tick.
///
/// Per ship, in UUID order: pull this tick's admitted `ScanTarget` commands off
/// the `sensors` system, resolve the named subject, gather the live conditions,
/// and store whatever the pure derivation returns.
///
/// A hull that authored no `[scan]` table still owes the console an answer when
/// it is asked to scan, so it gets the record holding
/// [`ScanRefusal::NotCapable`] rather than having the command dropped on the
/// floor.
///
/// Every reading that comes back also raises the subject's [`scanned_flag`] —
/// see the module docs for why the mirror is written here and nowhere else.
#[allow(clippy::too_many_arguments)]
pub fn tick_scans(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    mut ships: Query<
        (
            Entity,
            &EntityUuid,
            &Transform,
            &AdmittedCommands,
            Option<&ShipPowerSystem>,
            Option<&mut ShipScanRecord>,
        ),
        With<crate::server_app::Ship>,
    >,
    subjects: Query<(
        &EntityUuid,
        &Transform,
        Option<&EntityName>,
        Option<&InfrastructureCondition>,
        // The world's own `[[entity]] id` — the handle #1038's mirror flag is
        // keyed on, because a scenario can type it and a minted UUID is not
        // something any author has ever seen. See `scanned_flag`.
        Option<&crate::entities::spawner::EntityId>,
        // Authored mass (issue #1154). `Option` rather than required: every
        // entity `spawn_entity` produces carries this unconditionally, but a
        // handful of test fixtures build a subject entity by hand without
        // going through the real spawner, and `subject_mass` below falls
        // those back to the same documented default a bare TOML gets — never
        // to zero.
        Option<&crate::entities::spawner::EntityMass>,
        // The subject's `[debris]` table (issue #1347), when it is a moving
        // hazard. `Option` like everything else here: a subject that is not
        // debris reads exactly as it did before this existed.
        Option<&crate::debris::DebrisThreat>,
    )>,
    region_effects: Query<&crate::entities::spawner::RegionEffectsSection>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
    // The continuous-task lifecycle queue (issue #1341). `Option` so a reduced
    // fixture that runs this system without the narrative plugin scans exactly
    // as it did before this existed.
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    // The debris-assessment queue (issue #1347), on `lifecycle`'s exact terms:
    // the reading is composed HERE — this is where the suite, the range and the
    // power are — but the contact's authoritative threat state belongs on the
    // contact, and this system holds the subject query read-only.
    mut debris_assessed: Option<ResMut<EffectQueue<crate::debris::DebrisAssessed>>>,
    // The correlated action-feedback seam (issue #761): the outbound message bus
    // a `finish_action_feedback` reports Applied/Refused through, addressed to the
    // command's own submitter. `Option` on the same terms as the queues above.
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    let now_tick = tick.map(|t| t.0).unwrap_or(0);

    // UUID order, not archetype order: two hosts must take the same ship's
    // scans in the same sequence, because each one overwrites that ship's
    // single stored reading.
    let mut rows: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (operator_uuid, entity) in rows {
        let Ok((entity, _, transform, admitted, power, record)) = ships.get_mut(entity) else {
            continue;
        };
        // Carry the whole admitted command (issue #761), not just its target
        // uuid: the target still drives the reading and the task lifecycle, but
        // the command is what the correlated action-feedback is addressed to.
        let requested: Vec<AdmittedCommand> = admitted
            .for_target(crate::ship::system_registry::SENSORS_SYSTEM_ID)
            .filter_map(|cmd| match &cmd.payload {
                SystemControlPayload::ScanTarget { .. } => Some(cmd.clone()),
                _ => None,
            })
            .collect();
        if requested.is_empty() {
            continue;
        }
        let scan_slot = TaskSlot::new(
            operator_uuid.clone(),
            crate::ship::system_registry::SENSORS_SYSTEM_ID,
            TASK_VERB_SCAN,
        );
        let Some(mut record) = record else {
            // No `[scan]` table at all. Insert the record holding the refusal
            // rather than saying nothing; it lands a tick later, which no
            // console can see.
            commands.entity(entity).insert(ShipScanRecord {
                refusal: Some(ScanRefusal::NotCapable),
                ..Default::default()
            });
            // Each asked-for reading is still an activation that began and
            // ended (issue #1341) — a crew who asked a hull with no suite to
            // scan get an answer, and the timeline records the asking.
            for cmd in &requested {
                let SystemControlPayload::ScanTarget { uuid: target_uuid } = &cmd.payload else {
                    continue;
                };
                push_scan_lifecycle(
                    lifecycle.as_deref_mut(),
                    &scan_slot,
                    target_uuid,
                    TaskTerminalReason::NotCapable,
                );
                crate::command_admission::finish_action_feedback(
                    cmd,
                    &mut outbound,
                    ActionFeedbackOutcome::Refused,
                );
            }
            continue;
        };
        let effects = operator_region_effects(membership.as_deref(), &region_effects, entity);
        let ship_pos = transform.translation;

        for cmd in &requested {
            let SystemControlPayload::ScanTarget { uuid: target_uuid } = &cmd.payload else {
                continue;
            };
            let found = subjects.iter().find(|(uuid, ..)| {
                uuid.0.as_str() == target_uuid.as_str()
                    && runtime
                        .as_ref()
                        .map(|runtime| {
                            crate::gm_contact::mode(
                                &runtime.contact_overrides,
                                &operator_uuid,
                                target_uuid,
                            ) != crate::gm_contact::ContactMode::Conceal
                        })
                        .unwrap_or(true)
            });
            let Some((_, subject_transform, name, condition, authored_id, mass, debris)) = found
            else {
                record.last = None;
                record.refusal = Some(ScanRefusal::NoSuchTarget);
                crate::pwarn!(
                    log,
                    crate::logging::LogCat::Sensors,
                    entity = entity,
                    "scan refused: no entity in this world answers to '{target_uuid}'"
                );
                push_scan_lifecycle(
                    lifecycle.as_deref_mut(),
                    &scan_slot,
                    target_uuid,
                    TaskTerminalReason::NoSuchTarget,
                );
                crate::command_admission::finish_action_feedback(
                    cmd,
                    &mut outbound,
                    ActionFeedbackOutcome::Refused,
                );
                continue;
            };
            let subject = ScanSubject {
                uuid: target_uuid.clone(),
                name: name.map(|n| n.0.clone()).unwrap_or_default(),
                condition: subject_condition(condition),
                mass: subject_mass(mass),
                debris: debris_subject(debris, subject_transform, &subjects),
            };
            let conditions = ScanConditions {
                distance: ship_pos.distance(subject_transform.translation),
                // No power grid means no power *constraint* — the ceiling is
                // absent, not zero. The reading #1026 takes for a bare fixture,
                // and for the same reason: failing a hull for a component it
                // never had would make every test rig unscannable.
                power_level: match power {
                    Some(power) => power
                        .0
                        .level_for(&PowerGroupId(record.config.power_group.clone())),
                    None => u8::MAX,
                },
                power_locked: power.map(|power| power.0.locked()).unwrap_or(false),
                region_effects: effects.clone(),
            };

            match derive(&record.config, &subject, &conditions, now_tick) {
                Ok(reading) => {
                    crate::pdebug!(
                        log,
                        crate::logging::LogCat::Sensors,
                        entity = entity,
                        "scan of {target_uuid} returned at band {} ({:.0} units)",
                        reading.band,
                        conditions.distance
                    );
                    // The debris half of the reading goes to the contact it
                    // names (issue #1347), before the reading is moved onto the
                    // record. Queued rather than written: `debris::server::
                    // tick_debris_state` is the one owner of a contact's
                    // authoritative threat state, and it runs after this system.
                    if let (Some(assessment), Some(queue)) =
                        (reading.debris.clone(), debris_assessed.as_deref_mut())
                    {
                        queue.0.push(crate::debris::DebrisAssessed {
                            subject_uuid: target_uuid.clone(),
                            taken_at_tick: reading.taken_at_tick,
                            assessment,
                        });
                    }
                    record.last = Some(reading);
                    record.refusal = None;
                    if let Some(authored_id) = authored_id {
                        mirror_scanned(runtime.as_deref_mut(), &authored_id.0, &log);
                    }
                    push_scan_lifecycle(
                        lifecycle.as_deref_mut(),
                        &scan_slot,
                        target_uuid,
                        TaskTerminalReason::Completed,
                    );
                    crate::command_admission::finish_action_feedback(
                        cmd,
                        &mut outbound,
                        ActionFeedbackOutcome::Applied,
                    );
                }
                Err(refusal) => {
                    crate::pdebug!(
                        log,
                        crate::logging::LogCat::Sensors,
                        entity = entity,
                        "scan of {target_uuid} refused: {}",
                        refusal.string_id()
                    );
                    record.last = None;
                    record.refusal = Some(refusal);
                    push_scan_lifecycle(
                        lifecycle.as_deref_mut(),
                        &scan_slot,
                        target_uuid,
                        terminal_reason_for(refusal),
                    );
                    crate::command_admission::finish_action_feedback(
                        cmd,
                        &mut outbound,
                        ActionFeedbackOutcome::Refused,
                    );
                }
            }
        }
    }
}

/// Record one asked-for reading as a whole task lifecycle (issue #1341).
///
/// A scan is INSTANTANEOUS — there is no hold to open on one tick and close on
/// the next — so its start and its terminal moment land on the same fixed tick,
/// in that order. They are still two events, because the lifecycle contract is
/// what a later mechanic reuses and because "the crew asked" and "this is what
/// came back" are two different facts about the run: a request that was refused
/// is not the same as a request nobody made.
///
/// Both are queued together here, so a scan can never leave a start unmatched.
fn push_scan_lifecycle(
    queue: Option<&mut EffectQueue<TaskLifecycleRequest>>,
    slot: &TaskSlot,
    target_uuid: &str,
    reason: TaskTerminalReason,
) {
    let Some(queue) = queue else {
        return;
    };
    queue.0.push(TaskLifecycleRequest::Start {
        slot: slot.clone(),
        target: Some(target_uuid.to_string()),
    });
    queue.0.push(TaskLifecycleRequest::End {
        slot: slot.clone(),
        reason,
    });
}

/// The terminal reason a refused reading reports, from the refusal the pure
/// derivation returned (issue #1341).
///
/// Every scan refusal is a FAILURE class: the suite could not do the work asked
/// of it. The cancelled/interrupted classes belong to tasks that have a
/// duration to be interrupted during, which is why the tractor's mapping is the
/// richer one.
fn terminal_reason_for(refusal: ScanRefusal) -> TaskTerminalReason {
    match refusal {
        ScanRefusal::NotCapable => TaskTerminalReason::NotCapable,
        ScanRefusal::NoSuchTarget => TaskTerminalReason::NoSuchTarget,
        ScanRefusal::NoReadableCondition => TaskTerminalReason::Unreadable,
        ScanRefusal::OutOfRange => TaskTerminalReason::OutOfRange,
        ScanRefusal::Underpowered => TaskTerminalReason::Unpowered,
        ScanRefusal::Blinded => TaskTerminalReason::Blinded,
    }
}

/// Latch "this crew have read that structure" into the base-world flag store,
/// and queue the world event a scenario hangs its beat on.
///
/// The transition is decided from the store's own `(before, after)` rather than
/// from "this is a scan", which is
/// [`mirror_flags`](crate::infrastructure::server) exactly: re-scanning the same
/// structure is an ordinary thing for a crew to do and must not emit a second
/// `FlagSet` for a bit that was already up, in the same way a re-append of a
/// finding is a no-op in [`EvidenceLog`](crate::dossier::EvidenceLog).
///
/// A world with no `WorldContentRuntime` — every bare-`App` fixture — takes the
/// `None` arm and scans exactly as it did before this existed.
fn mirror_scanned(
    runtime: Option<&mut WorldContentRuntime>,
    subject_id: &str,
    log: &Option<Res<LogFilterConfig>>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    let flag = scanned_flag(subject_id);
    let (before, after) = runtime.flags.set_flag(&flag);
    if (before != 0) == (after != 0) {
        return;
    }
    crate::pdebug!(
        log,
        crate::logging::LogCat::Sensors,
        "scan mirror: {flag} raised — this crew have now read {subject_id}"
    );
    runtime.pending_world_events.push(WorldEvent::FlagSet {
        name: flag,
        origin_layer: None,
    });
}

/// The subject's published condition track, paired with the labels its scenario
/// authored — **and the only path a condition reaches a scan by**.
///
/// [`crate::core::messages::infrastructure_snapshot_from_state`] is #1025's publish gate, so a
/// structure the scenario keeps off the wire yields `None` here and the
/// derivation refuses it. Lifted out as its own function so that gate is one
/// readable line rather than a clause inside the tick, and so it is visibly the
/// same construction `dossier::server` makes.
fn subject_condition(condition: Option<&InfrastructureCondition>) -> Option<SubjectCondition> {
    let infra = condition?;
    let published = crate::core::messages::infrastructure_snapshot_from_state(&infra.0)?;
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
}

/// The subject's raw debris geometry (issue #1347), stated relative to the asset
/// its `[debris]` table names — **and the only path a hazard reaches a scan by**.
///
/// `None` only for a subject that carries no `[debris]` table at all: every
/// moving hazard a crew can point an instrument at answers, because being read
/// is what makes a rock *read*. `ScanReading::debris` says as much in its own
/// doc — `None` for every subject that is not debris — and the whole beat turns
/// on a contact the crew RULED OUT being distinguishable from one nobody has
/// been to yet.
///
/// Two of those answers carry no asset to project against, and both are stated
/// the same way: separation `[0.0, 0.0]` against an empty `protected_name`, so
/// [`assess`](crate::debris::assess) reports a course, no collision course and
/// no impact — which is exactly the finding, and exactly what its own
/// `an_unprotected_contact_is_never_on_a_collision_course` describes.
///
/// * A table that names **nothing** — a field of harmless wreckage the crew have
///   to rule out. Authored, and the reason ruling one in matters.
/// * A table naming an asset **no longer in the world**. A rock aimed at a depot
///   that has already been destroyed has nothing left to hit, and the honest
///   reading is that it is on course for nothing rather than that there is no
///   reading to be had.
///
/// Lifted out as its own function for `subject_condition`'s reason: the gate is
/// then one readable line inside the tick rather than a clause in the middle of
/// it, and the name lookup is visibly the SAME one the objective and comms
/// vocabularies resolve an authored entity reference by.
fn debris_subject(
    threat: Option<&crate::debris::DebrisThreat>,
    subject_transform: &Transform,
    subjects: &Query<(
        &EntityUuid,
        &Transform,
        Option<&EntityName>,
        Option<&InfrastructureCondition>,
        Option<&crate::entities::spawner::EntityId>,
        Option<&crate::entities::spawner::EntityMass>,
        Option<&crate::debris::DebrisThreat>,
    )>,
) -> Option<crate::debris::DebrisSubject> {
    let threat = threat?;
    let protected = &threat.config.protected_target;
    let asset = if protected.is_empty() {
        None
    } else {
        subjects
            .iter()
            .find(|(_, _, name, ..)| name.is_some_and(|n| n.0 == *protected))
            .map(|(_, tf, ..)| tf.translation)
    };
    // The course is the authored drift either way: what a contact is DOING is a
    // fact about the contact, and only what it is doing *to something* needs an
    // asset to be stated against.
    //
    // The protected asset is world furniture — a depot, a rung, a control tower
    // — and the rock is the only thing moving, so the authored drift IS the
    // relative velocity. The day a scenario protects something that moves, this
    // is the one line that subtracts.
    let relative_velocity = [threat.config.drift[0], threat.config.drift[2]];
    let Some(asset) = asset else {
        return Some(crate::debris::DebrisSubject {
            relative_position: [0.0, 0.0],
            relative_velocity,
            protected_name: String::new(),
            // No radius, so `assess` can never confirm this contact however near
            // it passes to anything — which is the authored point of a mass aimed
            // at nothing.
            impact_radius: 0.0,
        });
    };
    Some(crate::debris::DebrisSubject {
        relative_position: [
            subject_transform.translation.x - asset.x,
            subject_transform.translation.z - asset.z,
        ],
        relative_velocity,
        protected_name: protected.clone(),
        impact_radius: threat.config.impact_radius,
    })
}

/// The subject's authored mass (issue #1154), off its `EntityMass` component.
///
/// Falls back to [`crate::entities::config::DEFAULT_ENTITY_MASS`] — never to
/// `0.0` — for the handful of test fixtures that build a subject entity by
/// hand rather than through [`crate::entities::spawner::spawn_entity`], the
/// only path that inserts the component. Every entity the real spawner
/// produces carries `EntityMass` unconditionally, so this arm is untaken in
/// production.
fn subject_mass(mass: Option<&crate::entities::spawner::EntityMass>) -> f32 {
    mass.map(|m| m.0)
        .unwrap_or(crate::entities::config::DEFAULT_ENTITY_MASS)
}

/// Which authored region effects the scanning hull is standing in,
/// deduplicated and in a fixed order.
///
/// Sorted by declaration order rather than by whichever region entity the
/// membership set happened to yield first, so that two hosts that spawned the
/// same bands in different orders hand the pure module the same list.
fn operator_region_effects(
    membership: Option<&crate::regions::server::RegionMembership>,
    region_effects: &Query<&crate::entities::spawner::RegionEffectsSection>,
    operator: Entity,
) -> Vec<RegionEffectName> {
    let Some(regions) = membership.and_then(|m| m.inside.get(&operator)) else {
        return Vec::new();
    };
    let mut names: Vec<RegionEffectName> = regions
        .iter()
        .filter_map(|region| region_effects.get(*region).ok())
        .flat_map(|effects| {
            effects
                .0
                .iter()
                .map(crate::regions::effects::region_effect_name)
        })
        .collect();
    names.sort_by_key(|name| {
        RegionEffectName::ALL
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or(usize::MAX)
    });
    names.dedup();
    names
}

/// Publish each scanning ship's last reading.
///
/// Only ships that carry [`ShipScanRecord`] publish one, so a world whose hulls
/// author no `[scan]` puts exactly the payload on the wire it did before this
/// existed. Written only when the picture actually changed —
/// `ShipSystemBlackboards` feeds the diffed `BlackboardUpdate` broadcast, so a
/// ship that has not scanned since last tick costs nothing.
pub fn publish_scan_blackboard(
    runtime: Option<Res<WorldContentRuntime>>,
    mut ships: Query<(
        Option<&EntityUuid>,
        &ShipScanRecord,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    for (observer, record, mut blackboards) in ships.iter_mut() {
        let blackboard = SystemBlackboard::Scan(ScanBlackboard {
            // A hull with no bands can be asked and refused, which the console
            // renders as "no scan capability" rather than as an empty box.
            capable: !record.config.bands.is_empty(),
            reading: record
                .last
                .as_ref()
                .filter(|reading| {
                    observer
                        .and_then(|observer| {
                            runtime.as_ref().map(|runtime| {
                                crate::gm_contact::mode(
                                    &runtime.contact_overrides,
                                    &observer.0,
                                    &reading.subject_uuid,
                                )
                            })
                        })
                        .unwrap_or_default()
                        != crate::gm_contact::ContactMode::Conceal
                })
                .map(crate::core::messages::scan_reading_snapshot_from_reading),
            refusal: record.refusal.map(|r| r.string_id().to_string()),
        });
        let key = scan_blackboard_key();
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key, blackboard);
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;

pub use phoenix_sim_gameplay::science::SCAN_BLACKBOARD_KEY;

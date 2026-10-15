//! Bevy adapter for the science scan (issue #1032).
//!
//! One observation owner, timed acquisition and permitted publications. What a scan says is
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
//! A focused enquiry advances with actual paid work over several fixed ticks.
//! It validates geometry and publication policy throughout, and records the
//! measurement at completion. Its inputs are that tick's readings:
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
    ActionFeedbackOutcome, AdmittedCommand, AdmittedCommands, ScanBlackboard, SystemBlackboard,
    SystemControlPayload, SystemId,
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
    scanned_flag, ScanConditions, ScanConfig, ScanReading, ScanRefusal, ScanSubject,
};
use crate::ship::power::ShipPowerSystem;
use crate::world::content::WorldEvent;
use crate::world::server::WorldContentRuntime;

pub use phoenix_sim_gameplay::science::{scan_blackboard_key_for, scan_owner_from_blackboard_key};

#[derive(Clone, Debug, PartialEq)]
pub struct FocusedScanRecord {
    pub config: ScanConfig,
    pub policy: phoenix_model::sensing::FocusedConfig,
    pub last: Option<ScanReading>,
    pub refusal: Option<ScanRefusal>,
    pub history: Vec<ScanReading>,
    pub focused: Option<super::focused::FocusedWork>,
    pub unfinished: std::collections::BTreeMap<String, super::focused::FocusedWork>,
}
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ShipScanRecord(pub std::collections::BTreeMap<SystemId, FocusedScanRecord>);
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusedSaveState {
    pub last: Option<ScanReading>,
    pub refusal: Option<ScanRefusal>,
    pub history: Vec<ScanReading>,
    pub focused: Option<super::focused::FocusedWork>,
    pub unfinished: std::collections::BTreeMap<String, super::focused::FocusedWork>,
}
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScanSaveState {
    pub instruments: std::collections::BTreeMap<SystemId, FocusedSaveState>,
}
impl FocusedScanRecord {
    pub fn save_state(&self) -> FocusedSaveState {
        FocusedSaveState {
            last: self.last.clone(),
            refusal: self.refusal,
            history: self.history.clone(),
            focused: self.focused.clone(),
            unfinished: self.unfinished.clone(),
        }
    }
    pub fn restore(&mut self, state: &FocusedSaveState) {
        self.last = state.last.clone();
        self.refusal = state.refusal;
        self.history = state.history.clone();
        self.focused = state.focused.clone();
        self.unfinished = state.unfinished.clone();
    }
}
impl ShipScanRecord {
    pub fn from_config(config: &crate::entities::config::EntityConfig) -> Option<Self> {
        let scan = config.scan.as_ref()?;
        let profiles = &config.sensing.as_ref()?.focused;
        if profiles.is_empty() {
            return None;
        }
        Some(Self(
            profiles
                .iter()
                .map(|policy| {
                    (
                        policy.system.clone(),
                        FocusedScanRecord {
                            config: scan.clone(),
                            policy: policy.clone(),
                            last: None,
                            refusal: None,
                            history: Vec::new(),
                            focused: None,
                            unfinished: Default::default(),
                        },
                    )
                })
                .collect(),
        ))
    }
    pub fn save_state(&self) -> ScanSaveState {
        ScanSaveState {
            instruments: self
                .0
                .iter()
                .map(|(id, state)| (id.clone(), state.save_state()))
                .collect(),
        }
    }
    pub fn validate_state(&self, saved: &ScanSaveState, tick: u64) -> Result<(), String> {
        if !self.0.keys().eq(saved.instruments.keys()) {
            return Err("focused instrument owner set disagrees with fitted configuration".into());
        }
        for (id, state) in &self.0 {
            state.validate_state(&saved.instruments[id], tick)?;
        }
        Ok(())
    }
    pub fn restore(&mut self, saved: &ScanSaveState) {
        for (id, state) in &mut self.0 {
            state.restore(&saved.instruments[id]);
        }
    }
}
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(record) = ShipScanRecord::from_config(config) {
        commands.insert(record);
    }
}

/// Registers the scan systems. Added by `WorldPlugin` alongside
/// `InfrastructurePlugin`, because what it reads is its state.
pub struct SciencePlugin;

impl Plugin for SciencePlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::weapons::mounts::MountsPlugin>() {
            app.add_plugins(crate::weapons::mounts::MountsPlugin);
        }
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<super::enquiry::EnquirySubject>(
            StateClass::Derived,
            "focused-subject-observation-policy",
        );
        super::focused_ai::register(app);
        super::passive::register(app);
        super::protection::register(app);
        super::broad::register(app);
        super::sharing::register(app);
        super::references::register(app);
        super::reading::register(app);
        super::approaches::register(app);
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
/// A request for an unfitted focused instrument receives correlated refusal
/// and a NotCapable lifecycle result. It never creates a synthetic record.
///
/// Every reading that comes back also raises the subject's [`scanned_flag`] —
/// see the module docs for why the mirror is written here and nowhere else.
#[allow(clippy::too_many_arguments)]
pub fn tick_scans(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    time: Option<Res<Time>>,
    fixed: Option<Res<Time<Fixed>>>,
    paths: crate::regions::signal_path::RegionSignalQuery,
    enquiries: super::enquiry::EnquiryScene,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    mut ships: Query<
        (
            Entity,
            &EntityUuid,
            &Transform,
            &AdmittedCommands,
            Option<&mut ShipPowerSystem>,
            Option<&mut ShipScanRecord>,
            Option<&crate::ship::compartment_damage::ShipDamage>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::server_app::ShipSystemBlackboards>,
            Option<&super::broad::BroadTracks>,
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
    let now_tick = tick.as_ref().map(|t| t.0).unwrap_or(0);
    let dt = fixed.map_or_else(
        || time.map_or(0.0, |time| time.delta_secs()),
        |fixed| fixed.timestep().as_secs_f32(),
    );

    // UUID order, not archetype order: two hosts must take the same ship's
    // scans in the same sequence, because each one overwrites that ship's
    // single stored reading.
    let mut rows: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (operator_uuid, entity) in rows {
        let Ok((
            entity,
            _,
            transform,
            admitted,
            mut power,
            records,
            damage,
            sources,
            config,
            boards,
            tracks,
        )) = ships.get_mut(entity)
        else {
            continue;
        };
        for command in &admitted.0 {
            if matches!(
                command.payload,
                SystemControlPayload::ScanTarget { .. } | SystemControlPayload::CancelFocusedScan
            ) && records
                .as_ref()
                .is_none_or(|records| !records.0.contains_key(&command.target))
            {
                crate::command_admission::finish_action_feedback(
                    command,
                    &mut outbound,
                    ActionFeedbackOutcome::Refused,
                );
                if let SystemControlPayload::ScanTarget { uuid, .. } = &command.payload {
                    push_scan_lifecycle(
                        lifecycle.as_deref_mut(),
                        &TaskSlot::new(operator_uuid.clone(), &command.target.0, TASK_VERB_SCAN),
                        uuid,
                        TaskTerminalReason::NotCapable,
                    );
                }
            }
        }
        let Some(mut records) = records else {
            continue;
        };
        // Every accepted focused instrument uses explicit observation admission,
        // including a hull with no fitted broad radar. Absence cannot grant an
        // arbitrary physical actor reference to a focused scanner.
        let no_observations = super::broad::BroadTracks(Default::default());
        let tracks = Some(tracks.unwrap_or(&no_observations));
        for (owner, record) in &mut records.0 {
            let requested: Vec<AdmittedCommand> = admitted
                .for_target(&owner.0)
                .filter(|cmd| {
                    matches!(
                        cmd.payload,
                        SystemControlPayload::ScanTarget { .. }
                            | SystemControlPayload::CancelFocusedScan
                    )
                })
                .cloned()
                .collect();
            let scan_slot = TaskSlot::new(operator_uuid.clone(), &owner.0, TASK_VERB_SCAN);
            let policy = record.policy.clone();
            record.unfinished.retain(|_, work| {
                (now_tick.saturating_sub(work.last_valid_tick) as f64 * dt as f64)
                    <= policy.unfinished_retention_secs as f64
            });
            if requested.is_empty() && record.focused.is_none() {
                continue;
            }
            let mut started = false;
            for cmd in &requested {
                if matches!(cmd.payload, SystemControlPayload::CancelFocusedScan) {
                    if let Some(work) = record.focused.take() {
                        if policy.unfinished_retention_secs > 0.0 {
                            record.unfinished.insert(work.key(), work);
                        }
                        if let Some(queue) = lifecycle.as_deref_mut() {
                            queue.0.push(TaskLifecycleRequest::End {
                                slot: scan_slot.clone(),
                                reason: TaskTerminalReason::Released,
                            });
                        }
                    }
                    crate::command_admission::finish_action_feedback(
                        cmd,
                        &mut outbound,
                        ActionFeedbackOutcome::Applied,
                    );
                    continue;
                }
                let SystemControlPayload::ScanTarget {
                    uuid,
                    aim,
                    intrusive,
                } = &cmd.payload
                else {
                    continue;
                };
                let Some((work_secs, _)) = policy.aim_cost(*aim, *intrusive) else {
                    crate::command_admission::finish_action_feedback(
                        cmd,
                        &mut outbound,
                        ActionFeedbackOutcome::Refused,
                    );
                    record.refusal = Some(ScanRefusal::NotCapable);
                    record.last = None;
                    continue;
                };
                let presentation_id = uuid.clone();
                let actual = config.zip(boards).and_then(|(config, boards)| {
                    super::contact_routing::resolve(&config.0, boards, tracks, owner, uuid)
                });
                let Some(actual) = actual
                    .filter(|actual| subjects.iter().any(|(subject, ..)| &subject.0 == actual))
                else {
                    record.last = None;
                    record.refusal = Some(ScanRefusal::NoSuchTarget);
                    push_scan_lifecycle(
                        lifecycle.as_deref_mut(),
                        &scan_slot,
                        uuid,
                        TaskTerminalReason::NoSuchTarget,
                    );
                    crate::command_admission::finish_action_feedback(
                        cmd,
                        &mut outbound,
                        ActionFeedbackOutcome::Refused,
                    );
                    continue;
                };
                let uuid = &actual;
                if record.focused.as_ref().is_none_or(|work| {
                    &work.subject != uuid
                        || work.presentation_id != presentation_id
                        || work.aim != *aim
                        || work.intrusive != *intrusive
                }) {
                    if let Some(queue) = lifecycle.as_deref_mut() {
                        if record.focused.is_some() {
                            queue.0.push(TaskLifecycleRequest::End {
                                slot: scan_slot.clone(),
                                reason: TaskTerminalReason::Restarted,
                            });
                        }
                        queue.0.push(TaskLifecycleRequest::Start {
                            slot: scan_slot.clone(),
                            target: Some(presentation_id.clone()),
                        });
                    }
                    if let Some(previous) = record
                        .focused
                        .take()
                        .filter(|_| policy.unfinished_retention_secs > 0.0)
                    {
                        record.unfinished.insert(previous.key(), previous);
                    }
                    while record.unfinished.len() > usize::from(policy.history_limit) {
                        let oldest = record
                            .unfinished
                            .iter()
                            .min_by_key(|(id, work)| (work.last_valid_tick, *id))
                            .map(|(id, _)| id.clone())
                            .expect("nonempty retained enquiries");
                        record.unfinished.remove(&oldest);
                    }
                    let resumed = record
                        .unfinished
                        .remove(&super::focused::FocusedWork::key_for(
                            uuid, *aim, *intrusive,
                        ));
                    record.focused = Some(resumed.unwrap_or_else(|| super::focused::FocusedWork {
                        aim: *aim,
                        intrusive: *intrusive,
                        paid_started_tick: None,
                        subject: uuid.clone(),
                        presentation_id: presentation_id.clone(),
                        progress: 0.0,
                        work_secs,
                        started_tick: now_tick,
                        last_valid_tick: now_tick,
                        last_tick: now_tick,
                        waiting: String::new(),
                    }));
                    if let Some(work) = &mut record.focused {
                        work.presentation_id = presentation_id.clone();
                    }
                    started = true;
                }
                crate::command_admission::finish_action_feedback(
                    cmd,
                    &mut outbound,
                    ActionFeedbackOutcome::Applied,
                );
            }
            while record.unfinished.len() > usize::from(policy.history_limit) {
                let oldest = record
                    .unfinished
                    .iter()
                    .min_by_key(|(id, work)| (work.last_valid_tick, *id))
                    .map(|(id, _)| id.clone())
                    .expect("bounded unfinished work");
                record.unfinished.remove(&oldest);
            }
            let Some(work) = record.focused.as_ref() else {
                continue;
            };
            if !started && tick.is_some() && work.last_tick == now_tick {
                continue;
            }
            let continuation = AdmittedCommand {
                target: policy.system.clone(),
                payload: SystemControlPayload::ScanTarget {
                    uuid: work.subject.clone(),
                    aim: work.aim,
                    intrusive: work.intrusive,
                },
                response_token: None,
                feedback_correlation: None,
            };
            if let Some(work) = &mut record.focused {
                work.last_tick = now_tick;
            }
            let effects = operator_region_effects(membership.as_deref(), &region_effects, entity);
            let ship_pos = transform.translation;

            for cmd in [&continuation] {
                let SystemControlPayload::ScanTarget {
                    uuid: target_uuid, ..
                } = &cmd.payload
                else {
                    continue;
                };
                let permitted = record.focused.as_ref().and_then(|work| {
                    config.zip(boards).and_then(|(config, boards)| {
                        super::contact_routing::resolve(
                            &config.0,
                            boards,
                            tracks,
                            owner,
                            &work.presentation_id,
                        )
                    })
                });
                let found = subjects.iter().find(|(uuid, ..)| {
                    permitted.as_deref() == Some(target_uuid.as_str())
                        && uuid.0.as_str() == target_uuid.as_str()
                        && runtime
                            .as_ref()
                            .map(|runtime| {
                                crate::gm::operations::contact::mode(
                                    &runtime.contact_overrides,
                                    &operator_uuid,
                                    target_uuid,
                                ) != crate::gm::operations::contact::ContactMode::Conceal
                            })
                            .unwrap_or(true)
                });
                let Some((_, subject_transform, name, condition, authored_id, mass, debris)) =
                    found
                else {
                    record.last = None;
                    record.refusal = Some(ScanRefusal::NoSuchTarget);
                    if let Some(work) = &mut record.focused {
                        work.waiting = "source_unavailable".into();
                        expire_focused_work(work, &policy, now_tick, dt);
                    }
                    crate::pwarn!(
                        log,
                        crate::logging::LogCat::Sensors,
                        entity = entity,
                        "scan refused: no entity in this world answers to '{target_uuid}'"
                    );
                    crate::command_admission::finish_action_feedback(
                        cmd,
                        &mut outbound,
                        ActionFeedbackOutcome::Refused,
                    );
                    continue;
                };
                let mut subject = ScanSubject {
                    uuid: target_uuid.clone(),
                    name: name.map(|n| n.0.clone()).unwrap_or_default(),
                    condition: subject_condition(condition),
                    mass: subject_mass(mass),
                    debris: debris_subject(debris, subject_transform, &subjects),
                };
                let transmission = paths.transmission(
                    ship_pos,
                    subject_transform.translation,
                    crate::regions::signal_path::SignalClass::Sensors,
                );
                let receiver_factor =
                    paths.sensors_receiver_factor(ship_pos, &operator_uuid, owner);
                let conditions = ScanConditions {
                    // Path loss reduces usable reach on the same fidelity ladder.
                    // An opaque path cannot be bypassed by unfaded endpoints.
                    distance: if transmission > 0.0 && receiver_factor > 0.0 {
                        ship_pos.distance(subject_transform.translation)
                            / transmission
                            / receiver_factor
                    } else {
                        f32::INFINITY
                    },
                    // Legacy allocation rungs no longer grant observation work.
                    // The real owning reserve pays the bounded acquisition below.
                    power_level: u8::MAX,
                    power_locked: false,
                    region_effects: effects.clone(),
                };

                let work = record.focused.as_ref().expect("active enquiry");
                if work.aim != phoenix_model::focused_enquiry::FocusedAim::General {
                    subject.name = config
                        .zip(boards)
                        .zip(tracks)
                        .and_then(|((config, boards), tracks)| {
                            super::contact_routing::identity_for_source(
                                &config.0,
                                boards,
                                tracks,
                                owner,
                                target_uuid,
                            )
                        })
                        .unwrap_or_default();
                }
                let observed = config.zip(boards).and_then(|(config, boards)| {
                    let view = config.0.observations_for_system(owner, &boards.0);
                    let mut keys: Vec<_> = boards.0.keys().collect();
                    keys.sort();
                    keys.into_iter()
                        .filter_map(|key| view.get(key))
                        .filter_map(super::contact_routing::observations)
                        .flatten()
                        .find(|track| {
                            track.contact_id == work.presentation_id
                                && !track.predicted
                                && track.selectable
                        })
                        .map(|track| Vec3::new(track.position[0], 0.0, track.position[1]))
                });
                let quantities = enquiries.sample(
                    target_uuid,
                    work.aim,
                    work.intrusive,
                    ship_pos,
                    observed,
                    now_tick,
                    dt,
                );
                let reading = phoenix_sim_gameplay::science::scan::derive_enquiry_with_shield(
                    &record.config,
                    &subject,
                    &conditions,
                    work.aim,
                    work.intrusive,
                    quantities,
                    now_tick,
                    (work.aim == phoenix_model::focused_enquiry::FocusedAim::Shields)
                        .then(|| enquiries.sample_shields(target_uuid, work.intrusive))
                        .flatten(),
                );
                match reading {
                    Ok(mut reading) => {
                        let work = record.focused.as_mut().expect("active enquiry");
                        let budget = dt.max(0.0).min((work.work_secs - work.progress).max(0.0));
                        let factor = damage.map_or(1.0, |damage| {
                            damage.factor(
                                &policy.system,
                                phoenix_model::damage::DamageBinding::Operation,
                            )
                        });
                        let enabled = sources.is_some_and(|sources| {
                            sources.0.entries().any(|(id, _)| id == &policy.system)
                                && !sources.0.is_offline(&policy.system)
                                && sources.0.source_for(&policy.system)
                                    != crate::ship::control_source::ControlSource::Offline
                        });
                        let paid = if budget > 0.0 && enabled && factor > 0.0 {
                            power
                                .as_deref_mut()
                                .and_then(|power| {
                                    let (_, cost) = policy.aim_cost(work.aim, work.intrusive)?;
                                    power.0.work_continuous(&policy.system, budget, cost).ok()
                                })
                                .map_or(0.0, |grant| grant.fraction * factor)
                        } else {
                            0.0
                        };
                        if paid > 0.0 {
                            if work.paid_started_tick.is_none() {
                                work.paid_started_tick = Some(now_tick);
                                emit_enquiry(
                                    runtime.as_deref_mut(),
                                    &operator_uuid,
                                    target_uuid,
                                    owner,
                                    work.aim,
                                    work.intrusive,
                                    crate::world::config::EnquiryPhase::Started,
                                );
                            }
                            work.progress = (work.progress + budget * paid).min(work.work_secs);
                            work.last_valid_tick = now_tick;
                            work.waiting.clear();
                        } else {
                            work.waiting = "unpowered".into();
                            expire_focused_work(work, &policy, now_tick, dt);
                        }
                        if work.progress < work.work_secs {
                            continue;
                        }
                        emit_enquiry(
                            runtime.as_deref_mut(),
                            &operator_uuid,
                            target_uuid,
                            owner,
                            work.aim,
                            work.intrusive,
                            crate::world::config::EnquiryPhase::Completed,
                        );
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
                        reading.contact_id = record
                            .focused
                            .as_ref()
                            .map(|work| work.presentation_id.clone());
                        let general =
                            reading.aim == phoenix_model::focused_enquiry::FocusedAim::General;
                        record.remember(reading, usize::from(policy.history_limit), f64::from(dt));
                        record.focused = None;
                        if let Some(authored_id) = authored_id.filter(|_| general) {
                            mirror_scanned(runtime.as_deref_mut(), &authored_id.0, &log);
                        }
                        if let Some(queue) = lifecycle.as_deref_mut() {
                            queue.0.push(TaskLifecycleRequest::End {
                                slot: scan_slot.clone(),
                                reason: TaskTerminalReason::Completed,
                            });
                        }
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
                        if let Some(work) = &mut record.focused {
                            work.waiting = refusal.string_id().into();
                            expire_focused_work(work, &policy, now_tick, dt);
                        }
                        if matches!(
                            refusal,
                            ScanRefusal::NotCapable | ScanRefusal::NoReadableCondition
                        ) {
                            record.focused = None;
                            if let Some(queue) = lifecycle.as_deref_mut() {
                                queue.0.push(TaskLifecycleRequest::End {
                                    slot: scan_slot.clone(),
                                    reason: terminal_reason_for(refusal),
                                });
                            }
                        }
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
}

fn expire_focused_work(
    work: &mut super::focused::FocusedWork,
    policy: &phoenix_model::sensing::FocusedConfig,
    now: u64,
    dt: f32,
) {
    if now.saturating_sub(work.last_valid_tick) as f64 * dt as f64
        > policy.unfinished_retention_secs as f64
    {
        work.progress = 0.0;
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_enquiry(
    runtime: Option<&mut WorldContentRuntime>,
    observer: &str,
    target: &str,
    system: &SystemId,
    aim: phoenix_model::focused_enquiry::FocusedAim,
    intrusive: bool,
    phase: crate::world::config::EnquiryPhase,
) {
    if let Some(runtime) = runtime {
        runtime.pending_world_events.push(WorldEvent::Enquiry {
            target_uuid: target.to_owned(),
            observer_uuid: observer.to_owned(),
            system: system.clone(),
            aim: aim.as_str().to_owned(),
            intrusive,
            phase,
        });
    }
}

/// Record one asked-for reading as a whole task lifecycle (issue #1341).
///
/// Immediate refusals start and terminate on the same tick. Successful
/// enquiries use the paid multi-tick lifecycle in `tick_scans` instead.
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
        .filter(|region| {
            membership
                .and_then(|m| m.exposure.get(&(operator, **region)))
                .is_none_or(|strength| *strength > 0.0)
        })
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
    for (observer, records, mut blackboards) in ships.iter_mut() {
        for (owner, record) in &records.0 {
            let blackboard = SystemBlackboard::Scan(ScanBlackboard {
                default_aim: record.policy.default_aim,
                aims: {
                    let mut aims = record.policy.aims.clone();
                    if !aims
                        .iter()
                        .any(|row| row.aim == phoenix_model::focused_enquiry::FocusedAim::General)
                    {
                        aims.insert(
                            0,
                            phoenix_model::focused_enquiry::FocusedAimConfig {
                                aim: Default::default(),
                                work_multiplier: 1.0,
                                resource_multiplier: 1.0,
                                intrusive: false,
                            },
                        );
                    }
                    aims
                },
                owner: Some(owner.clone()),
                history: record
                    .history
                    .iter()
                    .filter(|reading| {
                        observer
                            .and_then(|observer| {
                                runtime.as_ref().map(|runtime| {
                                    crate::gm::operations::contact::mode(
                                        &runtime.contact_overrides,
                                        &observer.0,
                                        &reading.subject_uuid,
                                    )
                                })
                            })
                            .unwrap_or_default()
                            != crate::gm::operations::contact::ContactMode::Conceal
                    })
                    .map(crate::core::messages::scan_reading_snapshot_from_reading)
                    .collect(),
                focused: record.focused.as_ref().map(|work| {
                    crate::core::messages::FocusedScanSnapshot {
                        aim: work.aim,
                        intrusive: work.intrusive,
                        subject: work.presentation_id.clone(),
                        progress: ((work.progress / work.work_secs).clamp(0.0, 1.0) * 1000.0)
                            as u16,
                        waiting: work.waiting.clone(),
                    }
                }),
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
                                    crate::gm::operations::contact::mode(
                                        &runtime.contact_overrides,
                                        &observer.0,
                                        &reading.subject_uuid,
                                    )
                                })
                            })
                            .unwrap_or_default()
                            != crate::gm::operations::contact::ContactMode::Conceal
                    })
                    .map(crate::core::messages::scan_reading_snapshot_from_reading),
                refusal: record.refusal.map(|r| r.string_id().to_string()),
            });
            let key = scan_blackboard_key_for(owner);
            if blackboards.0.get(&key) != Some(&blackboard) {
                blackboards.0.insert(key, blackboard);
            }
        }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;

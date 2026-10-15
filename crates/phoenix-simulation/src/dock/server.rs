//! The Bevy adapter for helm docking (issue #1159).
//!
//! Gathers the live world into the plain values the pure sibling
//! [`crate::dock::mating`] takes, applies what comes back, and drives the own
//! ship onto the mated pose — the per-ship [`DockControl`] component, the
//! per-hull [`DockMarkers`] resolved from the rig sidecar at spawn, the
//! fixed-tick systems that take the dock/undock commands, decide whether the
//! dock holds, fly the own ship onto its mate, and publish the blackboard.
//! Nothing here decides geometry or eligibility itself: rule 10, the split the
//! tractor keeps between `coupling` and `server`.
//!
//! # It moves the OWN ship only
//!
//! Unlike the tractor's `move_coupled_target`, which writes the TARGET's
//! position, docking writes only the OWN ship's `ShipPhysics`/`Transform` — the
//! ship the crew are flying slides onto the berth; the structure it mates with is
//! never touched. So the tractor-review coordination note (a target moved by two
//! couplers at once) does not apply to the target. The one hull that could be
//! written by both is a docking ship that is also under someone's tractor; to
//! keep that deterministic this system is ordered `after` the tractor rig, so the
//! dock's own-ship placement is the last writer that tick. A docker holds at most
//! one target — `docking_target` is a single `Option` — which the umbilical (#1160)
//! reads; berth-side occupancy (one docker per target) is not enforced here.

use crate::ai::state_tree_runtime::{ShipAiTreePrograms, ShipAiTreeRuntime};
use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
use crate::ship::compartment_damage::{operation_disabled, ShipDamage};
use bevy::prelude::*;
use phoenix_sim_gameplay::ai::host::{decide_tree, TreeHostOutcome};
use phoenix_sim_gameplay::ship::composition::{
    dock_composition, CommandChannelConfig, CommandChannelKind, CommandChannelValue,
    SystemCommandChannels,
};

use crate::command_admission::ai_emit::emit_ai_command;
use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
use crate::core::messages::{
    ActionFeedbackOutcome, DockBlackboard, PowerGroupId, SystemAffinity, SystemBlackboard,
    SystemControlPayload, SystemId,
};
use crate::core::task_lifecycle::{
    physical_work_reports, PhysicalWork, PriorActivation, TaskLifecycleRequest, TaskSlot,
    TaskTerminalReason, TASK_VERB_DOCK_HOLD,
};
use crate::dock::mating::{nearest_viable_pair, DockConfig, DockMarker, DockRefusal, Pose};
use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};
use crate::ship::power::ShipPowerSystem;
use crate::ship::state::ShipPhysics;
use crate::ship::system_registry::dock_system_id;
use crate::world::server::WorldContentRuntime;

/// One hull's dock markers, resolved into the hull's OWN frame at spawn (issue
/// #1159).
///
/// Attached to every entity whose `[dock]` table opted it into docking AND whose
/// rig sidecar declared `dock`-prefixed `[markers.<name>]` blocks. The base rig
/// is folded in once here, so each marker is a ship-local point a live
/// `Transform` maps to world — the same composition
/// [`crate::entities::model_rig::ModelMarkers::resolve_world_position`] performs, done
/// once at spawn so the fixed tick never re-reads the sidecar. A hull that
/// declares no dock markers carries no `DockMarkers` and can never be docked
/// with.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct DockMarkers {
    /// The dock markers in the hull's own frame, in a stable authored order
    /// (sorted by name at spawn) so nearest-pair indices are deterministic.
    pub markers: Vec<DockMarker>,
}

impl DockMarkers {
    /// Whether this hull declares any dock markers. A hull with none can never
    /// be docked with.
    pub fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }
}

/// One ship's active dock control (issue #1159): the authored dock terms, the
/// power group it draws from, and the live engage/dock state.
///
/// Inserted from one validated Dock mating component and its power input.
/// Passive berths carry only DockMarkers, without active command channels.
///
/// `engaged` is the operator's INTENT to hold a dock; `docked` is whether the
/// mate has actually formed. `docking_target` is the hull the manoeuvre is
/// closing on or holding, latched at `Dock` time. On any interruption `engaged`,
/// `docked` and `docking_target` clear together — the crew watch the dock end.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct DockControl {
    /// The authored `[[system]].id` for the `kind = "dock"` instance. Commands,
    /// damage lookup, AI policy and blackboard publication all use this resolved
    /// identity; `"dock"` is only the conventional id used by shipped hulls.
    pub system_id: SystemId,
    /// The authored dock terms — range, engage distance, approach speed, mate
    /// tolerance, undock clearance, minimum power level.
    pub config: DockConfig,
    /// The power group the dock `[[system]]` declared, resolved once at spawn.
    pub power_group: PowerGroupId,
    /// The operator's standing intent to hold a dock. Set true by `Dock`, false
    /// by `Undock` and by every interruption.
    pub channels: SystemCommandChannels,
    pub enabled_channel: String,
    pub target_channel: String,
    /// Whether the two hulls are actually mated this tick. Only ever true while
    /// `engaged` and `docking_target` is set.
    pub docked: bool,
    /// Where the ship is backing out to during an undock, `Some(clear-pose)`
    /// until it arrives and returns to ordinary flight. Saved and folded so
    /// continuation completes the same mechanical clearance manoeuvre.
    pub undock_target: Option<Vec3>,
    /// The nearest dockable hull within the authored range this tick, resolved
    /// for the console so the contextual control can appear. A projection, not
    /// saved.
    pub available_target: Option<String>,
    /// Why the last dock could not form or was ended — the reason the console
    /// shows, retained until the operator acts again. `None` when idle, forming
    /// cleanly, or docked.
    pub last_refusal: Option<DockRefusal>,
}

impl DockControl {
    /// A fresh, idle dock control carrying its authored System identity, terms
    /// and resolved power group.
    pub fn new(system_id: SystemId, config: DockConfig, power_group: PowerGroupId) -> Self {
        Self {
            system_id,
            config,
            power_group,
            channels: SystemCommandChannels::from_definitions(&[
                CommandChannelConfig {
                    name: "enabled".into(),
                    kind: CommandChannelKind::Boolean,
                    initial: None,
                },
                CommandChannelConfig {
                    name: "target".into(),
                    kind: CommandChannelKind::EntityTarget,
                    initial: None,
                },
            ]),
            enabled_channel: "enabled".into(),
            target_channel: "target".into(),
            docked: false,
            undock_target: None,
            available_target: None,
            last_refusal: None,
        }
    }

    /// Construct from validated authored composition; no legacy data conversion.
    pub fn from_system(system: &crate::ship::config::SystemInstanceConfig) -> Self {
        let binding = dock_composition(&system.components).expect("validated Dock composition");
        let mut control = Self::new(
            system.id.clone(),
            binding.config.clone(),
            system
                .power_group
                .clone()
                .expect("validated Dock power input"),
        );
        control.channels = SystemCommandChannels::from_definitions(&system.channels);
        control.enabled_channel = binding.enabled_channel.into();
        control.target_channel = binding.target_channel.into();
        control
    }

    pub fn engaged(&self) -> bool {
        self.channels.boolean(&self.enabled_channel)
    }
    pub fn docking_target(&self) -> Option<&str> {
        self.channels.target(&self.target_channel)
    }
    pub fn set_engaged(&mut self, value: bool) {
        self.channels
            .set(&self.enabled_channel, CommandChannelValue::Boolean(value))
            .expect("validated engagement channel");
    }
    pub fn set_docking_target(&mut self, value: Option<String>) {
        self.channels
            .set(
                &self.target_channel,
                CommandChannelValue::EntityTarget(value),
            )
            .expect("validated target channel");
    }

    /// Present hardware conditions, independent of historical action refusals.
    pub fn availability_refusal(
        &self,
        power_level: u8,
        disabled: bool,
        has_markers: bool,
    ) -> Option<DockRefusal> {
        if disabled {
            Some(DockRefusal::Disabled)
        } else if power_level < self.config.min_power_level {
            Some(DockRefusal::Unpowered)
        } else if !has_markers {
            Some(DockRefusal::NoDockMarkers)
        } else {
            None
        }
    }

    /// The uuid of the hull this ship is DOCKED to, or `None` when it is not
    /// docked. This is the relationship the umbilical (#1160) gates on: two hulls
    /// are docked when one's `docked_partner()` names the other.
    pub fn docked_partner(&self) -> Option<&str> {
        if self.docked {
            self.docking_target()
        } else {
            None
        }
    }

    /// The persistable half — the engage/dock state and the docked target — for
    /// the snapshot payload. The authored System identity, config and power
    /// group ride the template and are re-derived on spawn, exactly as the
    /// tractor leaves its coupling terms out of `TractorSaveState`.
    pub fn save_state(&self) -> DockSaveState {
        DockSaveState {
            channels: self.channels.clone(),
            docked: self.docked,
            undock_target: self.undock_target.map(|target| target.to_array()),
        }
    }

    /// Reseed the engage/dock state and docked target from a restored snapshot,
    /// onto a control that already carries its authored System identity, config
    /// and resolved power group from the fresh spawn.
    ///
    /// Availability and historical refusal are presentation projections; the
    /// mechanical undock destination is restored with the channel intent.
    pub fn restore(&mut self, save: &DockSaveState) {
        self.channels = save.channels.clone();
        self.docked = save.docked;
        self.undock_target = save.undock_target.map(Vec3::from_array);
        self.available_target = None;
        self.last_refusal = None;
    }
}

/// Authoritative command intent and mechanical mating/clearance state.
/// Channel bindings and tuning are restored from the validated entity template.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockSaveState {
    pub channels: SystemCommandChannels,
    pub docked: bool,
    pub undock_target: Option<[f32; 3]>,
}

/// Shared full-candidate and single-row preflight. No owner is written here.
pub(crate) fn validate_saved_entity(
    world: &World,
    entity: Entity,
    saved: Option<&DockSaveState>,
    physics: Option<&[f32; 8]>,
) -> Result<(), String> {
    match (world.get::<DockControl>(entity), saved) {
        (None, None) => Ok(()),
        (Some(control), Some(saved)) => {
            if control.channels.0.keys().ne(saved.channels.0.keys())
                || control.channels.0.iter().any(|(key, value)| {
                    std::mem::discriminant(value) != std::mem::discriminant(&saved.channels.0[key])
                })
            {
                return Err("saved Dock channels disagree with actual equipment".into());
            }
            let engaged = saved.channels.boolean(&control.enabled_channel);
            let target = saved.channels.target(&control.target_channel);
            if target.is_some_and(|target| {
                target.trim().is_empty() || target.chars().any(char::is_control)
            }) || engaged != target.is_some()
                || saved.docked && !engaged
            {
                return Err("saved Dock relationship disagrees with its accepted intent".into());
            }
            if control.config.moves_body
                && !physics.is_some_and(|pose| pose.iter().all(|value| value.is_finite()))
            {
                return Err("moving Dock requires its one finite canonical physical pose".into());
            }
            if let Some(clear) = saved.undock_target {
                if !control.config.moves_body
                    || engaged
                    || saved.docked
                    || target.is_some()
                    || clear.iter().any(|value| !value.is_finite())
                {
                    return Err(
                        "saved controlled Dock clearance has invalid equipment, phase or geometry"
                            .into(),
                    );
                }
            }
            Ok(())
        }
        _ => Err("saved Dock continuation disagrees with actual equipment presence".into()),
    }
}

/// Registers the dock systems and its admitted-command consumer (issue #1159).
/// Added by `WorldPlugin` alongside `TractorPlugin`.
pub struct DockPlugin;

impl Plugin for DockPlugin {
    fn build(&self, app: &mut App) {
        // The dock `[[system]]` is an admitted-command consumer: `handle_dock_
        // commands` reads `Dock` / `Undock` for it, so admission fans those
        // commands into every ship's `AdmittedCommands` each tick.
        // Gated AI decider (issue #1162); `register_ai_cadence` is idempotent.
        crate::ai::cadence::register_ai_cadence(app);
        // Geometry/programs are reconstructed; private per-System continuation
        // is captured and folded in full, including inactive/human-owned trees.
        {
            use crate::authoritative::{DeclareState, StateClass};
            app.declare_state::<DockMarkers>(StateClass::Derived, "dock-controller")
                .declare_state::<ShipAiTreePrograms>(StateClass::Derived, "system-ai-tree-programs")
                .declare_state::<ShipAiTreeRuntime>(StateClass::Folded, "system-ai-tree-runtime");
        }
        // Registry coverage is keyed by the authoritative System kind, so an
        // arbitrary authored Dock id is linted through the same identity the
        // instance-correct consumer below reads.
        app.register_admitted_consumer(
            ConsumerMatcher::kind(crate::ship::system_registry::DOCK_KIND).with_feedback(
                crate::command_admission::FeedbackAddress::DeclaredKindOrCanonical(
                    crate::ship::system_registry::DOCK_KIND,
                ),
                &[Payload::Dock, Payload::Undock],
            ),
        );
        app.add_systems(
            FixedUpdate,
            (
                // Backfill Helm dock AI (issue #1162): on the shared AI cadence
                // (rule 7), emitting Dock / Undock BEFORE `handle_dock_commands`
                // consumes the tick.
                operate_dock_ai
                    .in_set(crate::sim_sets::FixedStep::OperateDockAi)
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready)
                    .before(handle_dock_commands),
                handle_dock_commands
                    .in_set(crate::sim_sets::FixedStep::HandleDockCommands)
                    .in_set(crate::sim_sets::SimSet::Input),
                // Decide the dock and fly the own ship onto its mate, after the
                // tractor rig so a hull that is both a docking ship and a tractor
                // target has a deterministic last writer (see module docs).
                tick_dock
                    .in_set(crate::sim_sets::FixedStep::TickDock)
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(crate::tractor::server::move_coupled_target),
                publish_dock_blackboard
                    .in_set(crate::sim_sets::FixedStep::PublishDockBlackboard)
                    .in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

fn physical_pose(transform: &Transform, physics: Option<&ShipPhysics>) -> Pose {
    physics.map_or(
        Pose {
            translation: transform.translation,
            rotation: transform.rotation,
        },
        |physics| Pose {
            translation: Vec3::new(physics.x, physics.y, physics.z),
            rotation: Quat::from_euler(EulerRot::YXZ, -physics.yaw, 0.0, physics.roll),
        },
    )
}
fn has_dock_equipment(
    config: Option<&crate::ship_plugin::ShipConfigComponent>,
    owner: &SystemId,
) -> bool {
    config
        .and_then(|config| config.0.system(owner))
        .is_some_and(|system| {
            system.kind == crate::ship::system_registry::DOCK_KIND
                && dock_composition(&system.components).is_some()
        })
}

// ── The dock / undock commands ───────────────────────────────────────────────

/// Runs in `SimSet::Input` and reads `AdmittedCommands` for the dock system
/// (issue #1159). It sets only intent; `tick_dock` decides whether a mate forms
/// and records any refusal.
///
/// Human and AI reach this identically: admission has already decided who may
/// speak and stripped the source, so nothing here asks who sent the command
/// (AGENTS.md rule 6). `Dock` latches the nearest available target `tick_dock`
/// published last tick; `Undock` ends the mate and starts the backing manoeuvre.
#[allow(clippy::type_complexity)]
pub fn handle_dock_commands(
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut ships: Query<(
        &crate::core::messages::AdmittedCommands,
        &Transform,
        &mut DockControl,
        Option<&EntityUuid>,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        Option<&crate::server_app::ShipSystemBlackboards>,
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::science::broad::SensingControls>,
        Option<&ShipPhysics>,
        Option<&DockMarkers>,
    )>,
    partners: Query<(&EntityUuid, &Transform, &DockMarkers, Option<&ShipPhysics>)>,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    for (admitted, transform, mut dock, uuid, config, boards, tracks, controls, physics, markers) in
        ships.iter_mut()
    {
        // Clone the resolved target string before mutating the component below.
        // The id itself remains authored topology, not a per-command decision.
        let system_id = dock.system_id.0.clone();
        for cmd in admitted.for_target(&system_id) {
            let outcome = match &cmd.payload {
                SystemControlPayload::Dock if !dock.engaged() && dock.config.moves_body => {
                    match dock.available_target.clone().filter(|target| {
                        crate::science::contact_routing::machinery_alias(
                            config.map(|config| &config.0),
                            boards,
                            tracks,
                            controls,
                            &dock.system_id,
                            target,
                        )
                        .is_some()
                    }) {
                        Some(target) => {
                            dock.set_engaged(true);
                            dock.docked = false;
                            dock.set_docking_target(Some(target));
                            dock.undock_target = None;
                            dock.last_refusal = None;
                            ActionFeedbackOutcome::Applied
                        }
                        // Nothing dockable in range — refuse by name rather than
                        // engaging a manoeuvre with no berth to reach. A hull that
                        // declares no dock markers of its own publishes no
                        // available target either, and `tick_dock` surfaces the
                        // sharper "no markers" reason for it while idle.
                        None => {
                            dock.last_refusal = Some(DockRefusal::NoTarget);
                            ActionFeedbackOutcome::Refused
                        }
                    }
                }
                SystemControlPayload::Undock if dock.engaged() || dock.docked => {
                    let clear = if dock.docked && dock.config.moves_body {
                        if !has_dock_equipment(config, &dock.system_id) {
                            dock.last_refusal = Some(DockRefusal::Disabled);
                            crate::command_admission::finish_action_feedback(
                                cmd,
                                &mut outbound,
                                ActionFeedbackOutcome::Refused,
                            );
                            continue;
                        }
                        let own = physical_pose(transform, physics);
                        let partner = dock
                            .docking_target()
                            .and_then(|target| {
                                partners.iter().find(|(uuid, _, _, _)| uuid.0 == target)
                            })
                            .map(|(_, transform, markers, physics)| {
                                (
                                    physical_pose(transform, physics),
                                    markers.markers.as_slice(),
                                )
                            });
                        let clear = crate::dock::mating::separation_destination(
                            own,
                            markers.map_or(&[], |markers| markers.markers.as_slice()),
                            partner,
                            dock.config.undock_clear_distance,
                        );
                        let Some(clear) = clear else {
                            dock.last_refusal = Some(DockRefusal::NoDockMarkers);
                            crate::command_admission::finish_action_feedback(
                                cmd,
                                &mut outbound,
                                ActionFeedbackOutcome::Refused,
                            );
                            continue;
                        };
                        Some(clear)
                    } else {
                        None
                    };
                    // Report the cancel BEFORE clearing intent, so a release of an
                    // idle dock (a stale-UI double tap) reports nothing at all
                    // (issue #1345). A live activation exists only once
                    // `tick_dock` has seen the mate form (`docked` becomes true);
                    // an undock of a still-approaching dock finds nothing live in
                    // the registry and this report is dropped harmlessly — the
                    // same "exactly one terminal" rule the tractor relies on.
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        uuid,
                        TaskLifecycleRequest::End {
                            slot: hold_slot(&dock.system_id, uuid),
                            reason: TaskTerminalReason::Released,
                        },
                    );
                    dock.set_engaged(false);
                    dock.docked = false;
                    dock.set_docking_target(None);
                    dock.undock_target = clear;
                    dock.last_refusal = None;
                    ActionFeedbackOutcome::Applied
                }
                SystemControlPayload::Dock | SystemControlPayload::Undock => {
                    ActionFeedbackOutcome::Refused
                }
                _ => continue,
            };
            crate::command_admission::finish_action_feedback(cmd, &mut outbound, outcome);
        }
    }
}

/// The lifecycle slot a hull's dock hold occupies (issue #1345) — its uuid, the
/// dock's own authored System identity (never a literal `"dock"`, since a hull
/// may author any instance id), and the dock-hold verb.
fn hold_slot(system_id: &SystemId, uuid: Option<&EntityUuid>) -> TaskSlot {
    TaskSlot::new(
        uuid.map(|u| u.0.clone()).unwrap_or_default(),
        system_id.0.clone(),
        TASK_VERB_DOCK_HOLD,
    )
}

/// Queue one lifecycle report, if there is a queue and the hull has a uuid to be
/// identified by (issue #1345). Mirrors `tractor::server::push_lifecycle`: a
/// hull with no [`EntityUuid`] reports nothing and behaves exactly as it did
/// before this existed.
fn push_lifecycle(
    queue: Option<&mut EffectQueue<TaskLifecycleRequest>>,
    uuid: Option<&EntityUuid>,
    request: TaskLifecycleRequest,
) {
    if uuid.is_none() {
        return;
    }
    if let Some(queue) = queue {
        queue.0.push(request);
    }
}

/// The terminal reason a dropped (or never-formed) dock reports, from the
/// refusal the pure mating module returned (issue #1345).
///
/// The refusal vocabularies line up closely with the tractor's: `TargetLost` is
/// the berth vanishing under a live mate, `OutOfRange`/`Unpowered`/`Disabled` are
/// the dock's own preconditions failing. `NoDockMarkers` — a hull that can never
/// mate at all — is `NotCapable`, the same reason a hull with no marker family
/// reports for any other capability it lacks. `NoTarget` only fires while
/// `decide_dock` is `engaged` with no `docking_target`, a state `handle_dock_
/// commands` never allows (`Dock` sets both together) — mapped defensively so
/// the match stays exhaustive, never expected to reach the timeline.
fn terminal_reason_for(refusal: DockRefusal) -> TaskTerminalReason {
    match refusal {
        DockRefusal::NoDockMarkers => TaskTerminalReason::NotCapable,
        DockRefusal::NoTarget => TaskTerminalReason::NoSuchTarget,
        DockRefusal::OutOfRange => TaskTerminalReason::OutOfRange,
        DockRefusal::Unpowered => TaskTerminalReason::Unpowered,
        DockRefusal::Disabled => TaskTerminalReason::Disabled,
        DockRefusal::TargetLost => TaskTerminalReason::TargetLost,
    }
}

/// Resolve a directive's named target to the UUID a dock's `available_target`
/// carries (issue #1162): a world entity NAME through the runtime map, or the
/// value itself when it is already a UUID (or the runtime is absent).
fn resolve_dock_target(name: &str, runtime: Option<&WorldContentRuntime>) -> String {
    runtime
        .and_then(|rt| rt.name_to_uuid.get(name).cloned())
        .unwrap_or_else(|| name.to_string())
}

/// Backfill Helm dock AI (issue #1162).
///
/// On an active `Transfer` directive (Helm affinity) naming a target the dock
/// reports available in range, engage the dock; with no such directive active,
/// undock. The concrete command is exactly the `Dock`/`Undock` a human helmsman
/// emits, sent through the SAME `emit_ai_command` seam so `handle_dock_commands`
/// never learns who spoke (AGENTS.md rule 6). "When the procedure requires it":
/// the dock waits until `available_target` names the ordered hull (published by
/// `tick_dock` once it is in range), exactly as the contextual control only
/// appears for a human then. Decides ONLY on the shared AI cadence (rule 7).
#[allow(clippy::type_complexity)]
pub fn operate_dock_ai(
    mut commands: Commands,
    tick: Res<crate::sim_tick::SimTick>,
    sessions: Res<crate::lobby::Sessions>,
    runtime: Option<Res<WorldContentRuntime>>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut ships: Query<(
        Entity,
        Option<&EntityUuid>,
        &crate::ship_plugin::ShipSystemControlSources,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        &DockControl,
        &crate::server_app::ShipSystemBlackboards,
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::science::broad::SensingControls>,
        &ShipAiTreePrograms,
        &ShipAiTreeRuntime,
        &mut crate::core::messages::AdmittedCommands,
    )>,
) {
    for (
        entity,
        uuid,
        sources,
        config,
        dock,
        blackboards,
        tracks,
        controls,
        programs,
        runtimes,
        mut admitted,
    ) in ships.iter_mut()
    {
        if !sources.0.policy_for(&dock.system_id).operate_ai {
            continue;
        }
        let directive_target: Option<String> = match config.and_then(|config| {
            config
                .0
                .observations_for_system(&dock.system_id, &blackboards.0)
                .get(&crate::ship::system_registry::viewscreen_system_id())
        }) {
            Some(SystemBlackboard::Viewscreen(vbb)) => crate::objectives::top_operate_directive(
                &vbb.scored_objectives,
                SystemAffinity::Helm,
                |d| crate::objectives::transfer_directive_target(d).is_some(),
            )
            .and_then(crate::objectives::transfer_directive_target)
            .map(str::to_string),
            _ => None,
        };

        let ready = directive_target.as_deref().is_some_and(|name| {
            let resolved = if crate::science::contact_routing::measured_machinery(tracks, controls)
            {
                config
                    .zip(tracks)
                    .and_then(|(config, tracks)| {
                        crate::science::contact_routing::request_contact(
                            &config.0,
                            blackboards,
                            tracks,
                            &dock.system_id,
                            name,
                        )
                    })
                    .map(|(_, source)| source)
            } else {
                Some(resolve_dock_target(name, runtime.as_deref()))
            };
            dock.available_target
                .as_deref()
                .is_some_and(|a| Some(a) == resolved.as_deref())
        });
        let Some(program) = programs.0.get(&dock.system_id) else {
            continue;
        };
        let mut next_runtimes = runtimes.clone();
        let Some(tree) = next_runtimes.0.get_mut(&dock.system_id) else {
            continue;
        };
        let was_owned = tree.memory("owned").unwrap_or(false);
        let active = dock.engaged() || dock.docked;
        let facts = std::collections::BTreeMap::from([
            ("order_present".into(), directive_target.is_some()),
            ("ready".into(), ready),
            ("active".into(), active),
            ("engaged".into(), dock.engaged()),
        ]);
        let channels =
            std::collections::BTreeMap::from([(dock.enabled_channel.clone(), dock.engaged())]);
        let result = match decide_tree(
            &sources.0,
            &dock.system_id,
            Some(program),
            &facts,
            &channels,
            tree,
            tick.0,
        ) {
            Ok(TreeHostOutcome::Evaluated(result)) => result,
            Ok(_) => continue,
            Err(error) => {
                warn!(system = %dock.system_id.0, %error, "Dock AI tree evaluation refused");
                continue;
            }
        };
        // Retain the controller's existing deferred boundary: publish successful
        // private memory at the same flush that previously applied Dock ownership.
        commands.entity(entity).insert(next_runtimes);
        let payload = match result.intents.get(&dock.enabled_channel).copied() {
            Some(true) if !dock.engaged() => Some(SystemControlPayload::Dock),
            Some(false) if active => {
                if was_owned && directive_target.is_none() {
                    // Preserve withdrawal before the shared handler's Released
                    // report, without introducing a second policy decision.
                    push_lifecycle(
                        lifecycle.as_deref_mut(),
                        uuid,
                        TaskLifecycleRequest::End {
                            slot: hold_slot(&dock.system_id, uuid),
                            reason: TaskTerminalReason::OrderWithdrawn,
                        },
                    );
                }
                Some(SystemControlPayload::Undock)
            }
            _ => None,
        };

        if let Some(payload) = payload {
            emit_ai_command(
                uuid,
                dock.system_id.clone(),
                payload,
                sources,
                &sessions,
                config,
                &mut admitted,
            );
        }
    }
}

// ── The dock tick ────────────────────────────────────────────────────────────

/// A hull carrying dock markers, read once per tick into plain values so the
/// borrow of the world is released before the own-ship write.
struct Candidate {
    uuid: String,
    pose: Pose,
    markers: Vec<DockMarker>,
}

/// What `tick_dock` decides for one own ship, applied in the write phase.
struct DockOutcome {
    entity: Entity,
    engaged: bool,
    docked: bool,
    docking_target: Option<String>,
    undock_target: Option<Vec3>,
    available_target: Option<String>,
    last_refusal: Option<DockRefusal>,
    /// The pose to write onto the own ship this tick, when the manoeuvre moved
    /// it. `None` leaves the ship where its own flight put it.
    new_pose: Option<Pose>,
}

/// Decide every dock this tick and fly each engaged own ship onto (or off) its
/// mate (issue #1159).
///
/// Reads the own ship's dock markers, the candidate hulls' markers and poses, the
/// dock's power level and damage tier, hands the geometry to the pure
/// [`nearest_viable_pair`], and applies the verdict. The close-in motion is a
/// thin adapter over [`crate::ai::docking_close_manoeuvre`] — the same low-speed
/// mate the AI helm flies — reused for the direction, advanced at the authored
/// approach speed. Each interruption (drift past range, unpowered, disabled,
/// target lost) ends the dock cleanly.
#[allow(clippy::type_complexity)]
pub fn tick_dock(
    time: Res<Time>,
    mut lifecycle: Option<ResMut<EffectQueue<TaskLifecycleRequest>>>,
    mut set: ParamSet<(
        // Own-ship rows: everything the verdict needs off the docker itself.
        Query<(
            Entity,
            &DockControl,
            &Transform,
            Option<&DockMarkers>,
            Option<&mut ShipPowerSystem>,
            Option<&EntitySystemHull>,
            Option<&ShipDamage>,
            &EntityUuid,
            (
                Option<&ShipPhysics>,
                Option<&crate::ship_plugin::ShipSystemControlSources>,
                Option<&crate::ship_plugin::ShipConfigComponent>,
                Option<&crate::resources::fitting_server::CompiledShipFitting>,
                Option<&crate::resources::fitting_server::ShipFitting>,
            ),
        )>,
        // Every hull carrying dock markers, to resolve candidates and targets.
        Query<(&EntityUuid, &Transform, &DockMarkers, Option<&ShipPhysics>)>,
        // Apply the verdict and any placement.
        Query<(
            &mut DockControl,
            &mut Transform,
            Option<&mut ShipPhysics>,
            &EntityUuid,
        )>,
    )>,
) {
    let dt = time.delta_secs();

    // Every dockable hull, read once.
    let candidates: Vec<Candidate> = set
        .p1()
        .iter()
        .filter(|(_, _, m, _)| !m.is_empty())
        .map(|(uuid, transform, markers, physics)| Candidate {
            uuid: uuid.0.clone(),
            pose: physical_pose(transform, physics),
            markers: markers.markers.clone(),
        })
        .collect();

    // Decide each own ship without holding the write borrow.
    let mut outcomes: Vec<DockOutcome> = Vec::new();
    for (
        entity,
        dock,
        transform,
        markers,
        mut power,
        hull,
        damage,
        uuid,
        (physics, sources, config, fitting_profile, fitting_state),
    ) in set.p0().iter_mut()
    {
        let own_pose = physical_pose(transform, physics);
        let own_markers: &[DockMarker] = markers.map(|m| m.markers.as_slice()).unwrap_or(&[]);
        // Allocation is incoming flow, not equipment availability. Geometry is
        // checked before paying, so impossible/idle attempts consume no energy.
        let power_level = u8::MAX;
        let disabled = operation_disabled(damage, hull, &dock.system_id);

        let mut outcome = decide_dock(
            entity,
            &uuid.0,
            dock,
            own_pose,
            own_markers,
            power_level,
            disabled,
            &candidates,
            dt,
        );
        if dock.config.moves_body && physics.is_none() {
            // Missing the accepted physical owner cannot fall back to an unsaved
            // Transform path. Keep the existing work until the owner is repaired.
            outcome.engaged = dock.engaged();
            outcome.docked = dock.docked;
            outcome.docking_target = dock.docking_target().map(str::to_owned);
            outcome.new_pose = None;
            outcome.undock_target = dock.undock_target;
            outcome.last_refusal = Some(DockRefusal::Disabled);
            outcomes.push(outcome);
            continue;
        }
        if !dock.config.moves_body {
            // A berth may retain/release an existing coupling but never moves.
            outcome.new_pose = None;
            outcome.available_target = None;
            if dock.undock_target.is_some() {
                outcome.undock_target = None;
            }
            outcomes.push(outcome);
            continue;
        }
        if let Some(clear) = dock.undock_target {
            let online = has_dock_equipment(config, &dock.system_id)
                && sources.is_some_and(|sources| {
                    sources.0.entries().any(|(id, _)| id == &dock.system_id)
                        && !sources.0.is_offline(&dock.system_id)
                        && sources.0.source_for(&dock.system_id)
                            != crate::ship::control_source::ControlSource::Offline
                })
                && match (fitting_profile, fitting_state) {
                    (Some(profile), Some(state)) => !crate::resources::fitting_server::unavailable(
                        profile,
                        state,
                        &dock.system_id,
                    ),
                    (None, None) => true,
                    _ => false,
                };
            let operation = damage.map_or(0.0, |damage| {
                damage.factor(
                    &dock.system_id,
                    phoenix_model::damage::DamageBinding::Operation,
                )
            });
            let distance = planar_distance(own_pose.translation, clear);
            let work_secs = (f64::from(dt) * f64::from(operation))
                .min(distance / f64::from(dock.config.approach_speed))
                as f32;
            let fraction = if online && !disabled && work_secs > 0.0 {
                power.as_deref_mut().map_or(0.0, |power| {
                    power
                        .0
                        .work_continuous(&dock.system_id, work_secs, 1.0)
                        .map_or(0.0, |grant| grant.fraction)
                })
            } else {
                0.0
            };
            if fraction > 0.0 {
                outcome = decide_dock(
                    entity,
                    &uuid.0,
                    dock,
                    own_pose,
                    own_markers,
                    power_level,
                    false,
                    &candidates,
                    work_secs * fraction,
                );
            } else if distance > 0.0 {
                outcome.new_pose = None;
                outcome.undock_target = Some(clear);
                outcome.last_refusal = Some(if !online || disabled || operation <= 0.0 {
                    DockRefusal::Disabled
                } else {
                    DockRefusal::Unpowered
                });
            } else {
                // Already at the accepted clear point: settling needs no travel work.
                outcome = decide_dock(
                    entity,
                    &uuid.0,
                    dock,
                    own_pose,
                    own_markers,
                    power_level,
                    false,
                    &candidates,
                    0.0,
                );
            }
            outcomes.push(outcome);
            continue;
        }
        if outcome.new_pose.is_some() && !disabled {
            let moving = outcome.new_pose.as_ref().is_some_and(|pose| {
                pose.translation != own_pose.translation || pose.rotation != own_pose.rotation
            });
            let fraction = if moving {
                power.as_deref_mut().map_or(0.0, |power| {
                    power
                        .0
                        .work_continuous(&dock.system_id, dt, 1.0)
                        .map_or(0.0, |grant| grant.fraction)
                })
            } else {
                1.0
            };
            let fraction = fraction
                * damage.map_or(1.0, |damage| {
                    damage.factor(
                        &dock.system_id,
                        phoenix_model::damage::DamageBinding::Operation,
                    )
                });
            if fraction < 1.0 {
                outcome = decide_dock(
                    entity,
                    &uuid.0,
                    dock,
                    own_pose,
                    own_markers,
                    power_level,
                    disabled,
                    &candidates,
                    dt * fraction,
                );
                // The mate snap has no travel-time term. Partially funded
                // equipment must not complete that snap at full strength.
                if outcome.docked && fraction < 1.0 {
                    if let Some(pose) = outcome.new_pose.as_mut() {
                        pose.translation = own_pose.translation.lerp(pose.translation, fraction);
                        pose.rotation = own_pose.rotation.slerp(pose.rotation, fraction);
                    }
                    if !dock.docked {
                        outcome.docked = false;
                    }
                }
                if fraction == 0.0 {
                    outcome.new_pose = None;
                    outcome.last_refusal = Some(DockRefusal::Unpowered);
                }
            }
            if !dock.docked
                && outcome.docked
                && !power.as_deref_mut().is_some_and(|power| {
                    power.0.try_work_discrete(&dock.system_id).unwrap_or(false)
                })
            {
                outcome.docked = false;
                outcome.new_pose = None;
                outcome.last_refusal = Some(DockRefusal::Unpowered);
            }
        }
        outcomes.push(outcome);
    }

    // Apply.
    let mut writes = set.p2();
    for out in outcomes {
        let Ok((mut dock, mut transform, physics, uuid)) = writes.get_mut(out.entity) else {
            continue;
        };
        // The task activation follows the MATE forming, never the intent (issue
        // #1345) — mirroring `tick_tractor`'s "the activation follows the
        // coupling" rule, because the mate is the only thing that knows what the
        // dock actually gripped. Considered only for a row that was actually
        // trying (`engaged`) or holding (`docked`); an idle dock merely
        // publishing "no markers" or "nothing in range" is not an attempt and
        // opens nothing.
        // A missing required physical owner holds the relationship without
        // attempting work. Do not turn that pause into a fresh failed task.
        if (!dock.config.moves_body || physics.is_some()) && (dock.engaged() || dock.docked) {
            let work = if !dock.docked && out.docked {
                Some(PhysicalWork::Formed {
                    target: out.docking_target.as_deref(),
                    follow_subject: false,
                })
            } else if !out.docked {
                out.last_refusal.map(|refusal| PhysicalWork::Refused {
                    target: dock.docking_target(),
                    reason: terminal_reason_for(refusal),
                })
            } else {
                None
            };
            if let Some(work) = work {
                for report in physical_work_reports(
                    hold_slot(&dock.system_id, Some(uuid)),
                    PriorActivation {
                        active: dock.docked,
                        target: dock.docking_target(),
                    },
                    work,
                ) {
                    push_lifecycle(lifecycle.as_deref_mut(), Some(uuid), report);
                }
            }
        }

        dock.set_engaged(out.engaged);
        dock.docked = out.docked;
        dock.set_docking_target(out.docking_target);
        dock.undock_target = out.undock_target;
        dock.available_target = out.available_target;
        dock.last_refusal = out.last_refusal;
        if let Some(pose) = out.new_pose {
            transform.translation = pose.translation;
            transform.rotation = pose.rotation;
            if let Some(mut physics) = physics {
                physics.x = pose.translation.x;
                physics.y = pose.translation.y;
                physics.z = pose.translation.z;
                let (yaw, _, _) = pose.rotation.to_euler(EulerRot::YXZ);
                physics.yaw = -yaw;
                // A ship parked on (or backing off) a berth carries no residual
                // velocity — the same reason the tractor zeroes a released load.
                physics.forward_speed = 0.0;
                physics.lateral_speed = 0.0;
                physics.vertical_speed = 0.0;
            }
        }
    }
}

/// Decide one own ship's dock for this tick, and where (if anywhere) to place it.
///
/// Pulled out of the system so the state machine is one place with no world
/// borrow in scope. The pure [`nearest_viable_pair`] owns the geometry; this
/// orders the interruptions and steps the approach/undock motion.
#[allow(clippy::too_many_arguments)]
fn decide_dock(
    entity: Entity,
    own_uuid: &str,
    dock: &DockControl,
    own_pose: Pose,
    own_markers: &[DockMarker],
    power_level: u8,
    disabled: bool,
    candidates: &[Candidate],
    dt: f32,
) -> DockOutcome {
    let cfg = &dock.config;
    let idle = |available: Option<String>, refusal: Option<DockRefusal>| DockOutcome {
        entity,
        engaged: false,
        docked: false,
        docking_target: None,
        undock_target: None,
        available_target: available,
        last_refusal: refusal,
        new_pose: None,
    };

    // The nearest viable partner within range, ignoring self — recomputed every
    // tick so the console control appears and disappears with proximity.
    let nearest_available = || -> Option<String> {
        let mut best: Option<(String, f32)> = None;
        for c in candidates {
            if c.uuid == own_uuid {
                continue;
            }
            if let Some(sol) = nearest_viable_pair(own_pose, own_markers, c.pose, &c.markers) {
                if sol.separation <= cfg.range
                    && best
                        .as_ref()
                        .map(|(_, s)| sol.separation < *s)
                        .unwrap_or(true)
                {
                    best = Some((c.uuid.clone(), sol.separation));
                }
            }
        }
        best.map(|(u, _)| u)
    };

    // ── Undocking: back straight out, then return to ordinary flight ──────────
    if let Some(clear) = dock.undock_target {
        if disabled {
            return DockOutcome {
                entity,
                engaged: false,
                docked: false,
                docking_target: None,
                undock_target: Some(clear),
                available_target: None,
                last_refusal: Some(DockRefusal::Disabled),
                new_pose: None,
            };
        }

        let (stepped, arrived) = step_toward(own_pose.translation, clear, cfg.approach_speed, dt);
        if arrived {
            // Clear of the berth; resume ordinary flight and re-offer the control
            // if still near a partner.
            return DockOutcome {
                entity,
                engaged: false,
                docked: false,
                docking_target: None,
                undock_target: None,
                available_target: nearest_available(),
                last_refusal: None,
                new_pose: Some(Pose {
                    translation: stepped,
                    rotation: own_pose.rotation,
                }),
            };
        }
        return DockOutcome {
            entity,
            engaged: false,
            docked: false,
            docking_target: None,
            undock_target: Some(clear),
            available_target: None,
            last_refusal: None,
            new_pose: Some(Pose {
                translation: stepped,
                rotation: own_pose.rotation,
            }),
        };
    }

    // ── Idle: publish availability, no motion ─────────────────────────────────
    if !dock.engaged() {
        // A hull that declares no dock markers can never dock, and says so the
        // moment it is idle — the console shows the reason rather than a control
        // that grips nothing.
        if own_markers.is_empty() {
            return idle(None, dock.last_refusal.or(Some(DockRefusal::NoDockMarkers)));
        }
        return idle(nearest_available(), dock.last_refusal);
    }

    // ── Engaged: approach or hold the mate ────────────────────────────────────
    let end = |refusal: DockRefusal| idle(None, Some(refusal));

    // Hardware and power before target acquisition — most-actionable-first, the
    // tractor's check order.
    if let Some(refusal) = dock.availability_refusal(power_level, disabled, !own_markers.is_empty())
    {
        return end(refusal);
    }
    let Some(target_uuid) = dock.docking_target().map(str::to_owned) else {
        return end(DockRefusal::NoTarget);
    };
    let Some(target) = candidates.iter().find(|c| c.uuid == target_uuid) else {
        // The berth is gone — destroyed or despawned mid-manoeuvre.
        return end(DockRefusal::TargetLost);
    };
    let Some(sol) = nearest_viable_pair(own_pose, own_markers, target.pose, &target.markers) else {
        return end(DockRefusal::NoDockMarkers);
    };
    if sol.separation > cfg.range {
        // Drifted apart — while forming or while held, the same reason.
        return end(DockRefusal::OutOfRange);
    }

    // Distance from the ship origin to the mate pose it must reach.
    let to_mate = sol.mate.translation - own_pose.translation;
    let mate_dist = Vec3::new(to_mate.x, 0.0, to_mate.z).length();

    if !dock.docked && mate_dist > cfg.mate_tolerance {
        // Still closing: the thin adapter over `docking_close_manoeuvre`. Ask it
        // for the ship-local close-in intent onto the mate point, rotate that
        // back to world, and advance the ship at the authored approach speed.
        let step_dir = close_in_world_dir(own_pose, sol.mate.translation, cfg.engage_distance)
            // Beyond the engage distance the manoeuvre is not yet live; close
            // straight in (the probe authors engage_distance >= range, so this
            // fallback only guards a hull whose markers sit far off-centre).
            .unwrap_or_else(|| {
                let d = Vec3::new(to_mate.x, 0.0, to_mate.z);
                d.normalize_or_zero()
            });
        let step_len = (cfg.approach_speed * dt).min(mate_dist);
        let stepped = own_pose.translation + step_dir * step_len;
        return DockOutcome {
            entity,
            engaged: true,
            docked: false,
            docking_target: Some(target_uuid),
            undock_target: None,
            available_target: Some(target.uuid.clone()),
            last_refusal: None,
            new_pose: Some(Pose {
                translation: stepped,
                rotation: own_pose.rotation,
            }),
        };
    }

    // Mated: snap onto the exact mate pose (following the berth if it drifts
    // inside range) and hold — the docked relationship between the two hulls.
    DockOutcome {
        entity,
        engaged: true,
        docked: true,
        docking_target: Some(target_uuid.clone()),
        undock_target: None,
        available_target: Some(target_uuid),
        last_refusal: None,
        new_pose: Some(sol.mate),
    }
}

/// Step a point toward a goal at `speed` units/second for `dt` seconds, bounded
/// so it never overshoots. Returns the new point and whether the goal was
/// reached this tick. Planar — the vertical is carried straight through.
fn planar_distance(from: Vec3, to: Vec3) -> f64 {
    let dx = f64::from(to.x) - f64::from(from.x);
    let dz = f64::from(to.z) - f64::from(from.z);
    let scale = dx.abs().max(dz.abs());
    if scale == 0.0 {
        return 0.0;
    }
    scale
        * f64::from(crate::simmath::hypot(
            (dx / scale) as f32,
            (dz / scale) as f32,
        ))
}
fn step_toward(from: Vec3, to: Vec3, speed: f32, dt: f32) -> (Vec3, bool) {
    let distance = planar_distance(from, to);
    let step = (f64::from(speed) * f64::from(dt)).max(0.0);
    if distance <= step {
        return (Vec3::new(to.x, from.y, to.z), true);
    }
    if step == 0.0 {
        return (from, false);
    }
    let fraction = step / distance;
    (
        Vec3::new(
            (f64::from(from.x) + (f64::from(to.x) - f64::from(from.x)) * fraction) as f32,
            from.y,
            (f64::from(from.z) + (f64::from(to.z) - f64::from(from.z)) * fraction) as f32,
        ),
        false,
    )
}

/// The world-space unit direction the close-in docking manoeuvre would drive the
/// ship this tick, obtained from [`crate::ai::docking_close_manoeuvre`] and
/// rotated back out of the ship frame (issue #1159).
///
/// `docking_close_manoeuvre` returns the mate direction decomposed into the
/// ship's own `[starboard, aft]` axes; this reverses that decomposition —
/// `world = lateral·(cos,sin) + aft·(-sin,cos)` on `(x,z)` — so the caller
/// advances the ship along the very heading the AI helm's close-in manoeuvre
/// would. Returns `None` when the mate is beyond `engage_distance` (the manoeuvre
/// is not yet live) or coincident with the ship.
fn close_in_world_dir(own: Pose, mate: Vec3, engage_distance: f32) -> Option<Vec3> {
    let (yaw, _, _) = own.rotation.to_euler(EulerRot::YXZ);
    let [lateral, aft] = crate::ai::docking_close_manoeuvre(
        own.translation.x,
        own.translation.z,
        yaw,
        mate.x,
        mate.z,
        engage_distance,
        1.0,
    )?;
    let cos = crate::simmath::cos(yaw);
    let sin = crate::simmath::sin(yaw);
    let world_x = lateral * cos - aft * sin;
    let world_z = lateral * sin + aft * cos;
    let dir = Vec3::new(world_x, 0.0, world_z);
    (dir.length_squared() > 1e-8).then(|| dir.normalize())
}

// ── The wire ─────────────────────────────────────────────────────────────────

/// Publish each dock-carrying ship's blackboard under its system id (issue
/// #1159).
///
/// Only ships that carry [`DockControl`] publish one, so a world whose hulls
/// author no `[dock]` system puts exactly the payload on the wire it did before
/// this existed. No English crosses: target names are world entity name ids and
/// the refusal is the pure module's `strings.csv` id.
pub fn publish_dock_blackboard(
    mut ships: Query<(
        &DockControl,
        &mut crate::server_app::ShipSystemBlackboards,
        Option<&crate::ship_plugin::ShipConfigComponent>,
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::science::broad::SensingControls>,
        Option<&ShipPhysics>,
        Option<&Transform>,
    )>,
    named: Query<(&EntityUuid, &EntityName)>,
) {
    let name_of = |uuid: &Option<String>| -> Option<String> {
        uuid.as_ref().and_then(|u| {
            named
                .iter()
                .find(|(id, _)| &id.0 == u)
                .map(|(_, name)| name.0.clone())
        })
    };
    for (dock, mut blackboards, config, tracks, controls, physics, transform) in ships.iter_mut() {
        let key = &dock.system_id;
        let measured = crate::science::contact_routing::measured_machinery(tracks, controls);
        let known = |target: &str| {
            crate::science::contact_routing::machinery_alias(
                config.map(|config| &config.0),
                Some(&blackboards),
                tracks,
                controls,
                key,
                target,
            )
            .is_some()
        };
        let available = dock.available_target.clone().filter(|target| known(target));
        let name = |target: Option<&str>| {
            if measured {
                config.zip(tracks).and_then(|(config, tracks)| {
                    target.and_then(|target| {
                        crate::science::contact_routing::identity_for_source(
                            &config.0,
                            &blackboards,
                            tracks,
                            key,
                            target,
                        )
                    })
                })
            } else {
                name_of(&target.map(str::to_owned))
            }
        };
        let blackboard = SystemBlackboard::Dock(DockBlackboard {
            separating: dock.undock_target.is_some(),
            separation_remaining: dock.undock_target.and_then(|clear| {
                physics
                    .map(|physics| Vec3::new(physics.x, physics.y, physics.z))
                    .or_else(|| transform.map(|transform| transform.translation))
                    .map(|position| {
                        planar_distance(position, clear).min(f64::from(f32::MAX)) as f32
                    })
            }),
            range: dock.config.range,
            available: dock.config.moves_body && available.is_some() && !dock.docked,
            available_target: available.clone(),
            available_target_name: name(available.as_deref()),
            engaged: dock.engaged(),
            docked: dock.docked,
            docked_to: dock.docked_partner().map(|s| s.to_string()),
            docked_to_name: name(dock.docked_partner()),
            refusal: dock.last_refusal.map(|r| r.string_id().to_string()),
        });
        if blackboards.0.get(key) != Some(&blackboard) {
            blackboards.0.insert(key.clone(), blackboard);
        }
    }
}

/// The conventional shipped dock blackboard key (issue #1159).
///
/// Runtime publication uses [`DockControl::system_id`], resolved from the
/// authored `kind = "dock"` instance. This helper remains for canonical
/// topology fixtures and callers that deliberately author `id = "dock"`.
pub fn dock_blackboard_key() -> SystemId {
    dock_system_id()
}

/// Resolve a hull's dock markers from its rig sidecar into its own frame (issue
/// #1159), for the spawn path to attach as a [`DockMarkers`] component.
///
/// A dock marker is any `[markers.<name>]` whose name begins with `dock` — the
/// same rig-marker vocabulary that already carries engines, cameras and weapon
/// hardpoints, identified by a reserved name prefix. The base rig is folded in
/// here so the stored markers are ship-local, and they are sorted by name so the
/// nearest-pair indices are stable across hosts. Returns an empty vec when the
/// sidecar declares no dock markers — the hull can then never be docked with.
pub fn resolve_dock_markers(rig: &crate::entities::model_rig::ModelRig) -> DockMarkers {
    let base = rig.base_bevy_transform();
    let mut named: Vec<(&String, &crate::entities::model_rig::Marker)> = rig
        .markers
        .iter()
        .filter(|(name, _)| name.starts_with(DOCK_MARKER_PREFIX))
        .collect();
    named.sort_by(|a, b| a.0.cmp(b.0));
    let markers = named
        .into_iter()
        .map(|(_, m)| {
            let position = base.transform_point(Vec3::from_array(m.position));
            let raw_dir = base.rotation * (base.scale * Vec3::from_array(m.direction));
            let direction = if raw_dir.length_squared() > 1e-8 {
                raw_dir.normalize()
            } else {
                raw_dir
            };
            DockMarker {
                position,
                direction,
            }
        })
        .collect();
    DockMarkers { markers }
}

/// The reserved rig-marker name prefix that flags a `[markers.<name>]` as a dock
/// marker (issue #1159). A naming convention over the shared marker vocabulary,
/// the way `phaser-<id>` and `shield-arc-<id>` are conventions over the system-id
/// vocabulary.
pub const DOCK_MARKER_PREFIX: &str = "dock";

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;

use crate::entities::model_rig::ModelRigTransform as _;

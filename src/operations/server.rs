//! Bevy adapter for external operations (issue #1026).
//!
//! One component, three systems, and no decisions of its own. Whether an
//! operation may run and how far it has got are the pure sibling
//! [`super::hold`]'s to say; everything here gathers the real inputs that
//! sibling cannot see, applies the verdict it returns, and publishes the result.
//!
//! # Where the inputs come from
//!
//! * **Proximity** — the ship's own [`ShipPhysics`] against the target's
//!   `Transform`. Both are current by `SimSet::Modifiers`: `sync_ship_position`
//!   has already mirrored physics into transforms back in `SimSet::Physics`.
//! * **Capability** — the hull's authored `[operations]` table, carried on
//!   [`ShipOperations::capabilities`] since spawn.
//! * **Power** — the ship's own [`ShipPowerSystem`], read as the capability's
//!   authored group level plus the grid's exhaustion lock. A hull with no power
//!   grid at all is *not* gated on power: the constraint is absent, not failed.
//!
//! # Where a completion lands
//!
//! A completed `stabilise` pays its authored `condition_on_complete` into
//! `WorldContentRuntime::pending_condition_adjustments` — #1025's queue, not the
//! target's component. That is not incidental plumbing: #1025 put every
//! condition move through `tick_infrastructure_condition` precisely so the code
//! that crosses an operational threshold is also the code that mirrors the
//! resulting flag. An operation writing `InfrastructureCondition` directly would
//! bring a skyhook back over its threshold with nobody listening. This system is
//! ordered `.before(tick_infrastructure_condition)` so the payoff lands on the
//! same tick it was earned rather than the next one.
//!
//! # Determinism
//!
//! Ships are walked in UUID order, never archetype order, and the target lookup
//! is by UUID rather than by whichever entity a query happened to yield first —
//! the same rule [`crate::sim_digest`] and #1025 apply to their own walks.

use bevy::prelude::*;

use crate::entities::spawner::{EntityName, EntityUuid};
use crate::infrastructure::{ConditionAdjustment, InfrastructureCondition};
use crate::logging::LogFilterConfig;
use crate::messages::{
    ActiveOperationSnapshot, AdmittedCommands, CapabilityOffer, OperationsBlackboard, PowerGroupId,
    SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::operations::hold::{
    eligibility, Ineligibility, OperationConditions, OperationHold, OperationVerb,
    OperationsConfig, Settlement,
};
use crate::ship::power::ShipPowerSystem;
use crate::ship::state::ShipPhysics;
use crate::world::server::WorldContentRuntime;

/// The blackboard channel key operations are published under.
///
/// **Not a system id.** No `[[system]]` block declares it, no station owns it,
/// it registers no `ControlSource` and no `ControlSystem` message may target it
/// — an operation is something a ship *does*, not a thing aboard it that can be
/// damaged or repaired. It is carried inside a [`SystemId`] value for the same
/// reason `"helm"` and `"tactical"` are: the blackboard map and the
/// `BlackboardUpdate` wire message are typed that way. The commands that start
/// and abort an operation target the real, station-owned `captain` system
/// instead.
pub const OPERATIONS_BLACKBOARD_KEY: &str = "operations";

/// The blackboard channel key as a [`SystemId`].
pub fn operations_blackboard_key() -> SystemId {
    SystemId(OPERATIONS_BLACKBOARD_KEY.to_string())
}

/// Everything one ship knows about external operations.
///
/// Authoritative per-ship simulation state: which verbs the hull can perform,
/// the operation it is running (or the last one it ran, so a console can still
/// show how it ended), and why the last start was turned down.
///
/// One component rather than three because the three move together and are read
/// together — the blackboard publisher wants all of it, a snapshot has to
/// restore all of it, and splitting them would only give the census three
/// entries where one tells the whole story.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ShipOperations {
    /// The hull's authored capability table, as spawned.
    pub capabilities: OperationsConfig,
    /// The live operation, or the last one to settle. Retained after it settles
    /// so the console can report "completed" / "failed: out of range" rather
    /// than the panel simply emptying; the next start replaces it.
    pub active: Option<OperationHold>,
    /// Why the most recent start was refused, if it was. Written only by the
    /// start path — an operation that stalls or fails mid-hold carries its
    /// reason on the hold itself.
    pub last_refusal: Option<Ineligibility>,
    /// Ids handed out so far, so each operation on this ship has a distinct one.
    /// A per-ship counter rather than a minted world id: an operation is not an
    /// entity, and starts are already in a deterministic order.
    pub next_id: u64,
}

/// The mutable part of a ship's operations record, as a save carries it
/// (issue #1026).
///
/// The authored `capabilities` are deliberately **not** in here. They are
/// re-derived from the hull's template on the tick the ship spawns, and a save
/// whose hull's `[operations]` table has since changed is refused as
/// content-moved long before this is read — so writing them would put content
/// into a save that `content_digest` is the thing answerable for.
///
/// What is here has to come back **together**. Restore the hold without
/// `next_id` and the next operation reuses an id the console has already shown;
/// restore `next_id` without the hold and a resumed mission forgets it was
/// halfway through stabilising a skyhook.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OperationsSaveState {
    /// The live or last-settled hold, whole — its banked ticks, its spent stall
    /// budget and its current state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<OperationHold>,
    /// Why the last start was refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refusal: Option<Ineligibility>,
    /// The next id to hand out.
    #[serde(default)]
    pub next_id: u64,
}

impl ShipOperations {
    /// Project the record onto what a save carries.
    pub fn save_state(&self) -> OperationsSaveState {
        OperationsSaveState {
            active: self.active.clone(),
            last_refusal: self.last_refusal,
            next_id: self.next_id,
        }
    }

    /// Take a save's state back, leaving the spawned capability table alone.
    pub fn restore(&mut self, state: &OperationsSaveState) {
        self.active = state.active.clone();
        self.last_refusal = state.last_refusal;
        self.next_id = state.next_id;
    }
}

/// One operation start queued by a script effect, already resolved to UUIDs.
///
/// Queued rather than applied where it is authored for the reason #1025's
/// [`ConditionAdjustment`] is: the applier holds `name_to_uuid` and nothing
/// else, while starting an operation needs the ship's capability table, its
/// power grid and the target's position. Resolution happens in the applier,
/// the decision happens in [`tick_operations`].
#[derive(Clone, Debug, PartialEq)]
pub struct PendingOperationStart {
    /// The performing ship's `EntityUuid`.
    pub ship_uuid: String,
    /// The verb to perform.
    pub verb: OperationVerb,
    /// The target's `EntityUuid`.
    pub target_uuid: String,
}

/// Registers the operation systems. Added by `WorldPlugin` alongside
/// `InfrastructurePlugin`, because the queue a completion pays into and the
/// queue a script start arrives on are both `WorldContentRuntime`'s.
pub struct OperationsPlugin;

impl Plugin for OperationsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                handle_operation_commands.in_set(crate::sim_sets::SimSet::Input),
                // Ordered ahead of the condition tick so a completed operation's
                // points land on the tick they were earned. See the module doc.
                tick_operations
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .before(crate::infrastructure::tick_infrastructure_condition),
                publish_operations_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── The admitted start/abort commands ────────────────────────────────────────

/// Start and abort operations on behalf of whoever asked — a console player or
/// the ship's own AI, indistinguishably.
///
/// Runs in `SimSet::Input` and reads `AdmittedCommands` for the **captain**
/// system: ordering an external operation is a command decision, and routing it
/// through a real station-owned system id is what gives it the same admission
/// path (and the same station-tenure check) as red alert or an objective
/// priority. Admission has already decided who may speak; nothing here asks.
pub fn handle_operation_commands(
    mut ships: Query<
        (Entity, &AdmittedCommands, Option<&mut ShipOperations>),
        With<crate::server_app::Ship>,
    >,
    mut commands: Commands,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    log: Option<Res<LogFilterConfig>>,
) {
    let tick_hz = tick_hz_of(world_config.as_deref());
    for (entity, admitted, ops) in ships.iter_mut() {
        // Owned copies, so the loop below can borrow the operations record
        // mutably without still holding the query row's command list.
        let requests: Vec<OperationRequest> = admitted
            .for_target(crate::system_registry::CAPTAIN_SYSTEM_ID)
            .filter_map(|cmd| match &cmd.payload {
                SystemControlPayload::StartOperation { verb, target_uuid } => {
                    Some(OperationRequest::Start {
                        verb: *verb,
                        target_uuid: target_uuid.clone(),
                    })
                }
                SystemControlPayload::AbortOperation => Some(OperationRequest::Abort),
                _ => None,
            })
            .collect();
        if requests.is_empty() {
            continue;
        }
        let Some(mut ops) = ops else {
            // A hull that authored no `[operations]` table at all still owes the
            // console an answer when it is asked to do one. Insert the record
            // holding the refusal rather than dropping the command on the floor;
            // it lands a tick later, which no console can see.
            let refused = requests
                .iter()
                .any(|r| matches!(r, OperationRequest::Start { .. }));
            commands.entity(entity).insert(ShipOperations {
                last_refusal: refused.then_some(Ineligibility::NotCapable),
                ..Default::default()
            });
            continue;
        };
        for request in requests {
            match request {
                OperationRequest::Start { verb, target_uuid } => {
                    start_operation(&mut ops, verb, &target_uuid, tick_hz, entity, &log);
                }
                OperationRequest::Abort => {
                    let aborted = ops.active.as_mut().and_then(|hold| hold.abort()).is_some();
                    crate::pdebug!(
                        log,
                        crate::logging::LogCat::Captain,
                        entity = entity,
                        "operation abort requested: {}",
                        if aborted {
                            "stood down"
                        } else {
                            "nothing running"
                        }
                    );
                }
            }
        }
    }
}

/// One admitted operation command, lifted out of its payload.
enum OperationRequest {
    Start {
        verb: OperationVerb,
        target_uuid: String,
    },
    Abort,
}

/// Open a hold, or record why one could not be opened.
///
/// The only eligibility this checks is **capability**, because it is the only
/// one that is a fact about the ship rather than about this instant: range and
/// power are re-tested every tick by [`tick_operations`], and refusing a start
/// on them would mean a crew could not queue the operation and then fly into
/// position. A start that is out of range simply opens stalled.
fn start_operation(
    ops: &mut ShipOperations,
    verb: OperationVerb,
    target_uuid: &str,
    tick_hz: f32,
    entity: Entity,
    log: &Option<Res<LogFilterConfig>>,
) {
    let Some(capability) = ops.capabilities.capability(verb).cloned() else {
        ops.last_refusal = Some(Ineligibility::NotCapable);
        crate::pwarn!(
            log,
            crate::logging::LogCat::Captain,
            entity = entity,
            "operation {} refused: this hull authored no capability for it",
            verb.as_str()
        );
        return;
    };
    let id = ops.next_id;
    ops.next_id = ops.next_id.saturating_add(1);
    ops.last_refusal = None;
    ops.active = Some(OperationHold::start(id, target_uuid, &capability, tick_hz));
    crate::pdebug!(
        log,
        crate::logging::LogCat::Captain,
        entity = entity,
        "operation {} #{id} started against {target_uuid}",
        verb.as_str()
    );
}

/// The world's authored logical tick rate, or the schedule clock's default for
/// an app that never loaded a world.
fn tick_hz_of(world_config: Option<&crate::world::config::WorldConfig>) -> f32 {
    world_config
        .map(|wc| wc.global.sim_tick_hz)
        .unwrap_or(crate::world::script::schedule::SchedClock::ZERO.tick_hz)
}

// ── The tick ─────────────────────────────────────────────────────────────────

/// Advance every live operation by one logical tick.
///
/// Per ship, in UUID order: drain any script-queued start, gather this tick's
/// real conditions, hand them to the pure eligibility test, apply what comes
/// back, and pay a completion into #1025's condition queue.
pub fn tick_operations(
    runtime: Option<ResMut<WorldContentRuntime>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<(
        Entity,
        &EntityUuid,
        &ShipPhysics,
        Option<&ShipPowerSystem>,
        &mut ShipOperations,
    )>,
    targets: Query<(&EntityUuid, &Transform, Has<InfrastructureCondition>)>,
    log: Option<Res<LogFilterConfig>>,
) {
    let Some(mut runtime) = runtime else {
        return;
    };
    // A read, so a world with no operations and no queued starts never marks
    // `WorldContentRuntime` changed.
    if ships.is_empty() && runtime.pending_operation_starts.is_empty() {
        return;
    }
    let tick_hz = tick_hz_of(world_config.as_deref());
    let queued = std::mem::take(&mut runtime.pending_operation_starts);

    let mut rows: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid, _, _, _)| (uuid.0.clone(), entity))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (uuid, entity) in rows {
        let Ok((_, _, physics, power, mut ops)) = ships.get_mut(entity) else {
            continue;
        };

        // Script-queued starts first, so an operation authored to begin this
        // tick is advanced this tick rather than idling for one.
        for start in queued.iter().filter(|s| s.ship_uuid == uuid) {
            start_operation(
                &mut ops,
                start.verb,
                &start.target_uuid,
                tick_hz,
                entity,
                &log,
            );
        }

        // The verb and target are read off the hold BEFORE it is borrowed
        // mutably, because resolving the capability reads the same record.
        let Some((verb, target_uuid)) = ops
            .active
            .as_ref()
            .filter(|hold| !hold.is_settled())
            .map(|hold| (hold.verb(), hold.target_uuid().to_string()))
        else {
            continue;
        };
        let capability = ops.capabilities.capability(verb).cloned();
        let Some(hold) = ops.active.as_mut() else {
            continue;
        };
        let target = targets.iter().find(|(uuid, _, _)| uuid.0 == target_uuid);
        let ship_pos = Vec3::new(physics.x, physics.y, physics.z);
        let conditions = OperationConditions {
            target_present: target.is_some(),
            // `stabilise` means something only on an entity that carries a
            // condition track. A pristine asteroid is present, in range, and
            // still not a thing you can stabilise.
            target_applicable: target.map(|(_, _, has)| has).unwrap_or(false),
            distance: target
                .map(|(_, tf, _)| ship_pos.distance(tf.translation))
                .unwrap_or(f32::INFINITY),
            // No power grid means no power *constraint* — the ceiling is absent,
            // not zero. Every hull that authors `[operations]` in practice
            // authors `[power]` too; this keeps a bare fixture honest rather
            // than failing it for a component it never had.
            power_level: match (power, capability.as_ref()) {
                (Some(power), Some(capability)) => power
                    .0
                    .level_for(&PowerGroupId(capability.power_group.clone())),
                _ => u8::MAX,
            },
            power_locked: power.map(|power| power.0.locked()).unwrap_or(false),
        };

        let before = hold.state();
        let settlement = hold.advance(eligibility(capability.as_ref(), &conditions));
        let after = hold.state();
        if before != after {
            crate::pdebug!(
                log,
                crate::logging::LogCat::Captain,
                entity = entity,
                "operation {} #{}: {} -> {}",
                hold.verb().as_str(),
                hold.id(),
                before.as_str(),
                after.as_str()
            );
        }

        // The payoff. Queued for `tick_infrastructure_condition` rather than
        // written onto the target, so the crossing it may cause is detected and
        // mirrored by the one system that owns operational-flag edges (#1025).
        if settlement == Some(Settlement::Completed) {
            let points = hold.condition_on_complete();
            let target_uuid = hold.target_uuid().to_string();
            if points > 0.0 {
                runtime
                    .pending_condition_adjustments
                    .push(ConditionAdjustment {
                        uuid: target_uuid,
                        delta: points,
                    });
            }
        }
    }
}

// ── The wire ─────────────────────────────────────────────────────────────────

/// Publish each operating ship's blackboard.
///
/// Only ships that carry [`ShipOperations`] publish one, so a world whose hulls
/// author no `[operations]` puts exactly the payload on the wire it did before
/// this existed.
pub fn publish_operations_blackboard(
    mut ships: Query<(
        &ShipOperations,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
    named: Query<(&EntityUuid, &EntityName)>,
) {
    for (ops, mut blackboards) in ships.iter_mut() {
        let active = ops.active.as_ref().map(|hold| {
            let state = hold.state();
            ActiveOperationSnapshot {
                id: hold.id(),
                verb: hold.verb().as_str().to_string(),
                verb_label: verb_label(hold.verb()).to_string(),
                target_uuid: hold.target_uuid().to_string(),
                target_name: named
                    .iter()
                    .find(|(uuid, _)| uuid.0 == hold.target_uuid())
                    .map(|(_, name)| name.0.clone()),
                progress: hold.progress(),
                state: state.as_str().to_string(),
                reason: state.reason().map(|r| r.string_id().to_string()),
            }
        });
        let blackboard = SystemBlackboard::Operations(OperationsBlackboard {
            capabilities: ops
                .capabilities
                .capabilities
                .iter()
                .map(|capability| CapabilityOffer {
                    verb: capability.verb.as_str().to_string(),
                    label: verb_label(capability.verb).to_string(),
                })
                .collect(),
            active,
            refusal: ops
                .last_refusal
                .map(|refusal| refusal.string_id().to_string()),
        });
        let key = operations_blackboard_key();
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key, blackboard);
        }
    }
}

/// The `strings.csv` id for a verb's crew-facing name. No English crosses the
/// wire (AGENTS.md rule 11); the client resolves this through `t()`.
///
/// A `match` rather than a formatted `"operation.verb.{verb}"`, because a
/// composed id is invisible to `scripts/check-strings.mjs` and would let a new
/// verb ship with no row behind it.
pub fn verb_label(verb: OperationVerb) -> &'static str {
    match verb {
        OperationVerb::Stabilise => "operation.verb.stabilise",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::hold::{CapabilityConfig, HoldState};

    const TARGET: &str = "depot-1";
    const SHIP: &str = "ship-1";

    fn stabilise_capability() -> CapabilityConfig {
        CapabilityConfig {
            verb: OperationVerb::Stabilise,
            range: 400.0,
            duration_secs: 2,
            power_group: "helm".to_string(),
            min_power_level: 2,
            condition_on_complete: 25.0,
            stall_limit_secs: None,
        }
    }

    fn capable() -> ShipOperations {
        ShipOperations {
            capabilities: OperationsConfig {
                capabilities: vec![stabilise_capability()],
            },
            ..Default::default()
        }
    }

    /// A bare app carrying one ship and one stabilisable target, ticked by hand.
    fn app_with(ops: ShipOperations, ship_x: f32) -> (App, Entity, Entity) {
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(Update, tick_operations);
        let ship = app
            .world_mut()
            .spawn((
                EntityUuid(SHIP.to_string()),
                ShipPhysics {
                    x: ship_x,
                    ..Default::default()
                },
                Transform::from_xyz(ship_x, 0.0, 0.0),
                ops,
            ))
            .id();
        let target = app
            .world_mut()
            .spawn((
                EntityUuid(TARGET.to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
                    &crate::infrastructure::InfrastructureConfig::default(),
                )),
            ))
            .id();
        (app, ship, target)
    }

    fn ops_of(app: &App, ship: Entity) -> ShipOperations {
        app.world()
            .get::<ShipOperations>(ship)
            .expect("the ship carries its operations record")
            .clone()
    }

    fn queued_adjustments(app: &App) -> Vec<ConditionAdjustment> {
        app.world()
            .resource::<WorldContentRuntime>()
            .pending_condition_adjustments
            .clone()
    }

    fn start_via_script(app: &mut App, verb: OperationVerb) {
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_operation_starts
            .push(PendingOperationStart {
                ship_uuid: SHIP.to_string(),
                verb,
                target_uuid: TARGET.to_string(),
            });
    }

    // ── AC4: a script effect starts an operation ──

    #[test]
    fn a_script_queued_start_opens_a_hold_on_the_tick_it_is_drained() {
        let (mut app, ship, _) = app_with(capable(), 100.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        app.update();

        let ops = ops_of(&app, ship);
        let hold = ops.active.expect("the queued start opened a hold");
        assert_eq!(hold.verb(), OperationVerb::Stabilise);
        assert_eq!(hold.target_uuid(), TARGET);
        assert_eq!(
            hold.elapsed_ticks(),
            1,
            "and it is ADVANCED on the same tick it is drained — an operation authored to begin \
             now must not idle for a tick first"
        );
        assert!(
            app.world()
                .resource::<WorldContentRuntime>()
                .pending_operation_starts
                .is_empty(),
            "…and the queue is drained regardless, so a stale start cannot accumulate"
        );
    }

    #[test]
    fn a_script_start_naming_a_ship_that_is_not_there_lands_on_nobody() {
        let (mut app, ship, _) = app_with(capable(), 100.0);
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_operation_starts
            .push(PendingOperationStart {
                ship_uuid: "no-such-ship".to_string(),
                verb: OperationVerb::Stabilise,
                target_uuid: TARGET.to_string(),
            });
        app.update();
        assert!(
            ops_of(&app, ship).active.is_none(),
            "a start naming a ship that is not in this world must not open a hold on whichever \
             ship happens to be first"
        );
    }

    // ── AC4: a player starts and aborts through admitted commands ──

    /// A bare app carrying one *ship* (with the `Ship` marker admission
    /// requires) whose admitted commands the test writes by hand — the same
    /// place `admit_system_commands` would have put them, so this exercises the
    /// handler on exactly the input the real admission path produces.
    fn app_with_commands(ops: Option<ShipOperations>) -> (App, Entity) {
        let mut app = App::new();
        app.add_systems(Update, handle_operation_commands);
        let mut ship = app.world_mut().spawn((
            crate::server_app::Ship,
            EntityUuid(SHIP.to_string()),
            AdmittedCommands::default(),
        ));
        if let Some(ops) = ops {
            ship.insert(ops);
        }
        let ship = ship.id();
        (app, ship)
    }

    fn admit(app: &mut App, ship: Entity, payload: SystemControlPayload) {
        app.world_mut()
            .get_mut::<AdmittedCommands>(ship)
            .expect("the ship carries an admitted-command list")
            .0
            .push(crate::messages::AdmittedCommand {
                target: SystemId(crate::system_registry::CAPTAIN_SYSTEM_ID.to_string()),
                payload,
                response_token: None,
            });
    }

    #[test]
    fn an_admitted_start_opens_a_hold_and_an_admitted_abort_stands_it_down() {
        let (mut app, ship) = app_with_commands(Some(capable()));
        admit(
            &mut app,
            ship,
            SystemControlPayload::StartOperation {
                verb: OperationVerb::Stabilise,
                target_uuid: TARGET.to_string(),
            },
        );
        app.update();
        let hold = ops_of(&app, ship).active.expect("the start opened a hold");
        assert_eq!(hold.target_uuid(), TARGET);
        assert!(
            !hold.is_settled(),
            "a start from a console opens a live hold, whatever the ship's position — range is \
             re-tested every tick, so refusing it here would stop a crew queueing the operation \
             and then flying to it"
        );

        app.world_mut()
            .get_mut::<AdmittedCommands>(ship)
            .unwrap()
            .0
            .clear();
        admit(&mut app, ship, SystemControlPayload::AbortOperation);
        app.update();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Aborted),
            "…and the same console can call it off through the same admission path"
        );
    }

    #[test]
    fn an_admitted_start_for_a_verb_the_hull_lacks_records_a_displayable_refusal() {
        // A hull that authored an `[operations]` table with nothing in it: the
        // narrowest reading of "a ship without the capability".
        let (mut app, ship) = app_with_commands(Some(ShipOperations::default()));
        admit(
            &mut app,
            ship,
            SystemControlPayload::StartOperation {
                verb: OperationVerb::Stabilise,
                target_uuid: TARGET.to_string(),
            },
        );
        app.update();
        let ops = ops_of(&app, ship);
        assert!(ops.active.is_none());
        assert_eq!(ops.last_refusal, Some(Ineligibility::NotCapable));
    }

    #[test]
    fn a_ship_that_authored_no_operations_at_all_still_answers_the_console() {
        let (mut app, ship) = app_with_commands(None);
        admit(
            &mut app,
            ship,
            SystemControlPayload::StartOperation {
                verb: OperationVerb::Stabilise,
                target_uuid: TARGET.to_string(),
            },
        );
        app.update();
        assert_eq!(
            ops_of(&app, ship).last_refusal,
            Some(Ineligibility::NotCapable),
            "a command dropped on the floor here would leave the crew tapping a button that \
             does nothing and never says why"
        );
    }

    #[test]
    fn a_command_for_another_system_is_not_an_operation_command() {
        let (mut app, ship) = app_with_commands(Some(capable()));
        app.world_mut()
            .get_mut::<AdmittedCommands>(ship)
            .unwrap()
            .0
            .push(crate::messages::AdmittedCommand {
                target: SystemId(crate::system_registry::RED_ALERT_SYSTEM_ID.to_string()),
                payload: SystemControlPayload::SetRedAlert { active: true },
                response_token: None,
            });
        app.update();
        let ops = ops_of(&app, ship);
        assert!(ops.active.is_none() && ops.last_refusal.is_none());
        assert_eq!(
            ops.next_id, 0,
            "the handler reads only the captain system's operation payloads — it must not react \
             to every command the ship happened to be given this tick"
        );
    }

    // ── AC1/AC2: the adapter applies the pure verdict, it does not re-decide ──

    #[test]
    fn an_incapable_hull_refuses_the_start_by_name_and_opens_nothing() {
        let (mut app, ship, _) = app_with(ShipOperations::default(), 100.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        app.update();

        let ops = ops_of(&app, ship);
        assert!(ops.active.is_none(), "no hold is opened");
        assert_eq!(
            ops.last_refusal,
            Some(Ineligibility::NotCapable),
            "…and the refusal is recorded with a reason the console can display, rather than the \
             command vanishing"
        );
    }

    #[test]
    fn a_ship_parked_outside_the_authored_range_stalls_instead_of_progressing() {
        let (mut app, ship, _) = app_with(capable(), 5_000.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        app.update();

        let hold = ops_of(&app, ship).active.expect("the hold opened");
        assert_eq!(
            hold.state(),
            HoldState::Stalled(Ineligibility::OutOfRange),
            "a start from out of range OPENS — the crew queue the operation and then fly to it — \
             but banks nothing until they arrive"
        );
        assert_eq!(hold.elapsed_ticks(), 0);
    }

    #[test]
    fn a_target_with_no_condition_track_is_refused_as_inapplicable_rather_than_missing() {
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(Update, tick_operations);
        let ship = app
            .world_mut()
            .spawn((
                EntityUuid(SHIP.to_string()),
                ShipPhysics::default(),
                capable(),
            ))
            .id();
        // Present, in range, and simply not a thing you can stabilise.
        app.world_mut()
            .spawn((EntityUuid(TARGET.to_string()), Transform::default()));
        start_via_script(&mut app, OperationVerb::Stabilise);
        app.update();

        let hold = ops_of(&app, ship).active.expect("the hold opened");
        assert_eq!(
            hold.state(),
            HoldState::Failed(Ineligibility::TargetNotApplicable),
            "'target gone' would send helm looking for a rock that is right there — the reason \
             has to say what is actually wrong"
        );
    }

    #[test]
    fn a_hull_with_no_power_grid_is_not_gated_on_power() {
        // The fixture ship carries no `ShipPowerSystem`. Absence of a grid is
        // absence of the constraint, not a failed one.
        let (mut app, ship, _) = app_with(capable(), 100.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        app.update();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Holding),
            "a fixture with no power component must not be refused for a component it never had"
        );
    }

    // ── AC7: the completion feeds #1025's condition queue ──

    #[test]
    fn a_completed_operation_queues_its_points_onto_the_infrastructure_track() {
        let (mut app, ship, _) = app_with(capable(), 100.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        // Two authored seconds at 60 Hz.
        for _ in 0..120 {
            app.update();
        }
        let hold = ops_of(&app, ship).active.expect("the hold is retained");
        assert_eq!(hold.state(), HoldState::Completed);
        assert_eq!(
            queued_adjustments(&app),
            vec![ConditionAdjustment {
                uuid: TARGET.to_string(),
                delta: 25.0,
            }],
            "the payoff is QUEUED for tick_infrastructure_condition, not written onto the \
             target — an operation writing the component directly would cross an operational \
             threshold with nobody listening (#1025)"
        );
    }

    #[test]
    fn a_completed_operation_pays_exactly_once_however_long_the_ship_sits_there() {
        let (mut app, _, _) = app_with(capable(), 100.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        for _ in 0..120 {
            app.update();
        }
        // Drain what the completion queued, as the condition tick would.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_condition_adjustments
            .clear();
        for _ in 0..120 {
            app.update();
        }
        assert!(
            queued_adjustments(&app).is_empty(),
            "a settled hold must not go on paying: the adapter pays off the ONE settlement the \
             pure hold reports, and it reports it once"
        );
    }

    #[test]
    fn an_operation_authored_to_pay_nothing_queues_nothing() {
        let mut ops = capable();
        ops.capabilities.capabilities[0].condition_on_complete = 0.0;
        let (mut app, ship, _) = app_with(ops, 100.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        for _ in 0..120 {
            app.update();
        }
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Completed),
            "it still completes"
        );
        assert!(
            queued_adjustments(&app).is_empty(),
            "…but an operation whose payoff is entirely scripted must not queue a zero \
             adjustment for the condition tick to walk"
        );
    }

    // ── Interruption, through the real inputs ──

    #[test]
    fn flying_out_of_range_mid_hold_stalls_it_and_flying_back_resumes_it() {
        let (mut app, ship, _) = app_with(capable(), 100.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        for _ in 0..60 {
            app.update();
        }
        let banked = ops_of(&app, ship).active.unwrap().elapsed_ticks();
        assert_eq!(banked, 60, "precondition: one second of eligible hold");

        app.world_mut().get_mut::<ShipPhysics>(ship).unwrap().x = 5_000.0;
        for _ in 0..60 {
            app.update();
        }
        let stalled = ops_of(&app, ship).active.unwrap();
        assert_eq!(
            stalled.state(),
            HoldState::Stalled(Ineligibility::OutOfRange),
            "the interruption arrives through the ship's REAL position, not through a flag \
             something else set"
        );
        assert_eq!(
            stalled.elapsed_ticks(),
            banked,
            "and banks nothing while away"
        );

        app.world_mut().get_mut::<ShipPhysics>(ship).unwrap().x = 100.0;
        for _ in 0..60 {
            app.update();
        }
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Completed),
            "flying back resumes the hold from where it stood — 60 more eligible ticks is all \
             the two-second operation still needed"
        );
    }

    #[test]
    fn a_target_that_despawns_mid_hold_fails_the_operation() {
        let (mut app, ship, target) = app_with(capable(), 100.0);
        start_via_script(&mut app, OperationVerb::Stabilise);
        for _ in 0..30 {
            app.update();
        }
        app.world_mut().entity_mut(target).despawn();
        app.update();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Failed(Ineligibility::TargetGone)),
            "a skyhook that falls mid-operation ends the operation — waiting cannot bring it back"
        );
        assert!(
            queued_adjustments(&app).is_empty(),
            "…and a failed operation pays nothing"
        );
    }

    // ── Determinism ──

    #[test]
    fn ships_are_walked_in_uuid_order_whatever_order_they_spawned_in() {
        fn run(order: [&str; 3]) -> Vec<ConditionAdjustment> {
            let mut app = App::new();
            app.init_resource::<WorldContentRuntime>();
            app.add_systems(Update, tick_operations);
            app.world_mut().spawn((
                EntityUuid(TARGET.to_string()),
                Transform::default(),
                InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
                    &crate::infrastructure::InfrastructureConfig::default(),
                )),
            ));
            for uuid in order {
                app.world_mut().spawn((
                    EntityUuid(uuid.to_string()),
                    ShipPhysics::default(),
                    capable(),
                ));
                app.world_mut()
                    .resource_mut::<WorldContentRuntime>()
                    .pending_operation_starts
                    .push(PendingOperationStart {
                        ship_uuid: uuid.to_string(),
                        verb: OperationVerb::Stabilise,
                        target_uuid: TARGET.to_string(),
                    });
            }
            for _ in 0..120 {
                app.update();
            }
            app.world()
                .resource::<WorldContentRuntime>()
                .pending_condition_adjustments
                .clone()
        }
        assert_eq!(
            run(["ship-a", "ship-b", "ship-c"]),
            run(["ship-c", "ship-b", "ship-a"]),
            "three ships completing the same operation on the same tick must queue their \
             adjustments in the same order on every host — the walk is keyed on the UUID, not on \
             archetype order"
        );
    }

    // ── The wire projection ──

    #[test]
    fn the_published_blackboard_carries_the_hold_the_console_has_to_render() {
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(
            Update,
            (tick_operations, publish_operations_blackboard).chain(),
        );
        let ship = app
            .world_mut()
            .spawn((
                EntityUuid(SHIP.to_string()),
                ShipPhysics::default(),
                capable(),
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(TARGET.to_string()),
            EntityName("world.entity.skyhook.name".to_string()),
            Transform::default(),
            InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
                &crate::infrastructure::InfrastructureConfig::default(),
            )),
        ));
        start_via_script(&mut app, OperationVerb::Stabilise);
        for _ in 0..60 {
            app.update();
        }

        let blackboards = app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(ship)
            .expect("the ship carries a blackboard map");
        let Some(SystemBlackboard::Operations(bb)) =
            blackboards.0.get(&operations_blackboard_key())
        else {
            panic!("operations publish under their own channel key, not onto an existing system");
        };
        assert_eq!(
            bb.capabilities,
            vec![CapabilityOffer {
                verb: "stabilise".to_string(),
                label: "operation.verb.stabilise".to_string(),
            }],
            "the hull's verbs reach the console, each with a strings.csv id rather than English"
        );
        let active = bb.active.as_ref().expect("the live hold is published");
        assert_eq!(active.state, "holding");
        assert_eq!(
            active.reason, None,
            "a holding operation has nothing wrong with it"
        );
        assert!(
            (active.progress - 0.5).abs() < 1e-6,
            "one second into a two-second hold reads as half done: {}",
            active.progress
        );
        assert_eq!(
            active.target_name.as_deref(),
            Some("world.entity.skyhook.name"),
            "the target's display name travels as its string id, so the console names what the \
             ship is working on"
        );
    }

    #[test]
    fn a_refusal_and_a_stall_both_reach_the_blackboard_with_their_reasons() {
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(
            Update,
            (tick_operations, publish_operations_blackboard).chain(),
        );
        let ship = app
            .world_mut()
            .spawn((
                EntityUuid(SHIP.to_string()),
                ShipPhysics {
                    x: 5_000.0,
                    ..Default::default()
                },
                capable(),
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(TARGET.to_string()),
            Transform::default(),
            InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
                &crate::infrastructure::InfrastructureConfig::default(),
            )),
        ));
        start_via_script(&mut app, OperationVerb::Stabilise);
        app.update();

        let read = |app: &App| -> OperationsBlackboard {
            let blackboards = app
                .world()
                .get::<crate::server_app::ShipSystemBlackboards>(ship)
                .unwrap();
            match blackboards.0.get(&operations_blackboard_key()) {
                Some(SystemBlackboard::Operations(bb)) => bb.clone(),
                _ => panic!("no operations blackboard published"),
            }
        };
        let stalled = read(&app);
        let active = stalled.active.expect("the stalled hold is published");
        assert_eq!(active.state, "stalled");
        assert_eq!(
            active.reason.as_deref(),
            Some("operation.refused.out_of_range"),
            "the console has to be able to tell the crew WHY it is not advancing, and it reads a \
             strings.csv id rather than English"
        );

        // Now ask an incapable hull.
        app.world_mut()
            .get_mut::<ShipOperations>(ship)
            .unwrap()
            .capabilities = OperationsConfig::default();
        start_via_script(&mut app, OperationVerb::Stabilise);
        app.update();
        assert_eq!(
            read(&app).refusal.as_deref(),
            Some("operation.refused.not_capable"),
            "a refused start is reported separately from a stalled hold — they are different \
             things and the crew act on them differently"
        );
    }

    #[test]
    fn a_ship_that_never_authored_operations_publishes_no_operations_blackboard() {
        let mut app = App::new();
        app.add_systems(Update, publish_operations_blackboard);
        let ship = app
            .world_mut()
            .spawn(crate::server_app::ShipSystemBlackboards::default())
            .id();
        app.update();
        assert!(
            app.world()
                .get::<crate::server_app::ShipSystemBlackboards>(ship)
                .unwrap()
                .0
                .is_empty(),
            "every world in the repository is in this arm, and must put exactly the payload on \
             the wire it did before operations existed"
        );
    }

    #[test]
    fn every_verb_has_a_display_label_behind_it() {
        for verb in OperationVerb::ALL {
            let label = verb_label(*verb);
            assert!(
                label.starts_with("operation.verb."),
                "{} must have a strings.csv id, not English: {label}",
                verb.as_str()
            );
        }
    }
}

//! Bevy adapter for external operations (issue #1026).
//!
//! One component, three systems, and no decisions of its own. Whether an
//! operation may run and how far it has got are the pure sibling
//! [`super::hold`]'s to say; everything here gathers the real inputs that
//! sibling cannot see, applies the verdict it returns, and publishes the result.
//!
//! # Where the inputs come from
//!
//! * **Proximity** — both ends read off `Transform`, the way the comms range
//!   check does. Current by `SimSet::Modifiers` even for a ship, because
//!   `sync_ship_position` has already mirrored `ShipPhysics` into the transform
//!   back in `SimSet::Physics`. Reading the transform rather than `ShipPhysics`
//!   is what lets something that is not a ship — a platform, a tender that never
//!   moves — perform an operation, and the target end has never had a choice.
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
use crate::infrastructure::{CapacityAdjustment, ConditionAdjustment, InfrastructureCondition};
use crate::logging::LogFilterConfig;
use crate::messages::{
    ActiveOperationSnapshot, AdmittedCommands, CapabilityOffer, OperationsBlackboard, PowerGroupId,
    SystemBlackboard, SystemControlPayload, SystemId, TeamSlot,
};
use crate::operations::hold::{
    verdict, CapacityReading, Ineligibility, OperationConditions, OperationHold, OperationVerb,
    OperationsConfig, RegionEffectName, Settlement, TransferDirection,
};
use crate::regions::effects::RegionEffectKind;
use crate::ship::power::ShipPowerSystem;
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

impl ShipOperations {
    /// How many of this ship's repair teams the running operation is holding
    /// (issue #1027) — the **capacity-as-cost** reading the repair console
    /// takes.
    ///
    /// Derived from the live hold rather than stored, which is what makes
    /// "released on completion or abort" true by construction: a settled hold
    /// commits nothing, so there is no release step to forget and nothing extra
    /// for a save to carry. The teams themselves never move — no slot is
    /// dispatched, no team travels anywhere, and the hull's own roster is
    /// untouched. They are simply spoken for.
    pub fn committed_repair_teams(&self) -> u8 {
        self.active
            .as_ref()
            .filter(|hold| !hold.is_settled())
            .and_then(|hold| {
                self.capabilities
                    .capability(hold.verb())
                    .map(|capability| capability.repair_teams)
            })
            .unwrap_or(0)
    }
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
                // After the tick that decides whether a tow is holding, so a
                // load is moved by the tow that is actually running this tick
                // rather than by one that stalled at the top of it.
                move_towed_targets
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(tick_operations),
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
/// real conditions, hand them to the pure verdict, apply what comes back, and
/// pay whatever it earned into #1025's queues.
///
/// # Where the interrupt signals come from (issue #1027)
///
/// * **Attack** — `RecentCombatActivity` folded through
///   `objectives::last_landed_hit_secs` against the world's authored
///   `attacked_memory_secs`, which is the same reading the doctrine gates and
///   the viewscreen take. Own weapons fire is deliberately not in it: shooting
///   back is not being interrupted.
/// * **Region** — `RegionMembership`, which `update_region_membership`
///   recomputes in `SimSet::Physics`, one set earlier than this. Membership is
///   only tracked for entities carrying the `Ship` marker, which every operator
///   is.
/// * **Power** — unchanged from #1026: it was already an eligibility condition,
///   and giving it a second spelling as an interrupt rule would let one
///   capability author two answers to the same question.
#[allow(clippy::too_many_arguments)]
pub fn tick_operations(
    runtime: Option<ResMut<WorldContentRuntime>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    time: Option<Res<Time>>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    mut ships: Query<(
        Entity,
        &EntityUuid,
        &Transform,
        Option<&ShipPowerSystem>,
        Option<&crate::console::repair::server::ShipRepairTeams>,
        Option<&crate::ship::combat_activity::RecentCombatActivity>,
        &mut ShipOperations,
    )>,
    targets: Query<(&EntityUuid, &Transform, Option<&InfrastructureCondition>)>,
    region_effects: Query<&crate::entities::spawner::RegionEffectsSection>,
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
    let now_secs = time.map(|t| t.elapsed_secs()).unwrap_or(0.0);
    let attacked_memory_secs = world_config
        .as_deref()
        .map(|wc| wc.global.attacked_memory_secs)
        .unwrap_or_else(|| crate::entity_config::GlobalConfig::default().attacked_memory_secs);
    // Draining through `DerefMut` would mark `WorldContentRuntime` changed on
    // every quiet tick, and every world in the repository carries that resource.
    // The emptiness check above is a `Deref` read, so it costs nothing.
    let queued = if runtime.pending_operation_starts.is_empty() {
        Vec::new()
    } else {
        std::mem::take(&mut runtime.pending_operation_starts)
    };

    let mut rows: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (uuid, entity) in rows {
        let Ok((_, _, transform, power, teams, combat, mut ops)) = ships.get_mut(entity) else {
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
        // The operator's own capacity reading is taken before the hold is
        // borrowed mutably, because a transfer's other end may be this very
        // ship.
        let operator_capacity = capability
            .as_ref()
            .and_then(|c| c.transfer.as_ref())
            .and_then(|transfer| {
                targets
                    .iter()
                    .find(|(id, _, _)| id.0 == uuid)
                    .and_then(|(_, _, condition)| capacity_of(condition, &transfer.capacity))
            });
        let Some(hold) = ops.active.as_mut() else {
            continue;
        };
        let target = targets.iter().find(|(uuid, _, _)| uuid.0 == target_uuid);
        let ship_pos = transform.translation;
        let conditions = OperationConditions {
            target_present: target.is_some(),
            target_has_condition_track: target
                .map(|(_, _, condition)| condition.is_some())
                .unwrap_or(false),
            target_capacity: capability
                .as_ref()
                .and_then(|c| c.transfer.as_ref())
                .and_then(|transfer| {
                    target.and_then(|(_, _, condition)| capacity_of(condition, &transfer.capacity))
                }),
            operator_capacity,
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
            // Idle slots, counted the way the repair AI counts them. A hull with
            // no repair roster at all reads as unconstrained, on exactly the
            // reading `power_level` takes for a hull with no grid: the ceiling
            // is absent, not zero.
            repair_teams_available: teams
                .map(|teams| {
                    teams
                        .0
                        .slots()
                        .iter()
                        .filter(|slot| matches!(slot, TeamSlot::Idle))
                        .count()
                        .min(usize::from(u8::MAX)) as u8
                })
                .unwrap_or(u8::MAX),
            // Landed hits only — the same fold the doctrine gates take. Firing
            // your own guns is not being attacked.
            under_attack: combat
                .map(|activity| {
                    crate::objectives::attacked_recently(
                        crate::objectives::last_landed_hit_secs(
                            activity.last_damage_taken,
                            activity.last_hostile_fire_taken,
                        ),
                        now_secs,
                        attacked_memory_secs,
                    )
                })
                .unwrap_or(false),
            region_effects: operator_region_effects(membership.as_deref(), &region_effects, entity),
        };

        let before = hold.state();
        let tick_verdict = verdict(capability.as_ref(), &conditions);
        let rate = tick_verdict.rate();
        let settlement = hold.advance(tick_verdict);
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

        // ── The payoffs ──
        //
        // Every one of them is QUEUED for `tick_infrastructure_condition`
        // rather than written onto the target, so the crossing a condition move
        // may cause is detected and mirrored by the one system that owns
        // operational-flag edges, and a capacity move re-publishes the counter
        // a scenario predicate reads (#1025).

        // `field_repair`'s per-tick slice, scaled by the rate this tick
        // actually banked at. Paid for every tick the hold advanced, including
        // the one it completed on — that tick was held like any other.
        if let Some(rate) = rate {
            let points = hold.condition_payout(rate);
            if points > 0.0 {
                runtime
                    .pending_condition_adjustments
                    .push(ConditionAdjustment {
                        uuid: hold.target_uuid().to_string(),
                        delta: points,
                    });
            }
        }

        if settlement == Some(Settlement::Completed) {
            // `stabilise`'s lump, paid once off the single settlement the pure
            // hold reports.
            let points = hold.condition_on_complete();
            if points > 0.0 {
                runtime
                    .pending_condition_adjustments
                    .push(ConditionAdjustment {
                        uuid: hold.target_uuid().to_string(),
                        delta: points,
                    });
            }
            // `transfer`'s load. Both ends move on the same tick and in a fixed
            // order — source first — so two hosts queue the pair identically.
            // Eligibility has already proved both ends can take part; the
            // clamp inside `adjust_capacity` is the backstop, not the check.
            if let Some(transfer) = capability.as_ref().and_then(|c| c.transfer.as_ref()) {
                let (source, destination) = match transfer.direction {
                    TransferDirection::Deliver => (uuid.clone(), hold.target_uuid().to_string()),
                    TransferDirection::Collect => (hold.target_uuid().to_string(), uuid.clone()),
                };
                runtime
                    .pending_capacity_adjustments
                    .push(CapacityAdjustment {
                        uuid: source,
                        capacity: transfer.capacity.clone(),
                        delta: -transfer.amount,
                    });
                runtime
                    .pending_capacity_adjustments
                    .push(CapacityAdjustment {
                        uuid: destination,
                        capacity: transfer.capacity.clone(),
                        delta: transfer.amount,
                    });
            }
        }
    }
}

/// One end of a transfer, read off an entity's condition track.
fn capacity_of(
    condition: Option<&InfrastructureCondition>,
    capacity: &str,
) -> Option<CapacityReading> {
    condition
        .and_then(|condition| condition.0.capacity_reading(capacity))
        .map(|reading| CapacityReading {
            level: reading.level,
            headroom: reading.headroom(),
        })
}

/// Which authored region effects the operator is standing in, deduplicated and
/// in a fixed order.
///
/// Sorted by declaration order rather than by whichever region entity the
/// membership set happened to yield first: two hosts that spawned the same
/// bands in different orders must hand the pure module the same list, because
/// the strictest-rule-wins tie-break reads it.
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
        .flat_map(|effects| effects.0.iter().map(region_effect_name))
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

/// The authorable name of a live region effect.
///
/// Total by construction — a new `RegionEffectKind` variant will not compile
/// until it has a name an interrupt rule can be written against, which is the
/// point. A hazard band nobody can author a rule for is a hazard operations
/// cannot be told about.
pub fn region_effect_name(kind: &RegionEffectKind) -> RegionEffectName {
    match kind {
        RegionEffectKind::DamageZone { .. } => RegionEffectName::DamageZone,
        RegionEffectKind::SlowZone { .. } => RegionEffectName::SlowZone,
        RegionEffectKind::BlocksImpulse => RegionEffectName::BlocksImpulse,
        RegionEffectKind::RadarDampening { .. } => RegionEffectName::RadarDampening,
        RegionEffectKind::CommsJam => RegionEffectName::CommsJam,
        RegionEffectKind::SensorBlind => RegionEffectName::SensorBlind,
        RegionEffectKind::NebulaFog { .. } => RegionEffectName::NebulaFog,
    }
}

// ── The tow ──────────────────────────────────────────────────────────────────

/// Hold every towed target on its operator's tow rig (issue #1027).
///
/// # Why this writes the target's position directly
///
/// Nothing in this codebase attaches one entity to another: there is no dock,
/// no parent, no carried marker. The only sanctioned precedent for overriding a
/// ship's authoritative position is the writer-policy table on
/// [`crate::ship::state::ShipPhysics`] — collision de-overlap, blaster recoil,
/// the low-LOD substitute — and a tow is exactly that shape: a **correction
/// layered on top of** the helm integration rather than a second integrator.
/// So this is a fifth row in that table, and the count in
/// `tests/headless_runner.rs` is bumped to match.
///
/// It writes `ShipPhysics` where the target has one and lets
/// `sync_ship_position` project it, and writes the `Transform` too so the two
/// agree within the tick rather than a tick apart. A target that is *not* a
/// ship — a derelict with no `[behaviour]` block — has no `ShipPhysics` and
/// nothing else that ever moves it, so the transform write is the whole of it.
///
/// # What a tow means for a civilian's orders
///
/// Nothing, deliberately. A towed hauler goes on publishing whatever route
/// directive #1028 gave it and goes on complying or refusing exactly as before:
/// a tow is a *physical* override, not an order, and the compliance state
/// machine is about orders. What the tow does take away is the motion — the
/// three speed fields are zeroed while attached — so a craft released from a
/// tow does not shoot off at whatever velocity its own helm had been quietly
/// accumulating against the rig.
///
/// # Determinism
///
/// Operators are walked in UUID order and the load is placed from the
/// operator's post-integration transform, so two hosts put the same craft in
/// the same place on the same tick.
pub fn move_towed_targets(
    mut set: ParamSet<(
        Query<(&EntityUuid, &Transform, &ShipOperations)>,
        Query<(
            &EntityUuid,
            &mut Transform,
            Option<&mut crate::ship::state::ShipPhysics>,
        )>,
    )>,
) {
    // Operator uuid, target uuid, and where the rig puts the load. The
    // authored offset is in the operator's OWN frame, so a tug that turns
    // swings its load around with it rather than dragging it sideways through
    // the towline — which is why the rotation is applied here, against the
    // operator's post-integration transform, rather than added in world space.
    let mut rigs: Vec<(String, String, Vec3)> = set
        .p0()
        .iter()
        .filter_map(|(uuid, transform, ops)| {
            let hold = ops.active.as_ref().filter(|hold| !hold.is_settled())?;
            if hold.verb() != OperationVerb::Tow {
                return None;
            }
            // Only a hold that is actually HOLDING drags its load. A tow that
            // has stalled — out of range, out of power, interrupted by an
            // authored rule — has let go, which is the readable behaviour: the
            // crew watch the hulk stop following them.
            if hold.state() != crate::operations::hold::HoldState::Holding {
                return None;
            }
            let capability = ops.capabilities.capability(hold.verb())?;
            Some((
                uuid.0.clone(),
                hold.target_uuid().to_string(),
                transform.translation + transform.rotation * Vec3::from(capability.tow_offset),
            ))
        })
        .collect();
    if rigs.is_empty() {
        return;
    }
    rigs.sort_by(|a, b| a.0.cmp(&b.0));

    for (_, target_uuid, placed) in rigs {
        let mut targets = set.p1();
        let Some((_, mut transform, physics)) = targets
            .iter_mut()
            .find(|(uuid, _, _)| uuid.0 == target_uuid)
        else {
            continue;
        };
        transform.translation = placed;
        if let Some(mut physics) = physics {
            physics.x = placed.x;
            physics.y = placed.y;
            physics.z = placed.z;
            // Zeroed so a craft released from the rig does not shoot off at
            // whatever velocity its own helm had been quietly accumulating
            // against a position it was never allowed to reach.
            physics.forward_speed = 0.0;
            physics.lateral_speed = 0.0;
            physics.vertical_speed = 0.0;
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
                // AC3: the slowed rate is visible. A crawling bar with no
                // number beside it reads as a bug; a crawling bar labelled
                // 25 % reads as the storm.
                rate_percent: hold.rate().as_percent(),
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
        OperationVerb::Tow => "operation.verb.tow",
        OperationVerb::Escort => "operation.verb.escort",
        OperationVerb::Transfer => "operation.verb.transfer",
        OperationVerb::FieldRepair => "operation.verb.field_repair",
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
            ..Default::default()
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
                Transform::default(),
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

        app.world_mut()
            .get_mut::<Transform>(ship)
            .unwrap()
            .translation
            .x = 5_000.0;
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

        app.world_mut()
            .get_mut::<Transform>(ship)
            .unwrap()
            .translation
            .x = 100.0;
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

    #[test]
    fn a_ship_with_nothing_to_do_leaves_the_runtime_unmarked() {
        let (mut app, _, _) = app_with(capable(), 100.0);
        app.update();
        app.update();
        let changed = app
            .world()
            .resource_ref::<WorldContentRuntime>()
            .is_changed();
        assert!(
            !changed,
            "a capable ship running nothing must not mark WorldContentRuntime changed — every \
             world in the repo carries that resource, and a needless mark is a needless wake-up \
             for everything that watches it"
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
                    Transform::default(),
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
                Transform::default(),
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
                Transform::from_xyz(5_000.0, 0.0, 0.0),
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

    // ══ Issue #1027 ══════════════════════════════════════════════════════════

    use crate::infrastructure::{CapacityConfig, InfrastructureConfig, InfrastructureState};
    use crate::operations::hold::{
        InterruptCause, InterruptResponse, InterruptRule, ProgressRate, TargetRequirement,
        TransferTerms,
    };

    fn capability_of(verb: OperationVerb) -> CapabilityConfig {
        CapabilityConfig {
            verb,
            range: 400.0,
            duration_secs: 2,
            ..Default::default()
        }
    }

    fn ops_with(capability: CapabilityConfig) -> ShipOperations {
        ShipOperations {
            capabilities: OperationsConfig {
                capabilities: vec![capability],
            },
            ..Default::default()
        }
    }

    // ── The region-effect vocabulary is total ──

    #[test]
    fn every_live_region_effect_maps_onto_an_authorable_name() {
        // The mapping is a `match` on `RegionEffectKind`, so a new hazard will
        // not compile until it is authorable. This pins the pairs so the
        // mapping cannot be made total by pointing two kinds at one name.
        let pairs = [
            (
                RegionEffectKind::DamageZone {
                    dps: 1.0,
                    shield_pierce: 0.0,
                },
                RegionEffectName::DamageZone,
            ),
            (
                RegionEffectKind::SlowZone {
                    thrust_modifier: None,
                    yaw_rate_modifier: None,
                },
                RegionEffectName::SlowZone,
            ),
            (
                RegionEffectKind::BlocksImpulse,
                RegionEffectName::BlocksImpulse,
            ),
            (
                RegionEffectKind::RadarDampening { multiplier: 0.5 },
                RegionEffectName::RadarDampening,
            ),
            (RegionEffectKind::CommsJam, RegionEffectName::CommsJam),
            (RegionEffectKind::SensorBlind, RegionEffectName::SensorBlind),
            (
                RegionEffectKind::NebulaFog {
                    color: [0.0; 3],
                    density: 0.01,
                },
                RegionEffectName::NebulaFog,
            ),
        ];
        assert_eq!(
            pairs.len(),
            RegionEffectName::ALL.len(),
            "every authorable name is reachable from a live region effect, and vice versa — a \
             hazard band an interrupt rule cannot name is a hazard operations cannot be told about"
        );
        for (kind, name) in pairs {
            assert_eq!(region_effect_name(&kind), name);
        }
    }

    // ── AC2/AC3: the interrupts arrive through the real world ──

    /// A bare app carrying one operating ship inside one authored region, plus
    /// the stabilisable target.
    fn app_in_a_region(
        ops: ShipOperations,
        effects: Vec<RegionEffectKind>,
    ) -> (App, Entity, Entity) {
        let (mut app, ship, target) = app_with(ops, 100.0);
        let region = app
            .world_mut()
            .spawn(crate::entities::spawner::RegionEffectsSection(effects))
            .id();
        let mut membership = crate::regions::server::RegionMembership::default();
        membership
            .inside
            .insert(ship, std::iter::once(region).collect());
        app.insert_resource(membership);
        (app, ship, target)
    }

    #[test]
    fn a_slow_zone_stretches_the_hold_through_the_real_region_membership() {
        let ops = ops_with(CapabilityConfig {
            duration_secs: 1,
            interrupts: vec![InterruptRule {
                cause: InterruptCause::Region,
                region_effect: Some(RegionEffectName::SlowZone),
                response: InterruptResponse::Slow,
                rate_percent: 50,
            }],
            ..capability_of(OperationVerb::Tow)
        });
        let (mut app, ship, _) = app_in_a_region(
            ops,
            vec![RegionEffectKind::SlowZone {
                thrust_modifier: None,
                yaw_rate_modifier: None,
            }],
        );
        start_via_script(&mut app, OperationVerb::Tow);
        for _ in 0..60 {
            app.update();
        }

        let hold = ops_of(&app, ship).active.expect("the hold opened");
        assert_eq!(
            hold.state(),
            HoldState::Holding,
            "the band SLOWS the operation — it does not stall it, and the crew are not being \
             punished for flying through the storm they were sent into"
        );
        assert_eq!(hold.rate(), ProgressRate::percent(50));
        assert_eq!(
            hold.elapsed_ticks(),
            30,
            "sixty ticks in the band buy thirty ticks of hold — the interruption arrives through \
             the ship's REAL region membership, not through a flag something else set"
        );
        assert_eq!(hold.stalled_ticks(), 0);
    }

    #[test]
    fn a_band_the_capability_authored_no_rule_for_does_nothing_at_all() {
        // The nebula the tender flies through on the way is not the storm.
        let ops = ops_with(CapabilityConfig {
            interrupts: vec![InterruptRule {
                cause: InterruptCause::Region,
                region_effect: Some(RegionEffectName::SlowZone),
                response: InterruptResponse::Fail,
                rate_percent: 50,
            }],
            ..stabilise_capability()
        });
        let (mut app, ship, _) = app_in_a_region(
            ops,
            vec![
                RegionEffectKind::CommsJam,
                RegionEffectKind::NebulaFog {
                    color: [0.1; 3],
                    density: 0.01,
                },
            ],
        );
        start_via_script(&mut app, OperationVerb::Stabilise);
        for _ in 0..30 {
            app.update();
        }
        let hold = ops_of(&app, ship).active.expect("the hold opened");
        assert_eq!(hold.state(), HoldState::Holding);
        assert_eq!(hold.elapsed_ticks(), 30, "at full rate");
    }

    #[test]
    fn coming_under_fire_interrupts_the_hold_on_the_authored_terms() {
        use crate::ship::combat_activity::RecentCombatActivity;

        let ops = ops_with(CapabilityConfig {
            interrupts: vec![InterruptRule {
                cause: InterruptCause::Attack,
                region_effect: None,
                response: InterruptResponse::Fail,
                rate_percent: 100,
            }],
            ..capability_of(OperationVerb::FieldRepair)
        });
        let (mut app, ship, _) = app_with(ops, 100.0);
        app.world_mut()
            .entity_mut(ship)
            .insert(RecentCombatActivity::default());
        start_via_script(&mut app, OperationVerb::FieldRepair);
        app.update();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Holding),
            "precondition: a ship nobody has hit is holding"
        );

        // A hit lands. `Time` in a bare app sits at zero, so a timestamp of
        // zero is inside any window — which is the same reading the doctrine
        // gates take of a ship hit on tick zero.
        app.world_mut()
            .get_mut::<RecentCombatActivity>(ship)
            .unwrap()
            .last_hostile_fire_taken = Some(0.0);
        app.update();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Failed(Ineligibility::UnderAttack)),
            "a repair party working an open hull under fire is called off — and the reason names \
             the fire, so the crew know what to fix"
        );
    }

    #[test]
    fn firing_your_own_guns_is_not_being_interrupted() {
        use crate::ship::combat_activity::RecentCombatActivity;

        let ops = ops_with(CapabilityConfig {
            interrupts: vec![InterruptRule {
                cause: InterruptCause::Attack,
                region_effect: None,
                response: InterruptResponse::Fail,
                rate_percent: 100,
            }],
            ..stabilise_capability()
        });
        let (mut app, ship, _) = app_with(ops, 100.0);
        app.world_mut()
            .entity_mut(ship)
            .insert(RecentCombatActivity {
                last_weapon_fired: Some(0.0),
                ..Default::default()
            });
        start_via_script(&mut app, OperationVerb::Stabilise);
        for _ in 0..30 {
            app.update();
        }
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Holding),
            "the interrupt reads LANDED HITS, the same fold the doctrine gates take. A tender \
             that fired a warning shot has not been interrupted, and folding its own fire in \
             would make every armed operator interrupt itself."
        );
    }

    // ── AC6: the tow moves its load ──

    #[test]
    fn a_towed_target_rides_the_authored_offset_in_the_operators_own_frame() {
        let ops = ops_with(CapabilityConfig {
            duration_secs: 60,
            range: 5_000.0,
            tow_offset: [0.0, 0.0, -150.0],
            ..capability_of(OperationVerb::Tow)
        });
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(Update, (tick_operations, move_towed_targets).chain());
        // The tug faces along -Z with no yaw, 1000 units out on X.
        let ship = app
            .world_mut()
            .spawn((
                EntityUuid(SHIP.to_string()),
                Transform::from_xyz(1_000.0, 0.0, 0.0),
                ops,
            ))
            .id();
        let hulk = app
            .world_mut()
            .spawn((
                EntityUuid(TARGET.to_string()),
                Transform::from_xyz(1_010.0, 0.0, 0.0),
            ))
            .id();
        start_via_script(&mut app, OperationVerb::Tow);
        app.update();

        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Holding),
            "a tow does not need a condition track on its target — a derelict freighter carries \
             none, and is exactly the thing you tow"
        );
        assert_eq!(
            app.world().get::<Transform>(hulk).unwrap().translation,
            Vec3::new(1_000.0, 0.0, -150.0),
            "the load rides 150 units astern of the tug, not at its own last position"
        );

        // Move the tug: the load follows.
        app.world_mut()
            .get_mut::<Transform>(ship)
            .unwrap()
            .translation = Vec3::new(2_000.0, 40.0, 0.0);
        app.update();
        assert_eq!(
            app.world().get::<Transform>(hulk).unwrap().translation,
            Vec3::new(2_000.0, 40.0, -150.0),
            "…and goes on following it, which is the whole of what a tow is"
        );
    }

    #[test]
    fn the_offset_turns_with_the_tug_rather_than_dragging_sideways() {
        let ops = ops_with(CapabilityConfig {
            duration_secs: 60,
            range: 5_000.0,
            tow_offset: [0.0, 0.0, -100.0],
            ..capability_of(OperationVerb::Tow)
        });
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(Update, (tick_operations, move_towed_targets).chain());
        let ship = app
            .world_mut()
            .spawn((
                EntityUuid(SHIP.to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0)
                    .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
                ops,
            ))
            .id();
        let hulk = app
            .world_mut()
            .spawn((EntityUuid(TARGET.to_string()), Transform::default()))
            .id();
        start_via_script(&mut app, OperationVerb::Tow);
        app.update();
        let _ = ship;

        let placed = app.world().get::<Transform>(hulk).unwrap().translation;
        assert!(
            (placed - Vec3::new(-100.0, 0.0, 0.0)).length() < 1e-3,
            "a tug yawed a quarter turn puts its load abeam of world axes but still ASTERN of \
             itself: the offset is authored in the operator's own frame, so a tug that turns \
             swings its load around with it rather than dragging it sideways through the \
             towline. Got {placed:?}"
        );
    }

    #[test]
    fn a_towed_ship_has_its_own_motion_zeroed_and_a_stalled_tow_lets_go() {
        use crate::ship::state::ShipPhysics;

        let ops = ops_with(CapabilityConfig {
            duration_secs: 60,
            range: 400.0,
            tow_offset: [0.0, 0.0, -50.0],
            ..capability_of(OperationVerb::Tow)
        });
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(Update, (tick_operations, move_towed_targets).chain());
        let ship = app
            .world_mut()
            .spawn((EntityUuid(SHIP.to_string()), Transform::default(), ops))
            .id();
        let hauler = app
            .world_mut()
            .spawn((
                EntityUuid(TARGET.to_string()),
                Transform::from_xyz(10.0, 0.0, 0.0),
                ShipPhysics {
                    x: 10.0,
                    forward_speed: 90.0,
                    ..Default::default()
                },
            ))
            .id();
        start_via_script(&mut app, OperationVerb::Tow);
        app.update();

        let physics = *app.world().get::<ShipPhysics>(hauler).unwrap();
        assert_eq!(
            (physics.x, physics.y, physics.z),
            (0.0, 0.0, -50.0),
            "the tow writes the target's OWN authoritative position source, so sync_ship_position \
             projects it rather than undoing it next tick"
        );
        assert_eq!(
            physics.forward_speed, 0.0,
            "…and its motion is zeroed, so a craft released from the rig does not shoot off at a \
             velocity its helm accumulated against a position it was never allowed to reach"
        );

        // Fly the tug out of range: the tow stalls, and lets go.
        app.world_mut()
            .get_mut::<Transform>(ship)
            .unwrap()
            .translation
            .x = 50_000.0;
        app.update();
        let after = *app.world().get::<ShipPhysics>(hauler).unwrap();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Stalled(Ineligibility::OutOfRange))
        );
        assert_eq!(
            (after.x, after.y, after.z),
            (0.0, 0.0, -50.0),
            "a stalled tow has LET GO — the load stays where the towline parted rather than being \
             yanked across the map to wherever the tug got to. Only a hold that is actually \
             holding drags its load."
        );
    }

    // ── AC7: transfer moves capacity between two infrastructure entities ──

    fn depot(capacity: &str, level: i64, ceiling: i64) -> InfrastructureCondition {
        InfrastructureCondition(InfrastructureState::from_config(&InfrastructureConfig {
            capacities: vec![CapacityConfig {
                id: capacity.to_string(),
                amount: level,
                ceiling: Some(ceiling),
            }],
            ..Default::default()
        }))
    }

    /// A tender and a depot, each carrying the same named capacity.
    fn transfer_app(
        direction: TransferDirection,
        amount: i64,
        operator: (i64, i64),
        target: (i64, i64),
    ) -> (App, Entity) {
        const CAPACITY: &str = "berths";
        let ops = ops_with(CapabilityConfig {
            transfer: Some(TransferTerms {
                capacity: CAPACITY.to_string(),
                amount,
                direction,
            }),
            ..capability_of(OperationVerb::Transfer)
        });
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(Update, tick_operations);
        let ship = app
            .world_mut()
            .spawn((
                EntityUuid(SHIP.to_string()),
                Transform::from_xyz(100.0, 0.0, 0.0),
                depot(CAPACITY, operator.0, operator.1),
                ops,
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(TARGET.to_string()),
            Transform::default(),
            depot(CAPACITY, target.0, target.1),
        ));
        (app, ship)
    }

    fn queued_capacity_moves(app: &App) -> Vec<(String, String, i64)> {
        app.world()
            .resource::<WorldContentRuntime>()
            .pending_capacity_adjustments
            .iter()
            .map(|a| (a.uuid.clone(), a.capacity.clone(), a.delta))
            .collect()
    }

    #[test]
    fn a_completed_delivery_queues_both_ends_of_the_move() {
        let (mut app, ship) = transfer_app(TransferDirection::Deliver, 12, (40, 40), (0, 40));
        start_via_script(&mut app, OperationVerb::Transfer);
        for _ in 0..120 {
            app.update();
        }
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Completed)
        );
        assert_eq!(
            queued_capacity_moves(&app),
            vec![
                (SHIP.to_string(), "berths".to_string(), -12),
                (TARGET.to_string(), "berths".to_string(), 12),
            ],
            "both ends move on the same tick and in a fixed order — source first — so two hosts \
             queue the pair identically. And they are QUEUED for \
             tick_infrastructure_condition rather than written onto the components, because that \
             is the one system that re-publishes the counter a scenario predicate reads (#1025)"
        );
    }

    #[test]
    fn a_collection_moves_the_load_the_other_way() {
        let (mut app, _) = transfer_app(TransferDirection::Collect, 5, (0, 40), (40, 40));
        start_via_script(&mut app, OperationVerb::Transfer);
        for _ in 0..120 {
            app.update();
        }
        assert_eq!(
            queued_capacity_moves(&app),
            vec![
                (TARGET.to_string(), "berths".to_string(), -5),
                (SHIP.to_string(), "berths".to_string(), 5),
            ],
            "collecting takes from the depot and puts it aboard, and the SOURCE is still queued \
             first — the order is the transfer's direction, not the entity's identity"
        );
    }

    #[test]
    fn a_transfer_with_no_room_at_the_far_end_stalls_and_moves_nothing() {
        // The tender is loaded; the depot is already full.
        let (mut app, ship) = transfer_app(TransferDirection::Deliver, 12, (40, 40), (40, 40));
        start_via_script(&mut app, OperationVerb::Transfer);
        for _ in 0..120 {
            app.update();
        }
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Stalled(Ineligibility::CapacityUnavailable)),
            "a full depot STALLS the transfer rather than failing it: a crew waiting at the \
             airlock for a berth to clear is playing the game"
        );
        assert!(
            queued_capacity_moves(&app).is_empty(),
            "…and nothing moves while it waits"
        );
    }

    #[test]
    fn a_transfer_against_a_target_that_has_no_such_capacity_is_inapplicable() {
        const CAPACITY: &str = "berths";
        let ops = ops_with(CapabilityConfig {
            transfer: Some(TransferTerms {
                capacity: CAPACITY.to_string(),
                amount: 1,
                direction: TransferDirection::Deliver,
            }),
            ..capability_of(OperationVerb::Transfer)
        });
        let mut app = App::new();
        app.init_resource::<WorldContentRuntime>();
        app.add_systems(Update, tick_operations);
        let ship = app
            .world_mut()
            .spawn((
                EntityUuid(SHIP.to_string()),
                Transform::from_xyz(100.0, 0.0, 0.0),
                depot(CAPACITY, 10, 10),
                ops,
            ))
            .id();
        // A depot that publishes a DIFFERENT capacity: present, in range, and
        // not a thing you can deliver berths to.
        app.world_mut().spawn((
            EntityUuid(TARGET.to_string()),
            Transform::default(),
            depot("fuel", 0, 100),
        ));
        start_via_script(&mut app, OperationVerb::Transfer);
        app.update();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Failed(Ineligibility::TargetNotApplicable)),
            "'no room' would send the crew away to wait for a berth that is never coming — the \
             refusal has to say the depot does not do berths at all"
        );
    }

    // ── AC4/AC5: field-repair pays per tick and commits teams ──

    #[test]
    fn a_field_repair_pays_condition_every_tick_it_holds_rather_than_on_completion() {
        let ops = ops_with(CapabilityConfig {
            duration_secs: 1,
            condition_per_second: 6.0,
            ..capability_of(OperationVerb::FieldRepair)
        });
        let (mut app, ship, _) = app_with(ops, 100.0);
        start_via_script(&mut app, OperationVerb::FieldRepair);
        for _ in 0..30 {
            app.update();
        }
        let paid = queued_adjustments(&app);
        assert_eq!(
            paid.len(),
            30,
            "half a second in, the repair has already paid thirty slices — a crew pulled off \
             here keep half a second of work, which is what makes field-repair different from \
             stabilise's all-or-nothing lump"
        );
        assert!(
            paid.iter()
                .all(|a| a.uuid == TARGET && (a.delta - 0.1).abs() < 1e-6),
            "each slice is a tenth of a point — six a second at 60 Hz — and lands on the target: \
             {paid:?}"
        );
        assert!(
            !ops_of(&app, ship).active.unwrap().is_settled(),
            "precondition: it has not completed yet, so none of this is a completion payout"
        );
    }

    #[test]
    fn a_stalled_field_repair_pays_nothing_and_a_slowed_one_pays_less() {
        let interrupts = vec![InterruptRule {
            cause: InterruptCause::Region,
            region_effect: Some(RegionEffectName::DamageZone),
            response: InterruptResponse::Slow,
            rate_percent: 50,
        }];
        let ops = ops_with(CapabilityConfig {
            duration_secs: 10,
            condition_per_second: 6.0,
            interrupts,
            ..capability_of(OperationVerb::FieldRepair)
        });
        let (mut app, ship, _) = app_in_a_region(
            ops,
            vec![RegionEffectKind::DamageZone {
                dps: 1.0,
                shield_pierce: 0.0,
            }],
        );
        start_via_script(&mut app, OperationVerb::FieldRepair);
        for _ in 0..10 {
            app.update();
        }
        let slowed = queued_adjustments(&app);
        assert_eq!(slowed.len(), 10);
        assert!(
            slowed.iter().all(|a| (a.delta - 0.05).abs() < 1e-6),
            "half rate pays half a slice: the work and its payoff cannot come apart, or parking \
             in a storm would be a way of farming condition points. Got {slowed:?}"
        );

        // Out of range: stalled, and paying nothing at all.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_condition_adjustments
            .clear();
        app.world_mut()
            .get_mut::<Transform>(ship)
            .unwrap()
            .translation
            .x = 50_000.0;
        for _ in 0..10 {
            app.update();
        }
        assert!(
            queued_adjustments(&app).is_empty(),
            "a stalled repair pays nothing — the party is not working"
        );
    }

    #[test]
    fn a_field_repair_commits_teams_and_releases_them_when_it_settles() {
        let ops = ops_with(CapabilityConfig {
            duration_secs: 1,
            repair_teams: 2,
            ..capability_of(OperationVerb::FieldRepair)
        });
        let (mut app, ship, _) = app_with(ops, 100.0);
        assert_eq!(
            ops_of(&app, ship).committed_repair_teams(),
            0,
            "a ship running nothing commits nothing"
        );
        start_via_script(&mut app, OperationVerb::FieldRepair);
        app.update();
        assert_eq!(
            ops_of(&app, ship).committed_repair_teams(),
            2,
            "the hold commits the capability's authored team count for the duration"
        );

        for _ in 0..60 {
            app.update();
        }
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Completed),
            "precondition: it finished"
        );
        assert_eq!(
            ops_of(&app, ship).committed_repair_teams(),
            0,
            "…and the teams are released on completion. Derived from the live hold rather than \
             stored, so there is no release step to forget and nothing extra for a save to carry."
        );
    }

    #[test]
    fn a_hull_short_of_teams_stalls_the_field_repair_rather_than_running_it_free() {
        use crate::console::repair::server::ShipRepairTeams;
        use crate::modifiers::repair_teams::RepairTeams;

        let ops = ops_with(CapabilityConfig {
            duration_secs: 1,
            repair_teams: 3,
            ..capability_of(OperationVerb::FieldRepair)
        });
        let (mut app, ship, _) = app_with(ops, 100.0);
        app.world_mut()
            .entity_mut(ship)
            .insert(ShipRepairTeams(RepairTeams::new(2)));
        start_via_script(&mut app, OperationVerb::FieldRepair);
        app.update();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Stalled(Ineligibility::TeamsUnavailable)),
            "an operation that commits three teams cannot run on a hull carrying two — and it \
             stalls rather than failing, because a team finishing its internal job would free it"
        );
    }

    #[test]
    fn a_hull_with_no_repair_roster_at_all_is_not_gated_on_teams() {
        // The fixture ship carries no `ShipRepairTeams`. Absence of a roster is
        // absence of the constraint, on the same reading a missing power grid
        // takes.
        let ops = ops_with(CapabilityConfig {
            duration_secs: 1,
            repair_teams: 4,
            ..capability_of(OperationVerb::FieldRepair)
        });
        let (mut app, ship, _) = app_with(ops, 100.0);
        start_via_script(&mut app, OperationVerb::FieldRepair);
        app.update();
        assert_eq!(
            ops_of(&app, ship).active.map(|h| h.state()),
            Some(HoldState::Holding),
            "a fixture with no repair roster must not be refused for a component it never had"
        );
    }

    // ── The wire ──

    #[test]
    fn the_published_hold_carries_the_slowed_rate_the_console_has_to_show() {
        let ops = ops_with(CapabilityConfig {
            duration_secs: 30,
            interrupts: vec![InterruptRule {
                cause: InterruptCause::Region,
                region_effect: Some(RegionEffectName::SlowZone),
                response: InterruptResponse::Slow,
                rate_percent: 25,
            }],
            ..capability_of(OperationVerb::Tow)
        });
        let (mut app, ship, _) = app_in_a_region(
            ops,
            vec![RegionEffectKind::SlowZone {
                thrust_modifier: None,
                yaw_rate_modifier: None,
            }],
        );
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::server_app::ShipSystemBlackboards::default());
        app.add_systems(Update, publish_operations_blackboard.after(tick_operations));
        start_via_script(&mut app, OperationVerb::Tow);
        app.update();

        let blackboards = app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(ship)
            .expect("the ship carries a blackboard map");
        let Some(SystemBlackboard::Operations(bb)) =
            blackboards.0.get(&operations_blackboard_key())
        else {
            panic!("operations publish under their own channel key");
        };
        let active = bb.active.as_ref().expect("the live hold is published");
        assert_eq!(active.state, "holding");
        assert_eq!(
            active.rate_percent, 25,
            "a bar that crawls with no number beside it reads as a bug; one labelled 25 % reads \
             as the storm, which is the whole reason the rate is on the wire"
        );
        assert_eq!(
            bb.capabilities
                .iter()
                .map(|c| c.verb.as_str())
                .collect::<Vec<_>>(),
            vec!["tow"],
            "…and the verb travels as its machine code, with a strings.csv id beside it"
        );
    }

    #[test]
    fn a_hull_offering_every_verb_publishes_every_one_of_them() {
        let mut app = App::new();
        app.add_systems(Update, publish_operations_blackboard);
        let ship = app
            .world_mut()
            .spawn((
                ShipOperations {
                    capabilities: OperationsConfig {
                        capabilities: OperationVerb::ALL
                            .iter()
                            .map(|verb| CapabilityConfig {
                                target_requirement: Some(TargetRequirement::Present),
                                ..capability_of(*verb)
                            })
                            .collect(),
                    },
                    ..Default::default()
                },
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();
        app.update();
        let blackboards = app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(ship)
            .unwrap();
        let Some(SystemBlackboard::Operations(bb)) =
            blackboards.0.get(&operations_blackboard_key())
        else {
            panic!("the blackboard is published");
        };
        assert_eq!(
            bb.capabilities
                .iter()
                .map(|c| (c.verb.as_str(), c.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("stabilise", "operation.verb.stabilise"),
                ("tow", "operation.verb.tow"),
                ("escort", "operation.verb.escort"),
                ("transfer", "operation.verb.transfer"),
                ("field_repair", "operation.verb.field_repair"),
            ],
            "a tender that can do everything offers everything, in authored order, each with a \
             strings id rather than English — the console renders the list without knowing what \
             any of them mean"
        );
    }

    // ── Determinism ──

    #[test]
    fn two_tugs_claiming_one_load_resolve_it_the_same_way_on_every_host() {
        // Contention is the case that can diverge: with a load each, the order
        // the rigs are applied in cannot matter. With two tugs on ONE load, the
        // last write wins — so which one wins must be a function of the UUIDs
        // and not of whichever entity a query happened to yield first.
        fn run(order: [(&str, f32); 2]) -> Vec3 {
            let mut app = App::new();
            app.init_resource::<WorldContentRuntime>();
            app.add_systems(Update, (tick_operations, move_towed_targets).chain());
            app.world_mut()
                .spawn((EntityUuid("hulk".to_string()), Transform::default()));
            for (uuid, x) in order {
                let ops = ops_with(CapabilityConfig {
                    duration_secs: 60,
                    range: 100_000.0,
                    tow_offset: [0.0, 0.0, -10.0],
                    ..capability_of(OperationVerb::Tow)
                });
                app.world_mut().spawn((
                    EntityUuid(uuid.to_string()),
                    Transform::from_xyz(x, 0.0, 0.0),
                    ops,
                ));
                app.world_mut()
                    .resource_mut::<WorldContentRuntime>()
                    .pending_operation_starts
                    .push(PendingOperationStart {
                        ship_uuid: uuid.to_string(),
                        verb: OperationVerb::Tow,
                        target_uuid: "hulk".to_string(),
                    });
            }
            app.update();
            app.world_mut()
                .query::<(&EntityUuid, &Transform)>()
                .iter(app.world())
                .find(|(uuid, _)| uuid.0 == "hulk")
                .map(|(_, tf)| tf.translation)
                .expect("the hulk is in the world")
        }
        let forwards = run([("tug-a", 100.0), ("tug-b", 900.0)]);
        let backwards = run([("tug-b", 900.0), ("tug-a", 100.0)]);
        assert_eq!(
            forwards, backwards,
            "two tugs claiming one hulk must put it in the same place on every host — the rigs \
             are applied in the operators' UUID order, not in archetype order"
        );
        assert_eq!(
            forwards,
            Vec3::new(900.0, 0.0, -10.0),
            "…and the resolution is the LAST rig in UUID order, which is a rule someone can read \
             off the sort rather than a coin toss"
        );
    }
}

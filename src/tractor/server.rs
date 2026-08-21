//! The Bevy adapter for the tractor beam (issue #1156).
//!
//! Gathers the live world into scalars, hands them to the pure sibling
//! [`crate::tractor::coupling`], and applies what comes back — the per-ship
//! [`TractorBeam`] component, the fixed-tick systems that take the engage/release
//! commands, decide whether the coupling holds, move the held target onto the
//! operator's rig, and publish the blackboard. Nothing here decides geometry or
//! eligibility itself: rule 10, the same split `operations::server` keeps with
//! `operations::hold`.
//!
//! # The parallel-to-operations decision
//!
//! This is a NEW system standing beside the live `operations` tow path, not a
//! change to it (the issue is explicit). The tow is a scripted operation verb; a
//! tractor is an admission-gated engineering `[[system]]` the crew engage
//! against the ship's own lock. They share the writer-policy shape — a
//! correction that writes the held target's position after integration — and the
//! coupling-position maths, now lifted into the pure module both could call, but
//! nothing else.

use bevy::prelude::*;

use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
use crate::console::weapons::beam::TacticalRadarSelection;
use crate::damage::DamageTier;
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};
use crate::infrastructure::condition::ConditionAdjustment;
use crate::infrastructure::InfrastructureCondition;
use crate::messages::{
    PowerGroupId, SystemBlackboard, SystemControlPayload, SystemId, TractorBlackboard,
};
use crate::ship::power::{power_level_for, ShipPowerSystem};
use crate::ship::state::ShipPhysics;
use crate::system_registry::{tractor_system_id, TRACTOR_SYSTEM_ID};
use crate::tractor::coupling::{coupled_position, hold_status, TractorConfig, TractorRefusal};
use crate::tractor::held_response::{condition_delta, held_offset, HeldResponseConfig};
use crate::world::server::WorldContentRuntime;

/// One ship's tractor beam (issue #1156): the authored coupling terms, the power
/// group it draws from, and the live engage/coupling state.
///
/// Inserted at spawn only on a hull that authored a `[tractor]` table AND a
/// `kind = "tractor"` `[[system]]` — a hull with neither carries no component
/// and is byte-identical in every way to one built before this existed
/// (AGENTS.md rule 11).
///
/// `engaged` is the operator's INTENT — set by `EngageTractor`, cleared by
/// `ReleaseTractor` and by any interruption. `coupled_target` is what the beam
/// is actually holding this tick, `Some(target-uuid)` only while the hold is
/// eligible. The two agree by construction: on any refusal both `engaged` and
/// `coupled_target` clear together, which is the "each interruption ends the
/// hold" the crew see as the hulk stopping.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TractorBeam {
    /// The authored coupling terms — range, offset, minimum power level.
    pub config: TractorConfig,
    /// The power group the tractor `[[system]]` declared, resolved once at spawn
    /// so the runtime never re-reads the authored systems list. The single
    /// authored source is the `[[system]] power_group` field.
    pub power_group: PowerGroupId,
    /// The operator's standing intent to hold. Set true by `EngageTractor`,
    /// false by `ReleaseTractor` and by every interruption.
    pub engaged: bool,
    /// The target-uuid the beam is holding this tick, or `None` when nothing is
    /// held. Present only while `engaged` and the hold is eligible.
    pub coupled_target: Option<String>,
    /// Why the last engage or hold could not form — the reason the console
    /// shows, retained until the operator engages or releases again. `None` when
    /// the beam is idle or holding cleanly.
    pub last_refusal: Option<TractorRefusal>,
}

impl TractorBeam {
    /// A fresh, idle beam carrying its authored terms and resolved power group.
    pub fn new(config: TractorConfig, power_group: PowerGroupId) -> Self {
        Self {
            config,
            power_group,
            engaged: false,
            coupled_target: None,
            last_refusal: None,
        }
    }

    /// The persistable half — the engage state and the coupled target — for the
    /// snapshot payload (issue #1156). The authored config and power group ride
    /// the template and are re-derived on spawn, so they are deliberately not
    /// here, exactly as the tow's `OperationsSaveState` leaves the capability
    /// table out.
    pub fn save_state(&self) -> TractorSaveState {
        TractorSaveState {
            engaged: self.engaged,
            coupled_target: self.coupled_target.clone(),
        }
    }

    /// Reseed the engage state and coupled target from a restored snapshot
    /// (issue #1156), onto a beam that already carries its authored config and
    /// resolved power group from the fresh spawn.
    ///
    /// The last refusal is deliberately NOT restored: it is a projection the next
    /// `tick_tractor` re-derives from the resumed world (a beam that comes back
    /// out of range will refuse again on its first tick), so carrying a stale one
    /// would show the crew a reason for a condition that no longer holds.
    pub fn restore(&mut self, save: &TractorSaveState) {
        self.engaged = save.engaged;
        self.coupled_target = save.coupled_target.clone();
        self.last_refusal = None;
    }
}

/// The snapshot-carried half of a [`TractorBeam`] (issue #1156): the engaged
/// state and the coupled target, and nothing else.
///
/// `Default` is the idle beam — not engaged, holding nothing — which is what a
/// hull that authored a tractor and never engaged it captures, so a resume of
/// such a ship restores byte-identically and folds the same number.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TractorSaveState {
    #[serde(default)]
    pub engaged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coupled_target: Option<String>,
}

/// Present on a TARGET entity that authored a `[held_response]` table (issue
/// #1158): what being held does to it.
///
/// Authored, immutable target-side config — the mirror of the OPERATOR's
/// [`TractorBeam`], which carries what the beam is. A target that authors no
/// table carries no component and is merely held in place (station-keep). The
/// tractor server reads this off the held target and applies whatever it
/// declares: the offset it rides at (via [`held_offset`], for formation-keep)
/// and the condition it banks (via [`condition_delta`], for arrest-decline).
/// The tractor itself never branches on the kind — it applies the value the
/// held target's own config resolves to.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct HeldResponseSection(pub HeldResponseConfig);

/// Registers the tractor systems and its admitted-command consumer (issue
/// #1156). Added by `WorldPlugin` alongside `OperationsPlugin`.
pub struct TractorPlugin;

impl Plugin for TractorPlugin {
    fn build(&self, app: &mut App) {
        // The tractor `[[system]]` is an admitted-command consumer: `handle_
        // tractor_commands` reads `EngageTractor` / `ReleaseTractor` for it, so
        // admission fans those commands into every ship's `AdmittedCommands`
        // each tick and the end-of-frame lint never warns them unrouted.
        app.register_admitted_consumer(ConsumerMatcher::exact(TRACTOR_SYSTEM_ID));
        app.add_systems(
            FixedUpdate,
            (
                handle_tractor_commands.in_set(crate::sim_sets::SimSet::Input),
                // Decide whether the coupling holds this tick, from the live
                // lock, range, power and damage.
                tick_tractor.in_set(crate::sim_sets::SimSet::Modifiers),
                // Then place the held target — after the tick that decided the
                // hold, so a beam that dropped this tick moves nothing, exactly
                // as the tow rig is ordered after `tick_operations`.
                move_coupled_target
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(tick_tractor),
                // Bank an arrest-decline target's condition (issue #1158) —
                // after the hold is decided, and BEFORE the infrastructure tick
                // that applies the decline this arrests and drains the queue,
                // the same ordering `tick_operations` keeps so its payoff lands
                // the same tick.
                arrest_held_declines
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(tick_tractor)
                    .before(crate::infrastructure::tick_infrastructure_condition),
                publish_tractor_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── The engage / release commands ────────────────────────────────────────────

/// Runs in `SimSet::Input` and reads `AdmittedCommands` for the tractor system
/// (issue #1156). It sets only the operator's INTENT; `tick_tractor` decides
/// whether a coupling actually forms and records any refusal, so an engage that
/// cannot hold is refused within the same tick before the blackboard publishes.
///
/// Human and AI reach this identically: admission has already decided who may
/// speak (a human tenure token at the network gate, or #1162's tractor AI
/// through the same `validate_and_admit` seam) and stripped the source, so
/// nothing here asks who sent the command (AGENTS.md rule 6).
pub fn handle_tractor_commands(
    mut ships: Query<(&crate::messages::AdmittedCommands, &mut TractorBeam)>,
) {
    for (admitted, mut beam) in ships.iter_mut() {
        // The last engage/release in the tick wins — the same latest-command-
        // wins policy the helm axes take, so a stale-UI double tap is idempotent.
        for cmd in admitted.for_target(TRACTOR_SYSTEM_ID) {
            match &cmd.payload {
                SystemControlPayload::EngageTractor if !beam.engaged => {
                    beam.engaged = true;
                    // Clear a stale refusal on a fresh engage; the tick will
                    // repopulate it if this engage cannot hold.
                    beam.last_refusal = None;
                }
                SystemControlPayload::ReleaseTractor => {
                    beam.engaged = false;
                    beam.coupled_target = None;
                    beam.last_refusal = None;
                }
                _ => {}
            }
        }
    }
}

// ── The hold verdict ─────────────────────────────────────────────────────────

/// Decide, for every engaged tractor, whether the coupling holds this tick, and
/// drop it (recording the reason) if not (issue #1156).
///
/// Reads the live world into the scalars the pure [`hold_status`] takes — the
/// ship's one lock (`TacticalRadarSelection`), the separation to the locked
/// target off both ends' `Transform`, the tractor's power-group level, and its
/// damage tier — and applies the verdict. On success the coupled target is the
/// lock; on any refusal `engaged` and `coupled_target` both clear and the reason
/// is retained for the console. An idle (`!engaged`) beam is left untouched, so
/// its retained refusal persists until the operator acts again.
pub fn tick_tractor(
    mut set: ParamSet<(
        // Operator rows: everything the verdict needs off the operator itself.
        Query<(
            Entity,
            &TractorBeam,
            Option<&TacticalRadarSelection>,
            &Transform,
            Option<&ShipPowerSystem>,
            Option<&EntitySystemHull>,
        )>,
        // Every entity's position, to resolve the locked target's separation.
        Query<(&EntityUuid, &Transform)>,
        // Apply the verdict.
        Query<&mut TractorBeam>,
    )>,
) {
    // Gather the operator inputs first, so the target lookup and the write can
    // each take the world without holding the other's borrow.
    struct Row {
        entity: Entity,
        lock: Option<String>,
        operator_pos: Vec3,
        power_level: u8,
        disabled: bool,
        range: f32,
        min_power_level: u8,
    }
    let rows: Vec<Row> = set
        .p0()
        .iter()
        .filter(|(_, beam, _, _, _, _)| beam.engaged)
        .map(|(entity, beam, selection, transform, power, hull)| {
            let power_level = power
                .map(|p| power_level_for(&p.0, &beam.power_group))
                .unwrap_or(0);
            let disabled = hull
                .map(|h| {
                    matches!(
                        h.0.tier_for(&tractor_system_id()),
                        DamageTier::Disabled | DamageTier::Destroyed
                    )
                })
                .unwrap_or(false);
            Row {
                entity,
                lock: selection.and_then(|s| s.0.clone()),
                operator_pos: transform.translation,
                power_level,
                disabled,
                range: beam.config.range,
                min_power_level: beam.config.min_power_level,
            }
        })
        .collect();
    if rows.is_empty() {
        return;
    }

    // The separation to each locked target, resolved once from the transform
    // query. `None` when the lock names an entity that no longer exists, which
    // the verdict reads as out of range.
    let separations: Vec<Option<f32>> = {
        let transforms = set.p1();
        rows.iter()
            .map(|row| {
                let lock = row.lock.as_deref()?;
                let target = transforms
                    .iter()
                    .find(|(uuid, _)| uuid.0 == lock)
                    .map(|(_, t)| t.translation)?;
                Some(row.operator_pos.distance(target))
            })
            .collect()
    };

    // Apply.
    let mut beams = set.p2();
    for (row, separation) in rows.iter().zip(separations) {
        let Ok(mut beam) = beams.get_mut(row.entity) else {
            continue;
        };
        match hold_status(
            row.lock.as_deref(),
            separation,
            row.range,
            row.power_level,
            row.min_power_level,
            row.disabled,
        ) {
            Ok(()) => {
                beam.coupled_target = row.lock.clone();
                beam.last_refusal = None;
            }
            Err(refusal) => {
                // Each interruption ends the hold: intent and coupling drop
                // together, and the crew watch the hulk stop following them.
                beam.engaged = false;
                beam.coupled_target = None;
                beam.last_refusal = Some(refusal);
            }
        }
    }
}

// ── The rig ──────────────────────────────────────────────────────────────────

/// Hold every coupled target on its operator's rig (issue #1156).
///
/// # Why this writes the target's position directly
///
/// This is a sixth row in the `ShipPhysics` writer-policy table
/// ([`crate::ship::state::ShipPhysics`]) — the exact shape the tow rig
/// (`operations::server::move_towed_targets`) is, and for the same reason:
/// nothing in this codebase attaches one entity to another, and the only
/// sanctioned way to override a hull's authoritative position is a correction
/// layered on top of the helm integration, applied in `SimSet::Modifiers` after
/// `sync_ship_position` has mirrored `ShipPhysics` into `Transform`.
///
/// It writes `ShipPhysics` where the target has one (a demoted craft) and lets
/// `sync_ship_position` project it, and writes `Transform` too so the two agree
/// within the tick. A derelict — no `[behaviour]`, so no `ShipPhysics` — has
/// nothing else that ever moves it, so the transform write is the whole of it.
/// The three speed fields are zeroed for the tow's reason turned around: a craft
/// released from the rig must not shoot off at a velocity its own helm
/// accumulated against a position it was never allowed to reach.
///
/// # Where the load rides is what the held target declares (issue #1158)
///
/// The offset fed to the pure `coupled_position` is not always the operator's
/// authored coupling rig: a **formation-kept** target rides its OWN authored
/// slot ([`held_offset`]), so escort is distinct from station-keeping a target
/// in place on the operator's rig. Every other response — follow, station-keep,
/// arrest-decline, and the default a target with no `[held_response]` authors —
/// rides the operator's rig exactly as before. The offset is chosen by the
/// held target's own resolved config; this system never branches on the kind.
///
/// Determinism: operators are walked in UUID order and each load is placed from
/// the operator's post-integration transform, so two hosts put the same craft in
/// the same place on the same tick.
pub fn move_coupled_target(
    mut set: ParamSet<(
        Query<(&EntityUuid, &Transform, &TractorBeam)>,
        Query<(
            &EntityUuid,
            &mut Transform,
            Option<&mut ShipPhysics>,
            Option<&HeldResponseSection>,
        )>,
    )>,
) {
    // Operator uuid, target uuid, and the operator's post-integration frame plus
    // its authored coupling rig — the placement is deferred until the target's
    // own held-response is known, because a formation-kept target rides its own
    // slot rather than the operator's rig.
    let mut rigs: Vec<(String, String, Vec3, Quat, Vec3)> = set
        .p0()
        .iter()
        .filter_map(|(uuid, transform, beam)| {
            let target = beam.coupled_target.as_ref()?;
            Some((
                uuid.0.clone(),
                target.clone(),
                transform.translation,
                transform.rotation,
                Vec3::from(beam.config.coupling_offset),
            ))
        })
        .collect();
    if rigs.is_empty() {
        return;
    }
    rigs.sort_by(|a, b| a.0.cmp(&b.0));

    for (_, target_uuid, op_translation, op_rotation, operator_rig) in rigs {
        let mut targets = set.p1();
        let Some((_, mut transform, physics, held)) = targets
            .iter_mut()
            .find(|(uuid, _, _, _)| uuid.0 == target_uuid)
        else {
            continue;
        };
        // The offset the held target declares — its own formation slot, or the
        // operator's rig for everything else and for a target that authored no
        // held-response at all.
        let offset = match held {
            Some(section) => held_offset(&section.0.resolve(), operator_rig),
            None => operator_rig,
        };
        let placed = coupled_position(op_translation, op_rotation, offset);
        transform.translation = placed;
        if let Some(mut physics) = physics {
            physics.x = placed.x;
            physics.y = placed.y;
            physics.z = placed.z;
            physics.forward_speed = 0.0;
            physics.lateral_speed = 0.0;
            physics.vertical_speed = 0.0;
        }
    }
}

// ── Held-response: arrest-decline (issue #1158) ──────────────────────────────

/// Bank an arrest-decline held target's condition this tick (issue #1158).
///
/// For every ship holding a coupled target whose `[held_response]` resolves to
/// arrest-decline, queue a [`ConditionAdjustment`] on the target's OWN
/// infrastructure condition track. The pure [`condition_delta`] returns a value
/// that cancels the decline the infrastructure tick applies this tick and adds
/// the authored recovery, so the net movement is the authored rate.
///
/// Ordered before `tick_infrastructure_condition` (see [`TractorPlugin`]) so the
/// adjustment lands the SAME tick as the decline it arrests — the ordering the
/// tow/stabilise payoff keeps. Releasing the beam clears `coupled_target`, so
/// this queues nothing and the target's ordinary decline resumes on the very
/// next tick with nothing to arrest it.
///
/// The condition move goes through the queue rather than onto the component so
/// the recovered condition crosses the target's OWN authored thresholds by the
/// one system that owns the flag edges — the same path a scripted repair takes,
/// which is why a structure recovered across `restores_above` sets the
/// operational flag a scenario already reads.
pub fn arrest_held_declines(
    runtime: Option<ResMut<WorldContentRuntime>>,
    time: Option<Res<Time>>,
    operators: Query<&TractorBeam>,
    targets: Query<(&EntityUuid, &HeldResponseSection, &InfrastructureCondition)>,
) {
    // Collect from read-only queries first, so a world with no arrest-decline
    // hold never takes `WorldContentRuntime` mutably and marks it changed on a
    // quiet tick.
    let dt = time.map(|t| t.delta_secs()).unwrap_or(0.0);
    let mut adjustments: Vec<ConditionAdjustment> = operators
        .iter()
        .filter_map(|beam| {
            let target_uuid = beam.coupled_target.as_ref()?;
            let (_, section, condition) =
                targets.iter().find(|(uuid, _, _)| &uuid.0 == target_uuid)?;
            let delta = condition_delta(&section.0.resolve(), condition.0.decay_per_sec(), dt);
            if delta == 0.0 {
                return None;
            }
            Some(ConditionAdjustment {
                uuid: target_uuid.clone(),
                delta,
            })
        })
        .collect();
    if adjustments.is_empty() {
        return;
    }
    // UUID order so two hosts queue identically — the walk-order rule the
    // infrastructure and operations ticks both keep.
    adjustments.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    let Some(mut runtime) = runtime else {
        return;
    };
    runtime.pending_condition_adjustments.extend(adjustments);
}

// ── The wire ─────────────────────────────────────────────────────────────────

/// Publish each tractor-carrying ship's blackboard under its system id (issue
/// #1156).
///
/// Only ships that carry [`TractorBeam`] publish one, so a world whose hulls
/// author no `[tractor]` puts exactly the payload on the wire it did before this
/// existed. No English crosses: the target name is a world entity name id and
/// the refusal is the pure module's `strings.csv` id.
pub fn publish_tractor_blackboard(
    mut ships: Query<(&TractorBeam, &mut crate::server_app::ShipSystemBlackboards)>,
    named: Query<(&EntityUuid, &EntityName)>,
) {
    let key = tractor_system_id();
    for (beam, mut blackboards) in ships.iter_mut() {
        let target_name = beam.coupled_target.as_ref().and_then(|uuid| {
            named
                .iter()
                .find(|(id, _)| &id.0 == uuid)
                .map(|(_, name)| name.0.clone())
        });
        let blackboard = SystemBlackboard::Tractor(TractorBlackboard {
            range: beam.config.range,
            engaged: beam.engaged,
            coupled_target: beam.coupled_target.clone(),
            coupled_target_name: target_name,
            refusal: beam.last_refusal.map(|r| r.string_id().to_string()),
        });
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key.clone(), blackboard);
        }
    }
}

/// The tractor's published blackboard channel key — its system id (issue #1156).
/// A convenience mirror of [`tractor_system_id`] for readers that key off a
/// function, matching `operations::operations_blackboard_key`.
pub fn tractor_blackboard_key() -> SystemId {
    tractor_system_id()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beam() -> TractorBeam {
        TractorBeam::new(
            TractorConfig {
                range: 500.0,
                coupling_offset: [0.0, 0.0, -120.0],
                min_power_level: 2,
            },
            PowerGroupId("tractor".into()),
        )
    }

    #[test]
    fn save_state_carries_engage_and_target_only() {
        let mut b = beam();
        b.engaged = true;
        b.coupled_target = Some("derelict-1".into());
        b.last_refusal = Some(TractorRefusal::OutOfRange);
        let save = b.save_state();
        assert!(save.engaged);
        assert_eq!(save.coupled_target.as_deref(), Some("derelict-1"));
    }

    #[test]
    fn an_idle_beam_saves_as_default() {
        assert_eq!(beam().save_state(), TractorSaveState::default());
    }
}

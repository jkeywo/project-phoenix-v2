//! The Ship-owned saved frontier of Helm controls and Radar target memory.
//!
//! Explicit payloads preserve the existing save layout. Component projection
//! and reinstatement live together here; snapshot orchestration still matches
//! EntityUuid, checks prerequisites and chooses cross-System restore order.
//! No runtime type gains serialization, and this module does not import snapshot.

use crate::console::weapons::beam::{LastShipAttacker, TacticalRadarSelection};
use crate::ship::components::LastHelmInput;
use crate::ship::helm::{
    BoostCommand, ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput,
    VerticalThrustInput,
};
use crate::ship::helm_ai::{
    HelmBoostAiPolicyState, HelmEnginesAiPolicyState, HelmRecoveryHistory,
    HelmSteeringAiPolicyState,
};
use crate::ship::impulse::ImpulsePhase;
use crate::ship::sensors::SensorRadarSelection;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// A ship's **active control state**: what its helm was being told to do.
///
/// Named by this issue's acceptance criteria, and not optional in practice even
/// though `world_digest` does not fold it. `ShipPhysics` records where a ship
/// *is* and how fast it is going; these six axes record what it is being asked
/// to do next, and `integrate_ship_physics` reads them on the very first step
/// after a restore. A resumed ship without them keeps its captured velocity and
/// then immediately coasts, which reads as a divergence one frame after a
/// restore that was otherwise exact — the first thing this slice's continuation
/// test caught.
///
/// Stored as plain scalars rather than by giving `ImpulsePhase`, `ImpulseState`
/// and `BoostState` serde derives. The reason is the type-shape constraint
/// `sim_digest` documents: a `derive` on an enum silently makes its variant
/// *order* stored surface, and a scalar written out at the call site makes that
/// commitment visible where it is made. Three fewer types become save format.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlState {
    pub thrust: f32,
    pub steering: f32,
    pub lateral: f32,
    pub vertical: f32,
    pub boost: bool,
    /// `ImpulsePhase` as `0` = `Idle`, `1` = `Charging`, `2` = `Active`.
    /// Anything else restores as `Idle` rather than panicking — an unknown
    /// phase is a save from a build that had one this one does not, and the
    /// content/format gate is what refuses that, not a `match` arm here.
    pub impulse_phase: u8,
    /// `LastHelmInput`'s `(thrust, steering, lateral)`. Distinct from the three
    /// axes above: those are the *desired* input, this is what the integrator
    /// last actually applied, and the helm AI's rate limiting reads the
    /// difference.
    pub last_helm: [f32; 3],
    /// `TacticalRadarSelection` — the uuid this ship's Tactical radar is locked
    /// on, or `None`.
    ///
    /// Targeting is radar-owned, and the lock is what every downstream decision
    /// hangs off: a restored ship without it has no target, so its helm AI
    /// steers nowhere and its weapons hold fire. That is a whole ship behaving
    /// differently, one tick after a restore whose digest matched exactly —
    /// which is precisely the class of silent gap this payload exists to close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_lock: Option<String>,
    /// `LastShipAttacker` — who last shot this ship, the AI's fallback target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attacker: Option<String>,
    /// `SensorRadarSelection` — the uuid this ship's Sensors radar has locked as
    /// its **Science Target**, or `None`. The sibling of [`Self::target_lock`]
    /// (which is the Tactical radar's Combat Lock): both are per-ship radar
    /// selections a run *chose*, and both feed the ship's own AI through the
    /// frozen viewscreen read surface. `PublishAggregate` lifts this into
    /// `ViewscreenBlackboard::science_target` (issue #829), which
    /// `helm_shared_target_view` and the weapons doctrine both decide from — so a
    /// resumed ship whose Sensors radar came back empty resolves a different
    /// shared target on its first cadence tick and steers differently, the same
    /// silent one-tick divergence `target_lock` was captured to close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_lock: Option<String>,
    /// The three stateful helm policies' runtime state, in the fixed order
    /// `(engines, steering, boost)` — see [`PolicyState`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm_policies: Option<[PolicyState; 3]>,
    /// `HelmRecoveryHistory` — see [`RecoveryHistory`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helm_recovery: Option<RecoveryHistory>,
}

/// The host-side bounded range windows a ship's helm policies read through
/// `fact(safe_distance_held)` and the pressed detector.
///
/// `HelmPolicyRuntime`'s own docs call its five components "one thing", and the
/// payload had three of them. The two it did not have are not alike, and only
/// one of them is state: `HelmPassSurface` is republished from scratch by
/// `ai_policy_state_tick` every AI tick, so a restored ship rebuilds it on its
/// first tick and storing it would store a derivation. These windows are the
/// opposite — they are an *accumulation* over the last N shared AI ticks, and
/// there is no tick on which they are recomputed from the world. A ship
/// restored without them has held its safe distance for zero samples, which is
/// a different answer to a question its transitions are gated on.
///
/// The capacities are authored (`safe_distance_window_ticks`,
/// `pressed_window_ticks`) and re-applied every tick from config, so they are
/// stored only so the samples can be replayed into a window of the right size
/// before the first tick re-authors it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecoveryHistory {
    /// The uuid both windows were measured against. A target switch clears
    /// them, so restoring the samples without the identity they belong to
    /// would credit a new threat with distance held against the old one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The level window (`safe_distance_held`), oldest sample first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<f64>,
    pub ranges_capacity: u32,
    /// The trend window (the pressed detector), oldest sample first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub separation: Vec<f64>,
    pub separation_capacity: u32,
}

/// One stateful AI policy's runtime state (issue #882's `AiPolicyRuntimeState`).
///
/// Captured because a cold policy runtime is a ship that behaves differently:
/// the state id and `entered_at_secs` are what `state_time` is measured
/// against, so a restored ship whose policy was reset evaluates every
/// time-gated transition from zero and takes a different branch on the first
/// tick after the restore. That is the second silent gap this slice's
/// continuation test caught, after the helm axes.
///
/// Written out field-by-field rather than by deriving serde on
/// `AiPolicyRuntimeState` itself, for the reason [`ControlState`] gives:
/// `AiPolicyMemory` already carries serde (added for this payload), and the two
/// scalars beside it do not need a third type's shape pinned as save format to
/// travel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyState {
    /// The currently-entered state id.
    pub current: String,
    /// The tick-derived clock reading `current` was entered at.
    pub entered_at_secs: f64,
    /// The fine system's typed private memory.
    pub memory: crate::world::flags::AiPolicyMemory,
}

impl ControlState {
    /// An absent Helm is not a centred stick: only an entity carrying thrust
    /// has a saved control frontier. Optional sibling components retain their
    /// existing default/absence semantics, including the Sensors target.
    pub fn capture_from(entity: EntityRef<'_>) -> Option<Self> {
        let thrust = entity.get::<ThrustInput>()?;
        Some(Self {
            thrust: thrust.0,
            steering: entity.get::<SteeringInput>().map_or(0.0, |s| s.0),
            lateral: entity.get::<LateralThrustInput>().map_or(0.0, |s| s.0),
            vertical: entity.get::<VerticalThrustInput>().map_or(0.0, |s| s.0),
            boost: entity.get::<BoostCommand>().is_some_and(|s| s.0),
            impulse_phase: entity.get::<ImpulseCommand>().map_or(0, |s| match s.0 {
                ImpulsePhase::Idle => 0,
                ImpulsePhase::Charging => 1,
                ImpulsePhase::Active => 2,
            }),
            last_helm: entity
                .get::<LastHelmInput>()
                .map_or([0.0; 3], |s| [s.thrust, s.steering, s.lateral]),
            target_lock: entity
                .get::<TacticalRadarSelection>()
                .and_then(|s| s.0.clone()),
            last_attacker: entity.get::<LastShipAttacker>().and_then(|s| s.0.clone()),
            sensor_lock: entity
                .get::<SensorRadarSelection>()
                .and_then(|s| s.0.clone()),
            helm_policies: Some([
                policy_state(entity.get::<HelmEnginesAiPolicyState>().map(|s| &s.0)),
                policy_state(entity.get::<HelmSteeringAiPolicyState>().map(|s| &s.0)),
                policy_state(entity.get::<HelmBoostAiPolicyState>().map(|s| &s.0)),
            ]),
            helm_recovery: entity
                .get::<HelmRecoveryHistory>()
                .map(|r| RecoveryHistory {
                    target: r.target.map(|t| t.to_string()),
                    ranges: r.ranges.iter().collect(),
                    ranges_capacity: r.ranges.capacity() as u32,
                    separation: r.separation.iter().collect(),
                    separation_capacity: r.separation.capacity() as u32,
                }),
        })
    }

    /// Replace present components, including saved default/cleared values.
    /// Component installation remains the bootstrap/LOD owner's responsibility.
    pub fn restore_into(&self, entity_mut: &mut EntityWorldMut<'_>) {
        let control = self;
        // The drive consumer also accepts Changed direct-intent writes. A
        // captured command is already consumed, even if a bootstrap command
        // was changed just before restore. Rebase that edge for every reader;
        // a subsequent real producer write still stamps the current tick.
        let consumed = bevy::ecs::change_detection::Tick::new(
            entity_mut
                .world()
                .read_change_tick()
                .get()
                .wrapping_sub(bevy::ecs::change_detection::MAX_CHANGE_AGE),
        );
        // Restored intent values are continuation, not a fresh producer write.
        if let Some(mut writes) = entity_mut.get_mut::<crate::ship::helm::DriveCommandWrites>() {
            *writes = Default::default();
        }
        if let Some(mut thrust) = entity_mut.get_mut::<ThrustInput>() {
            thrust.0 = control.thrust;
        }
        if let Some(mut steering) = entity_mut.get_mut::<SteeringInput>() {
            steering.0 = control.steering;
        }
        if let Some(mut lateral) = entity_mut.get_mut::<LateralThrustInput>() {
            lateral.0 = control.lateral;
        }
        if let Some(mut vertical) = entity_mut.get_mut::<VerticalThrustInput>() {
            vertical.0 = control.vertical;
        }
        if let Some(mut boost) = entity_mut.get_mut::<BoostCommand>() {
            boost.bypass_change_detection().0 = control.boost;
            boost.set_last_changed(consumed);
        }
        if let Some(mut impulse) = entity_mut.get_mut::<ImpulseCommand>() {
            impulse.bypass_change_detection().0 = match control.impulse_phase {
                1 => ImpulsePhase::Charging,
                2 => ImpulsePhase::Active,
                // Including anything this build does not recognise — see
                // `ControlState::impulse_phase`.
                _ => ImpulsePhase::Idle,
            };
            impulse.set_last_changed(consumed);
        }
        if let Some(mut last) = entity_mut.get_mut::<LastHelmInput>() {
            last.thrust = control.last_helm[0];
            last.steering = control.last_helm[1];
            last.lateral = control.last_helm[2];
        }
        if let Some(policies) = &control.helm_policies {
            if let Some(mut state) = entity_mut.get_mut::<HelmEnginesAiPolicyState>() {
                apply_policy_state(&mut state.0, &policies[0]);
            }
            if let Some(mut state) = entity_mut.get_mut::<HelmSteeringAiPolicyState>() {
                apply_policy_state(&mut state.0, &policies[1]);
            }
            if let Some(mut state) = entity_mut.get_mut::<HelmBoostAiPolicyState>() {
                apply_policy_state(&mut state.0, &policies[2]);
            }
        }
        if let Some(stored) = &control.helm_recovery {
            if let Some(mut history) =
                entity_mut.get_mut::<crate::ship::helm_ai::HelmRecoveryHistory>()
            {
                history.target = stored
                    .target
                    .as_deref()
                    .and_then(|t| uuid::Uuid::parse_str(t).ok());
                history.ranges.set_capacity(stored.ranges_capacity as usize);
                history.ranges.clear();
                for sample in &stored.ranges {
                    history.ranges.push(*sample);
                }
                history
                    .separation
                    .set_capacity(stored.separation_capacity as usize);
                history.separation.clear();
                for sample in &stored.separation {
                    history.separation.push(*sample);
                }
            }
        }
        if let Some(mut lock) = entity_mut.get_mut::<TacticalRadarSelection>() {
            lock.0 = control.target_lock.clone();
        }
        if let Some(mut sensor_lock) =
            entity_mut.get_mut::<crate::ship::sensors::SensorRadarSelection>()
        {
            sensor_lock.0 = control.sensor_lock.clone();
        }
        if let Some(mut attacker) = entity_mut.get_mut::<LastShipAttacker>() {
            // `set_if_neq` semantics matter here: `LastShipAttacker`'s
            // change detection is the rising-edge latch behind
            // `on_entity_attacked` triggers, so a blind write on restore
            // would re-fire a scenario trigger the capture had already
            // spent.
            let restored = control.last_attacker.clone();
            if attacker.0 != restored {
                attacker.0 = restored;
            }
        }
    }
}

fn policy_state(runtime: Option<&crate::ai::policy::AiPolicyRuntimeState>) -> PolicyState {
    runtime.map_or_else(PolicyState::default, |r| PolicyState {
        current: r.current.clone(),
        entered_at_secs: r.entered_at_secs,
        memory: r.memory.clone(),
    })
}

fn apply_policy_state(runtime: &mut crate::ai::policy::AiPolicyRuntimeState, stored: &PolicyState) {
    runtime.current = stored.current.clone();
    runtime.entered_at_secs = stored.entered_at_secs;
    runtime.memory = stored.memory.clone();
}

#[cfg(test)]
#[path = "continuation_tests.rs"]
mod tests;

//! Fixed-tick adapter for peer-local save-slot capture (issue #865).
//!
//! [`crate::save_slots`] owns the pure scheduling decisions. This module owns
//! the cross-target Bevy resources that feed those decisions and turns every
//! due decision into a [`crate::snapshot::StoredRun`]. Storage deliberately
//! happens later: [`PendingStoredRuns`] is a local outbox, so quota failures or
//! a slow backend can neither delay a logical tick nor enter the digest/mesh.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::core::messages::GamePhase;
use crate::save_slots::{
    CaptureDecision, CaptureReason, CaptureSlot, ManualSaveRefusalReason, RefusedManualSave,
    SavePhase, SaveSchedule,
};

/// The authored scenario path copied from the shared boot plan.
///
/// It is artifact metadata, not simulation authority. Keeping it as a normal
/// resource gives browser, native, and headless composition the same capture
/// seam without reaching into a target-specific bridge.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveScenario(pub String);

/// A native target has installed a consumer for [`PendingStoredRuns`].
///
/// Browser builds have their canonical bridge consumer and do not need this
/// marker. Native/headless compositions insert it only when a Store or another
/// explicit adapter is installed. The scheduler still advances without one,
/// but it skips the expensive snapshot/digest walk and never grows an outbox a
/// headless or balance run will not drain.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveCaptureConsumer;

/// Manual requests consumed without a snapshot, awaiting a target-local UI or
/// Store adapter.
///
/// Browser code removes the matching capture token from its pending-intent map;
/// native code clears its reserved display name and records a local outcome.
/// This FIFO is presentation state and never enters digest or mesh state.
#[derive(Resource, Debug, Default)]
pub struct RefusedManualSaves {
    outcomes: VecDeque<RefusedManualSave>,
}

impl RefusedManualSaves {
    pub fn pop_front(&mut self) -> Option<RefusedManualSave> {
        self.outcomes.pop_front()
    }

    pub fn len(&self) -> usize {
        self.outcomes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.outcomes.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &RefusedManualSave> {
        self.outcomes.iter()
    }

    fn extend(&mut self, outcomes: impl IntoIterator<Item = RefusedManualSave>) {
        self.outcomes.extend(outcomes);
    }
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StartupRestoreCaptureGate {
    pending: bool,
}

/// Peer-local manual requests waiting to be assigned to their next fixed tick.
///
/// Every call appends a distinct entry. In particular, requesting the same
/// slot twice is not coalesced or overwritten.
#[derive(Resource, Debug, Default)]
pub struct ManualSaveRequests {
    slots: VecDeque<String>,
}

impl ManualSaveRequests {
    pub fn request(&mut self, slot_id: impl Into<String>) {
        self.slots.push_back(slot_id.into());
    }

    fn take_all(&mut self) -> VecDeque<String> {
        std::mem::take(&mut self.slots)
    }
}

/// Queue one peer-local manual capture without knowing the adapter resource's
/// internal representation.
pub fn request_manual_save(world: &mut World, slot_id: impl Into<String>) {
    world
        .get_resource_or_insert_with(ManualSaveRequests::default)
        .request(slot_id);
}

/// One captured record awaiting a target-local storage adapter.
#[derive(Debug)]
pub struct PendingStoredRun {
    pub decision: CaptureDecision,
    pub run: crate::snapshot::StoredRun,
}

/// FIFO capture results. Draining this queue is intentionally outside the
/// fixed schedule and outside the authoritative digest.
#[derive(Resource, Debug, Default)]
pub struct PendingStoredRuns {
    runs: VecDeque<PendingStoredRun>,
}

impl PendingStoredRuns {
    pub(crate) fn push_back(&mut self, run: PendingStoredRun) {
        self.runs.push_back(run);
    }

    pub fn pop_front(&mut self) -> Option<PendingStoredRun> {
        self.runs.pop_front()
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &PendingStoredRun> {
        self.runs.iter()
    }

    fn clear(&mut self) {
        self.runs.clear();
    }
}

#[derive(Resource, Debug, Default)]
struct SaveLifecycleState(SaveSchedule);

/// Suppress every lifecycle capture while a fresh App bootstraps a staged run.
///
/// Call this only after the selected record has passed its full compatibility
/// gate. It also discards any already-built bootstrap artifact and refuses all
/// pending manual intents exactly once.
pub fn begin_startup_restore(world: &mut World) {
    ensure_lifecycle_resources(world);
    world.resource_mut::<StartupRestoreCaptureGate>().pending = true;
    let tick = continuation_tick(world);
    refuse_all_manual(world, tick, ManualSaveRefusalReason::StartupRestorePending);
    world.resource_mut::<PendingStoredRuns>().clear();
}

/// Re-enable capture after a successful restore and rebase periodic cadence.
///
/// `continuation_tick` is the restored `SimTick`: the current phase is marked
/// observed, so no bootstrap/duplicate `RunStarted` is written, and the next
/// periodic decision is exactly `continuation_tick + interval_ticks`.
pub fn complete_startup_restore(world: &mut World, continuation_tick: u64) {
    ensure_lifecycle_resources(world);
    refuse_all_manual(
        world,
        continuation_tick,
        ManualSaveRefusalReason::StartupRestorePending,
    );
    world.resource_mut::<PendingStoredRuns>().clear();
    let phase = current_save_phase(world);
    world
        .resource_mut::<SaveLifecycleState>()
        .0
        .rebase_after_restore(continuation_tick, phase);
    world.resource_mut::<StartupRestoreCaptureGate>().pending = false;
}

/// Abandon a staged restore without allowing its bootstrap captures to leak.
///
/// The current continuation becomes the cadence origin if the fresh session is
/// already live; a pre-run cancellation leaves the ordinary first-run capture
/// available when the mission eventually starts.
/// This also resolves failed post-application verification: it does not undo
/// snapshot or layer changes already applied to the World.
pub fn cancel_startup_restore(world: &mut World) {
    ensure_lifecycle_resources(world);
    let tick = continuation_tick(world);
    refuse_all_manual(world, tick, ManualSaveRefusalReason::StartupRestorePending);
    world.resource_mut::<PendingStoredRuns>().clear();
    let phase = current_save_phase(world);
    world
        .resource_mut::<SaveLifecycleState>()
        .0
        .rebase_after_restore(tick, phase);
    world.resource_mut::<StartupRestoreCaptureGate>().pending = false;
}

/// Whether lifecycle capture is currently suspended for startup restore.
pub fn startup_restore_pending(world: &World) -> bool {
    world
        .get_resource::<StartupRestoreCaptureGate>()
        .is_some_and(|gate| gate.pending)
}

/// Register the one cross-target lifecycle adapter.
///
/// The exclusive system runs in `FixedLast`: all committed [`crate::sim_sets::SimSet`]
/// work (and the fixed state transition) has completed and
/// [`crate::sim_tick::advance_sim_tick`] has moved `SimTick` onto the next step.
/// Decisions retain the just-committed step label `T`; the stored continuation
/// starts at `T + 1`, so restore cannot execute `T` twice.
pub fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};

    crate::sim_tick::register_sim_tick(app);
    app.init_resource::<SaveLifecycleState>()
        .init_resource::<ManualSaveRequests>()
        .init_resource::<PendingStoredRuns>()
        .init_resource::<RefusedManualSaves>()
        .init_resource::<StartupRestoreCaptureGate>()
        .declare_state::<SaveLifecycleState>(StateClass::Timer, "t2-peer-local-save-slot-lifecycle")
        .declare_state::<ManualSaveRequests>(StateClass::Timer, "t2-peer-local-save-slot-lifecycle")
        .declare_state::<PendingStoredRuns>(StateClass::Timer, "t2-peer-local-save-slot-lifecycle")
        .declare_state::<RefusedManualSaves>(StateClass::Timer, "t2-peer-local-save-slot-lifecycle")
        .declare_state::<StartupRestoreCaptureGate>(
            StateClass::Timer,
            "t2-peer-local-save-slot-lifecycle",
        )
        .declare_state::<SaveScenario>(StateClass::Derived, "t2-peer-local-save-slot-lifecycle")
        .add_systems(
            FixedLast,
            capture_due_runs.after(crate::sim_tick::advance_sim_tick),
        );
}

fn capture_due_runs(world: &mut World) {
    let continuation_tick = continuation_tick(world);
    let decision_tick = continuation_tick.wrapping_sub(1);
    if startup_restore_pending(world) {
        let manual = world.resource_mut::<ManualSaveRequests>().take_all();
        {
            let mut lifecycle = world.resource_mut::<SaveLifecycleState>();
            lifecycle.0.queue_manual_for_tick(decision_tick, manual);
        }
        refuse_all_manual(
            world,
            decision_tick,
            ManualSaveRefusalReason::StartupRestorePending,
        );
        world.resource_mut::<PendingStoredRuns>().clear();
        return;
    }
    // Read after the fixed-loop StateTransition schedule. A request may have
    // entered this rendered frame while the run was live, then crossed a
    // GameOver or ReturnToLobby boundary during FixedUpdate.
    let phase = current_save_phase(world);
    let interval_ticks = world
        .get_resource::<crate::world::config::WorldConfig>()
        .and_then(|world| world.global.checked_autosave_interval_ticks())
        .or_else(|| {
            crate::entities::config::GlobalConfig::default().checked_autosave_interval_ticks()
        })
        .unwrap_or(0);

    let manual = world.resource_mut::<ManualSaveRequests>().take_all();
    let output = {
        let mut lifecycle = world.resource_mut::<SaveLifecycleState>();
        // Requests enter between fixed steps. After `advance_sim_tick`, the
        // just-committed step is one behind `SimTick`; that is their next
        // deterministic boundary and remains the human-facing decision label.
        lifecycle.0.queue_manual_for_tick(decision_tick, manual);
        lifecycle.0.step_with_outcomes(
            decision_tick,
            phase,
            interval_ticks,
            std::iter::empty::<String>(),
        )
    };
    world
        .resource_mut::<RefusedManualSaves>()
        .extend(output.refused_manual);
    let decisions = output.decisions;
    if decisions.is_empty() {
        return;
    }
    if !has_capture_consumer(world) {
        // Scheduling is deliberately not conditional on persistence: the
        // first/periodic/final latches above have advanced exactly as they do
        // for a persisted run. Only the artifact walk and queue are omitted.
        return;
    }

    let Some(scenario) = world
        .get_resource::<SaveScenario>()
        .map(|source| source.0.clone())
        .filter(|path| !path.is_empty())
    else {
        // Bare-App fixtures and pre-boot assemblies remain safe. A real boot
        // always supplies this resource before canonical sim registration.
        return;
    };

    let versions = crate::snapshot::versions(&crate::content_ledger::frozen_or_live());
    let seed = world
        .get_resource::<crate::sim_rng::SimRng>()
        .map_or(0, |rng| rng.seed());
    let mut captured = VecDeque::with_capacity(decisions.len());
    for decision in decisions {
        // The pure scheduler already applies this rule. Keep the adapter-side
        // guard at the actual snapshot site as the invariant's final line: no
        // manual decision may ever serialize a non-live phase.
        if decision.reason == CaptureReason::Manual
            && current_save_phase(world) != SavePhase::InProgress
        {
            let CaptureSlot::Manual(slot_id) = decision.slot else {
                unreachable!("manual reason always carries a manual slot")
            };
            let phase = current_save_phase(world);
            world
                .resource_mut::<RefusedManualSaves>()
                .extend([RefusedManualSave {
                    tick: decision.tick,
                    slot_id,
                    reason: ManualSaveRefusalReason::PhaseChanged { phase },
                }]);
            continue;
        }
        let mut payload = crate::snapshot::capture(world);
        debug_assert_eq!(payload.tick, decision.tick.wrapping_add(1));
        // Inside a multi-step rendered frame Bevy's fixed overstep still holds
        // the remaining catch-up steps. That is frame batching, not
        // authoritative continuation state. Every lifecycle save is taken at a
        // committed fixed boundary, so serialize that boundary canonically.
        payload.fixed_overstep_nanos = Some(0);
        let digest = crate::sim_digest::world_digest(world);
        let run =
            crate::snapshot::run_for(payload, digest, seed, scenario.clone(), versions.clone());
        captured.push_back(PendingStoredRun { decision, run });
    }
    world
        .resource_mut::<PendingStoredRuns>()
        .runs
        .append(&mut captured);
}

fn ensure_lifecycle_resources(world: &mut World) {
    world.get_resource_or_insert_with(SaveLifecycleState::default);
    world.get_resource_or_insert_with(ManualSaveRequests::default);
    world.get_resource_or_insert_with(PendingStoredRuns::default);
    world.get_resource_or_insert_with(RefusedManualSaves::default);
    world.get_resource_or_insert_with(StartupRestoreCaptureGate::default);
}

fn continuation_tick(world: &World) -> u64 {
    world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0)
}

fn refuse_all_manual(world: &mut World, tick: u64, reason: ManualSaveRefusalReason) {
    let ingress = world.resource_mut::<ManualSaveRequests>().take_all();
    let refusals = {
        let mut lifecycle = world.resource_mut::<SaveLifecycleState>();
        lifecycle.0.queue_manual_for_tick(tick, ingress);
        lifecycle.0.refuse_pending_manual(tick, reason)
    };
    world.resource_mut::<RefusedManualSaves>().extend(refusals);
}

fn current_save_phase(world: &World) -> SavePhase {
    world
        .get_resource::<State<GamePhase>>()
        .map_or(SavePhase::BeforeRun, |phase| match phase.get() {
            GamePhase::InProgress => SavePhase::InProgress,
            GamePhase::GameOver => SavePhase::GameOver,
            GamePhase::Lobby | GamePhase::Loading => SavePhase::BeforeRun,
        })
}

#[cfg(target_arch = "wasm32")]
fn has_capture_consumer(world: &World) -> bool {
    // Live browser hosts drain to LocalStorage. Disposable Test installs no
    // persistence flush and must not retain undeliverable snapshot artifacts.
    !world.contains_resource::<crate::workshop::test_clock::DisposableTest>()
}

#[cfg(not(target_arch = "wasm32"))]
fn has_capture_consumer(world: &World) -> bool {
    world.contains_resource::<SaveCaptureConsumer>()
        && !world.contains_resource::<crate::workshop::test_clock::DisposableTest>()
}

#[cfg(test)]
#[path = "save_slots_lifecycle_tests.rs"]
mod tests;

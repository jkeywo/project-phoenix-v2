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
fn has_capture_consumer(_world: &World) -> bool {
    // The browser host's canonical bridge always drains PendingStoredRuns to
    // LocalStorage after fixed updates.
    true
}

#[cfg(not(target_arch = "wasm32"))]
fn has_capture_consumer(world: &World) -> bool {
    world.contains_resource::<SaveCaptureConsumer>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;
    use bevy::time::{TimePlugin, TimeUpdateStrategy};

    const TEST_TICK_HZ: f32 = 4.0;
    const PERIOD: std::time::Duration = std::time::Duration::from_millis(10);

    #[derive(Resource)]
    struct FixedPhaseBoundary(Option<GamePhase>);

    fn apply_fixed_phase_boundary(
        mut boundary: ResMut<FixedPhaseBoundary>,
        mut next_phase: ResMut<NextState<GamePhase>>,
    ) {
        if let Some(phase) = boundary.0.take() {
            next_phase.set(phase);
        }
    }

    fn test_app_with_consumer(interval_ticks: u64, consumer: bool) -> App {
        let mut app = App::new();
        app.add_plugins(TimePlugin)
            .add_plugins(StatesPlugin)
            .insert_state(GamePhase::InProgress)
            .insert_resource(SaveScenario("assets/worlds/test.toml".into()));
        let mut config = crate::world::config::WorldConfig::default();
        config.global.sim_tick_hz = TEST_TICK_HZ;
        config.global.autosave_interval_secs = interval_ticks as f32 / TEST_TICK_HZ;
        app.insert_resource(config);
        register(&mut app);
        if consumer {
            app.insert_resource(SaveCaptureConsumer);
        }
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(PERIOD);
        app
    }

    fn test_app(interval_ticks: u64) -> App {
        test_app_with_consumer(interval_ticks, true)
    }

    fn run_ticks(app: &mut App, ticks_per_frame: u32, total_ticks: u64) {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(PERIOD * ticks_per_frame));
        app.update(); // establishes the zero-delta baseline
        for _ in 0..(total_ticks / u64::from(ticks_per_frame)) {
            app.update();
        }
        assert_eq!(
            app.world().resource::<crate::sim_tick::SimTick>().0,
            total_ticks
        );
    }

    fn captured_ticks(app: &App) -> Vec<(u64, crate::save_slots::CaptureReason)> {
        app.world()
            .resource::<PendingStoredRuns>()
            .iter()
            .map(|pending| {
                assert_eq!(
                    pending.run.ledger.final_tick,
                    pending.decision.tick.wrapping_add(1)
                );
                assert_eq!(
                    pending.run.snapshot.as_ref().map(|snapshot| snapshot.tick),
                    Some(pending.decision.tick.wrapping_add(1))
                );
                assert_eq!(
                    pending
                        .run
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.state.fixed_overstep_nanos),
                    Some(0)
                );
                (pending.decision.tick, pending.decision.reason)
            })
            .collect()
    }

    fn captured_artifacts(app: &App) -> Vec<String> {
        app.world()
            .resource::<PendingStoredRuns>()
            .iter()
            .map(|pending| crate::snapshot::export_artifact(&pending.run).unwrap())
            .collect()
    }

    #[test]
    fn automatic_capture_ticks_ignore_rendered_frame_batching() {
        let mut per_tick = test_app(5);
        let mut per_four = test_app(5);

        run_ticks(&mut per_tick, 1, 12);
        run_ticks(&mut per_four, 4, 12);

        assert_eq!(captured_ticks(&per_tick), captured_ticks(&per_four));
        assert_eq!(
            captured_artifacts(&per_tick),
            captured_artifacts(&per_four),
            "frame batching must not enter the stored artifact"
        );
        assert_eq!(
            captured_ticks(&per_tick),
            vec![
                (0, crate::save_slots::CaptureReason::RunStarted),
                (5, crate::save_slots::CaptureReason::Periodic),
                (10, crate::save_slots::CaptureReason::Periodic),
            ]
        );

        let saved = per_tick
            .world()
            .resource::<PendingStoredRuns>()
            .iter()
            .find(|pending| pending.decision.tick == 10)
            .unwrap()
            .run
            .clone();
        let saved_snapshot = saved.snapshot.as_ref().unwrap();
        let mut boundary = test_app(5);
        run_ticks(&mut boundary, 1, 11);
        assert_eq!(
            crate::sim_digest::world_digest(boundary.world()),
            saved_snapshot.digest,
            "the capture fold must equal an independently paced T+1 boundary: {:?}",
            crate::sim_digest::digest_stages(boundary.world())
        );
        let mut resumed = test_app(5);
        resumed.insert_resource(TimeUpdateStrategy::ManualDuration(PERIOD));
        resumed.update(); // establish the zero-delta baseline before restore
        let report = crate::snapshot::restore(resumed.world_mut(), &saved_snapshot.state);
        assert!(report.is_complete());
        assert_eq!(
            resumed.world().resource::<crate::sim_tick::SimTick>().0,
            11,
            "restore starts at T+1"
        );
        resumed.update(); // execute tick 11 once, reaching the source at 12
        assert_eq!(resumed.world().resource::<crate::sim_tick::SimTick>().0, 12);
        // This deliberately partial fixture lacks the empty asteroid/collision
        // resources a restore creates, and `world_digest` distinguishes absent
        // from present-empty. The full-world digest/continuation proof lives in
        // `tests/save_slots_persistence.rs`; here the exact tick boundary and
        // byte-identical artifacts are the useful unit-level assertions.
    }

    #[test]
    fn no_consumer_advances_the_schedule_without_capturing_or_growing_a_queue() {
        let mut app = test_app_with_consumer(5, false);
        run_ticks(&mut app, 1, 1_000);
        assert!(app.world().resource::<PendingStoredRuns>().is_empty());

        // Installing a consumer later does not replay run-start or any missed
        // periodic captures. The already-advanced schedule emits only the next
        // decision that is genuinely due.
        app.insert_resource(SaveCaptureConsumer);
        app.update();
        let pending = app.world().resource::<PendingStoredRuns>();
        assert_eq!(pending.len(), 1);
        let captured = pending.iter().next().unwrap();
        assert_eq!(captured.decision.tick, 1_000);
        assert_eq!(
            captured.decision.reason,
            crate::save_slots::CaptureReason::Periodic
        );
        assert_eq!(captured.run.ledger.final_tick, 1_001);
    }

    #[test]
    fn manual_requests_are_peer_local_next_tick_and_duplicates_survive() {
        let mut requesting_peer = test_app(100);
        let mut other_peer = test_app(100);
        run_ticks(&mut requesting_peer, 1, 1);
        run_ticks(&mut other_peer, 1, 1);

        request_manual_save(requesting_peer.world_mut(), "manual-a");
        request_manual_save(requesting_peer.world_mut(), "manual-a");
        requesting_peer.update();
        other_peer.update();
        let requesting = requesting_peer.world().resource::<PendingStoredRuns>();
        assert_eq!(requesting.len(), 3);
        assert_eq!(
            requesting
                .iter()
                .skip(1)
                .map(|pending| (pending.decision.tick, &pending.decision.slot))
                .collect::<Vec<_>>(),
            vec![
                (
                    1,
                    &crate::save_slots::CaptureSlot::Manual("manual-a".into())
                ),
                (
                    1,
                    &crate::save_slots::CaptureSlot::Manual("manual-a".into())
                ),
            ]
        );
        assert_eq!(
            other_peer.world().resource::<PendingStoredRuns>().len(),
            1,
            "a local manual request must not enter another peer's queue"
        );
    }

    #[test]
    fn fixed_phase_boundary_refuses_manual_capture_before_snapshotting() {
        for boundary in [GamePhase::GameOver, GamePhase::Lobby] {
            let mut app = test_app(100);
            app.insert_resource(FixedPhaseBoundary(None))
                .add_systems(FixedUpdate, apply_fixed_phase_boundary);
            run_ticks(&mut app, 1, 1);
            while app
                .world_mut()
                .resource_mut::<PendingStoredRuns>()
                .pop_front()
                .is_some()
            {}

            // Ingress observes a live run. During that request's fixed step,
            // the production-shaped fixed StateTransition applies the terminal
            // or ReturnToLobby boundary before FixedLast capture.
            request_manual_save(app.world_mut(), "manual-at-boundary");
            app.world_mut().resource_mut::<FixedPhaseBoundary>().0 = Some(boundary.clone());
            app.update();

            let pending = app.world().resource::<PendingStoredRuns>();
            assert!(pending
                .iter()
                .all(|capture| capture.decision.reason != CaptureReason::Manual));
            assert!(pending.iter().all(|capture| {
                capture
                    .run
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.state.phase != Some(GamePhase::Lobby))
            }));
            match boundary {
                GamePhase::GameOver => {
                    let final_capture = pending.iter().collect::<Vec<_>>();
                    assert_eq!(final_capture.len(), 1);
                    assert_eq!(final_capture[0].decision.reason, CaptureReason::GameOver);
                    assert_eq!(
                        final_capture[0]
                            .run
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.state.phase.clone()),
                        Some(GamePhase::GameOver)
                    );
                }
                GamePhase::Lobby => assert!(pending.is_empty()),
                GamePhase::Loading | GamePhase::InProgress => unreachable!(),
            }
            let refused = app
                .world()
                .resource::<RefusedManualSaves>()
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                refused,
                vec![RefusedManualSave {
                    tick: 1,
                    slot_id: "manual-at-boundary".into(),
                    reason: ManualSaveRefusalReason::PhaseChanged {
                        phase: match boundary {
                            GamePhase::GameOver => SavePhase::GameOver,
                            GamePhase::Lobby => SavePhase::BeforeRun,
                            GamePhase::Loading | GamePhase::InProgress => unreachable!(),
                        },
                    },
                }]
            );
        }
    }

    #[test]
    fn startup_restore_gate_discards_bootstrap_and_rebases_cadence() {
        let mut app = test_app(5);
        begin_startup_restore(app.world_mut());
        request_manual_save(app.world_mut(), "manual-during-restore");

        run_ticks(&mut app, 1, 12);
        assert!(startup_restore_pending(app.world()));
        assert!(app.world().resource::<PendingStoredRuns>().is_empty());
        assert_eq!(
            app.world()
                .resource::<RefusedManualSaves>()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![RefusedManualSave {
                tick: 0,
                slot_id: "manual-during-restore".into(),
                reason: ManualSaveRefusalReason::StartupRestorePending,
            }]
        );

        // snapshot::restore writes SimTick before this API is called in
        // production. Reproduce that continuation boundary directly here.
        app.insert_resource(crate::sim_tick::SimTick(20));
        complete_startup_restore(app.world_mut(), 20);
        assert!(!startup_restore_pending(app.world()));
        for _ in 0..5 {
            app.update();
        }
        assert!(app.world().resource::<PendingStoredRuns>().is_empty());
        app.update();
        let pending = app.world().resource::<PendingStoredRuns>();
        assert_eq!(pending.len(), 1);
        let first = pending.iter().next().unwrap();
        assert_eq!(first.decision.tick, 25);
        assert_eq!(first.decision.reason, CaptureReason::Periodic);
        assert_eq!(first.run.ledger.final_tick, 26);
    }

    #[test]
    fn draining_or_failing_storage_cannot_change_simulation_state() {
        let mut app = test_app(100);
        run_ticks(&mut app, 1, 1);
        let digest_before = crate::sim_digest::world_digest(app.world());
        let tick_before = app.world().resource::<crate::sim_tick::SimTick>().0;

        let pending = app
            .world_mut()
            .resource_mut::<PendingStoredRuns>()
            .pop_front()
            .expect("run-start capture should be waiting");
        let fake_storage_result: Result<(), &str> = Err("quota exceeded");
        drop((pending, fake_storage_result));

        assert_eq!(crate::sim_digest::world_digest(app.world()), digest_before);
        assert_eq!(
            app.world().resource::<crate::sim_tick::SimTick>().0,
            tick_before
        );
        assert!(app.world().resource::<PendingStoredRuns>().is_empty());
    }
}

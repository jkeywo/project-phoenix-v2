use super::*;

/// One counted tick per fixed step, none on frames whose accumulated time
/// stays under the timestep, and catch-up frames count every step they run.
#[test]
fn sim_tick_counts_fixed_steps_not_frames() {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    register_sim_tick(&mut app);

    let period = std::time::Duration::from_millis(10);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);

    // Frame 0 establishes the time baseline (zero delta): no step.
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, 0);

    // A frame of exactly half a period: still no step.
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period / 2));
    app.update();
    assert_eq!(
        app.world().resource::<SimTick>().0,
        0,
        "a frame shorter than the timestep must not advance the logical tick"
    );

    // The second half arrives: exactly one step.
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, 1);

    // A long frame of three periods: three catch-up steps in one frame.
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 3));
    app.update();
    assert_eq!(
        app.world().resource::<SimTick>().0,
        4,
        "a frame spanning several periods must run (and count) every step"
    );
}

/// The reconciler applies the authored `[global] sim_tick_hz`, and leaves
/// apps without a `WorldConfig` alone.
#[test]
fn fixed_timestep_follows_the_authored_rate() {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.add_systems(First, reconcile_fixed_timestep);

    // No WorldConfig: whatever the harness set stands.
    let harness_period = std::time::Duration::from_millis(200);
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(harness_period);
    app.update();
    assert_eq!(
        app.world().resource::<Time<Fixed>>().timestep(),
        harness_period,
        "without a WorldConfig the reconciler must not touch the timestep"
    );

    // With one: the authored rate wins.
    let mut cfg = crate::world::config::WorldConfig::default();
    cfg.global.sim_tick_hz = 100.0;
    app.insert_resource(cfg);
    app.update();
    assert_eq!(
        app.world().resource::<Time<Fixed>>().timestep(),
        sim_tick_period(100.0),
        "an authored sim_tick_hz must reconfigure Time<Fixed>"
    );
}

/// A `GamePhase` change written from `FixedUpdate` lands on a TICK
/// boundary, and the same schedule crosses it identically whether the host
/// runs one fixed step per frame or four.
///
/// This is the frame-pacing hole [`register_fixed_state_transition`]
/// closes. With `StateTransition` left frame-only, a four-step frame
/// applies the change at the START of the NEXT frame, so the three steps
/// after the writer still run under the stale phase — the number of ticks
/// spent in the old phase, and the tick `OnEnter` spawns land on, would
/// then be a function of the frame rate.
#[test]
fn phase_transitions_land_on_the_same_tick_whatever_the_frame_pacing() {
    use crate::core::messages::GamePhase;

    /// The step the writer flips the phase on, and the total steps driven.
    const SWITCH_ON: u64 = 5;
    const STEPS: u64 = 12;

    #[derive(Resource, Default, Debug, PartialEq, Eq)]
    struct Crossing {
        /// The tick `OnEnter(InProgress)` observed.
        entered_on: Option<u64>,
        /// Fixed steps that ran under `in_state(InProgress)`.
        steps_in_progress: u64,
    }

    fn flip_on_the_fifth_tick(tick: Res<SimTick>, mut next: ResMut<NextState<GamePhase>>) {
        if tick.0 == SWITCH_ON {
            next.set(GamePhase::InProgress);
        }
    }

    fn record_enter(tick: Res<SimTick>, mut crossing: ResMut<Crossing>) {
        crossing.entered_on.get_or_insert(tick.0);
    }

    fn count_in_progress_steps(mut crossing: ResMut<Crossing>) {
        crossing.steps_in_progress += 1;
    }

    fn cross(ticks_per_frame: u32) -> Crossing {
        let period = std::time::Duration::from_millis(10);
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .init_resource::<Crossing>();
        register_sim_tick(&mut app);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);
        app.add_systems(FixedUpdate, flip_on_the_fifth_tick)
            .add_systems(
                FixedUpdate,
                count_in_progress_steps.run_if(in_state(GamePhase::InProgress)),
            )
            .add_systems(OnEnter(GamePhase::InProgress), record_enter);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            period * ticks_per_frame,
        ));
        // The first update carries a zero delta and runs no step.
        app.update();
        for _ in 0..(STEPS / ticks_per_frame as u64) {
            app.update();
        }
        assert_eq!(
            app.world().resource::<SimTick>().0,
            STEPS,
            "precondition: {ticks_per_frame} tick(s) per frame must still \
                 cover {STEPS} steps"
        );
        std::mem::take(&mut *app.world_mut().resource_mut::<Crossing>())
    }

    let per_tick = cross(1);
    let per_four = cross(4);

    assert_eq!(
        per_tick.entered_on,
        Some(SWITCH_ON),
        "OnEnter must run inside the step that wrote NextState"
    );
    // Steps 6..=11 run under the new phase; step 5 wrote the change after
    // its own gated systems had already been skipped.
    assert_eq!(per_tick.steps_in_progress, STEPS - SWITCH_ON - 1);
    assert_eq!(
        per_tick, per_four,
        "a four-step frame must cross the phase boundary on the SAME \
             logical tick as a one-step frame — a difference means the \
             transition is still frame-timed and the steps after the writer \
             ran under the stale phase"
    );
}

/// Sibling to the phase-transition test above, for the actual hazard
/// issue #907's review found: production's `NextState<GamePhase>` writers
/// for game-start (`headless_auto_start` in `headless/app.rs`,
/// `apply_force_start` in `server/bridge.rs`) used to write from
/// `PreUpdate`, so `OnEnter(GamePhase::InProgress)` — and the player-ship
/// `WorldId` minted inside it by `spawn_game_start_entities` — landed at
/// the FRAME-level `StateTransition`, whose relationship to `SimTick` is a
/// function of frame pacing rather than of a tick. Moving those writers
/// into `FixedUpdate` (this test's `auto_start`, shaped exactly like
/// `headless_auto_start`) puts them on the SAME fixed-schedule
/// `StateTransition` `register_fixed_state_transition` installs above, so
/// the mint inside `OnEnter` now stamps a tick that does not move when the
/// frame rate does — asserted here directly on the minted [`WorldId`]
/// rather than only on the tick number, since two instances agreeing on
/// the tick but minting in a different order would still disagree on
/// identity.
#[test]
fn game_start_mint_is_frame_pacing_invariant() {
    use crate::core::messages::GamePhase;
    use crate::world_id::{IdNamespace, WorldId, WorldIdMint};

    #[derive(Resource, Default, Debug, PartialEq, Eq)]
    struct Minted(Option<WorldId>);

    /// Shaped exactly like `headless::app::headless_auto_start`: a
    /// `Local<bool>` latch that fires exactly once, the first time it
    /// observes `Lobby`.
    fn auto_start(
        state: Res<State<GamePhase>>,
        mut next: ResMut<NextState<GamePhase>>,
        mut started: Local<bool>,
    ) {
        if *started || state.get() != &GamePhase::Lobby {
            return;
        }
        next.set(GamePhase::InProgress);
        *started = true;
    }

    /// Shaped exactly like `server_app::spawn_game_start_entities`
    /// minting the player ship's `WorldId` from `OnEnter`.
    fn mint_player_ship(mint: ResMut<WorldIdMint>, mut minted: ResMut<Minted>) {
        minted.0 = Some(mint.mint(IdNamespace::Entity));
    }

    fn run(ticks_per_frame: u32) -> WorldId {
        let period = std::time::Duration::from_millis(10);
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<GamePhase>()
            .init_resource::<Minted>();
        register_sim_tick(&mut app);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);
        app.add_systems(FixedUpdate, auto_start)
            .add_systems(OnEnter(GamePhase::InProgress), mint_player_ship);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            period * ticks_per_frame,
        ));
        // The first update carries a zero delta and runs no step.
        app.update();
        // A handful more frames is plenty for either pacing to have
        // crossed into InProgress and minted.
        for _ in 0..4 {
            app.update();
        }
        app.world()
            .resource::<Minted>()
            .0
            .expect("OnEnter(InProgress) must have minted by now")
    }

    assert_eq!(
        run(1),
        run(4),
        "the game-start mint must land on, and mint against, the SAME \
             tick whatever the frame pacing — a difference here is exactly \
             issue #907's off-tick player-ship-mint hazard"
    );
}

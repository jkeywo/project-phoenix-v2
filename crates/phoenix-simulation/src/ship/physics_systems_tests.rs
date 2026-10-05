use super::*;
use crate::server_app::{LocalShip, Ship};
use crate::ship::control_source::ControlSource;
use crate::ship::helm_ai::helm_axes_operate_ai;
use crate::ship::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
use crate::ship::test_support::*;

#[test]
fn gm_system_disable_stops_charging_active_impulse_and_boost_without_restarting_on_restore() {
    use crate::ship::system_registry::{helm_boost_system_id, helm_impulse_system_id};
    for phase in [ImpulsePhase::Charging, ImpulsePhase::Active] {
        let mut app = test_app();
        tick(&mut app);
        let ship = find_ship_entity(&mut app);
        set_ship_impulse(
            &mut app,
            crate::ship::impulse::ImpulseState {
                phase,
                charge_progress: 0.99,
            },
        );
        app.world_mut()
            .get_mut::<ShipBoost>(ship)
            .unwrap()
            .0
            .activate();
        {
            let mut sources = app
                .world_mut()
                .get_mut::<ShipSystemControlSources>(ship)
                .unwrap();
            sources.0.set_gm_disabled(helm_impulse_system_id(), true);
            sources.0.set_gm_disabled(helm_boost_system_id(), true);
        }
        tick(&mut app);
        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Idle);
        assert!(!app.world().get::<ShipBoost>(ship).unwrap().0.is_active());
        assert_eq!(
            app.world().get::<ShipPhysics>(ship).unwrap().forward_speed,
            0.0,
            "disabled impulse cannot drive autopilot physics"
        );
        {
            let mut sources = app
                .world_mut()
                .get_mut::<ShipSystemControlSources>(ship)
                .unwrap();
            sources.0.set_gm_disabled(helm_impulse_system_id(), false);
            sources.0.set_gm_disabled(helm_boost_system_id(), false);
        }
        tick(&mut app);
        assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Idle,
            "Restore does not restart a cancelled drive"
        );
        assert!(!app.world().get::<ShipBoost>(ship).unwrap().0.is_active());
        app.world_mut().get_mut::<ImpulseCommand>(ship).unwrap().0 = ImpulsePhase::Charging;
        app.world_mut().get_mut::<BoostCommand>(ship).unwrap().0 = true;
        tick(&mut app);
        assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Charging);
        assert!(app.world().get::<ShipBoost>(ship).unwrap().0.is_active());
    }
}

#[test]
fn added_drive_defaults_stay_inert_but_real_writes_apply_in_order() {
    use crate::core::messages::{AdmittedCommand, AdmittedCommands, SystemControlPayload};
    use crate::entities::spawner::EntitySystemHull;
    use crate::server_app::{ImpulseHullHistory, ShipImpulse};
    use crate::ship::damage::SystemHull;
    use crate::ship::helm::DriveCommandWrites;
    use crate::ship::impulse::{ImpulsePhase, ImpulseState};
    use crate::ship::impulse_boost_systems::handle_impulse_messages;

    for (damage, admitted_start) in [(false, false), (true, false), (true, true)] {
        let mut app = App::new();
        app.add_systems(
            Update,
            (
                handle_impulse_messages,
                crate::ship::helm_admission::process_helm_inputs,
                apply_helm_commands,
            )
                .chain(),
        );
        let commands = if admitted_start {
            vec![AdmittedCommand {
                target: crate::ship::system_registry::helm_impulse_system_id(),
                payload: SystemControlPayload::StartImpulseCharge,
                response_token: None,
                feedback_correlation: None,
            }]
        } else {
            Vec::new()
        };
        let entity = app
            .world_mut()
            .spawn((
                crate::ai::server::AiHighFidelity,
                ShipImpulse(ImpulseState {
                    phase: ImpulsePhase::Active,
                    charge_progress: 1.0,
                }),
                ImpulseHullHistory(Some(if damage { 101.0 } else { 100.0 })),
                EntitySystemHull(SystemHull::from_config(&[(
                    crate::core::messages::SystemId("hull".into()),
                    100.0,
                )])),
                ImpulseCommand::default(),
                AdmittedCommands(commands),
            ))
            .id();
        app.update();
        let expected = if damage && !admitted_start {
            ImpulsePhase::Idle
        } else {
            ImpulsePhase::Active
        };
        assert_eq!(
            app.world().get::<ShipImpulse>(entity).unwrap().0.phase,
            expected,
            "damage={damage}, later admitted start={admitted_start}"
        );
        assert_eq!(
            *app.world().get::<DriveCommandWrites>(entity).unwrap(),
            DriveCommandWrites::default()
        );
    }
}

// Regression test for issue #695 follow-up: LOD promotion re-inserting
// a fresh default `ImpulseCommand` must not silently cancel an
// in-progress impulse charge on the tick it's (re-)added. Bevy marks a
// freshly-inserted component as "changed" on its insertion tick, so
// without the `!cmd.is_added()` guard in `apply_helm_commands`, a
// ship's legitimate `Charging` state would get force-reset to `Idle`
// purely as a side effect of gaining `AiHighFidelity`/`ImpulseCommand`
// again, not from any explicit AI decision or player command.
#[test]
fn impulse_command_reinsertion_does_not_cancel_in_progress_charge() {
    let mut app = test_app();
    // Let the app settle past the initial-spawn insertion tick.
    tick(&mut app);

    // Simulate the ship having been mid-charge (e.g. promoted while a
    // human/AI decision had already started charging impulse).
    set_ship_impulse(
        &mut app,
        crate::ship::impulse::ImpulseState {
            phase: ImpulsePhase::Charging,
            charge_progress: 0.4,
        },
    );

    // Simulate LOD demotion: remove the intent component but leave
    // `ShipImpulse` untouched, exactly as `lod_ai_ships`'s demote
    // branch does.
    let ship = find_ship_entity(&mut app);
    app.world_mut().entity_mut(ship).remove::<ImpulseCommand>();
    tick(&mut app);

    // The impulse charge must have persisted across demotion (no
    // `ImpulseCommand` present means `apply_helm_commands` skips this
    // ship entirely).
    assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Charging);

    // Simulate LOD re-promotion: `lod_ai_ships` inserts a fresh
    // default `ImpulseCommand` (phase = Idle) on the ship.
    app.world_mut()
        .entity_mut(ship)
        .insert(ImpulseCommand::default());
    tick(&mut app);

    // On the insertion tick, the default `Idle` value must NOT be
    // force-applied: the in-progress charge should persist untouched
    // because no explicit AI/human decision wrote a new value yet.
    assert_eq!(
            get_ship_impulse(&mut app).phase,
            ImpulsePhase::Charging,
            "re-inserting ImpulseCommand on LOD promotion must not cancel an in-progress impulse charge"
        );

    // A subsequent tick where something explicitly writes a changed
    // value (not merely re-inserts) should still apply normally.
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ImpulseCommand>()
        .unwrap()
        .0 = ImpulsePhase::Idle;
    tick(&mut app);
    assert_eq!(get_ship_impulse(&mut app).phase, ImpulsePhase::Idle);
}

// ── integrate_ship_physics single-writer tests (issue #699) ───────────────

/// Minimal app exercising `integrate_ship_physics` in isolation, with the
/// debug helm write-tracker wired up exactly as `ShipPlugin` wires it.
///
/// Deliberately excludes `process_helm_inputs` and the per-axis AI helm
/// systems so a test
/// can seed the intent components directly and observe what the integrator
/// alone does with them — which is the whole point of the #695/#699 split.
fn integrator_only_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin).insert_resource(
        bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(200)),
    );
    #[cfg(debug_assertions)]
    app.init_resource::<crate::ship::helm::HelmPhysicsFrame>()
        .add_systems(First, crate::ship::helm::tick_helm_physics_frame);
    app.add_systems(Update, integrate_ship_physics);
    app
}

/// Spawn a ship into `integrator_only_app` with the given helm control
/// source and intent, optionally as the `LocalShip`.
fn spawn_integrator_ship(
    app: &mut App,
    source: ControlSource,
    is_local: bool,
    thrust: f32,
    steering: f32,
    lateral: f32,
) -> Entity {
    let mut sources = ShipSystemControlSources::default();
    sources.0.set(
        crate::ship::system_registry::helm_thrust_system_id(),
        source,
    );
    sources.0.set(
        crate::ship::system_registry::helm_steering_system_id(),
        source,
    );
    let entity = app
        .world_mut()
        .spawn((
            Ship,
            sources,
            ShipPhysics::default(),
            crate::ai::server::AiHighFidelity,
            ThrustInput(thrust),
            SteeringInput(steering),
            LateralThrustInput(lateral),
            VerticalThrustInput::default(),
        ))
        .id();
    if is_local {
        app.world_mut().entity_mut(entity).insert(LocalShip);
    }
    entity
}

fn physics_of(app: &mut App, entity: Entity) -> ShipPhysics {
    *app.world().entity(entity).get::<ShipPhysics>().unwrap()
}

/// AC: `integrate_ship_physics` is the sole helm-path writer of
/// `ShipPhysics`, observed through the debug write-tracker.
///
/// After a tick, every high-fidelity ship must be stamped, and stamped by
/// `integrate_ship_physics` — no other system claimed the helm write.
#[cfg(debug_assertions)]
#[test]
fn integrate_ship_physics_is_sole_helm_writer() {
    let mut app = integrator_only_app();
    let ship = spawn_integrator_ship(&mut app, ControlSource::Human, true, 1.0, 0.0, 0.0);

    // Several ticks: the tracker must not trip, and the stamp must track
    // the frame counter rather than going stale.
    for _ in 0..5 {
        tick(&mut app);
        let frame = app
            .world()
            .resource::<crate::ship::helm::HelmPhysicsFrame>()
            .0;
        let guard = app
            .world()
            .entity(ship)
            .get::<crate::ship::helm::HelmPhysicsWriteGuard>()
            .expect("AiHighFidelity must bring a write guard onto every ship it marks");
        assert_eq!(
            guard.last_write(),
            Some((frame, "integrate_ship_physics")),
            "integrate_ship_physics must be the sole helm-path writer of ShipPhysics"
        );
    }

    // Sanity: the ship actually moved, so the tracker was tracking a real
    // integration rather than a no-op.
    assert!(
        physics_of(&mut app, ship).forward_speed > 0.0,
        "ship must have actually been integrated"
    );
}

/// The write-tracker must actually bite: if some other writer stamps the
/// ship for the frame `integrate_ship_physics` is about to run, the
/// integrator panics rather than silently double-integrating.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "single-writer violation")]
fn write_tracker_panics_when_a_second_helm_writer_claims_the_same_frame() {
    let mut app = integrator_only_app();
    let ship = spawn_integrator_ship(&mut app, ControlSource::Human, true, 1.0, 0.0, 0.0);
    tick(&mut app);

    // Impersonate a second helm-path writer claiming the *next* frame
    // (the frame counter is bumped in `First`, before the integrator runs).
    let next_frame = app
        .world()
        .resource::<crate::ship::helm::HelmPhysicsFrame>()
        .0
        + 1;
    {
        let mut ship_mut = app.world_mut().entity_mut(ship);
        let mut guard = ship_mut
            .get_mut::<crate::ship::helm::HelmPhysicsWriteGuard>()
            .unwrap();
        guard.record_write(ship, "some_future_helm_system", next_frame);
    }

    tick(&mut app);
}

/// AC: AI helm and human helm produce identical trajectories given
/// equivalent inputs.
///
/// Post-#695 both paths converge on the same intent components, and
/// `integrate_ship_physics` is the only thing downstream of them. This pins
/// that the integrator does not branch on human-vs-AI: two ships whose helm
/// `ControlSource` differs, but whose intent is identical, must trace the
/// same path. (Set up via `integrator_only_app` so the two *deciders* are
/// out of the picture — this is specifically about the shared integrator.)
#[test]
fn ai_and_human_helm_produce_identical_trajectories() {
    let mut app = integrator_only_app();
    // Non-trivial intent: thrust + steering + strafe, so any human-vs-AI
    // branch anywhere in the integrator would show up as divergence.
    const THRUST: f32 = 0.8;
    // Sized so `acceleration * dt` (≈6.7/tick at the `HELM_AI_MAX_DT_SECS`
    // cap) carries the ship past `THRUST * TEST_MAX_SPEED` = 16.0 well
    // inside the loop below.
    const TEST_MAX_SPEED: f32 = 20.0;
    const TEST_ACCELERATION: f32 = 200.0;
    let human = spawn_integrator_ship(&mut app, ControlSource::Human, false, THRUST, 0.6, -0.4);
    let ai = spawn_integrator_ship(&mut app, ControlSource::Ai, false, THRUST, 0.6, -0.4);

    // `compute_physics` is acceleration-rate-limited: while
    // `|target - forward_speed| > acceleration * dt` the per-tick delta is
    // exactly `acceleration * dt` *regardless of thrust magnitude*, so any
    // thrust divergence between the two ships is invisible. With the stock
    // config and `dt` capped at `HELM_AI_MAX_DT_SECS`, neither ship escapes
    // that regime within a short test — which made this test blind to
    // thrust. Give both ships an identical high-acceleration config so
    // `forward_speed` reaches its thrust-proportional target within a few
    // ticks and thrust becomes observable. Test setup, not gameplay tuning.
    let test_cfg = ShipPhysicsConfigResource(ShipPhysicsConfig {
        max_speed: TEST_MAX_SPEED,
        acceleration: TEST_ACCELERATION,
        ..ShipPhysicsConfig::new()
    });
    for e in [human, ai] {
        app.world_mut().entity_mut(e).insert(test_cfg.clone());
    }

    // Precondition: the two ships genuinely differ in control source,
    // otherwise this test is vacuous.
    let ai_of = |app: &App, e: Entity| {
        helm_axes_operate_ai(
            app.world()
                .entity(e)
                .get::<ShipSystemControlSources>()
                .unwrap(),
        )
    };
    assert!(
        ai_of(&app, ai) && !ai_of(&app, human),
        "test must compare an AI-controlled helm against a human-controlled one"
    );

    // Compare the whole trajectory, not just the endpoint.
    for step in 0..10 {
        tick(&mut app);
        assert_eq!(
            physics_of(&mut app, human),
            physics_of(&mut app, ai),
            "human- and AI-controlled helm must integrate identically from \
                 identical intent (diverged at step {step})"
        );
    }

    // Sanity: the ships actually moved and turned, so equality is not the
    // trivial equality of two untouched defaults.
    let p = physics_of(&mut app, human);
    assert!(
        p.yaw != 0.0 && p.lateral_speed != 0.0,
        "ships must have actually manoeuvred, got {p:?}"
    );

    // Anti-vacuity guard for thrust specifically. `forward_speed` must have
    // settled at its thrust-proportional target, proving the trajectory left
    // `compute_physics`'s acceleration-rate-limited regime — the regime in
    // which the per-tick delta is independent of thrust and the equality
    // above therefore says nothing about it. Without this, retuning
    // `acceleration`/`max_speed` could silently re-blind the test to thrust.
    assert_eq!(
        p.forward_speed,
        THRUST * TEST_MAX_SPEED,
        "forward_speed must reach its thrust-proportional target, else this \
             test cannot observe thrust at all"
    );
}

/// AC: impulse override zeroes steering and lateral.
///
/// While impulse is active the autopilot forces thrust=1/steering=0/
/// lateral=0 regardless of helm intent. A control ship with identical
/// intent but no impulse must yaw and strafe, proving the override is what
/// suppresses them rather than the inputs being ignored generally.
#[test]
fn impulse_override_zeroes_steering_and_lateral() {
    let mut app = integrator_only_app();
    let impulsing = spawn_integrator_ship(&mut app, ControlSource::Human, true, 0.0, 1.0, 1.0);
    let control = spawn_integrator_ship(&mut app, ControlSource::Human, true, 0.0, 1.0, 1.0);

    let mut active = crate::ship::impulse::ImpulseState::new();
    active.start_charge();
    active.tick(IMPULSE_CHARGE_DURATION, IMPULSE_CHARGE_DURATION);
    assert_eq!(active.phase, ImpulsePhase::Active, "impulse must be active");
    app.world_mut()
        .entity_mut(impulsing)
        .insert(ShipImpulse(active));
    // Use a zero steering_multiplier to preserve the "impulse zeroes steering" assertion.
    app.world_mut()
        .entity_mut(impulsing)
        .insert(ImpulseConfigResource {
            steering_multiplier: 0.0,
            ..ImpulseConfigResource::default()
        });

    for _ in 0..5 {
        tick(&mut app);
    }

    let p = physics_of(&mut app, impulsing);
    assert_eq!(p.yaw, 0.0, "impulse override must zero steering, got {p:?}");
    assert_eq!(
        p.lateral_speed, 0.0,
        "impulse override must zero lateral thrust, got {p:?}"
    );
    assert_eq!(
        p.roll, 0.0,
        "impulse override must level the ship, got {p:?}"
    );
    assert!(
        p.forward_speed > 0.0,
        "impulse autopilot must force full forward thrust despite thrust=0.0 intent, got {p:?}"
    );

    // The control ship shares the same intent but has no impulse: it must
    // turn and strafe, so the assertions above are about the override.
    let c = physics_of(&mut app, control);
    assert!(
        c.yaw != 0.0 && c.lateral_speed != 0.0,
        "control ship (no impulse) must steer and strafe from the same intent, got {c:?}"
    );
}

/// **Issue #1116.** Banking is not a projection: `ShipPhysics.roll` is
/// folded into the authoritative digest, so it must not depend on which
/// hull a host happens to project to its own crew.
///
/// Two ships with identical steering intent, one tagged `LocalShip` and one
/// not, must roll identically. Before #1116 the untagged one stayed at
/// exactly zero while the tagged one leaned — which is a digest two hosts
/// running the same mission could never agree on, because each tags a
/// different ship.
#[test]
fn banking_does_not_depend_on_which_ship_this_host_projects() {
    let mut app = integrator_only_app();
    let local = spawn_integrator_ship(&mut app, ControlSource::Human, true, 0.0, 1.0, 0.0);
    let remote = spawn_integrator_ship(&mut app, ControlSource::Human, false, 0.0, 1.0, 0.0);
    // `BankConfigResource::default()` authors `max_bank_deg = 0.0` — a hull
    // that does not lean — so both ships would hold roll at exactly zero and
    // the comparison below would be two zeroes agreeing. Give both the same
    // banking hull, which is what the precondition then checks.
    for entity in [local, remote] {
        app.world_mut()
            .entity_mut(entity)
            .insert(BankConfigResource {
                max_bank_deg: 30.0,
                ..BankConfigResource::default()
            });
    }

    for _ in 0..5 {
        tick(&mut app);
    }

    let local_roll = physics_of(&mut app, local).roll;
    let remote_roll = physics_of(&mut app, remote).roll;
    assert!(
            local_roll.abs() > 1e-6,
            "precondition: the steering intent must actually produce a lean, or              the equality below is two zeroes agreeing"
        );
    assert_eq!(
            local_roll, remote_roll,
            "roll is folded into the authoritative digest, so it cannot be              gated on `LocalShip`: two hosts tag different ships and would              disagree from the first tick either crew steered"
        );
}

/// Set `entity`'s starting `ShipPhysics` so a coast-down is observable
/// rather than being confused with "never moved".
///
/// Call it AFTER a warm-up tick: `integrator_only_app`'s very first
/// `app.update()` runs with a zero `Time::delta`, so a state seeded before
/// it would be measured one step later than the test thinks.
fn seed_physics(app: &mut App, entity: Entity, seed: ShipPhysics) {
    *app.world_mut()
        .entity_mut(entity)
        .get_mut::<ShipPhysics>()
        .expect("spawned with physics") = seed;
}

/// Issue #968: a DESTROYED lateral thruster must stop pushing the hull, and
/// the hull must COAST DOWN rather than stop dead.
///
/// `LateralThrustInput` is a latched intent component, and the per-axis AI
/// operator stops emitting the moment its system goes offline — so without a
/// capability gate here the last fraction the thruster was ever commanded
/// keeps being integrated for the rest of the run. On `combat_test` that was
/// a wrecked destroyer strafing at a fixed 8.77 u/s for 300 s, out of the
/// belt and out of the mission. Both ships below carry the identical latched
/// intent AND the identical starting strafe; the only difference is which
/// one's thruster is in the offline set.
///
/// The 8.77 u/s start is the measured figure, and it is what makes the
/// coast-down assertion mean something: the gate feeds the axis `0.0`, so
/// `compute_lateral_speed` decelerates at the hull's authored
/// `lateral_acceleration` (15 u/s² ⇒ 0.5 per capped 1/30 s step) instead of
/// zeroing the speed outright.
///
/// The repair leg at the end pins what this gate does and does not do. It
/// MASKS the latch; it does not clear it. Clearing is
/// `process_helm_inputs`' job (issue #968, see
/// `helm_admission::an_offline_axis_clears_its_latched_intent`) and this
/// integrator-only fixture deliberately does not run it — so here, a repaired
/// thruster picks the stale fraction straight back up. That is the honest
/// contract of this file: the gate is a per-tick capability check.
#[test]
fn a_destroyed_lateral_thruster_stops_applying_its_latched_command() {
    let mut app = integrator_only_app();

    // The wreck's measured strafe when its thruster died.
    const STRAFE: f32 = 8.77;
    // `lateral_acceleration` (15) × the `HELM_AI_MAX_DT_SECS` step (1/30).
    const COAST_STEP: f32 = 0.5;

    let working = spawn_integrator_ship(&mut app, ControlSource::Ai, false, 0.0, 0.0, 1.0);
    let destroyed = spawn_integrator_ship(&mut app, ControlSource::Ai, false, 0.0, 0.0, 1.0);
    app.world_mut()
        .entity_mut(destroyed)
        .get_mut::<ShipSystemControlSources>()
        .expect("spawned with control sources")
        .0
        .set_offline(
            crate::ship::system_registry::lateral_thrust_system_id(),
            true,
        );

    tick(&mut app); // warm-up: the first update integrates a zero `dt`.
    for e in [working, destroyed] {
        seed_physics(
            &mut app,
            e,
            ShipPhysics {
                lateral_speed: STRAFE,
                ..ShipPhysics::default()
            },
        );
    }

    // One tick: the axis must be coasting, not snapped to zero.
    tick(&mut app);
    let coasting = physics_of(&mut app, destroyed);
    assert!(
        (coasting.lateral_speed - (STRAFE - COAST_STEP)).abs() < 1e-4,
        "a destroyed lateral thruster must coast down on the hull's authored \
             acceleration, not stop dead — expected {:.2}, got {coasting:?}",
        STRAFE - COAST_STEP
    );

    // And it must reach zero and stay there.
    for _ in 0..25 {
        tick(&mut app);
    }
    let dead = physics_of(&mut app, destroyed);
    assert_eq!(
        dead.lateral_speed, 0.0,
        "a destroyed lateral thruster must command nothing, got {dead:?}"
    );

    // Control: the same latched intent and the same starting strafe on an
    // ONLINE thruster drives on to the hull's cap, so the assertions above
    // are about the offline gate and not about the intent never having been
    // applied at all.
    let alive = physics_of(&mut app, working);
    assert!(
        alive.lateral_speed > STRAFE,
        "an online lateral thruster must still apply its intent, got {alive:?}"
    );

    // Repair edge: the gate is a mask, not an erase. With the latch still in
    // place (nothing in this fixture clears it) a repaired thruster resumes
    // from it immediately — which is exactly why `process_helm_inputs` clears
    // the intent while the axis is offline.
    app.world_mut()
        .entity_mut(destroyed)
        .get_mut::<ShipSystemControlSources>()
        .expect("spawned with control sources")
        .0
        .set_offline(
            crate::ship::system_registry::lateral_thrust_system_id(),
            false,
        );
    tick(&mut app);
    assert!(
        physics_of(&mut app, destroyed).lateral_speed > 0.0,
        "the gate masks the latch for as long as the axis is offline and no \
             longer; clearing the latch is `process_helm_inputs`' job"
    );
}

/// Issue #968: a DESTROYED `helm-steering` must stop turning the hull.
///
/// The worst of the three axes and the one the flee fix originally missed.
/// `SteeringInput` latches exactly like `LateralThrustInput`, `ai_helm_steering`
/// `continue`s on the same `!policy_for(..).operate_ai` gate, and the
/// integrator passed `steering` through ungated — so a hull that lost its
/// steering gear kept applying its last yaw fraction and simply circled out
/// of the scenario for ever.
#[test]
fn a_destroyed_helm_steering_stops_turning_the_hull() {
    let mut app = integrator_only_app();

    let working = spawn_integrator_ship(&mut app, ControlSource::Ai, false, 0.0, 1.0, 0.0);
    let destroyed = spawn_integrator_ship(&mut app, ControlSource::Ai, false, 0.0, 1.0, 0.0);
    app.world_mut()
        .entity_mut(destroyed)
        .get_mut::<ShipSystemControlSources>()
        .expect("spawned with control sources")
        .0
        .set_offline(
            crate::ship::system_registry::helm_steering_system_id(),
            true,
        );

    for _ in 0..5 {
        tick(&mut app);
    }

    let dead = physics_of(&mut app, destroyed);
    assert_eq!(
        dead.yaw, 0.0,
        "destroyed steering gear must command no yaw, got {dead:?}"
    );

    let alive = physics_of(&mut app, working);
    assert!(
        alive.yaw > 0.0,
        "online steering gear must still apply its latched intent, got {alive:?}"
    );
}

/// Issue #968: a DESTROYED `helm-thrust` must stop driving the hull, even
/// with both engines intact.
///
/// The engine scaling of issue #511 covers `helm-engine-port` /
/// `helm-engine-starboard` only; the throttle system itself was ungated, so a
/// hull whose `helm-thrust` was shot away but whose engines were fine cruised
/// off at its last commanded throttle. Neither ship here has an engine in the
/// offline set, so `engine_thrust_scale` is 1.0 for both and the only
/// difference is the throttle system.
#[test]
fn a_destroyed_helm_thrust_stops_driving_the_hull() {
    let mut app = integrator_only_app();

    // Under way when the throttle died, so the assertion below is a
    // coast-down and not "the ship never started".
    const CRUISE: f32 = 10.0;
    // `deceleration` (25) × the `HELM_AI_MAX_DT_SECS` step (1/30).
    const COAST_STEP: f32 = 25.0 / 30.0;

    let working = spawn_integrator_ship(&mut app, ControlSource::Ai, false, 1.0, 0.0, 0.0);
    let destroyed = spawn_integrator_ship(&mut app, ControlSource::Ai, false, 1.0, 0.0, 0.0);
    app.world_mut()
        .entity_mut(destroyed)
        .get_mut::<ShipSystemControlSources>()
        .expect("spawned with control sources")
        .0
        .set_offline(crate::ship::system_registry::helm_thrust_system_id(), true);

    tick(&mut app); // warm-up: the first update integrates a zero `dt`.
    for e in [working, destroyed] {
        seed_physics(
            &mut app,
            e,
            ShipPhysics {
                forward_speed: CRUISE,
                ..ShipPhysics::default()
            },
        );
    }

    tick(&mut app);
    let coasting = physics_of(&mut app, destroyed);
    assert!(
        (coasting.forward_speed - (CRUISE - COAST_STEP)).abs() < 1e-4,
        "a destroyed throttle must coast down on the hull's authored \
             deceleration — expected {:.2}, got {coasting:?}",
        CRUISE - COAST_STEP
    );

    for _ in 0..20 {
        tick(&mut app);
    }
    let dead = physics_of(&mut app, destroyed);
    assert_eq!(
        dead.forward_speed, 0.0,
        "a destroyed throttle must command nothing even with both engines \
             online, got {dead:?}"
    );

    let alive = physics_of(&mut app, working);
    assert!(
        alive.forward_speed > CRUISE,
        "an online throttle with the same latched intent must still drive the \
             hull, got {alive:?}"
    );
}

/// Issue #1053, through the path the bug actually arrived on.
///
/// `compute_physics` is where the clamp lived and where the pure tests pin
/// the bleed, but the CAP is not a physics constant — it is
/// `config.max_speed` multiplied by this ship's `MaxSpeed` modifier, right
/// here in the integrator. The bug was only ever reachable because a power
/// decider can move that modifier under a ship at flank. So this drives the
/// real thing: a `PowerGroup` MaxSpeed bonus applied, held until the hull
/// is sitting on the raised cap, then shed.
///
/// The lurch was the observable, so the assertions are about the shape of
/// the descent rather than its endpoint alone: not in one tick, monotone
/// throughout, and settled exactly on the new cap.
#[test]
fn a_helm_power_shed_at_the_cap_bleeds_speed_down_over_several_ticks() {
    use crate::core::messages::{ModifierSource, PowerGroupId};
    use crate::modifiers::Modifier;

    let helm = ModifierSource::PowerGroup(PowerGroupId("helm".into()));
    let base_cap = ShipPhysicsConfig::new().max_speed;
    let boosted_cap = base_cap * 1.25;

    let mut app = integrator_only_app();
    let ship = spawn_integrator_ship(&mut app, ControlSource::Ai, false, 1.0, 0.0, 0.0);

    // The x1.25 the issue measured, as a helm power-group bonus.
    {
        let mut mods = ShipModifiers::new();
        mods.add_or_update(Modifier {
            source: helm.clone(),
            slot: ModifierSlot::MaxSpeed,
            bonus: 0.25,
        });
        app.world_mut().entity_mut(ship).insert(mods);
    }

    // Up to the RAISED cap and held there, so the shed lands on a hull
    // genuinely at flank rather than one still accelerating.
    for _ in 0..200 {
        tick(&mut app);
    }
    let at_flank = physics_of(&mut app, ship).forward_speed;
    assert!(
        (at_flank - boosted_cap).abs() < 1e-3,
        "the ship must be sitting on its boosted cap before the shed; \
             got {at_flank} against {boosted_cap}"
    );

    // The shed: helm power drops a level and the bonus goes with it.
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<ShipModifiers>()
        .unwrap()
        .remove(&helm, &ModifierSlot::MaxSpeed);

    tick(&mut app);
    let after_one = physics_of(&mut app, ship).forward_speed;
    assert!(
        after_one > base_cap,
        "the excess must not be deleted in the tick the modifier changed — \
             this is the measured 67.5 -> 54.0 lurch (#1053); got {after_one} \
             against a new cap of {base_cap}"
    );

    let mut previous = after_one;
    let mut ticks = 1;
    for _ in 0..200 {
        tick(&mut app);
        let now = physics_of(&mut app, ship).forward_speed;
        assert!(
            now <= previous + 1e-5,
            "the bleed must be monotone; went {previous} -> {now}"
        );
        assert!(
            now >= base_cap - 1e-3,
            "the bleed must not undershoot the new cap; reached {now}"
        );
        previous = now;
        ticks += 1;
        if (now - base_cap).abs() < 1e-3 {
            break;
        }
    }
    assert!(
        ticks > 3,
        "a descent this fast is still a lurch; took {ticks} ticks"
    );
    assert!(
        (previous - base_cap).abs() < 1e-3,
        "the hull must settle ON the new cap, not near it; settled at {previous}"
    );
}

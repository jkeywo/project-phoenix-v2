use bevy::prelude::*;

use crate::messages::ModifierSlot;
use crate::modifiers::ShipModifiers;
use crate::server_app::LocalShip;
use crate::ship::components::{
    BankConfigResource, BoostConfigResource, ImpulseConfigResource, ShipPhysicsConfigResource,
    ShipSystemControlSources, HELM_AI_MAX_DT_SECS,
};
use crate::ship::helm::{
    BoostCommand, ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput,
    VerticalThrustInput,
};
use crate::ship_physics::{compute_physics, ShipPhysicsConfig, ShipPhysicsInput, ShipPhysicsState};
use crate::ship_state::ShipPhysics;
use crate::simulation::{ShipBoost, ShipImpulse};

pub(crate) fn sync_ship_position(mut ship_query: Query<(&ShipPhysics, &mut Transform)>) {
    for (physics, mut transform) in ship_query.iter_mut() {
        transform.translation.x = physics.x;
        transform.translation.y = physics.y;
        transform.translation.z = physics.z;
        transform.rotation = Quat::from_euler(EulerRot::YXZ, -physics.yaw, 0.0, physics.roll);
    }
}

/// Applies commanded impulse/boost phase transitions (issue #695), split
/// out from `integrate_ship_physics` so it can run *before*
/// `process_helm_inputs` — whose stale-input edge-detection needs to
/// observe this tick's freshly-transitioned `ShipImpulse.phase`, not last
/// tick's (the old fused `process_helm_inputs` mutated `ShipImpulse` via
/// `handle_impulse_messages` before its own edge-detect read it in the
/// same tick; splitting admission from integration would otherwise delay
/// that transition by one tick).
///
/// Uses change detection (`Ref::is_changed`) rather than unconditionally
/// re-applying the persisted intent every tick: the intent components
/// default to `Idle`/`false`, and blindly re-applying that default every
/// tick would fight any *other* code path (including test harnesses) that
/// sets `ShipImpulse`/`ShipBoost` directly without going through the
/// intent-command pipeline. Only a tick where the intent was actually
/// written (by `handle_impulse_messages`' hull-damage cancel, or by
/// `process_helm_inputs` applying an admitted impulse/boost payload from
/// either a human or an AI operator) triggers a transition; `start_charge`/
/// `cancel_charge` and `activate`/`deactivate` are themselves idempotent,
/// so re-applying an intent that happens to already match current state is
/// harmless.
pub(crate) fn apply_helm_commands(
    mut ships: Query<
        (
            Option<&mut ShipImpulse>,
            Option<Ref<ImpulseCommand>>,
            Option<&mut ShipBoost>,
            Option<Ref<BoostCommand>>,
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
) {
    for (impulse, impulse_cmd, boost, boost_cmd) in ships.iter_mut() {
        if let (Some(mut impulse), Some(cmd)) = (impulse, impulse_cmd) {
            // Exclude the insertion tick (issue #695 follow-up): LOD
            // promotion inserts a fresh default `ImpulseCommand`, which
            // Bevy also reports as "changed" on that same tick. Without
            // `!cmd.is_added()`, a promoted NPC's legitimate in-progress
            // impulse would be silently force-reset to `Idle` purely as a
            // side effect of gaining `AiHighFidelity`, not from any actual
            // AI decision or player command. The default should persist
            // untouched until something explicitly writes a new value on a
            // later tick.
            if cmd.is_changed() && !cmd.is_added() {
                match cmd.0 {
                    crate::impulse::ImpulsePhase::Charging => impulse.0.start_charge(),
                    crate::impulse::ImpulsePhase::Idle => impulse.0.cancel_charge(),
                    crate::impulse::ImpulsePhase::Active => {}
                }
            }
        }
        if let (Some(mut boost), Some(cmd)) = (boost, boost_cmd) {
            // Same insertion-tick exclusion as above. Since issue #881 this
            // is live for NPCs too: `process_helm_inputs` applies an admitted
            // `SetBoost`/`ToggleBoost` for every ship, so a non-local
            // `AiHighFidelity` NPC's boost policy engages here in the same
            // tick it was decided.
            if cmd.is_changed() && !cmd.is_added() {
                if cmd.0 {
                    boost.0.activate();
                } else {
                    boost.0.deactivate();
                }
            }
        }
    }
}

/// Sole writer of the helm path into `ShipPhysics` (issue #699; extracted
/// from the old fused `process_helm_inputs` monolith by issue #695).
///
/// Reads the `ThrustInput`/`SteeringInput`/`LateralThrustInput` intent
/// components — written this tick by whichever of `process_helm_inputs`
/// (human admission) or the per-axis helm AI (`ai_helm_thrust` /
/// `ai_helm_steering` / `ai_helm_lateral_thrust`) is authoritative for a given
/// ship's helm, per the existing `ControlTickPolicy` gate — plus the
/// post-transition `ShipImpulse`/`ShipBoost` state applied
/// by `apply_helm_commands`, and performs the actual physics integration.
/// Runs for both the player ship and any AI-promoted NPC (anything
/// carrying `AiHighFidelity`, which is exactly the set of ships carrying
/// these intent components). Human and AI helm therefore share one
/// integrator and produce identical trajectories from identical intent —
/// nothing below this point branches on human-vs-AI.
///
/// Concerns handled here, in order:
///  - impulse autopilot override (forces thrust=1, steering=0, lateral=0),
///  - engine-damage thrust scaling (issue #511),
///  - impulse acceleration multiplier,
///  - boost-drive speed/acceleration/steering multiplier,
///  - exactly one `compute_physics` call per ship per frame,
///  - visual banking/roll lerp.
///
/// Visual banking/roll is preserved as LocalShip-only, exactly as before — the
/// helm AI has never applied roll to NPCs (the `operate_helm_ai` monolith did
/// not, nor do its per-axis successors), and this system doesn't start doing so
/// either.
///
/// This system is the only *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`, enforced in debug
/// builds by `HelmPhysicsWriteGuard`. It is not the only writer of those
/// fields overall — see the sanctioned-exception table on `ShipPhysics`
/// (`src/ship/state.rs`) for the four out-of-band writers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn integrate_ship_physics(
    time: Res<Time>,
    physics_cfg_res: Option<Res<ShipPhysicsConfigResource>>,
    bank_cfg_res: Option<Res<BankConfigResource>>,
    mut ships: Query<
        (
            Entity,
            Has<LocalShip>,
            &ShipSystemControlSources,
            &mut ShipPhysics,
            Option<&ShipModifiers>,
            Option<&ShipPhysicsConfigResource>,
            Option<&ImpulseConfigResource>,
            Option<&BoostConfigResource>,
            Option<&BankConfigResource>,
            &ThrustInput,
            &SteeringInput,
            &LateralThrustInput,
            &VerticalThrustInput,
            (Option<&ShipImpulse>, Option<&ShipBoost>),
        ),
        With<crate::ai_plugin::AiHighFidelity>,
    >,
    #[cfg(debug_assertions)] frame: Res<crate::ship::helm::HelmPhysicsFrame>,
    #[cfg(debug_assertions)] mut guard_q: Query<&mut crate::ship::helm::HelmPhysicsWriteGuard>,
    #[cfg(debug_assertions)] mut commands: Commands,
) {
    // The `HELM_AI_MAX_DT_SECS` clamp is DEAD in production, and is kept only
    // for the bare-`App` fixtures (issue #895).
    //
    // Since #895 this system runs in `FixedUpdate`, where `Res<Time>` is the
    // fixed clock, so `dt == 1 / [global] sim_tick_hz`. The divergence trap
    // that used to live here — a slow host silently getting a shortened step,
    // so two hosts integrated differently from the same commands — is now
    // closed at LOAD instead: `world::config::parse_world` rejects any authored
    // `sim_tick_hz` below `entity_config::MIN_SIM_TICK_HZ`, which is derived
    // from this very constant. A shipped world therefore cannot reach the
    // clamp, and no run-time branch decides fidelity.
    //
    // What still reaches it is the bare-`App` fixture: it authors no world (so
    // no floor applies) and paces itself at `test_support::TEST_TICK` (200 ms).
    // The clamp is what keeps those fixtures' integration step at the 1/30 s
    // every helm assertion in this crate was written against. Deleting it is a
    // behaviour re-bless of ~6 combat-AI tests, not a determinism fix — see
    // `helm_ai::tests::backfill_helm_ai_caps_long_frame_yaw_step`, which pins
    // exactly that fixture contract.
    let dt = time.delta_secs().min(HELM_AI_MAX_DT_SECS);

    for (
        // Only read by the debug-only write tracker below; underscored so
        // release builds need no blanket `allow(unused_variables)`.
        _entity,
        is_local,
        sources,
        mut physics,
        modifiers,
        physics_cfg_comp,
        impulse_cfg,
        boost_cfg_comp,
        bank_cfg_comp,
        thrust_in,
        steering_in,
        lateral_in,
        vertical_in,
        (impulse, boost),
    ) in ships.iter_mut()
    {
        // Debug-only single-writer tripwire (issue #699). Self-healing: ships
        // that lack a guard get one, so promotion/demotion needs no bookkeeping.
        #[cfg(debug_assertions)]
        {
            let entity = _entity;
            match guard_q.get_mut(entity) {
                Ok(mut guard) => guard.record_write(entity, "integrate_ship_physics", frame.0),
                Err(_) => {
                    let mut guard = crate::ship::helm::HelmPhysicsWriteGuard::default();
                    guard.record_write(entity, "integrate_ship_physics", frame.0);
                    commands.entity(entity).insert(guard);
                }
            }
        }

        let default_modifiers;
        let modifiers: &ShipModifiers = match modifiers {
            Some(m) => m,
            None => {
                default_modifiers = ShipModifiers::new();
                &default_modifiers
            }
        };

        let state = ShipPhysicsState {
            x: physics.x,
            y: physics.y,
            z: physics.z,
            yaw: physics.yaw,
            forward_speed: physics.forward_speed,
            lateral_speed: physics.lateral_speed,
            vertical_speed: physics.vertical_speed,
        };

        let impulse_active = impulse.map(|i| i.0.is_active()).unwrap_or(false);

        let input = if impulse_active {
            // Autopilot: full forward thrust, steering scaled by authored multiplier.
            let impulse_cfg_for_steering = impulse_cfg.cloned().unwrap_or_default();
            ShipPhysicsInput {
                thrust: 1.0,
                steering: steering_in.0 * impulse_cfg_for_steering.steering_multiplier,
                lateral: 0.0,
                // Impulse autopilot levels out: no vertical manoeuvring while
                // the drive is engaged (issue #744), mirroring lateral.
                vertical: 0.0,
            }
        } else {
            ShipPhysicsInput {
                thrust: thrust_in.0,
                steering: steering_in.0,
                lateral: lateral_in.0,
                vertical: vertical_in.0,
            }
        };

        // ── Engine-damage thrust scaling (issue #511) ──────────────────────
        // Count how many fine engine systems are online. Each offline engine
        // removes 50% of the computed thrust. If both engines are offline,
        // thrust is zeroed.
        let port_offline = sources
            .0
            .is_offline(&crate::system_registry::helm_engine_port_system_id());
        let stbd_offline = sources
            .0
            .is_offline(&crate::system_registry::helm_engine_starboard_system_id());
        let engine_thrust_scale: f32 = match (port_offline, stbd_offline) {
            (true, true) => 0.0,
            (true, false) | (false, true) => 0.5,
            (false, false) => 1.0,
        };
        let scaled_input = ShipPhysicsInput {
            thrust: input.thrust * engine_thrust_scale,
            steering: input.steering,
            lateral: input.lateral,
            vertical: input.vertical,
        };

        let mut config = physics_cfg_comp
            .map(|c| c.0)
            .or_else(|| physics_cfg_res.as_deref().map(|c| c.0))
            .unwrap_or_else(ShipPhysicsConfig::new);
        config.max_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
        config.max_reverse_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
        config.max_yaw_rate *= modifiers.get(&ModifierSlot::MaxYawRate);

        if impulse_active {
            // Mirror `ship/impulse.rs::apply_to_physics`: a non-positive
            // multiplier (e.g. an unset TOML field defaulting to 0) falls
            // back to the const instead of nuking acceleration entirely.
            let impulse_cfg = impulse_cfg.cloned().unwrap_or_default();
            let mult = if impulse_cfg.acceleration_multiplier > 0.0 {
                impulse_cfg.acceleration_multiplier
            } else {
                crate::impulse::IMPULSE_ACCELERATION_MULTIPLIER
            };
            config.acceleration *= mult;
        }

        // Boost drive: while engaged, multiply max speed and acceleration.
        // Only applies when the ship's TOML enabled the feature.
        let boost_cfg = boost_cfg_comp.cloned().unwrap_or_default();
        let boost_active = boost.map(|b| b.0.is_active()).unwrap_or(false);
        if boost_cfg.enabled && boost_active {
            config.max_speed *= boost_cfg.multiplier;
            config.max_reverse_speed *= boost_cfg.multiplier;
            config.acceleration *= boost_cfg.multiplier;
            config.max_yaw_rate *= boost_cfg.steering_multiplier;
        }

        let result = compute_physics(state, scaled_input, dt, &config);

        physics.x = result.x;
        physics.y = result.y;
        physics.z = result.z;
        physics.yaw = result.yaw;
        physics.forward_speed = result.forward_speed;
        physics.lateral_speed = result.lateral_speed;
        physics.vertical_speed = result.vertical_speed;

        // Visual banking: LocalShip only, exactly as before — the helm AI has
        // never applied roll to NPCs (neither the old `operate_helm_ai` nor its
        // per-axis successors), and this shared step doesn't start doing so
        // either. Uses the unscaled
        // `input.steering` so roll reflects intent, not engine count.
        if is_local {
            let bank_cfg = bank_cfg_comp
                .cloned()
                .or_else(|| bank_cfg_res.as_deref().cloned())
                .unwrap_or_default();
            let max_bank_rad = bank_cfg.max_bank_deg.to_radians();
            let target_roll = if impulse_active {
                0.0
            } else {
                -input.steering * max_bank_rad
            };
            let lerp_factor = (bank_cfg.bank_lerp_rate * dt).min(1.0);
            physics.roll = physics.roll + (target_roll - physics.roll) * lerp_factor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_source::ControlSource;
    use crate::impulse::{ImpulsePhase, IMPULSE_CHARGE_DURATION};
    use crate::ship::helm_ai::helm_axes_operate_ai;
    use crate::ship::test_support::*;
    use crate::simulation::Ship;

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
            crate::impulse::ImpulseState {
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
        sources
            .0
            .set(crate::system_registry::helm_thrust_system_id(), source);
        sources
            .0
            .set(crate::system_registry::helm_steering_system_id(), source);
        let entity = app
            .world_mut()
            .spawn((
                Ship,
                sources,
                ShipPhysics::default(),
                crate::ai_plugin::AiHighFidelity,
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
                .expect("integrate_ship_physics must self-heal a write guard onto every ship");
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

        let mut active = crate::impulse::ImpulseState::new();
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
}

use bevy::prelude::*;

use crate::messages::{
    AdmittedCommands, InterSystemMsg, InterSystemPayload, InterSystemQueue, SystemControlPayload,
};
use crate::server_app::LocalShip;
use crate::ship::components::{HelmInputTimer, LastHelmInput, ShipSystemControlSources};
use crate::ship::helm::{LateralThrustInput, SteeringInput, ThrustInput};
use crate::simulation::ShipImpulse;

// Ã¢â€â‚¬Ã¢â€â‚¬ Systems Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Human-admission path (issue #695): turns `AdmittedCommands` into
/// `LastHelmInput` (kept for broadcast/back-compat consumers) and the
/// shared `ThrustInput`/`SteeringInput`/`LateralThrustInput` intent
/// components. Physics integration itself now lives in
/// `integrate_ship_physics`, which reads those intent components for both
/// the player ship and any AI-promoted NPC.
pub(crate) fn process_helm_inputs(
    time: Res<Time>,
    mut timer: ResMut<HelmInputTimer>,
    ship_query: Query<(&AdmittedCommands, &ShipSystemControlSources), With<LocalShip>>,
    mut last_input_q: Query<&mut LastHelmInput, With<LocalShip>>,
    mut intent_q: Query<
        (
            Option<&mut ThrustInput>,
            Option<&mut SteeringInput>,
            Option<&mut LateralThrustInput>,
        ),
        With<LocalShip>,
    >,
    impulse_q: Query<&ShipImpulse, With<LocalShip>>,
    mut prev_phase: Local<Option<crate::impulse::ImpulsePhase>>,
) {
    let Some(mut last_input) = last_input_q.iter_mut().next() else {
        return;
    };
    // Edge-detect Idle → Charging (or any → Charging) and zero out the
    // last cached helm input so a stale steering/thrust value can't
    // resurface the moment impulse cancels or the autopilot disengages.
    let current_phase = impulse_q
        .iter()
        .next()
        .map(|i| i.0.phase)
        .unwrap_or(crate::impulse::ImpulsePhase::Idle);
    if Some(current_phase) != *prev_phase {
        if current_phase == crate::impulse::ImpulsePhase::Charging {
            last_input.thrust = 0.0;
            last_input.steering = 0.0;
        }
        *prev_phase = Some(current_phase);
    }

    let Some((admitted, sources)) = ship_query.iter().next() else {
        return;
    };

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    // Per-axis admission (issue #801). Each axis is applied only when its own
    // declared system is NOT AI-operated: admission already refuses human
    // commands for an AI-held axis (`accept_human_input == false`), and the
    // per-axis AI (`ai_helm_thrust` / `ai_helm_steering`) is the authoritative
    // writer of that axis's intent this tick. Skipping per axis — instead of
    // the old ship-wide coarse-`helm` gate — is what fixes the #701 mismatch:
    // with `helm-thrust = Ai` and `helm-steering = Human`, the human's
    // steering is admitted and applied while the thrust axis stays with the AI.
    let thrust_ai = sources
        .0
        .policy_for(&crate::system_registry::helm_thrust_system_id())
        .operate_ai;
    let steering_ai = sources
        .0
        .policy_for(&crate::system_registry::helm_steering_system_id())
        .operate_ai;

    if !thrust_ai {
        for cmd in admitted.for_target(crate::system_registry::HELM_THRUST_SYSTEM_ID) {
            if let SystemControlPayload::SetThrust { value } = &cmd.payload {
                last_input.thrust = *value;
            }
        }
    }

    if !steering_ai {
        for cmd in admitted.for_target(crate::system_registry::HELM_STEERING_SYSTEM_ID) {
            if let SystemControlPayload::SetSteering { value } = &cmd.payload {
                last_input.steering = *value;
            }
        }
    }

    for cmd in admitted.for_target(&crate::system_registry::lateral_thrust_system_id().0) {
        if let SystemControlPayload::LateralThrustInput { lateral } = &cmd.payload {
            last_input.lateral = *lateral;
        }
    }

    if let Some((thrust_in, steering_in, lateral_in)) = intent_q.iter_mut().next() {
        if let Some(mut t) = thrust_in {
            if !thrust_ai {
                t.0 = last_input.thrust;
            }
        }
        if let Some(mut s) = steering_in {
            if !steering_ai {
                s.0 = last_input.steering;
            }
        }
        if let Some(mut l) = lateral_in {
            l.0 = last_input.lateral;
        }
    }
}

// ── Fine-grained Helm systems: channel-1 joystick → engines (issue #511) ──────

/// Forwards the current joystick state from the Helm Joystick fine system to
/// both Helm Engine fine systems via the `InterSystemQueue` (channel 1).
///
/// Runs in `SimSet::Physics` AFTER `process_helm_inputs` so `LastHelmInput`
/// has been populated from admitted commands this tick. Both engine instances
/// receive the same joystick payload; each engine independently gates on its
/// own online state when interpreting the message.
pub(crate) fn publish_joystick_to_engines(
    ships: Query<(&ShipSystemControlSources, &LastHelmInput), With<LocalShip>>,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    for (sources, last_input) in ships.iter() {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::helm_joystick_system_id());
        // Only publish when the joystick system can operate (human or AI).
        if !policy.accept_human_input && !policy.operate_ai {
            continue;
        }
        let port_id = crate::system_registry::helm_engine_port_system_id();
        let stbd_id = crate::system_registry::helm_engine_starboard_system_id();
        for target in [port_id, stbd_id] {
            inter_system.0.push(InterSystemMsg {
                target,
                payload: InterSystemPayload::JoystickState {
                    thrust: last_input.thrust,
                    steering: last_input.steering,
                },
                source_entity: None,
            });
        }
    }
}

/// Per-engine AI bookkeeping (issue #511). Mirrors what the joystick publishes
/// but for ships where an engine is under AI control.
///
/// The per-axis helm AI already drives physics (via the intent components and
/// `integrate_ship_physics`); this system only ensures the fine engine systems
/// reflect AI-controlled thrust in the blackboard so the GUI can show AUTO
/// badges correctly.
///
/// Reads the `LastHelmInput` thrust/steering pair, so it must run after both
/// `ai_helm_thrust` and `ai_helm_steering` — they declare
/// `.before(operate_helm_engine_ai)` themselves. See the registration note.
pub(crate) fn operate_helm_engine_ai(
    ships: Query<(&ShipSystemControlSources, &LastHelmInput), With<LocalShip>>,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    for (sources, last_input) in ships.iter() {
        // `publish_joystick_to_engines` already covers the normal case where
        // the joystick system is operable (human or AI). This system only
        // needs to push engine messages when the joystick itself is offline
        // (e.g. joystick damaged/disabled) but an individual engine is still
        // AI-controlled. This prevents a double-push on every Backfill tick.
        let joystick_policy = sources
            .0
            .policy_for(&crate::system_registry::helm_joystick_system_id());
        let joystick_publishing = joystick_policy.accept_human_input || joystick_policy.operate_ai;
        if joystick_publishing {
            // `publish_joystick_to_engines` will cover both engines this tick.
            continue;
        }

        let port_policy = sources
            .0
            .policy_for(&crate::system_registry::helm_engine_port_system_id());
        let stbd_policy = sources
            .0
            .policy_for(&crate::system_registry::helm_engine_starboard_system_id());

        // Joystick is offline; push for any engine that is still AI-operable.
        if port_policy.operate_ai {
            inter_system.0.push(InterSystemMsg {
                target: crate::system_registry::helm_engine_port_system_id(),
                payload: InterSystemPayload::JoystickState {
                    thrust: last_input.thrust,
                    steering: last_input.steering,
                },
                source_entity: None,
            });
        }
        if stbd_policy.operate_ai {
            inter_system.0.push(InterSystemMsg {
                target: crate::system_registry::helm_engine_starboard_system_id(),
                payload: InterSystemPayload::JoystickState {
                    thrust: last_input.thrust,
                    steering: last_input.steering,
                },
                source_entity: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_source::ControlSource;
    use crate::messages::ClientMessage;
    use crate::ship::test_support::*;

    #[test]
    fn control_system_helm_input_updates_last_input_and_moves_ship() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.25 },
            },
        );
        tick_twice(&mut app);

        assert_eq!(
            get_last_helm_input(&mut app),
            LastHelmInput {
                thrust: 1.0,
                steering: 0.25,
                lateral: 0.0,
            }
        );
        assert!(get_ship_physics(&mut app).forward_speed > 0.0);
    }

    #[test]
    fn ai_helm_operates_without_human_holder() {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        tick_twice(&mut app);

        assert_eq!(
            get_last_helm_input(&mut app),
            LastHelmInput {
                thrust: 0.0,
                steering: 0.0,
                lateral: 0.0,
            }
        );
        assert_eq!(get_ship_physics(&mut app).forward_speed, 0.0);
    }

    #[test]
    fn ai_helm_ignores_human_input() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        set_helm_control_source(&mut app, ControlSource::Ai);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: -1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 1.0 },
            },
        );
        tick_twice(&mut app);

        // Human input must be ignored when policy is AI; no AiControllerComponent
        // on the player ship yet, so LastHelmInput stays at default.
        assert_eq!(get_last_helm_input(&mut app), LastHelmInput::default());
    }

    /// The #701 mismatch, fixed by #801: with `helm-thrust = Ai` and
    /// `helm-steering = Human`, the human's combined joystick input used to be
    /// admitted or refused on the COARSE helm policy, so the whole input got
    /// in and the AI's thrust write had to win by ordering. Per-axis wire
    /// targets make admission itself per-axis: the human's `SetSteering` is
    /// admitted, the human's `SetThrust` is refused at the gate.
    #[test]
    fn per_axis_admission_fixes_the_coarse_vs_per_axis_mismatch() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        // AI holds the throttle; the human keeps the stick.
        set_fine_control_source(
            &mut app,
            crate::system_registry::helm_thrust_system_id(),
            ControlSource::Ai,
        );

        // The human joystick fans out into the two per-axis messages.
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 1.0 },
            },
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_steering_system_id(),
                payload: SystemControlPayload::SetSteering { value: 0.25 },
            },
        );
        tick_twice(&mut app);

        let last = get_last_helm_input(&mut app);
        assert_eq!(
            last.steering, 0.25,
            "the human-held steering axis must admit the human's SetSteering"
        );
        assert_eq!(
            last.thrust, 0.0,
            "the AI-held thrust axis must refuse the human's SetThrust at admission"
        );
    }

    #[test]
    fn human_helm_suppresses_ai_operate() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);

        tick(&mut app);

        assert_eq!(get_last_helm_input(&mut app), LastHelmInput::default());
        assert_eq!(get_ship_physics(&mut app).forward_speed, 0.0);
    }

    /// When helm is under AI control (`operate_ai = true`), `process_helm_inputs`
    /// must NOT admit stale human input over the AI's decision.
    ///
    /// Post-#695 `process_helm_inputs` no longer integrates physics at all —
    /// `integrate_ship_physics` is the sole helm-path writer. What this test
    /// pins is the *admission* skip: with helm AI-controlled, a stale non-zero
    /// `LastHelmInput` must not reach the intent components and therefore must
    /// not move the ship. (Before #695 this same setup guarded against a second
    /// `compute_physics` call at a different dt, which made the player ship
    /// move ~3× faster than AI-driven NPCs.)
    #[test]
    fn ai_controlled_helm_does_not_admit_stale_human_input() {
        let mut app = test_app();
        set_helm_control_source(&mut app, ControlSource::Ai);

        // Set a non-zero last input so that if process_helm_inputs incorrectly
        // runs compute_physics it will produce a non-trivial displacement.
        set_last_helm_input(
            &mut app,
            LastHelmInput {
                thrust: 1.0,
                steering: 0.0,
                lateral: 0.0,
            },
        );

        // Snapshot physics before the tick.
        let before = get_ship_physics(&mut app);

        tick(&mut app);

        let after = get_ship_physics(&mut app);

        // operate_helm_ai has no objectives in this test (blackboard empty), so
        // it zeros the intent components. If process_helm_inputs admitted the
        // stale thrust=1.0 anyway, integrate_ship_physics would have moved the
        // ship.
        assert_eq!(
            after.x, before.x,
            "ShipPhysics.x must not advance when helm is AI-controlled: \
             process_helm_inputs must skip admission"
        );
        assert_eq!(
            after.forward_speed, before.forward_speed,
            "forward_speed must not change when process_helm_inputs skips admission"
        );
    }

    #[test]
    fn joystick_publishes_state_to_engines_via_inter_system() {
        let mut app = test_app_with_engine_hull();
        // Ensure InterSystemQueue is initialised.
        app.init_resource::<InterSystemQueue>();

        // Set a known LastHelmInput before ticking.
        set_last_helm_input(
            &mut app,
            LastHelmInput {
                thrust: 0.75,
                steering: 0.25,
                lateral: 0.0,
            },
        );

        tick(&mut app);

        let queue = app.world().resource::<InterSystemQueue>();
        let port_id = crate::system_registry::helm_engine_port_system_id();
        let stbd_id = crate::system_registry::helm_engine_starboard_system_id();

        let port_msgs: Vec<_> = queue.for_target(port_id.0.as_str()).collect();
        let stbd_msgs: Vec<_> = queue.for_target(stbd_id.0.as_str()).collect();

        // `publish_joystick_to_engines` and `operate_helm_engine_ai` may both push.
        // At least one message must arrive for each engine.
        assert!(
            !port_msgs.is_empty(),
            "expected at least one JoystickState message for helm-engine-port"
        );
        assert!(
            !stbd_msgs.is_empty(),
            "expected at least one JoystickState message for helm-engine-starboard"
        );

        // The first message should carry the joystick values.
        let InterSystemPayload::JoystickState { thrust, steering } = &port_msgs[0].payload else {
            panic!("expected JoystickState payload for port engine");
        };
        assert!(
            (*thrust - 0.75).abs() < 0.01,
            "port engine thrust should match joystick thrust"
        );
        assert!(
            (*steering - 0.25).abs() < 0.01,
            "port engine steering should match joystick steering"
        );
    }
}

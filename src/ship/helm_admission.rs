use bevy::prelude::*;

use crate::messages::{
    AdmittedCommands, InterSystemMsg, InterSystemPayload, InterSystemQueue, SystemControlPayload,
};
use crate::region_plugin::RegionMembership;
use crate::server_app::LocalShip;
use crate::ship::components::{LastHelmInput, ShipSystemControlSources};
use crate::ship::helm::{ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput};

// Ã¢â€â‚¬Ã¢â€â‚¬ Systems Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// True when `entity` is inside a region whose authored effects include
/// `BlocksImpulse`. Per-entity generalisation of the LocalShip-only helper
/// in `impulse_boost_systems` (issue #824): the check is a property of the
/// ship's position, not of who commanded the charge.
fn entity_inside_blocks_impulse(
    entity: Entity,
    membership: &Option<Res<RegionMembership>>,
    region_query: &Query<&crate::entity_spawner::RegionEffectsSection>,
) -> bool {
    let Some(membership) = membership else {
        return false;
    };
    let Some(inside) = membership.inside.get(&entity) else {
        return false;
    };
    for &region_entity in inside {
        if let Ok(effects) = region_query.get(region_entity) {
            if effects
                .0
                .contains(&crate::region_effects::RegionEffectKind::BlocksImpulse)
            {
                return true;
            }
        }
    }
    false
}

/// Per-entity admitted-command applier for the Helm path (issue #824): turns
/// every ship's own `AdmittedCommands` into that ship's
/// `ThrustInput`/`SteeringInput`/`LateralThrustInput`/`ImpulseCommand` intent
/// components. Physics integration itself lives in `integrate_ship_physics`,
/// which reads those intent components for both the player ship and any
/// AI-promoted NPC.
///
/// **An admitted command applies regardless of source** (the spec's anonymity
/// rule, `pasm/spec/RADAR_TARGET_AUTHORITY_AND_ADMISSION.md` §3): authority
/// was checked once at admission — `admit_system_commands` for network
/// messages, `validate_and_admit` for the per-axis helm AI's same-tick
/// emissions — so the per-axis `!operate_ai` gates this system used to carry
/// are gone. A human command for an AI-held axis never reaches this system
/// (refused at the gate), and the AI's own commands arrive through the same
/// gate as everyone else's.
///
/// `LastHelmInput` is mirrored for the LocalShip only, exactly as the old
/// AI-side mirrors did: the viewscreen HUD (`recompute_hud_state`) and
/// `tick_boost`/`publish_joystick_to_engines` read it for the player ship,
/// while NPC `LastHelmInput` deliberately stays at its spawn default so
/// `ai_power_allocation`'s movement rule observes exactly what it observed
/// before this migration.
///
/// Impulse `StartImpulseCharge`/`CancelImpulse` are applied here for every
/// ship (they were split between `handle_impulse_messages` for the human path
/// and a direct `ImpulseCommand` write in `ai_helm_impulse` before #824),
/// gated by the same `BlocksImpulse` region check the human path has always
/// had. Hull-damage auto-cancel stays in `handle_impulse_messages`, which
/// runs earlier (`SimSet::Input`) so an admitted command can still override
/// it within the same tick — matching the old sequential order.
pub(crate) fn process_helm_inputs(
    membership: Option<Res<RegionMembership>>,
    region_query: Query<&crate::entity_spawner::RegionEffectsSection>,
    mut ships: Query<(
        Entity,
        &AdmittedCommands,
        Option<&mut LastHelmInput>,
        Option<&mut ThrustInput>,
        Option<&mut SteeringInput>,
        Option<&mut LateralThrustInput>,
        Option<&mut ImpulseCommand>,
        Has<LocalShip>,
    )>,
) {
    for (
        entity,
        admitted,
        last_input,
        mut thrust_in,
        mut steering_in,
        mut lateral_in,
        mut impulse_cmd,
        is_local,
    ) in ships.iter_mut()
    {
        let mut last_input = if is_local { last_input } else { None };

        if admitted.0.is_empty() {
            continue;
        }

        for cmd in admitted.0.iter() {
            match (&cmd.target.0, &cmd.payload) {
                (t, SystemControlPayload::SetThrust { value })
                    if t.as_str() == crate::system_registry::HELM_THRUST_SYSTEM_ID =>
                {
                    if let Some(ti) = thrust_in.as_deref_mut() {
                        ti.0 = *value;
                    }
                    if let Some(li) = last_input.as_deref_mut() {
                        li.thrust = *value;
                    }
                }
                (t, SystemControlPayload::SetSteering { value })
                    if t.as_str() == crate::system_registry::HELM_STEERING_SYSTEM_ID =>
                {
                    if let Some(si) = steering_in.as_deref_mut() {
                        si.0 = *value;
                    }
                    if let Some(li) = last_input.as_deref_mut() {
                        li.steering = *value;
                    }
                }
                (t, SystemControlPayload::LateralThrustInput { lateral })
                    if *t == crate::system_registry::lateral_thrust_system_id().0 =>
                {
                    if let Some(la) = lateral_in.as_deref_mut() {
                        la.0 = *lateral;
                    }
                    if let Some(li) = last_input.as_deref_mut() {
                        li.lateral = *lateral;
                    }
                }
                (t, SystemControlPayload::StartImpulseCharge)
                    if t.as_str() == crate::system_registry::HELM_IMPULSE_SYSTEM_ID =>
                {
                    if !entity_inside_blocks_impulse(entity, &membership, &region_query) {
                        if let Some(ic) = impulse_cmd.as_deref_mut() {
                            ic.0 = crate::impulse::ImpulsePhase::Charging;
                        }
                        // Zero the LocalShip's cached helm input the moment a
                        // charge is commanded, so a stale steering/thrust
                        // value can't resurface when impulse cancels or the
                        // autopilot disengages (the pre-#824 phase-edge
                        // detection, applied at the command rather than one
                        // tick later at the observed transition). A Set*
                        // admitted later in this same tick still overrides —
                        // the loop applies commands in admission order.
                        if is_local {
                            if let Some(li) = last_input.as_deref_mut() {
                                li.thrust = 0.0;
                                li.steering = 0.0;
                            }
                            if let Some(ti) = thrust_in.as_deref_mut() {
                                ti.0 = 0.0;
                            }
                            if let Some(si) = steering_in.as_deref_mut() {
                                si.0 = 0.0;
                            }
                        }
                    }
                }
                (t, SystemControlPayload::CancelImpulse)
                    if t.as_str() == crate::system_registry::HELM_IMPULSE_SYSTEM_ID =>
                {
                    if let Some(ic) = impulse_cmd.as_deref_mut() {
                        ic.0 = crate::impulse::ImpulsePhase::Idle;
                    }
                }
                _ => {}
            }
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

    // ── Ship-aware admission symmetry (issue #824) ─────────────────────────

    /// Spawn a minimal NPC ship the admission gate can route to: its own
    /// `AdmittedCommands`, control sources with `helm-thrust` on `source`,
    /// a `ShipConfigComponent`, and a `ThrustInput` intent for
    /// `process_helm_inputs` to land on. Registers `ai:<uuid>` in the
    /// `AiTokenRegistry` and returns `(entity, token)`.
    fn spawn_admission_npc(app: &mut App, source: ControlSource) -> (Entity, String) {
        let mut sources = ShipSystemControlSources::default();
        sources
            .0
            .set(crate::system_registry::helm_thrust_system_id(), source);
        let npc = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::ship::components::ShipConfigComponent::default(),
                sources,
                crate::messages::AdmittedCommands::default(),
                ThrustInput::default(),
            ))
            .id();
        let uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut()
            .resource_mut::<crate::ai::server::AiTokenRegistry>()
            .register_with_entity(&uuid, npc);
        (npc, format!("ai:{uuid}"))
    }

    fn thrust_input_of(app: &App, entity: Entity) -> f32 {
        app.world().entity(entity).get::<ThrustInput>().unwrap().0
    }

    /// AC (issue #824): a registered `ai:` token's `ControlSystem` resolves
    /// through `AiTokenRegistry` to the owning NPC entity and is admitted
    /// into THAT entity's `AdmittedCommands` — and the admitted command is
    /// applied to the NPC's own intent components, not the LocalShip's.
    #[test]
    fn ai_token_routes_to_owning_npc_entity_and_applies_to_its_intents() {
        let mut app = test_app();
        let (npc, token) = spawn_admission_npc(&mut app, ControlSource::Ai);

        push(
            &mut app,
            &token,
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.7 },
            },
        );
        tick(&mut app);

        assert_eq!(
            thrust_input_of(&app, npc),
            0.7,
            "the NPC's admitted AI command must apply to the NPC's own ThrustInput"
        );
        let local = find_ship_entity(&mut app);
        assert_eq!(
            thrust_input_of(&app, local),
            0.0,
            "the LocalShip's ThrustInput must be untouched by an NPC-routed command"
        );
    }

    /// AC (issue #824): a human token still routes to the LocalShip even
    /// with NPC ships present.
    #[test]
    fn human_token_still_routes_to_the_local_ship() {
        let mut app = test_app();
        start_game_with_helm_and_science(&mut app);
        let (npc, _token) = spawn_admission_npc(&mut app, ControlSource::Ai);

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.4 },
            },
        );
        tick_twice(&mut app);

        let local = find_ship_entity(&mut app);
        assert_eq!(
            thrust_input_of(&app, local),
            0.4,
            "the station holder's SetThrust must land on the LocalShip's intent"
        );
        assert_eq!(
            thrust_input_of(&app, npc),
            0.0,
            "a human command must never land on an NPC's intent"
        );
    }

    /// AC (issue #824): mismatched authority is rejected — an `ai:` token
    /// addressing a system the owning ship holds as Human is refused by that
    /// ship's own `ControlSourceResolver` at the gate.
    #[test]
    fn mismatched_authority_ai_token_is_rejected() {
        let mut app = test_app();
        let (npc, token) = spawn_admission_npc(&mut app, ControlSource::Human);

        push(
            &mut app,
            &token,
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_thrust_system_id(),
                payload: SystemControlPayload::SetThrust { value: 0.9 },
            },
        );
        tick(&mut app);

        assert_eq!(
            thrust_input_of(&app, npc),
            0.0,
            "an ai: token must be refused when the owning ship's helm-thrust is human-held"
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

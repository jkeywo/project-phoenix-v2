use bevy::prelude::*;

use crate::core::messages::{
    ActionFeedbackOutcome, AdmittedCommands, InterSystemMsg, InterSystemPayload, InterSystemQueue,
    SystemControlPayload,
};
use crate::regions::server::RegionMembership;
use crate::server_app::LocalShip;
use crate::server_app::ShipBoost;
use crate::ship::components::{BoostConfigResource, LastHelmInput, ShipSystemControlSources};
use crate::ship::helm::{
    BoostCommand, ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput,
    VerticalThrustInput,
};

// ── Systems ─────────────────────────────────────────────────────────────────

/// True when `entity` is inside a region whose authored effects include
/// `BlocksImpulse`. Per-entity generalisation of the LocalShip-only helper
/// in `impulse_boost_systems` (issue #824): the check is a property of the
/// ship's position, not of who commanded the charge.
fn entity_inside_blocks_impulse(
    entity: Entity,
    membership: &Option<Res<RegionMembership>>,
    region_query: &Query<&crate::entities::spawner::RegionEffectsSection>,
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
                .contains(&crate::regions::effects::RegionEffectKind::BlocksImpulse)
            {
                return true;
            }
        }
    }
    false
}

pub(crate) fn authored_system_id_for_kind<'a>(
    config: Option<&'a crate::ship::components::ShipConfigComponent>,
    kind: &str,
) -> Option<&'a crate::core::messages::SystemId> {
    config.and_then(|config| {
        config
            .0
            .systems
            .iter()
            .find(|system| system.kind == kind)
            .map(|system| &system.id)
    })
}

fn target_is_owner(
    target: &str,
    authored: Option<&crate::core::messages::SystemId>,
    legacy_id: &str,
) -> bool {
    authored.map_or(target == legacy_id, |system_id| target == system_id.0)
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
/// `publish_joystick_to_engines` read it for the player ship,
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
///
/// Boost `SetBoost`/`ToggleBoost` are applied here for every ship too (issue
/// #881), following the same retirement #824 did for impulse: the old
/// `handle_boost_messages` was `With<LocalShip>` and read a single entity, so
/// the admitted `SetBoost` `ai_helm_boost` (#780) emits for a *non-local*
/// `AiHighFidelity` NPC was admitted and then silently dropped. `BoostCommand`
/// now has exactly one writer, and nothing below admission branches on origin
/// (AGENTS.md #6). The `enabled` guard is unchanged — an absent or
/// feature-disabled `BoostConfigResource` means the hull authors no boost, and
/// the payload is ignored.
pub(crate) fn process_helm_inputs(
    membership: Option<Res<RegionMembership>>,
    region_query: Query<&crate::entities::spawner::RegionEffectsSection>,
    mut ships: Query<(
        Entity,
        &AdmittedCommands,
        Option<&mut LastHelmInput>,
        Option<&mut ThrustInput>,
        Option<&mut SteeringInput>,
        Option<&mut LateralThrustInput>,
        Option<&mut VerticalThrustInput>,
        Option<&mut ImpulseCommand>,
        Option<&mut BoostCommand>,
        Option<&mut crate::ship::helm::DriveCommandWrites>,
        Option<&BoostConfigResource>,
        Option<&ShipBoost>,
        Option<&crate::ship::components::ShipConfigComponent>,
        Option<&ShipSystemControlSources>,
        Has<LocalShip>,
    )>,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    for (
        entity,
        admitted,
        last_input,
        mut thrust_in,
        mut steering_in,
        mut lateral_in,
        mut vertical_in,
        mut impulse_cmd,
        mut boost_cmd,
        mut drive_writes,
        boost_cfg,
        ship_boost,
        ship_config,
        sources,
        is_local,
    ) in ships.iter_mut()
    {
        let mut last_input = if is_local { last_input } else { None };
        let thrust_system_id = authored_system_id_for_kind(
            ship_config,
            crate::ship::system_registry::HELM_THRUST_KIND,
        );
        let steering_system_id = authored_system_id_for_kind(
            ship_config,
            crate::ship::system_registry::HELM_STEERING_KIND,
        );
        let lateral_system_id = authored_system_id_for_kind(
            ship_config,
            crate::ship::system_registry::LATERAL_THRUST_KIND,
        );
        let vertical_system_id = authored_system_id_for_kind(
            ship_config,
            crate::ship::system_registry::VERTICAL_THRUST_KIND,
        );
        let impulse_system_id = authored_system_id_for_kind(
            ship_config,
            crate::ship::system_registry::HELM_IMPULSE_KIND,
        );
        let boost_system_id =
            authored_system_id_for_kind(ship_config, crate::ship::system_registry::HELM_BOOST_KIND);

        // ── Offline-axis latch clear (issue #968) ─────────────────────────
        // The four helm intent components are LATCHES: written on admission and
        // never otherwise reset. `integrate_ship_physics` gates each axis on its
        // system being online, which stops a destroyed actuator from acting —
        // but a gate only MASKS the latched fraction. The value survives in the
        // component (`snapshot.rs` even serialises it), so the tick a repair
        // lifts the tier back out of `Disabled` the stale command is applied
        // again, up to a full AI decision period (30 Hz) before the axis owner
        // gets round to writing a fresh one. A ship that lost its thruster
        // mid-strafe should not resume that strafe the instant the damage
        // control party finishes.
        //
        // So the axis OWNER clears its own intent while the axis is offline.
        // This runs ahead of the admitted-command loop below, so a command that
        // did make it through admission this tick still wins — admission is the
        // authority on what may be commanded, and this is only about what an
        // axis that may command NOTHING is left holding. `Offline` refuses both
        // human input and AI operation, so in practice nothing overrides it.
        //
        // `LastHelmInput` is deliberately untouched: it is the LocalShip's HUD
        // mirror of what the human asked for, not an actuator command.
        if let Some(sources) = sources {
            if thrust_system_id.map_or_else(
                || {
                    sources
                        .0
                        .is_offline(&crate::ship::system_registry::helm_thrust_system_id())
                },
                |system_id| sources.0.is_offline(system_id),
            ) {
                if let Some(ti) = thrust_in.as_deref_mut() {
                    ti.0 = 0.0;
                }
            }
            if steering_system_id.map_or_else(
                || {
                    sources
                        .0
                        .is_offline(&crate::ship::system_registry::helm_steering_system_id())
                },
                |system_id| sources.0.is_offline(system_id),
            ) {
                if let Some(si) = steering_in.as_deref_mut() {
                    si.0 = 0.0;
                }
            }
            if lateral_system_id.map_or_else(
                || {
                    sources
                        .0
                        .is_offline(&crate::ship::system_registry::lateral_thrust_system_id())
                },
                |system_id| sources.0.is_offline(system_id),
            ) {
                if let Some(la) = lateral_in.as_deref_mut() {
                    la.0 = 0.0;
                }
            }
            if vertical_system_id.map_or_else(
                || {
                    sources
                        .0
                        .is_offline(&crate::ship::system_registry::vertical_thrust_system_id())
                },
                |system_id| sources.0.is_offline(system_id),
            ) {
                if let Some(vi) = vertical_in.as_deref_mut() {
                    vi.0 = 0.0;
                }
            }
        }

        if admitted.0.is_empty() {
            continue;
        }

        // Boost capability gate, evaluated once per ship: a hull that authors no
        // `[helm_console.boost]` (or authors it disabled) has no boost system to
        // command. Mirrors the retired `handle_boost_messages` guard exactly.
        let boost_enabled = boost_cfg.map(|c| c.enabled).unwrap_or(false);
        // Accumulated across this tick's admitted boost payloads so a
        // `ToggleBoost` pair cancels out, then written ONCE at the end — the
        // retired applier's shape. Left `None` when no boost payload was
        // admitted so `BoostCommand` is not touched, which matters:
        // `apply_helm_commands` transitions on `Ref::is_changed`, and a
        // write-every-tick would fight code that sets `ShipBoost` directly.
        let mut desired_boost: Option<bool> = None;

        for cmd in admitted.0.iter() {
            match (&cmd.target.0, &cmd.payload) {
                (t, SystemControlPayload::SetThrust { value })
                    if target_is_owner(
                        t,
                        thrust_system_id,
                        crate::ship::system_registry::HELM_THRUST_SYSTEM_ID,
                    ) =>
                {
                    if let Some(ti) = thrust_in.as_deref_mut() {
                        ti.0 = *value;
                    }
                    if let Some(li) = last_input.as_deref_mut() {
                        li.thrust = *value;
                    }
                }
                (t, SystemControlPayload::SetSteering { value })
                    if target_is_owner(
                        t,
                        steering_system_id,
                        crate::ship::system_registry::HELM_STEERING_SYSTEM_ID,
                    ) =>
                {
                    if let Some(si) = steering_in.as_deref_mut() {
                        si.0 = *value;
                    }
                    if let Some(li) = last_input.as_deref_mut() {
                        li.steering = *value;
                    }
                }
                (t, SystemControlPayload::LateralThrustInput { lateral })
                    if target_is_owner(
                        t,
                        lateral_system_id,
                        crate::ship::system_registry::LATERAL_THRUST_SYSTEM_ID,
                    ) =>
                {
                    if let Some(la) = lateral_in.as_deref_mut() {
                        la.0 = *lateral;
                    }
                    if let Some(li) = last_input.as_deref_mut() {
                        li.lateral = *lateral;
                    }
                }
                // Vertical thrust (issue #744): AI-only, so no `LastHelmInput`
                // mirror (that HUD cache carries no vertical field).
                (t, SystemControlPayload::VerticalThrustInput { vertical })
                    if target_is_owner(
                        t,
                        vertical_system_id,
                        crate::ship::system_registry::VERTICAL_THRUST_SYSTEM_ID,
                    ) =>
                {
                    if let Some(vi) = vertical_in.as_deref_mut() {
                        vi.0 = *vertical;
                    }
                }
                (t, SystemControlPayload::StartImpulseCharge)
                    if target_is_owner(
                        t,
                        impulse_system_id,
                        crate::ship::system_registry::HELM_IMPULSE_SYSTEM_ID,
                    ) =>
                {
                    let blocked = entity_inside_blocks_impulse(entity, &membership, &region_query);
                    let outcome = if !blocked {
                        if let Some(ic) = impulse_cmd.as_deref_mut() {
                            ic.0 = crate::ship::impulse::ImpulsePhase::Charging;
                            if let Some(writes) = drive_writes.as_deref_mut() {
                                writes.impulse = true;
                            }
                            // Clear authoritative actuator latches on EVERY
                            // ship, so remote peers cannot retain old AI inputs
                            // during charge. Only the display cache is local.
                            // A later admitted Set* still overrides in order.
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
                            ActionFeedbackOutcome::Applied
                        } else {
                            ActionFeedbackOutcome::Refused
                        }
                    } else {
                        ActionFeedbackOutcome::Refused
                    };
                    crate::command_admission::finish_action_feedback(cmd, &mut outbound, outcome);
                }
                (t, SystemControlPayload::CancelImpulse)
                    if target_is_owner(
                        t,
                        impulse_system_id,
                        crate::ship::system_registry::HELM_IMPULSE_SYSTEM_ID,
                    ) =>
                {
                    let outcome = if let Some(ic) = impulse_cmd.as_deref_mut() {
                        ic.0 = crate::ship::impulse::ImpulsePhase::Idle;
                        if let Some(writes) = drive_writes.as_deref_mut() {
                            writes.impulse = true;
                        }
                        ActionFeedbackOutcome::Applied
                    } else {
                        ActionFeedbackOutcome::Refused
                    };
                    crate::command_admission::finish_action_feedback(cmd, &mut outbound, outcome);
                }
                (t, SystemControlPayload::SetBoost { active })
                    if target_is_owner(
                        t,
                        boost_system_id,
                        crate::ship::system_registry::HELM_BOOST_SYSTEM_ID,
                    ) =>
                {
                    let outcome = if boost_enabled && boost_cmd.is_some() {
                        desired_boost = Some(*active);
                        ActionFeedbackOutcome::Applied
                    } else {
                        ActionFeedbackOutcome::Refused
                    };
                    crate::command_admission::finish_action_feedback(cmd, &mut outbound, outcome);
                }
                (t, SystemControlPayload::ToggleBoost)
                    if target_is_owner(
                        t,
                        boost_system_id,
                        crate::ship::system_registry::HELM_BOOST_SYSTEM_ID,
                    ) =>
                {
                    let outcome = if boost_enabled && boost_cmd.is_some() {
                        // Read-modify-write against this ship's live `ShipBoost`,
                        // or against an earlier payload in the same tick.
                        let current = desired_boost.unwrap_or_else(|| {
                            ship_boost.map(|b| b.0.is_active()).unwrap_or(false)
                        });
                        desired_boost = Some(!current);
                        ActionFeedbackOutcome::Applied
                    } else {
                        ActionFeedbackOutcome::Refused
                    };
                    crate::command_admission::finish_action_feedback(cmd, &mut outbound, outcome);
                }
                _ => {}
            }
        }

        if let Some(active) = desired_boost {
            if let Some(bc) = boost_cmd.as_deref_mut() {
                bc.0 = active;
                if let Some(writes) = drive_writes.as_deref_mut() {
                    writes.boost = true;
                }
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
            .policy_for(&crate::ship::system_registry::helm_joystick_system_id());
        // Only publish when the joystick system can operate (human or AI).
        if !policy.accept_human_input && !policy.operate_ai {
            continue;
        }
        let port_id = crate::ship::system_registry::helm_engine_port_system_id();
        let stbd_id = crate::ship::system_registry::helm_engine_starboard_system_id();
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
            .policy_for(&crate::ship::system_registry::helm_joystick_system_id());
        let joystick_publishing = joystick_policy.accept_human_input || joystick_policy.operate_ai;
        if joystick_publishing {
            // `publish_joystick_to_engines` will cover both engines this tick.
            continue;
        }

        let port_policy = sources
            .0
            .policy_for(&crate::ship::system_registry::helm_engine_port_system_id());
        let stbd_policy = sources
            .0
            .policy_for(&crate::ship::system_registry::helm_engine_starboard_system_id());

        // Joystick is offline; push for any engine that is still AI-operable.
        if port_policy.operate_ai {
            inter_system.0.push(InterSystemMsg {
                target: crate::ship::system_registry::helm_engine_port_system_id(),
                payload: InterSystemPayload::JoystickState {
                    thrust: last_input.thrust,
                    steering: last_input.steering,
                },
                source_entity: None,
            });
        }
        if stbd_policy.operate_ai {
            inter_system.0.push(InterSystemMsg {
                target: crate::ship::system_registry::helm_engine_starboard_system_id(),
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
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "helm_admission_tests.rs"]
mod tests;

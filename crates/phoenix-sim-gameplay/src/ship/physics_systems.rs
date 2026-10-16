use bevy::prelude::*;

use crate::core::messages::ModifierSlot;
use crate::modifiers::ShipModifiers;
use crate::server_app::{ShipBoost, ShipImpulse};
use crate::ship::components::{
    BankConfigResource, BoostConfigResource, ImpulseConfigResource, ShipPhysicsConfigResource,
    ShipSystemControlSources, HELM_AI_MAX_DT_SECS,
};
use crate::ship::helm::{
    BoostCommand, ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput,
    VerticalThrustInput,
};
use crate::ship::physics::{
    compute_physics, compute_physics_with_axis_modes, ShipPhysicsConfig, ShipPhysicsInput,
    ShipPhysicsState,
};
use crate::ship::state::ShipPhysics;

fn movement_axis_online(
    sources: &ShipSystemControlSources,
    owner: &crate::core::messages::SystemId,
) -> bool {
    sources.0.entries().any(|(id, _)| id == owner)
        && !sources.0.is_offline(owner)
        && sources.0.source_for(owner) != crate::ship::control_source::ControlSource::Offline
}

fn boost_work_fraction(
    sources: &ShipSystemControlSources,
    damage: Option<&crate::ship::physical_damage::ShipDamage>,
    power: Option<&mut crate::modifiers::power_system::PowerSystem>,
    dt: f32,
    activity: f32,
) -> f32 {
    if !crate::ship::impulse_boost_systems::boost_can_operate(
        Some(sources),
        damage,
        power.as_deref(),
    ) || activity <= 0.0
    {
        return 0.0;
    }
    let power = power.expect("operable boost has its actual consumer");
    let owner = crate::ship::impulse_boost_systems::boost_owner(power)
        .expect("operable boost has one owner")
        .clone();
    let operation = damage.map_or(1.0, |damage| {
        damage.factor(&owner, phoenix_model::damage::DamageBinding::Operation)
    });
    power
        .work_continuous(&owner, dt, activity)
        .map_or(0.0, |grant| grant.fraction)
        * operation
}

/// Same physical rotation used by sync, exact contact and attachment restore.
pub fn physical_rotation(physics: &ShipPhysics) -> Quat {
    Quat::from_euler(EulerRot::YXZ, -physics.yaw, 0.0, physics.roll)
}

/// A rigid relation is a projection, not another integration of velocity.
pub fn marking_attached_pose(subject: &ShipPhysics, offset: [f32; 3]) -> ShipPhysics {
    let point = Vec3::new(subject.x, subject.y, subject.z)
        + physical_rotation(subject) * Vec3::from_array(offset);
    ShipPhysics {
        x: point.x,
        y: point.y,
        z: point.z,
        ..*subject
    }
}

/// Sole attachment pose owner, called before physical sync and after Dock's
/// final placement. Missing observations never detach; only actual body loss does.
#[allow(clippy::type_complexity)]
pub fn project_marking_attachments(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut bodies: ParamSet<(
        Query<
            (&crate::entities::spawner::EntityUuid, &ShipPhysics),
            (
                Without<crate::weapons::marking::ShipMarkingAttachment>,
                bevy::ecs::query::Allow<bevy::ecs::entity_disabling::Disabled>,
            ),
        >,
        Query<(
            &mut ShipPhysics,
            &mut Transform,
            &mut crate::weapons::marking::ShipMarkingAttachment,
        )>,
    )>,
) {
    let Some(tick) = tick else { return };
    let parents: std::collections::BTreeMap<_, _> = bodies
        .p0()
        .iter()
        .map(|(id, pose)| (id.0.clone(), *pose))
        .collect();
    for (mut pose, mut transform, mut attachment) in &mut bodies.p1() {
        let phoenix_model::marking::MarkingLocation::Attached { subject, offset } =
            &attachment.0.location
        else {
            continue;
        };
        if let Some(parent) = parents.get(subject) {
            *pose = marking_attached_pose(parent, *offset);
            transform.translation = Vec3::new(pose.x, pose.y, pose.z);
            transform.rotation = physical_rotation(&pose);
        } else {
            attachment.0.location =
                phoenix_model::marking::MarkingLocation::Detached { tick: tick.0 };
            pose.forward_speed = 0.0;
            pose.lateral_speed = 0.0;
            pose.vertical_speed = 0.0;
        }
    }
}

/// Actual removal transaction uses the same relation owner. This cannot infer
/// removal from missing information and never moves the last projected body.
pub fn detach_marking_subject(world: &mut World, subject: &str, tick: u64) {
    let mut markers = world.query_filtered::<(
        &mut ShipPhysics,
        &mut crate::weapons::marking::ShipMarkingAttachment,
    ), bevy::ecs::query::Allow<bevy::ecs::entity_disabling::Disabled>>();
    for (mut pose, mut marker) in markers.iter_mut(world) {
        if matches!(&marker.0.location, phoenix_model::marking::MarkingLocation::Attached { subject: parent, .. } if parent == subject)
        {
            marker.0.location = phoenix_model::marking::MarkingLocation::Detached { tick };
            pose.forward_speed = 0.0;
            pose.lateral_speed = 0.0;
            pose.vertical_speed = 0.0;
        }
    }
}

pub fn sync_ship_position(mut ship_query: Query<(&ShipPhysics, &mut Transform)>) {
    for (physics, mut transform) in ship_query.iter_mut() {
        transform.translation.x = physics.x;
        transform.translation.y = physics.y;
        transform.translation.z = physics.z;
        transform.rotation = physical_rotation(physics);
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
/// Uses explicit real-write markers (including insertion ticks), plus change
/// detection for existing non-added direct intents, rather than unconditionally
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
pub fn apply_helm_commands(
    mut ships: Query<
        (
            Option<&mut ShipImpulse>,
            Option<Ref<ImpulseCommand>>,
            Option<&mut ShipBoost>,
            Option<Ref<BoostCommand>>,
            Option<&mut crate::ship::helm::DriveCommandWrites>,
            Option<&ShipSystemControlSources>,
            Option<&crate::ship::components::ShipConfigComponent>,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
) {
    for (impulse, impulse_cmd, boost, boost_cmd, writes, sources, config) in ships.iter_mut() {
        // Consume even when the drive is unavailable. A later enable must not
        // replay an intent whose original application was already handled.
        let writes = writes
            .map(|mut writes| std::mem::take(&mut *writes))
            .unwrap_or_default();
        if let (Some(mut impulse), Some(cmd)) = (impulse, impulse_cmd) {
            // Fresh LOD defaults remain inert, but real damage/admitted writes
            // on that same insertion tick must apply. Keep change detection
            // for existing non-added direct-intent producers and fixtures.
            if !crate::ship::impulse_boost_systems::drive_available(
                sources,
                config,
                crate::ship::system_registry::HELM_IMPULSE_KIND,
                crate::ship::system_registry::helm_impulse_system_id(),
            ) {
                impulse.0.cancel_charge();
            } else if writes.impulse || (cmd.is_changed() && !cmd.is_added()) {
                match cmd.0 {
                    crate::ship::impulse::ImpulsePhase::Charging => impulse.0.start_charge(),
                    crate::ship::impulse::ImpulsePhase::Idle => impulse.0.cancel_charge(),
                    crate::ship::impulse::ImpulsePhase::Active => {}
                }
            }
        }
        if let (Some(mut boost), Some(cmd)) = (boost, boost_cmd) {
            // Same insertion-tick exclusion as above. Since issue #881 this
            // is live for NPCs too: `process_helm_inputs` applies an admitted
            // `SetBoost`/`ToggleBoost` for every ship, so a non-local
            // `AiHighFidelity` NPC's boost policy engages here in the same
            // tick it was decided.
            if !crate::ship::impulse_boost_systems::drive_available(
                sources,
                config,
                crate::ship::system_registry::HELM_BOOST_KIND,
                crate::ship::system_registry::helm_boost_system_id(),
            ) {
                boost.0.deactivate();
            } else if writes.boost || (cmd.is_changed() && !cmd.is_added()) {
                if cmd.0 {
                    boost.0.active = true;
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
/// Visual banking/roll runs for EVERY ship since issue #1116. `ShipPhysics.roll`
/// is folded into the authoritative digest, so gating it on `LocalShip` made a
/// folded value depend on which hull a host happens to project — see the site
/// itself for why banking every ship is the fix rather than narrowing the fold.
///
/// This system is the only *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`, enforced in debug
/// builds by `HelmPhysicsWriteGuard`. It is not the only writer of those
/// fields overall — see the sanctioned-exception table on `ShipPhysics`
/// (`crates/phoenix-sim-gameplay/src/ship/state.rs`) for the four out-of-band writers.
#[allow(clippy::too_many_arguments)]
pub fn integrate_ship_physics(
    physical_q: Query<&crate::ship::physical_damage::ShipDamage>,
    time: Res<Time>,
    logical_tick: Option<Res<crate::sim_tick::SimTick>>,
    physics_cfg_res: Option<Res<ShipPhysicsConfigResource>>,
    bank_cfg_res: Option<Res<BankConfigResource>>,
    mut power_q: Query<&mut crate::ship::power::ShipPowerSystem>,
    mut ships: Query<
        (
            Entity,
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
            (
                Option<&crate::ship::helm_control::CompiledHelmControl>,
                Option<&crate::ship::helm_control::HelmControlState>,
                Option<&mut crate::ship::impulse_boost_systems::BoostActuation>,
                Option<&crate::ship::helm_control::HelmNavigationInputs>,
            ),
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
    #[cfg(debug_assertions)] frame: Res<crate::ship::helm::HelmPhysicsFrame>,
    #[cfg(debug_assertions)] mut guard_q: Query<&mut crate::ship::helm::HelmPhysicsWriteGuard>,
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
        (movement_profile, movement_state, mut boost_actuation, navigation_inputs),
    ) in ships.iter_mut()
    {
        // Debug-only single-writer tripwire (issue #699). The guard arrives
        // with `AiHighFidelity` itself (`#[require]`, issue #1051), so this
        // loop — whose query is `With<AiHighFidelity>` — is structurally
        // guaranteed to find one, and there is no `Commands::insert` here to
        // move a ship's archetype mid-run. It used to self-heal instead, which
        // made every debug build create an archetype no release build ever did
        // and moved the authoritative digest across build profiles; see
        // `ai::server::AiHighFidelity` for the whole story.
        #[cfg(debug_assertions)]
        {
            let entity = _entity;
            guard_q
                .get_mut(entity)
                .expect(
                    "AiHighFidelity requires HelmPhysicsWriteGuard, so every ship this \
                     system iterates must already carry one",
                )
                .record_write(entity, "integrate_ship_physics", frame.0);
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

        if let Some(actuation) = boost_actuation.as_deref_mut() {
            actuation.0 = 0.0;
        }
        let physical = physical_q.get(_entity).ok();
        let mut power = power_q.get_mut(_entity).ok();
        // Configured hulls have exactly one retained intent map. Missing state
        // must never fall back to compatibility stick latches.
        if movement_profile.is_some() != movement_state.is_some() {
            continue;
        }
        let impulse_requested = impulse.is_some_and(|i| i.0.is_active())
            && !sources
                .0
                .is_offline(&crate::ship::system_registry::helm_thrust_system_id())
            && !(sources
                .0
                .is_offline(&crate::ship::system_registry::helm_engine_port_system_id())
                && sources
                    .0
                    .is_offline(&crate::ship::system_registry::helm_engine_starboard_system_id()));
        let impulse_fraction = if impulse_requested {
            crate::ship::energy_work::continuous_for_kind(
                power.as_deref_mut().map(|p| &mut p.0),
                crate::ship::system_registry::HELM_IMPULSE_KIND,
                dt,
                1.0,
            )
        } else {
            0.0
        };
        let impulse_fraction = impulse_fraction
            * power.as_deref().map_or(0.0, |power| {
                crate::ship::physical_damage::operation_for_kind(
                    physical,
                    power.0.ship_config(),
                    crate::ship::system_registry::HELM_IMPULSE_KIND,
                )
            });
        let impulse_active = impulse_requested && impulse_fraction > 0.0;

        let navigation_inputs = navigation_inputs
            .and_then(|input| input.current(logical_tick.as_ref().map(|tick| tick.0)));
        let mut movement_config = None;
        let mut movement_resolved = None;
        if !impulse_active {
            if let (Some(profile), Some(intent)) = (movement_profile, movement_state) {
                let mut config = physics_cfg_comp
                    .map(|c| c.0)
                    .or_else(|| physics_cfg_res.as_deref().map(|c| c.0))
                    .unwrap_or_else(ShipPhysicsConfig::new);
                config.max_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
                config.max_reverse_speed *= modifiers.get(&ModifierSlot::MaxSpeed);
                config.max_yaw_rate *= modifiers.get(&ModifierSlot::MaxYawRate);
                let Some(request) =
                    intent.resolve(&profile.0, state, dt, &config, navigation_inputs)
                else {
                    continue;
                };
                let boost_cfg = boost_cfg_comp.cloned().unwrap_or_default();
                if boost_cfg.enabled && boost.is_some_and(|boost| boost.0.is_active()) {
                    let mut potential = config;
                    potential.max_speed *= boost_cfg.multiplier;
                    potential.max_reverse_speed *= boost_cfg.multiplier;
                    potential.acceleration *= boost_cfg.multiplier;
                    potential.max_yaw_rate *= boost_cfg.steering_multiplier;
                    let request = intent
                        .resolve(&profile.0, state, dt, &potential, navigation_inputs)
                        .unwrap_or(request);
                    let activity = if movement_axis_online(sources, &profile.0.thrust) {
                        request.thrust.abs()
                    } else {
                        0.0
                    } + if movement_axis_online(sources, &profile.0.steering) {
                        request.steering.abs()
                    } else {
                        0.0
                    };
                    let fraction = boost_work_fraction(
                        sources,
                        physical,
                        power.as_deref_mut().map(|power| &mut power.0),
                        dt,
                        activity,
                    );
                    if let Some(actuation) = boost_actuation.as_deref_mut() {
                        actuation.0 = fraction;
                    }
                    let multiplier = 1.0 + (boost_cfg.multiplier - 1.0) * fraction;
                    config.max_speed *= multiplier;
                    config.max_reverse_speed *= multiplier;
                    config.acceleration *= multiplier;
                    config.max_yaw_rate *= 1.0 + (boost_cfg.steering_multiplier - 1.0) * fraction;
                }
                let Some(request) =
                    intent.resolve(&profile.0, state, dt, &config, navigation_inputs)
                else {
                    continue;
                };
                movement_config = Some(config);
                movement_resolved = Some(request);
            }
        }
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
        } else if let Some(request) = movement_resolved {
            ShipPhysicsInput {
                thrust: request.thrust,
                steering: request.steering,
                lateral: request.lateral.map_or(lateral_in.0, |(value, _)| value),
                vertical: vertical_in.0,
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
            .is_offline(&crate::ship::system_registry::helm_engine_port_system_id());
        let stbd_offline = sources
            .0
            .is_offline(&crate::ship::system_registry::helm_engine_starboard_system_id());
        let engine_thrust_scale: f32 = match (port_offline, stbd_offline) {
            (true, true) => 0.0,
            (true, false) | (false, true) => 0.5,
            (false, false) => 1.0,
        };

        // ── Destroyed-actuator gate (issue #968) ───────────────────────────
        // FOUR axes, one rule: an actuator that is damage-offline commands
        // nothing. The engine scaling above has always existed; none of
        // `helm-thrust`, `helm-steering`, `helm-lateral-thrust` or
        // `helm-vertical-thrust` had an equivalent, and that asymmetry was a
        // mission-ending bug.
        //
        // Every one of `ThrustInput` / `SteeringInput` / `LateralThrustInput` /
        // `VerticalThrustInput` is a LATCHED intent component of the same shape:
        // `process_helm_inputs` writes it when a command is admitted and nothing
        // clears it otherwise, and the per-axis AI operator `continue`s the
        // moment its own system goes offline (`ControlSource::Offline` ⇒
        // `operate_ai: false` — `ai_helm_thrust` and `ai_helm_steering` gate on
        // exactly the same expression as `ai_helm_lateral`). So a hull whose
        // actuator was shot away kept applying whatever fraction it last
        // commanded, for ever — measured on `combat_test`: the destroyer lost
        // `helm-lateral-thrust` (and both engines) at t=285.75 s and then strafed
        // at a fixed 8.77 u/s for the remaining 300 s of the run, out of the belt
        // and away from every hostile, its hazard surface reporting real
        // repulsion the whole time and nothing able to act on it.
        //
        // The lateral axis is the one that was observed, but it is the mildest of
        // the three that shipped content can reach: a latched `SteeringInput`
        // circles the hull out of its
        // scenario on a fixed yaw rate, and a Destroyed `helm-thrust` with intact
        // engines cruises straight off the map at whatever throttle it last held.
        // Both of those are live in shipped content — every Alliance and Harrow
        // hull declares `helm-thrust` and `helm-steering` `[[system]]`s. The
        // vertical arm is the only one that is inert today: no shipped hull
        // declares a `helm-vertical-thrust` system, and `sync_console_damage_tiers`
        // only flips SystemIds that are present in `EntitySystemHull`, so nothing
        // can put that id into the offline set until a hull authors it. It is
        // written here anyway so the axis arrives already covered.
        //
        // `0.0` input rather than a zeroed speed, so the hull coasts down on its
        // authored acceleration instead of stopping dead. This gate is the
        // capability check; `process_helm_inputs` separately CLEARS the latch
        // while the axis is offline so the stale fraction cannot survive a
        // repair — see the note there.
        let thrust_offline = movement_profile.map_or_else(
            || {
                sources
                    .0
                    .is_offline(&crate::ship::system_registry::helm_thrust_system_id())
            },
            |profile| !movement_axis_online(sources, &profile.0.thrust),
        );
        let steering_offline = movement_profile.map_or_else(
            || {
                sources
                    .0
                    .is_offline(&crate::ship::system_registry::helm_steering_system_id())
            },
            |profile| !movement_axis_online(sources, &profile.0.steering),
        );
        let lateral_offline = movement_profile
            .and_then(|profile| profile.0.lateral.as_ref())
            .map_or_else(
                || {
                    sources
                        .0
                        .is_offline(&crate::ship::system_registry::lateral_thrust_system_id())
                },
                |owner| !movement_axis_online(sources, owner),
            );
        let vertical_offline = sources
            .0
            .is_offline(&crate::ship::system_registry::vertical_thrust_system_id());
        let mut scaled_input = ShipPhysicsInput {
            thrust: if thrust_offline {
                0.0
            } else {
                input.thrust * engine_thrust_scale
            },
            steering: if steering_offline {
                0.0
            } else {
                input.steering
            },
            lateral: if lateral_offline { 0.0 } else { input.lateral },
            vertical: if vertical_offline {
                0.0
            } else {
                input.vertical
            },
        };

        // Pay only the admitted, damage-available actuator work. These fractions
        // constrain this tick's work; incoming allocation never changes output.
        for (axis, kind) in [
            (
                &mut scaled_input.thrust,
                crate::ship::system_registry::HELM_THRUST_KIND,
            ),
            (
                &mut scaled_input.steering,
                crate::ship::system_registry::HELM_STEERING_KIND,
            ),
            (
                &mut scaled_input.lateral,
                crate::ship::system_registry::LATERAL_THRUST_KIND,
            ),
            (
                &mut scaled_input.vertical,
                crate::ship::system_registry::VERTICAL_THRUST_KIND,
            ),
        ] {
            if *axis != 0.0 {
                let owner = movement_profile.and_then(|profile| match kind {
                    crate::ship::system_registry::HELM_THRUST_KIND => Some(&profile.0.thrust),
                    crate::ship::system_registry::HELM_STEERING_KIND => Some(&profile.0.steering),
                    crate::ship::system_registry::LATERAL_THRUST_KIND => profile.0.lateral.as_ref(),
                    _ => None,
                });
                *axis *= if let Some(owner) = owner {
                    power.as_deref_mut().map_or(0.0, |power| {
                        power
                            .0
                            .work_continuous(owner, dt, axis.abs().min(1.0))
                            .map_or(0.0, |grant| grant.fraction)
                    })
                } else {
                    crate::ship::energy_work::continuous_for_kind(
                        power.as_deref_mut().map(|p| &mut p.0),
                        kind,
                        dt,
                        axis.abs().min(1.0),
                    )
                };
            }
        }

        if let Some(power) = power.as_deref() {
            for (axis, kind) in [
                (
                    &mut scaled_input.thrust,
                    crate::ship::system_registry::HELM_THRUST_KIND,
                ),
                (
                    &mut scaled_input.steering,
                    crate::ship::system_registry::HELM_STEERING_KIND,
                ),
                (
                    &mut scaled_input.lateral,
                    crate::ship::system_registry::LATERAL_THRUST_KIND,
                ),
                (
                    &mut scaled_input.vertical,
                    crate::ship::system_registry::VERTICAL_THRUST_KIND,
                ),
            ] {
                let owner = movement_profile.and_then(|profile| match kind {
                    crate::ship::system_registry::HELM_THRUST_KIND => Some(&profile.0.thrust),
                    crate::ship::system_registry::HELM_STEERING_KIND => Some(&profile.0.steering),
                    crate::ship::system_registry::LATERAL_THRUST_KIND => profile.0.lateral.as_ref(),
                    _ => None,
                });
                *axis *= if let Some(owner) = owner {
                    physical.map_or(0.0, |physical| {
                        physical.factor(owner, phoenix_model::damage::DamageBinding::Operation)
                    })
                } else {
                    crate::ship::physical_damage::operation_for_kind(
                        physical,
                        power.0.ship_config(),
                        kind,
                    )
                };
            }
        }
        if let Some(physical) = physical {
            let mut factors = Vec::new();
            for (id, offline) in [
                (
                    crate::ship::system_registry::helm_engine_port_system_id(),
                    port_offline,
                ),
                (
                    crate::ship::system_registry::helm_engine_starboard_system_id(),
                    stbd_offline,
                ),
            ] {
                if !offline && physical.definition.components.contains_key(&id) {
                    factors.push(
                        physical.factor(&id, phoenix_model::damage::DamageBinding::Operation),
                    );
                }
            }
            if !factors.is_empty() {
                scaled_input.thrust *= factors.iter().sum::<f32>() / factors.len() as f32;
            }
        }

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
            let speed_mult = if impulse_cfg.speed_multiplier > 0.0 {
                impulse_cfg.speed_multiplier
            } else {
                crate::ship::impulse::IMPULSE_SPEED_MULTIPLIER
            };
            let funded_speed = 1.0 + (speed_mult - 1.0) * impulse_fraction;
            config.max_speed *= funded_speed;
            config.max_reverse_speed *= funded_speed;
            let mult = if impulse_cfg.acceleration_multiplier > 0.0 {
                impulse_cfg.acceleration_multiplier
            } else {
                crate::ship::impulse::IMPULSE_ACCELERATION_MULTIPLIER
            };
            config.acceleration *= 1.0 + (mult - 1.0) * impulse_fraction;
        }

        // Boost drive: while engaged, multiply max speed and acceleration.
        // Only applies when the ship's TOML enabled the feature.
        let boost_cfg = boost_cfg_comp.cloned().unwrap_or_default();
        let boost_active = boost.map(|b| b.0.is_active()).unwrap_or(false);
        if movement_config.is_none() && boost_cfg.enabled && boost_active {
            let activity = scaled_input.thrust.abs() + scaled_input.steering.abs();
            let fraction = boost_work_fraction(
                sources,
                physical,
                power.as_deref_mut().map(|power| &mut power.0),
                dt,
                activity,
            );
            if let Some(actuation) = boost_actuation.as_deref_mut() {
                actuation.0 = fraction;
            }
            let multiplier = 1.0 + (boost_cfg.multiplier - 1.0) * fraction;
            config.max_speed *= multiplier;
            config.max_reverse_speed *= multiplier;
            config.acceleration *= multiplier;
            config.max_yaw_rate *= 1.0 + (boost_cfg.steering_multiplier - 1.0) * fraction;
        }

        let result = if let (Some(config), Some(request)) = (movement_config, movement_resolved) {
            compute_physics_with_axis_modes(
                state,
                scaled_input,
                dt,
                &config,
                request.drive,
                request.lateral.is_some_and(|(_, direct)| direct),
                request.steering_direct,
            )
        } else {
            compute_physics(state, scaled_input, dt, &config)
        };

        physics.x = result.x;
        physics.y = result.y;
        physics.z = result.z;
        physics.yaw = result.yaw;
        physics.forward_speed = result.forward_speed;
        physics.lateral_speed = result.lateral_speed;
        physics.vertical_speed = result.vertical_speed;

        // Banking, for EVERY ship (issue #1116, was `LocalShip` only).
        //
        // This looks like a presentation detail and is not one: `roll` is a
        // `ShipPhysics` field, and `sim_digest::fold_physics` folds all eight of
        // them. Gating it on `LocalShip` therefore made an authoritative value
        // depend on WHICH SHIP THIS HOST HAPPENS TO PROJECT — so two hosts
        // running one mission, each tagging its own hull, disagreed on the
        // fleet's roll from the first tick either of them steered. It was the
        // first thing `tests/local_ship_neutrality.rs` caught.
        //
        // Of the three ways out — fold nothing, fold everything, or stop
        // folding `roll` — this is the one that keeps the digest honest.
        // Dropping `roll` from the fold would narrow the authoritative record
        // over a field the snapshot still carries and `cross_target_probe`
        // deliberately writes; banking every hull instead makes the value mean
        // the same thing everywhere, and costs one lerp per ship per tick.
        //
        // It is also what the unified-ship model says: `LocalShip` "must never
        // gate shared gameplay mechanics — those run on `With<Ship>` so the
        // local ship and NPCs behave identically". An NPC now leans into its
        // turns like anything else with a helm, which is a visual improvement
        // rather than a cost. Uses the unscaled `input.steering` so roll
        // reflects intent, not engine count.
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

#[cfg(test)]
#[path = "marking_projection_tests.rs"]
mod marking_projection_tests;

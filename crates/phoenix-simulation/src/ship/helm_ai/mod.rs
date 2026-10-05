//! The helm AI's shared decision spine, plus the `FineSystemAiPolicies`
//! component (issue #1208, #1209) — a Bevy adapter, and the module the six
//! per-host-entity axis files ([`engines`], [`steering`], [`impulse`],
//! [`lateral`], [`vertical`], [`boost`]) plus [`surfaces`] and [`facts`]
//! attach to.
//!
//! Owns three things the split (issue #1206) left here rather than in a
//! dedicated `policies.rs`: [`FineSystemAiPolicies`] itself, the one
//! keyed-by-`SystemId` component every axis resolves its own entry from;
//! [`HelmAxisHost`] + [`run_helm_axis`], the generic driver that walks the
//! gate/declare/resolve spine so no per-axis system copies it; and
//! [`ai_policy_state_tick`], the shared #882 machine tick that advances every
//! stateful axis's runtime state once per ship per tick.
//!
//! Invariant: the six per-axis systems keep their own distinct Bevy queries
//! on purpose — no axis widens to a component a sibling needs — so the
//! deterministic schedule and its #894 digest stay byte-identical to before
//! the split.
//!
//! # Per-axis helm AI (issues #701, #703, #824)
//!
//! `ai_helm_thrust`, `ai_helm_steering`, `ai_helm_lateral_thrust` and
//! `ai_helm_impulse` are the per-axis helm AI: one decides the throttle, one
//! the yaw, one the dodge, one the impulse drive. Each gates on its own axis
//! alone:
//!
//! ```text
//! if !<own axis>.operate_ai { continue; }
//! ```
//!
//! They are the successors to the `operate_helm_ai` monolith (deleted in #704,
//! after #800/#703 declared every axis on every shipped hull and removed the
//! coarse half of each gate).
//!
//! **Since #824 no per-axis system writes an intent component.** Each one
//! emits its decision as an admitted `SystemControlPayload` — `SetThrust`,
//! `SetSteering`, `LateralThrustInput`, `StartImpulseCharge`/`CancelImpulse` —
//! into its own ship's per-entity `AdmittedCommands`, through the same
//! `validate_and_admit` seam every network command passes (admission symmetry,
//! `pasm/spec/RADAR_TARGET_AUTHORITY_AND_ADMISSION.md` §2). The write into
//! `AdmittedCommands` is direct and same-tick — deliberately NOT a round-trip
//! through the `InboundMessage` queue, which would add a one-tick lag and move
//! every NPC trajectory. `process_helm_inputs` then applies the admitted
//! payloads to the intent components later in the same tick, for AI and human
//! commands alike, with no branching on source downstream of admission.
//!
//! **Each axis has exactly one decider, and the applier is shared:**
//!
//! ```text
//! SetThrust            ← ai_helm_thrust         iff T
//! SetSteering          ← ai_helm_steering       iff S
//! LateralThrustInput   ← ai_helm_lateral_thrust iff L
//! Start/CancelImpulse  ← ai_helm_impulse        iff I
//! ```
//!
//! (T/S/L/I = the helm-thrust / helm-steering / helm-lateral-thrust /
//! helm-impulse `operate_ai` policies.) One decider per axis means Bevy's
//! arbitrary intra-set ordering cannot decide the outcome (the #697 failure
//! mode) because there is nothing to decide between; the shared applier
//! (`process_helm_inputs`) applies whatever admission let through.
//!
//! **The coarse `helm` policy C is no longer an input to any of this.** It gated
//! the monolith and nothing else; with the monolith gone, no helm-AI system reads
//! it. That is a load-bearing absence, not an accident: `C` is exactly the
//! coarse-fallback channel #800 spent an issue proving dormant, and re-admitting
//! it would resurrect the failure mode where an axis is silently driven by
//! something other than its own declaration.
//! `helm_writers_are_invariant_under_coarse_policy` pins the whole outcome
//! invariant under C over every (C, T, S, L, I) combination;
//! `coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do` pins it
//! end-to-end through a ticking app;
//! `shipped_hull_helm_is_driven_by_the_per_axis_declarations_alone` pins it on a
//! real hull's control sources.
//!
//! The corollary is that an axis a hull does not declare is an axis no AI drives.
//! `ControlSource::default()` is `Human` (`operate_ai == false`), so an
//! undeclared axis resolves to "human-held" and its system stands down; before
//! #704 the monolith quietly covered that case. All nine shipped hulls therefore
//! declare all four axes — see `shipped_hull_config_drives_the_per_axis_helm_systems`
//! and `shipped_hull_config_drives_ai_helm_lateral_thrust`, which pin the
//! declarations themselves against the real TOMLs. Adding a hull means declaring
//! four axes, not one.
//!
//! **The decision surface is assembled once, by `build_helm_ai_surfaces_frame`**
//! (issue #824 — see the `HelmAiSurfacesFrame` note above). The owner's ruling
//! recorded here through #823 said each per-axis system should call the pure
//! `operate_helm` itself and keep only its own output, duplicating the
//! `WorldView` build per ship per tick, because a shared cached `HelmDecision`
//! would re-create the mini-monolith this split exists to remove. #824 keeps
//! the load-bearing half of that ruling and retires the duplication: there is
//! still **no shared decision** — the frame carries only derived, read-only
//! decision *inputs*, rebuilt every AI tick, and each axis still calls its own
//! pure decision function (`operate_helm` per axis is pure and cheap; the
//! expensive part was always the view build). The identical-inputs invariant
//! the old shape left unenforced — both `operate_helm` callers must see the
//! same view or the axes disagree — is now true by construction, and
//! `all_four_axes_observe_the_same_frame` pins it.
//!
//! **No shared mutable state** (issue #702). `operate_helm` is a pure function:
//! it reads the frame (built from `TacticalRadarSelection`, `NavigationWaypoint` +
//! `HelmWaypointClearance`, `ObjectiveCursors`, the scored pool) and returns
//! `(thrust, steering)`. The axis systems consume the frame via `Res<_>` —
//! immutable by construction — so "did some axis mutate the surface between
//! systems?" is not a question anyone has to answer.
//!
//! **`LastHelmInput` has one writer now.** The per-axis systems no longer
//! mirror their fields; `process_helm_inputs` mirrors every applied helm
//! payload into the LocalShip's `LastHelmInput` as it applies the intent. The
//! pair readers in `SimSet::Physics` (`publish_joystick_to_engines`,
//! `operate_helm_engine_ai`, `tick_boost`) are ordered
//! `.after(process_helm_inputs)`, so a torn pair — this tick's AI throttle
//! beside last tick's stale human steering — cannot be observed;
//! `helm_ai_last_input_pair_is_not_torn` pins the result.

use bevy::prelude::*;

// Vertical thrust and boost (the AI-only / non-shim axes) still emit directly
// through the shared arbiter. The player-facing per-axis operators (engines,
// steering, impulse, lateral) route their emit through the AI host spine's
// `AiHostEnv::emitter()` instead (issue #1211, which deleted the per-axis
// `helm_ai_emit` / `helm_lateral_emit` pass-through shims — crates/phoenix-simulation/src/ai/host.rs now
// carries the single-owner observed admission edge). Both paths cross the same
// `command_admission::ai_emit::emit_ai_command` seam a human command does.
use crate::server_app::{ShipBoost, ShipImpulse};
#[cfg(test)]
use crate::ship::components::LastHelmInput;
use crate::ship::components::{
    BoostConfigResource, HelmWaypointClearance, ImpulseConfigResource, PendingArcBearingRequest,
    ShipSystemControlSources,
};
#[cfg(test)]
use crate::ship::helm::{
    ImpulseCommand, LateralThrustInput, SteeringInput, ThrustInput, VerticalThrustInput,
};
use crate::ship::state::ShipPhysics;

// The shared fixed-rate AI sim tick (issues #803, #889) used to live here as a
// helm-private `AiHelmTickTimer`/`AiHelmTickReady` pair. It was never
// helm-specific in anything but name: #889 promoted it to
// `crate::ai::cadence`, the ONE timer that gates every AI policy host, and
// `[global] ai_helm_tick_hz` to `ai_tick_hz` (the old key kept as a serde
// alias). The six per-axis helm systems keep the identical gate under its new
// name — see `crate::ai::cadence::ai_tick_ready`, which `ship_plugin` applies
// to each of them at registration.
#[cfg(test)]
use crate::ai::cadence::ai_tick_ready;

// ── Per-host-entity module decomposition (issue #1206) ──────────────────
// helm_ai.rs was split so each PASM helm host entity owns exactly one file:
//   engines.rs / steering.rs / impulse.rs / lateral.rs / vertical.rs /
//   boost.rs   — the six per-axis `ai_helm_*` systems + their policy newtypes
//   surfaces.rs — shared frame / desired-motion state
//   facts.rs    — helm fact seeding
// This module keeps the shared decision glue and re-exports every child
// item so the historical flat `ship::helm_ai::*` paths stay stable.
mod boost;
mod engines;
mod facts;
mod impulse;
mod lateral;
mod steering;
mod surfaces;
mod vertical;

pub use self::boost::*;
pub use self::engines::*;
// Impulse, Lateral and Vertical are the STATELESS axes: since #1209 deleted
// their per-axis policy newtypes they export only `pub(crate)` items (the axis
// marker + host system), so their re-export is crate-visible too — a `pub use`
// would re-export nothing and warn. Engines/Steering/Boost keep `pub use` for
// their public `Helm*AiPolicyState` twins.
pub(crate) use self::impulse::*;
pub(crate) use self::lateral::*;
pub use self::steering::*;
pub use self::surfaces::*;
pub(crate) use self::vertical::*;

// ── Shared helm-AI decision inputs (issue #701) ───────────────────────────────
//
// The per-axis `ai_helm_thrust` / `ai_helm_steering` / `ai_helm_lateral_thrust`
// / `ai_helm_impulse` all need the same three inputs: the world entity list,
// the entity's scored objectives, and a `WorldView`. These helpers are the
// single implementation of each, so the per-axis systems cannot silently
// drift from the monolith they replace in #704.

/// Mark Reach objectives complete once any ship arrives within its
/// TOML-authored `[behaviour] waypoint_arrival_radius` of the objective's
/// anchor (falling back to `WAYPOINT_ARRIVAL_RADIUS` for ships without a
/// behaviour section).
///
/// Runs in `Broadcast` (after `PublishAggregate` so `scored_objectives` is
/// fresh) and only counts ships whose helm system is AI-controlled.
/// Iterates every ship (player + NPC) so any ship pursuing a shared
/// world Reach objective can complete it. The `ObjectiveManagerRes` is a
/// single world-level resource, so multiple ships arriving at the same
/// anchor complete the shared objective once (idempotent complete()).
pub(crate) fn detect_reached_objective_completion(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    objectives: Option<ResMut<crate::world::server::ObjectiveManagerRes>>,
    objective_instances: Option<ResMut<crate::world::server::ObjectiveInstanceManagerRes>>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    ships: Query<
        (
            Option<&crate::entities::spawner::EntityUuid>,
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entities::spawner::BehaviourSection>,
            Option<&crate::ship_slots::AuthoredShipSlotId>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
) {
    let Some(mut objectives) = objectives else {
        return;
    };
    let mut objective_instances = objective_instances;
    let mut fleet: Vec<_> = ships
        .iter()
        .filter_map(|(uuid, _, _, _, _, slot, faction)| {
            Some(crate::objective_instances::PlayerShipMembership {
                ship_id: uuid?.0.clone(),
                slot_id: slot?.0.clone(),
                faction: faction
                    .and_then(|faction| {
                        faction_registry
                            .as_ref()
                            .and_then(|registry| registry.get(&faction.0))
                    })
                    .map(|faction| faction.name.clone())
                    .unwrap_or_default(),
            })
        })
        .collect();
    fleet.sort_by(|a, b| a.ship_id.cmp(&b.ship_id));
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();

    for (_uuid, sources, physics, blackboards, behaviour_section, _slot, _faction) in ships.iter() {
        if !helm_axes_operate_ai(sources) {
            continue;
        }

        let arrival_radius = behaviour_section
            .map(|b| b.0.waypoint_arrival_radius)
            .unwrap_or(crate::ai::WAYPOINT_ARRIVAL_RADIUS);

        let scored: Vec<crate::core::messages::ScoredObjective> = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => {
                bb.scored_objectives.clone()
            }
            _ => continue,
        };

        for obj in &scored {
            if obj.score <= 0.0 {
                continue;
            }
            let crate::core::messages::AiDirective::Reach { anchor } = &obj.directive else {
                continue;
            };
            let Some(&target) = anchors.get(anchor.as_str()) else {
                continue;
            };
            let dx = target[0] - physics.x;
            let dz = target[2] - physics.z;
            if (dx * dx + dz * dz).sqrt() < arrival_radius {
                // Guard the tracer on the actual transition so repeated arrivals
                // at a shared anchor (idempotent complete) emit once (issue #841).
                let instance_key = objective_instances
                    .as_ref()
                    .and_then(|instances| instances.0.key_for_display(&obj.snapshot.id));
                let (changed, balance_id) = if let Some(key) = instance_key {
                    let changed = objective_instances.as_mut().is_some_and(|instances| {
                        instances.0.complete(&key, &fleet).unwrap_or(false)
                    });
                    (changed, key.objective_id)
                } else {
                    (
                        objectives.0.complete(&obj.snapshot.id),
                        obj.snapshot.id.clone(),
                    )
                };
                if changed {
                    if let Some(ref mut msgs) = balance_events {
                        msgs.write(crate::core::balance::BalanceEvent::ObjectiveCompleted {
                            objective_id: balance_id.clone(),
                        });
                        msgs.write(crate::core::balance::BalanceEvent::ObjectiveChanged {
                            objective_id: balance_id.clone(),
                            status: crate::core::messages::ObjectiveStatus::Completed,
                            targets: objectives
                                .0
                                .targets(&balance_id)
                                .unwrap_or_default()
                                .to_vec(),
                        });
                    }
                }
            }
        }
    }
}

// Per-axis helm AI design notes (issues #701, #703, #824) — see the module
// doc (`//!`) at the top of this file.

/// The read-only per-tick inputs `ai_policy_state_tick` reads besides the
/// [`crate::ai::host::AiHostEnv`], bundled as one `SystemParam` (issue #1185):
/// the world config (authored AI tick rate), the helm-AI surfaces frame, the
/// motion plan, and the run's master seed (`sim_rng`, the WORLD field of the
/// orbit-direction composite key).
///
/// A signature grouping only — every field keeps its type and `Option` fallback
/// (`world_config`/`sim_rng` stay `Option` for the bare-`App` fixtures that never
/// insert them), so the access set is byte-for-byte unchanged; the system
/// destructures it back to its original locals at entry.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct HelmPolicyInputs<'w> {
    world_config: Option<Res<'w, crate::world::config::WorldConfig>>,
    frame: Res<'w, HelmAiSurfacesFrame>,
    plan: Res<'w, crate::ship::helm_planner::HelmMotionPlan>,
    sim_rng: Option<Res<'w, crate::sim_rng::SimRng>>,
}

/// Advance every stateful fine-system policy's state machine, ONCE per shared
/// AI tick, and COMMIT the entered state before any output resolves this tick
/// (issue #882).
///
/// Ordering (declared in `ship_plugin.rs`): `.after(helm_motion_planner)` so
/// the hazard surface a transition guard reads is this tick's, and `.before`
/// the per-axis actuator systems so the state they resolve their continuous
/// outputs in is the state committed here — AC2's "the resulting state supplies
/// continuous outputs immediately in the same tick". Runs under the same
/// `run_if(ai_tick_ready)` latch as those systems.
///
/// AC2's other half — at most ONE transition per eligible tick — is not
/// enforced here at all: [`crate::ai::policy::AiPolicy::resolve_transition`]
/// returns an `Option`, so this host has no way to chain two.
///
/// AC5 reset: a ship whose Boost system is not AI-operated, or whose boost
/// capability is absent/disabled, is reset to `initial` every tick it stays
/// that way. So the tick AI *gains* control — and the tick an unavailable
/// system *recovers* — begins from the initial state with authored memory,
/// never resuming a stale mid-manoeuvre state.
///
/// ## This host is also the WRITER of this fine system's private memory
///
/// There is no authored write verb and there never will be: a policy READS
/// `memory(name)`, the host WRITES it. That is the same split #779/#780 use for
/// continuous magnitudes (the planner owns the number, the policy owns the
/// decision), and it is what makes memory more than a second spelling of
/// `param` — the values are retained across ticks and only
/// [`crate::ai::policy::AiPolicyRuntimeState::reset`] puts them back to their
/// authored declarations. Two slots are written here, both named by the host,
/// neither knowable from a single tick's facts:
///
/// * [`PEAK_HAZARD_MEMORY`] — a running maximum, folded every gated tick. This
///   is the shape issue #883's closest-approach detector needs.
/// * [`ENGAGEMENTS_MEMORY`] — incremented when a committed transition enters a
///   state whose OWN rules engage boost. The host asks the policy what the
///   entered state does on this system's channel, so the counter needs no
///   knowledge of authored state names.
///
/// Issue #883 adds the two travel axes and two more host-written slots, folded
/// for EVERY machine by [`tick_policy_machine`]:
///
/// * [`MIN_RANGE_SEEN_MEMORY`] — a running MINIMUM of `range_to_target`, scoped
///   to the current state (the host resets it on every commit). Closest approach
///   is then "the range has re-opened past the authored hysteresis", which one
///   tick of retention is exactly enough to know and no single-tick fact can say.
/// * [`ESCAPE_HEADING_MEMORY`] — the ship's yaw at the instant a transition
///   commits, so the state that was just entered can fly a heading frozen at the
///   merge rather than a heading that keeps being re-solved.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_policy_state_tick(
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    // The world config, helm-AI surfaces frame, motion plan, and seeded RNG
    // bundled as one `SystemParam` (issue #1185). See [`HelmPolicyInputs`].
    inputs: HelmPolicyInputs,
    clock: ResMut<AiPolicyTickClock>,
    ships: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::ship_plugin::ShipPhysicsConfigResource>,
            Option<&BoostConfigResource>,
            Option<&ImpulseConfigResource>,
            // Optional for the same reason the per-axis hosts take it optionally:
            // a bare-`App` fixture may attach only the policy it is testing. The
            // three stateful axes (Engines/Steering/Boost) each read their own
            // entry out of this ONE keyed map (issue #1209) by `system_id()`; a
            // ship whose map lacks an axis falls back to that axis's canonical
            // default, which is stateless, so its machine tick returns
            // immediately. Taking the map optionally keeps the whole QUERY from
            // failing to match and silently skipping the ship — the same class of
            // silent skip #883 added the `resolve_helm_channel` guard for.
            Option<&FineSystemAiPolicies>,
            // This ship's OWN shields (issue #788). Read-only here — `tick_shields`
            // is the single writer — so this adds no ordering question, only a
            // reading that may be one tick old.
            Option<&crate::ship::shields::ShipShields>,
            // This ship's OWN tubes and rounds in flight (issue #791). Read-only,
            // and unlike the shields above this one DOES carry an ordering
            // question — `handle_fire_torpedo` appends to `in_flight` and
            // `tick_torpedo_lifecycle` removes from it, both in `SimSet::Physics`
            // — so `ship_plugin` pins this system after both of them.
            Option<&crate::console::weapons::TorpedoSystemResource>,
            // This ship's OWN blaster banks (issue #792), read for their authored
            // `range`/`projectile_speed` alone. Bank CONFIG never changes at
            // runtime, so unlike the tubes above this carries no ordering
            // question — no system in the schedule writes the field this reads.
            Option<&crate::console::weapons::BlasterSystemResource>,
            // This ship's OWN phaser banks (issue #929), read for `facing_deg` /
            // `auto_arc_deg` / `beam_damage_per_sec` alone, to derive how much
            // arc margin the target has left. Config, so it carries no ordering
            // question for the same reason the blaster banks above do not.
            Option<&crate::console::weapons::PhaserCombatConfigResource>,
            // The SHIP field of the orbit-direction composite key.
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::power::ShipPowerSystem>,
            HelmPolicyRuntime,
        ),
        With<crate::ai::server::AiHighFidelity>,
    >,
    // Every entity a target uuid could name, for the facing-shield reading
    // (issue #791). The same shape `ai_torpedo_auto_fire` resolves its own
    // striking arc through, and read-only, so it conflicts with nothing the
    // ship query above mutates.
    targets: Query<(
        &crate::entities::spawner::EntityUuid,
        &Transform,
        Option<&crate::ship::shields::ShipShields>,
        Option<&ShipPhysics>,
    )>,
    // Balance tracer sink (issue #915). `Option<ResMut<Messages<_>>>` rather
    // than `MessageWriter` for the same reason the objective tracer above uses
    // it: a bare-`App` fixture that never registered the message must not fail
    // parameter validation.
    balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
    // The last doctrine phase reported per ship, so the tracer emits once per
    // observed change (including the initial phase on the first gated tick)
    // rather than once per tick.
    last_reported_phase: Local<std::collections::HashMap<Entity, String>>,
) {
    phoenix_sim_gameplay::ship::helm_ai::ai_policy_state_tick(
        &ai_env,
        inputs.world_config.as_deref().map(|wc| &wc.global),
        inputs.frame,
        inputs.plan,
        inputs.sim_rng,
        clock,
        ships,
        targets,
        balance_events,
        last_reported_phase,
    );
}

// ── The six helm axes behind one trait + one driver (issue #1208) ─────────────
//
// Every per-axis helm host walks the identical decision spine: **gate** the
// axis's Control Source on AI, **check** it declares a policy, **resolve** the
// axis's single mode channel, and — on a fired-and-accepted verb — **actuate**.
// Issue #1205 lifted that spine into [`crate::ai::host::decide`]; this trait is
// what lets the six axes SHARE it. Each axis is one [`HelmAxisHost`] impl naming
// its system id, channel, statefulness, accepted verb(s), fact seeding and
// actuation; [`run_helm_axis`] is the one generic driver that walks the spine
// for any of them, so the twelve-step gate/declare/resolve preamble that used to
// be copied into each of the six ~120-line systems now lives ONCE, in the spine.
//
// The six per-axis SYSTEMS stay distinct (each keeps its own query), on purpose:
// no axis gains a component another needs, so the Bevy access footprint — and
// therefore the deterministic schedule and its digest — is byte-identical to
// before #1208. Each system body is now a thin loop that builds the per-ship
// [`HelmAxisCtx`]/[`HelmAxisIo`], calls `run_helm_axis::<ThisAxis>`, and emits
// whatever payload it returns through the axis's own admission shim.

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "mod_tests.rs"]
mod tests;

pub use phoenix_sim_gameplay::ship::helm_ai::{
    helm_axes_operate_ai, helm_axis_outcome, resolve_helm_channel, run_helm_axis,
    tick_policy_machine, AiPolicyTickClock, FineSystemAiPolicies, HelmAxisCtx, HelmAxisHost,
    HelmAxisIo, HelmPolicyRuntime, HelmPolicyRuntimeItem,
};

pub use self::facts::*;

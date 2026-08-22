use bevy::prelude::*;

// Vertical thrust and boost (the AI-only / non-shim axes) still emit directly
// through the shared arbiter. The player-facing per-axis operators (engines,
// steering, impulse, lateral) route their emit through the AI host spine's
// `AiHostEnv::emitter()` instead (issue #1211, which deleted the per-axis
// `helm_ai_emit` / `helm_lateral_emit` pass-through shims — src/ai/host.rs now
// carries the single-owner observed admission edge). Both paths cross the same
// `command_admission::ai_emit::emit_ai_command` seam a human command does.
use crate::command_admission::ai_emit::emit_ai_command;
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
pub(crate) use self::facts::*;
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

/// Per-ship map of the helm fine-system AI **policies**, keyed by each fine
/// system's [`SystemId`](crate::core::messages::SystemId) (issue #1209). Built at spawn
/// from the authored `[helm_console.*_ai]` blocks — one entry per block the hull
/// declares — collapsing the six former `Helm*AiPolicy` newtypes
/// (`HelmEnginesAiPolicy`, `HelmSteeringAiPolicy`, `HelmLateralAiPolicy`,
/// `HelmVerticalAiPolicy`, `HelmImpulseAiPolicy`, `HelmBoostAiPolicy`) into one
/// keyed component — the same shape the weapon banks use
/// ([`PhaserBankAiPolicies`](crate::console::weapons::PhaserBankAiPolicies) /
/// [`TorpedoTubeAiPolicies`](crate::console::weapons::TorpedoTubeAiPolicies)).
///
/// A `BTreeMap` for order-stable iteration: each axis host reads only its OWN
/// policy by its [`HelmAxisHost::system_id`], so the map is never iterated on the
/// hot path, but keying on the ordered `SystemId` keeps the component a pure
/// function of its contents regardless of insertion order — the same property the
/// per-ship blackboard `BTreeMap` relies on. A system with no entry takes no AI
/// action on that axis, exactly as an absent newtype did.
///
/// Only the AUTHORED policy is keyed here. The runtime STATE twins
/// (`Helm*AiPolicyState`) stay SEPARATE per-fine-system components: they are
/// LOD-carried (`ai_high_fidelity_components`) and snapshotted, whereas this
/// authored policy is neither — so collapsing them would confuse two lifetimes.
#[derive(Component, Default, Clone, Debug)]
pub struct FineSystemAiPolicies(
    pub std::collections::BTreeMap<crate::core::messages::SystemId, crate::ai::policy::AiPolicy>,
);

/// True when the AI helm is flying this ship: both stick axes
/// (`helm-thrust` AND `helm-steering`) are AI-operated. The coarse `helm`
/// system this used to gate on was deleted by #801; per Rule 6 the answer
/// derives from the per-axis declarations, never a coarse fallback.
pub(crate) fn helm_axes_operate_ai(sources: &ShipSystemControlSources) -> bool {
    sources
        .0
        .policy_for(&crate::ship::system_registry::helm_thrust_system_id())
        .operate_ai
        && sources
            .0
            .policy_for(&crate::ship::system_registry::helm_steering_system_id())
            .operate_ai
}

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
    ships: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::entities::spawner::BehaviourSection>,
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
    let anchors = world_config
        .as_ref()
        .map(|wc| wc.anchors.clone())
        .unwrap_or_default();

    for (sources, physics, blackboards, behaviour_section) in ships.iter() {
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
                if objectives.0.complete(&obj.snapshot.id) {
                    if let Some(ref mut msgs) = balance_events {
                        msgs.write(crate::core::balance::BalanceEvent::ObjectiveCompleted {
                            objective_id: obj.snapshot.id.clone(),
                        });
                    }
                }
            }
        }
    }
}

// ── Per-axis helm AI (issues #701, #703, #824) ────────────────────────────────
//
// `ai_helm_thrust`, `ai_helm_steering`, `ai_helm_lateral_thrust` and
// `ai_helm_impulse` are the per-axis helm AI: one decides the throttle, one
// the yaw, one the dodge, one the impulse drive. Each gates on its own axis
// alone:
//
//     if !<own axis>.operate_ai { continue; }
//
// They are the successors to the `operate_helm_ai` monolith (deleted in #704,
// after #800/#703 declared every axis on every shipped hull and removed the
// coarse half of each gate).
//
// **Since #824 no per-axis system writes an intent component.** Each one
// emits its decision as an admitted `SystemControlPayload` — `SetThrust`,
// `SetSteering`, `LateralThrustInput`, `StartImpulseCharge`/`CancelImpulse` —
// into its own ship's per-entity `AdmittedCommands`, through the same
// `validate_and_admit` seam every network command passes (admission symmetry,
// `pasm/spec/RADAR_TARGET_AUTHORITY_AND_ADMISSION.md` §2). The write into
// `AdmittedCommands` is direct and same-tick — deliberately NOT a round-trip
// through the `InboundMessage` queue, which would add a one-tick lag and move
// every NPC trajectory. `process_helm_inputs` then applies the admitted
// payloads to the intent components later in the same tick, for AI and human
// commands alike, with no branching on source downstream of admission.
//
// **Each axis has exactly one decider, and the applier is shared:**
//
//   SetThrust            ← `ai_helm_thrust`         iff T
//   SetSteering          ← `ai_helm_steering`       iff S
//   LateralThrustInput   ← `ai_helm_lateral_thrust` iff L
//   Start/CancelImpulse  ← `ai_helm_impulse`        iff I
//
// (T/S/L/I = the helm-thrust / helm-steering / helm-lateral-thrust /
// helm-impulse `operate_ai` policies.) One decider per axis means Bevy's
// arbitrary intra-set ordering cannot decide the outcome (the #697 failure
// mode) because there is nothing to decide between; the shared applier
// (`process_helm_inputs`) applies whatever admission let through.
//
// **The coarse `helm` policy C is no longer an input to any of this.** It gated
// the monolith and nothing else; with the monolith gone, no helm-AI system reads
// it. That is a load-bearing absence, not an accident: `C` is exactly the
// coarse-fallback channel #800 spent an issue proving dormant, and re-admitting
// it would resurrect the failure mode where an axis is silently driven by
// something other than its own declaration.
// `helm_writers_are_invariant_under_coarse_policy` pins the whole outcome
// invariant under C over every (C, T, S, L, I) combination;
// `coarse_helm_alone_drives_no_intent_but_the_per_axis_systems_do` pins it
// end-to-end through a ticking app;
// `shipped_hull_helm_is_driven_by_the_per_axis_declarations_alone` pins it on a
// real hull's control sources.
//
// The corollary is that an axis a hull does not declare is an axis no AI drives.
// `ControlSource::default()` is `Human` (`operate_ai == false`), so an
// undeclared axis resolves to "human-held" and its system stands down; before
// #704 the monolith quietly covered that case. All nine shipped hulls therefore
// declare all four axes — see `shipped_hull_config_drives_the_per_axis_helm_systems`
// and `shipped_hull_config_drives_ai_helm_lateral_thrust`, which pin the
// declarations themselves against the real TOMLs. Adding a hull means declaring
// four axes, not one.
//
// **The decision surface is assembled once, by `build_helm_ai_surfaces_frame`**
// (issue #824 — see the `HelmAiSurfacesFrame` note above). The owner's ruling
// recorded here through #823 said each per-axis system should call the pure
// `operate_helm` itself and keep only its own output, duplicating the
// `WorldView` build per ship per tick, because a shared cached `HelmDecision`
// would re-create the mini-monolith this split exists to remove. #824 keeps
// the load-bearing half of that ruling and retires the duplication: there is
// still **no shared decision** — the frame carries only derived, read-only
// decision *inputs*, rebuilt every AI tick, and each axis still calls its own
// pure decision function (`operate_helm` per axis is pure and cheap; the
// expensive part was always the view build). The identical-inputs invariant
// the old shape left unenforced — both `operate_helm` callers must see the
// same view or the axes disagree — is now true by construction, and
// `all_four_axes_observe_the_same_frame` pins it.
//
// **No shared mutable state** (issue #702). `operate_helm` is a pure function:
// it reads the frame (built from `TacticalRadarSelection`, `NavigationWaypoint` +
// `HelmWaypointClearance`, `ObjectiveCursors`, the scored pool) and returns
// `(thrust, steering)`. The axis systems consume the frame via `Res<_>` —
// immutable by construction — so "did some axis mutate the surface between
// systems?" is not a question anyone has to answer.
//
// **`LastHelmInput` has one writer now.** The per-axis systems no longer
// mirror their fields; `process_helm_inputs` mirrors every applied helm
// payload into the LocalShip's `LastHelmInput` as it applies the intent. The
// pair readers in `SimSet::Physics` (`publish_joystick_to_engines`,
// `operate_helm_engine_ai`, `tick_boost`) are ordered
// `.after(process_helm_inputs)`, so a torn pair — this tick's AI throttle
// beside last tick's stale human steering — cannot be observed;
// `helm_ai_last_input_pair_is_not_torn` pins the result.

/// The tick-derived clock the policy state machines measure `state_time`
/// against (issue #882, AC4).
///
/// Advanced by exactly one increment of the authored AI-helm tick period each
/// time [`ai_policy_state_tick`] runs — and that system runs under
/// `run_if(ai_tick_ready)`, the shared fixed-rate latch. It is therefore
/// derived from the shared AI tick cadence and NOT from `Time::delta`: a 144 Hz
/// host and a 60 Hz host advance policy state time identically, which is the
/// whole point of the #803 latch and of PRD #620's determinism goal. Issue #784
/// retired the last per-frame AI timer; nothing here reintroduces one.
#[derive(Resource, Default)]
pub(crate) struct AiPolicyTickClock(pub(crate) f64);

/// The mutable per-ship policy runtime [`ai_policy_state_tick`] owns, bundled as
/// one `QueryData` (issue #788).
///
/// Bundled because Bevy's query tuples cap out and that system already carries
/// most of a ship's helm configuration, but also because these five components
/// are one thing: the runtime state of a ship's helm policy machines plus the
/// two surfaces derived from them. Nothing else writes any of them, so there is
/// exactly one writer for the whole bundle.
#[derive(bevy::ecs::query::QueryData)]
#[query_data(mutable)]
pub(crate) struct HelmPolicyRuntime {
    engines: &'static mut HelmEnginesAiPolicyState,
    steering: &'static mut HelmSteeringAiPolicyState,
    boost: &'static mut HelmBoostAiPolicyState,
    pass: &'static mut HelmPassSurface,
    recovery: &'static mut HelmRecoveryHistory,
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
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    frame: Res<HelmAiSurfacesFrame>,
    plan: Res<crate::ship::helm_planner::HelmMotionPlan>,
    // The run's master seed — the WORLD field of the orbit-direction composite
    // key (issue #788). `Option` for the same reason every other simulation
    // system takes it optionally: a bare `Res` fails Bevy parameter validation
    // in every bare-`App` fixture in this crate. Absent resolves to seed 0,
    // which is still deterministic.
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
    mut clock: ResMut<AiPolicyTickClock>,
    mut ships: Query<
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
            // The SHIP field of the orbit-direction composite key.
            Option<&crate::entities::spawner::EntityUuid>,
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
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
    // The last doctrine phase reported per ship, so the tracer emits once per
    // observed change (including the initial phase on the first gated tick)
    // rather than once per tick.
    mut last_reported_phase: Local<std::collections::HashMap<Entity, String>>,
) {
    // One authored tick period per run — the shared cadence, never Time::delta.
    let hz = world_config
        .as_deref()
        .map(|wc| wc.global.ai_tick_hz)
        .unwrap_or_else(|| crate::entities::config::GlobalConfig::default().ai_tick_hz);
    if hz > 0.0 {
        clock.0 += 1.0 / hz as f64;
    }
    let now = clock.0;

    let world_seed = sim_rng.as_deref().map(|r| r.seed()).unwrap_or(0);

    for (
        entity,
        sources,
        physics,
        physics_cfg,
        boost_cfg,
        impulse_cfg,
        fine_policies,
        shields,
        torpedoes,
        blasters,
        entity_uuid,
        mut runtime,
    ) in ships.iter_mut()
    {
        // Engines and Steering must BOTH be declared for the shared tick to mean
        // anything: the STEERING policy's params seed the recovery/pressed
        // readings every machine reads, and `build_pass_surface` below is a
        // function of both. Since #885b stage 5d there is no synthesised
        // stand-in — strict AI-declaration mode rejects an AI-capable hull that
        // omits either block at load, so a ship missing one takes no helm AI
        // action rather than being handed a policy nobody wrote. Both are read
        // out of the ship's one keyed `FineSystemAiPolicies` map (issue #1209).
        let engines_policy = fine_policies.and_then(|p| {
            p.0.get(&crate::ship::system_registry::helm_thrust_system_id())
        });
        let steering_policy = fine_policies.and_then(|p| {
            p.0.get(&crate::ship::system_registry::helm_steering_system_id())
        });
        let (Some(engines_policy), Some(steering_policy)) = (engines_policy, steering_policy)
        else {
            continue;
        };

        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2), shared by all three machines this tick.
        let flag_chain = ai_env.flag_chain(entity);

        // One fact snapshot per ship per tick, shared by all three machines —
        // they must reason about the SAME world or they would reach different
        // legs. Private memory is what stays per-system (AC3): the derived
        // memory fact is folded in separately, inside each machine's own tick.
        let mut facts = seed_helm_actuator_facts(
            plan.ships.get(&entity).map(|sp| &sp.hazard),
            impulse_cfg.is_some(),
            boost_cfg.map(|c| c.enabled).unwrap_or(false),
            physics.y,
            frame_red_alert(frame.ships.get(&entity)),
        );
        // The identity of the target the geometry above was seeded from. The
        // running range minimum is scoped to it, so a mid-state target switch
        // restarts the fold rather than inheriting a stranger's minimum.
        let travel_target = seed_helm_travel_facts(
            &mut facts,
            frame.ships.get(&entity),
            physics,
            physics_cfg.map(|c| c.0.max_speed).unwrap_or(0.0),
        );
        // Shield-recovery readings (issue #788): own shield fraction, the derived
        // safe ring, and the bounded distance history's verdict. Read off the
        // STEERING policy's params — the axis that owns the recovery legs — so
        // all three machines see one consistent ring.
        seed_recovery_facts(
            &mut facts,
            &steering_policy.params,
            shields.map(|s| s.0.fraction()),
            &mut runtime.recovery,
            travel_target,
        );
        // Pressed readings (issue #789): the SECOND bounded window's separation
        // trend and the "inside the target's own reach" comparison. Folded here,
        // once per shared tick, after the target scope above has been settled.
        seed_pressed_facts(&mut facts, &steering_policy.params, &mut runtime.recovery);
        // Torpedo-opportunity readings (issue #791): whether the ONE shield arc
        // of the target that faces us is down, how many of our own rounds are
        // still in the air, and whether a whole salvo is still reachable at all.
        // All three are pure world readings with no authored threshold, so they
        // are seeded for every hull; a hull whose doctrine never asks simply
        // never reads them.
        seed_torpedo_opportunity_facts(
            &mut facts,
            travel_target,
            physics,
            &targets,
            torpedoes.map(|t| &t.0),
            sources,
        );

        // ── Engines ──────────────────────────────────────────────────────────
        tick_policy_machine(
            engines_policy,
            &mut runtime.engines.0,
            sources
                .0
                .policy_for(&crate::ship::system_registry::helm_thrust_system_id())
                .operate_ai,
            &facts,
            travel_target,
            now,
            physics.yaw,
            &flag_chain,
            |_| {},
        );

        // Doctrine-phase tracer (issue #915): the Engines machine's committed
        // state IS the ship's doctrine movement phase, so report every observed
        // change of it — including the initial phase on the first gated tick —
        // as a balance event. Emitted unconditionally (all ships, every build),
        // like every other balance chokepoint; the headless report folds these
        // into per-ship time-in-phase. Stateless policies have no phases and
        // emit nothing.
        if engines_policy.machine().is_some() {
            let phase = &runtime.engines.0.current;
            if !phase.is_empty()
                && last_reported_phase.get(&entity).map(String::as_str) != Some(phase.as_str())
            {
                if let (Some(msgs), Some(uuid)) = (balance_events.as_mut(), entity_uuid) {
                    msgs.write(crate::core::balance::BalanceEvent::DoctrinePhaseChanged {
                        ship: uuid.0.clone(),
                        phase: phase.clone(),
                    });
                    last_reported_phase.insert(entity, phase.clone());
                }
            }
        }

        // ── Steering ─────────────────────────────────────────────────────────
        let steering_entered = tick_policy_machine(
            steering_policy,
            &mut runtime.steering.0,
            sources
                .0
                .policy_for(&crate::ship::system_registry::helm_steering_system_id())
                .operate_ai,
            &facts,
            travel_target,
            now,
            physics.yaw,
            &flag_chain,
            |_| {},
        );
        if let Some(entered) = steering_entered {
            // Entering an ORBITING state draws that orbit's circulation direction
            // (issues #788, #790). The host asks the policy what the state it
            // just entered does on this system's own channel, exactly as the
            // boost engagement counter below does, so this needs no knowledge of
            // authored state names.
            //
            // BOTH orbit verbs draw, and they must: the shield-recovery standoff
            // and the combat broadside ring each need a definite side to circle
            // on, and gating the draw on one of them would leave the other
            // reading whatever the hull happened to declare — a constant, which
            // is precisely what a seeded choice exists to avoid.
            let orbits = matches!(
                resolve_helm_channel(
                    steering_policy,
                    Some(&runtime.steering.0),
                    crate::entities::config::HELM_YAW_CHANNEL,
                    &facts,
                    now,
                    &flag_chain,
                ),
                Some(
                    &crate::ai::policy::AiPolicyVerb::HoldRecoveryOrbit
                        | &crate::ai::policy::AiPolicyVerb::HoldCombatOrbit
                )
            );
            if orbits {
                let occurrence = runtime
                    .steering
                    .0
                    .memory
                    .get(ORBIT_OCCURRENCES_MEMORY)
                    .unwrap_or(0.0)
                    + 1.0;
                runtime
                    .steering
                    .0
                    .memory
                    .set(ORBIT_OCCURRENCES_MEMORY, occurrence);
                // Deterministic from (world, ship, system, transition,
                // occurrence) and from nothing else — no `Time`, no frame count,
                // no OS entropy. Two runs of the same seeded scenario break the
                // same way; two ships breaking off on the same tick do not.
                let key = crate::composite_rng::CompositeKey {
                    world: world_seed,
                    ship: entity_uuid
                        .and_then(|u| uuid::Uuid::parse_str(&u.0).ok())
                        .map(|u| u.as_u128() as u64)
                        .unwrap_or_else(|| entity.to_bits()),
                    system: crate::composite_rng::key_from_name(STEERING_SEED_SYSTEM_NAME),
                    transition: crate::composite_rng::key_from_name(&entered),
                    occurrence: occurrence as u64,
                };
                runtime.steering.0.memory.set(
                    ORBIT_DIRECTION_MEMORY,
                    crate::composite_rng::signed_choice(&key),
                );
            }
        }

        // ── Boost ────────────────────────────────────────────────────────────
        // Availability is part of this axis's AC5 reset gate: an absent or
        // feature-disabled boost holds the machine at `initial`.
        //
        // An undeclared `[helm_console.boost_ai]` leaves the machine untouched,
        // for the same reason as above: no declaration, no automation.
        let boost_operable = sources
            .0
            .policy_for(&crate::ship::system_registry::helm_boost_system_id())
            .operate_ai
            && boost_cfg.map(|c| c.enabled).unwrap_or(false);
        let boost_policy = fine_policies.and_then(|p| {
            p.0.get(&crate::ship::system_registry::helm_boost_system_id())
        });
        let entered = boost_policy.and_then(|boost_policy| {
            tick_policy_machine(
                boost_policy,
                &mut runtime.boost.0,
                boost_operable,
                &facts,
                travel_target,
                now,
                physics.yaw,
                &flag_chain,
                // Boost's own extra host-written slot (issue #882): a running
                // maximum of the hazard faced since the last reset.
                |memory| {
                    let urgency = facts
                        .get(HAZARD_URGENCY_FACT)
                        .unwrap_or(0.0)
                        .max(memory.get(PEAK_HAZARD_MEMORY).unwrap_or(0.0));
                    memory.set(PEAK_HAZARD_MEMORY, urgency);
                },
            )
        });
        if let (Some(_), Some(boost_policy)) = (entered.as_ref(), boost_policy) {
            // Count the entries into a boost-ENGAGING state. The host asks the
            // policy what the state it just entered does on this system's own
            // channel, so the counter needs no knowledge of authored state
            // names: any content whose entered state engages boost increments
            // it. This survives the transition that produced it and every
            // later tick, and only `AiPolicyRuntimeState::reset` clears it back
            // to the authored declaration — which is the property that makes
            // `memory(...)` different from `param(...)`.
            let engages = resolve_helm_channel(
                boost_policy,
                Some(&runtime.boost.0),
                crate::entities::config::HELM_BOOST_CHANNEL,
                &facts,
                now,
                &flag_chain,
            ) == Some(&crate::ai::policy::AiPolicyVerb::EngageBoost);
            if engages {
                let n = runtime
                    .boost
                    .0
                    .memory
                    .get(ENGAGEMENTS_MEMORY)
                    .unwrap_or(0.0);
                runtime.boost.0.memory.set(ENGAGEMENTS_MEMORY, n + 1.0);
            }
        }

        // ── Publish the derived fly-through pass surface (issues #883, #788) ──
        let surface = build_pass_surface(
            engines_policy,
            steering_policy,
            &runtime.steering.0,
            sources,
            &facts,
            now,
            // The artillery lead speed (issue #792): a reading of this hull's own
            // longest-reaching bolt, resolved here so the pure planner never has
            // to know what a blaster bank is.
            blasters.map(|b| artillery_lead_speed(&b.0)).unwrap_or(0.0),
            &flag_chain,
        );
        *runtime.pass = surface;
    }
}

/// Advance ONE fine system's policy machine by one shared AI tick (issue #883,
/// generalised from #882's Boost-only body).
///
/// Returns the state entered when a transition committed this tick.
///
/// Everything a *fly-through* needs beyond #882 is the two host-written slots
/// folded here, and both are deliberately generic rather than doctrine-specific:
///
/// * [`MIN_RANGE_SEEN_MEMORY`] is a running minimum **scoped to the current
///   state AND to the current target's identity**. The host resets it to the
///   current range on every commit, and restarts the fold whenever `target`
///   differs from the one it was accumulated against
///   ([`MIN_RANGE_TARGET_MEMORY`]). The state scoping is what lets a machine
///   cycle through repeated attack runs without the host knowing a single
///   authored state name; the identity scoping is what stops a mid-state target
///   switch — to a further ship, say — synthesising a `range_above_min_seen`
///   spike out of the previous target's minimum and firing a closest approach
///   the ship never flew.
/// * [`ESCAPE_HEADING_MEMORY`] is written on every commit, so any state can be
///   authored to fly the heading captured at the transition that entered it.
///
/// A stateless policy — every hull that ships today — returns immediately
/// without touching memory, state, or the transition scan.
#[allow(clippy::too_many_arguments)]
fn tick_policy_machine<F>(
    policy: &crate::ai::policy::AiPolicy,
    state: &mut crate::ai::policy::AiPolicyRuntimeState,
    operable: bool,
    facts: &crate::world::flags::AiFacts,
    // The target this tick's `range_to_target` was seeded from, as returned by
    // `seed_helm_travel_facts`.
    target: Option<uuid::Uuid>,
    now: f64,
    yaw: f32,
    // The scenario world-flag chain (issue #891 stage 2), read by transition
    // guards exactly as by channel guards.
    flags: &[&crate::world::flags::FlagStore],
    fold_extra_memory: F,
) -> Option<String>
where
    F: FnOnce(&mut crate::world::flags::AiPolicyMemory),
{
    // Stateless policies never enter any of this.
    policy.machine()?;
    // AC5: not AI-operated, or the system is unavailable → hold at initial, so
    // the tick AI *gains* control begins from the authored initial state rather
    // than resuming a stale mid-manoeuvre one.
    if !operable {
        *state = crate::ai::policy::AiPolicyRuntimeState::reset(policy, now);
        return None;
    }
    // A state component that was never initialised (or whose authored machine
    // changed) starts at `initial`.
    if policy
        .machine()
        .and_then(|m| m.state(&state.current))
        .is_none()
    {
        *state = crate::ai::policy::AiPolicyRuntimeState::reset(policy, now);
    }

    fold_extra_memory(&mut state.memory);

    // ── The ONE history fold (issue #890) ────────────────────────────────────
    //
    // Every authored `history(...)` window on this fine system advances by
    // exactly one sample, here and nowhere else in the crate. This host runs
    // under the shared `run_if(ai_tick_ready)` latch and is called once per fine
    // system per tick, so "one call" is "one shared AI tick" — which is the
    // whole property. Folding from the per-axis actuator systems instead (they
    // all resolve guards off this same ship in this same tick) would advance a
    // window four times a tick, so an authored 30-tick span would silently mean
    // seven and a half: the sharp edge #789 documented and could only work
    // around by keeping its bespoke facts out of rule guards altogether.
    //
    // `entities::ai_flag_hosts` records this function as the fold site for the
    // three helm machine axes and rejects a `history(...)` guard at load on
    // every host that has none; `ai_flag_hosts::tests` then re-derives that
    // classification from this source, so a second fold site cannot appear
    // quietly.
    //
    // Ordered BEFORE the transition scan below and before the per-axis systems
    // run, so a window read from a transition guard and the same window read
    // from a per-state rule guard are the same window at the same tick — the
    // two authorable positions agree by construction.
    //
    // Deliberately NOT re-scoped on commit, unlike the running range minimum: a
    // window is a measurement of the world over an authored span, and clearing
    // it at a transition would make "has held for 30 ticks" unanswerable in the
    // state that wants to ask.
    state.memory.fold_history(&policy.history_windows(), facts);

    // Running minimum of the range, scoped to the state AND to the target's
    // identity (see the doc comment). A target switch restarts the fold at this
    // tick's range: carrying the previous target's minimum forward would let a
    // swap to a further ship read as a huge `range_above_min_seen` and fire a
    // closest approach that never happened.
    if let Some(range) = facts.get(RANGE_TO_TARGET_FACT) {
        if facts.get(TARGET_VALID_FACT).unwrap_or(0.0) > 0.0 {
            let fingerprint = target.map(target_identity_fingerprint);
            let same_target = fingerprint == state.memory.get(MIN_RANGE_TARGET_MEMORY);
            let folded = if same_target {
                state
                    .memory
                    .get(MIN_RANGE_SEEN_MEMORY)
                    .map_or(range, |min| min.min(range))
            } else {
                range
            };
            state.memory.set(MIN_RANGE_SEEN_MEMORY, folded);
            if let Some(fingerprint) = fingerprint {
                state.memory.set(MIN_RANGE_TARGET_MEMORY, fingerprint);
            }
        }
    }

    // The private bag is seeded from THIS fine system's own state component and
    // nothing else (AC3) — including the memory-derived fact.
    let mut facts_with_memory = facts.clone();
    seed_memory_derived_facts(&mut facts_with_memory, &state.memory);
    let memory = state.memory_at(now);
    let current = state.current.clone();

    // ── Read-only policy-state diagnostics (issue #1152) ──────────────────────
    //
    // Record what the machine considered and did this tick, for the AI
    // policy-state debug surface. Both writes are diagnostic only: they are never
    // read by the machine's decision below, are not folded into the #894 digest
    // (`sim_digest` does not fold policy runtime state) and are not snapshotted,
    // so recording them cannot move a seeded run — and evaluating a guard is
    // itself side-effect free. `blocking_transition` is the inverse scan of
    // `resolve_transition` and shares its tie-break, so "which guard is holding
    // the machine" is answered the same way "which transition fires" is.
    state.blocked_transition = policy
        .blocking_transition(&current, &facts_with_memory, &memory, flags)
        .map(|t| crate::ai::policy::BlockedTransition {
            from: current.clone(),
            to: t.to.clone(),
            guard: t.when.render(),
        });

    let transition = policy.resolve_transition(&current, &facts_with_memory, &memory, flags)?;
    let to = transition.to.clone();
    let committed = crate::ai::policy::CommittedTransition {
        from: current,
        to: to.clone(),
        guard: transition.when.render(),
        at_secs: now,
    };
    state.enter(&to, now);
    state.last_transition = Some(committed);

    // Commit-time host writes. The heading is captured from THIS tick's yaw, so
    // "the current outward heading" means the heading at the merge instant.
    state.memory.set(ESCAPE_HEADING_MEMORY, yaw as f64);
    // Re-scope the running minimum to the state just entered.
    if let Some(range) = facts.get(RANGE_TO_TARGET_FACT) {
        state.memory.set(MIN_RANGE_SEEN_MEMORY, range);
    }
    Some(to)
}

/// Resolve one helm fine system's single mode channel, on whichever of the two
/// policy paths the authored content chose (issues #779, #882, #883).
///
/// A stateless policy (`machine: None` — every hull that ships today) takes the
/// frozen `resolve_channel` path. A policy that opted into the #882 machine
/// resolves the SAME channel inside the state `ai_policy_state_tick` committed
/// earlier this tick, so an entered state's outputs are live immediately.
///
/// ## The loud middle case
///
/// `(Some(machine), None)` — content declares a machine but the ship carries no
/// runtime-state component — silently fell back to the stateless path before
/// #883. That is precisely the failure mode of #882's blocking bug (a per-ship
/// AI component reaching one spawn path and not the other), and it had now
/// recurred three times. The `debug_assert!` makes a fourth recurrence stop the
/// test suite instead of quietly degrading a doctrine to its stateless shadow.
/// Release builds still degrade rather than panic — a live scenario should not
/// die over it — but they can no longer do so unnoticed in development.
fn resolve_helm_channel<'a>(
    policy: &'a crate::ai::policy::AiPolicy,
    state: Option<&crate::ai::policy::AiPolicyRuntimeState>,
    channel: &str,
    facts: &crate::world::flags::AiFacts,
    now_secs: f64,
    flags: &[&crate::world::flags::FlagStore],
) -> Option<&'a crate::ai::policy::AiPolicyVerb> {
    match (policy.machine(), state) {
        (Some(_), Some(st)) => {
            let mut facts = facts.clone();
            seed_memory_derived_facts(&mut facts, &st.memory);
            policy.resolve_channel_in_state(
                &st.current,
                channel,
                &facts,
                &st.memory_at(now_secs),
                flags,
            )
        }
        (Some(_), None) => {
            debug_assert!(
                false,
                "fine system channel '{channel}' has a STATEFUL authored policy but the ship \
                 carries no policy-state component: the machine cannot run and this would \
                 silently degrade to the stateless path. Every per-ship AI component must be \
                 declared in ai_high_fidelity_components() (src/ai/server.rs), never inserted \
                 by hand on one spawn path"
            );
            policy.resolve_channel(channel, facts, flags)
        }
        (None, _) => policy.resolve_channel(channel, facts, flags),
    }
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

/// The read-only per-ship context a [`HelmAxisHost`] seeds facts and actuates
/// from (issue #1208).
///
/// Built once per ship by the axis's system body from that body's OWN query, so
/// a field an axis does not read is simply left `None`/default — the ctx is the
/// union of what the six axes need, never a widened query. `seed` and `act` read
/// only the fields their axis populates.
pub(crate) struct HelmAxisCtx<'a> {
    /// The ship's live physics (pose, forward speed, vertical offset `y`).
    /// `None` only for the Lateral axis, whose query carries no `ShipPhysics`
    /// (its dodge reads the plan's hazard surface, never the pose) — keeping the
    /// six per-axis query footprints byte-identical to before #1208.
    pub(crate) physics: Option<&'a ShipPhysics>,
    /// `ShipPhysicsConfigResource.0.max_speed`, or `0.0` when unconfigured.
    pub(crate) max_speed: f32,
    /// This ship's shared motion plan for the tick (`HelmMotionPlan.ships`),
    /// carrying the decoded travel/facing intent, hazard surface and docking
    /// flag. `None` for a ship the planner published no entry for.
    pub(crate) plan: Option<&'a crate::ship::helm_planner::ShipMotionPlan>,
    /// This ship's shared decision frame for the tick (`HelmAiSurfacesFrame`).
    pub(crate) frame: Option<&'a HelmAiShipFrame>,
    /// World anchors, captured once per tick on the frame.
    pub(crate) anchors: &'a std::collections::HashMap<String, [f32; 3]>,
    /// The impulse drive component (phase), when the hull carries one.
    pub(crate) impulse: Option<&'a ShipImpulse>,
    /// The impulse drive per-hull config (engage/cancel distances). Its presence
    /// is the impulse capability.
    pub(crate) impulse_cfg: Option<&'a ImpulseConfigResource>,
    /// The boost drive per-hull config; `enabled` is the boost capability.
    pub(crate) boost_cfg: Option<&'a BoostConfigResource>,
    /// The boost drive component (active state), when the hull carries one.
    pub(crate) boost: Option<&'a ShipBoost>,
    /// The authored `[behaviour]` section (doctrine, avoidance sensitivities).
    pub(crate) behaviour: Option<&'a crate::entities::spawner::BehaviourSection>,
    /// The authored `[helm_capability]` section (vertical movement mode/limits).
    pub(crate) capability: Option<&'a crate::entities::spawner::HelmCapabilitySection>,
    /// This ship's per-objective patrol cursors, for target resolution.
    pub(crate) cursors: Option<&'a crate::ai::server::ObjectiveCursors>,
}

/// The policy/state/mutable-scratch a [`HelmAxisHost::act`] reads to actuate
/// (issue #1208).
///
/// Separate from [`HelmAxisCtx`] because these are the pieces an axis needs
/// mutable or policy-shaped access to — today only the Steering axis, which
/// reads its policy's leg-yield flag and mutates the pending arc-bearing
/// request. Every other axis ignores it.
pub(crate) struct HelmAxisIo<'a> {
    /// The axis's resolvable policy (Steering reads `leg_yields_to_arc_requests`).
    pub(crate) policy: Option<&'a crate::ai::policy::AiPolicy>,
    /// The axis's runtime state, for the current leg id.
    pub(crate) state: Option<&'a crate::ai::policy::AiPolicyRuntimeState>,
    /// The Weapons→Helm arc-bearing request this axis owns, mutated in place.
    pub(crate) pending: Option<&'a mut PendingArcBearingRequest>,
}

/// One helm fine-system axis, reduced to what its shared decision spine needs
/// (issue #1208). Six impls — Engines, Steering, Impulse, Lateral, Vertical,
/// Boost — collapse the six near-identical hosts behind [`run_helm_axis`].
pub(crate) trait HelmAxisHost {
    /// The fine system whose Control Source gates this axis.
    fn system_id() -> crate::core::messages::SystemId;
    /// The single output channel this axis resolves its mode verb on.
    const CHANNEL: &'static str;
    /// Whether this axis's policy may run the #882 state machine. Stateful axes
    /// (Engines, Steering, Boost) take the state-aware `resolve_channel_in_state`
    /// path; stateless ones (Impulse, Lateral, Vertical) always take the frozen
    /// `resolve_channel`, exactly as the retired `helm_policy_actuates` did.
    const STATEFUL: bool;

    /// True when `verb` is one this axis actuates on. A verb outside the set
    /// holds (the actuator latches its last input), exactly as a no-rule "hold".
    fn accepts(verb: &crate::ai::policy::AiPolicyVerb) -> bool;

    /// Seed this axis's immutable per-tick fact snapshot from the ctx.
    fn seed(cx: &HelmAxisCtx) -> crate::world::flags::AiFacts;

    /// A sanctioned override that PRECEDES the policy resolution (issue #780) —
    /// today only the Lateral axis's docking translation. `Some` emits it and
    /// skips the policy path for the tick; the default is `None`. It runs AFTER
    /// the Control-Source gate, so a human-held axis overrides nothing.
    fn pre_override(_cx: &HelmAxisCtx) -> Option<crate::core::messages::SystemControlPayload> {
        None
    }

    /// Turn the spine's verdict into the payload to admit — the axis's
    /// actuation, including any post-resolution override it owns (the Steering
    /// arc-bearing request) or on-change gating it applies (Boost, Impulse).
    /// `None` emits nothing this tick. The driver has already ruled out
    /// `NotAiOperated`; each axis decides what the remaining outcomes mean for
    /// it (most actuate only on an accepted [`HostOutcome::Act`], but Boost also
    /// releases on `Held`).
    fn act(
        outcome: crate::ai::host::HostOutcome,
        cx: &HelmAxisCtx,
        io: &mut HelmAxisIo,
    ) -> Option<crate::core::messages::SystemControlPayload>;
}

/// Resolve one helm axis's verdict through the shared [`decide`] spine, choosing
/// the stateful or stateless resolution path exactly as the retired
/// `resolve_helm_channel` / `helm_policy_actuates` pair did (issue #1208).
///
/// [`decide`]: crate::ai::host::decide
fn helm_axis_outcome<'p, H: HelmAxisHost>(
    sources: &ShipSystemControlSources,
    policy: Option<&'p crate::ai::policy::AiPolicy>,
    state: Option<&crate::ai::policy::AiPolicyRuntimeState>,
    facts: &crate::world::flags::AiFacts,
    now: f64,
    flags: &[&crate::world::flags::FlagStore],
) -> crate::ai::host::HostOutcome<'p> {
    use crate::ai::host::{decide, HostState, HostTick};
    let system = H::system_id();
    if H::STATEFUL {
        match (policy.and_then(|p| p.machine()), state) {
            // A stateful policy WITH its runtime-state component: resolve inside
            // the committed state over the memory-seeded facts and time-stamped
            // memory, byte-identical to `resolve_helm_channel`'s stateful arm.
            (Some(_), Some(st)) => {
                let mut seeded = facts.clone();
                seed_memory_derived_facts(&mut seeded, &st.memory);
                let memory = st.memory_at(now);
                let tick = HostTick {
                    system,
                    channel: H::CHANNEL,
                    facts: &seeded,
                    flags,
                    state: Some(HostState {
                        current: st.current.as_str(),
                        memory: &memory,
                    }),
                };
                decide(&sources.0, policy, &tick)
            }
            // Machine declared but no state component — the #883 loud middle
            // case: stop the test suite rather than silently degrade, then fall
            // through to the stateless path in release exactly as before.
            (Some(_), None) => {
                debug_assert!(
                    false,
                    "fine system channel '{}' has a STATEFUL authored policy but the ship \
                     carries no policy-state component: the machine cannot run and this would \
                     silently degrade to the stateless path. Every per-ship AI component must be \
                     declared in ai_high_fidelity_components() (src/ai/server.rs), never inserted \
                     by hand on one spawn path",
                    H::CHANNEL
                );
                let tick = HostTick {
                    system,
                    channel: H::CHANNEL,
                    facts,
                    flags,
                    state: None,
                };
                decide(&sources.0, policy, &tick)
            }
            (None, _) => {
                let tick = HostTick {
                    system,
                    channel: H::CHANNEL,
                    facts,
                    flags,
                    state: None,
                };
                decide(&sources.0, policy, &tick)
            }
        }
    } else {
        // Stateless axis: always the frozen `resolve_channel` path (the retired
        // `helm_policy_actuates`), machine field ignored.
        let tick = HostTick {
            system,
            channel: H::CHANNEL,
            facts,
            flags,
            state: None,
        };
        decide(&sources.0, policy, &tick)
    }
}

/// The one generic driver every per-axis helm system body runs (issue #1208):
/// seed the axis's facts, walk the [`decide`] spine, honour the axis's
/// sanctioned pre-policy override, and hand the verdict to the axis's `act`.
///
/// Returns the payload to admit, or `None` to emit nothing this tick. The
/// Control-Source gate lives only inside [`decide`] (via [`helm_axis_outcome`]):
/// a human-held or offline axis stands down here and overrides nothing.
///
/// [`decide`]: crate::ai::host::decide
fn run_helm_axis<H: HelmAxisHost>(
    sources: &ShipSystemControlSources,
    policy: Option<&crate::ai::policy::AiPolicy>,
    state: Option<&crate::ai::policy::AiPolicyRuntimeState>,
    now: f64,
    flags: &[&crate::world::flags::FlagStore],
    cx: &HelmAxisCtx,
    io: &mut HelmAxisIo,
) -> Option<crate::core::messages::SystemControlPayload> {
    let facts = H::seed(cx);
    let outcome = helm_axis_outcome::<H>(sources, policy, state, &facts, now, flags);
    // Gate — the ONE place a human/offline axis suppresses the AI.
    if matches!(outcome, crate::ai::host::HostOutcome::NotAiOperated) {
        return None;
    }
    // Sanctioned pre-policy override (Lateral docking), AFTER the gate.
    if let Some(payload) = H::pre_override(cx) {
        return Some(payload);
    }
    H::act(outcome, cx, io)
}

#[cfg(test)]
// Fixture ids only (issue #907): a test that needs "some distinct id" has no
// run to reproduce. Production identity is minted by `crate::world_id`, and
// clippy.toml bans `Uuid::new_v4` outside scopes like this one.
#[allow(clippy::disallowed_methods)]
#[path = "mod_tests.rs"]
mod tests;

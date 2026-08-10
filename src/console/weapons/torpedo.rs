//! Torpedo systems, extracted from `server.rs` (issue #728).
//!
//! Note: `crate::torpedo::TorpedoSystem` (the pure-Rust tube/magazine model
//! in `src/weapons/torpedo.rs`) is a different module from this file — this
//! one holds the Bevy systems and the ECS wrapper resource.

use bevy::prelude::*;

use super::shared::{system_is_registered, TorpedoTargetSnapshot};
use super::{
    AsteroidDestroyedVfx, ShipDestroyedVfx, TacticalRadarSelection, DEFAULT_SHIP_EXPLOSION_RADIUS,
};
use crate::entity_spawner::EntitySystemHull;
use crate::lobby::{Target, WorldResource};
use crate::messages::{
    AdmittedCommands, InterSystemMsg, InterSystemPayload, InterSystemQueue, ServerMessage,
    SystemControlPayload,
};
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipPhysics;
use crate::simulation::{AsteroidUuid, SimOutbox};
use crate::torpedo::TorpedoSystem;

/// Wraps the pure-Rust torpedo system so it can be used as a Bevy resource.
///
/// Derives both `Resource` (existing player-ship singleton path) and
/// `Component` (per-entity path, PR 5 unification).
#[derive(Resource, Component, Clone)]
pub struct TorpedoSystemResource(pub TorpedoSystem);

/// Per-ship map of each torpedo tube's inline stateless load + launch policy
/// (issue #782), keyed by `TorpedoTubeConfig.id`. The torpedo twin of
/// [`crate::weapons_plugin::BlasterBankAiPolicies`]: built at spawn from each
/// tube's authored `ai` block, falling back to the canonical
/// [`crate::entities::config::default_torpedo_tube_ai_config`] (unconditional
/// load + launch) so a tube without an authored policy keeps behaving exactly as
/// before (AC1). Read by `ai_torpedo_load` (the `torpedo_load` channel) and
/// `ai_torpedo_auto_fire` (the `torpedo_launch` channel).
#[derive(Component, Default, Clone, Debug)]
pub struct TorpedoTubeAiPolicies(
    pub std::collections::HashMap<crate::entity_config::TorpedoTubeId, crate::ai::policy::AiPolicy>,
);

/// The shared torpedo magazine's inline stateless grant policy (issue #782,
/// AC1). Resolved inside [`handle_torpedo_magazine_inter_system`] right before
/// the authoritative `claim_magazine_round`, so the magazine — the single writer
/// of `torpedoes_remaining` — consults a data-authored arbiter before granting a
/// pending claim. Built at spawn from `[torpedoes].ai`, else the canonical
/// [`crate::entities::config::default_torpedo_magazine_ai_config`] (unconditional
/// grant), so baseline claim behaviour is preserved.
#[derive(Component, Default, Clone, Debug)]
pub struct TorpedoMagazineAiPolicy(pub crate::ai::policy::AiPolicy);

/// Seed the per-tick policy fact snapshot for one torpedo tube's LOAD decision
/// (issue #782), the torpedo twin of
/// [`crate::weapons_plugin::seed_blaster_bank_facts`]. Closes the #779
/// empty-facts edge for torpedo tubes: the host resolves the tube's live loading
/// state before calling this, so a `fact(...)` guard evaluates over real per-tube
/// state while `policy.rs` stays Bevy-free (AGENTS.md #10).
pub fn seed_torpedo_tube_load_facts(
    loaded_count: u32,
    target_count: u32,
    ai_target_count: u32,
    magazine: u32,
    operates_ai: bool,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("loaded_count", loaded_count as f64);
    facts.set("target_count", target_count as f64);
    facts.set("ai_target_count", ai_target_count as f64);
    facts.set("magazine", magazine as f64);
    facts.set("operates_ai", if operates_ai { 1.0 } else { 0.0 });
    facts
}

/// Seed the per-tick policy fact snapshot for one torpedo tube's LAUNCH decision
/// (issue #782). Mirrors [`seed_torpedo_tube_load_facts`]; the host has already
/// resolved the tube's live readiness (loaded, target valid, in range, in arc)
/// and the shield arc the shot would strike before calling this.
///
/// `tubes_full` is the SHIP-WIDE reading added by issue #791: every tube on this
/// ship at `loaded_count == volley_max`. It is deliberately not derivable from
/// the per-tube `loaded` fact, which is `loaded_count > 0` — the two answer
/// different questions, and a doctrine that fires a whole salvo into a shield
/// gap in one go needs the stronger one. Note `target_facing_shields` beside it
/// is an HP reading, not a boolean: `<= 0` means the striking arc is not
/// blocking (down, or absent entirely).
/// `red_alert` is the other SHIP-WIDE reading, added by issue #872 — this
/// ship's own [`crate::ship_state::ShipRedAlert`]. Seeded on the LAUNCH
/// snapshot only: loading a tube and granting a round from the magazine are not
/// offensive fire and stay ungated.
#[allow(clippy::too_many_arguments)]
pub fn seed_torpedo_tube_launch_facts(
    loaded: bool,
    target_valid: bool,
    in_range: bool,
    in_arc: bool,
    target_facing_shields: i32,
    tubes_full: bool,
    red_alert: bool,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("loaded", if loaded { 1.0 } else { 0.0 });
    facts.set("target_valid", if target_valid { 1.0 } else { 0.0 });
    facts.set("in_range", if in_range { 1.0 } else { 0.0 });
    facts.set("in_arc", if in_arc { 1.0 } else { 0.0 });
    facts.set(
        crate::entities::config::TARGET_FACING_SHIELDS_FACT,
        target_facing_shields as f64,
    );
    facts.set("tubes_full", if tubes_full { 1.0 } else { 0.0 });
    facts.set(
        crate::entities::config::POWER_RED_ALERT_FACT,
        if red_alert { 1.0 } else { 0.0 },
    );
    facts
}

/// Seed the per-tick policy fact snapshot for the shared magazine's GRANT
/// decision (issue #782). `magazine` is the live `torpedoes_remaining`;
/// `in_flight` is the count of this ship's torpedoes currently in flight — the
/// AC5 public fact the magazine policy can gate on.
pub fn seed_torpedo_magazine_facts(magazine: u32, in_flight: u32) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("magazine", magazine as f64);
    facts.set("in_flight", in_flight as f64);
    facts
}

/// Seed the policy fact snapshot for the shared magazine's CONSERVATION
/// decision (issue #943) — the world-scoped half of the torpedo doctrine,
/// resolved once per ship per tick, ahead of that ship's admitted command loop
/// in [`handle_fire_torpedo`], for human-origin and AI-origin launches alike.
///
/// `rounds_aboard` is [`crate::torpedo::TorpedoSystem::rounds_aboard`] — the
/// magazine PLUS the rounds already parked in the tubes, not the bare
/// `torpedoes_remaining` counter, which a hull with a "keep the tubes loaded"
/// doctrine drives permanently below what it is actually carrying and which
/// would therefore strand the parked volley for the rest of the mission.
/// `mission_threat_remaining` is the scenario's own
/// [`crate::entities::config::MISSION_THREAT_REMAINING_COUNTER`] as this ship's
/// layered flag chain reads it, so nothing here knows how long a mission is —
/// the world says. `targeted_objective_count` is how many of the ship's own
/// `[behaviour].doctrine` entries are a Destroy directive naming its target, the
/// reading a sole-objective carve-out clause gates on.
///
/// The derived `rounds_per_threat` exists because the predicate grammar
/// compares ONE atom to ONE operand and has no arithmetic: "rounds per remaining
/// unit of threat" is the quantity a reserve is authored against, and only the
/// host can compute it. With no remaining threat published it is
/// `f64::INFINITY`, so an unpaced world (and a mission whose threat is spent)
/// takes the permissive branch of `>= param(...)` — the pre-#943 behaviour, and
/// the reason a world that authors no counter is unaffected.
pub fn seed_torpedo_conservation_facts(
    rounds_aboard: u32,
    mission_threat_remaining: i64,
    targeted_objective_count: usize,
) -> crate::world::flags::AiFacts {
    use crate::entities::config as cfg;
    let mut facts = crate::world::flags::AiFacts::new();
    let remaining = mission_threat_remaining.max(0);
    facts.set(cfg::TORPEDO_ROUNDS_ABOARD_FACT, rounds_aboard as f64);
    facts.set(cfg::TORPEDO_MISSION_THREAT_FACT, remaining as f64);
    facts.set(
        cfg::TORPEDO_ROUNDS_PER_THREAT_FACT,
        if remaining > 0 {
            rounds_aboard as f64 / remaining as f64
        } else {
            f64::INFINITY
        },
    );
    facts.set(
        cfg::TORPEDO_TARGETED_OBJECTIVE_COUNT_FACT,
        targeted_objective_count as f64,
    );
    facts
}

/// How many of a ship's standing doctrine entries name a specific Destroy
/// target — the carve-out lever of [`seed_torpedo_conservation_facts`]
/// (issue #943).
///
/// See [`crate::entities::config::TORPEDO_TARGETED_OBJECTIVE_COUNT_FACT`] for
/// why the question is "how many NAMED targets" rather than "how many doctrine
/// entries": a world's spawn override appends its brief to the template's
/// standing orders instead of replacing them, so the entry count of a ship sent
/// after one specific target is never 1.
pub fn targeted_objective_count(behaviour: &crate::entities::config::BehaviourConfig) -> usize {
    behaviour
        .doctrine
        .iter()
        .filter(|d| d.directive_kind.as_deref() == Some("Destroy") && d.directive_target.is_some())
        .count()
}

/// Does this magazine policy author a conservation doctrine at all (issue #943)?
///
/// The difference between "this hull holds its rounds back" and "this hull was
/// never asked to". A policy with no rule on
/// [`crate::entities::config::TORPEDO_CONSERVATION_CHANNEL`] resolves that
/// channel to `None`, which is indistinguishable from an authored guard that
/// declined — so without this question every legacy hull, every bare-`App`
/// fixture and every world that publishes no threat counter would silently stop
/// launching torpedoes the moment the channel existed. Conservation is content:
/// unauthored means unconstrained.
///
/// Scans the stateless rules and NOTHING else, because the magazine host
/// resolves this channel statelessly: [`torpedo_conservation_policy_fires`] goes
/// through [`crate::ai::policy::AiPolicy::resolve_channel`], which reads
/// `self.rules` and never `self.machine`. A machine-shaped `[torpedoes].ai` is
/// authorable and validates, so a conservation rule CAN be written into a state
/// — and would be unreachable from here. Counting it would invert the default
/// this whole question exists to protect: declared, never fires, holds for ever,
/// muting that hull's torpedoes for the entire mission. So a state-authored rule
/// fails OPEN, exactly like the unauthored case above.
pub fn torpedo_conservation_declared(policy: &crate::ai::policy::AiPolicy) -> bool {
    let channel = crate::entities::config::TORPEDO_CONSERVATION_CHANNEL;
    policy.rules.iter().any(|r| r.channel == channel)
}

/// Resolve the shared magazine's policy to a bare "spend a round on this launch?"
/// boolean (issue #943). Returns `true` only when a guard fires on the
/// `torpedo_conservation` channel yielding `ReleaseTorpedo`; `None`/idle/
/// mismatched verbs "hold" — the launch is dropped and the round stays loaded.
///
/// Callers must ask [`torpedo_conservation_declared`] first: an unauthored
/// channel also resolves to `None`, and that case means "no conservation
/// doctrine", not "hold".
pub fn torpedo_conservation_policy_fires(
    policy: &crate::ai::policy::AiPolicy,
    facts: &crate::world::flags::AiFacts,
    flags: &[&crate::world::flags::FlagStore],
) -> bool {
    policy.resolve_channel(
        crate::entities::config::TORPEDO_CONSERVATION_CHANNEL,
        facts,
        flags,
    ) == Some(&crate::ai::policy::AiPolicyVerb::ReleaseTorpedo)
}

/// Resolve a torpedo tube's policy to a bare "load this tick?" boolean
/// (issue #782). Returns `true` only when a guard fires on the `torpedo_load`
/// channel yielding `LoadTorpedo`; `None`/idle/mismatched verbs "hold".
pub fn torpedo_tube_load_policy_fires(
    policy: &crate::ai::policy::AiPolicy,
    facts: &crate::world::flags::AiFacts,
    flags: &[&crate::world::flags::FlagStore],
) -> bool {
    policy.resolve_channel(crate::entities::config::TORPEDO_LOAD_CHANNEL, facts, flags)
        == Some(&crate::ai::policy::AiPolicyVerb::LoadTorpedo)
}

/// Resolve a torpedo tube's policy to a bare "launch this tick?" boolean
/// (issue #782). Returns `true` only when a guard fires on the `torpedo_launch`
/// channel yielding `LaunchTorpedo`; `None`/idle/mismatched verbs "hold".
pub fn torpedo_tube_launch_policy_fires(
    policy: &crate::ai::policy::AiPolicy,
    facts: &crate::world::flags::AiFacts,
    flags: &[&crate::world::flags::FlagStore],
) -> bool {
    policy.resolve_channel(
        crate::entities::config::TORPEDO_LAUNCH_CHANNEL,
        facts,
        flags,
    ) == Some(&crate::ai::policy::AiPolicyVerb::LaunchTorpedo)
}

/// Resolve the shared magazine's policy to a bare "grant this claim?" boolean
/// (issue #782). Returns `true` only when a guard fires on the
/// `torpedo_magazine_grant` channel yielding `GrantTorpedoRound`; `None`/idle/
/// mismatched verbs "hold" (refuse the claim without touching the counter).
pub fn torpedo_magazine_grant_policy_fires(
    policy: &crate::ai::policy::AiPolicy,
    facts: &crate::world::flags::AiFacts,
    flags: &[&crate::world::flags::FlagStore],
) -> bool {
    policy.resolve_channel(
        crate::entities::config::TORPEDO_MAGAZINE_CHANNEL,
        facts,
        flags,
    ) == Some(&crate::ai::policy::AiPolicyVerb::GrantTorpedoRound)
}

/// Admitted-command consumer for `LoadTube` (issue #846).
///
/// Reads each ship's own `AdmittedCommands` for `LoadTube` payloads targeting
/// `torpedo-tube-<id>`, resolves the tube from the target `SystemId`, gates
/// on the tube's fine-system policy, and emits a channel-2
/// `ClaimTorpedoRound` to the magazine. The magazine consumer
/// (`handle_torpedo_magazine_inter_system`) decides whether to grant the
/// round and start loading.
///
/// Runs in `SimSet::Physics` — after AI emitters have written their admitted
/// commands, so AI load orders are not silently dropped.
pub(crate) fn handle_load_tube(
    mut ship_query: Query<
        (Entity, &ShipSystemControlSources, &AdmittedCommands),
        With<crate::server_app::Ship>,
    >,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    for (ship_entity, control_sources, admitted) in ship_query.iter_mut() {
        for cmd in admitted.0.iter() {
            let SystemControlPayload::LoadTube = &cmd.payload else {
                continue;
            };
            // The target SystemId must be a known torpedo-tube-* id.
            // Resolve by stripping the prefix — the tube id is the suffix.
            let tube_id = cmd.target.0.strip_prefix("torpedo-tube-").map(|s| {
                // Restore underscores that were folded to hyphens.
                s.replace('-', "_")
            });
            let Some(tube_id) = tube_id else {
                continue;
            };

            // Gate on the tube's fine-system policy (default-source policy
            // for unregistered ids — issue #801). Admission already gated
            // the token; this is a system-state gate.
            let tube_system_id = crate::system_registry::torpedo_tube_system_id(&tube_id)
                .filter(|id| system_is_registered(control_sources, id));
            let tube_policy = match &tube_system_id {
                Some(id) => control_sources.0.policy_for(id),
                None => crate::ship::control_source::control_tick_policy(
                    crate::ship::control_source::ControlSource::default(),
                ),
            };
            if !tube_policy.accept_human_input && !tube_policy.operate_ai {
                continue;
            }

            inter_system.0.push(InterSystemMsg {
                target: crate::system_registry::torpedo_magazine_system_id(),
                payload: InterSystemPayload::ClaimTorpedoRound { tube: tube_id },
                source_entity: Some(ship_entity),
            });
        }
    }
}

/// Admitted-command consumer for `UnloadTube` (issue #846).
///
/// Reads each ship's own `AdmittedCommands` for `UnloadTube` payloads,
/// resolves the tube from the target `SystemId`, gates on the tube's
/// fine-system policy, then calls [`TorpedoSystem::start_unload`] on the
/// ship's own `TorpedoSystemResource` component.
///
/// Runs in `SimSet::Physics` — after AI emitters have written their admitted
/// commands.
pub(crate) fn handle_unload_tube(
    mut ship_query: Query<
        (
            &ShipSystemControlSources,
            &AdmittedCommands,
            Option<&mut TorpedoSystemResource>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
) {
    for (control_sources, admitted, torpedo_sys_comp) in ship_query.iter_mut() {
        let mut torpedo_sys_comp = torpedo_sys_comp;
        let torpedo_sys: &mut TorpedoSystem = match torpedo_sys_comp.as_deref_mut() {
            Some(c) => &mut c.0,
            None => &mut torpedo_sys_res.0,
        };

        for cmd in admitted.0.iter() {
            let SystemControlPayload::UnloadTube = &cmd.payload else {
                continue;
            };

            // Resolve the command's target to one of THIS ship's tubes by
            // running the canonical forward mapping and comparing.
            let Some(tube_id) = torpedo_sys
                .tubes
                .iter()
                .find(|t| {
                    crate::system_registry::torpedo_tube_system_id(&t.id).as_ref()
                        == Some(&cmd.target)
                })
                .map(|t| t.id.clone())
            else {
                continue;
            };

            // Gate on the tube's fine-system policy (default-source policy
            // for unregistered ids — issue #801). Both origins allowed.
            let is_registered = system_is_registered(control_sources, &cmd.target);
            let tube_policy = if is_registered {
                control_sources.0.policy_for(&cmd.target)
            } else {
                crate::ship::control_source::control_tick_policy(
                    crate::ship::control_source::ControlSource::default(),
                )
            };
            if !tube_policy.accept_human_input && !tube_policy.operate_ai {
                continue;
            }

            torpedo_sys.start_unload(&tube_id);
        }
    }
}

/// Handle `ControlSystem { target: "torpedo-tube-<id>", payload: SetTorpedoVolleyTarget { count } }`.
///
/// Reads every ship's own `AdmittedCommands`, resolves the tube id from the
/// target SystemId, gates on the tube's fine-system policy, then calls
/// [`TorpedoSystem::set_volley_target`] on that ship's own torpedo system.
///
/// # Why per-ship `AdmittedCommands` rather than raw `InboundMessage`
///
/// This used to read the inbound message stream and query `With<LocalShip>`,
/// which meant an NPC could never receive the command — so when
/// `console_ai::server::ai_torpedo_load` started issuing volley orders for
/// AI-crewed ships, they would have been dropped on the floor. Same shape and
/// same reason as [`crate::console::captain::handle_set_red_alert`]: the AI
/// pushes into each ship's own `AdmittedCommands`, so the consumer has to
/// iterate every ship (`With<Ship>`), not just the player's.
///
/// Admission (`admit_system_commands`) has already answered "may this token
/// do this?" — including that a human holds the console owning the tube — and
/// strips the source identity, so there is no token check here and no
/// human-vs-AI branch. What remains is the *system-state* gate: the tube must
/// be operable at all. A tube whose fine system is `Offline` (rating or damage
/// tier) rejects volley orders from either origin.
///
/// A ship with no `TorpedoSystemResource` component simply has no tubes and is
/// skipped: the global `TorpedoSystemResource` Resource mirrors the LOCAL
/// ship's magazine, so falling back to it here would let one ship's volley
/// order retarget another ship's tubes (issue #738 removed the same fallback
/// from `handle_unload_tube` / `handle_fire_torpedo` for exactly that reason).
///
/// Runs in `SimSet::Input`.
pub(crate) fn handle_set_torpedo_volley_target(
    mut ship_query: Query<
        (
            &ShipSystemControlSources,
            &crate::messages::AdmittedCommands,
            Option<&mut TorpedoSystemResource>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (control_sources, admitted, torpedo_sys_comp) in ship_query.iter_mut() {
        // The ship's own component, never the global Resource (issue #738).
        let mut torpedo_sys_comp = torpedo_sys_comp;
        let Some(torpedo_sys) = torpedo_sys_comp.as_deref_mut().map(|c| &mut c.0) else {
            continue;
        };
        let torpedo_sys: &mut TorpedoSystem = torpedo_sys;
        for cmd in admitted.0.iter() {
            let SystemControlPayload::SetTorpedoVolleyTarget { count } = &cmd.payload else {
                continue;
            };
            // Resolve the command's target back to one of THIS ship's tubes by
            // running the canonical forward mapping
            // (`system_registry::torpedo_tube_system_id`) over each tube id and
            // comparing — never by inverting the string.
            //
            // The inverse ("strip `torpedo-tube-`, put the underscores back")
            // is lossy: the mapping folds `_` to `-`, so a hull that authors its
            // tubes with hyphens (`id = "fore-port"`, as `alliance_battleship`
            // does) came back as `fore_port`, matched no tube, and every volley
            // order for that hull was silently dropped — its AI crew never
            // loaded a round in its life. Comparing forward-mapped ids accepts
            // either spelling and keeps one resolver instead of two.
            let Some(tube_id) = torpedo_sys
                .tubes
                .iter()
                .find(|t| {
                    crate::system_registry::torpedo_tube_system_id(&t.id).as_ref()
                        == Some(&cmd.target)
                })
                .map(|t| t.id.clone())
            else {
                continue;
            };
            // Gate on the tube's own fine-system policy (default-source policy
            // for unregistered ids — issue #801). Operable for *either*
            // origin: `accept_human_input` for a Human tube, `operate_ai` for
            // an Ai one. Both false means Offline — nobody may load it.
            let is_registered = system_is_registered(control_sources, &cmd.target);
            let tube_policy = if is_registered {
                control_sources.0.policy_for(&cmd.target)
            } else {
                // Unregistered fine system → default-source policy (issue #801).
                crate::ship::control_source::control_tick_policy(
                    crate::ship::control_source::ControlSource::default(),
                )
            };
            if !tube_policy.accept_human_input && !tube_policy.operate_ai {
                continue;
            }
            torpedo_sys.set_volley_target(&tube_id, *count);
        }
    }
}

/// Admitted-command consumer for `FireTorpedo` (issue #846).
///
/// Reads each ship's own `AdmittedCommands` for `FireTorpedo` payloads,
/// resolves the tube id from the target `SystemId`, gates on the tube's
/// fine-system policy and the magazine's policy, then calls
/// [`TorpedoSystem::launch`].
///
/// Runs in `SimSet::Physics` — after the AI decider (`ai_torpedo_auto_fire`,
/// in `SimSet::Physics` via `ConsoleAiPlugin`) has emitted its admitted
/// commands, but within the same tick so torpedoes launch without a tick of
/// queue lag.
///
/// No `InboundMessage` / token resolution: admission stripped the source
/// identity. No human-vs-AI branch below this point.
///
/// # Why the world-scoped conservation gate lives here (issue #943)
///
/// This is the ONLY consumer of `SystemControlPayload::FireTorpedo`, for every
/// ship in the world. `ai_torpedo_auto_fire` does not launch anything — it
/// resolves the tube's authored `torpedo_launch` doctrine and then emits the
/// same admitted command a human's Tactical console does, which lands here. A
/// gate placed in the AI decider (the shape the red-alert gate of issue #872
/// takes, because it belongs to the tube) would therefore constrain NPC crews
/// and leave a human player free to empty the magazine into the first wave —
/// which is exactly the defect #943 reports. So the magazine's
/// `torpedo_conservation` channel is resolved below, downstream of admission,
/// where the source identity is already gone and there is nothing to branch on.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_fire_torpedo(
    mut ship_q: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            &Transform,
            Option<&crate::model_rig::ModelMarkers>,
            &AdmittedCommands,
            Option<&crate::server_app::ShipSystemBlackboards>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&mut TorpedoSystemResource>,
            Option<&mut crate::server_app::WeaponFiredThisTick>,
            // The world-scoped conservation gate's three inputs (issue #943):
            // the magazine's authored doctrine, this ship's own standing
            // objectives, and the layer it was spawned into (which anchors the
            // flag chain the mission counter is read through).
            Option<&TorpedoMagazineAiPolicy>,
            Option<&crate::entity_spawner::BehaviourSection>,
            Option<&crate::world::server::EntityOriginLayer>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
    // Torpedo ids are minted from the tick-scoped counter (issue #907), same
    // reason as the blaster's projectile ids: an id that is a function of
    // draw order made two instances diverge even on the same seed.
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
    // Read-only scenario flag/counter chain (issue #943, same shape as
    // `handle_torpedo_magazine_inter_system`). `Option` so bare-`App` fixtures
    // still pass parameter validation — absent, the chain is empty and the
    // mission counter reads 0, i.e. no conservation pressure.
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
) {
    for (
        control_sources,
        physics,
        transform,
        markers_opt,
        admitted,
        blackboards_opt,
        source_uuid_opt,
        torpedo_sys_comp,
        weapon_fired_comp,
        magazine_policy_opt,
        behaviour_opt,
        origin_layer_opt,
    ) in ship_q.iter_mut()
    {
        // Per-entity component first; global Resource fallback for legacy tests.
        let mut torpedo_sys_comp = torpedo_sys_comp;
        let torpedo_sys: &mut crate::torpedo::TorpedoSystem = match torpedo_sys_comp.as_deref_mut()
        {
            Some(c) => &mut c.0,
            None => &mut torpedo_sys_res.0,
        };

        // Nothing below this point concerns a ship with no launch to gate, and
        // this system runs for EVERY `With<Ship>` entity on every `FixedUpdate`
        // tick — the axis a horde scenario grows, while an admitted set holding
        // a `FireTorpedo` is the rare tick. Asking the cheap question first
        // keeps the snapshot's cost (a flag-chain `Vec`, a fact map, and a full
        // predicate resolve) on the ticks that actually spend a round; it is the
        // same short-circuit `handle_torpedo_magazine_inter_system` takes on an
        // empty claim list. Placed AFTER the `torpedo_sys` binding above so the
        // component's change-detection behaviour is untouched.
        if !admitted
            .0
            .iter()
            .any(|c| matches!(c.payload, SystemControlPayload::FireTorpedo { .. }))
        {
            continue;
        }

        // ── World-scoped conservation snapshot (issue #943) ─────────────────
        //
        // Resolved once per ship rather than per command, which is also what
        // makes a same-tick multi-tube salvo ONE decision instead of a race
        // between tubes. `rounds_aboard` does move inside the loop — a launch
        // spends rounds — and taking the reading before the first of them is
        // deliberate: the volley caps (issue #942) decide how much a single
        // opportunity may spend, conservation decides whether the ship can
        // afford to take the opportunity at all.
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_layer_opt,
            runtime.as_deref(),
            layers.as_deref(),
        );
        let conservation_facts = seed_torpedo_conservation_facts(
            torpedo_sys.rounds_aboard(),
            crate::world::flags::counter_in_chain(
                &flag_chain,
                crate::entities::config::MISSION_THREAT_REMAINING_COUNTER,
            ),
            behaviour_opt
                .map(|b| targeted_objective_count(&b.0))
                .unwrap_or(0),
        );
        // A ship whose magazine authors no conservation doctrine — and every
        // fixture with no `TorpedoMagazineAiPolicy` at all — is unconstrained,
        // exactly as before the channel existed.
        let conservation_holds = magazine_policy_opt
            .map(|p| &p.0)
            .filter(|p| torpedo_conservation_declared(p))
            .is_some_and(|p| {
                !torpedo_conservation_policy_fires(p, &conservation_facts, &flag_chain)
            });

        // Track whether any command in this ship's admitted set fired a torpedo,
        // so the WeaponFiredThisTick component (Mut<T>, not Copy) is only set
        // once, outside the inner loop.
        let mut any_fired = false;

        for cmd in admitted.0.iter() {
            let SystemControlPayload::FireTorpedo { target_uuid } = &cmd.payload else {
                continue;
            };

            // Resolve the command's target to one of THIS ship's tubes by
            // running the canonical forward mapping and comparing.
            let Some(tube_id) = torpedo_sys
                .tubes
                .iter()
                .find(|t| {
                    crate::system_registry::torpedo_tube_system_id(&t.id).as_ref()
                        == Some(&cmd.target)
                })
                .map(|t| t.id.clone())
            else {
                continue;
            };

            // Gate on the tube's fine-system policy (default-source policy
            // for unregistered ids — issue #801). Admission gated the token;
            // this is a system-state gate.
            let is_registered = system_is_registered(control_sources, &cmd.target);
            let tube_policy = if is_registered {
                control_sources.0.policy_for(&cmd.target)
            } else {
                crate::ship::control_source::control_tick_policy(
                    crate::ship::control_source::ControlSource::default(),
                )
            };
            if !tube_policy.accept_human_input && !tube_policy.operate_ai {
                continue;
            }

            // Magazine-online gate: a Disabled/Destroyed magazine blocks fire.
            let magazine_id = crate::system_registry::torpedo_magazine_system_id();
            let magazine_declared = control_sources
                .0
                .entries()
                .any(|(id, _)| id == &magazine_id)
                || control_sources.0.is_offline(&magazine_id);
            if magazine_declared {
                let magazine_policy = control_sources.0.policy_for(&magazine_id);
                if !magazine_policy.accept_human_input && !magazine_policy.operate_ai {
                    continue;
                }
            }

            // Conservation gate (issue #943). The last gate before the round is
            // spent, and the only one that is about the MISSION rather than
            // about this ship's hardware: holding drops the launch without
            // unloading the tube, so the same decision is offered again next
            // tick and a shot held for wave 6 is still a shot.
            if conservation_holds {
                continue;
            }

            let uuid = crate::world_id::mint_id_with(
                id_mint.as_deref(),
                crate::world_id::IdNamespace::Projectile,
            );
            let tube_facing_rad = torpedo_sys
                .tube(tube_id.as_str())
                .map(|t| t.facing_deg.to_radians())
                .unwrap_or(0.0);
            let launch_heading = physics.yaw + tube_facing_rad;
            let source_uuid = source_uuid_opt.map(|u| u.0.clone());
            // Homing target: the ship's frozen Combat Lock (issue #829),
            // else the explicit target on the FireTorpedo payload.
            let combat_lock = match blackboards_opt
                .as_ref()
                .and_then(|bbs| bbs.0.get(&crate::system_registry::viewscreen_system_id()))
            {
                Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
                _ => None,
            };
            let homing_uuid: Option<String> = combat_lock.or_else(|| target_uuid.clone());
            // Resolve one world-XZ origin per authored barrel marker (issue
            // #766), mirroring `tick_blaster_system`. A tube with no barrels
            // authored yields an empty slice, so `launch_with_barrels` falls
            // back to the ship-centre origin — the pre-#766 behaviour, so
            // shipped single-marker torpedo hulls are unchanged. Each barrel
            // marker resolves via the rig; an unresolved marker falls back to
            // ship centre.
            // Keep the marker's full 3D position — `pos.y` is no longer dropped
            // (issue #768). A patterned-origin barrel therefore launches at its
            // authored altitude, and the ship-centre fallback carries the hull's
            // live `physics.y` (0 for Planar hulls).
            let barrel_origins: Vec<(f32, f32, f32)> = torpedo_sys
                .tube(tube_id.as_str())
                .map(|t| t.barrels.clone())
                .unwrap_or_default()
                .iter()
                .map(|name| {
                    markers_opt
                        .and_then(|m| m.resolve_world_position(transform, name))
                        .map(|pos| (pos.x, pos.y, pos.z))
                        .unwrap_or((physics.x, physics.y, physics.z))
                })
                .collect();
            use crate::torpedo::LaunchResult;
            let result = torpedo_sys.launch_with_barrels(
                tube_id.as_str(),
                uuid.clone(),
                &barrel_origins,
                physics.x,
                physics.y,
                physics.z,
                launch_heading,
                homing_uuid.clone(),
                source_uuid.clone(),
            );
            match result {
                LaunchResult::Launched {
                    uuid: launched_uuid,
                    ..
                } => {
                    any_fired = true;
                    // The immediate torpedo's real spawn origin (barrel marker
                    // or ship centre) so the broadcast matches the sim. Burst
                    // rounds carry their own origins via `burst_launched`.
                    let (launch_x, launch_y, launch_z) = torpedo_sys
                        .in_flight
                        .iter()
                        .rev()
                        .find(|t| t.uuid == launched_uuid)
                        .map(|t| (t.x, t.y, t.z))
                        .unwrap_or((physics.x, physics.y, physics.z));
                    if let Some(ref mut msgs) = balance_events {
                        msgs.write(crate::balance::BalanceEvent::WeaponFired {
                            shooter: source_uuid.clone().filter(|u| !u.is_empty()),
                            weapon: tube_id.clone(),
                            kind: crate::balance::FIRED_KIND_TORPEDO.to_string(),
                        });
                    }
                    outbox.0.push((
                        Target::All,
                        ServerMessage::TorpedoLaunched {
                            uuid: launched_uuid,
                            tube: tube_id.clone(),
                            x: launch_x,
                            y: launch_y,
                            z: launch_z,
                            heading: launch_heading,
                        },
                    ));
                }
                LaunchResult::TubeNotLoaded
                | LaunchResult::NoTorpedoes
                | LaunchResult::UnknownTube => {}
            }
        }

        // Set WeaponFiredThisTick once if any command in this ship's admitted
        // set resulted in a successful launch (Mut<T> is not Copy, so avoid
        // repeated moves inside the inner loop).
        if any_fired {
            if let Some(mut wf) = weapon_fired_comp {
                wf.0 = true;
            }
        }
    }
}

/// Consumer for the Torpedo Magazine's inbound channel-2 `ClaimTorpedoRound`
/// messages (issue #512).
///
/// Runs in `SimSet::Physics` on ANY ship carrying a `TorpedoSystemResource`
/// component (`With<Ship>`) — routing by `source_entity` mirrors the
/// [`crate::ship::power::handle_power_inter_system`] pattern so multiple
/// ships with magazines each mutate their own state. Falls back to the
/// LocalShip when `source_entity` is `None` (legacy path), and to the
/// global `TorpedoSystemResource` when no matching Ship entity exists at
/// all (legacy test paths without a Ship entity).
///
/// For every `ClaimTorpedoRound` targeted at
/// [`crate::system_registry::torpedo_magazine_system_id`]:
///
/// 1. Refuse the claim (no-op) when the magazine is offline (Disabled /
///    Destroyed hull tier — reflected as `!accept_human_input && !operate_ai`
///    in the control-source resolver).
/// 2. Refuse the claim when the shared magazine counter is zero.
/// 3. Otherwise decrement the counter and start loading the named tube via
///    [`crate::torpedo::TorpedoSystem::start_load_reserved`].
///
/// This is the sole path the Bevy weapons handler uses to consume from the
/// magazine — the tube handler (`handle_load_tube`) only *sends* the claim.
pub fn handle_torpedo_magazine_inter_system(
    queue: Res<InterSystemQueue>,
    // Read-only scenario flag/counter chain (issue #891 stage 2). `Option` so
    // bare-`App` fixtures still pass parameter validation.
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`
    // — this call site is per-claim (worst case in the crate).
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    mut ship_q: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &mut TorpedoSystemResource,
            Option<&TorpedoMagazineAiPolicy>,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
) {
    let magazine_id = crate::system_registry::torpedo_magazine_system_id();
    // Collect targeted claims: (source_entity, tube_id). Only claims for the
    // magazine system are relevant here — everything else is ignored.
    let claims: Vec<(Option<Entity>, String)> = queue
        .0
        .iter()
        .filter(|m| m.target == magazine_id)
        .filter_map(|m| match &m.payload {
            InterSystemPayload::ClaimTorpedoRound { tube } => Some((m.source_entity, tube.clone())),
            _ => None,
        })
        .collect();
    if claims.is_empty() {
        return;
    }

    // Snapshot the LocalShip entity once so `source_entity: None` (legacy
    // path) resolves to the player ship consistently across the loop.
    let local_ship_entity: Option<Entity> =
        ship_q
            .iter()
            .find_map(|(e, _, _, _, is_local)| if is_local { Some(e) } else { None });

    for (source_entity, tube_id) in claims {
        let target_entity = source_entity.or(local_ship_entity);
        if let Some(target) = target_entity {
            if let Ok((_e, control_sources, mut torpedo_sys, mag_policy_opt, _is_local)) =
                ship_q.get_mut(target)
            {
                // Gate: magazine must be online (or absent → treat as online for
                // ships that don't declare a magazine fine system, preserving
                // legacy behaviour). The `torpedo_magazine` system is added to
                // the resolver by lobby setup when the ship TOML declares it.
                let magazine_declared = control_sources
                    .0
                    .entries()
                    .any(|(id, _)| id == &magazine_id)
                    || control_sources.0.is_offline(&magazine_id);
                if magazine_declared {
                    let policy = control_sources.0.policy_for(&magazine_id);
                    if !policy.accept_human_input && !policy.operate_ai {
                        // This ship's magazine is offline — refuse this claim.
                        // Other ships' claims (different `source_entity`) are
                        // still handled below in subsequent iterations.
                        continue;
                    }
                }
                // AC1/AC6: the shared magazine consults its authored grant policy
                // right before the authoritative reservation. The offline gate
                // above stays the hard authority; this data-authored arbiter can
                // refuse a claim (e.g. hold rounds while in-flight torpedoes are
                // high) without ever becoming a second writer of
                // `torpedoes_remaining`. Claims are still drained in queue order,
                // so same-tick contention stays deterministic. Facts read the
                // live counter plus this ship's in-flight count (the AC5 fact).
                //
                // No attached policy ⇒ no grant. Since #885b stage 5d there is
                // no synthesised stand-in, and strict AI-declaration mode
                // rejects a `[torpedoes]` block that authors no `ai`.
                let Some(mag_policy) = mag_policy_opt.map(|p| &p.0) else {
                    continue;
                };
                let facts = seed_torpedo_magazine_facts(
                    torpedo_sys.0.torpedoes_remaining,
                    torpedo_sys.0.in_flight.len() as u32,
                );
                // The scenario flag chain, anchored at the layer that spawned
                // this ship (issue #891 stage 2).
                let flag_chain = crate::world::server::entity_flag_chain(
                    origin_q.get(target).ok(),
                    runtime.as_deref(),
                    layers.as_deref(),
                );
                if !torpedo_magazine_grant_policy_fires(mag_policy, &facts, &flag_chain) {
                    continue; // authored policy refuses this claim.
                }
                if !torpedo_sys.0.claim_magazine_round() {
                    continue; // magazine empty — refuse this claim.
                }
                if !torpedo_sys.0.start_load_reserved(&tube_id) {
                    // Tube already loaded / unknown — return the round to the magazine.
                    torpedo_sys.0.torpedoes_remaining += 1;
                }
                continue;
            }
        }
        // Resource-only fallback (no Ship entity with the component).
        if !torpedo_sys_res.0.claim_magazine_round() {
            continue;
        }
        if !torpedo_sys_res.0.start_load_reserved(&tube_id) {
            torpedo_sys_res.0.torpedoes_remaining += 1;
        }
    }
}

/// Phase 1 of the torpedo tick (issue #724): build the one-tick
/// [`TorpedoTargetSnapshot`] — target positions (live ECS with a
/// `WorldResource` fallback) and the proximity detonation target list —
/// which `tick_torpedo_lifecycle` reads later in the same tick.
pub(crate) fn build_torpedo_target_snapshot(
    world: Res<WorldResource>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    // Virtual entities (asteroid-field anchors, region trigger volumes) are
    // organisational/effect-only. They carry an `EntityUuid` and a non-zero
    // `radius` in the world snapshot (from `outer_radius` or region shape),
    // so without this filter `find_detonation_hits` treats them as giant
    // hittable targets — and a torpedo fired anywhere inside a 350 m
    // asteroid-field annulus detonates on the field anchor on its first
    // physics tick. (Regression that made torpedoes invisible from the
    // viewscreen because the sphere lifetime was a single frame.)
    virtual_entity_q: Query<
        &crate::entity_spawner::EntityUuid,
        Or<(
            With<crate::entity_spawner::AsteroidFieldSection>,
            With<crate::entity_spawner::RegionShapeSection>,
        )>,
    >,
    mut snapshot: ResMut<TorpedoTargetSnapshot>,
) {
    snapshot.clear();

    // ── Build shared world snapshots up-front (used by every ship's tick) ───

    // UUIDs of virtual (non-hittable) entities — anchors / regions. Used to
    // exclude them from the detonation target list below.
    // Borrowed, not cloned: these sets are read-only lookups that die at the end
    // of the system, and both sources outlive them.
    let virtual_uuids: std::collections::HashSet<&str> =
        virtual_entity_q.iter().map(|u| u.0.as_str()).collect();
    // World snapshot also carries virtual entities — recognise them by the
    // shape field (`Some("torus" | "sphere" | "box")` marks a region or
    // asteroid-field anchor). The live ECS filter above is the source of
    // truth when the entity is present; this catches snapshot-only entries.
    let virtual_snapshot_uuids: std::collections::HashSet<&str> = world
        .0
        .entities
        .iter()
        .filter(|e| e.shape.is_some())
        .map(|e| e.uuid.as_str())
        .collect();

    // Build target positions from *live* ECS transforms, falling back to the
    // (stale) WorldResource snapshot for entities not currently in the ECS.
    let target_positions: std::collections::HashMap<String, (f32, f32, f32)> = {
        let mut map: std::collections::HashMap<String, (f32, f32, f32)> =
            std::collections::HashMap::new();
        for (u, t) in asteroid_q.iter() {
            map.insert(
                u.0.clone(),
                (t.translation.x, t.translation.y, t.translation.z),
            );
        }
        for (u, t) in entity_q.iter() {
            map.insert(
                u.0.clone(),
                (t.translation.x, t.translation.y, t.translation.z),
            );
        }
        // Fill remaining entries from WorldResource snapshot for completeness.
        for e in world.0.entities.iter() {
            map.entry(e.uuid.clone())
                .or_insert_with(|| (e.x(), e.y(), e.z()));
        }
        map
    };

    // Radius by UUID, indexed once up front. This used to be a
    // `world.0.entities.iter().find(|e| e.uuid == ...)` inside each of the two
    // loops below — a linear string-compare scan of the whole world entity list
    // *per live entity*, i.e. O(n²) in world size. At the 256-entity default
    // world that was ~65k string comparisons every tick, and it grew
    // quadratically with any world the designers made bigger. Same lookups,
    // same results, one pass.
    //
    // First-wins on duplicate UUIDs, because `find` returned the first match
    // and `collect` would keep the last. World UUIDs are meant to be unique, so
    // this should never bite — but the point of the change is to be faster, not
    // to be different.
    let radius_by_uuid: std::collections::HashMap<&str, f32> = {
        let mut map: std::collections::HashMap<&str, f32> =
            std::collections::HashMap::with_capacity(world.0.entities.len());
        for e in world.0.entities.iter() {
            map.entry(e.uuid.as_str())
                .or_insert_with(|| e.radius_or_zero());
        }
        map
    };

    // Proximity detonation target list (uuid, x, y, z, radius). Built once and
    // shared across every ship's `find_detonation_hits` call. Y threaded for 3D
    // collision (issue #768).
    let targets: Vec<(String, f32, f32, f32, f32)> = {
        let mut map: std::collections::HashMap<String, (f32, f32, f32, f32)> =
            std::collections::HashMap::new();
        for (u, t) in asteroid_q.iter() {
            let radius = radius_by_uuid.get(u.0.as_str()).copied().unwrap_or(0.0);
            map.insert(
                u.0.clone(),
                (t.translation.x, t.translation.y, t.translation.z, radius),
            );
        }
        for (u, t) in entity_q.iter() {
            if virtual_uuids.contains(u.0.as_str()) || virtual_snapshot_uuids.contains(u.0.as_str())
            {
                continue;
            }
            let radius = radius_by_uuid.get(u.0.as_str()).copied().unwrap_or(0.0);
            map.insert(
                u.0.clone(),
                (t.translation.x, t.translation.y, t.translation.z, radius),
            );
        }
        for e in world.0.entities.iter() {
            if virtual_uuids.contains(e.uuid.as_str())
                || virtual_snapshot_uuids.contains(e.uuid.as_str())
            {
                continue;
            }
            map.entry(e.uuid.clone())
                .or_insert_with(|| (e.x(), e.y(), e.z(), e.radius_or_zero()));
        }
        // Sorted by UUID before it leaves this system. `HashMap` iteration order
        // is a function of the per-process random seed in `RandomState`, so this
        // list came out in a different order in every process — and it is the
        // proximity target list the detonation phase walks to decide what a
        // torpedo hits. Two `--seed` runs of the same binary therefore diverged
        // as soon as a torpedo flew, which is why the duel scenario was not
        // reproducible while the (torpedo-free) default world was.
        let mut out: Vec<(String, f32, f32, f32, f32)> = map
            .into_iter()
            .map(|(uuid, (x, y, z, r))| (uuid, x, y, z, r))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };

    snapshot.target_positions = target_positions;
    snapshot.targets = targets;
}

/// The two position lookups the detonation phase uses to place an explosion,
/// bundled as one `SystemParam`. Same reason as
/// [`crate::server_app::WorldAndTracked`]: the torpedo lifecycle sits on Bevy's
/// 16-parameter ceiling, and the player-death latch had to fit.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct HitPositionQueries<'w, 's> {
    pub asteroids: Query<
        'w,
        's,
        (&'static AsteroidUuid, &'static Transform),
        Without<crate::entity_spawner::EntityUuid>,
    >,
    pub entities: Query<
        'w,
        's,
        (
            &'static crate::entity_spawner::EntityUuid,
            &'static Transform,
        ),
        Without<AsteroidUuid>,
    >,
}

/// Phase 2 of the torpedo tick (issue #724): per-ship torpedo tick —
/// guidance/expiry via the [`TorpedoTargetSnapshot`] built earlier this
/// tick, proximity detonation, shield routing, hull damage, despawn,
/// broadcasts and VFX events.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tick_torpedo_lifecycle(
    mut torpedo_sys_q: Query<&mut TorpedoSystemResource, With<crate::server_app::Ship>>,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
    // `world` + the reported registry bundled into one param to stay under
    // Bevy's 16-parameter ceiling (issue #838). `world_tracked.world` is the
    // former `world: ResMut<WorldResource>`.
    mut world_tracked: crate::server_app::WorldAndTracked,
    time: Res<Time>,
    mut outbox: ResMut<SimOutbox>,
    mut hull_query: Query<(
        Entity,
        Option<&AsteroidUuid>,
        Option<&crate::entity_spawner::EntityUuid>,
        &mut EntitySystemHull,
        Option<&mut crate::ship::shields::ShipShields>,
        Option<&mut crate::entity_spawner::EntityShipArcHull>,
        Option<&crate::entity_spawner::ColliderSection>,
        bevy::ecs::query::Has<crate::server_app::LocalShip>,
        // Where the victim is and which way it is pointing — shield arcs are
        // authored in the victim's own frame, so routing the hit to an arc
        // needs both. `Option` because asteroids and bare-`App` test fixtures
        // carry neither.
        Option<&Transform>,
        Option<&ShipPhysics>,
    )>,
    mut death_latch: crate::server_app::PlayerDeathLatch,
    mut commands: Commands,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    mut ship_vfx_events: MessageWriter<ShipDestroyedVfx>,
    // The two position lookups bundled as one `SystemParam` (issue #838) —
    // separately they put this system over Bevy's 16-parameter ceiling. Same
    // queries main spelled inline; `WeaponsTarget` is now
    // `TacticalRadarSelection` (issue #822).
    hit_pos: HitPositionQueries,
    mut weapons_target_q: Query<&mut TacticalRadarSelection, With<crate::server_app::LocalShip>>,
    snapshot: Res<TorpedoTargetSnapshot>,
    // `Option<ResMut<Messages<_>>>` so bare-`App` fixtures that never
    // registered the message still pass Bevy's parameter validation.
    mut balance_events: Option<ResMut<Messages<crate::balance::BalanceEvent>>>,
    // Seeded RNG + log filter, bundled: separately they put this system one
    // over Bevy's 16-parameter ceiling.
    ambient: crate::server_app::SimRngAndLog,
) {
    let sim_rng = &ambient.rng;
    let id_mint = ambient.id_mint.as_deref();
    let log = &ambient.log;
    let dt = time.delta_secs();
    // Alias so the `world.0.entities` read sites below read naturally.
    let world = &mut world_tracked.world;
    let mut weapons_target_opt = weapons_target_q.single_mut().ok();
    let target_positions = &snapshot.target_positions;
    let targets = &snapshot.targets;

    // ── Phase 1: tick every ship's TorpedoSystem + collect detonation events ──
    //
    // Iterate all ships (`With<Ship>`) with a `TorpedoSystemResource`
    // component — player + NPC. Each ship ticks its own tubes, expires its
    // own torpedoes, and produces its own detonation-hit list.
    //
    // The Resource fallback runs only when NO Ship entity carries the
    // component; this preserves the legacy Resource-only test paths.
    #[derive(Clone, Debug)]
    struct Detonation {
        target_uuid: String,
        damage_hull: i32,
        damage_shields: i32,
        shield_pierce: f32,
        /// Who launched it — the firing ship is out of scope by phase 2, so
        /// balance attribution has to ride along with the detonation.
        source_uuid: Option<String>,
        /// Which tube fired it, for the same reason — the balance contract
        /// wants a configured weapon id, not a generic kind label.
        tube_id: String,
        /// Where the torpedo was when it detonated. Shield routing is
        /// directional, and the torpedo is gone from `in_flight` by phase 2,
        /// so its impact point has to travel with the detonation the same way
        /// `source_uuid` does.
        impact_x: f32,
        impact_z: f32,
        // NB: the torpedo's vertical impact position rides on the pure-model
        // `TorpedoDetonation.impact_y` (issue #768). It is deliberately NOT
        // re-plumbed here: shield-arc routing is a horizontal-plane bearing
        // (`attacker_bearing_relative` is XZ), and the explosion VFX messages
        // carry only x/z, so this internal struct has no consumer for it.
    }
    let mut detonations: Vec<Detonation> = Vec::new();
    let mut any_ship_component = false;

    for mut torpedo_sys in torpedo_sys_q.iter_mut() {
        any_ship_component = true;
        let result = torpedo_sys.0.tick(dt, target_positions, &mut || {
            crate::world_id::mint_id_with(id_mint, crate::world_id::IdNamespace::Projectile)
        });
        for expired_uuid in result.expired {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: expired_uuid },
            ));
        }
        for (tube, uuid, x, y, z, heading) in result.burst_launched {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoLaunched {
                    uuid,
                    tube,
                    x,
                    y,
                    z,
                    heading,
                },
            ));
        }
        let hits = torpedo_sys.0.find_detonation_hits(targets);
        for (torpedo_uuid, target_uuid) in hits {
            let Some(det) = torpedo_sys.0.handle_collision_full(&torpedo_uuid) else {
                continue;
            };
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: torpedo_uuid },
            ));
            detonations.push(Detonation {
                target_uuid,
                damage_hull: det.damage_hull,
                damage_shields: det.damage_shields,
                shield_pierce: det.shield_pierce,
                source_uuid: det.source_uuid,
                tube_id: det.tube_id,
                impact_x: det.impact_x,
                impact_z: det.impact_z,
            });
        }
    }

    // Resource-only fallback: tests that only insert the global
    // `TorpedoSystemResource` (no Ship entity carrying it) still work.
    if !any_ship_component {
        let result = torpedo_sys_res.0.tick(dt, target_positions, &mut || {
            crate::world_id::mint_id_with(id_mint, crate::world_id::IdNamespace::Projectile)
        });
        for expired_uuid in result.expired {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: expired_uuid },
            ));
        }
        for (tube, uuid, x, y, z, heading) in result.burst_launched {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoLaunched {
                    uuid,
                    tube,
                    x,
                    y,
                    z,
                    heading,
                },
            ));
        }
        let hits = torpedo_sys_res.0.find_detonation_hits(targets);
        for (torpedo_uuid, target_uuid) in hits {
            let Some(det) = torpedo_sys_res.0.handle_collision_full(&torpedo_uuid) else {
                continue;
            };
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: torpedo_uuid },
            ));
            detonations.push(Detonation {
                target_uuid,
                damage_hull: det.damage_hull,
                damage_shields: det.damage_shields,
                shield_pierce: det.shield_pierce,
                source_uuid: det.source_uuid,
                tube_id: det.tube_id,
                impact_x: det.impact_x,
                impact_z: det.impact_z,
            });
        }
    }

    // ── Phase 2: apply detonations to hulls / shields ───────────────────────

    for det in detonations {
        let target_uuid = det.target_uuid;
        let mut asteroid_destroyed = false;
        let mut non_local_ship_destroyed = false;
        let mut local_ship_destroyed = false;
        let mut hit_x = 0.0_f32;
        let mut hit_z = 0.0_f32;
        let mut destroyed_ship_radius = DEFAULT_SHIP_EXPLOSION_RADIUS;

        for (
            entity,
            asteroid_uuid,
            entity_uuid,
            mut hull_comp,
            mut shield_comp,
            mut target_arc_hull,
            collider_opt,
            target_is_local,
            target_tf,
            target_physics,
        ) in hull_query.iter_mut()
        {
            let uuid_matches = asteroid_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str())
                || entity_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str());
            if !uuid_matches {
                continue;
            }
            let is_asteroid = asteroid_uuid.is_some();

            // Route shield-eligible damage through any `ShipShields`
            // component, with overflow leaking to hull. Hull damage
            // (always-pierces) goes straight to hull. Asteroids carry no
            // shield so the shielded path is a no-op for them.
            let mut hull_damage = det.damage_hull as f32;
            let mut shield_absorbed = 0.0f32;
            let shield_eligible = det.damage_shields as f32;
            // Snapshot online facings before the shield apply so the
            // online→offline edge can be reported (issue #841).
            let arcs_online_before: Vec<(String, bool)> = shield_comp
                .as_ref()
                .map(|s| {
                    s.0.facings
                        .iter()
                        .map(|f| (f.id.clone(), f.is_online()))
                        .collect()
                })
                .unwrap_or_default();
            if shield_eligible > 0.0 {
                if let Some(ref mut shields) = shield_comp {
                    let all_offline = shields.0.facings.iter().all(|f| !f.is_online());
                    if all_offline {
                        hull_damage += shield_eligible;
                    } else {
                        let (pierced, absorbed) = crate::damage::split_damage_for_pierce(
                            shield_eligible,
                            det.shield_pierce,
                        );
                        // Route the hit to the arc the torpedo actually flew
                        // into, by handing `apply_damage` the bearing the
                        // torpedo arrived on rather than a hardcoded 0.0.
                        //
                        // # Why this matters
                        //
                        // `ShieldSystem::apply_damage` resolves the facing with
                        // `facing_index_for_bearing`, so a constant 0.0 put
                        // every torpedo — from every direction — on whichever
                        // arc contains bearing 0 (fore, on a four-arc Alliance
                        // hull). `ai_torpedo_auto_fire`'s doctrine gate asks
                        // the same resolver which arc is in the way before it
                        // shoots; with the hit hardcoded to fore, a shot
                        // green-lit against a collapsed AFT arc was absorbed by
                        // the healthy FORE arc. Gate and hit now share one
                        // resolver, so they agree.
                        //
                        // Attacker position is the *torpedo's* impact point,
                        // not the firing ship's: the gate predicts from the
                        // launcher, but a homing torpedo curves, and the arc it
                        // meets is the one it is nose-on to when it goes off.
                        //
                        // Bearing falls back to 0.0 for a victim with no
                        // `Transform` (the degenerate case the beam path also
                        // takes) — nothing better is knowable there.
                        let bearing = match target_tf {
                            Some(tf) => crate::shield::attacker_bearing_relative(
                                det.impact_x,
                                det.impact_z,
                                tf.translation.x,
                                tf.translation.z,
                                target_physics.map(|p| p.yaw).unwrap_or(0.0),
                            ),
                            None => 0.0,
                        };
                        let leak = shields.0.apply_damage(absorbed.round() as i32, bearing);
                        shield_absorbed = (absorbed - leak as f32).max(0.0);
                        hull_damage += pierced + leak as f32;
                    }
                } else {
                    hull_damage += shield_eligible;
                }
            }
            let mut hull_applied = 0.0f32;
            if hull_damage > 0.0 {
                let before = hull_comp.0.total_current();
                hull_applied = crate::sim_rng::with_stream(
                    sim_rng.as_deref(),
                    crate::sim_rng::SimStream::TorpedoDamage,
                    |rng| {
                        hull_comp.0.apply_damage(hull_damage, rng);
                        let absorbed = before - hull_comp.0.total_current();
                        // Distribute the same absorbed amount across per-arc
                        // hull (issue #514).
                        if let Some(ref mut arc_hull) = target_arc_hull {
                            arc_hull.0.apply_damage(absorbed, rng);
                        }
                        absorbed
                    },
                );
            }

            // Human-readable logging alongside the structured BalanceEvent
            // (does NOT replace it). Same level discipline as the beam site:
            // per-hit detail is `trace`, and the one `info` edge is
            // destruction — the state change a balancer reads as a headline.
            // Both entity-scoped to the victim so `--log-entity` narrows to
            // one hull.
            let attacker_label: &str = det.source_uuid.as_deref().unwrap_or("unknown");
            crate::ptrace!(
                log,
                crate::logging::LogCat::Damage,
                entity = entity,
                "took {} (shield {:.0}/hull {:.0}) from {} via {}",
                det.damage_hull + det.damage_shields,
                shield_absorbed,
                hull_applied,
                attacker_label,
                det.tube_id
            );
            if hull_comp.0.is_destroyed() && !is_asteroid {
                crate::pinfo!(
                    log,
                    crate::logging::LogCat::Damage,
                    entity = entity,
                    "destroyed by {}",
                    attacker_label
                );
            }

            // Balance tracer — every torpedo hit, on every ship, regardless of
            // whether anything downstream is player-facing.
            if let Some(ref mut msgs) = balance_events {
                msgs.write(crate::balance::BalanceEvent::DamageApplied {
                    attacker: det.source_uuid.clone(),
                    victim: target_uuid.clone(),
                    victim_kind: if is_asteroid {
                        crate::balance::VictimKind::Asteroid
                    } else {
                        crate::balance::VictimKind::Ship
                    },
                    weapon: det.tube_id.clone(),
                    amount: (det.damage_hull + det.damage_shields) as f32,
                    shield_absorbed,
                    hull_damage: hull_applied,
                    system_hit: None,
                });
                if !is_asteroid {
                    if let Some(ref shields) = shield_comp {
                        for (id, was_online) in &arcs_online_before {
                            if !was_online {
                                continue;
                            }
                            let now_offline = shields
                                .0
                                .facings
                                .iter()
                                .find(|f| &f.id == id)
                                .map(|f| !f.is_online())
                                .unwrap_or(false);
                            if now_offline {
                                msgs.write(crate::balance::BalanceEvent::ShieldArcCollapsed {
                                    ship: target_uuid.clone(),
                                    arc_id: id.clone(),
                                });
                            }
                        }
                    }
                }
            }

            if hull_comp.0.is_destroyed() {
                // The player's ship is never despawned on death — the run ends
                // instead, and the report still needs the wreck to read from.
                // Same rule the beam and blaster kill sites follow.
                if !target_is_local {
                    commands.entity(entity).try_despawn();
                }
                if is_asteroid {
                    asteroid_destroyed = true;
                } else if target_is_local {
                    local_ship_destroyed = true;
                } else {
                    non_local_ship_destroyed = true;
                    destroyed_ship_radius = collider_opt
                        .map(|c| c.0.radius)
                        .unwrap_or(DEFAULT_SHIP_EXPLOSION_RADIUS);
                }
                // Use live position from whichever query matches (asteroid or ship).
                if is_asteroid {
                    if let Some((_, t)) = hit_pos.asteroids.iter().find(|(u, _)| u.0 == target_uuid)
                    {
                        hit_x = t.translation.x;
                        hit_z = t.translation.z;
                    }
                } else if let Some((_, t)) =
                    hit_pos.entities.iter().find(|(u, _)| u.0 == target_uuid)
                {
                    hit_x = t.translation.x;
                    hit_z = t.translation.z;
                }
            }
        }

        if local_ship_destroyed {
            // A torpedo can now deliver the killing blow to the player: AI
            // crews only started firing them once the doctrine gate stopped
            // demanding every shield arc be down at once. Until then this
            // branch was unreachable, and the player simply despawned with the
            // run still `InProgress` — the death was recorded in the ledger but
            // nothing ever latched game-over. Mirrors the beam kill site
            // (`tick_beams_apply_damage`) exactly, including the shared
            // first-write `GameOverReason` latch that the `EntityDestroyed`
            // tracer piggybacks on.
            outbox.0.push((Target::All, ServerMessage::ShipDestroyed));
            if let Some(ref mut gs) = death_latch.next_state {
                gs.set(crate::messages::GamePhase::GameOver);
            }
            if let Some(ref mut reason) = death_latch.reason {
                if reason.0.is_none() {
                    reason.0 = Some("server.game_over.ship_destroyed".into());
                    // The LocalShip died → defeat (#843).
                    reason.1 = Some(crate::balance::Outcome::Defeat);
                    if let Some(ref mut msgs) = balance_events {
                        msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                            victim: target_uuid.clone(),
                            killer: det.source_uuid.clone(),
                        });
                    }
                }
            }
        } else if asteroid_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            vfx_events.write(AsteroidDestroyedVfx { x: hit_x, z: hit_z });
            outbox.0.push((
                Target::All,
                ServerMessage::AsteroidDestroyed {
                    uuid: target_uuid.clone(),
                },
            ));
            if weapons_target_opt.as_deref().and_then(|wt| wt.0.as_deref())
                == Some(target_uuid.as_str())
            {
                if let Some(ref mut wt) = weapons_target_opt {
                    wt.0 = None;
                }
            }
        } else if non_local_ship_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                entity_uuid: target_uuid.clone(),
            });
            ship_vfx_events.write(ShipDestroyedVfx {
                x: hit_x,
                z: hit_z,
                radius: destroyed_ship_radius,
            });
            outbox.0.push((
                Target::All,
                ServerMessage::EntityDespawned {
                    uuid: target_uuid.clone(),
                },
            ));
            if let Some(t) = world_tracked.tracked.as_mut() {
                t.forget(&target_uuid);
            }
            // EntityDestroyed for the torpedo kill, co-located with the
            // AiEntityDestroyed write (exactly once). Killer = launching ship.
            if let Some(ref mut msgs) = balance_events {
                msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                    victim: target_uuid.clone(),
                    killer: det.source_uuid.clone(),
                });
            }
            if weapons_target_opt.as_deref().and_then(|wt| wt.0.as_deref())
                == Some(target_uuid.as_str())
            {
                if let Some(ref mut wt) = weapons_target_opt {
                    wt.0 = None;
                }
            }
        }
    }
}

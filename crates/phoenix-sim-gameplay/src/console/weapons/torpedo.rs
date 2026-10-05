use crate::weapons::torpedo::*;
use bevy::prelude::*;

/// Wraps the pure-Rust torpedo system so it can be used as a Bevy resource.
///
/// Derives both `Resource` (existing player-ship singleton path) and
/// `Component` (per-entity path, PR 5 unification).
#[derive(Resource, Component, Clone)]
pub struct TorpedoSystemResource(pub TorpedoSystem);

/// Per-ship map of each torpedo tube's inline stateless load + launch policy
/// (issue #782), keyed by `TorpedoTubeConfig.id`. The torpedo twin of
/// [`crate::console::weapons::BlasterBankAiPolicies`]: built at spawn from each
/// tube's authored `ai` block, falling back to the canonical
/// [`crate::entities::config::default_torpedo_tube_ai_config`] (unconditional
/// load + launch) so a tube without an authored policy keeps behaving exactly as
/// before (AC1). Read by `ai_torpedo_load` (the `torpedo_load` channel) and
/// `ai_torpedo_auto_fire` (the `torpedo_launch` channel).
#[derive(Component, Default, Clone, Debug)]
pub struct TorpedoTubeAiPolicies(
    pub  std::collections::HashMap<
        crate::entities::config::TorpedoTubeId,
        crate::ai::policy::AiPolicy,
    >,
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
/// [`crate::console::weapons::seed_blaster_bank_facts`]. Closes the #779
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
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::LOADED_COUNT, loaded_count as f64);
    facts.set_fact(fid::TARGET_COUNT, target_count as f64);
    facts.set_fact(fid::AI_TARGET_COUNT, ai_target_count as f64);
    facts.set_fact(fid::MAGAZINE, magazine as f64);
    facts.set_fact(fid::OPERATES_AI, if operates_ai { 1.0 } else { 0.0 });
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
/// `posture` is the firing reading, added by issue #872 and widened by issue
/// #1396 — this ship's own [`crate::ship::state::ShipRedAlert`] and whether THIS
/// tube's authored power group is cold, folded into the one `red_alert` fact by
/// [`crate::console::weapons::WeaponsAlertPosture`]. Seeded on the LAUNCH
/// snapshot only: loading a tube and granting a round from the magazine are not
/// offensive fire and stay ungated — a restrained ship may fill its tubes, it
/// simply will not shoot them.
#[allow(clippy::too_many_arguments)]
pub fn seed_torpedo_tube_launch_facts(
    loaded: bool,
    target_valid: bool,
    in_range: bool,
    in_arc: bool,
    target_facing_shields: i32,
    tubes_full: bool,
    posture: crate::console::weapons::WeaponsAlertPosture,
) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::LOADED, if loaded { 1.0 } else { 0.0 });
    facts.set_fact(fid::TARGET_VALID, if target_valid { 1.0 } else { 0.0 });
    facts.set_fact(fid::IN_RANGE, if in_range { 1.0 } else { 0.0 });
    facts.set_fact(fid::IN_ARC, if in_arc { 1.0 } else { 0.0 });
    facts.set_fact(fid::TARGET_FACING_SHIELDS, target_facing_shields as f64);
    facts.set_fact(fid::TUBES_FULL, if tubes_full { 1.0 } else { 0.0 });
    facts.set_fact(fid::RED_ALERT, posture.alert_fact_value());
    facts
}

/// Seed the per-tick policy fact snapshot for the shared magazine's GRANT
/// decision (issue #782). `magazine` is the live `torpedoes_remaining`;
/// `in_flight` is the count of this ship's torpedoes currently in flight — the
/// AC5 public fact the magazine policy can gate on.
pub fn seed_torpedo_magazine_facts(magazine: u32, in_flight: u32) -> crate::world::flags::AiFacts {
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(fid::MAGAZINE, magazine as f64);
    facts.set_fact(fid::IN_FLIGHT, in_flight as f64);
    facts
}

/// Seed the policy fact snapshot for the shared magazine's CONSERVATION
/// decision (issue #943) — the world-scoped half of the torpedo doctrine,
/// resolved once per ship per tick, ahead of that ship's admitted command loop
/// in [`handle_fire_torpedo`], for human-origin and AI-origin launches alike.
///
/// `rounds_aboard` is [`crate::weapons::torpedo::TorpedoSystem::rounds_aboard`] — the
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
        .filter(|doctrine| {
            matches!(
                crate::ai::core::parse_doctrine_directive(doctrine),
                Ok(crate::core::messages::AiDirective::Destroy { target })
                    if !target.trim().is_empty()
            )
        })
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

//! Which fine-system AI hosts can actually evaluate `flag(...)` and
//! `counter(...)` guards — and the load-time rejection for any that cannot
//! (issue #891). Issue #890 adds a second question of exactly the same shape:
//! which hosts fold a bounded history window, and therefore where a
//! `history(...)` guard can ever read anything but absent.
//!
//! # The trap this closed
//!
//! `flag(name)` and `counter(name) CMP n` are full citizens of the shared
//! `world::flags` predicate grammar, and every fine-system policy/selector API
//! (`AiPolicy::resolve_channel`, `resolve_channel_ranked`,
//! `resolve_channel_in_state`, `resolve_transition`,
//! `TargetSelector::select`) takes a
//! `flags: &[&FlagStore]` chain. But a chain is only as real as what the HOST
//! passes — and until #891 stage 2, sixteen of the nineteen hosts passed a
//! literal `&[]`, on which a `flag(...)` guard parsed, validated, and then
//! read `false` for ever: the same silent-nothing failure mode as an unseeded
//! `fact(...)` name, except here the grammar advertised the feature as
//! available.
//!
//! Stage 1 converted that silent trap into a load error. Stage 2 threaded the
//! real chain into every host — each builds it per ship through
//! [`crate::world::server::entity_flag_chain`], anchored at the layer that
//! spawned the ship and climbing `loader_path` to the base
//! `WorldContentRuntime` store, the SAME layered walk world triggers read
//! through — and lifted the rejection: every host below is
//! [`FlagChain::Plumbed`] today. The rejection machinery stays, so a future
//! host added with an empty chain (declared [`FlagChain::Empty`]) rejects
//! authored world-state guards at load instead of reviving the trap.
//!
//! # Migration note: three hosts changed which store an unprefixed guard reads
//!
//! Three of the nineteen hosts — Power reactor, Comms dialogue response, and
//! Comms hail selector — were already reading world flags BEFORE stage 2, but
//! through a flat `vec![&rt.flags]`: base-store-only, no layering. Stage 2
//! moved all three onto the same entity-anchored [`crate::world::server::
//! entity_flag_chain`] every other host uses. For a ship spawned by a loaded
//! layer, this is a real behaviour change, not just a plumbing detail: an
//! UNPREFIXED `flag(...)`/`counter(...)` guard on one of these three hosts
//! used to read the BASE store (the only store the old flat vec ever held)
//! and now reads the LAYER store first (`resolve_chain` indexes by depth, and
//! chain[0] is the spawning layer). Content authored against the old
//! base-only reading needs a `parent:` prefix to keep reaching the base store
//! from a layer-spawned ship; content that already used `parent:` for these
//! three hosts was reading nothing before (no outer entry existed) and reads
//! the base store correctly now. See `console_ai::server::tests::
//! scenario_flag_chain_is_anchored_at_the_ships_spawning_layer` for the
//! layering behaviour driven through a real host end to end.
//!
//! # The flag-chain classification, after the host spine (issue #1212)
//!
//! `flag_chain` records whether a host's runtime evaluation receives a real
//! world-flag chain. Until the [`crate::ai::host`] spine (issue #1208) every
//! host called `resolve_channel` / `select` itself and passed its own chain
//! argument, so this classification could drift from what a host's source
//! actually passed — and drift SILENTLY, back to the failure mode being fixed.
//! Two source-scanning drift tests stood in for the missing seam: one
//! RE-DERIVED each `flag_chain` by reading the resolve/select call out of the
//! crate's own source, and one walked every call site in the crate to catch a
//! host that claimed none.
//!
//! The spine is that seam now. Resolution happens once, inside
//! [`crate::ai::host::decide`], and the chain is built through
//! [`crate::ai::host::AiHostEnv::flag_chain`], which holds bare `Res` and so
//! cannot degrade to the empty chain the drift lint policed. Those two tests —
//! and the `eval_sites` table they re-derived against — retired with #1212;
//! `flag_chain` is now an ordinary data property, exercised through the real
//! validators by `tests::every_host_reads_world_flags_today` and the
//! load-rejection tests below.

use crate::world::flags::{FactContext, FactId, Predicate};

/// Whether a host's runtime evaluation call receives a populated world-flag
/// chain, and therefore whether `flag(...)`/`counter(...)` in one of its guards
/// can ever read true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagChain {
    /// The host builds its chain per ship through
    /// `crate::world::server::entity_flag_chain` (anchored at the spawning
    /// layer, terminating at `WorldContentRuntime.flags`).
    Plumbed,
    /// The host passes a literal `&[]`. No shipped host does since #891
    /// stage 2; the variant stays so a future host that cannot evaluate world
    /// state rejects authored `flag()`/`counter()` guards at load rather than
    /// reading them false for ever.
    Empty,
}

/// One named function site, pinned against the crate's own source by a drift
/// test that reads that function out and inspects it: a host's
/// [`history_fold`](AiHost::history_fold) site (issue #890), and the
/// declaration attachment sites in
/// [`crate::entities::ai_declaration_manifest`] (issue #885).
///
/// `file` is crate-root-relative with forward slashes; `func` is the name as it
/// appears after `fn` in the definition, so the drift test can find it without
/// parsing Rust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalSite {
    pub file: &'static str,
    pub func: &'static str,
}

const fn site(file: &'static str, func: &'static str) -> EvalSite {
    EvalSite { file, func }
}

/// A fine-system AI policy or target-selector host: the authored TOML block, the
/// flag chain its runtime evaluation gets, and where that is decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiHost {
    /// Human name of the owning system, as it should read in a load error.
    pub system: &'static str,
    /// The authored block this host validates, quoted in the load error so the
    /// author knows which table to edit.
    pub block: &'static str,
    /// Whether the host's runtime evaluation receives a real world-flag chain,
    /// and therefore whether `flag(...)`/`counter(...)` in its guards can ever
    /// read true. Every shipped host is [`FlagChain::Plumbed`]; the
    /// load-rejection in [`check_world_state`](Self::check_world_state) keeps a
    /// future [`FlagChain::Empty`] host from reviving the silent-nothing trap.
    pub flag_chain: FlagChain,
    /// Where this host advances its bounded history windows, or `None` when
    /// nothing does (issue #890).
    ///
    /// `Some(site)` is a promise with two halves, both pinned by tests: that
    /// function calls [`crate::world::flags::AiPolicyMemory::fold_history`], and
    /// it is the ONLY function in the crate that calls it at all. Without the
    /// second half a window could
    /// quietly acquire a second fold site — the per-axis actuator systems all
    /// resolve guards off the same ship in the same tick — and every authored
    /// span would mean a fraction of what the file says.
    ///
    /// `None` makes a `history(...)` guard on this host a LOAD ERROR rather
    /// than a permanently-absent reading. Widening the set is how a future host
    /// gains the operator: add the fold, name it here, and the rejection stops
    /// firing for that host alone.
    pub history_fold: Option<EvalSite>,
    /// Every typed fact this host SEEDS, so an authored `fact(...)` /
    /// `candidate_fact(...)` / `target_fact(...)` naming anything else is a
    /// load error rather than a silent permanent `false` (issue #1210). This is
    /// the registry [`check_facts`](AiHost::check_facts) validates against, and
    /// [`tests::the_fact_catalogue_and_the_host_registry_agree`] pins it against
    /// the [`FACT_CATALOGUE`].
    pub facts: &'static [FactDescriptor],
}

// ── The twenty hosts ─────────────────────────────────────────────────────────
//
// Fifteen policy hosts, then five selector hosts, grouped by the console each
// belongs to. (The fifteenth policy host is [`WEAPONS_DOCTRINE`], added by issue
// #956 when the weapon-family arc order stopped being a Rust array.)
//
// That grouping is the whole of the claim. This file deliberately does NOT
// promise to be in the same order as the validation blocks in
// `EntityConfig::from_toml` — nothing enforces such a claim, and it was already
// false when it was written: `COMMS_RESPONSE` is listed with the policy hosts
// but validated last of all, after the comms SELECTOR. The tests below iterate
// this slice as a SET (`BTreeSet` of blocks), so ordering here is a reading aid
// and never a correctness property.

/// The three helm axes that may author a #882 state machine are the only hosts
/// that fold a bounded history window today (issue #890), and they all fold it
/// in the one place their machines are advanced: `tick_policy_machine`, called
/// once per fine system per shared AI tick from `ai_policy_state_tick`.
///
/// Their per-state RULE guards are resolved later in the same tick by the
/// per-axis actuator systems, off the same per-system bag — so a window is
/// readable in both authorable positions on these three, and folded in neither
/// of them.
const HELM_HISTORY_FOLD: Option<EvalSite> =
    Some(site("src/ship/helm_ai/mod.rs", "tick_policy_machine"));

pub const CAPTAIN_RED_ALERT: AiHost = AiHost {
    system: "Captain",
    block: "[captain_console.ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: CAPTAIN_FACTS,
};

pub const HELM_ENGINES: AiHost = AiHost {
    system: "Helm engines",
    block: "[helm_console.engines_ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: HELM_HISTORY_FOLD,
    facts: HELM_FACTS,
};

pub const HELM_STEERING: AiHost = AiHost {
    system: "Helm steering",
    block: "[helm_console.steering_ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: HELM_HISTORY_FOLD,
    facts: HELM_FACTS,
};

pub const HELM_LATERAL: AiHost = AiHost {
    system: "Helm lateral thrust",
    block: "[helm_console.lateral_ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: HELM_FACTS,
};

pub const HELM_VERTICAL: AiHost = AiHost {
    system: "Helm vertical thrust",
    block: "[helm_console.vertical_ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: HELM_FACTS,
};

pub const HELM_IMPULSE: AiHost = AiHost {
    system: "Helm impulse",
    block: "[helm_console.impulse_ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: HELM_FACTS,
};

pub const HELM_BOOST: AiHost = AiHost {
    system: "Helm boost",
    block: "[helm_console.boost_ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: HELM_HISTORY_FOLD,
    facts: HELM_FACTS,
};

pub const PHASER_BANK: AiHost = AiHost {
    system: "Phaser bank",
    block: "[[weapons_console.phaser_banks]].ai",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: PHASER_FACTS,
};

pub const BLASTER_BANK: AiHost = AiHost {
    system: "Blaster bank",
    block: "[[weapons_console.blaster_banks]].ai",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: BLASTER_FACTS,
};

pub const TORPEDO_TUBE: AiHost = AiHost {
    system: "Torpedo tube",
    block: "[[torpedoes.tubes]].ai",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: TORPEDO_TUBE_FACTS,
};

/// The ship-level weapons doctrine (issue #956): which family the ship turns to
/// bring to bear. Its three arc-bearing rank channels are all resolved through
/// one shared helper, so that helper is the single site fixing the chain.
pub const WEAPONS_DOCTRINE: AiHost = AiHost {
    system: "Weapons doctrine",
    block: "[weapons_console.ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: WEAPONS_DOCTRINE_FACTS,
};

pub const TORPEDO_MAGAZINE: AiHost = AiHost {
    system: "Torpedo magazine",
    block: "[torpedoes].ai",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: TORPEDO_MAGAZINE_FACTS,
};

pub const SHIELDS_FOCUS: AiHost = AiHost {
    system: "Shields focus",
    block: "[shields_console.ai_policy]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: SHIELDS_FACTS,
};

pub const POWER_ALLOCATION: AiHost = AiHost {
    system: "Power reactor",
    block: "[power.ai_policy]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: POWER_FACTS,
};

pub const COMMS_RESPONSE: AiHost = AiHost {
    system: "Comms dialogue response",
    block: "[comms_console.ai]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: COMMS_RESPONSE_FACTS,
};

pub const SENSORS_SELECTOR: AiHost = AiHost {
    system: "Sensors target selector",
    block: "[sensors_console.selector]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: SENSORS_SELECTOR_FACTS,
};

pub const TACTICAL_SELECTOR: AiHost = AiHost {
    system: "Tactical target selector",
    block: "[weapons_console.selector]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: TACTICAL_SELECTOR_FACTS,
};

pub const NAVIGATION_SELECTOR: AiHost = AiHost {
    system: "Navigation target selector",
    block: "[navigation_console.selector]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: NAVIGATION_SELECTOR_FACTS,
};

pub const REPAIR_SELECTOR: AiHost = AiHost {
    system: "Repair target selector",
    block: "[repair.selector]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: REPAIR_SELECTOR_FACTS,
};

pub const COMMS_SELECTOR: AiHost = AiHost {
    system: "Comms hail selector",
    block: "[comms_console.selector]",
    flag_chain: FlagChain::Plumbed,
    history_fold: None,
    facts: COMMS_SELECTOR_FACTS,
};

/// Roll call. The tests below iterate this, so a host added above but left out
/// here loses its load-time flag/history/fact validation.
pub const AI_HOSTS: &[AiHost] = &[
    CAPTAIN_RED_ALERT,
    HELM_ENGINES,
    HELM_STEERING,
    HELM_LATERAL,
    HELM_VERTICAL,
    HELM_IMPULSE,
    HELM_BOOST,
    PHASER_BANK,
    BLASTER_BANK,
    TORPEDO_TUBE,
    WEAPONS_DOCTRINE,
    TORPEDO_MAGAZINE,
    SHIELDS_FOCUS,
    POWER_ALLOCATION,
    COMMS_RESPONSE,
    SENSORS_SELECTOR,
    TACTICAL_SELECTOR,
    NAVIGATION_SELECTOR,
    REPAIR_SELECTOR,
    COMMS_SELECTOR,
];

// ── The typed AI fact registry (issue #1210) ─────────────────────────────────
//
// PRD #774 §11 names the hole this closes by its cost: `check_policy_predicate`
// validates `param(...)` and `memory(...)` against a declaration, but a
// `fact(...)` name was checked against NOTHING — facts are seeded host-side per
// system with no registry to check against — so a mistyped fact name parsed,
// validated, and then read false for ever. The registry below is that missing
// declaration: every host records the facts it seeds, and
// [`AiHost::check_facts`] rejects a `fact(...)` / `candidate_fact(...)` /
// `target_fact(...)` naming anything a host does not seed, quoting the host, the
// block, and the nearest registered name.
//
// The vocabulary is a small explicit table ([`FACT_CATALOGUE`]) rather than a
// crate-source scan of the ~143 `AiFacts::set` sites: the seeders reference a
// [`FactId`] catalogue constant through `AiFacts::set_fact`, and
// `tests::the_fact_catalogue_and_the_host_registry_agree` pins the catalogue to
// the per-host descriptors below — a small-table drift check, the same guard
// STYLE the flag-chain/history tables above use, on a table rather than the
// crate's own source.

/// Which fact keyword a descriptor answers: `fact(...)`/`self_fact(...)`
/// ([`Ship`](FactScope::Ship)), `candidate_fact(...)`
/// ([`Candidate`](FactScope::Candidate)) or `target_fact(...)`
/// ([`Target`](FactScope::Target)). Mirrors the three world
/// [`FactContext`] variants a policy or selector guard can read; the two
/// private contexts (`memory`, `state_time`) are validated elsewhere and are
/// never facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactScope {
    /// The operating ship itself — bare `fact(...)` (every policy host) or
    /// `self_fact(...)` (a selector's self context).
    Ship,
    /// A target candidate being scored — `candidate_fact(...)`.
    Candidate,
    /// The currently-retained selection — `target_fact(...)`.
    Target,
}

impl FactScope {
    /// The [`FactContext`] a `referenced_facts` atom carries for this scope.
    fn from_context(ctx: FactContext) -> Option<Self> {
        match ctx {
            FactContext::SelfCtx => Some(FactScope::Ship),
            FactContext::Candidate => Some(FactScope::Candidate),
            FactContext::Target => Some(FactScope::Target),
            // Private contexts are validated against declarations, not seeds.
            FactContext::Memory | FactContext::StateTime => None,
        }
    }

    /// The `fact` keyword an author writes for this scope, for the rejection.
    fn keyword(self) -> &'static str {
        match self {
            FactScope::Ship => "fact",
            FactScope::Candidate => "candidate_fact",
            FactScope::Target => "target_fact",
        }
    }
}

/// Whether a descriptor names one exact fact or an open FAMILY of facts whose
/// suffix is a data-driven id (issue #1210).
///
/// Two families exist: `power_<group>` (the reactor's per-group current level,
/// keyed by [`crate::ship::power::PowerSystem`]'s group ids) and
/// `recent_damage_<facing>` (per shield-arc bounded damage, keyed by the hull's
/// arc ids). A registry constant cannot spell a data-driven suffix, so these
/// match by prefix — a typo INSIDE the family (`power_halm`) is not caught, but
/// the family boundary still is (`powr_helm` is rejected), and the exact facts
/// that share the `recent_damage_` stem (`recent_damage_total`, …) are matched
/// as exacts and take precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactShape {
    Exact,
    Prefix,
}

/// One fact a host seeds: what to validate an authored atom against, plus the
/// documentation of what the reading means (issue #1210).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactDescriptor {
    /// The fact name, as a [`FACT_CATALOGUE`] constant.
    pub name: FactId,
    /// Which fact keyword reads it.
    pub scope: FactScope,
    /// Exact name, or an open data-driven family matched by prefix.
    pub shape: FactShape,
    /// The owning fine-System kind that seeds this reading.
    pub owner: &'static str,
    /// What the reading means.
    pub context: &'static str,
    /// What an ABSENT reading means — the absent-fact contract the seeder keeps.
    pub absent_means: &'static str,
    /// Where it is seeded, so a maintainer can find the site.
    pub seed_site: &'static str,
}

const fn ship(
    name: FactId,
    owner: &'static str,
    context: &'static str,
    absent_means: &'static str,
    seed_site: &'static str,
) -> FactDescriptor {
    FactDescriptor {
        name,
        scope: FactScope::Ship,
        shape: FactShape::Exact,
        owner,
        context,
        absent_means,
        seed_site,
    }
}

const fn ship_family(
    name: FactId,
    owner: &'static str,
    context: &'static str,
    absent_means: &'static str,
    seed_site: &'static str,
) -> FactDescriptor {
    FactDescriptor {
        name,
        scope: FactScope::Ship,
        shape: FactShape::Prefix,
        owner,
        context,
        absent_means,
        seed_site,
    }
}

const fn cand(
    name: FactId,
    owner: &'static str,
    context: &'static str,
    absent_means: &'static str,
    seed_site: &'static str,
) -> FactDescriptor {
    FactDescriptor {
        name,
        scope: FactScope::Candidate,
        shape: FactShape::Exact,
        owner,
        context,
        absent_means,
        seed_site,
    }
}

// ── Fact catalogue: one FactId per seeded fact name ──────────────────────────
//
// The ten names that already have a `*_FACT: &str` const in `entities::config`
// reference it, so the string has ONE definition; the rest are defined here and
// referenced back by their seeder-side consts (e.g. helm's `HAZARD_URGENCY_FACT`
// derives from `HAZARD_URGENCY`). `tests::the_catalogue_matches_the_config_fact_consts`
// pins the borrowed ten against their source.

// Captain
pub const SECS_SINCE_COMBAT: FactId = FactId("secs_since_combat");
pub const HOSTILE_CONTACT: FactId = FactId(crate::entities::config::CAPTAIN_HOSTILE_CONTACT_FACT);
pub const HOSTILE_RANGE: FactId = FactId(crate::entities::config::CAPTAIN_HOSTILE_RANGE_FACT);
// Power reactor
pub const BATTERY_PCT: FactId = FactId(crate::entities::config::POWER_BATTERY_PCT_FACT);
pub const THRUST: FactId = FactId(crate::entities::config::POWER_THRUST_FACT);
pub const RED_ALERT: FactId = FactId(crate::entities::config::POWER_RED_ALERT_FACT);
pub const POWER_GROUP: FactId = FactId("power_");
pub const TOTAL_ALLOCATION: FactId = FactId("total_allocation");
pub const NEAREST_ENEMY_DIST: FactId = FactId("nearest_enemy_dist");
pub const HAS_DESTROY_OBJECTIVE: FactId = FactId("has_destroy_objective");
pub const OFFLINE_SYSTEM_COUNT: FactId = FactId("offline_system_count");
// Shields focus
pub const RECENT_DAMAGE_ARC: FactId = FactId("recent_damage_");
pub const RECENT_DAMAGE_TOTAL: FactId = FactId("recent_damage_total");
pub const RECENT_DAMAGE_FRACTION_MAX: FactId = FactId("recent_damage_fraction_max");
pub const RECENT_DAMAGE_PCT_MAX: FactId = FactId("recent_damage_pct_max");
pub const HEALTH_FRACTION_MIN_RATIO: FactId = FactId("health_fraction_min_ratio");
pub const HEALTH_RATIO_PCT: FactId = FactId("health_ratio_pct");
// Comms dialogue response
pub const RESPONSE_COUNT: FactId = FactId("response_count");
pub const AVAILABLE_RESPONSE_COUNT: FactId = FactId("available_response_count");
pub const IMPORTANT_RESPONSE_COUNT: FactId = FactId("important_response_count");
pub const IS_URGENT: FactId = FactId("is_urgent");
pub const IS_READ: FactId = FactId("is_read");
pub const IS_ORPHANED: FactId = FactId("is_orphaned");
pub const SENDER_IN_RANGE: FactId = FactId("sender_in_range");
pub const COMMS_AVAILABLE: FactId = FactId("comms_available");
pub const POWER_RATING: FactId = FactId("power_rating");
pub const CONTACT_COUNT: FactId = FactId("contact_count");
// Weapon banks / tubes
pub const TARGET_VALID: FactId = FactId("target_valid");
pub const ON_COOLDOWN: FactId = FactId("on_cooldown");
pub const COOLDOWN_REMAINING: FactId = FactId("cooldown_remaining");
pub const IN_RANGE: FactId = FactId("in_range");
pub const IN_ARC: FactId = FactId("in_arc");
pub const FREQUENCY: FactId = FactId("frequency");
pub const LOADED_COUNT: FactId = FactId("loaded_count");
pub const TARGET_COUNT: FactId = FactId("target_count");
pub const AI_TARGET_COUNT: FactId = FactId("ai_target_count");
pub const MAGAZINE: FactId = FactId("magazine");
pub const OPERATES_AI: FactId = FactId("operates_ai");
pub const LOADED: FactId = FactId("loaded");
pub const TARGET_FACING_SHIELDS: FactId =
    FactId(crate::entities::config::TARGET_FACING_SHIELDS_FACT);
pub const TUBES_FULL: FactId = FactId("tubes_full");
pub const IN_FLIGHT: FactId = FactId("in_flight");
pub const ROUNDS_ABOARD: FactId = FactId(crate::entities::config::TORPEDO_ROUNDS_ABOARD_FACT);
pub const MISSION_THREAT_REMAINING: FactId =
    FactId(crate::entities::config::TORPEDO_MISSION_THREAT_FACT);
pub const ROUNDS_PER_THREAT: FactId =
    FactId(crate::entities::config::TORPEDO_ROUNDS_PER_THREAT_FACT);
pub const TARGETED_OBJECTIVE_COUNT: FactId =
    FactId(crate::entities::config::TORPEDO_TARGETED_OBJECTIVE_COUNT_FACT);
// Helm (all six axes share the seeders in `ship::helm_ai::facts`)
pub const HAZARD_URGENCY: FactId = FactId("hazard_urgency");
pub const POSTURE: FactId = FactId("posture");
pub const MOVING_HAZARD_THREAT: FactId = FactId("moving_hazard_threat");
pub const HAZARD_PRESENT: FactId = FactId("hazard_present");
pub const IMPULSE_AVAILABLE: FactId = FactId("impulse_available");
pub const BOOST_AVAILABLE: FactId = FactId("boost_available");
pub const VERTICAL_OFFSET: FactId = FactId("vertical_offset");
pub const HOSTILE_ARC_EXPOSURE: FactId = FactId("hostile_arc_exposure");
pub const HOSTILE_ARC_ESCAPE_DEG: FactId = FactId("hostile_arc_escape_deg");
pub const HOSTILE_ARC_INESCAPABLE: FactId = FactId("hostile_arc_inescapable");
pub const RANGE_TO_TARGET: FactId = FactId("range_to_target");
pub const CLOSING_RATE: FactId = FactId("closing_rate");
pub const BEARING_TO_TARGET: FactId = FactId("bearing_to_target");
pub const SPEED_FRACTION: FactId = FactId("speed_fraction");
pub const TARGET_DIRECT_FIRE_RANGE: FactId = FactId("target_direct_fire_range");
pub const RANGE_ABOVE_MIN_SEEN: FactId = FactId("range_above_min_seen");
pub const SHIELD_FRACTION: FactId = FactId("shield_fraction");
pub const SAFE_RANGE: FactId = FactId("safe_range");
pub const SAFE_DISTANCE_HELD: FactId = FactId("safe_distance_held");
pub const SEPARATION_PROGRESS: FactId = FactId("separation_progress");
pub const INSIDE_THREAT_RANGE: FactId = FactId("inside_threat_range");
pub const TARGET_FACING_SHIELD_DOWN: FactId = FactId("target_facing_shield_down");
pub const TORPEDOES_IN_FLIGHT: FactId = FactId("torpedoes_in_flight");
pub const TUBES_FILLABLE: FactId = FactId("tubes_fillable");
// Candidate facts (selector hosts)
pub const DETECTABLE: FactId = FactId("detectable");
pub const HOSTILE: FactId = FactId("hostile");
pub const SOURCE_COMBAT_LOCK: FactId = FactId("source_combat_lock");
pub const SOURCE_OBJECTIVE: FactId = FactId("source_objective");
pub const SOURCE_RADAR: FactId = FactId("source_radar");
pub const SOURCE_SENSORS_DESIGNATION: FactId = FactId("source_sensors_designation");
pub const SOURCE_OPERATE: FactId = FactId("source_operate");
pub const SOURCE_LAST_ATTACKER: FactId = FactId("source_last_attacker");
pub const SOURCE_RETAINED: FactId = FactId("source_retained");
pub const REACHABLE: FactId = FactId("reachable");
pub const SOURCE_NAV_OBJECTIVE: FactId = FactId("source_nav_objective");
pub const OBJECTIVE_SCORE: FactId = FactId("objective_score");
pub const SOURCE_CHART_CONTACT: FactId = FactId("source_chart_contact");
pub const TIER_ORDINAL: FactId = FactId("tier_ordinal");
pub const DEFICIT: FactId = FactId("deficit");
pub const DAMAGE_FRACTION: FactId = FactId("damage_fraction");
pub const WORST_SYSTEM_DAMAGE_FRACTION: FactId = FactId("worst_system_damage_fraction");
pub const SYSTEM_COUNT: FactId = FactId("system_count");
pub const IS_CORE: FactId = FactId("is_core");
pub const SOURCE_REPAIR_REQUEST: FactId = FactId("source_repair_request");
pub const SOURCE_CORE_BUCKET: FactId = FactId("source_core_bucket");
pub const ASSIGNED: FactId = FactId("assigned");
pub const SOURCE_HAIL_OBJECTIVE: FactId = FactId("source_hail_objective");
pub const SOURCE_COMMS_CONTACT: FactId = FactId("source_comms_contact");
pub const HAS_OPEN_HAIL_THREAD: FactId = FactId("has_open_hail_thread");
pub const HAS_UNREAD_FROM_SENDER: FactId = FactId("has_unread_from_sender");
pub const MANDATORY: FactId = FactId("mandatory");
// Repair self facts
pub const FREE_TEAM_COUNT: FactId = FactId("free_team_count");
pub const TOTAL_HULL_HEALTH_FRACTION: FactId = FactId("total_hull_health_fraction");

/// Every fact name the registry knows. The catalogue half of the drift check:
/// `tests::the_fact_catalogue_and_the_host_registry_agree` requires each entry
/// here to be seeded by some host, and each host descriptor to name an entry
/// here, so the vocabulary and the per-host registry cannot drift apart.
pub const FACT_CATALOGUE: &[FactId] = &[
    SECS_SINCE_COMBAT,
    HOSTILE_CONTACT,
    HOSTILE_RANGE,
    BATTERY_PCT,
    THRUST,
    RED_ALERT,
    POWER_GROUP,
    TOTAL_ALLOCATION,
    NEAREST_ENEMY_DIST,
    HAS_DESTROY_OBJECTIVE,
    OFFLINE_SYSTEM_COUNT,
    RECENT_DAMAGE_ARC,
    RECENT_DAMAGE_TOTAL,
    RECENT_DAMAGE_FRACTION_MAX,
    RECENT_DAMAGE_PCT_MAX,
    HEALTH_FRACTION_MIN_RATIO,
    HEALTH_RATIO_PCT,
    RESPONSE_COUNT,
    AVAILABLE_RESPONSE_COUNT,
    IMPORTANT_RESPONSE_COUNT,
    IS_URGENT,
    IS_READ,
    IS_ORPHANED,
    SENDER_IN_RANGE,
    COMMS_AVAILABLE,
    POWER_RATING,
    CONTACT_COUNT,
    TARGET_VALID,
    ON_COOLDOWN,
    COOLDOWN_REMAINING,
    IN_RANGE,
    IN_ARC,
    FREQUENCY,
    LOADED_COUNT,
    TARGET_COUNT,
    AI_TARGET_COUNT,
    MAGAZINE,
    OPERATES_AI,
    LOADED,
    TARGET_FACING_SHIELDS,
    TUBES_FULL,
    IN_FLIGHT,
    ROUNDS_ABOARD,
    MISSION_THREAT_REMAINING,
    ROUNDS_PER_THREAT,
    TARGETED_OBJECTIVE_COUNT,
    HAZARD_URGENCY,
    POSTURE,
    MOVING_HAZARD_THREAT,
    HAZARD_PRESENT,
    IMPULSE_AVAILABLE,
    BOOST_AVAILABLE,
    VERTICAL_OFFSET,
    HOSTILE_ARC_EXPOSURE,
    HOSTILE_ARC_ESCAPE_DEG,
    HOSTILE_ARC_INESCAPABLE,
    RANGE_TO_TARGET,
    CLOSING_RATE,
    BEARING_TO_TARGET,
    SPEED_FRACTION,
    TARGET_DIRECT_FIRE_RANGE,
    RANGE_ABOVE_MIN_SEEN,
    SHIELD_FRACTION,
    SAFE_RANGE,
    SAFE_DISTANCE_HELD,
    SEPARATION_PROGRESS,
    INSIDE_THREAT_RANGE,
    TARGET_FACING_SHIELD_DOWN,
    TORPEDOES_IN_FLIGHT,
    TUBES_FILLABLE,
    DETECTABLE,
    HOSTILE,
    SOURCE_COMBAT_LOCK,
    SOURCE_OBJECTIVE,
    SOURCE_RADAR,
    SOURCE_SENSORS_DESIGNATION,
    SOURCE_OPERATE,
    SOURCE_LAST_ATTACKER,
    SOURCE_RETAINED,
    REACHABLE,
    SOURCE_NAV_OBJECTIVE,
    OBJECTIVE_SCORE,
    SOURCE_CHART_CONTACT,
    TIER_ORDINAL,
    DEFICIT,
    DAMAGE_FRACTION,
    WORST_SYSTEM_DAMAGE_FRACTION,
    SYSTEM_COUNT,
    IS_CORE,
    SOURCE_REPAIR_REQUEST,
    SOURCE_CORE_BUCKET,
    ASSIGNED,
    SOURCE_HAIL_OBJECTIVE,
    SOURCE_COMMS_CONTACT,
    HAS_OPEN_HAIL_THREAD,
    HAS_UNREAD_FROM_SENDER,
    MANDATORY,
    FREE_TEAM_COUNT,
    TOTAL_HULL_HEALTH_FRACTION,
];

// ── Per-host descriptor arrays ───────────────────────────────────────────────

const CAPTAIN_FACTS: &[FactDescriptor] = &[
    ship(
        SECS_SINCE_COMBAT,
        "captain",
        "seconds since the ship was last in combat",
        "false — never in combat",
        "console::captain::server::operate_captain_ai",
    ),
    ship(
        HOSTILE_CONTACT,
        "captain",
        "1.0 when a hostile contact is known",
        "false — no contact reading",
        "console::captain::server::operate_captain_ai",
    ),
    ship(
        HOSTILE_RANGE,
        "captain",
        "range to the nearest hostile",
        "false — no hostile in range",
        "console::captain::server::operate_captain_ai",
    ),
];

const POWER_FACTS: &[FactDescriptor] = &[
    ship(
        BATTERY_PCT,
        "power",
        "battery charge fraction",
        "false — no battery reading",
        "ship::power::seed_power_facts",
    ),
    ship(
        THRUST,
        "power",
        "commanded thrust magnitude",
        "false — no thrust reading",
        "ship::power::seed_power_facts",
    ),
    ship(
        RED_ALERT,
        "power",
        "1.0 while the ship is at red alert",
        "false — reads as not-alert",
        "ship::power::seed_power_facts",
    ),
    ship_family(
        POWER_GROUP,
        "power",
        "`power_<group>` — a group's current allocation level",
        "false — that group is unknown",
        "ship::power::seed_power_facts (power_<group.id>)",
    ),
    ship(
        TOTAL_ALLOCATION,
        "power",
        "sum of all group allocations",
        "false — no reading",
        "ship::power::seed_power_facts",
    ),
    ship(
        SECS_SINCE_COMBAT,
        "power",
        "seconds since the ship was last in combat",
        "false — never in combat",
        "ship::power::seed_power_facts",
    ),
    ship(
        NEAREST_ENEMY_DIST,
        "power",
        "distance to the nearest known enemy",
        "false — none known",
        "ship::power::seed_power_facts",
    ),
    ship(
        HAS_DESTROY_OBJECTIVE,
        "power",
        "1.0 when the ship holds a Destroy objective",
        "false — none",
        "ship::power::seed_power_facts",
    ),
    ship(
        OFFLINE_SYSTEM_COUNT,
        "power",
        "how many fine systems are offline",
        "false — no reading",
        "ship::power::seed_power_facts",
    ),
];

const SHIELDS_FACTS: &[FactDescriptor] = &[
    ship_family(
        RECENT_DAMAGE_ARC,
        "shields",
        "`recent_damage_<facing>` — bounded recent damage on one arc",
        "false — that arc is unknown",
        "console_ai::core::seed_shield_focus_facts (recent_damage_<arc>)",
    ),
    ship(
        RECENT_DAMAGE_TOTAL,
        "shields",
        "recent damage summed across arcs",
        "false — no reading",
        "console_ai::core::seed_shield_focus_facts",
    ),
    ship(
        RECENT_DAMAGE_FRACTION_MAX,
        "shields",
        "fraction of recent damage on the worst arc",
        "false — no reading",
        "console_ai::core::seed_shield_focus_facts",
    ),
    ship(
        RECENT_DAMAGE_PCT_MAX,
        "shields",
        "that fraction as a percentage",
        "false — no reading",
        "console_ai::core::seed_shield_focus_facts",
    ),
    ship(
        HEALTH_FRACTION_MIN_RATIO,
        "shields",
        "lowest arc health as a ratio of the second-lowest",
        "false — fewer than two arcs",
        "console_ai::core::seed_shield_focus_facts",
    ),
    ship(
        HEALTH_RATIO_PCT,
        "shields",
        "that ratio as a percentage",
        "false — fewer than two arcs",
        "console_ai::core::seed_shield_focus_facts",
    ),
];

const COMMS_RESPONSE_FACTS: &[FactDescriptor] = &[
    ship(
        RESPONSE_COUNT,
        "comms",
        "how many responses the open dialogue node offers",
        "false — no open dialogue",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        AVAILABLE_RESPONSE_COUNT,
        "comms",
        "how many responses are currently selectable",
        "false — none",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        IMPORTANT_RESPONSE_COUNT,
        "comms",
        "how many responses are author-marked important",
        "false — none",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        IS_URGENT,
        "comms",
        "1.0 when the message is flagged urgent",
        "false — not urgent",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        IS_READ,
        "comms",
        "1.0 when the message has been read",
        "false — unread",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        IS_ORPHANED,
        "comms",
        "1.0 when the sender has gone away",
        "false — sender present",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        SENDER_IN_RANGE,
        "comms",
        "1.0 when the sender is in comms range",
        "false — out of range",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        RED_ALERT,
        "comms",
        "1.0 while the ship is at red alert",
        "false — reads as not-alert",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        COMMS_AVAILABLE,
        "comms",
        "1.0 while the Comms fine system is available",
        "false — offline",
        "console::comms::server::seed_comms_response_facts",
    ),
    ship(
        POWER_RATING,
        "comms",
        "authored ship power rating",
        "false — the ship declares none",
        "console::comms::server::seed_comms_response_facts",
    ),
];

const PHASER_FACTS: &[FactDescriptor] = &[
    ship(
        TARGET_VALID,
        "phaser bank",
        "1.0 when the bank has a valid target",
        "false — no target",
        "console::weapons::beam::seed_phaser_bank_facts",
    ),
    ship(
        ON_COOLDOWN,
        "phaser bank",
        "1.0 while the bank is cooling down",
        "false — ready",
        "console::weapons::beam::seed_phaser_bank_facts",
    ),
    ship(
        COOLDOWN_REMAINING,
        "phaser bank",
        "seconds of cooldown left",
        "false — no reading",
        "console::weapons::beam::seed_phaser_bank_facts",
    ),
    ship(
        IN_RANGE,
        "phaser bank",
        "1.0 when the target is in range",
        "false — out of range",
        "console::weapons::beam::seed_phaser_bank_facts",
    ),
    ship(
        IN_ARC,
        "phaser bank",
        "1.0 when the target is in the bank's arc",
        "false — out of arc",
        "console::weapons::beam::seed_phaser_bank_facts",
    ),
    ship(
        FREQUENCY,
        "phaser bank",
        "the bank's tuned frequency",
        "false — no reading",
        "console::weapons::beam::seed_phaser_bank_facts",
    ),
    ship(
        RED_ALERT,
        "phaser bank",
        "the ship's firing posture (red alert / weapons hold)",
        "false — reads as not weapons-free",
        "console::weapons::beam::seed_phaser_bank_facts",
    ),
];

const BLASTER_FACTS: &[FactDescriptor] = &[
    ship(
        TARGET_VALID,
        "blaster bank",
        "1.0 when the bank has a valid target",
        "false — no target",
        "console::weapons::blaster::seed_blaster_bank_facts",
    ),
    ship(
        ON_COOLDOWN,
        "blaster bank",
        "1.0 while the bank is cooling down",
        "false — ready",
        "console::weapons::blaster::seed_blaster_bank_facts",
    ),
    ship(
        COOLDOWN_REMAINING,
        "blaster bank",
        "seconds of cooldown left",
        "false — no reading",
        "console::weapons::blaster::seed_blaster_bank_facts",
    ),
    ship(
        IN_RANGE,
        "blaster bank",
        "1.0 when the target is in range",
        "false — out of range",
        "console::weapons::blaster::seed_blaster_bank_facts",
    ),
    ship(
        IN_ARC,
        "blaster bank",
        "1.0 when the target is in the bank's arc",
        "false — out of arc",
        "console::weapons::blaster::seed_blaster_bank_facts",
    ),
    ship(
        RED_ALERT,
        "blaster bank",
        "the ship's firing posture (red alert / weapons hold)",
        "false — reads as not weapons-free",
        "console::weapons::blaster::seed_blaster_bank_facts",
    ),
];

const TORPEDO_TUBE_FACTS: &[FactDescriptor] = &[
    ship(
        LOADED_COUNT,
        "torpedo tube",
        "rounds loaded in this tube",
        "false — no reading",
        "console::weapons::torpedo::seed_torpedo_tube_load_facts",
    ),
    ship(
        TARGET_COUNT,
        "torpedo tube",
        "candidate target count",
        "false — none",
        "console::weapons::torpedo::seed_torpedo_tube_load_facts",
    ),
    ship(
        AI_TARGET_COUNT,
        "torpedo tube",
        "AI-selected target count",
        "false — none",
        "console::weapons::torpedo::seed_torpedo_tube_load_facts",
    ),
    ship(
        MAGAZINE,
        "torpedo tube",
        "rounds remaining in the shared magazine",
        "false — no reading",
        "console::weapons::torpedo::seed_torpedo_tube_load_facts",
    ),
    ship(
        OPERATES_AI,
        "torpedo tube",
        "1.0 when the tube is AI-operated",
        "false — human/offline",
        "console::weapons::torpedo::seed_torpedo_tube_load_facts",
    ),
    ship(
        LOADED,
        "torpedo tube",
        "1.0 when at least one round is loaded",
        "false — empty",
        "console::weapons::torpedo::seed_torpedo_tube_launch_facts",
    ),
    ship(
        TARGET_VALID,
        "torpedo tube",
        "1.0 when the tube has a valid target",
        "false — no target",
        "console::weapons::torpedo::seed_torpedo_tube_launch_facts",
    ),
    ship(
        IN_RANGE,
        "torpedo tube",
        "1.0 when the target is in range",
        "false — out of range",
        "console::weapons::torpedo::seed_torpedo_tube_launch_facts",
    ),
    ship(
        IN_ARC,
        "torpedo tube",
        "1.0 when the target is in the tube's arc",
        "false — out of arc",
        "console::weapons::torpedo::seed_torpedo_tube_launch_facts",
    ),
    ship(
        TARGET_FACING_SHIELDS,
        "torpedo tube",
        "striking arc's shield HP (<= 0 is not blocking)",
        "false — no target",
        "console::weapons::torpedo::seed_torpedo_tube_launch_facts",
    ),
    ship(
        TUBES_FULL,
        "torpedo tube",
        "1.0 when every tube is at volley_max",
        "false — not a full salvo",
        "console::weapons::torpedo::seed_torpedo_tube_launch_facts",
    ),
    ship(
        RED_ALERT,
        "torpedo tube",
        "the ship's firing posture (red alert / weapons hold)",
        "false — reads as not weapons-free",
        "console::weapons::torpedo::seed_torpedo_tube_launch_facts",
    ),
];

const WEAPONS_DOCTRINE_FACTS: &[FactDescriptor] = &[
    ship(
        TARGET_FACING_SHIELDS,
        "weapons doctrine",
        "striking arc's shield HP (<= 0 is not blocking)",
        "false — no target",
        "console::weapons::seed_weapons_doctrine_facts",
    ),
    ship(
        RED_ALERT,
        "weapons doctrine",
        "the ship's firing posture (red alert / weapons hold)",
        "false — reads as not weapons-free",
        "console::weapons::seed_weapons_doctrine_facts",
    ),
];

const TORPEDO_MAGAZINE_FACTS: &[FactDescriptor] = &[
    ship(
        MAGAZINE,
        "torpedo magazine",
        "rounds remaining in the magazine",
        "false — no reading",
        "console::weapons::torpedo::seed_torpedo_magazine_facts",
    ),
    ship(
        IN_FLIGHT,
        "torpedo magazine",
        "this ship's torpedoes currently in flight",
        "false — none",
        "console::weapons::torpedo::seed_torpedo_magazine_facts",
    ),
    ship(
        ROUNDS_ABOARD,
        "torpedo magazine",
        "rounds in the magazine plus rounds parked in tubes",
        "false — no reading",
        "console::weapons::torpedo::seed_torpedo_conservation_facts",
    ),
    ship(
        MISSION_THREAT_REMAINING,
        "torpedo magazine",
        "the scenario's remaining mission threat count",
        "false — no reading",
        "console::weapons::torpedo::seed_torpedo_conservation_facts",
    ),
    ship(
        ROUNDS_PER_THREAT,
        "torpedo magazine",
        "rounds aboard per remaining unit of threat",
        "false — no reading (INFINITY when threat is spent)",
        "console::weapons::torpedo::seed_torpedo_conservation_facts",
    ),
    ship(
        TARGETED_OBJECTIVE_COUNT,
        "torpedo magazine",
        "how many Destroy objectives name this ship's target",
        "false — none",
        "console::weapons::torpedo::seed_torpedo_conservation_facts",
    ),
];

/// The helm actuator + movement-doctrine facts. All six helm axes share this
/// list: the actuator/hostile-arc facts are seeded on every axis, and the
/// travel/recovery/pressed/torpedo-opportunity facts on the three machine axes
/// (engines, steering, boost) that own a #882 state machine. The transition-vs-
/// rule SCOPE of a fact within an axis is a separate concern (see the notes in
/// `ship::helm_ai::facts`); this registry closes only the typo hole — a helm
/// `fact(...)` naming something no helm seeder produces.
const HELM_FACTS: &[FactDescriptor] = &[
    ship(
        HAZARD_URGENCY,
        "helm",
        "shared hazard-avoidance urgency",
        "false — no hazard reading",
        "ship::helm_ai::facts::seed_helm_actuator_facts",
    ),
    ship(
        POSTURE,
        "helm",
        "which movement doctrine the alert state licenses",
        "false — reads as defensive",
        "ship::helm_ai::facts::seed_helm_actuator_facts",
    ),
    ship(
        MOVING_HAZARD_THREAT,
        "helm",
        "threat from a moving hazard",
        "false — none",
        "ship::helm_ai::facts::seed_helm_actuator_facts",
    ),
    ship(
        HAZARD_PRESENT,
        "helm",
        "1.0 when any hazard is present",
        "false — clear",
        "ship::helm_ai::facts::seed_helm_actuator_facts",
    ),
    ship(
        IMPULSE_AVAILABLE,
        "helm",
        "1.0 when impulse is available",
        "false — unavailable",
        "ship::helm_ai::facts::seed_helm_actuator_facts",
    ),
    ship(
        BOOST_AVAILABLE,
        "helm",
        "1.0 when boost is available",
        "false — unavailable",
        "ship::helm_ai::facts::seed_helm_actuator_facts",
    ),
    ship(
        VERTICAL_OFFSET,
        "helm",
        "current vertical offset from the cruise plane",
        "false — no reading",
        "ship::helm_ai::facts::seed_helm_actuator_facts",
    ),
    ship(
        HOSTILE_ARC_EXPOSURE,
        "helm",
        "how many hostile weapon arcs bear on this ship",
        "false — none (0.0 when clear)",
        "ship::helm_ai::facts::seed_hostile_arc_facts",
    ),
    ship(
        HOSTILE_ARC_ESCAPE_DEG,
        "helm",
        "bearing change that clears the nearest bearing arc",
        "false — nothing bears",
        "ship::helm_ai::facts::seed_hostile_arc_facts",
    ),
    ship(
        HOSTILE_ARC_INESCAPABLE,
        "helm",
        "1.0 when a bearing arc spans a full turn",
        "false — escapable",
        "ship::helm_ai::facts::seed_hostile_arc_facts",
    ),
    ship(
        TARGET_VALID,
        "helm",
        "1.0 when the helm can see its target",
        "false — no target",
        "ship::helm_ai::facts::seed_helm_travel_facts",
    ),
    ship(
        SPEED_FRACTION,
        "helm",
        "forward speed as a fraction of max",
        "false — no reading",
        "ship::helm_ai::facts::seed_helm_travel_facts",
    ),
    ship(
        RANGE_TO_TARGET,
        "helm",
        "planar distance to the target",
        "false — no target",
        "ship::helm_ai::facts::seed_helm_travel_facts",
    ),
    ship(
        CLOSING_RATE,
        "helm",
        "rate the range is shrinking (sign flip is closest approach)",
        "false — no target",
        "ship::helm_ai::facts::seed_helm_travel_facts",
    ),
    ship(
        BEARING_TO_TARGET,
        "helm",
        "signed bearing to the target, radians",
        "false — no target",
        "ship::helm_ai::facts::seed_helm_travel_facts",
    ),
    ship(
        TARGET_DIRECT_FIRE_RANGE,
        "helm",
        "the target's longest usable direct-fire range",
        "false — no target",
        "ship::helm_ai::facts::seed_helm_travel_facts",
    ),
    ship(
        RANGE_ABOVE_MIN_SEEN,
        "helm",
        "how far the range has re-opened above the state minimum",
        "false — no target",
        "ship::helm_ai::facts::seed_range_above_min_seen_fact",
    ),
    ship(
        SHIELD_FRACTION,
        "helm",
        "this ship's own shield health fraction",
        "false — no shield system",
        "ship::helm_ai::facts::seed_recovery_facts",
    ),
    ship(
        SAFE_RANGE,
        "helm",
        "derived safe-ring radius (target reach + margin)",
        "false — no margin authored",
        "ship::helm_ai::facts::seed_recovery_facts",
    ),
    ship(
        SAFE_DISTANCE_HELD,
        "helm",
        "1.0 when the safe ring held across the history window",
        "false — window not full / breached",
        "ship::helm_ai::facts::seed_recovery_facts",
    ),
    ship(
        SEPARATION_PROGRESS,
        "helm",
        "net separation change across the pressed window",
        "false — window not full",
        "ship::helm_ai::facts::seed_pressed_facts",
    ),
    ship(
        INSIDE_THREAT_RANGE,
        "helm",
        "1.0 when inside the target's effective threat range",
        "false — outside / no target",
        "ship::helm_ai::facts::seed_pressed_facts",
    ),
    ship(
        TARGET_FACING_SHIELD_DOWN,
        "helm",
        "1.0 when the target's facing shield arc is down",
        "false — no target",
        "ship::helm_ai::facts::seed_torpedo_opportunity_facts",
    ),
    ship(
        TORPEDOES_IN_FLIGHT,
        "helm",
        "this ship's unresolved torpedo rounds",
        "false — none / no tubes",
        "ship::helm_ai::facts::seed_torpedo_opportunity_facts",
    ),
    ship(
        TUBES_FULL,
        "helm",
        "1.0 when every tube is at volley_max",
        "false — not a full salvo",
        "ship::helm_ai::facts::seed_torpedo_opportunity_facts",
    ),
    ship(
        TUBES_FILLABLE,
        "helm",
        "1.0 when a whole salvo is still reachable",
        "false — not reachable / no tubes",
        "ship::helm_ai::facts::seed_torpedo_opportunity_facts",
    ),
];

const SENSORS_SELECTOR_FACTS: &[FactDescriptor] = &[
    cand(
        DETECTABLE,
        "sensors selector",
        "1.0 for a detectable contact",
        "false — not surfaced",
        "ship::sensors::detectable_candidate",
    ),
    cand(
        HOSTILE,
        "sensors selector",
        "1.0 for a hostile contact",
        "false — not hostile",
        "ship::sensors::detectable_candidate",
    ),
    cand(
        SOURCE_COMBAT_LOCK,
        "sensors selector",
        "1.0 when the candidate came from the combat lock source",
        "false — not that source",
        "ship::sensors::operate_sensors_ai",
    ),
    cand(
        SOURCE_OBJECTIVE,
        "sensors selector",
        "1.0 when the candidate came from the objective source",
        "false — not that source",
        "ship::sensors::operate_sensors_ai",
    ),
    cand(
        SOURCE_RADAR,
        "sensors selector",
        "1.0 when the candidate came from the radar source",
        "false — not that source",
        "ship::sensors::operate_sensors_ai",
    ),
    ship(
        POWER_RATING,
        "sensors selector",
        "authored ship power rating (self context)",
        "false — the ship declares none",
        "ship::sensors::operate_sensors_ai",
    ),
];

const TACTICAL_SELECTOR_FACTS: &[FactDescriptor] = &[
    cand(
        DETECTABLE,
        "tactical selector",
        "1.0 for a detectable contact",
        "false — not surfaced",
        "console::weapons::ai_target_selection",
    ),
    cand(
        HOSTILE,
        "tactical selector",
        "1.0 for a hostile contact",
        "false — not hostile",
        "console::weapons::ai_target_selection",
    ),
    cand(
        SOURCE_SENSORS_DESIGNATION,
        "tactical selector",
        "1.0 when the candidate is the Sensors designation",
        "false — not that source",
        "console::weapons::ai_target_selection",
    ),
    cand(
        SOURCE_OBJECTIVE,
        "tactical selector",
        "1.0 when the candidate came from an objective",
        "false — not that source",
        "console::weapons::ai_target_selection",
    ),
    cand(
        SOURCE_OPERATE,
        "tactical selector",
        "1.0 when the candidate came from the operate source",
        "false — not that source",
        "console::weapons::ai_target_selection",
    ),
    cand(
        SOURCE_LAST_ATTACKER,
        "tactical selector",
        "1.0 when the candidate is a recent attacker",
        "false — not that source",
        "console::weapons::ai_target_selection",
    ),
    cand(
        SOURCE_RETAINED,
        "tactical selector",
        "1.0 when the candidate is the retained selection",
        "false — not that source",
        "console::weapons::ai_target_selection",
    ),
    cand(
        SOURCE_RADAR,
        "tactical selector",
        "1.0 when the candidate came from radar",
        "false — not that source",
        "console::weapons::ai_target_selection",
    ),
    ship(
        POWER_RATING,
        "tactical selector",
        "authored ship power rating (self context)",
        "false — the ship declares none",
        "console::weapons::ai_target_selection",
    ),
];

const NAVIGATION_SELECTOR_FACTS: &[FactDescriptor] = &[
    cand(
        REACHABLE,
        "navigation selector",
        "1.0 for a reachable destination",
        "false — not reachable",
        "console::navigation::nav_objective_candidate",
    ),
    cand(
        SOURCE_NAV_OBJECTIVE,
        "navigation selector",
        "1.0 when the candidate came from a nav objective",
        "false — not that source",
        "console::navigation::nav_objective_candidate",
    ),
    cand(
        OBJECTIVE_SCORE,
        "navigation selector",
        "the originating objective's score",
        "false — no reading",
        "console::navigation::nav_objective_candidate",
    ),
    cand(
        SOURCE_CHART_CONTACT,
        "navigation selector",
        "1.0 when the candidate is a chart contact",
        "false — not that source",
        "console::navigation::chart_contact_candidate",
    ),
    ship(
        POWER_RATING,
        "navigation selector",
        "authored ship power rating (self context)",
        "false — the ship declares none",
        "console::navigation::operate_navigation_ai",
    ),
];

const REPAIR_SELECTOR_FACTS: &[FactDescriptor] = &[
    cand(
        TIER_ORDINAL,
        "repair selector",
        "the station's repair-tier ordinal",
        "false — no reading",
        "console::repair::server::seed_repair_facts",
    ),
    cand(
        DEFICIT,
        "repair selector",
        "the station's health deficit",
        "false — no reading",
        "console::repair::server::seed_repair_facts",
    ),
    cand(
        DAMAGE_FRACTION,
        "repair selector",
        "damage as a fraction of capacity",
        "false — no reading",
        "console::repair::server::seed_repair_facts",
    ),
    cand(
        WORST_SYSTEM_DAMAGE_FRACTION,
        "repair selector",
        "the worst fine-system's damage fraction",
        "false — no reading",
        "console::repair::server::seed_repair_facts",
    ),
    cand(
        SYSTEM_COUNT,
        "repair selector",
        "how many fine systems the station carries",
        "false — no reading",
        "console::repair::server::seed_repair_facts",
    ),
    cand(
        IS_CORE,
        "repair selector",
        "1.0 for a core station",
        "false — non-core",
        "console::repair::server::seed_repair_facts",
    ),
    cand(
        SOURCE_REPAIR_REQUEST,
        "repair selector",
        "1.0 when a repair was requested for this station",
        "false — no request",
        "console::repair::server::seed_repair_facts",
    ),
    cand(
        SOURCE_CORE_BUCKET,
        "repair selector",
        "1.0 for a core-bucket candidate",
        "false — not that bucket",
        "console::repair::server::seed_repair_facts",
    ),
    cand(
        ASSIGNED,
        "repair selector",
        "1.0 when a team is already assigned here",
        "false — unassigned",
        "console::repair::server::seed_repair_facts",
    ),
    ship(
        FREE_TEAM_COUNT,
        "repair selector",
        "how many repair teams are idle this tick (self context)",
        "false — no reading",
        "console::repair::server::seed_repair_self_facts",
    ),
    ship(
        TOTAL_HULL_HEALTH_FRACTION,
        "repair selector",
        "ship-wide health fraction (self context)",
        "false — no reading",
        "console::repair::server::seed_repair_self_facts",
    ),
    ship(
        RED_ALERT,
        "repair selector",
        "1.0 while the ship is at red alert (self context)",
        "false — reads as not-alert",
        "console::repair::server::seed_repair_self_facts",
    ),
    ship(
        POWER_RATING,
        "repair selector",
        "authored ship power rating (self context)",
        "false — the ship declares none",
        "console::repair::server::seed_repair_self_facts",
    ),
];

const COMMS_SELECTOR_FACTS: &[FactDescriptor] = &[
    cand(
        SOURCE_HAIL_OBJECTIVE,
        "comms hail selector",
        "1.0 when the candidate came from a hail objective",
        "false — not that source",
        "console::comms::server::seed_comms_hail_facts",
    ),
    cand(
        SOURCE_COMMS_CONTACT,
        "comms hail selector",
        "1.0 when the candidate is a comms contact",
        "false — not that source",
        "console::comms::server::seed_comms_hail_facts",
    ),
    cand(
        OBJECTIVE_SCORE,
        "comms hail selector",
        "the originating objective's score",
        "false — no reading",
        "console::comms::server::seed_comms_hail_facts",
    ),
    cand(
        IN_RANGE,
        "comms hail selector",
        "1.0 when the contact is in comms range",
        "false — out of range",
        "console::comms::server::seed_comms_hail_facts",
    ),
    cand(
        IS_URGENT,
        "comms hail selector",
        "1.0 when the hail is urgent",
        "false — not urgent",
        "console::comms::server::seed_comms_hail_facts",
    ),
    cand(
        HAS_OPEN_HAIL_THREAD,
        "comms hail selector",
        "1.0 when a hail thread is already open",
        "false — none open",
        "console::comms::server::seed_comms_hail_facts",
    ),
    cand(
        HAS_UNREAD_FROM_SENDER,
        "comms hail selector",
        "1.0 when there is unread traffic from the sender",
        "false — none unread",
        "console::comms::server::seed_comms_hail_facts",
    ),
    cand(
        MANDATORY,
        "comms hail selector",
        "1.0 when the originating objective is mandatory",
        "false — optional",
        "console::comms::server::seed_comms_hail_facts",
    ),
    ship(
        COMMS_AVAILABLE,
        "comms hail selector",
        "1.0 while the Comms fine system is available (self context)",
        "false — offline",
        "console::comms::server::seed_comms_self_facts",
    ),
    ship(
        RED_ALERT,
        "comms hail selector",
        "1.0 while the ship is at red alert (self context)",
        "false — reads as not-alert",
        "console::comms::server::seed_comms_self_facts",
    ),
    ship(
        CONTACT_COUNT,
        "comms hail selector",
        "how many hailable contacts the roster carries (self context)",
        "false — no reading",
        "console::comms::server::seed_comms_self_facts",
    ),
    ship(
        POWER_RATING,
        "comms hail selector",
        "authored ship power rating (self context)",
        "false — the ship declares none",
        "console::comms::server::seed_comms_self_facts",
    ),
];

/// Levenshtein edit distance, for the nearest-fact suggestion in a rejection
/// (issue #1210). Small inputs (fact names), run only on a load error, so the
/// classic two-row dynamic-programming form is more than fast enough.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

impl AiHost {
    /// Reject a guard expression this host could never evaluate — world state
    /// with no flag chain (issue #891 stage 1), or a bounded history window
    /// nothing folds (issue #890).
    ///
    /// `what` is the validator's own rule/transition/term label, so the message
    /// reads as one sentence with the rest of the content-error surface.
    pub fn check_guard(&self, what: &str, pred: &Predicate) -> Result<(), String> {
        self.check_world_state(what, pred)?;
        self.check_history(what, pred)
    }

    /// Reject a typed `fact(...)` / `candidate_fact(...)` / `target_fact(...)`
    /// atom naming a fact this host does not seed (issue #1210).
    ///
    /// The author-facing half of closing PRD #774 §11's unvalidated-`fact()`
    /// hole: a fact seeded nowhere reads absent (false) for ever, so a typo in a
    /// guard silently disables it — exactly the failure `param(...)` and
    /// `memory(...)` validation already prevents for their vocabularies. The
    /// per-host [`facts`](Self::facts) registry is the declaration the name is
    /// checked against, and the rejection names the host, the block, the atom,
    /// and the nearest fact the host really seeds.
    ///
    /// A sibling of [`check_guard`](Self::check_guard) rather than folded into
    /// it: `check_guard` asks whether an atom's evaluation CONTEXT exists on the
    /// host (a flag chain, a history fold), while this asks whether a fact NAME
    /// is in the host's seeded vocabulary. The two production validators —
    /// `check_policy_predicate` and `validate_selector_inner` — run both.
    pub fn check_facts(&self, what: &str, pred: &Predicate) -> Result<(), String> {
        let mut refs = Vec::new();
        pred.referenced_facts(&mut refs);
        for (ctx, name) in refs {
            // The private contexts (memory / state_time) are not facts and are
            // validated against their own declarations elsewhere.
            let Some(scope) = FactScope::from_context(ctx) else {
                continue;
            };
            if self.seeds_fact(scope, &name) {
                continue;
            }
            return Err(format!(
                "{what} reads {}({name}), but the {} system ({}) never seeds a fact by \
                 that name — an unseeded fact reads absent (false) for ever, so the guard \
                 would be silently dead.{}",
                scope.keyword(),
                self.system,
                self.block,
                self.nearest_seeded(scope, &name)
            ));
        }
        Ok(())
    }

    /// Whether this host seeds `name` in `scope`, honouring prefix families.
    fn seeds_fact(&self, scope: FactScope, name: &str) -> bool {
        self.facts.iter().any(|d| {
            d.scope == scope
                && match d.shape {
                    FactShape::Exact => d.name.name() == name,
                    FactShape::Prefix => name.starts_with(d.name.name()),
                }
        })
    }

    /// A " Did you mean `x`?" tail naming the nearest exact fact this host seeds
    /// in `scope`, or `""` when nothing is close enough to be a likely typo.
    fn nearest_seeded(&self, scope: FactScope, name: &str) -> String {
        let best = self
            .facts
            .iter()
            .filter(|d| d.scope == scope && d.shape == FactShape::Exact)
            .map(|d| d.name.name())
            .min_by_key(|candidate| levenshtein(candidate, name));
        match best {
            Some(candidate)
                if levenshtein(candidate, name) <= name.len().max(candidate.len()) / 2 =>
            {
                format!(" Did you mean `{candidate}`?")
            }
            _ => String::new(),
        }
    }

    /// Reject a `history(...)` guard on a host that folds no window
    /// (issue #890).
    ///
    /// Without this the atom would parse, validate, and then read ABSENT for
    /// ever — the `fact(...)` trap #779 shipped, the `flag(...)` trap #891
    /// closed, and precisely the failure this operator exists to stop content
    /// authors walking into. A window that nobody advances is never full, and a
    /// window that is never full reduces to nothing, so every comparison against
    /// it is quietly `false`.
    fn check_history(&self, what: &str, pred: &Predicate) -> Result<(), String> {
        if self.history_fold.is_some() {
            return Ok(());
        }
        let Some(atom) = pred.history_atom() else {
            return Ok(());
        };
        Err(format!(
            "{what} reads {}, but nothing folds a bounded history window for the {} \
             system ({}) — no host advances one for it once per shared AI tick — so \
             the window would never fill and the comparison would read false for \
             ever. Remove it, or add the once-per-tick fold for that host and name \
             it in ai_flag_hosts::AI_HOSTS",
            atom.render(),
            self.system,
            self.block
        ))
    }

    /// Reject a `flag(...)`/`counter(...)` guard on a host whose runtime
    /// evaluation gets no flag chain (issue #891 stage 1).
    ///
    /// The offending atom is quoted back verbatim: with `flag(...)` and
    /// `counter(...)` both rejected here, "which one did I write?" is the
    /// author's immediate next question.
    fn check_world_state(&self, what: &str, pred: &Predicate) -> Result<(), String> {
        if self.flag_chain == FlagChain::Plumbed {
            return Ok(());
        }
        let mut refs = Vec::new();
        pred.referenced_world_state(&mut refs);
        let Some(atom) = refs.first() else {
            return Ok(());
        };
        Err(format!(
            "{what} references {atom}, but the {} system ({}) evaluates its AI \
             guards with NO world-flag chain plumbed — so {atom} would read false \
             for ever. Remove it, or plumb the flag chain into that host first \
             (issue #891 stage 2)",
            self.system, self.block
        ))
    }
}

#[cfg(test)]
#[path = "ai_flag_hosts_tests.rs"]
mod tests;

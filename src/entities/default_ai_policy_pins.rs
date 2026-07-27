//! Behavioural PINS for the nineteen Rust-side AI synthesisers (issue #885,
//! step 1): the fourteen `default_*_ai_config()` policies and the five
//! `default_*_target_selector_config()` target selectors.
//!
//! # Why this file exists, and when to delete it
//!
//! Today a hull that authors nothing for an AI-capable fine system does not get
//! "no AI" — it gets a canonical policy **invented for it in Rust** at spawn.
//! Those synthesisers are therefore the de facto specification of default NPC
//! behaviour, and that specification is written down **nowhere else**: not in
//! TOML, not in PASM, not in the wiki. Issue #885 replaces roughly 190–220
//! synthesised declarations with authored ones across twelve hulls and then
//! deletes the synthesisers. Every one of those authored declarations must
//! reproduce what is pinned here *exactly* or NPC behaviour regresses silently.
//!
//! This module is the artefact to diff the authored replacements against. It
//! asserts CONTENT — channels, verbs, rule priorities, guard expressions,
//! params, payloads, statelessness; and for the selectors, sources, horizons,
//! eligibility, every scoring term and its weight, the switch margin and the
//! tie-break — not merely that a policy is `Some(_)`, so a reader can
//! transcribe the equivalent TOML from a failing assertion without opening
//! `config.rs`.
//!
//! **This suite is expected to be DELETED together with the synthesisers when
//! #885 lands.** It pins the current baseline on purpose; it is not a statement
//! that the baseline is desirable. Anything surprising found while writing it
//! was reported on #885 rather than fixed, because a pin that "fixes" what it
//! pins is worthless.
//!
//! # The fourteen, and what each one substitutes for
//!
//! | synthesiser | authored block it stands in for | runtime component |
//! |---|---|---|
//! | `default_captain_ai_config` | `[captain_console.ai]` | `CaptainAiPolicy` |
//! | `default_comms_response_ai_config` | `[comms_console.ai]` | `CommsResponseAiPolicy` |
//! | `default_engines_ai_config` | `[helm_console.engines_ai]` | `HelmEnginesAiPolicy` |
//! | `default_steering_ai_config` | `[helm_console.steering_ai]` | `HelmSteeringAiPolicy` |
//! | `default_lateral_ai_config` | `[helm_console.lateral_ai]` | `HelmLateralAiPolicy` |
//! | `default_vertical_ai_config` | `[helm_console.vertical_ai]` | `HelmVerticalAiPolicy` |
//! | `default_impulse_ai_config` | `[helm_console.impulse_ai]` | `HelmImpulseAiPolicy` |
//! | `default_boost_ai_config` | `[helm_console.boost_ai]` | `HelmBoostAiPolicy` |
//! | `default_phaser_bank_ai_config` | `[[weapons_console.phaser_banks]].ai` | `PhaserBankAiPolicies[id]` |
//! | `default_blaster_bank_ai_config` | `[[weapons_console.blaster_banks]].ai` | `BlasterBankAiPolicies[id]` |
//! | `default_torpedo_tube_ai_config` | `[[torpedoes.tubes]].ai` | `TorpedoTubeAiPolicies[id]` |
//! | `default_torpedo_magazine_ai_config` | `[torpedoes].ai` | `TorpedoMagazineAiPolicy` |
//! | `default_shields_focus_ai_config` | `[shields_console.ai_policy]` | `ShieldsFocusAiPolicy` |
//! | `default_power_ai_config` | `[power.ai_policy]` | `PowerAiPolicy` |
//!
//! # The five SELECTORS, and why they are the same story
//!
//! | synthesiser | authored block it stands in for | runtime component |
//! |---|---|---|
//! | `default_sensors_target_selector_config` | `[sensors_console.selector]` | `SensorsTargetSelector` |
//! | `default_tactical_target_selector_config` | `[weapons_console.selector]` | `TacticalTargetSelector` |
//! | `default_navigation_target_selector_config` | `[navigation_console.selector]` | `NavigationTargetSelector` |
//! | `default_repair_target_selector_config` | `[repair.selector]` | `RepairTargetSelector` |
//! | `default_comms_target_selector_config` | `[comms_console.selector]` | `CommsTargetSelector` |
//!
//! They are a *separate* synthesised family — they produce
//! `FineSystemAiSelectorToml` → `TargetSelector` ("which entity is my
//! target?"), not `FineSystemAiConfigToml` → `AiPolicy` ("which verb do I emit
//! on channel C?") — but they are squarely inside #885's 190–220 declaration
//! count, they are attached at the same spawn, and they share the same fate:
//! authored per hull, then deleted. So they are pinned here rather than in a
//! second suite telling a second story.
//!
//! Two things make them the *riskier* half to migrate:
//!
//! - **They carry the densest logic in the set.** `default_repair_*`'s banded
//!   tier/deficit ladder is a page of reasoning, and `default_tactical_*`
//!   encodes a documented dominance invariant (`500 + 200 + 100 + 1 = 801 <
//!   1000 − 50`) that makes an explicit mission objective strictly beat the
//!   maximum stack of everything else, hysteresis included.
//! - **Not one shipped hull authors a selector block.** Where the policies have
//!   `alliance_cruiser`'s hand-written `[captain_console.ai]` as a worked
//!   example proving a verbatim transcription round-trips, the selectors have
//!   nothing: all 5 × 12 declarations are synthesised, and this suite is the
//!   only baseline they can be diffed against.
//!
//! # How the pins are layered
//!
//! 1. **Roll call** — fourteen policies, all stateless; five selectors, all
//!    reachable, all decoding and validating against their registered sources.
//! 2. **Content pins** — one test per synthesiser over the *authorable* shape
//!    (`FineSystemAiConfigToml` / `FineSystemAiSelectorToml`), because that is
//!    what #885 must retype in TOML. Values are asserted as LITERALS, never
//!    against the constant the synthesiser itself reads, so a changed constant
//!    fails the pin instead of silently moving with it. (`config.rs` has its own
//!    `const {}` invariant tests written *in terms of* those constants; these
//!    are the complementary half — the two disagree the moment a constant moves.)
//! 3. **Decode pins** — the same content after `to_policy()`, pinning which
//!    typed `AiPolicyVerb` each channel yields and which payloads ride along.
//! 4. **Guard truth tables** — every guarded rule is proved to fire *and* to be
//!    able to read false, through `resolve_channel`. This is the trap #885
//!    calls out: an unseeded or misspelled `fact(...)` parses, validates, and
//!    reads false for ever.
//! 5. **Selector INVARIANT pins** — for the selectors the numbers are not the
//!    specification, the *ordering* is. Pinned by running the real
//!    `TargetSelector::select` over hand-built candidates, so a reweight where
//!    every individual weight still looks plausible but the ordering inverts
//!    fails here. This is the layer that earns the suite its keep.
//! 6. **Spawn-path pins** — WHICH systems get a synthesised policy or selector
//!    at all, run through the real `spawn_entity` path on shipped hulls rather
//!    than by calling the synthesisers directly. #792 found a fixture that
//!    omitted the very components that broke the feature in production; a pin
//!    that does not travel the real path can drift from it.

use super::config::{
    default_blaster_bank_ai_config, default_boost_ai_config, default_captain_ai_config,
    default_comms_response_ai_config, default_comms_target_selector_config,
    default_engines_ai_config, default_impulse_ai_config, default_lateral_ai_config,
    default_navigation_target_selector_config, default_phaser_bank_ai_config,
    default_power_ai_config, default_repair_target_selector_config,
    default_sensors_target_selector_config, default_shields_focus_ai_config,
    default_steering_ai_config, default_tactical_target_selector_config,
    default_torpedo_magazine_ai_config, default_torpedo_tube_ai_config, default_vertical_ai_config,
    validate_fine_system_ai_selector, FineSystemAiConfigToml, FineSystemAiSelectorToml,
    COMMS_SELECTOR_SOURCES, NAVIGATION_SELECTOR_SOURCES, REPAIR_SELECTOR_SOURCES,
    SENSORS_SELECTOR_SOURCES, TACTICAL_SELECTOR_SOURCES,
};
use crate::ai::policy::{AiPolicy, AiPolicyVerb};
use crate::ai::selector::{SelectorCandidate, SelfContext, TargetSelector};
use crate::world::flags::AiFacts;

// ─────────────────────────────────────────────────────────────────────────────
// Assertion helpers
// ─────────────────────────────────────────────────────────────────────────────

/// One authored rule exactly as a designer would have to retype it in TOML.
///
/// Field-for-field with `FineSystemAiRuleToml`, so a failing assertion tells the
/// #885 author precisely which TOML key is wrong.
struct PinnedRule {
    priority: i32,
    channel: &'static str,
    when: &'static str,
    verb: &'static str,
    value: bool,
    level: u8,
    response_index: u8,
}

/// A value-less rule: `value`, `level` and `response_index` are all defaulted.
///
/// Most synthesised rules are of this shape because most verbs carry no payload
/// — the magnitude lives in a host-seeded fact, not in the policy.
const fn modal(
    priority: i32,
    channel: &'static str,
    when: &'static str,
    verb: &'static str,
) -> PinnedRule {
    PinnedRule {
        priority,
        channel,
        when,
        verb,
        value: false,
        level: 0,
        response_index: 0,
    }
}

/// Assert a synthesiser's rule list matches `expected` exactly, in order.
///
/// Order matters and is asserted: resolution is "highest priority wins, ties to
/// the earliest-authored", so the position of a rule in the list is part of the
/// specification, not an accident of formatting.
fn assert_rules(name: &str, cfg: &FineSystemAiConfigToml, expected: &[PinnedRule]) {
    assert_eq!(
        cfg.rule.len(),
        expected.len(),
        "{name}: rule COUNT is part of the pin — #885 must author exactly {} rule(s), \
         found {}. A rule gained or lost here changes NPC behaviour silently.",
        expected.len(),
        cfg.rule.len()
    );
    for (i, (actual, want)) in cfg.rule.iter().zip(expected).enumerate() {
        assert_eq!(
            actual.priority, want.priority,
            "{name}: rule[{i}] priority. Priorities order the rules WITHIN a channel \
             (higher wins, ties to earliest-authored), so this number decides which \
             rule the runtime picks."
        );
        assert_eq!(
            actual.channel, want.channel,
            "{name}: rule[{i}] channel. The channel name is the host's lookup key — \
             a rule on an unknown channel is never resolved and the system goes silent."
        );
        assert_eq!(
            actual.when, want.when,
            "{name}: rule[{i}] guard expression, character for character. Every \
             `fact(...)` name in it must be one the host actually seeds: an unseeded \
             or misspelled fact parses, validates, and reads FALSE for ever."
        );
        assert_eq!(
            actual.verb, want.verb,
            "{name}: rule[{i}] verb. The verb is the typed output the host acts on; \
             an unknown verb is rejected at load, a WRONG-but-known one is not."
        );
        assert_eq!(
            actual.value, want.value,
            "{name}: rule[{i}] `value` (the boolean payload; only `set_red_alert` reads it)."
        );
        assert_eq!(
            actual.level, want.level,
            "{name}: rule[{i}] `level` (the magnitude payload; only \
             `set_power_group_allocation` reads it)."
        );
        assert_eq!(
            actual.response_index, want.response_index,
            "{name}: rule[{i}] `response_index` (only `respond_to_message` reads it)."
        );
    }
}

/// Assert the synthesiser declares exactly this set of named params.
///
/// The set is exact in both directions: a param the guards never reference is
/// still part of the pin (the host may read it straight out of `policy.params`,
/// which is how Shields gets its windows and thresholds), and a MISSING param
/// makes every `param(...)` comparison referencing it evaluate false.
fn assert_params(name: &str, cfg: &FineSystemAiConfigToml, expected: &[(&str, f32)]) {
    assert_eq!(
        cfg.param.len(),
        expected.len(),
        "{name}: param COUNT. Declared params are pinned exactly — validation rejects a \
         `param(...)` reference the author never declared, and some hosts read params \
         directly out of the resolved policy. Got {:?}",
        cfg.param
    );
    for (key, want) in expected {
        assert_eq!(
            cfg.param.get(*key),
            Some(want),
            "{name}: param `{key}` must be {want}. This is the tuning number #885 has \
             to carry across verbatim; the synthesiser is its only current home."
        );
    }
}

/// Assert the synthesiser declares NO #882 state machine.
///
/// All fourteen are stateless: no `initial_state`, no `[[...state]]` tables, no
/// `memory` slots — so `to_policy()` yields `machine: None` and the stateful
/// evaluator is never entered. A stateless policy also may not reference
/// `memory(...)` or `state_time` at all; validation rejects that.
fn assert_stateless(name: &str, cfg: &FineSystemAiConfigToml) {
    assert_eq!(
        cfg.initial_state, None,
        "{name}: STATELESS — no `initial_state` is authored."
    );
    assert!(
        cfg.state.is_empty(),
        "{name}: STATELESS — no `[[...state]]` tables are authored; a machine would \
         change how every rule resolves (in-state only)."
    );
    assert!(
        cfg.memory.is_empty(),
        "{name}: STATELESS — no `memory` slots are declared, so no guard here may \
         reference `memory(...)` or `state_time`."
    );

    let policy = cfg
        .to_policy()
        .unwrap_or_else(|e| panic!("{name}: the synthesised block must decode: {e}"));
    assert!(
        policy.machine().is_none() && policy.initial_state().is_none(),
        "{name}: a stateless block decodes to `machine: None` — the #882 state/memory \
         code is never entered."
    );
}

/// A fact snapshot built from `(name, value)` pairs; anything not listed is
/// ABSENT, which makes every comparison against it evaluate false.
fn facts(pairs: &[(&str, f64)]) -> AiFacts {
    let mut f = AiFacts::new();
    for (k, v) in pairs {
        f.set(k, *v);
    }
    f
}

/// The resolved verb for one channel against a fact snapshot and no world flags.
fn resolve(
    cfg: &FineSystemAiConfigToml,
    channel: &str,
    snapshot: &AiFacts,
) -> Option<AiPolicyVerb> {
    cfg.to_policy()
        .expect("synthesised block decodes")
        .resolve_channel(channel, snapshot, &[])
        .cloned()
}

/// The canonical decoded policy for a synthesiser, for equality pins against
/// what the real spawn path attached.
fn decoded(cfg: FineSystemAiConfigToml) -> AiPolicy {
    cfg.to_policy().expect("synthesised block decodes")
}

// ── Selector assertion helpers ───────────────────────────────────────────────

/// One authored selector exactly as a designer would have to retype it in TOML.
///
/// Field-for-field with `FineSystemAiSelectorToml` (minus the `param` map and
/// `score` list, which are asserted separately so a failure names the offending
/// entry), so a failing assertion tells the #885 author precisely which TOML key
/// is wrong.
struct PinnedSelector {
    /// Registered candidate-source ids, in authored order.
    sources: &'static [&'static str],
    horizon: f32,
    switch_margin: f32,
    eligibility: &'static str,
    /// Declared `[*.selector.param]` entries, exact in both directions.
    params: &'static [(&'static str, f32)],
    /// `[[*.selector.score]]` terms as `(when, weight)`, in authored order.
    score: &'static [(&'static str, f32)],
}

/// Assert a selector synthesiser matches `expected` exactly, field for field.
///
/// Everything here is asserted against a LITERAL rather than against the
/// `DEFAULT_*` constant the synthesiser reads, so a retuned constant fails the
/// pin instead of travelling with it. That is the whole point: `config.rs`
/// already has `const {}` tests phrased in terms of those constants, and a pin
/// phrased the same way would agree with any value they were changed to.
fn assert_selector(name: &str, cfg: &FineSystemAiSelectorToml, want: &PinnedSelector) {
    assert_eq!(
        cfg.sources, want.sources,
        "{name}: candidate SOURCES, in order. The union of these is the entire \
         population the selector can ever choose from — a source dropped here is a \
         target the system stops being able to see, and content validation rejects \
         a source id that is not registered for this system."
    );
    assert_eq!(
        cfg.horizon, want.horizon,
        "{name}: `horizon` — the planar distance beyond which candidates are dropped \
         before scoring. Every one of the five is the same very large static outer \
         bound, because each HOST owns its own live gate (Sensors' damage-scaled \
         range, Comms' authored `[comms].range`); the selector's horizon is not the \
         real range limit and must not be 'helpfully' tightened to look like one."
    );
    assert_eq!(
        cfg.switch_margin, want.switch_margin,
        "{name}: `switch_margin` — the hysteresis band within which the CURRENT \
         target is retained over a better-scoring rival. Part of the ranking \
         specification, not a comfort setting: Tactical's value is load-bearing in \
         its dominance invariant."
    );
    assert_eq!(
        cfg.eligibility, want.eligibility,
        "{name}: `eligibility` guard, character for character. Every \
         `candidate_fact(...)` / `self_fact(...)` name in it must be one the host \
         actually seeds: an unseeded or misspelled fact parses, validates, and reads \
         FALSE for ever, which for an eligibility guard means the system selects \
         NOTHING, permanently and silently."
    );

    assert_eq!(
        cfg.param.len(),
        want.params.len(),
        "{name}: declared param COUNT. Exact in both directions — validation rejects \
         a `param(...)` reference the author never declared, and a MISSING param makes \
         every comparison referencing it read false. Got {:?}",
        cfg.param
    );
    for (key, value) in want.params {
        assert_eq!(
            cfg.param.get(*key),
            Some(value),
            "{name}: param `{key}` must be {value}. This is a tuning number #885 has \
             to carry across verbatim; the synthesiser is its only current home."
        );
    }

    assert_eq!(
        cfg.score.len(),
        want.score.len(),
        "{name}: score TERM COUNT is part of the pin — #885 must author exactly {} \
         `[[...score]]` table(s), found {}. The ladders are counted ladders: Repair's \
         three tier steps and three deficit bands, and Comms' three score bands, mean \
         what they mean because of how many of them there are.",
        want.score.len(),
        cfg.score.len()
    );
    for (i, (actual, (when, weight))) in cfg.score.iter().zip(want.score).enumerate() {
        assert_eq!(
            &actual.when, when,
            "{name}: score[{i}] guard expression, character for character."
        );
        assert_eq!(
            actual.weight, *weight,
            "{name}: score[{i}] weight. Terms are ADDITIVE and a single candidate can \
             satisfy several at once, so this number only means something relative to \
             its siblings — see the invariant pins."
        );
    }
}

/// The canonical decoded selector, for equality pins against what the real
/// spawn path attached and as the subject of the invariant pins.
fn decoded_selector(cfg: FineSystemAiSelectorToml) -> TargetSelector {
    cfg.to_selector()
        .expect("the synthesised selector block must decode")
}

/// A candidate at the ship's own origin, so the horizon filter never
/// participates — these pins are about ranking, not about distance.
fn candidate(uuid: &str, pairs: &[(&str, f64)]) -> SelectorCandidate {
    SelectorCandidate {
        uuid: uuid.to_string(),
        position: [0.0, 0.0, 0.0],
        facts: facts(pairs),
    }
}

/// A SELF context at the origin carrying `pairs` as `self_fact(...)` readings.
fn self_ctx(pairs: &[(&str, f64)]) -> SelfContext {
    SelfContext {
        position: [0.0, 0.0, 0.0],
        facts: facts(pairs),
    }
}

/// Run a selector over `candidates` with no world flags, returning the chosen
/// uuid.
fn pick(
    sel: &TargetSelector,
    ctx: &SelfContext,
    candidates: &[SelectorCandidate],
    current: Option<&str>,
) -> Option<String> {
    sel.select(ctx, candidates, current, &[])
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Roll call
// ─────────────────────────────────────────────────────────────────────────────

/// Every synthesiser, paired with the authored TOML block it stands in for.
///
/// Kept in one place so the count and the block-path mapping are asserted
/// together: #885 AC2 deletes all fourteen, and a fifteenth appearing without a
/// pin is exactly the drift this suite exists to catch.
fn all_synthesisers() -> Vec<(&'static str, &'static str, FineSystemAiConfigToml)> {
    vec![
        (
            "captain",
            "[captain_console.ai]",
            default_captain_ai_config(),
        ),
        (
            "comms_response",
            "[comms_console.ai]",
            default_comms_response_ai_config(),
        ),
        (
            "engines",
            "[helm_console.engines_ai]",
            default_engines_ai_config(),
        ),
        (
            "steering",
            "[helm_console.steering_ai]",
            default_steering_ai_config(),
        ),
        (
            "lateral",
            "[helm_console.lateral_ai]",
            default_lateral_ai_config(),
        ),
        (
            "vertical",
            "[helm_console.vertical_ai]",
            default_vertical_ai_config(),
        ),
        (
            "impulse",
            "[helm_console.impulse_ai]",
            default_impulse_ai_config(),
        ),
        (
            "boost",
            "[helm_console.boost_ai]",
            default_boost_ai_config(),
        ),
        (
            "phaser_bank",
            "[[weapons_console.phaser_banks]].ai",
            default_phaser_bank_ai_config(),
        ),
        (
            "blaster_bank",
            "[[weapons_console.blaster_banks]].ai",
            default_blaster_bank_ai_config(),
        ),
        (
            "torpedo_tube",
            "[[torpedoes.tubes]].ai",
            default_torpedo_tube_ai_config(),
        ),
        (
            "torpedo_magazine",
            "[torpedoes].ai",
            default_torpedo_magazine_ai_config(),
        ),
        (
            "shields_focus",
            "[shields_console.ai_policy]",
            default_shields_focus_ai_config(),
        ),
        ("power", "[power.ai_policy]", default_power_ai_config()),
    ]
}

/// The family has exactly fourteen members and every one of them is a
/// well-formed, STATELESS declaration that decodes without error.
///
/// The count is the pin: #885 AC2 is "all 14 `default_*_ai_config()`
/// synthesisers are deleted", so a fifteenth added without a matching pin — or
/// one quietly removed before its hulls author a replacement — fails here.
#[test]
fn there_are_exactly_fourteen_synthesisers_and_all_are_stateless() {
    let all = all_synthesisers();
    assert_eq!(
        all.len(),
        14,
        "#885 AC2 counts FOURTEEN synthesisers. If this number moved, the migration \
         scope moved with it and a synthesiser is now unpinned."
    );
    for (name, block, cfg) in &all {
        assert_stateless(name, cfg);
        assert!(
            cfg.idle || !cfg.rule.is_empty(),
            "{name} ({block}): a declaration is either explicit `idle` or has rules — \
             'silence' (neither) is rejected by content validation, so #885's authored \
             replacement cannot be an empty block."
        );
    }
}

/// Exactly ONE of the fourteen is an explicit idle: Boost.
///
/// This matters to #885 because "explicit policy or explicit idle" (its AC1) has
/// a different authored shape for each, and getting it backwards is not a
/// validation error — an idle system simply never acts.
#[test]
fn boost_is_the_only_synthesised_idle() {
    let idlers: Vec<&str> = all_synthesisers()
        .iter()
        .filter(|(_, _, cfg)| cfg.idle)
        .map(|(name, _, _)| *name)
        .collect();
    assert_eq!(
        idlers,
        vec!["boost"],
        "Boost is the ONLY system whose synthesised default is `idle = true` (no AI \
         ever engages boost today). Every other default actively emits."
    );
}

/// Every selector synthesiser, paired with the authored block it stands in for
/// and the registered source list its content is validated against.
///
/// Kept in one place for the same reason as `all_synthesisers()`: the count and
/// the block-path mapping are part of what #885 has to replace, and a sixth
/// selector appearing without a pin is exactly the drift this suite catches.
fn all_selectors() -> Vec<(
    &'static str,
    &'static str,
    &'static [&'static str],
    FineSystemAiSelectorToml,
)> {
    vec![
        (
            "sensors",
            "[sensors_console.selector]",
            SENSORS_SELECTOR_SOURCES,
            default_sensors_target_selector_config(),
        ),
        (
            "tactical",
            "[weapons_console.selector]",
            TACTICAL_SELECTOR_SOURCES,
            default_tactical_target_selector_config(),
        ),
        (
            "navigation",
            "[navigation_console.selector]",
            NAVIGATION_SELECTOR_SOURCES,
            default_navigation_target_selector_config(),
        ),
        (
            "repair",
            "[repair.selector]",
            REPAIR_SELECTOR_SOURCES,
            default_repair_target_selector_config(),
        ),
        (
            "comms_hail",
            "[comms_console.selector]",
            COMMS_SELECTOR_SOURCES,
            default_comms_target_selector_config(),
        ),
    ]
}

/// The selector family has exactly five members, and every one of them is a
/// well-formed declaration that VALIDATES against its own registered sources and
/// DECODES to a typed selector.
///
/// The count is the pin. #885's scope line reads "…and five selectors (Sensors,
/// Tactical, Navigation, Repair, Comms) — which today all run Rust-side
/// `*::default()` selectors with nothing authored at all", so a sixth added
/// without a matching pin, or one quietly removed before its hulls author a
/// replacement, fails here.
///
/// Validating as well as decoding matters: `to_selector()` only rejects an
/// unparseable expression, while `validate_fine_system_ai_selector` is what
/// rejects an unregistered source id and a `param(...)` reference to a parameter
/// the author never declared. #885's authored replacements go through the
/// second gate, so the baseline has to clear it too.
#[test]
fn there_are_exactly_five_selector_synthesisers_and_all_validate() {
    let all = all_selectors();
    assert_eq!(
        all.len(),
        5,
        "#885 counts FIVE synthesised target selectors. If this number moved, the \
         migration scope moved with it and a selector is now unpinned."
    );
    for (name, block, sources, cfg) in &all {
        assert!(
            validate_fine_system_ai_selector(cfg, sources).is_ok(),
            "{name} ({block}): the canonical selector must pass the same content \
             validation an authored replacement will face — unregistered source ids \
             and undeclared `param(...)` references are rejected there, not by decoding."
        );
        assert!(
            cfg.to_selector().is_ok(),
            "{name} ({block}): the canonical selector must decode to a typed \
             `TargetSelector`; an unparseable eligibility or score guard fails here."
        );
        assert!(
            !cfg.eligibility.is_empty(),
            "{name} ({block}): a selector has no `idle` flag and no 'silence' shape — \
             its eligibility guard is the only thing that can make it select nothing, \
             so an empty guard is not a legal declaration."
        );
        assert!(
            !cfg.sources.is_empty(),
            "{name} ({block}): a selector with no sources has no candidate population \
             and can never select anything."
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Content pins — one per synthesiser
// ─────────────────────────────────────────────────────────────────────────────

/// Captain Red Alert: raise while recently in combat, otherwise stand down.
///
/// Two rules on ONE channel, which is why they carry distinct priorities: the
/// priority-0 `true` rule is the stand-down fallback and the priority-10 rule
/// overrides it while the combat window holds. Equal priorities on one channel
/// are rejected by validation (issue #794).
#[test]
fn pin_captain() {
    let cfg = default_captain_ai_config();
    assert!(!cfg.idle, "captain: the default ACTS, it is not idle.");
    assert_params("captain", &cfg, &[("combat_window_secs", 10.0)]);
    assert_rules(
        "captain",
        &cfg,
        &[
            PinnedRule {
                priority: 10,
                channel: "red_alert",
                when: "fact(secs_since_combat) < param(combat_window_secs)",
                verb: "set_red_alert",
                // The ONLY synthesised rule anywhere that sets `value = true`.
                value: true,
                level: 0,
                response_index: 0,
            },
            modal(0, "red_alert", "true", "set_red_alert"),
        ],
    );
    assert_stateless("captain", &cfg);
}

/// Comms dialogue response: answer with response index 0 whenever the Comms
/// system is usable and the sender is still in range.
///
/// The two guard clauses are load-bearing rather than decorative. The retired
/// stub ran once, on channel-2 arrival, so its sender was in range by
/// construction; this policy is re-resolved EVERY tick against every open
/// dialogue, and an unguarded rule would re-emit a response the router rejects
/// for ever.
#[test]
fn pin_comms_response() {
    let cfg = default_comms_response_ai_config();
    assert!(!cfg.idle, "comms_response: the default ACTS.");
    assert_params("comms_response", &cfg, &[]);
    assert_rules(
        "comms_response",
        &cfg,
        &[PinnedRule {
            priority: 0,
            channel: "comms_respond",
            when: "fact(comms_available) > 0 and fact(sender_in_range) > 0",
            verb: "respond_to_message",
            value: false,
            level: 0,
            // Response index 0 = "always take the FIRST response", which is
            // precisely the decision the retired channel-2 stub made.
            response_index: 0,
        }],
    );
    assert_stateless("comms_response", &cfg);
}

/// The five unconditional helm/weapon "always act" defaults share one shape:
/// exactly one rule, priority 0, guard `true`, no params.
///
/// Pinned as a table because the shape — not any one hull's instance — is what
/// #885 has to reproduce twelve times over.
#[test]
fn pin_the_unconditional_single_rule_defaults() {
    let cases: Vec<(&str, &str, &str, FineSystemAiConfigToml)> = vec![
        (
            "engines",
            "longitudinal",
            "actuate_desired_travel",
            default_engines_ai_config(),
        ),
        (
            "steering",
            "yaw",
            "actuate_desired_facing",
            default_steering_ai_config(),
        ),
        (
            "lateral",
            "lateral",
            "actuate_lateral_thrust",
            default_lateral_ai_config(),
        ),
        (
            "vertical",
            "vertical",
            "actuate_vertical_thrust",
            default_vertical_ai_config(),
        ),
        (
            "impulse",
            "impulse",
            "engage_impulse",
            default_impulse_ai_config(),
        ),
        (
            "phaser_bank",
            "phaser_fire",
            "fire_phaser",
            default_phaser_bank_ai_config(),
        ),
        (
            "blaster_bank",
            "blaster_fire",
            "fire_blaster",
            default_blaster_bank_ai_config(),
        ),
        (
            "torpedo_magazine",
            "torpedo_magazine_grant",
            "grant_torpedo_round",
            default_torpedo_magazine_ai_config(),
        ),
    ];
    for (name, channel, verb, cfg) in &cases {
        assert!(
            !cfg.idle,
            "{name}: unconditional defaults ACT; only Boost is idle."
        );
        assert_params(name, cfg, &[]);
        assert_rules(name, cfg, &[modal(0, channel, "true", verb)]);
        assert_stateless(name, cfg);
    }
}

/// Boost: an explicit idle. No rules, no params — the AI never engages boost.
///
/// `idle = true` is a legal declaration DISTINCT from an empty block: validation
/// rejects silence but accepts a deliberate idle, and the host resolves `None`
/// every tick and emits nothing. #885's authored replacement for Boost is
/// therefore `idle = true`, not an omitted section.
#[test]
fn pin_boost_idle() {
    let cfg = default_boost_ai_config();
    assert!(
        cfg.idle,
        "boost: the default is an EXPLICIT idle — today no AI ever engages boost, so \
         'takes no boost action' is the baseline-preserving declaration."
    );
    assert_params("boost", &cfg, &[]);
    assert_rules("boost", &cfg, &[]);
    assert_stateless("boost", &cfg);
    assert_eq!(
        resolve(&cfg, "boost", &facts(&[])),
        None,
        "boost: an idle policy resolves `None` on its channel no matter what the facts \
         say — `resolve_channel` short-circuits on `idle` before scanning any rule."
    );
}

/// Torpedo tube: TWO unconditional rules, one per channel — load and launch are
/// separate decisions on separate channels, both defaulting to "always".
///
/// Both sit at priority 0, which is legal precisely because they are on
/// DIFFERENT channels; two priority-0 rules on one channel would be rejected.
#[test]
fn pin_torpedo_tube() {
    let cfg = default_torpedo_tube_ai_config();
    assert!(!cfg.idle, "torpedo_tube: the default ACTS.");
    assert_params("torpedo_tube", &cfg, &[]);
    assert_rules(
        "torpedo_tube",
        &cfg,
        &[
            modal(0, "torpedo_load", "true", "load_torpedo"),
            modal(0, "torpedo_launch", "true", "launch_torpedo"),
        ],
    );
    assert_stateless("torpedo_tube", &cfg);
}

/// Shields focus: two rules on ONE channel, plus four params the HOST reads
/// straight out of `policy.params` rather than through any guard.
///
/// The four params are the arc-ranking kernel's windows and thresholds. Three of
/// them (`damage_window_secs`, `min_damage_window_secs`, `health_ratio_threshold`)
/// are never referenced by a guard at all — they exist purely so the host can
/// read them off the resolved policy. #885 must carry all four across even
/// though only one appears in a `when`.
#[test]
fn pin_shields_focus() {
    let cfg = default_shields_focus_ai_config();
    assert!(!cfg.idle, "shields_focus: the default ACTS.");
    assert_params(
        "shields_focus",
        &cfg,
        &[
            // Window over which incoming damage is accumulated per arc.
            ("damage_window_secs", 4.0),
            // Minimum elapsed window before concentration is trusted.
            ("min_damage_window_secs", 1.0),
            // Share (0–100) of windowed damage one arc must take to count as
            // concentrated. The fact is pre-scaled to a percentage host-side
            // because the predicate grammar has no arithmetic.
            ("damage_pct_threshold", 50.0),
            // Health-imbalance threshold used by the kernel's fallback ranking.
            ("health_ratio_threshold", 50.0),
        ],
    );
    assert_rules(
        "shields_focus",
        &cfg,
        &[
            modal(
                10,
                "shield_focus",
                "fact(recent_damage_pct_max) >= param(damage_pct_threshold)",
                "focus_shield_arc",
            ),
            modal(0, "shield_focus", "true", "focus_shield_arc"),
        ],
    );
    assert_stateless("shields_focus", &cfg);
}

/// Power allocation: four rules across TWO channels (`helm`, `weapons`), each
/// channel an elevate-vs-baseline pair, and four params.
///
/// The magnitude rides the verb here — `set_power_group_allocation` is one of
/// only two value-carrying verbs — so `level` is part of the pin: 3 elevated,
/// 2 baseline. Every rule declares a minimum battery reserve, including the
/// lowering baseline rules, which reference a shared zero-valued
/// `min_reserve_baseline` so they too declare one while never being able to gate
/// a de-allocation.
///
/// Groups this default does not name (`sensors`, `ops`, …) resolve to `None` and
/// hold whatever level the reactor seeded.
#[test]
fn pin_power() {
    let cfg = default_power_ai_config();
    assert!(!cfg.idle, "power: the default ACTS.");
    assert_params(
        "power",
        &cfg,
        &[
            ("thrust_threshold", 0.7),
            ("min_reserve_helm", 50.0),
            ("min_reserve_weapons", 10.0),
            ("min_reserve_baseline", 0.0),
        ],
    );
    assert_rules(
        "power",
        &cfg,
        &[
            PinnedRule {
                priority: 10,
                channel: "helm",
                when: "fact(thrust) >= param(thrust_threshold) \
                       and fact(battery_pct) >= param(min_reserve_helm)",
                verb: "set_power_group_allocation",
                value: false,
                level: 3,
                response_index: 0,
            },
            PinnedRule {
                priority: 0,
                channel: "helm",
                when: "fact(battery_pct) >= param(min_reserve_baseline)",
                verb: "set_power_group_allocation",
                value: false,
                level: 2,
                response_index: 0,
            },
            PinnedRule {
                priority: 10,
                channel: "weapons",
                when: "fact(red_alert) > 0 \
                       and fact(battery_pct) >= param(min_reserve_weapons)",
                verb: "set_power_group_allocation",
                value: false,
                level: 3,
                response_index: 0,
            },
            PinnedRule {
                priority: 0,
                channel: "weapons",
                when: "fact(battery_pct) >= param(min_reserve_baseline)",
                verb: "set_power_group_allocation",
                value: false,
                level: 2,
                response_index: 0,
            },
        ],
    );
    assert_stateless("power", &cfg);

    let named: Vec<&str> = cfg.rule.iter().map(|r| r.channel.as_str()).collect();
    assert!(
        !named.contains(&"sensors"),
        "power: the default names ONLY `helm` and `weapons`. Any other authored group \
         resolves to `None` and holds its seeded level — that silence is itself part \
         of the specification #885 must reproduce."
    );
}

// ── 2b. Content pins — one per SELECTOR ──────────────────────────────────────
//
// Same contract as the policy content pins above: everything a designer would
// have to retype in TOML, asserted as a literal. A reader should be able to
// author `[sensors_console.selector]` from `pin_sensors_selector` alone.

/// Sensors: rank detectable hostiles by which SOURCE surfaced them —
/// combat-lock ≫ objective ≫ radar — with no hysteresis.
///
/// The weights reproduce the retired hardcoded Sensors tier chain: Sensors
/// mirrors whatever Tactical has locked (so the science console shows what the
/// ship is shooting at), else an explicitly named `Destroy` objective, else the
/// nearest hostile its own radar found.
///
/// `switch_margin` is 0 — Sensors has NO hysteresis. The gap between tiers
/// (1000 / 100 / 1) is itself the anti-thrash mechanism; there is no band in
/// which a rival is ignored.
#[test]
fn pin_sensors_selector() {
    assert_selector(
        "sensors",
        &default_sensors_target_selector_config(),
        &PinnedSelector {
            sources: &["combat-lock", "objective-destroy", "radar-contacts"],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(detectable) > 0 and candidate_fact(hostile) > 0",
            params: &[
                ("combat_lock_weight", 1000.0),
                ("objective_weight", 100.0),
                ("radar_weight", 1.0),
            ],
            score: &[
                ("candidate_fact(source_combat_lock) > 0", 1000.0),
                ("candidate_fact(source_objective) > 0", 100.0),
                ("candidate_fact(source_radar) > 0", 1.0),
            ],
        },
    );
}

/// Tactical: the densest of the five. Five additive source weights, a
/// four-branch eligibility guard, and the ONLY non-zero switch margin.
///
/// `combat-lock` is deliberately ABSENT from the sources: it is Tactical's own
/// authoritative output, so unioning it would be circular. The ship's current
/// lock reaches the ranking as an internal `source_retained` retention
/// candidate the host pushes, which is why `source_retained` appears in the
/// guards but not in the source list — a mismatch that looks like a typo and is
/// not.
///
/// The eligibility guard is the AC3 independent-revalidation rule: `detectable`
/// is required of everything, and beyond that a candidate is engageable either
/// because the host already vetted it (objective order / last attacker /
/// retained lock) OR because it is independently `hostile`. That disjunction is
/// what lets Tactical honour a mission naming a factionless target while
/// refusing a FRIENDLY Sensors designation — the designation carries only
/// `source_sensors_designation`, which is not one of the three vetted markers.
///
/// The weights are not four independent knobs; see
/// `pin_tactical_objective_beats_the_maximum_non_objective_stack`.
#[test]
fn pin_tactical_selector() {
    assert_selector(
        "tactical",
        &default_tactical_target_selector_config(),
        &PinnedSelector {
            sources: &[
                "sensors-designation",
                "objective-destroy",
                "last-attacker",
                "radar-contacts",
            ],
            horizon: 1.0e9,
            // The only non-zero margin of the five, and load-bearing: the
            // dominance invariant is stated against `objective − margin`.
            switch_margin: 50.0,
            eligibility:
                "candidate_fact(detectable) > 0 and (candidate_fact(source_objective) > 0 \
                          or candidate_fact(source_last_attacker) > 0 \
                          or candidate_fact(source_retained) > 0 \
                          or candidate_fact(hostile) > 0)",
            params: &[
                ("objective_weight", 1000.0),
                ("sensors_designation_weight", 500.0),
                ("retained_weight", 200.0),
                ("last_attacker_weight", 100.0),
                ("radar_weight", 1.0),
            ],
            score: &[
                ("candidate_fact(source_objective) > 0", 1000.0),
                ("candidate_fact(source_sensors_designation) > 0", 500.0),
                ("candidate_fact(source_retained) > 0", 200.0),
                ("candidate_fact(source_last_attacker) > 0", 100.0),
                ("candidate_fact(source_radar) > 0", 1.0),
            ],
        },
    );
}

/// Navigation: two sources, one of which is deliberately inert by default.
///
/// The eligibility guard admits only `reachable` candidates, and ONLY the
/// objective source marks its resolved destination reachable — so under the
/// canonical policy the AI waypoint is driven by objectives alone, reproducing
/// the retired contract. `chart-contacts` is surfaced so an author can widen
/// eligibility to admit them without touching Rust; by default a chart contact
/// can only ENRICH a coincident objective destination (see
/// `pin_navigation_chart_contacts_enrich_but_never_steer`).
#[test]
fn pin_navigation_selector() {
    assert_selector(
        "navigation",
        &default_navigation_target_selector_config(),
        &PinnedSelector {
            sources: &["navigation-objectives", "chart-contacts"],
            horizon: 1.0e9,
            switch_margin: 0.0,
            eligibility: "candidate_fact(reachable) > 0",
            params: &[("objective_weight", 100.0), ("chart_contact_weight", 1.0)],
            score: &[
                ("candidate_fact(source_nav_objective) > 0", 100.0),
                ("candidate_fact(source_chart_contact) > 0", 1.0),
            ],
        },
    );
}

/// Repair: SIX score terms across two ladders, and the eligibility guard that
/// makes N free teams pick N distinct stations.
///
/// The tier ladder is three terms guarded `tier_ordinal >= 1 | 2 | 3`, each
/// worth the full `tier_weight`, so a station accumulates one step per damage
/// tier reached. The deficit ladder is three terms guarded on
/// `damage_fraction >= param(deficit_band_low | mid | high)`.
///
/// A LADDER rather than a multiplier because the predicate grammar has no
/// arithmetic: an authored score term contributes a fixed weight when its
/// boolean guard fires, so a continuous reading can only enter the ranking as a
/// series of thresholds. The retired comparator sorted by
/// `(tier desc, deficit desc)`; the ladders reproduce that ordering additively
/// (see `pin_repair_one_tier_step_beats_the_whole_deficit_ladder`).
///
/// The bands sit at 0.80 / 0.90 / 0.95 — inside the *urgent* range, NOT at the
/// `DamageTier` thresholds. Because tier strictly dominates deficit, the
/// deficit ladder only ever discriminates WITHIN one tier; bands placed at the
/// tier boundaries would all fire together for every Disabled station and
/// discriminate nothing. Do not "helpfully" realign them.
///
/// `tier_ordinal` is the `DamageTier` discriminant (Operational 0, Damaged 1,
/// Disabled 2, Destroyed 3) — a structural enum ordinal, which is why the `> 0`
/// and `< 3` bounds are literals rather than params: Destroyed is excluded
/// because a repair team alone cannot lift the latch.
#[test]
fn pin_repair_selector() {
    assert_selector(
        "repair",
        &default_repair_target_selector_config(),
        &PinnedSelector {
            sources: &["damaged-stations", "core-bucket"],
            horizon: 1.0e9,
            // 0: Repair's retained pick is the authoritative `TeamSlot` and only
            // Idle teams are dispatched, so there is no AI-side hysteresis.
            switch_margin: 0.0,
            eligibility: "candidate_fact(source_repair_request) > 0 \
                          and candidate_fact(assigned) < 1 \
                          and candidate_fact(tier_ordinal) > 0 \
                          and candidate_fact(tier_ordinal) < 3",
            params: &[
                ("tier_weight", 1000.0),
                ("deficit_weight", 100.0),
                ("deficit_band_low", 0.80),
                ("deficit_band_mid", 0.90),
                ("deficit_band_high", 0.95),
            ],
            score: &[
                ("candidate_fact(tier_ordinal) >= 1", 1000.0),
                ("candidate_fact(tier_ordinal) >= 2", 1000.0),
                ("candidate_fact(tier_ordinal) >= 3", 1000.0),
                (
                    "candidate_fact(damage_fraction) >= param(deficit_band_low)",
                    100.0,
                ),
                (
                    "candidate_fact(damage_fraction) >= param(deficit_band_mid)",
                    100.0,
                ),
                (
                    "candidate_fact(damage_fraction) >= param(deficit_band_high)",
                    100.0,
                ),
            ],
        },
    );
}

/// Comms hail: FIVE eligibility clauses — the most gated of the set — and a
/// three-rung band ladder over objective utility.
///
/// This is the only selector whose eligibility reads a `self_fact(...)`:
/// `comms_available`, off `EntitySystemHull`, so a Disabled or Destroyed Comms
/// system stops the ship hailing at all. Every other clause is candidate-side.
///
/// `has_open_hail_thread < 1` is the anti-respam gate, and it is TERMINATING:
/// a hail arms it even when it fires no `on_hailed` template, so a standing
/// directive cannot re-emit every tick. It re-arms on a human `ClearComms` or
/// on the target ceasing to be a live candidate — the second is what an
/// unmanned ship relies on.
///
/// The bands sit at 25 / 45 / 75 because that is where the shipped population
/// actually is: authored `base_priority` values are 20 / 30 / 35 / 40 / 45 / 50
/// / 80 / 100, which these thresholds split four ways (0, 1, 2, 3 rungs). Bands
/// at 100/200/300 would fire for nothing and bands at 1/2/3 for everything —
/// either way every hail would tie and the "ranking" would collapse onto the
/// smallest-UUID tie-break.
#[test]
fn pin_comms_hail_selector() {
    assert_selector(
        "comms_hail",
        &default_comms_target_selector_config(),
        &PinnedSelector {
            sources: &["hail-objectives", "comms-contacts"],
            horizon: 1.0e9,
            // 0, and the host passes `current: None`: a hail is a ONE-SHOT
            // event, not a retained target, so there is nothing to retain.
            switch_margin: 0.0,
            eligibility: "candidate_fact(source_hail_objective) > 0 \
                          and candidate_fact(in_range) > 0 \
                          and candidate_fact(objective_score) > 0 \
                          and candidate_fact(has_open_hail_thread) < 1 \
                          and self_fact(comms_available) > 0",
            params: &[
                ("score_band_weight", 100.0),
                ("score_band_low", 25.0),
                ("score_band_mid", 45.0),
                ("score_band_high", 75.0),
            ],
            score: &[
                (
                    "candidate_fact(objective_score) >= param(score_band_low)",
                    100.0,
                ),
                (
                    "candidate_fact(objective_score) >= param(score_band_mid)",
                    100.0,
                ),
                (
                    "candidate_fact(objective_score) >= param(score_band_high)",
                    100.0,
                ),
            ],
        },
    );
}

/// Only Repair and Comms actually REFERENCE a declared param from a guard.
///
/// Thirteen of the eighteen declared selector params are read by nothing: the
/// score terms carry literal `weight:` fields, so retuning
/// `param.objective_weight` on Sensors, Tactical or Navigation changes no
/// behaviour whatsoever. `validate_fine_system_ai_selector` rejects a
/// `param(...)` reference to an UNDECLARED parameter, but nothing rejects an
/// UNREFERENCED declaration — so this passes content validation silently.
///
/// Pinned rather than fixed, because it is a live trap for #885: an author
/// transcribing these blocks will carry the weight params across believing they
/// are levers. They are documentation. Only the band thresholds
/// (`deficit_band_*`, `score_band_*`) are wired to anything.
#[test]
fn pin_which_selector_params_are_actually_referenced_by_a_guard() {
    let referenced: Vec<(&str, Vec<String>)> = all_selectors()
        .iter()
        .map(|(name, _, _, cfg)| {
            let selector = decoded_selector(cfg.clone());
            let mut refs = Vec::new();
            selector.eligibility.referenced_params(&mut refs);
            for term in &selector.score {
                term.when.referenced_params(&mut refs);
            }
            refs.sort_unstable();
            refs.dedup();
            (*name, refs)
        })
        .collect();

    let as_slices: Vec<(&str, Vec<&str>)> = referenced
        .iter()
        .map(|(n, r)| (*n, r.iter().map(String::as_str).collect()))
        .collect();

    assert_eq!(
        as_slices,
        vec![
            // Three params declared, ZERO referenced.
            ("sensors", vec![]),
            // Five params declared, ZERO referenced.
            ("tactical", vec![]),
            // Two params declared, ZERO referenced.
            ("navigation", vec![]),
            // Five declared, THREE referenced: `tier_weight` and
            // `deficit_weight` are inert.
            (
                "repair",
                vec!["deficit_band_high", "deficit_band_low", "deficit_band_mid"]
            ),
            // Four declared, THREE referenced: `score_band_weight` is inert.
            (
                "comms_hail",
                vec!["score_band_high", "score_band_low", "score_band_mid"]
            ),
        ],
        "Which params a guard can actually read is part of the baseline. If a \
         param moves from inert to referenced (or back), the meaning of \
         retuning it changed, and #885's authored blocks must follow."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Decode pins — which typed verb each channel yields, and its payload
// ─────────────────────────────────────────────────────────────────────────────

/// The verb string on every synthesised rule decodes to the expected typed
/// `AiPolicyVerb`, payload and all.
///
/// The verb strings are validated against a closed set at load, but a
/// well-formed WRONG verb is not caught by anything — so the string→variant
/// mapping is pinned here rather than assumed.
#[test]
fn pin_decoded_verbs() {
    // Captain — the only boolean-payload verb, and the only default that emits
    // two different payloads from one channel.
    let captain = decoded(default_captain_ai_config());
    assert_eq!(
        captain.rules[0].verb,
        AiPolicyVerb::SetRedAlert(true),
        "captain rule[0] (in-combat) raises Red Alert."
    );
    assert_eq!(
        captain.rules[1].verb,
        AiPolicyVerb::SetRedAlert(false),
        "captain rule[1] (fallback) stands Red Alert down."
    );

    // Comms — the response INDEX rides the verb; WHICH message is answered
    // comes from host context.
    assert_eq!(
        decoded(default_comms_response_ai_config()).rules[0].verb,
        AiPolicyVerb::RespondToMessage(0),
        "comms_response answers with the FIRST response, reproducing the retired stub."
    );

    // Power — the magnitude rides the verb: 3 elevated, 2 baseline.
    let power = decoded(default_power_ai_config());
    let levels: Vec<AiPolicyVerb> = power.rules.iter().map(|r| r.verb.clone()).collect();
    assert_eq!(
        levels,
        vec![
            AiPolicyVerb::SetPowerGroupAllocation(3),
            AiPolicyVerb::SetPowerGroupAllocation(2),
            AiPolicyVerb::SetPowerGroupAllocation(3),
            AiPolicyVerb::SetPowerGroupAllocation(2),
        ],
        "power: helm-elevate, helm-baseline, weapons-elevate, weapons-baseline."
    );

    // Everything else is value-less: the magnitude lives in a host-seeded fact
    // or the host context, never in the policy.
    let modal_cases: Vec<(&str, AiPolicyVerb, AiPolicy)> = vec![
        (
            "engines",
            AiPolicyVerb::ActuateDesiredTravel,
            decoded(default_engines_ai_config()),
        ),
        (
            "steering",
            AiPolicyVerb::ActuateDesiredFacing,
            decoded(default_steering_ai_config()),
        ),
        (
            "lateral",
            AiPolicyVerb::ActuateLateralThrust,
            decoded(default_lateral_ai_config()),
        ),
        (
            "vertical",
            AiPolicyVerb::ActuateVerticalThrust,
            decoded(default_vertical_ai_config()),
        ),
        (
            "impulse",
            AiPolicyVerb::EngageImpulse,
            decoded(default_impulse_ai_config()),
        ),
        (
            "phaser_bank",
            AiPolicyVerb::FirePhaser,
            decoded(default_phaser_bank_ai_config()),
        ),
        (
            "blaster_bank",
            AiPolicyVerb::FireBlaster,
            decoded(default_blaster_bank_ai_config()),
        ),
        (
            "torpedo_magazine",
            AiPolicyVerb::GrantTorpedoRound,
            decoded(default_torpedo_magazine_ai_config()),
        ),
    ];
    for (name, want, policy) in &modal_cases {
        assert_eq!(
            &policy.rules[0].verb, want,
            "{name}: the single default rule decodes to this value-less mode verb."
        );
    }

    let tube = decoded(default_torpedo_tube_ai_config());
    assert_eq!(tube.rules[0].verb, AiPolicyVerb::LoadTorpedo);
    assert_eq!(tube.rules[1].verb, AiPolicyVerb::LaunchTorpedo);

    let shields = decoded(default_shields_focus_ai_config());
    assert_eq!(shields.rules[0].verb, AiPolicyVerb::FocusShieldArc);
    assert_eq!(
        shields.rules[1].verb,
        AiPolicyVerb::FocusShieldArc,
        "shields_focus: BOTH rules emit the same value-less verb — which of the four \
         arcs gets focused is decided by the retained ranking kernel, not by the policy."
    );
}

/// Params decode into the runtime `AiParams` bag under the same names, so a
/// `param(...)` reference in a guard resolves.
///
/// A param that failed to survive decoding would make its comparison read false
/// for ever — the same silent-death mode as an unseeded fact.
#[test]
fn pin_params_survive_decoding() {
    let power = decoded(default_power_ai_config());
    for (name, want) in [
        ("thrust_threshold", 0.7_f64),
        ("min_reserve_helm", 50.0),
        ("min_reserve_weapons", 10.0),
        ("min_reserve_baseline", 0.0),
    ] {
        let got = power
            .params
            .get(name)
            .unwrap_or_else(|| panic!("power: `param({name})` must exist after decoding"));
        assert!(
            (got - want).abs() < 1e-6,
            "power: `param({name})` decodes to {want}, got {got}"
        );
    }

    let captain = decoded(default_captain_ai_config());
    assert!(
        captain
            .params
            .get("combat_window_secs")
            .is_some_and(|v| (v - 10.0).abs() < 1e-6),
        "captain: `param(combat_window_secs)` decodes to 10.0 seconds."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Guard truth tables — every guard fires AND can read false
// ─────────────────────────────────────────────────────────────────────────────

/// Captain: in combat within the window ⇒ raise; outside it ⇒ stand down; NO
/// combat history at all ⇒ stand down.
///
/// The absent-fact case is the interesting one. `secs_since_combat` is absent
/// when the ship has never been in combat, and an absent fact makes every
/// comparison against it false — so the priority-10 rule loses and the
/// unconditional fallback correctly stands the alert down.
#[test]
fn pin_captain_guard_truth_table() {
    let cfg = default_captain_ai_config();
    assert_eq!(
        resolve(&cfg, "red_alert", &facts(&[("secs_since_combat", 3.0)])),
        Some(AiPolicyVerb::SetRedAlert(true)),
        "3s since combat is inside the 10s window ⇒ Red Alert raised."
    );
    assert_eq!(
        resolve(&cfg, "red_alert", &facts(&[("secs_since_combat", 10.0)])),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "the comparison is STRICTLY less-than, so exactly 10s is already outside \
         the window ⇒ stand down."
    );
    assert_eq!(
        resolve(&cfg, "red_alert", &facts(&[("secs_since_combat", 30.0)])),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "30s since combat is outside the 10s window ⇒ stand down."
    );
    assert_eq!(
        resolve(&cfg, "red_alert", &facts(&[])),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "never been in combat ⇒ the fact is ABSENT ⇒ the guard reads false ⇒ the \
         unconditional fallback stands the alert down. The default is never silent."
    );
}

/// Comms response: both clauses must hold, and either one alone can silence the
/// policy.
///
/// `None` here means the ship simply does not answer this tick — distinct from
/// answering with a different index. Both fact names are seeded by
/// `seed_comms_response_facts`, so both guards are live rather than dead.
#[test]
fn pin_comms_response_guard_truth_table() {
    let cfg = default_comms_response_ai_config();
    assert_eq!(
        resolve(
            &cfg,
            "comms_respond",
            &facts(&[("comms_available", 1.0), ("sender_in_range", 1.0)])
        ),
        Some(AiPolicyVerb::RespondToMessage(0)),
        "Comms usable AND sender in range ⇒ answer with response 0."
    );
    assert_eq!(
        resolve(
            &cfg,
            "comms_respond",
            &facts(&[("comms_available", 0.0), ("sender_in_range", 1.0)])
        ),
        None,
        "a Disabled/Destroyed Comms system stops the ship ANSWERING, not just hailing."
    );
    assert_eq!(
        resolve(
            &cfg,
            "comms_respond",
            &facts(&[("comms_available", 1.0), ("sender_in_range", 0.0)])
        ),
        None,
        "the router refuses a response whose sender has left comms range; without \
         this clause the AI would re-emit the doomed response every tick for ever."
    );
    assert_eq!(
        resolve(&cfg, "comms_respond", &facts(&[])),
        None,
        "both facts absent ⇒ no answer. This is the fail-SAFE direction."
    );
}

/// Shields focus: the priority-10 damage rule fires above the threshold and not
/// below it — but the priority-0 fallback is unconditional, so the CHANNEL never
/// resolves to `None` either way.
///
/// That is the pin, and it is deliberately blunt: because both rules emit the
/// same value-less `focus_shield_arc` verb, the threshold has NO effect on what
/// the host does with the resolved verb. The arc-ranking kernel runs every tick
/// regardless — which is exactly what "omitting the block reproduces today's
/// decisions bit-for-bit" means. #885 must keep that property.
#[test]
fn pin_shields_focus_guard_truth_table() {
    let cfg = default_shields_focus_ai_config();
    let policy = cfg.to_policy().expect("decodes");

    let concentrated = facts(&[("recent_damage_pct_max", 80.0)]);
    let diffuse = facts(&[("recent_damage_pct_max", 20.0)]);

    // The guard itself is live in both directions.
    assert!(
        policy.rules[0]
            .when
            .evaluate_with(&concentrated, &policy.params, &[]),
        "80% of windowed damage on one arc is at or above the 50% threshold ⇒ the \
         priority-10 damage rule FIRES."
    );
    assert!(
        !policy.rules[0]
            .when
            .evaluate_with(&diffuse, &policy.params, &[]),
        "20% is below the 50% threshold ⇒ the priority-10 damage rule reads FALSE. \
         The fact name must be one the host seeds or this could never be false."
    );

    // …but the resolved channel is the same either way, and never silent.
    assert_eq!(
        resolve(&cfg, "shield_focus", &concentrated),
        Some(AiPolicyVerb::FocusShieldArc),
        "concentrated damage ⇒ act."
    );
    assert_eq!(
        resolve(&cfg, "shield_focus", &diffuse),
        Some(AiPolicyVerb::FocusShieldArc),
        "diffuse damage ⇒ STILL act, via the unconditional priority-0 fallback."
    );
    assert_eq!(
        resolve(&cfg, "shield_focus", &facts(&[])),
        Some(AiPolicyVerb::FocusShieldArc),
        "no damage facts at all ⇒ still act. The kernel runs every tick, which is \
         the pre-#783 baseline this default exists to preserve."
    );
}

/// Power: the elevate rules need BOTH their trigger and their battery reserve;
/// the baseline rules hold the line whenever a battery reading exists at all.
///
/// The brownout property is structural rather than a separate branch: an elevate
/// guard cannot fire below its reserve, so allocation never rises when the
/// battery cannot sustain it.
#[test]
fn pin_power_guard_truth_table() {
    let cfg = default_power_ai_config();

    // ── helm ────────────────────────────────────────────────────────────────
    assert_eq!(
        resolve(
            &cfg,
            "helm",
            &facts(&[("thrust", 0.9), ("battery_pct", 80.0)])
        ),
        Some(AiPolicyVerb::SetPowerGroupAllocation(3)),
        "sustained thrust (0.9 ≥ 0.7) AND battery above the 50% helm reserve ⇒ elevate."
    );
    assert_eq!(
        resolve(
            &cfg,
            "helm",
            &facts(&[("thrust", 0.9), ("battery_pct", 30.0)])
        ),
        Some(AiPolicyVerb::SetPowerGroupAllocation(2)),
        "same thrust, battery BELOW the 50% reserve ⇒ the elevate guard reads false \
         and helm holds baseline. This is the brownout guard, and it is the reserve \
         param that enforces it — not a global emergency branch."
    );
    assert_eq!(
        resolve(
            &cfg,
            "helm",
            &facts(&[("thrust", 0.2), ("battery_pct", 80.0)])
        ),
        Some(AiPolicyVerb::SetPowerGroupAllocation(2)),
        "thrust below the 0.7 threshold ⇒ baseline, however full the battery."
    );

    // ── weapons ─────────────────────────────────────────────────────────────
    assert_eq!(
        resolve(
            &cfg,
            "weapons",
            &facts(&[("red_alert", 1.0), ("battery_pct", 80.0)])
        ),
        Some(AiPolicyVerb::SetPowerGroupAllocation(3)),
        "red alert AND battery above the 10% weapons reserve ⇒ elevate."
    );
    assert_eq!(
        resolve(
            &cfg,
            "weapons",
            &facts(&[("red_alert", 1.0), ("battery_pct", 5.0)])
        ),
        Some(AiPolicyVerb::SetPowerGroupAllocation(2)),
        "red alert but battery below the 10% weapons reserve ⇒ baseline."
    );
    assert_eq!(
        resolve(
            &cfg,
            "weapons",
            &facts(&[("red_alert", 0.0), ("battery_pct", 80.0)])
        ),
        Some(AiPolicyVerb::SetPowerGroupAllocation(2)),
        "no red alert ⇒ baseline."
    );

    // ── the absent-battery edge, on both channels ───────────────────────────
    for channel in ["helm", "weapons"] {
        assert_eq!(
            resolve(&cfg, channel, &facts(&[])),
            None,
            "{channel}: with NO `battery_pct` reading even the baseline rule's guard \
             reads false, so the channel resolves to `None` and the group HOLDS its \
             seeded level. The zero-valued `min_reserve_baseline` does not make the \
             baseline rule unconditional — it still requires the fact to be PRESENT."
        );
    }

    // ── a group the default never names ─────────────────────────────────────
    assert_eq!(
        resolve(
            &cfg,
            "sensors",
            &facts(&[("battery_pct", 80.0), ("red_alert", 1.0), ("thrust", 1.0)])
        ),
        None,
        "`sensors` is not named by the default, so it resolves to `None` and holds \
         whatever level the reactor seeded — no matter how favourable the facts."
    );
}

/// The unconditional defaults fire on an EMPTY fact snapshot.
///
/// This is what "baseline preserving" means for them: the pre-policy hosts
/// actuated every tick with no gate at all, so a `when = "true"` guard must
/// resolve even before any fact is seeded. It is also the #779 empty-facts
/// regression guard — a guard that needed a fact here would silently disable
/// the axis on the first tick.
#[test]
fn pin_unconditional_defaults_fire_with_no_facts() {
    let cases: Vec<(&str, &str, AiPolicyVerb, FineSystemAiConfigToml)> = vec![
        (
            "engines",
            "longitudinal",
            AiPolicyVerb::ActuateDesiredTravel,
            default_engines_ai_config(),
        ),
        (
            "steering",
            "yaw",
            AiPolicyVerb::ActuateDesiredFacing,
            default_steering_ai_config(),
        ),
        (
            "lateral",
            "lateral",
            AiPolicyVerb::ActuateLateralThrust,
            default_lateral_ai_config(),
        ),
        (
            "vertical",
            "vertical",
            AiPolicyVerb::ActuateVerticalThrust,
            default_vertical_ai_config(),
        ),
        (
            "impulse",
            "impulse",
            AiPolicyVerb::EngageImpulse,
            default_impulse_ai_config(),
        ),
        (
            "phaser_bank",
            "phaser_fire",
            AiPolicyVerb::FirePhaser,
            default_phaser_bank_ai_config(),
        ),
        (
            "blaster_bank",
            "blaster_fire",
            AiPolicyVerb::FireBlaster,
            default_blaster_bank_ai_config(),
        ),
        (
            "torpedo_tube",
            "torpedo_load",
            AiPolicyVerb::LoadTorpedo,
            default_torpedo_tube_ai_config(),
        ),
        (
            "torpedo_tube",
            "torpedo_launch",
            AiPolicyVerb::LaunchTorpedo,
            default_torpedo_tube_ai_config(),
        ),
        (
            "torpedo_magazine",
            "torpedo_magazine_grant",
            AiPolicyVerb::GrantTorpedoRound,
            default_torpedo_magazine_ai_config(),
        ),
    ];
    for (name, channel, want, cfg) in &cases {
        assert_eq!(
            resolve(cfg, channel, &facts(&[])).as_ref(),
            Some(want),
            "{name}/{channel}: an unconditional default resolves on an EMPTY fact \
             snapshot — the host still owns every readiness gate (cooldown, range, \
             arc, availability); the policy only says 'permitted'."
        );
    }
}

/// An unknown channel resolves to `None` on every synthesiser.
///
/// Pinned because it is the failure mode of a mistyped channel name in the
/// authored replacement: nothing rejects it at the policy level, the system just
/// goes permanently silent.
#[test]
fn pin_unknown_channel_resolves_to_nothing() {
    for (name, _, cfg) in all_synthesisers() {
        assert_eq!(
            resolve(&cfg, "not_a_channel", &facts(&[])),
            None,
            "{name}: an unrecognised channel resolves to `None`. A mistyped channel in \
             #885's authored TOML fails exactly this quietly."
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Selector INVARIANT pins — the ORDERING, not the numbers
// ─────────────────────────────────────────────────────────────────────────────
//
// For the selectors the specification is relational: "an explicit mission
// objective always beats everything else", "one damage tier always beats the
// whole deficit ladder". Those hold because of how the weights sit RELATIVE to
// one another, and a reweight can leave every individual number looking
// perfectly plausible while inverting the order.
//
// So these pins run the real `TargetSelector::select` over hand-built
// candidates and assert who wins. `config.rs` carries the arithmetic form of
// the same invariants in `const {}` blocks; these are the behavioural form, and
// they are the ones that fail when the arithmetic is "corrected" to match a
// broken intent.

/// The #777 additive-stacking invariant, pinned as an ORDERING.
///
/// The selector SUMS weights and one entity commonly carries several source
/// markers at once — the ship's current lock is often also its Sensors
/// designation, and may also be the last attacker and the nearest hostile. A
/// naively-large `retained` weight would let that stack overtake a distinct
/// in-range `objective`, and the ship would refuse to retarget onto its own
/// explicit mission objective.
///
/// So the maximum achievable non-objective stack must lose to `objective`:
///
/// ```text
///   sensors_designation + retained + last_attacker + radar
///     = 500 + 200 + 100 + 1 = 801  <  1000 − 50 = 950  =  objective − margin
/// ```
///
/// Both halves are asserted. The `current: None` case pins that objective wins
/// the raw ranking; the `current: Some(stacked)` case pins that it ALSO
/// overcomes hysteresis retention, which is the half the arithmetic form exists
/// to guarantee and the half a reweight is most likely to break.
///
/// The objective candidate deliberately carries the LARGER uuid and
/// `hostile = 0`: larger, so the smallest-uuid tie-break cannot be what makes it
/// win; not hostile, so the test simultaneously proves the eligibility guard
/// admits a factionless mission target on `source_objective` alone.
#[test]
fn pin_tactical_objective_beats_the_maximum_non_objective_stack() {
    let sel = decoded_selector(default_tactical_target_selector_config());

    // Everything a candidate can be at once, short of being the objective.
    let stacked = candidate(
        "aaa-everything-else",
        &[
            ("detectable", 1.0),
            ("hostile", 1.0),
            ("source_sensors_designation", 1.0),
            ("source_retained", 1.0),
            ("source_last_attacker", 1.0),
            ("source_radar", 1.0),
        ],
    );
    // The mission objective: one marker, no faction hostility, worse uuid.
    let objective = candidate(
        "zzz-mission-objective",
        &[
            ("detectable", 1.0),
            ("hostile", 0.0),
            ("source_objective", 1.0),
        ],
    );
    let pool = vec![stacked, objective];
    let ctx = self_ctx(&[]);

    assert_eq!(
        pick(&sel, &ctx, &pool, None).as_deref(),
        Some("zzz-mission-objective"),
        "an explicitly named Destroy objective outranks a candidate carrying EVERY \
         other source marker at once. If this fails, the weights were retuned into a \
         set where stacking beats mission orders — the exact #777 defect."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool, Some("aaa-everything-else")).as_deref(),
        Some("zzz-mission-objective"),
        "…and it still wins when the stacked candidate is the INCUMBENT, i.e. it \
         clears the 50-point switch margin too. This is the assertion that catches a \
         reweight where each individual weight still looks plausible: raise \
         `retained` to 250 and the raw ranking above still passes while this one \
         fails, and the ship would sit on its old lock refusing its mission."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool[..1], None).as_deref(),
        Some("aaa-everything-else"),
        "control: with the objective removed, the stacked candidate IS selected — so \
         the assertions above are about ranking, not about it being ineligible."
    );
}

/// Retention outranks a fresh attacker: an established engagement is not broken
/// off by whoever just shot at us.
///
/// This is the retired tier-2 > tier-3 ordering (`retained` 200 >
/// `last_attacker` 100), and it is the OTHER side of the invariant — the
/// bounded `retained` contribution has to stay large enough to be meaningful
/// while staying small enough to lose to `objective`. Asserted as an ordering
/// so a reweight cannot satisfy one side by sacrificing the other.
///
/// The attacker gets the smaller uuid so the tie-break cannot be the reason.
#[test]
fn pin_tactical_retention_outranks_a_fresh_attacker() {
    let sel = decoded_selector(default_tactical_target_selector_config());
    let pool = vec![
        candidate(
            "aaa-fresh-attacker",
            &[
                ("detectable", 1.0),
                ("hostile", 1.0),
                ("source_last_attacker", 1.0),
            ],
        ),
        candidate(
            "zzz-established-lock",
            &[
                ("detectable", 1.0),
                ("hostile", 1.0),
                ("source_retained", 1.0),
            ],
        ),
    ];
    let ctx = self_ctx(&[]);
    assert_eq!(
        pick(&sel, &ctx, &pool, Some("zzz-established-lock")).as_deref(),
        Some("zzz-established-lock"),
        "the ship stays on the target it is already engaging rather than wheeling \
         onto a fresh attacker."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool, None).as_deref(),
        Some("zzz-established-lock"),
        "…and it outranks the attacker on raw score too (200 > 100), so the ordering \
         does not depend on hysteresis to hold."
    );
}

/// AC3 independent revalidation: a FRIENDLY Sensors designation is dropped, not
/// copied — but the same friendly entity IS engageable once a mission names it.
///
/// The Sensors designation is advisory intelligence, so the eligibility guard
/// requires it to be independently `hostile`; the three host-vetted markers
/// (`source_objective` / `source_last_attacker` / `source_retained`) bypass that
/// check. Pinned as behaviour because the guard is a four-branch disjunction and
/// dropping the wrong branch changes which of these two cases flips.
#[test]
fn pin_tactical_drops_a_friendly_sensors_designation() {
    let sel = decoded_selector(default_tactical_target_selector_config());
    let ctx = self_ctx(&[]);

    let friendly_designation = vec![candidate(
        "ally",
        &[
            ("detectable", 1.0),
            ("hostile", 0.0),
            ("source_sensors_designation", 1.0),
        ],
    )];
    assert_eq!(
        pick(&sel, &ctx, &friendly_designation, None),
        None,
        "Tactical refuses to open fire on a friendly just because Sensors pointed at \
         it. The designation is advice; hostility is revalidated independently."
    );

    let hostile_designation = vec![candidate(
        "ally",
        &[
            ("detectable", 1.0),
            ("hostile", 1.0),
            ("source_sensors_designation", 1.0),
        ],
    )];
    assert_eq!(
        pick(&sel, &ctx, &hostile_designation, None).as_deref(),
        Some("ally"),
        "…while a HOSTILE designation is engaged. This is the branch proving the \
         `hostile` clause can read true as well as false."
    );

    let ordered_friendly = vec![candidate(
        "ally",
        &[
            ("detectable", 1.0),
            ("hostile", 0.0),
            ("source_sensors_designation", 1.0),
            ("source_objective", 1.0),
        ],
    )];
    assert_eq!(
        pick(&sel, &ctx, &ordered_friendly, None).as_deref(),
        Some("ally"),
        "…and an explicit mission order overrides faction entirely: a named Destroy \
         target is engaged whatever its colours. That asymmetry is deliberate."
    );

    let hidden = vec![candidate(
        "cloaked",
        &[
            ("detectable", 0.0),
            ("hostile", 1.0),
            ("source_objective", 1.0),
        ],
    )];
    assert_eq!(
        pick(&sel, &ctx, &hidden, None),
        None,
        "`detectable` is a precondition on EVERY branch — it sits outside the \
         disjunction, so not even a mission order can lock something unseen."
    );
}

/// The #785 Repair invariant, pinned as an ORDERING: one damage-tier step beats
/// the ENTIRE deficit ladder.
///
/// The retired comparator sorted `(tier desc, deficit desc)` — deficit was only
/// ever the tie-break. Reproducing that additively requires
/// `3 × deficit_weight < tier_weight` (300 < 1000), which is asserted here by
/// making a barely-damaged Disabled station beat an almost-dead Damaged one:
///
/// ```text
///   Disabled @ 0.30 damage = 2 tier steps + 0 bands = 2000
///   Damaged  @ 0.99 damage = 1 tier step  + 3 bands = 1300
/// ```
///
/// A reweight that raised `deficit_weight` to 400 would invert this while each
/// number still looked reasonable, and the AI would start sending teams to
/// nearly-dead minor stations ahead of disabled critical ones.
#[test]
fn pin_repair_one_tier_step_beats_the_whole_deficit_ladder() {
    let sel = decoded_selector(default_repair_target_selector_config());
    let ctx = self_ctx(&[]);
    let pool = vec![
        // Damaged (tier 1) but almost destroyed — every deficit band fires.
        candidate(
            "aaa-damaged-but-nearly-dead",
            &[
                ("source_repair_request", 1.0),
                ("assigned", 0.0),
                ("tier_ordinal", 1.0),
                ("damage_fraction", 0.99),
            ],
        ),
        // Disabled (tier 2) and lightly hurt — not one deficit band fires.
        candidate(
            "zzz-disabled-but-lightly-hurt",
            &[
                ("source_repair_request", 1.0),
                ("assigned", 0.0),
                ("tier_ordinal", 2.0),
                ("damage_fraction", 0.30),
            ],
        ),
    ];
    assert_eq!(
        pick(&sel, &ctx, &pool, None).as_deref(),
        Some("zzz-disabled-but-lightly-hurt"),
        "tier STRICTLY dominates deficit: a Disabled station outranks a Damaged one \
         no matter how much worse the Damaged one's deficit is. Deficit is the \
         tie-break, exactly as in the retired comparator — and the winner here also \
         carries the larger uuid, so the tie-break is not what decided it."
    );
}

/// …and the deficit ladder still does its job WITHIN a tier, which is the only
/// place it was ever meant to discriminate.
///
/// Two Disabled stations at 0.96 (three bands) and 0.81 (one band) score 2300 vs
/// 2100, so the nearly-dead one goes first. Two inside the SAME band tie and
/// fall through to the documented smallest-id tie-break, which is deterministic
/// — that determinism is the AC4 property, not an accident.
#[test]
fn pin_repair_deficit_ladder_discriminates_within_a_tier() {
    let sel = decoded_selector(default_repair_target_selector_config());
    let ctx = self_ctx(&[]);
    let disabled = |uuid: &str, damage: f64| {
        candidate(
            uuid,
            &[
                ("source_repair_request", 1.0),
                ("assigned", 0.0),
                ("tier_ordinal", 2.0),
                ("damage_fraction", damage),
            ],
        )
    };

    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[disabled("aaa-barely", 0.81), disabled("zzz-critical", 0.96)],
            None
        )
        .as_deref(),
        Some("zzz-critical"),
        "within one tier the deeper deficit wins — 3 bands (300) over 1 (100) — and \
         it wins from the WORSE uuid, so this is the ladder and not the tie-break. If \
         the bands were realigned to the DamageTier thresholds this would tie instead."
    );

    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[
                disabled("zzz-same-band", 0.85),
                disabled("aaa-same-band", 0.82)
            ],
            None
        )
        .as_deref(),
        Some("aaa-same-band"),
        "two stations inside the same band score identically and fall through to the \
         smallest-id tie-break. The banding is a deliberately COARSE stand-in for the \
         retired continuous ordering, and this is the resolution it gives up."
    );
}

/// Every clause of the Repair eligibility guard can independently read false.
///
/// This is the #885 fact trap in its most dangerous form: eligibility is not a
/// preference, it is a gate, so a clause that can never be false is dead code
/// and a clause on a misspelled fact silently stops the system repairing
/// anything at all. Each case below removes exactly one qualification from an
/// otherwise-perfect candidate.
#[test]
fn pin_repair_eligibility_clauses_each_gate_independently() {
    let sel = decoded_selector(default_repair_target_selector_config());
    let ctx = self_ctx(&[]);
    let station = |pairs: &[(&str, f64)]| vec![candidate("station", pairs)];

    assert_eq!(
        pick(
            &sel,
            &ctx,
            &station(&[
                ("source_repair_request", 1.0),
                ("assigned", 0.0),
                ("tier_ordinal", 2.0),
                ("damage_fraction", 0.9),
            ]),
            None
        )
        .as_deref(),
        Some("station"),
        "control: a reported, unassigned, Disabled station IS selected."
    );

    assert_eq!(
        pick(
            &sel,
            &ctx,
            &station(&[
                ("source_repair_request", 0.0),
                ("assigned", 0.0),
                ("tier_ordinal", 2.0),
                ("damage_fraction", 0.9),
            ]),
            None
        ),
        None,
        "damage nobody REPORTED is not a candidate. #830 removed the raw hull poll on \
         purpose, so the coordination-delivered request is the only surface."
    );
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &station(&[
                ("source_repair_request", 1.0),
                ("assigned", 1.0),
                ("tier_ordinal", 2.0),
                ("damage_fraction", 0.9),
            ]),
            None
        ),
        None,
        "a station a team is already handling — or that an earlier team was dispatched \
         to THIS SAME TICK — is excluded. This is what makes N free teams pick N \
         DISTINCT stations instead of all piling onto the worst one."
    );
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &station(&[
                ("source_repair_request", 1.0),
                ("assigned", 0.0),
                ("tier_ordinal", 0.0),
                ("damage_fraction", 0.0),
            ]),
            None
        ),
        None,
        "an Operational station (tier 0) has nothing to repair."
    );
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &station(&[
                ("source_repair_request", 1.0),
                ("assigned", 0.0),
                ("tier_ordinal", 3.0),
                ("damage_fraction", 1.0),
            ]),
            None
        ),
        None,
        "a DESTROYED station (tier 3) is excluded — a repair team alone cannot lift \
         the latch, so dispatching one would waste the team indefinitely. Note the \
         asymmetry: this is the worst-damaged candidate possible and it is the one \
         the guard refuses."
    );
    assert_eq!(
        pick(&sel, &ctx, &station(&[]), None),
        None,
        "no facts at all ⇒ every clause reads false ⇒ nothing selected. The \
         fail-SAFE direction: an unseeded fact cannot cause a spurious dispatch."
    );
}

/// The Comms band ladder actually RANKS — the property the band placement
/// exists to buy, and the one that silently disappears if the bands move.
///
/// Four hails at objective utilities drawn from the shipped authoring range
/// score 3 / 2 / 1 / 0 rungs. Note the lowest is still ELIGIBLE: it clears
/// `objective_score > 0` and simply scores nothing, so a background-chatter hail
/// is sent when it is the only one available and is outranked whenever anything
/// else is. Bands at 100/200/300 (all zero rungs) or 1/2/3 (all three rungs)
/// would leave every hail tied and hand the decision to the uuid tie-break,
/// which is the #785 lesson this ladder was written to avoid.
///
/// The uuids run counter to the scores so the tie-break cannot explain any of
/// the four results.
#[test]
fn pin_comms_band_ladder_ranks_hails_by_objective_utility() {
    let sel = decoded_selector(default_comms_target_selector_config());
    // Comms is the only selector reading a self fact; without it nothing hails.
    let ctx = self_ctx(&[("comms_available", 1.0)]);
    let hail = |uuid: &str, score: f64| {
        candidate(
            uuid,
            &[
                ("source_hail_objective", 1.0),
                ("in_range", 1.0),
                ("has_open_hail_thread", 0.0),
                ("objective_score", score),
            ],
        )
    };
    // 100 → 3 rungs, 50 → 2, 30 → 1, 20 → 0.
    let pool = vec![
        hail("aaa-mission-critical", 100.0),
        hail("bbb-priority", 50.0),
        hail("ccc-routine", 30.0),
        hail("zzz-chatter", 20.0),
    ];

    assert_eq!(
        pick(&sel, &ctx, &pool, None).as_deref(),
        Some("aaa-mission-critical"),
        "three rungs (300) beats two (200): the mission-critical hail goes first."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool[1..], None).as_deref(),
        Some("bbb-priority"),
        "two rungs (200) beats one (100)."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool[2..], None).as_deref(),
        Some("ccc-routine"),
        "one rung (100) beats none (0) — even though the loser has the SMALLER \
         objective score and would win a uuid tie-break if the ladder had collapsed."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool[3..], None).as_deref(),
        Some("zzz-chatter"),
        "a zero-rung hail is still ELIGIBLE and is sent when nothing outranks it. \
         Scoring nothing is not the same as being filtered out — only \
         `objective_score > 0` filters, and 20 clears it."
    );
}

/// Every clause of the Comms eligibility guard can independently silence the
/// hail, including the `self_fact` one.
///
/// Five gates, each removed in turn from an otherwise-perfect hail. The
/// `comms_available` case is the one that distinguishes this selector from the
/// other four: it is the only eligibility guard in the set that reads SELF
/// state, so a Disabled Comms system stops the ship hailing at all rather than
/// merely changing whom it hails.
#[test]
fn pin_comms_eligibility_clauses_each_gate_independently() {
    let sel = decoded_selector(default_comms_target_selector_config());
    let live = self_ctx(&[("comms_available", 1.0)]);
    let target = |pairs: &[(&str, f64)]| vec![candidate("contact", pairs)];
    let perfect: &[(&str, f64)] = &[
        ("source_hail_objective", 1.0),
        ("in_range", 1.0),
        ("has_open_hail_thread", 0.0),
        ("objective_score", 50.0),
    ];

    assert_eq!(
        pick(&sel, &live, &target(perfect), None).as_deref(),
        Some("contact"),
        "control: an in-range contact a live Hail directive names, with no thread \
         already open, IS hailed."
    );

    let mut no_directive = perfect.to_vec();
    no_directive[0] = ("source_hail_objective", 0.0);
    assert_eq!(
        pick(&sel, &live, &target(&no_directive), None),
        None,
        "a contact on the comms roster that no Hail directive names is NOT hailed on \
         the ship's own initiative — `comms-contacts` enriches a coincident directive \
         and never independently selects, the same shape as Navigation's chart \
         contacts. An author may widen this; the baseline does not."
    );

    let mut out_of_range = perfect.to_vec();
    out_of_range[1] = ("in_range", 0.0);
    assert_eq!(
        pick(&sel, &live, &target(&out_of_range), None),
        None,
        "out of comms range ⇒ no hail. Defence in depth: `handle_hail` keeps its own \
         hard server-side range check regardless."
    );

    let mut already_hailed = perfect.to_vec();
    already_hailed[2] = ("has_open_hail_thread", 1.0);
    assert_eq!(
        pick(&sel, &live, &target(&already_hailed), None),
        None,
        "the anti-respam gate, and it is TERMINATING: a hail arms it even when it \
         fires no `on_hailed` template, so a standing directive cannot re-emit every \
         tick for ever."
    );

    let mut zero_score = perfect.to_vec();
    zero_score[3] = ("objective_score", 0.0);
    assert_eq!(
        pick(&sel, &live, &target(&zero_score), None),
        None,
        "the zero-gate drop, reproducing the retired `s.score > 0.0` filter."
    );

    assert_eq!(
        pick(
            &sel,
            &self_ctx(&[("comms_available", 0.0)]),
            &target(perfect),
            None
        ),
        None,
        "a Disabled or Destroyed Comms system stops the ship hailing at ALL. This is \
         the only `self_fact(...)` in any of the five eligibility guards, so it is the \
         only one whose failure silences the system rather than re-ranking it."
    );
    assert_eq!(
        pick(&sel, &self_ctx(&[]), &target(perfect), None),
        None,
        "…and an ABSENT `comms_available` reads false just like a zero one, so a \
         host that stopped seeding it would silently mute every ship's hails."
    );
}

/// Sensors ranks strictly by which source surfaced the contact, and a contact
/// surfaced by several sources sums all of them.
///
/// The additive stacking is the same mechanism Tactical has to defend against;
/// on Sensors it is harmless because the tiers are three orders of magnitude
/// apart, and pinning it here records that the gap — not any guard — is what
/// keeps the tiers separated.
#[test]
fn pin_sensors_tier_order_and_additive_stacking() {
    let sel = decoded_selector(default_sensors_target_selector_config());
    let ctx = self_ctx(&[]);
    let contact = |uuid: &str, source: &str| {
        candidate(
            uuid,
            &[("detectable", 1.0), ("hostile", 1.0), (source, 1.0)],
        )
    };
    let pool = vec![
        contact("aaa-lock", "source_combat_lock"),
        contact("bbb-objective", "source_objective"),
        contact("ccc-radar", "source_radar"),
    ];

    assert_eq!(
        pick(&sel, &ctx, &pool, None).as_deref(),
        Some("aaa-lock"),
        "combat lock (1000) ≫ objective (100) ≫ radar (1): Sensors mirrors what \
         Tactical is engaging so the science console shows the ship's actual fight."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool[1..], None).as_deref(),
        Some("bbb-objective"),
        "with no lock, a named Destroy objective is designated."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool[2..], None).as_deref(),
        Some("ccc-radar"),
        "with neither, the nearest hostile radar contact is designated as advisory \
         intelligence for Tactical."
    );

    // Stacking: markers sum, so an objective that is ALSO a radar contact scores
    // 101 and beats a plain objective at 100 — from the worse uuid.
    let stacked = candidate(
        "zzz-objective-and-radar",
        &[
            ("detectable", 1.0),
            ("hostile", 1.0),
            ("source_objective", 1.0),
            ("source_radar", 1.0),
        ],
    );
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[contact("aaa-objective-only", "source_objective"), stacked],
            None
        )
        .as_deref(),
        Some("zzz-objective-and-radar"),
        "source markers are ADDITIVE, not a tier lookup: being surfaced twice scores \
         twice. Harmless here only because 1 ≪ 100 ≪ 1000."
    );
}

/// Sensors has NO hysteresis band — and yet a tie still favours the incumbent.
///
/// `switch_margin = 0` makes the retention test `cur_score >= best - 0.0`, which
/// is satisfied by an exact tie. So on equal scores the CURRENT target is kept
/// rather than the smallest-uuid rule applying, even though this selector is
/// documented as having no hysteresis. Non-obvious, easy to break by "tidying"
/// the retention comparison to a strict `>`, and it is what stops Sensors
/// flapping between two equally-ranked radar contacts every tick.
#[test]
fn pin_sensors_zero_margin_still_retains_an_exact_tie() {
    let sel = decoded_selector(default_sensors_target_selector_config());
    let ctx = self_ctx(&[]);
    let radar = |uuid: &str| {
        candidate(
            uuid,
            &[("detectable", 1.0), ("hostile", 1.0), ("source_radar", 1.0)],
        )
    };
    let pool = vec![radar("aaa-rival"), radar("zzz-incumbent")];

    assert_eq!(
        pick(&sel, &ctx, &pool, Some("zzz-incumbent")).as_deref(),
        Some("zzz-incumbent"),
        "identical scores ⇒ the incumbent is retained, because `switch_margin = 0` \
         still admits an EXACT tie. Without a current target the same pool resolves \
         the other way (below), so this is retention, not the tie-break."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool, None).as_deref(),
        Some("aaa-rival"),
        "with no incumbent the smallest-uuid tie-break decides — deterministically, \
         independent of query order."
    );
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[
                radar("zzz-incumbent"),
                candidate(
                    "aaa-objective",
                    &[
                        ("detectable", 1.0),
                        ("hostile", 1.0),
                        ("source_objective", 1.0),
                    ],
                ),
            ],
            Some("zzz-incumbent")
        )
        .as_deref(),
        Some("aaa-objective"),
        "…but ANY score improvement switches immediately: with a zero margin there is \
         no band in which a better candidate is ignored."
    );
}

/// The Sensors eligibility guard is live in the GRAMMAR but unreachable-false
/// in the HOST — the mirror image of the `fact(...)` trap, pinned deliberately.
///
/// #885 warns that a guard on an unseeded or misspelled fact reads false for
/// ever. The Sensors selector has the opposite defect: its guard is
/// structurally always TRUE in production. The host's only candidate
/// constructor (`ship::sensors::detectable_candidate`) hardcodes
/// `detectable = 1` and `hostile = 1` on every candidate it builds, so no
/// candidate that fails the guard can ever reach the selector. The documented
/// "hidden/friendly drop (AC4)" is enforced upstream by the host's own horizon
/// check and `find_nearest_hostile`, not by this guard.
///
/// Its sibling behaves differently: Tactical's `make_candidate` computes real
/// hostility per candidate, which is why `pin_tactical_drops_a_friendly_sensors_designation`
/// can exercise both directions through the real host contract and this cannot.
///
/// Pinned rather than fixed, and pinned in BOTH halves:
///
///   - the guard discriminates when it is given a failing candidate, so it is
///     not dead code in the predicate grammar;
///   - the exact two-clause form is pinned by `pin_sensors_selector`, so an
///     author who notices the clauses look redundant and drops them fails a
///     test rather than silently changing what the selector is capable of.
///
/// That second half is the point. Dropping the clauses is behaviour-preserving
/// *today*, given what the host feeds it — but it would silently remove the
/// only thing that would start filtering the moment a host learned to surface
/// an undetectable or friendly contact. #885 must make that call deliberately.
#[test]
fn pin_sensors_eligibility_is_live_in_the_grammar_but_unreachable_false_in_the_host() {
    let sel = decoded_selector(default_sensors_target_selector_config());
    let ctx = self_ctx(&[]);

    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[candidate(
                "friendly",
                &[("detectable", 1.0), ("hostile", 0.0), ("source_radar", 1.0)]
            )],
            None
        ),
        None,
        "given a friendly candidate the guard DOES drop it — the clause is real \
         predicate logic, not dead syntax."
    );
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[candidate(
                "cloaked",
                &[("detectable", 0.0), ("hostile", 1.0), ("source_radar", 1.0)]
            )],
            None
        ),
        None,
        "…and likewise an undetectable one."
    );
    assert_eq!(
        pick(&sel, &ctx, &[candidate("nothing_known", &[])], None),
        None,
        "absent facts read false, so an unseeded candidate is dropped — the \
         fail-SAFE direction, and the reason the host must seed both facts on \
         every candidate rather than relying on defaults."
    );

    // The host-reality half: every Sensors candidate is constructed with both
    // facts hardcoded true, so the three cases above cannot occur in production.
    // Asserted through the shape of what the host DOES produce.
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[candidate(
                "as_the_host_builds_it",
                &[("detectable", 1.0), ("hostile", 1.0), ("source_radar", 1.0)]
            )],
            None
        )
        .as_deref(),
        Some("as_the_host_builds_it"),
        "this is the ONLY candidate shape `detectable_candidate` can emit \
         (`src/ship/sensors.rs`), so in production the guard admits everything it \
         is ever shown. Recorded as current behaviour: the migration must not \
         quietly 'fix' this into something that can fail, nor quietly delete the \
         clauses because they look inert."
    );
}

/// Navigation steers on objectives alone; chart contacts enrich but never steer.
///
/// The eligibility guard admits only `reachable` candidates and only the
/// objective source marks its destination reachable, so a chart contact on its
/// own is invisible to the ranking. When the SAME entity is surfaced by both
/// sources the selector's dedup folds the facts together, and the contact's
/// marker adds its +1 to the objective's 100 — which is the entire meaning of
/// "enrich". Pinned as behaviour because the enrichment is a property of the
/// dedup merge, not of anything visible in the authored block.
#[test]
fn pin_navigation_chart_contacts_enrich_but_never_steer() {
    let sel = decoded_selector(default_navigation_target_selector_config());
    let ctx = self_ctx(&[]);

    let chart_only = vec![candidate("charted", &[("source_chart_contact", 1.0)])];
    assert_eq!(
        pick(&sel, &ctx, &chart_only, None),
        None,
        "a chart contact is not `reachable`, so it is ineligible and the AI waypoint \
         is driven by objectives ALONE — the retired contract. An author may widen \
         eligibility to admit them; the baseline must not."
    );

    let destination =
        |uuid: &str| candidate(uuid, &[("reachable", 1.0), ("source_nav_objective", 1.0)]);
    assert_eq!(
        pick(&sel, &ctx, &[destination("dest")], None).as_deref(),
        Some("dest"),
        "the objective source marks its resolved destination reachable, so it selects."
    );

    // The same entity from both sources: dedup keeps one candidate and merges
    // the markers, so it scores 100 + 1 and outranks a plain destination.
    let coincident = vec![
        destination("aaa-plain-destination"),
        candidate("zzz-both", &[("source_chart_contact", 1.0)]),
        destination("zzz-both"),
    ];
    assert_eq!(
        pick(&sel, &ctx, &coincident, None).as_deref(),
        Some("zzz-both"),
        "a destination the chart ALSO shows scores 101 against a plain destination's \
         100 and wins from the worse uuid. That +1 is the only influence chart \
         contacts have under the canonical policy: they break ties between \
         objective destinations, and nothing else."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Spawn-path pins — WHICH systems get a synthesised policy or selector
// ─────────────────────────────────────────────────────────────────────────────
//
// These go through the real `entities::spawner::spawn_entity` path on shipped
// hull TOMLs, not through hand-built fixtures. The mapping "authored nothing ⇒
// this exact policy gets invented" is itself part of the specification, and it
// is what #794 AC1's "explicit policy or explicit idle" has to replace.

mod spawn_path {
    use super::*;
    use crate::entity_config::EntityConfig;
    use bevy::prelude::*;

    /// The Harrow Lancer: the one shipped hull that carries all three weapon
    /// kinds AND authors not a single AI policy block, so all fourteen
    /// synthesisers fire on one entity.
    const LANCER: &str = include_str!("../../assets/entities/ship_harrow_lancer.toml");
    /// The Alliance Battleship: two phaser banks, one blaster bank, five torpedo
    /// tubes, none of them authoring an inline `ai` block — the per-weapon
    /// derivation rule's shape pin.
    const BATTLESHIP: &str = include_str!("../../assets/entities/alliance_battleship.toml");
    /// The Harrow Cruiser: a MIXED hull. Its torpedo tubes author their own
    /// policies while its phaser banks do not, so the "authored wins, absent is
    /// synthesised" rule is observable per weapon on one ship.
    const HARROW_CRUISER: &str = include_str!("../../assets/entities/ship_harrow_cruiser.toml");
    /// The Alliance Cruiser: authors `[captain_console.ai]` and
    /// `[power.ai_policy]`, so those two synthesisers must NOT fire for it.
    const ALLIANCE_CRUISER: &str = include_str!("../../assets/entities/alliance_cruiser.toml");
    /// Axiom Station: a shipped entity with NO `[behaviour]` block at all. The
    /// negative half of the attachment mapping — nothing AI is attached to it.
    const STATION: &str = include_str!("../../assets/entities/station_axiom.toml");

    fn spawn(toml: &str, what: &str) -> (App, Entity) {
        let config = EntityConfig::from_toml(toml)
            .unwrap_or_else(|e| panic!("{what} template must parse: {e}"));
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        let entity = {
            let mut commands = app.world_mut().commands();
            crate::entity_spawner::spawn_entity(
                &mut commands,
                &config,
                Vec3::ZERO,
                format!("pin-{what}"),
                None,
            )
        };
        app.update();
        (app, entity)
    }

    /// A hull that authors NOTHING gets all fourteen policies invented for it at
    /// spawn, each byte-identical to its synthesiser.
    ///
    /// This is the mapping #885 AC1 replaces: today "declares intent" is
    /// satisfied by Rust, not by authoring, and there is nothing for content
    /// validation to reject.
    #[test]
    fn lancer_authoring_nothing_gets_every_policy_synthesised() {
        let (app, e) = spawn(LANCER, "lancer");
        let w = app.world();

        macro_rules! pin_component {
            ($ty:path, $label:literal, $default:expr) => {{
                let got = w.get::<$ty>(e).unwrap_or_else(|| {
                    panic!(
                        "{}: a hull with [behaviour] and no authored block MUST still be \
                         given a synthesised policy at spawn — its absence means the \
                         system falls back to a tick-local default (or nothing) instead",
                        $label
                    )
                });
                assert_eq!(
                    got.0,
                    decoded($default),
                    "{}: the spawned policy is the canonical synthesised default, \
                     verbatim. #885's authored replacement must decode to exactly this.",
                    $label
                );
            }};
        }

        pin_component!(
            crate::captain_plugin::CaptainAiPolicy,
            "captain",
            default_captain_ai_config()
        );
        pin_component!(
            crate::console::comms::server::CommsResponseAiPolicy,
            "comms_response",
            default_comms_response_ai_config()
        );
        pin_component!(
            crate::ship::helm_ai::HelmEnginesAiPolicy,
            "engines",
            default_engines_ai_config()
        );
        pin_component!(
            crate::ship::helm_ai::HelmSteeringAiPolicy,
            "steering",
            default_steering_ai_config()
        );
        pin_component!(
            crate::ship::helm_ai::HelmLateralAiPolicy,
            "lateral",
            default_lateral_ai_config()
        );
        pin_component!(
            crate::ship::helm_ai::HelmVerticalAiPolicy,
            "vertical",
            default_vertical_ai_config()
        );
        pin_component!(
            crate::ship::helm_ai::HelmImpulseAiPolicy,
            "impulse",
            default_impulse_ai_config()
        );
        pin_component!(
            crate::ship::helm_ai::HelmBoostAiPolicy,
            "boost",
            default_boost_ai_config()
        );
        pin_component!(
            crate::ship::shields::ShieldsFocusAiPolicy,
            "shields_focus",
            default_shields_focus_ai_config()
        );
        pin_component!(
            crate::power_plugin::PowerAiPolicy,
            "power",
            default_power_ai_config()
        );
        pin_component!(
            crate::weapons_plugin::TorpedoMagazineAiPolicy,
            "torpedo_magazine",
            default_torpedo_magazine_ai_config()
        );

        // The three per-weapon maps, keyed by the hull's authored ids.
        let phasers = w
            .get::<crate::weapons_plugin::PhaserBankAiPolicies>(e)
            .expect("phaser_bank: a hull with phaser banks gets a per-bank policy map");
        assert_eq!(
            phasers.0.keys().collect::<Vec<_>>(),
            vec!["lash"],
            "phaser_bank: one entry per AUTHORED bank id, and only those."
        );
        assert_eq!(
            phasers.0["lash"],
            decoded(default_phaser_bank_ai_config()),
            "phaser_bank `lash`: the canonical unconditional-fire default."
        );

        let blasters = w
            .get::<crate::weapons_plugin::BlasterBankAiPolicies>(e)
            .expect("blaster_bank: a hull with blaster banks gets a per-bank policy map");
        assert_eq!(
            blasters.0.keys().collect::<Vec<_>>(),
            vec!["spike"],
            "blaster_bank: one entry per AUTHORED bank id."
        );
        assert_eq!(
            blasters.0["spike"],
            decoded(default_blaster_bank_ai_config()),
            "blaster_bank `spike`: the canonical unconditional-fire default."
        );

        let tubes = w
            .get::<crate::weapons_plugin::TorpedoTubeAiPolicies>(e)
            .expect("torpedo_tube: a hull with tubes gets a per-tube policy map");
        assert_eq!(
            tubes.0.keys().collect::<Vec<_>>(),
            vec!["lance"],
            "torpedo_tube: one entry per AUTHORED tube id."
        );
        assert_eq!(
            tubes.0["lance"],
            decoded(default_torpedo_tube_ai_config()),
            "torpedo_tube `lance`: the canonical unconditional load + launch default."
        );
    }

    /// The per-weapon DERIVATION RULE, pinned as a shape rather than as one
    /// hull's instance: one policy per authored bank/tube id, keyed by that id,
    /// every one of them the canonical default when no inline `ai` is authored.
    ///
    /// The Battleship is the widest case shipped — two phaser banks, one blaster
    /// bank, five torpedo tubes — so it also pins that the map size follows the
    /// authored list length rather than any fixed count.
    #[test]
    fn battleship_derives_one_synthesised_policy_per_authored_weapon() {
        let (app, e) = spawn(BATTLESHIP, "battleship");
        let w = app.world();

        let phasers = &w
            .get::<crate::weapons_plugin::PhaserBankAiPolicies>(e)
            .expect("battleship must carry a phaser policy map")
            .0;
        let mut phaser_ids: Vec<&str> = phasers.keys().map(String::as_str).collect();
        phaser_ids.sort_unstable();
        assert_eq!(
            phaser_ids,
            vec!["aft", "fore"],
            "one policy per authored phaser bank id — the map is DERIVED from the \
             `[[weapons_console.phaser_banks]]` list, not from a fixed set."
        );

        let blasters = &w
            .get::<crate::weapons_plugin::BlasterBankAiPolicies>(e)
            .expect("battleship must carry a blaster policy map")
            .0;
        assert_eq!(
            blasters.keys().collect::<Vec<_>>(),
            vec!["heavy-fore"],
            "one policy per authored blaster bank id."
        );

        let tubes = &w
            .get::<crate::weapons_plugin::TorpedoTubeAiPolicies>(e)
            .expect("battleship must carry a tube policy map")
            .0;
        let mut tube_ids: Vec<&str> = tubes.keys().map(String::as_str).collect();
        tube_ids.sort_unstable();
        assert_eq!(
            tube_ids,
            vec![
                "aft-port",
                "aft-starboard",
                "fore-centre",
                "fore-port",
                "fore-starboard"
            ],
            "one policy per authored tube id — five tubes, five policies. #885 must \
             author five separate declarations here, not one shared block."
        );

        // Every derived entry is the canonical default, since this hull authors
        // no inline `ai` on any bank or tube.
        let phaser_default = decoded(default_phaser_bank_ai_config());
        for (id, policy) in phasers {
            assert_eq!(
                policy, &phaser_default,
                "phaser bank `{id}`: no inline `ai` authored ⇒ the canonical default."
            );
        }
        let blaster_default = decoded(default_blaster_bank_ai_config());
        for (id, policy) in blasters {
            assert_eq!(
                policy, &blaster_default,
                "blaster bank `{id}`: no inline `ai` authored ⇒ the canonical default."
            );
        }
        let tube_default = decoded(default_torpedo_tube_ai_config());
        for (id, policy) in tubes {
            assert_eq!(
                policy, &tube_default,
                "tube `{id}`: no inline `ai` authored ⇒ the canonical default."
            );
        }
    }

    /// The synthesiser is a per-weapon FALLBACK, not a per-ship one: on a hull
    /// that authors policies for its tubes but not for its phaser banks, only
    /// the phaser banks get invented content.
    ///
    /// This is the property that makes #885 a per-declaration migration rather
    /// than a per-hull one.
    #[test]
    fn harrow_cruiser_synthesises_only_the_weapons_it_did_not_author() {
        let (app, e) = spawn(HARROW_CRUISER, "harrow-cruiser");
        let w = app.world();

        let phasers = &w
            .get::<crate::weapons_plugin::PhaserBankAiPolicies>(e)
            .expect("harrow cruiser must carry a phaser policy map")
            .0;
        let phaser_default = decoded(default_phaser_bank_ai_config());
        assert_eq!(phasers.len(), 2, "the hull authors two phaser banks.");
        for (id, policy) in phasers {
            assert_eq!(
                policy, &phaser_default,
                "phaser bank `{id}`: this hull authors NO inline `ai` on its banks, so \
                 the canonical unconditional-fire default is synthesised for each."
            );
        }

        let tubes = &w
            .get::<crate::weapons_plugin::TorpedoTubeAiPolicies>(e)
            .expect("harrow cruiser must carry a tube policy map")
            .0;
        let tube_default = decoded(default_torpedo_tube_ai_config());
        assert_eq!(tubes.len(), 2, "the hull authors two torpedo tubes.");
        for (id, policy) in tubes {
            assert_ne!(
                policy, &tube_default,
                "tube `{id}`: this hull DOES author an inline `ai`, so the synthesiser \
                 must not fire for it. Authored always wins over synthesised."
            );
        }
    }

    /// The WORKED EXAMPLE #885 has to repeat ~200 times.
    ///
    /// The Alliance Cruiser is one of the handful of hulls that already authors
    /// `[captain_console.ai]` and `[power.ai_policy]` by hand — and both blocks
    /// decode to policies **byte-identical to the synthesised defaults**. That
    /// is the standard the migration is held to: transcribing the synthesiser
    /// into TOML is expected to round-trip exactly, so a diff against this
    /// suite's content pins is a sufficient check for the remaining hulls.
    ///
    /// If this test ever fails, either an authored block or its synthesiser
    /// drifted, and the two are no longer interchangeable — which is precisely
    /// the regression #885 risks.
    #[test]
    fn alliance_cruiser_authored_blocks_round_trip_to_the_synthesised_defaults() {
        let (app, e) = spawn(ALLIANCE_CRUISER, "alliance-cruiser");
        let w = app.world();

        let captain = w
            .get::<crate::captain_plugin::CaptainAiPolicy>(e)
            .expect("alliance cruiser must carry a captain policy");
        assert_eq!(
            captain.0,
            decoded(default_captain_ai_config()),
            "this hull's hand-authored `[captain_console.ai]` decodes to EXACTLY the \
             synthesised default. It is the shipped proof that a verbatim \
             transcription is behaviour-preserving."
        );

        let power = w
            .get::<crate::power_plugin::PowerAiPolicy>(e)
            .expect("alliance cruiser must carry a power policy");
        assert_eq!(
            power.0,
            decoded(default_power_ai_config()),
            "same for its hand-authored `[power.ai_policy]`: four rules, four params, \
             identical to the synthesiser down to the guard strings."
        );

        // …while the systems it left unauthored are still invented for it.
        // Authoring is per-SYSTEM, never per-hull: this hull declaring two
        // blocks buys nothing for the other twelve fine systems.
        assert_eq!(
            w.get::<crate::ship::helm_ai::HelmEnginesAiPolicy>(e)
                .expect("engines policy attached")
                .0,
            decoded(default_engines_ai_config()),
            "the same hull leaves `[helm_console.engines_ai]` unauthored, so Engines \
             is still synthesised — the twelve remaining declarations this hull owes \
             #885 are exactly the ones with no block in its TOML."
        );
    }

    /// Assert all five selectors on a spawned hull are the canonical
    /// synthesised ones, verbatim.
    ///
    /// Factored out because the interesting pin is that this holds for EVERY
    /// shipped hull regardless of shape — no hull authors a `[*.selector]`
    /// block, so #885 has 5 × 12 declarations to write and not one shipped
    /// worked example to check a transcription against.
    fn assert_all_five_canonical(w: &World, e: Entity, hull: &str) {
        assert_eq!(
            w.get::<crate::ship::sensors::SensorsTargetSelector>(e)
                .unwrap_or_else(|| panic!("{hull}: sensors selector attached"))
                .selector,
            decoded_selector(default_sensors_target_selector_config()),
            "{hull}: Sensors runs the canonical Rust-side selector."
        );
        assert_eq!(
            w.get::<crate::weapons_plugin::TacticalTargetSelector>(e)
                .unwrap_or_else(|| panic!("{hull}: tactical selector attached"))
                .selector,
            decoded_selector(default_tactical_target_selector_config()),
            "{hull}: Tactical (radar targeting) runs the canonical Rust-side selector."
        );
        assert_eq!(
            w.get::<crate::console::navigation::NavigationTargetSelector>(e)
                .unwrap_or_else(|| panic!("{hull}: navigation selector attached"))
                .selector,
            decoded_selector(default_navigation_target_selector_config()),
            "{hull}: Navigation runs the canonical Rust-side selector."
        );
        assert_eq!(
            w.get::<crate::console::repair::server::RepairTargetSelector>(e)
                .unwrap_or_else(|| panic!("{hull}: repair selector attached"))
                .selector,
            decoded_selector(default_repair_target_selector_config()),
            "{hull}: Repair runs the canonical Rust-side selector."
        );
        assert_eq!(
            w.get::<crate::console::comms::server::CommsTargetSelector>(e)
                .unwrap_or_else(|| panic!("{hull}: comms hail selector attached"))
                .selector,
            decoded_selector(default_comms_target_selector_config()),
            "{hull}: the Comms HAIL selector is distinct from the Comms RESPONSE \
             policy — one ranks who to hail, the other decides how to answer — and \
             both are synthesised."
        );
    }

    /// The five per-system target SELECTORS are a separate synthesised family
    /// (`default_*_target_selector_config`), outside the fourteen policies —
    /// but they are attached at the same spawn and are equally unauthored.
    ///
    /// The Lancer is the sharpest case: it carries `[behaviour]` and a
    /// `[weapons_console]` and nothing else, yet it is given all five. #885
    /// lists them in scope ("five selectors … which today all run Rust-side
    /// `*::default()` selectors with nothing authored at all"), and their
    /// presence is what keeps "every AI-capable fine system declares intent"
    /// false even after the fourteen are gone.
    #[test]
    fn the_five_selectors_are_also_synthesised_and_are_not_part_of_the_fourteen() {
        let (app, e) = spawn(LANCER, "lancer-selectors");
        assert_all_five_canonical(app.world(), e, "lancer");
    }

    /// A selector is attached whether or not the hull carries the console
    /// section it belongs to.
    ///
    /// The Lancer authors no `[sensors_console]`, no `[navigation_console]`, no
    /// `[repair]` and no `[comms_console]` — yet it is given a Sensors,
    /// Navigation, Repair and Comms-hail selector all the same, because the
    /// spawn path gates the whole block on `[behaviour]` alone.
    ///
    /// This is the mapping most likely to be missed when #885 authors per hull:
    /// reading the Lancer's TOML gives no hint that four of its five selectors
    /// exist, so a per-hull migration driven by "what sections does this file
    /// have?" would drop them and the hull would silently lose its ranking.
    #[test]
    fn selectors_are_attached_even_for_console_sections_the_hull_never_declares() {
        let config = crate::entity_config::EntityConfig::from_toml(LANCER)
            .expect("lancer template must parse");
        assert!(
            config.behaviour.is_some(),
            "the Lancer is an AI-bearing hull — `[behaviour]` is the ONLY gate on the \
             whole selector block."
        );
        assert!(
            config.sensors_console.is_none()
                && config.navigation_console.is_none()
                && config.repair.is_none()
                && config.comms_console.is_none(),
            "precondition: the Lancer declares none of these four console sections. \
             If a future edit adds one, move this pin to a hull that still omits them \
             rather than deleting it — the attachment rule is the point."
        );

        let (app, e) = spawn(LANCER, "lancer-absent-sections");
        assert_all_five_canonical(app.world(), e, "lancer");
    }

    /// …and a hull that DOES carry all the console sections still gets the
    /// canonical selectors, because carrying `[sensors_console]` is not the
    /// same as authoring `[sensors_console.selector]`.
    ///
    /// The Battleship and the Alliance Cruiser both declare
    /// `[sensors_console]`, `[weapons_console]`, `[navigation_console]` and
    /// `[repair]`, and the Cruiser additionally hand-authors two AI POLICY
    /// blocks. None of that buys a single authored selector. The two families
    /// are independent, which is why #885 cannot treat "this hull has authored
    /// AI" as "this hull is done".
    #[test]
    fn hulls_with_every_console_section_still_get_the_canonical_selectors() {
        for (toml, hull) in [(BATTLESHIP, "battleship"), (ALLIANCE_CRUISER, "cruiser")] {
            let config = crate::entity_config::EntityConfig::from_toml(toml)
                .unwrap_or_else(|e| panic!("{hull} template must parse: {e}"));
            assert!(
                config.sensors_console.is_some()
                    && config.weapons_console.is_some()
                    && config.navigation_console.is_some()
                    && config.repair.is_some(),
                "{hull}: precondition — this hull declares the console sections the \
                 Lancer omits, so it covers the opposite shape."
            );
            let (app, e) = spawn(toml, hull);
            assert_all_five_canonical(app.world(), e, hull);
        }
    }

    /// Authored per-weapon POLICY does not imply an authored SELECTOR.
    ///
    /// The Harrow Cruiser authors inline `ai` blocks on its torpedo tubes — the
    /// hull most obviously "doing AI authoring" of the four — and still runs
    /// all five canonical selectors. Pinned so the migration does not assume a
    /// hull that authors anything has been dealt with.
    #[test]
    fn harrow_cruiser_authored_tube_policies_buy_it_no_authored_selector() {
        let (app, e) = spawn(HARROW_CRUISER, "harrow-cruiser-selectors");
        assert_all_five_canonical(app.world(), e, "harrow-cruiser");
    }

    /// The negative half of the mapping: an entity with no `[behaviour]` block
    /// gets NO selector at all.
    ///
    /// Axiom Station is a shipped, hailable, damageable entity — it has
    /// `[hull]` and `[comms]` — and it is still not an AI actor, so not one of
    /// the five components is attached. That boundary is part of what #885's
    /// "every AI-capable fine system" has to mean: `[behaviour]` is what makes a
    /// fine system AI-capable, and content validation for missing intent must
    /// not start demanding declarations from scenery.
    #[test]
    fn an_entity_without_a_behaviour_block_gets_no_selector_at_all() {
        let config = crate::entity_config::EntityConfig::from_toml(STATION)
            .expect("station template must parse");
        assert!(
            config.behaviour.is_none(),
            "precondition: Axiom Station carries no `[behaviour]` block."
        );

        let (app, e) = spawn(STATION, "station");
        let w = app.world();
        assert!(
            w.get::<crate::ship::sensors::SensorsTargetSelector>(e)
                .is_none(),
            "station: no `[behaviour]` ⇒ no Sensors selector."
        );
        assert!(
            w.get::<crate::weapons_plugin::TacticalTargetSelector>(e)
                .is_none(),
            "station: no Tactical selector."
        );
        assert!(
            w.get::<crate::console::navigation::NavigationTargetSelector>(e)
                .is_none(),
            "station: no Navigation selector."
        );
        assert!(
            w.get::<crate::console::repair::server::RepairTargetSelector>(e)
                .is_none(),
            "station: no Repair selector."
        );
        assert!(
            w.get::<crate::console::comms::server::CommsTargetSelector>(e)
                .is_none(),
            "station: no Comms hail selector — even though the station DOES carry a \
             `[comms]` block and is a hail CONTACT. Being hailable is not being an AI \
             that hails."
        );
    }

    /// Tactical is the only one of the five with an explicit IDLE lever, and no
    /// shipped hull pulls it.
    ///
    /// `[weapons_console] selector_idle` (#781 AC6) sets
    /// `TacticalTargetSelector.idle`; Sensors, Navigation, Repair and the Comms
    /// hail selector have no idle field at all. That asymmetry is load-bearing
    /// for #885: #794 AC1 requires "inline policy **or explicit idle**" from
    /// every AI-capable fine system, and for four of the five selectors an
    /// explicit idle is currently not expressible in the schema.
    ///
    /// Pinned as current behaviour, not fixed — a pin that changes what it pins
    /// is worthless, and the schema gap is a scoping question for #885.
    #[test]
    fn tactical_is_the_only_selector_with_an_idle_lever_and_no_hull_pulls_it() {
        for (toml, hull) in [
            (LANCER, "lancer"),
            (BATTLESHIP, "battleship"),
            (HARROW_CRUISER, "harrow-cruiser"),
            (ALLIANCE_CRUISER, "alliance-cruiser"),
        ] {
            let (app, e) = spawn(toml, hull);
            assert!(
                !app.world()
                    .get::<crate::weapons_plugin::TacticalTargetSelector>(e)
                    .unwrap_or_else(|| panic!("{hull}: tactical selector attached"))
                    .idle,
                "{hull}: `selector_idle` is unauthored, so Tactical's radar runs its \
                 selector. This is the ONLY selector idle that exists — the other four \
                 have no such field, so #885 cannot declare them explicitly idle \
                 without a schema change."
            );
        }
    }
}

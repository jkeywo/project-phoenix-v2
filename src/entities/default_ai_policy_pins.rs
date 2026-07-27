//! Behavioural PINS for the fourteen `default_*_ai_config()` synthesisers
//! (issue #885, step 1).
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
//! params, payloads, statelessness — not merely that a policy is `Some(_)`, so
//! a reader can transcribe the equivalent TOML from a failing assertion without
//! opening `config.rs`.
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
//! # How the pins are layered
//!
//! 1. **Roll call** — the family has exactly fourteen members, all stateless.
//! 2. **Content pins** — one test per synthesiser over the *authorable* shape
//!    (`FineSystemAiConfigToml`), because that is what #885 must retype in TOML.
//!    Values are asserted as LITERALS, never against the constant the
//!    synthesiser itself reads, so a changed constant fails the pin instead of
//!    silently moving with it.
//! 3. **Decode pins** — the same content after `to_policy()`, pinning which
//!    typed `AiPolicyVerb` each channel yields and which payloads ride along.
//! 4. **Guard truth tables** — every guarded rule is proved to fire *and* to be
//!    able to read false, through `resolve_channel`. This is the trap #885
//!    calls out: an unseeded or misspelled `fact(...)` parses, validates, and
//!    reads false for ever.
//! 5. **Spawn-path pins** — WHICH systems get a synthesised policy at all, run
//!    through the real `spawn_entity` path on shipped hulls rather than by
//!    calling the synthesisers directly. #792 found a fixture that omitted the
//!    very components that broke the feature in production; a pin that does not
//!    travel the real path can drift from it.

use super::config::{
    default_blaster_bank_ai_config, default_boost_ai_config, default_captain_ai_config,
    default_comms_response_ai_config, default_engines_ai_config, default_impulse_ai_config,
    default_lateral_ai_config, default_phaser_bank_ai_config, default_power_ai_config,
    default_shields_focus_ai_config, default_steering_ai_config,
    default_torpedo_magazine_ai_config, default_torpedo_tube_ai_config, default_vertical_ai_config,
    FineSystemAiConfigToml,
};
use crate::ai::policy::{AiPolicy, AiPolicyVerb};
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
// 5. Spawn-path pins — WHICH systems get a synthesised policy at all
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

    /// The five per-system target SELECTORS are a separate synthesised family
    /// (`default_*_target_selector_config`), outside the fourteen this suite
    /// pins — but they are attached at the same spawn and are equally
    /// unauthored today.
    ///
    /// Recorded here because #885 lists them in its scope ("five selectors …
    /// which today all run Rust-side defaults with nothing authored at all"),
    /// and because their presence is what makes "every AI-capable fine system
    /// declares intent" false today even after the fourteen are gone.
    #[test]
    fn the_five_selectors_are_also_synthesised_and_are_not_part_of_the_fourteen() {
        let (app, e) = spawn(LANCER, "lancer-selectors");
        let w = app.world();

        assert_eq!(
            w.get::<crate::ship::sensors::SensorsTargetSelector>(e)
                .expect("sensors selector attached")
                .selector,
            crate::entities::config::default_sensors_target_selector_config()
                .to_selector()
                .expect("canonical sensors selector decodes"),
            "Sensors runs the canonical Rust-side selector; the hull authors none."
        );
        assert_eq!(
            w.get::<crate::weapons_plugin::TacticalTargetSelector>(e)
                .expect("tactical selector attached")
                .selector,
            crate::entities::config::default_tactical_target_selector_config()
                .to_selector()
                .expect("canonical tactical selector decodes"),
            "Tactical (radar targeting) runs the canonical Rust-side selector."
        );
        assert_eq!(
            w.get::<crate::console::navigation::NavigationTargetSelector>(e)
                .expect("navigation selector attached")
                .selector,
            crate::entities::config::default_navigation_target_selector_config()
                .to_selector()
                .expect("canonical navigation selector decodes"),
            "Navigation runs the canonical Rust-side selector."
        );
        assert_eq!(
            w.get::<crate::console::repair::server::RepairTargetSelector>(e)
                .expect("repair selector attached")
                .selector,
            crate::entities::config::default_repair_target_selector_config()
                .to_selector()
                .expect("canonical repair selector decodes"),
            "Repair runs the canonical Rust-side selector."
        );
        assert_eq!(
            w.get::<crate::console::comms::server::CommsTargetSelector>(e)
                .expect("comms hail selector attached")
                .selector,
            crate::entities::config::default_comms_target_selector_config()
                .to_selector()
                .expect("canonical comms selector decodes"),
            "The Comms HAIL selector is distinct from the Comms RESPONSE policy \
             pinned above: one ranks who to hail, the other decides how to answer."
        );
    }
}

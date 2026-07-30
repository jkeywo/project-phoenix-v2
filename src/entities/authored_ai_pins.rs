//! Behavioural pins for the AUTHORED fine-system AI declarations (issue #885b
//! stage 5d).
//!
//! # What this replaces, and what it deliberately does not
//!
//! Until stage 5d, `src/entities/default_ai_policy_pins.rs` pinned the nineteen
//! Rust-side synthesisers that invented a declaration for any hull that authored
//! none. That file's own module doc said it existed to be diffed against during
//! the content migration and was expected to be deleted with the synthesisers.
//! Stage 5c finished the authoring, stage 5d deleted the synthesisers, and this
//! module is what survived the deletion.
//!
//! Three layers of that suite were retired on purpose:
//!
//! * **The roll call** ("there are exactly fourteen, exactly five, all
//!   stateless"). Its subject no longer exists. What it really guarded — that no
//!   AI-capable fine system is quietly handed a Rust-side default — is now
//!   guarded structurally, by [`crate::entities::ai_declaration_manifest`]:
//!   strict mode rejects an undeclared slot at load, and
//!   `no_synthesiser_is_defined_or_called_anywhere` fails if a synthesiser comes
//!   back.
//! * **The literal content pins** (every rule, guard, param and weight asserted
//!   as a Rust literal). They existed so the migration could transcribe the Rust
//!   baseline into TOML without drift. That transcription has happened, and the
//!   TOML is now the specification — a designer retuning a weight is doing
//!   exactly what AGENTS.md rule #11 asks for, and must not have to edit Rust to
//!   do it. Re-asserting the same numbers here would rebuild the coupling the
//!   migration removed.
//! * **The spawn-path pins** ("which systems get a declaration attached at
//!   all"). Wholly subsumed by
//!   `ai_declaration_manifest::tests::the_manifest_matches_the_real_spawner`,
//!   which runs the real `spawn_entity` over every shipped hull and checks the
//!   slot set in both directions.
//!
//! Three layers are genuinely about behaviour rather than about the
//! synthesisers, and they are kept — re-pointed at the shipped authored content:
//!
//! 1. **The fleet-baseline pin** ([`BESPOKE_DOCTRINES`]). The old suite asserted
//!    twelve hand-written manoeuvres NOT-EQUAL to their synthesised baselines;
//!    with the baselines deleted there is nothing left to compare against, so
//!    the baseline is now DERIVED FROM THE FLEET: for each policy kind, the
//!    configuration the non-bespoke hulls all share. Same bidirectional
//!    guarantee, no Rust-side default — a bespoke doctrine collapsing onto the
//!    fleet baseline fails, and so does a hull silently departing from it.
//! 2. **The guard truth tables.** Every guard on the shipped policies is proved
//!    to fire AND to be able to read false. That was always an assertion about
//!    content, not about synthesis: a guard on an unseeded or misspelled fact
//!    parses, validates, and reads false for ever.
//! 3. **The selector ordering invariants.** For the selectors the numbers are
//!    not the specification, the ORDERING is ("an explicit mission objective
//!    always beats everything else"; "one damage tier beats the whole deficit
//!    ladder"). A reweight can leave every individual number looking plausible
//!    while inverting the order, and these run the real
//!    `TargetSelector::select` over hand-built candidates to catch it.
//!
//! Everything here reads `assets/entities/*.toml` — the shipped content — so a
//! failure names a hull and a block an author can open.

use crate::ai::policy::{AiPolicy, AiPolicyVerb};
use crate::ai::selector::{SelectorCandidate, SelfContext, TargetSelector};
use crate::entities::config::{EntityConfig, FineSystemAiConfigToml, FineSystemAiSelectorToml};
use crate::world::flags::AiFacts;
use std::collections::BTreeMap;

// ─────────────────────────────────────────────────────────────────────────────
// Reading the shipped content
// ─────────────────────────────────────────────────────────────────────────────

fn crate_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `assets/entities/*.toml` stem, sorted.
fn entity_stems() -> Vec<String> {
    let dir = crate_root().join("assets/entities");
    let mut out: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("assets/entities must be readable") {
        let path = entry.expect("readable dir entry").path();
        if path.extension().is_some_and(|e| e == "toml") {
            out.push(
                path.file_stem()
                    .expect("toml file has a stem")
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    out.sort();
    out
}

/// One shipped entity, parsed through the real (strict) load path — INCLUDING
/// include resolution (issue #906), so this stays the real load path once a hull
/// is composed rather than quietly becoming an assertion on unresolved text.
fn entity(stem: &str) -> EntityConfig {
    let path = crate_root().join(format!("assets/entities/{stem}.toml"));
    let key = path.to_string_lossy().replace('\\', "/");
    crate::entity_includes::load_entity_config(&key)
        .unwrap_or_else(|e| panic!("{stem} must load: {e}"))
}

/// Every AI-BEARING shipped hull (`[behaviour]` is the gate), stem and config.
fn ai_hulls() -> Vec<(String, EntityConfig)> {
    entity_stems()
        .into_iter()
        .map(|stem| {
            let c = entity(&stem);
            (stem, c)
        })
        .filter(|(_, c)| c.behaviour.is_some())
        .collect()
}

fn policy(cfg: &FineSystemAiConfigToml) -> AiPolicy {
    cfg.to_policy().expect("an authored policy block decodes")
}

fn selector(cfg: &FineSystemAiSelectorToml) -> TargetSelector {
    cfg.to_selector()
        .expect("an authored selector block decodes")
}

/// A fact snapshot; anything not listed is ABSENT, which makes every comparison
/// against it evaluate false.
fn facts(pairs: &[(&str, f64)]) -> AiFacts {
    let mut f = AiFacts::new();
    for (k, v) in pairs {
        f.set(k, *v);
    }
    f
}

fn resolve(p: &AiPolicy, channel: &str, snapshot: &AiFacts) -> Option<AiPolicyVerb> {
    p.resolve_channel(channel, snapshot, &[]).cloned()
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

fn self_ctx(pairs: &[(&str, f64)]) -> SelfContext {
    SelfContext {
        position: [0.0, 0.0, 0.0],
        facts: facts(pairs),
    }
}

fn pick(
    sel: &TargetSelector,
    ctx: &SelfContext,
    candidates: &[SelectorCandidate],
    current: Option<&str>,
) -> Option<String> {
    sel.select(ctx, candidates, current, &[])
}

/// Every authored POLICY on one hull, keyed by the
/// [`crate::entities::ai_declaration_manifest::Slot::key`] form.
///
/// Written out by hand rather than derived, because the point of the fleet
/// baseline below is to compare like with like: the key names the KIND, and only
/// policies of the same kind are ever compared.
fn authored_policies(c: &EntityConfig) -> Vec<(String, FineSystemAiConfigToml)> {
    let mut out: Vec<(String, FineSystemAiConfigToml)> = Vec::new();
    let mut push = |key: &str, cfg: Option<&FineSystemAiConfigToml>| {
        if let Some(cfg) = cfg {
            out.push((key.to_string(), cfg.clone()));
        }
    };
    push(
        "captain",
        c.captain_console.as_ref().and_then(|x| x.ai.as_ref()),
    );
    push(
        "comms_response",
        c.comms_console.as_ref().and_then(|x| x.ai.as_ref()),
    );
    let helm = c.helm_console.as_ref();
    push("engines", helm.and_then(|h| h.engines_ai.as_ref()));
    push("steering", helm.and_then(|h| h.steering_ai.as_ref()));
    push("lateral", helm.and_then(|h| h.lateral_ai.as_ref()));
    push("vertical", helm.and_then(|h| h.vertical_ai.as_ref()));
    push("impulse", helm.and_then(|h| h.impulse_ai.as_ref()));
    push("boost", helm.and_then(|h| h.boost_ai.as_ref()));
    push(
        "shields_focus",
        c.shields_console
            .as_ref()
            .and_then(|x| x.ai_policy.as_ref()),
    );
    push("power", c.power.as_ref().and_then(|x| x.ai_policy.as_ref()));
    push(
        "torpedo_magazine",
        c.torpedoes.as_ref().and_then(|t| t.ai.as_ref()),
    );
    // `push` borrows `out` mutably; the per-weapon loops below need it again.
    let _ = push;
    for bank in c.weapons_console.iter().flat_map(|w| w.phaser_banks.iter()) {
        if let Some(ai) = bank.ai.as_ref() {
            out.push((format!("phaser_bank[{}]", bank.id), ai.clone()));
        }
    }
    for bank in c
        .weapons_console
        .iter()
        .flat_map(|w| w.blaster_banks.iter())
    {
        if let Some(ai) = bank.ai.as_ref() {
            out.push((format!("blaster_bank[{}]", bank.id), ai.clone()));
        }
    }
    for tube in c.torpedoes.iter().flat_map(|t| t.tubes.iter()) {
        if let Some(ai) = tube.ai.as_ref() {
            out.push((format!("torpedo_tube[{}]", tube.id), ai.clone()));
        }
    }
    out
}

/// The KIND a slot key belongs to: `torpedo_tube[bow_port]` → `torpedo_tube`.
fn kind_of(slot: &str) -> &str {
    slot.split('[').next().unwrap_or(slot)
}

/// Every authored SELECTOR on one hull, keyed by system.
fn authored_selectors(c: &EntityConfig) -> Vec<(&'static str, FineSystemAiSelectorToml)> {
    let mut out: Vec<(&'static str, FineSystemAiSelectorToml)> = Vec::new();
    if let Some(s) = c.sensors_console.as_ref().and_then(|x| x.selector.as_ref()) {
        out.push(("sensors", s.clone()));
    }
    if let Some(s) = c.weapons_console.as_ref().and_then(|x| x.selector.as_ref()) {
        out.push(("tactical", s.clone()));
    }
    if let Some(s) = c
        .navigation_console
        .as_ref()
        .and_then(|x| x.selector.as_ref())
    {
        out.push(("navigation", s.clone()));
    }
    if let Some(s) = c.repair.as_ref().and_then(|x| x.selector.as_ref()) {
        out.push(("repair", s.clone()));
    }
    if let Some(s) = c.comms_console.as_ref().and_then(|x| x.selector.as_ref()) {
        out.push(("comms_hail", s.clone()));
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. The fleet baseline — what replaced the not-equal-to-the-synthesiser pin
// ─────────────────────────────────────────────────────────────────────────────

/// The (hull, slot) pairs where a hull authors a policy that DELIBERATELY
/// differs from the rest of the fleet.
///
/// These are the authored manoeuvres (#790–#792, #801) that make three Harrow
/// hulls fly differently from everything else: the broadside orbit and its
/// arc-aware tubes, the lance run and the fleet's only AI boost, and the
/// artillery platform that holds position rather than closing.
///
/// # Why this list is the successor to `BESPOKE_POLICIES`
///
/// The retired pin file asserted these twelve NOT-EQUAL to the Rust synthesiser
/// that would otherwise have been invented for them, and everything else EQUAL
/// to it. That is what caught a hand-written manoeuvre silently collapsing back
/// onto the default — a whole hull's doctrine vanishing with no test failing
/// anywhere else, because a policy that reverts to the baseline is still a
/// perfectly valid policy.
///
/// Stage 5d deleted the synthesisers, so there is no Rust-side baseline left to
/// compare against. The baseline is now derived FROM THE FLEET: for each policy
/// kind, the configuration every non-bespoke hull shares. The guarantee is
/// unchanged and bidirectional in the same two directions:
///
/// * a bespoke doctrine that collapses onto the fleet baseline fails, because
///   its entry here demands a difference;
/// * a hull that silently departs from the baseline fails, because it is not
///   listed here.
///
/// It is in fact slightly stronger: the old form could not notice two hulls
/// drifting apart together, and this one can, because the baseline has to be
/// unanimous among the hulls that are not listed.
const BESPOKE_DOCTRINES: &[(&str, &str)] = &[
    // The broadside orbit: a stateful engines/steering pair, plus two tubes
    // that hold fire until the target's striking arc is actually down.
    ("ship_harrow_cruiser", "engines"),
    ("ship_harrow_cruiser", "steering"),
    ("ship_harrow_cruiser", "torpedo_tube[bow_port]"),
    ("ship_harrow_cruiser", "torpedo_tube[bow_starboard]"),
    // The lance run, and the one hull in the fleet whose AI engages boost.
    ("ship_harrow_destroyer", "engines"),
    ("ship_harrow_destroyer", "steering"),
    ("ship_harrow_destroyer", "boost"),
    // The artillery platform: it holds position rather than closing, which is
    // why its impulse axis is authored IDLE rather than permitting.
    ("ship_harrow_warhawk", "engines"),
    ("ship_harrow_warhawk", "steering"),
    ("ship_harrow_warhawk", "impulse"),
    ("ship_harrow_warhawk", "torpedo_tube[fore]"),
    ("ship_harrow_warhawk", "torpedo_tube[aft]"),
    // ── The always-armed Harrow gun line (issue #872) ────────────────────────
    //
    // Every offensive weapon in the fleet is gated by ONE authored predicate,
    // `fact(red_alert) >= param(min_alert_to_fire)`, and the hulls differ only
    // in the threshold. An Alliance hull authors 1 — it has a captain's console
    // and holds fire until the alert is called. A Harrow authors 0 — always
    // armed, because it has no bridge crew to call one and its captain policy
    // stands the alert down out of combat, so a Harrow that waited would never
    // open fire at all.
    //
    // That single number is a real doctrinal departure and the list has to say
    // so, in both directions: a Harrow gun that quietly acquires the player
    // threshold stops shooting (and only this entry notices), and an Alliance
    // gun that quietly acquires the Harrow one becomes a weapon that fires with
    // the crew stood down (and the unanimity requirement notices that).
    //
    // The four Harrow tubes are NOT repeated here — they are already listed
    // above for their arc-aware launch doctrine, and they carry the same
    // threshold.
    ("ship_harrow_cruiser", "phaser_bank[fore]"),
    ("ship_harrow_cruiser", "phaser_bank[aft]"),
    ("ship_harrow_destroyer", "blaster_bank[harrow-lance-port]"),
    (
        "ship_harrow_destroyer",
        "blaster_bank[harrow-lance-starboard]",
    ),
    ("ship_harrow_lancer", "phaser_bank[lash]"),
    ("ship_harrow_lancer", "blaster_bank[spike]"),
    ("ship_harrow_lancer", "torpedo_tube[lance]"),
    ("ship_harrow_patrol", "phaser_bank[port]"),
    ("ship_harrow_patrol", "phaser_bank[starboard]"),
    ("ship_harrow_warhawk", "phaser_bank[port]"),
    ("ship_harrow_warhawk", "phaser_bank[starboard]"),
    ("ship_harrow_warhawk", "blaster_bank[bow_artillery]"),
];

fn is_bespoke(hull: &str, slot: &str) -> bool {
    BESPOKE_DOCTRINES.contains(&(hull, slot))
}

/// **The replacement for the deleted not-equal-to-the-synthesiser assertion.**
///
/// For every one of the fourteen policy kinds: the hulls NOT on
/// [`BESPOKE_DOCTRINES`] must all author the identical configuration (that is
/// the fleet baseline), and every hull that IS on the list must differ from it.
#[test]
fn each_policy_kind_has_one_fleet_baseline_and_exactly_the_bespoke_hulls_depart_from_it() {
    let hulls = ai_hulls();
    assert!(
        hulls.len() >= 2,
        "the fleet is too small to have a baseline"
    );

    // kind → (baseline hull, baseline policy)
    let mut baseline: BTreeMap<String, (String, AiPolicy)> = BTreeMap::new();
    let mut departures: Vec<(String, String)> = Vec::new();
    let mut kinds_seen: BTreeMap<String, usize> = BTreeMap::new();

    // Pass 1: the baseline is whatever the non-bespoke hulls unanimously say.
    for (hull, config) in &hulls {
        for (slot, cfg) in authored_policies(config) {
            *kinds_seen.entry(kind_of(&slot).to_string()).or_insert(0) += 1;
            if is_bespoke(hull, &slot) {
                continue;
            }
            let decoded = policy(&cfg);
            match baseline.get(kind_of(&slot)) {
                None => {
                    baseline.insert(kind_of(&slot).to_string(), (hull.clone(), decoded));
                }
                Some((first_hull, want)) => assert_eq!(
                    &decoded,
                    want,
                    "{hull}/{slot} does not match the fleet baseline that {first_hull} \
                     sets for `{}`. Every hull that is not on BESPOKE_DOCTRINES must \
                     author the identical configuration for a kind — that unanimity IS \
                     the baseline the bespoke doctrines are measured against. If this \
                     hull is meant to fly differently, add it to BESPOKE_DOCTRINES with \
                     a reason; otherwise it is a transcription slip.",
                    kind_of(&slot)
                ),
            }
        }
    }

    // Pass 2: every bespoke pair must actually depart from that baseline.
    for (hull, config) in &hulls {
        for (slot, cfg) in authored_policies(config) {
            if !is_bespoke(hull, &slot) {
                continue;
            }
            let (_, want) = baseline.get(kind_of(&slot)).unwrap_or_else(|| {
                panic!(
                    "{hull}/{slot}: no non-bespoke hull authors `{}`, so there is no \
                     fleet baseline to depart from and this entry proves nothing. \
                     Either the kind lost its baseline hulls or the list is stale.",
                    kind_of(&slot)
                )
            });
            assert_ne!(
                &policy(&cfg),
                want,
                "{hull}/{slot}: this hull is listed as authoring a DELIBERATELY \
                 different policy — a manoeuvre of its own, not the fleet baseline — \
                 and it now decodes to exactly the baseline. That doctrine has been \
                 lost. This is the regression the list exists to catch: a policy that \
                 reverts to the baseline is still a valid policy, so nothing else \
                 fails."
            );
            departures.push((hull.clone(), slot));
        }
    }

    let mut expected: Vec<(String, String)> = BESPOKE_DOCTRINES
        .iter()
        .map(|(h, s)| (h.to_string(), s.to_string()))
        .collect();
    expected.sort();
    departures.sort();
    assert_eq!(
        departures, expected,
        "BESPOKE_DOCTRINES must name exactly the (hull, slot) pairs that exist and \
         depart from the fleet baseline. A stale entry silently stops guarding \
         anything."
    );

    // The fourteen policy kinds are all represented, so no kind slipped out of
    // the comparison by having no authored block anywhere.
    let kinds: Vec<&str> = kinds_seen.keys().map(|s| s.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "blaster_bank",
            "boost",
            "captain",
            "comms_response",
            "engines",
            "impulse",
            "lateral",
            "phaser_bank",
            "power",
            "shields_focus",
            "steering",
            "torpedo_magazine",
            "torpedo_tube",
            "vertical",
        ],
        "all fourteen policy kinds must be authored somewhere in the fleet, or a kind \
         is being compared against nothing"
    );
}

/// …and the five selectors are unanimous across the whole fleet.
///
/// Stage 5b authored all fifty as byte-identical copies of one canonical
/// configuration per kind (#878 will collapse them into shared fragments). Until
/// then this is what stops one of the ten copies drifting: there is no bespoke
/// selector anywhere, so any difference at all is a mistake.
#[test]
fn every_hull_authors_the_same_five_selectors() {
    let mut baseline: BTreeMap<&'static str, (String, TargetSelector)> = BTreeMap::new();
    let mut hulls = 0usize;
    for (hull, config) in ai_hulls() {
        let authored = authored_selectors(&config);
        let names: Vec<&str> = authored.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec!["sensors", "tactical", "navigation", "repair", "comms_hail"],
            "{hull}: an AI-bearing hull must author all five `[*.selector]` blocks. \
             Since #885b stage 5d there is no synthesised stand-in — an omitted \
             selector is a system that simply never ranks anything."
        );
        for (name, cfg) in authored {
            let decoded = selector(&cfg);
            match baseline.get(name) {
                None => {
                    baseline.insert(name, (hull.clone(), decoded));
                }
                Some((first, want)) => assert_eq!(
                    &decoded, want,
                    "{hull}: its `{name}` selector differs from {first}'s. All ten \
                     copies are meant to be identical — no hull authors a bespoke \
                     selector — so this is drift, not doctrine."
                ),
            }
        }
        hulls += 1;
    }
    assert_eq!(baseline.len(), 5, "five selector kinds");
    assert!(hulls >= 2, "the scan found too few hulls to compare");
}

/// The shipped authored block for one policy kind, as authorable TOML.
///
/// **The fixture seam for the rest of the crate's tests.** Before stage 5d a
/// unit test that needed "a valid Captain policy" reached for
/// `default_captain_ai_config()`; with the synthesisers gone the honest
/// replacement is the block a shipped hull actually authors, so a fixture cannot
/// drift away from the content it stands for. Deliberately taken from a
/// non-bespoke hull — see [`fleet_baseline_policy`].
pub(crate) fn shipped_policy_toml(kind: &str) -> FineSystemAiConfigToml {
    for (hull, config) in ai_hulls() {
        for (slot, cfg) in authored_policies(&config) {
            if kind_of(&slot) == kind && !is_bespoke(&hull, &slot) {
                return cfg;
            }
        }
    }
    panic!("no non-bespoke hull authors `{kind}`");
}

/// The shipped authored selector block for one system, as authorable TOML.
/// Sibling of [`shipped_policy_toml`]; `kind` is one of `sensors`, `tactical`,
/// `navigation`, `repair`, `comms_hail`.
pub(crate) fn shipped_selector_toml(kind: &str) -> FineSystemAiSelectorToml {
    for (_, config) in ai_hulls() {
        for (name, cfg) in authored_selectors(&config) {
            if name == kind {
                return cfg;
            }
        }
    }
    panic!("no hull authors the `{kind}` selector");
}

/// The fleet baseline for one policy kind, for the truth tables below.
///
/// Deliberately taken from whichever hulls are NOT bespoke rather than from a
/// named file: the truth tables are about the configuration the fleet actually
/// flies, and pointing them at one hull by name would quietly stop testing the
/// baseline the day that hull gained a doctrine of its own.
fn fleet_baseline_policy(kind: &str) -> AiPolicy {
    policy(&shipped_policy_toml(kind))
}

/// The fleet's one authored selector of a kind, for the invariant pins below.
fn fleet_selector(kind: &str) -> TargetSelector {
    selector(&shipped_selector_toml(kind))
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Guard truth tables — every guard fires AND can read false
// ─────────────────────────────────────────────────────────────────────────────
//
// Re-pointed from the deleted synthesisers onto the shipped authored blocks.
// These were never assertions about synthesis: a guard on an unseeded or
// misspelled fact parses, validates, and reads FALSE for ever, and the only way
// to catch that is to show the guard reading both ways.
//
// Thresholds are read out of the authored `param` map rather than restated as
// literals, so a designer retuning a window in TOML retunes the test with it —
// which is the whole point of the values living in TOML (AGENTS.md rule #11).

fn param(p: &AiPolicy, name: &str) -> f64 {
    p.params
        .get(name)
        .unwrap_or_else(|| panic!("the authored policy declares `{name}`"))
}

/// Captain: in combat within the authored window ⇒ raise; outside it ⇒ stand
/// down; NO combat history at all ⇒ stand down.
///
/// The absent-fact case is the interesting one. `secs_since_combat` is absent
/// when the ship has never been in combat, and an absent fact makes every
/// comparison against it false — so the priority-10 rule loses and the
/// unconditional fallback correctly stands the alert down. The default is never
/// silent.
#[test]
fn captain_guard_truth_table() {
    let p = fleet_baseline_policy("captain");
    let window = param(&p, "combat_window_secs");
    assert!(window > 0.0, "the combat window must be a live threshold");

    assert_eq!(
        resolve(
            &p,
            "red_alert",
            &facts(&[("secs_since_combat", window / 2.0)])
        ),
        Some(AiPolicyVerb::SetRedAlert(true)),
        "inside the authored window ⇒ Red Alert raised."
    );
    assert_eq!(
        resolve(&p, "red_alert", &facts(&[("secs_since_combat", window)])),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "the comparison is STRICTLY less-than, so exactly the window is already \
         outside it ⇒ stand down."
    );
    assert_eq!(
        resolve(
            &p,
            "red_alert",
            &facts(&[("secs_since_combat", window * 3.0)])
        ),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "well outside the window ⇒ stand down."
    );
    assert_eq!(
        resolve(&p, "red_alert", &facts(&[])),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "never been in combat ⇒ the fact is ABSENT ⇒ the guard reads false ⇒ the \
         unconditional fallback stands the alert down."
    );
}

/// Comms response: both clauses must hold, and either one alone can silence the
/// policy.
///
/// `None` here means the ship simply does not answer this tick — distinct from
/// answering with a different index. Both fact names are seeded by
/// `seed_comms_response_facts`, so both guards are live rather than dead.
#[test]
fn comms_response_guard_truth_table() {
    let p = fleet_baseline_policy("comms_response");
    assert_eq!(
        resolve(
            &p,
            "comms_respond",
            &facts(&[("comms_available", 1.0), ("sender_in_range", 1.0)])
        ),
        Some(AiPolicyVerb::RespondToMessage(0)),
        "Comms usable AND sender in range ⇒ answer with response 0."
    );
    assert_eq!(
        resolve(
            &p,
            "comms_respond",
            &facts(&[("comms_available", 0.0), ("sender_in_range", 1.0)])
        ),
        None,
        "a Disabled/Destroyed Comms system stops the ship ANSWERING, not just hailing."
    );
    assert_eq!(
        resolve(
            &p,
            "comms_respond",
            &facts(&[("comms_available", 1.0), ("sender_in_range", 0.0)])
        ),
        None,
        "the router refuses a response whose sender has left comms range; without \
         this clause the AI would re-emit the doomed response every tick for ever."
    );
    assert_eq!(
        resolve(&p, "comms_respond", &facts(&[])),
        None,
        "both facts absent ⇒ no answer. This is the fail-SAFE direction."
    );
}

/// Shields focus: the priority-10 damage rule fires above the authored threshold
/// and not below it — but the priority-0 fallback is unconditional, so the
/// CHANNEL never resolves to `None` either way.
///
/// That is the pin, and it is deliberately blunt: because both rules emit the
/// same value-less `focus_shield_arc` verb, the threshold has NO effect on what
/// the host does with the resolved verb. The arc-ranking kernel runs every tick
/// regardless. The redundancy is recorded on #885 rather than fixed here — a pin
/// that changes what it pins is worthless.
#[test]
fn shields_focus_guard_truth_table() {
    let p = fleet_baseline_policy("shields_focus");
    let threshold = param(&p, "damage_pct_threshold");

    let concentrated = facts(&[("recent_damage_pct_max", threshold + 30.0)]);
    let diffuse = facts(&[("recent_damage_pct_max", threshold - 30.0)]);

    assert!(
        p.rules[0].when.evaluate_with(&concentrated, &p.params, &[]),
        "damage above the authored threshold ⇒ the priority-10 damage rule FIRES."
    );
    assert!(
        !p.rules[0].when.evaluate_with(&diffuse, &p.params, &[]),
        "damage below it ⇒ the priority-10 rule reads FALSE. The fact name must be \
         one the host seeds or this could never be false."
    );

    for (name, snapshot) in [
        ("concentrated", &concentrated),
        ("diffuse", &diffuse),
        ("no damage facts at all", &facts(&[])),
    ] {
        assert_eq!(
            resolve(&p, "shield_focus", snapshot),
            Some(AiPolicyVerb::FocusShieldArc),
            "{name} ⇒ still act, via the unconditional priority-0 fallback. The kernel \
             runs every tick, which is the pre-#783 baseline this policy preserves."
        );
    }
}

/// Power: the elevate rules need BOTH their trigger and their battery reserve;
/// the baseline rules hold the line whenever a battery reading exists at all.
///
/// The brownout property is structural rather than a separate branch: an elevate
/// guard cannot fire below its reserve, so allocation never rises when the
/// battery cannot sustain it.
#[test]
fn power_guard_truth_table() {
    let p = fleet_baseline_policy("power");
    let thrust_threshold = param(&p, "thrust_threshold");
    let helm_reserve = param(&p, "min_reserve_helm");
    let weapons_reserve = param(&p, "min_reserve_weapons");

    let elevated = |level: u8| Some(AiPolicyVerb::SetPowerGroupAllocation(level));
    let (hi, lo) = (3u8, 2u8);

    // ── helm ────────────────────────────────────────────────────────────────
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_reserve + 30.0)
            ])
        ),
        elevated(hi),
        "sustained thrust AND battery above the helm reserve ⇒ elevate."
    );
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_reserve - 20.0)
            ])
        ),
        elevated(lo),
        "same thrust, battery BELOW the reserve ⇒ the elevate guard reads false and \
         helm holds baseline. This is the brownout guard, and it is the reserve param \
         that enforces it — not a global emergency branch."
    );
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("thrust", thrust_threshold - 0.5),
                ("battery_pct", helm_reserve + 30.0)
            ])
        ),
        elevated(lo),
        "thrust below the threshold ⇒ baseline, however full the battery."
    );

    // ── weapons ─────────────────────────────────────────────────────────────
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[("red_alert", 1.0), ("battery_pct", weapons_reserve + 70.0)])
        ),
        elevated(hi),
        "red alert AND battery above the weapons reserve ⇒ elevate."
    );
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[("red_alert", 1.0), ("battery_pct", weapons_reserve - 5.0)])
        ),
        elevated(lo),
        "red alert but battery below the weapons reserve ⇒ baseline."
    );
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[("red_alert", 0.0), ("battery_pct", weapons_reserve + 70.0)])
        ),
        elevated(lo),
        "no red alert ⇒ baseline."
    );

    // ── the absent-battery edge, on both channels ───────────────────────────
    for channel in ["helm", "weapons"] {
        assert_eq!(
            resolve(&p, channel, &facts(&[])),
            None,
            "{channel}: with NO `battery_pct` reading even the baseline rule's guard \
             reads false, so the channel resolves to `None` and the group HOLDS its \
             seeded level. A zero-valued `min_reserve_baseline` does not make the \
             baseline rule unconditional — it still requires the fact to be PRESENT."
        );
    }

    // ── a group the policy never names ──────────────────────────────────────
    assert_eq!(
        resolve(
            &p,
            "sensors",
            &facts(&[("battery_pct", 80.0), ("red_alert", 1.0), ("thrust", 1.0)])
        ),
        None,
        "`sensors` is not named, so it resolves to `None` and holds whatever level the \
         reactor seeded — no matter how favourable the facts."
    );
}

/// The offensive-weapon RED-ALERT fire gate, on all three fire channels, in
/// both directions and on both sides of the fleet (issue #872).
///
/// This is the truth table the three fire channels moved into when
/// [`the_unconditional_baselines_fire_with_no_facts`] stopped being able to
/// claim them. It is deliberately stronger than the entry it replaced: the old
/// one could only say "this guard resolves"; this one says the guard resolves
/// BECAUSE of a fact, by showing the same policy holding when the fact says
/// otherwise, and it does so over the authored threshold rather than a literal.
///
/// The gate is content, not code. Nothing in `beam.rs`, `blaster.rs`,
/// `torpedo.rs` or `console_ai/server.rs` tests red alert to decide firing —
/// they only SEED `fact(red_alert)` — so if this predicate were deleted from
/// the TOML the weapons would fire ungated, which
/// `ai_flag_hosts::removing_the_authored_fire_gate_removes_the_gate` proves on
/// a real shipped hull.
#[test]
fn weapons_fire_guard_truth_table() {
    // (policy kind, fire channel, the verb the channel yields)
    let channels = [
        ("phaser_bank", "phaser_fire", AiPolicyVerb::FirePhaser),
        ("blaster_bank", "blaster_fire", AiPolicyVerb::FireBlaster),
        (
            "torpedo_tube",
            "torpedo_launch",
            AiPolicyVerb::LaunchTorpedo,
        ),
    ];

    // ── The fleet baseline: a hull WITH a captain, so it holds ──────────────
    for (kind, channel, want) in &channels {
        let p = fleet_baseline_policy(kind);
        let threshold = param(&p, "min_alert_to_fire");
        assert_eq!(
            threshold, 1.0,
            "{kind}: a hull with a captain's console authors a threshold of 1 — \
             `fact(red_alert)` is seeded 1/0, so anything else would not be a gate."
        );

        assert_eq!(
            resolve(&p, channel, &facts(&[("red_alert", 1.0)])).as_ref(),
            Some(want),
            "{kind}/{channel}: red alert raised ⇒ fire. Every readiness gate the \
             host owns (cooldown, range, arc, target validity) has already passed \
             by the time the policy is asked."
        );
        assert_eq!(
            resolve(&p, channel, &facts(&[("red_alert", 0.0)])),
            None,
            "{kind}/{channel}: red alert DOWN ⇒ hold, and this is the half that \
             makes the assertion above mean something. The host offered the \
             decision — the weapon is loaded, bearing and in range — and the \
             authored guard is the only thing refusing."
        );
        // Under fire changes nothing: there is no "return fire" leg anywhere in
        // the predicate, which is the point of AC2. A ship being shot at is
        // shot at whether or not its own facts say so, and the only fact that
        // opens this gate is the captain's.
        assert_eq!(
            resolve(
                &p,
                channel,
                &facts(&[
                    ("red_alert", 0.0),
                    ("target_valid", 1.0),
                    ("in_range", 1.0),
                    ("in_arc", 1.0),
                    ("loaded", 1.0),
                    ("tubes_full", 1.0),
                    ("target_facing_shields", 0.0),
                ])
            ),
            None,
            "{kind}/{channel}: every OTHER reading favourable and the alert still \
             down ⇒ hold. Nothing but red alert opens this gate."
        );
        // The absent-fact edge (#779). The host seeds `red_alert`
        // unconditionally, so this case cannot arise at runtime — but a
        // misspelled fact name would present exactly like it, and the answer
        // must be "hold" rather than "fire", i.e. the gate must fail CLOSED.
        assert_eq!(
            resolve(&p, channel, &facts(&[])),
            None,
            "{kind}/{channel}: with no `red_alert` reading at all the comparison \
             reads false and the weapon holds. A guard that failed open here \
             would be a gate that a typo removes."
        );
    }

    // ── The Harrow gun line: always armed, same predicate text ──────────────
    let harrow = entity("ship_harrow_patrol");
    let bank = harrow
        .weapons_console
        .as_ref()
        .expect("the Harrow patrol carries phasers")
        .phaser_banks
        .first()
        .expect("…at least one bank");
    let p = policy(
        bank.ai
            .as_ref()
            .expect("every shipped bank authors a policy"),
    );
    assert_eq!(
        param(&p, "min_alert_to_fire"),
        0.0,
        "the Harrow authors the always-armed threshold"
    );
    assert_eq!(
        p.rules
            .iter()
            .find(|r| r.channel == "phaser_fire")
            .expect("the bank authors a phaser_fire rule")
            .when,
        fleet_baseline_policy("phaser_bank")
            .rules
            .iter()
            .find(|r| r.channel == "phaser_fire")
            .expect("so does the baseline")
            .when,
        "ONE predicate, two thresholds: the Harrow's guard EXPRESSION must be \
         identical to the fleet baseline's. If the two ever diverge into separate \
         doctrines, the claim that this gate is one authored rule serving both \
         sides of the fleet is no longer true."
    );
    for snapshot in [facts(&[("red_alert", 0.0)]), facts(&[("red_alert", 1.0)])] {
        assert_eq!(
            resolve(&p, "phaser_fire", &snapshot),
            Some(AiPolicyVerb::FirePhaser),
            "the Harrow fires with the alert up OR down — it has no captain to \
             raise one, and the threshold of 0 is how the hull says so."
        );
    }
}

/// The unconditional baseline policies fire on an EMPTY fact snapshot.
///
/// This is what "baseline preserving" means for them: the pre-policy hosts
/// actuated every tick with no gate at all, so a `when = "true"` guard must
/// resolve even before any fact is seeded. It is also the #779 empty-facts
/// regression guard — a guard that needed a fact here would silently disable the
/// axis on the first tick, and the host still owns every readiness gate
/// (cooldown, range, arc, availability); the policy only says "permitted".
#[test]
fn the_unconditional_baselines_fire_with_no_facts() {
    for (kind, channel, want) in [
        (
            "engines",
            "longitudinal",
            AiPolicyVerb::ActuateDesiredTravel,
        ),
        ("steering", "yaw", AiPolicyVerb::ActuateDesiredFacing),
        ("lateral", "lateral", AiPolicyVerb::ActuateLateralThrust),
        ("vertical", "vertical", AiPolicyVerb::ActuateVerticalThrust),
        ("impulse", "impulse", AiPolicyVerb::EngageImpulse),
        // The three OFFENSIVE fire channels used to be listed here. Issue #872
        // gated them on an authored red-alert predicate, so they are no longer
        // unconditional by construction and asserting they fire on an empty
        // snapshot would now be asserting the gate away. They moved to
        // [`weapons_fire_guard_truth_table`], which proves the same #779
        // property in the stronger form the gate demands: the guard fires when
        // its fact is seeded, and reads false when it is not.
        //
        // LOADING a tube and GRANTING a round from the magazine stay here.
        // Neither is offensive fire — a tube kept full between engagements is
        // exactly what makes "fire the instant the alert is called" possible —
        // so both remain unconditional and both must still resolve with no
        // facts at all.
        ("torpedo_tube", "torpedo_load", AiPolicyVerb::LoadTorpedo),
        (
            "torpedo_magazine",
            "torpedo_magazine_grant",
            AiPolicyVerb::GrantTorpedoRound,
        ),
    ] {
        let p = fleet_baseline_policy(kind);
        assert_eq!(
            resolve(&p, channel, &facts(&[])).as_ref(),
            Some(&want),
            "{kind}/{channel}: the fleet baseline resolves on an EMPTY fact snapshot."
        );
    }
}

/// Boost is the only baseline that is an explicit IDLE, and the fleet's one
/// bespoke boost doctrine is not.
///
/// "Explicit policy or explicit idle" (#794 AC1) has a different authored shape
/// for each, and getting it backwards is not a validation error — an idle system
/// simply never acts.
#[test]
fn boost_is_the_only_idle_baseline_and_one_hull_departs_from_it() {
    for kind in [
        "captain",
        "comms_response",
        "engines",
        "steering",
        "lateral",
        "vertical",
        "impulse",
        "phaser_bank",
        "blaster_bank",
        "torpedo_tube",
        "torpedo_magazine",
        "shields_focus",
        "power",
    ] {
        assert!(
            !fleet_baseline_policy(kind).idle,
            "{kind}: only the Boost baseline is authored idle; every other kind \
             actively emits."
        );
    }
    assert!(
        fleet_baseline_policy("boost").idle,
        "the Boost baseline is `idle = true` — no AI engages boost by default."
    );
    let destroyer = entity("ship_harrow_destroyer");
    let boost = destroyer
        .helm_console
        .as_ref()
        .and_then(|h| h.boost_ai.as_ref())
        .expect("the Harrow Destroyer authors `[helm_console.boost_ai]`");
    assert!(
        !policy(boost).idle,
        "the Harrow Destroyer is the ONE hull whose AI engages boost — its entry on \
         BESPOKE_DOCTRINES means nothing if the policy is idle."
    );
}

/// An unknown channel resolves to `None` on every authored policy in the fleet.
///
/// Pinned because it is the failure mode of a mistyped channel name: nothing
/// rejects it at the policy level, the system just goes permanently silent.
#[test]
fn an_unknown_channel_resolves_to_nothing_on_every_authored_policy() {
    let mut checked = 0usize;
    for (hull, config) in ai_hulls() {
        for (slot, cfg) in authored_policies(&config) {
            assert_eq!(
                resolve(&policy(&cfg), "not_a_channel", &facts(&[])),
                None,
                "{hull}/{slot}: an unrecognised channel resolves to `None`. A mistyped \
                 channel in authored TOML fails exactly this quietly."
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 141,
        "the fourteen policy kinds account for 141 of the fleet's 191 AI-capable \
         fine-system slots (the other 50 are the selectors). A change in this number \
         means a hull, weapon or kind moved."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Selector INVARIANT pins — the ORDERING, not the numbers
// ─────────────────────────────────────────────────────────────────────────────
//
// For the selectors the specification is relational: "an explicit mission
// objective always beats everything else", "one damage tier always beats the
// whole deficit ladder". Those hold because of how the weights sit RELATIVE to
// one another, and a reweight can leave every individual number looking
// perfectly plausible while inverting the order.
//
// So these pins run the real `TargetSelector::select` over hand-built candidates
// and assert who WINS, against the shipped authored blocks. They are what a
// designer retuning a weight in TOML has to keep satisfying.

/// The #777 additive-stacking invariant, pinned as an ORDERING.
///
/// The selector SUMS weights and one entity commonly carries several source
/// markers at once — the ship's current lock is often also its Sensors
/// designation, and may also be the last attacker and the nearest hostile. A
/// naively-large `retained` weight would let that stack overtake a distinct
/// in-range `objective`, and the ship would refuse to retarget onto its own
/// explicit mission objective.
///
/// Both halves are asserted. The `current: None` case pins that objective wins
/// the raw ranking; the `current: Some(stacked)` case pins that it ALSO
/// overcomes hysteresis retention, which is the half a reweight is most likely
/// to break.
///
/// The objective candidate deliberately carries the LARGER uuid and
/// `hostile = 0`: larger, so the smallest-uuid tie-break cannot be what makes it
/// win; not hostile, so the test simultaneously proves the eligibility guard
/// admits a factionless mission target on `source_objective` alone.
#[test]
fn tactical_objective_beats_the_maximum_non_objective_stack() {
    let sel = fleet_selector("tactical");
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
         clears the switch margin too. This is the assertion that catches a reweight \
         where each individual weight still looks plausible: raise `retained` and the \
         raw ranking above still passes while this one fails, and the ship would sit \
         on its old lock refusing its mission."
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
/// This is the retired tier-2 > tier-3 ordering, and it is the OTHER side of the
/// invariant — the bounded `retained` contribution has to stay large enough to
/// be meaningful while staying small enough to lose to `objective`. Asserted as
/// an ordering so a reweight cannot satisfy one side by sacrificing the other.
///
/// The attacker gets the smaller uuid so the tie-break cannot be the reason.
#[test]
fn tactical_retention_outranks_a_fresh_attacker() {
    let sel = fleet_selector("tactical");
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
        "…and it outranks the attacker on raw score too, so the ordering does not \
         depend on hysteresis to hold."
    );
}

/// AC3 independent revalidation: a FRIENDLY Sensors designation is dropped, not
/// copied — but the same friendly entity IS engageable once a mission names it.
///
/// The Sensors designation is advisory intelligence, so the eligibility guard
/// requires it to be independently `hostile`; the three host-vetted markers
/// (`source_objective` / `source_last_attacker` / `source_retained`) bypass that
/// check. Pinned as behaviour because the guard is a four-branch disjunction and
/// dropping the wrong branch changes which of these cases flips.
#[test]
fn tactical_drops_a_friendly_sensors_designation() {
    let sel = fleet_selector("tactical");
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
/// ever the tie-break. Reproducing that additively requires the whole deficit
/// ladder to stay below one tier step, which is asserted here by making a
/// barely-damaged Disabled station beat an almost-dead Damaged one. A reweight
/// that raised the deficit weight would invert this while each number still
/// looked reasonable, and the AI would start sending teams to nearly-dead minor
/// stations ahead of disabled critical ones.
#[test]
fn repair_one_tier_step_beats_the_whole_deficit_ladder() {
    let sel = fleet_selector("repair");
    let ctx = self_ctx(&[]);
    let pool = vec![
        candidate(
            "aaa-damaged-but-nearly-dead",
            &[
                ("source_repair_request", 1.0),
                ("assigned", 0.0),
                ("tier_ordinal", 1.0),
                ("damage_fraction", 0.99),
            ],
        ),
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
/// Two Disabled stations at 0.96 (three bands) and 0.81 (one band) rank the
/// nearly-dead one first. Two inside the SAME band tie and fall through to the
/// documented smallest-id tie-break, which is deterministic — that determinism
/// is the AC4 property, not an accident.
#[test]
fn repair_deficit_ladder_discriminates_within_a_tier() {
    let sel = fleet_selector("repair");
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
        "within one tier the deeper deficit wins — three bands over one — and it wins \
         from the WORSE uuid, so this is the ladder and not the tie-break. If the \
         bands were realigned to the DamageTier thresholds this would tie instead."
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
/// Eligibility is not a preference, it is a gate: a clause that can never be
/// false is dead code, and a clause on a misspelled fact silently stops the
/// system repairing anything at all. Each case below removes exactly one
/// qualification from an otherwise-perfect candidate.
#[test]
fn repair_eligibility_clauses_each_gate_independently() {
    let sel = fleet_selector("repair");
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
         asymmetry: this is the worst-damaged candidate possible and it is the one the \
         guard refuses."
    );
    assert_eq!(
        pick(&sel, &ctx, &station(&[]), None),
        None,
        "no facts at all ⇒ every clause reads false ⇒ nothing selected. The fail-SAFE \
         direction: an unseeded fact cannot cause a spurious dispatch."
    );
}

/// The Comms band ladder actually RANKS — the property the band placement exists
/// to buy, and the one that silently disappears if the bands move.
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
fn comms_band_ladder_ranks_hails_by_objective_utility() {
    let sel = fleet_selector("comms_hail");
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
    let pool = vec![
        hail("aaa-mission-critical", 100.0),
        hail("bbb-priority", 50.0),
        hail("ccc-routine", 30.0),
        hail("zzz-chatter", 20.0),
    ];

    assert_eq!(
        pick(&sel, &ctx, &pool, None).as_deref(),
        Some("aaa-mission-critical"),
        "three rungs beats two: the mission-critical hail goes first."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool[1..], None).as_deref(),
        Some("bbb-priority"),
        "two rungs beats one."
    );
    assert_eq!(
        pick(&sel, &ctx, &pool[2..], None).as_deref(),
        Some("ccc-routine"),
        "one rung beats none — even though the loser has the SMALLER objective score \
         and would win a uuid tie-break if the ladder had collapsed."
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
fn comms_eligibility_clauses_each_gate_independently() {
    let sel = fleet_selector("comms_hail");
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
        "…and an ABSENT `comms_available` reads false just like a zero one, so a host \
         that stopped seeding it would silently mute every ship's hails."
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
fn sensors_tier_order_and_additive_stacking() {
    let sel = fleet_selector("sensors");
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
        "combat lock ≫ objective ≫ radar: Sensors mirrors what Tactical is engaging \
         so the science console shows the ship's actual fight."
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
         twice. Harmless only because the tiers are orders of magnitude apart."
    );
}

/// Sensors has NO hysteresis band — and yet a tie still favours the incumbent.
///
/// A zero switch margin makes the retention test `cur_score >= best - 0.0`,
/// which is satisfied by an exact tie. So on equal scores the CURRENT target is
/// kept rather than the smallest-uuid rule applying, even though this selector
/// is documented as having no hysteresis. Non-obvious, easy to break by
/// "tidying" the retention comparison to a strict `>`, and it is what stops
/// Sensors flapping between two equally-ranked radar contacts every tick.
#[test]
fn sensors_zero_margin_still_retains_an_exact_tie() {
    let sel = fleet_selector("sensors");
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
        "identical scores ⇒ the incumbent is retained, because a zero switch margin \
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

/// The Sensors eligibility guard is live in the GRAMMAR but unreachable-false in
/// the HOST — the mirror image of the misspelled-fact trap, pinned deliberately.
///
/// A guard on an unseeded or misspelled fact reads false for ever. The Sensors
/// selector has the opposite defect: its guard is structurally always TRUE in
/// production. The host's only candidate constructor
/// (`ship::sensors::detectable_candidate`) hardcodes `detectable = 1` and
/// `hostile = 1` on every candidate it builds, so no candidate that fails the
/// guard can ever reach the selector. The documented hidden/friendly drop (AC4)
/// is enforced upstream by the host's own horizon check and
/// `find_nearest_hostile`, not by this guard.
///
/// Its sibling behaves differently: Tactical's `make_candidate` computes real
/// hostility per candidate, which is why
/// [`tactical_drops_a_friendly_sensors_designation`] can exercise both
/// directions through the real host contract and this cannot.
///
/// Pinned rather than fixed, in BOTH halves: the guard discriminates when it is
/// given a failing candidate, so it is not dead syntax; and it admits everything
/// the host can actually build. Dropping the clauses is behaviour-preserving
/// *today* given what the host feeds it, and would silently remove the only
/// thing that would start filtering the moment a host learned to surface an
/// undetectable or friendly contact.
#[test]
fn sensors_eligibility_is_live_in_the_grammar_but_unreachable_false_in_the_host() {
    let sel = fleet_selector("sensors");
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
        "absent facts read false, so an unseeded candidate is dropped — the fail-SAFE \
         direction, and the reason the host must seed both facts on every candidate \
         rather than relying on defaults."
    );
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
         (`src/ship/sensors.rs`), so in production the guard admits everything it is \
         ever shown. Recorded as current behaviour: do not quietly 'fix' this into \
         something that can fail, nor quietly delete the clauses because they look \
         inert."
    );
}

/// Navigation steers on objectives alone; chart contacts enrich but never steer.
///
/// The eligibility guard admits only `reachable` candidates and only the
/// objective source marks its destination reachable, so a chart contact on its
/// own is invisible to the ranking. When the SAME entity is surfaced by both
/// sources the selector's dedup folds the facts together, and the contact's
/// marker adds its weight to the objective's — which is the entire meaning of
/// "enrich". Pinned as behaviour because the enrichment is a property of the
/// dedup merge, not of anything visible in the authored block.
#[test]
fn navigation_chart_contacts_enrich_but_never_steer() {
    let sel = fleet_selector("navigation");
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

    let coincident = vec![
        destination("aaa-plain-destination"),
        candidate("zzz-both", &[("source_chart_contact", 1.0)]),
        destination("zzz-both"),
    ];
    assert_eq!(
        pick(&sel, &ctx, &coincident, None).as_deref(),
        Some("zzz-both"),
        "a destination the chart ALSO shows outscores a plain destination and wins \
         from the worse uuid. That increment is the only influence chart contacts \
         have under the shipped policy: they break ties between objective \
         destinations, and nothing else."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Declared selector params are the referenced ones
// ─────────────────────────────────────────────────────────────────────────────

/// Every `param` a selector declares is named by a guard, and every `param(...)`
/// a guard names is declared.
///
/// Stage 5b deleted thirteen declared-but-unreferenced params: a `ScoreTerm`
/// carries its own authored `weight`, so a term's magnitude never reaches for
/// `param(...)`, and those thirteen looked like tuning levers while changing
/// nothing. Validation rejects a reference to an UNDECLARED param but nothing
/// rejects an unreferenced declaration, so this is the only thing standing
/// between the fleet and a second crop of inert numbers.
#[test]
fn every_declared_selector_param_is_referenced_by_a_guard() {
    for (hull, config) in ai_hulls() {
        for (name, cfg) in authored_selectors(&config) {
            let mut guards = cfg.eligibility.clone();
            for term in &cfg.score {
                guards.push(' ');
                guards.push_str(&term.when);
            }
            let mut declared: Vec<&String> = cfg.param.keys().collect();
            declared.sort();
            for key in &declared {
                assert!(
                    guards.contains(&format!("param({key})")),
                    "{hull}/{name}: `{key}` is declared and referenced by no guard, so \
                     retuning it changes nothing. Either wire it into a `when` or drop \
                     it — a param that looks like a lever and is not is worse than no \
                     param at all (#885b stage 5b)."
                );
            }
            let mut rest = guards.as_str();
            while let Some(i) = rest.find("param(") {
                rest = &rest[i + "param(".len()..];
                let end = rest.find(')').expect("a `param(` reference closes");
                let referenced = &rest[..end];
                assert!(
                    cfg.param.contains_key(referenced),
                    "{hull}/{name}: a guard references `param({referenced})` which the \
                     block never declares"
                );
                rest = &rest[end..];
            }
        }
    }
}

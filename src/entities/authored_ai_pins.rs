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

use crate::ai::policy::{AiPolicy, AiPolicyState, AiPolicyVerb};
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
///
/// TOP LEVEL ONLY, and deliberately so — the same rule
/// `include_resolve::tests::shipped_tree::shipped_templates` follows. A
/// subdirectory of `assets/entities/` is by convention not shipped content:
/// `fragments/` holds the partial documents hulls compose FROM, and `test/`
/// holds fixtures that exist for one test world (issue #954 put the
/// three-weapon RNG-coverage escort there). Neither is a hull the fleet flies,
/// so neither belongs in a baseline derived from what the fleet unanimously
/// says.
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
    crate::entities::include_resolve::load_entity_config(&key)
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
    push(
        "weapons_doctrine",
        c.weapons_console.as_ref().and_then(|x| x.ai.as_ref()),
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
    // ── The composed class doctrine (issue #875) ─────────────────────────────
    //
    // The player destroyer is the first hull to take its movement from the
    // shared fragment library rather than authoring it inline: it includes
    // `fragments/ai/movement_attack_pass.toml`, which replaces all three travel
    // axes with the posture-gated attack pass, and tunes it by `param` alone.
    //
    // Boost in particular is a real departure and not a side effect. The fleet
    // baseline authors `idle = true` — no AI on an ordinary hull engages the
    // drive — and this hull's escape leg burns it, which is why
    // `boost_is_the_only_idle_baseline_and_two_hulls_depart_from_it` now names
    // two hulls rather than one.
    //
    // The rest of this hull's ship-level declarations come from
    // `fleet_baseline.toml` and `captain_alliance.toml` and are therefore NOT
    // listed here: composing a policy that was already the fleet baseline leaves
    // it the fleet baseline, which is precisely the property that makes the
    // library safe to adopt.
    ("alliance_destroyer", "engines"),
    ("alliance_destroyer", "steering"),
    ("alliance_destroyer", "boost"),
    // ── The composed artillery doctrine (issue #876) ─────────────────────────
    //
    // The player battleship takes `fragments/ai/movement_artillery.toml` by
    // `includes` plus a two-key `param` table and nothing else: it holds a
    // standoff ring while the alert is down and a predictive firing position
    // once it is up.
    //
    // Note what is NOT listed: BOOST. A gun line has no leg that engages the
    // drive, so this hull keeps the fleet baseline's `idle = true` — which is why
    // `boost_is_the_only_idle_baseline_and_two_hulls_depart_from_it` still names
    // two hulls after this issue rather than three. A movement fragment owes
    // `idle = false` only when it supplies a boost machine.
    //
    ("alliance_battleship", "engines"),
    ("alliance_battleship", "steering"),
    // …and IMPULSE, which the artillery fragment authors idle for the same
    // reason `ship_harrow_warhawk` does inline: the authored hold band lies
    // inside the impulse drive's default cruise window, and an engaged drive
    // hard-overrides commanded throttle, so a permitting policy would sail the
    // hull through its own gun line. Two hulls depart from the impulse baseline
    // now, and they depart identically — which the unanimity requirement on the
    // hulls that are NOT listed is what makes meaningful.
    ("alliance_battleship", "impulse"),
    // ── The composed broadside orbit (issue #876 AC1) ────────────────────────
    //
    // The player CRUISER takes `fragments/ai/movement_broadside_orbit.toml` by
    // `includes` plus FOUR `param` keys across TWO tables and nothing else —
    // `combat_orbit_range` and `combat_orbit_speed` on Steering, which is the axis
    // the host reads the fighting ring off, and `torpedo_run_range` on Steering
    // AND Engines, because the guard that reads it exists on both copies of the
    // machine. That buys a wide defensive standoff ring while the alert is down,
    // and at red alert a fighting ring at its own `combat_orbit_range` with a
    // torpedo run cut into it.
    //
    // It was absent from this list until AC1 landed, with a note calling it "one
    // of the hulls the baseline is derived FROM rather than a departure from it".
    // That framing was only ever true of a hull that composed the ship-level
    // spine and NO class movement fragment — it described the withheld state, not
    // a property of the cruiser — and it does not survive the doctrine shipping.
    // Both travel axes are now machines, so the hull is a departure and has to be
    // named as one, in both directions: a broadside orbit that quietly collapsed
    // back onto the fleet's stateless `actuate_desired_travel` would still
    // validate, still spawn and still report every station crewed, and only this
    // entry notices.
    //
    // Note what is NOT listed, and it is the same pair the artillery doctrine
    // leaves alone for different reasons. BOOST: no leg of a ring engages the
    // drive, so the hull keeps the baseline's `idle = true` and
    // `boost_is_the_only_idle_baseline_and_two_hulls_depart_from_it` still names
    // two hulls. IMPULSE: a fighting ring is tens of units across, comfortably
    // inside the drive's own 40-unit cancel radius, so the autopilot releases
    // before the ring begins — unlike the artillery band, which sits inside the
    // cruise window and is why THAT fragment has to idle the drive. Two hulls
    // depart from the impulse baseline, not three.
    ("alliance_cruiser", "engines"),
    ("alliance_cruiser", "steering"),
    // ── The composed HARROW doctrines (issue #878) ───────────────────────────
    //
    // These three used to author their manoeuvres inline, hull by hull; they now
    // take them from the SAME three fragments the player hulls fly and tune them
    // by `param` alone. The entries do not change, and that is the finding rather
    // than an accident: a departure from the fleet baseline is a departure
    // whether it was typed into the hull or composed into it, so the list still
    // names exactly the slots that fly a machine — and would still notice one
    // collapsing back onto the fleet's stateless defaults, which is now one
    // dropped `includes` line away rather than several hundred deleted ones.
    //
    // What the three of them share is `press_posture = 0.0`, the lowest rung of
    // the posture ladder: the class doctrines rest on a standoff ring until their
    // captain calls red alert, and a Harrow has no captain to call one. That
    // single parameter is what makes the composed machine reduce to the inline
    // one, and `the_harrow_hulls_unlock_their_class_doctrine_by_posture_alone`
    // is what proves it in both directions.
    //
    // The broadside orbit: a stateful engines/steering pair, plus two tubes that
    // hold fire until the target's striking arc is actually down. The MANOEUVRE's
    // matching reading now rides `torpedo_run_shield_gap = 1.0` on the shared
    // fragment (see `the_harrow_cruiser_breaks_its_ring_only_for_a_struck_down_arc`)
    // rather than a bespoke guard; the TUBES keep theirs, because a launch gate is
    // a property of the loadout.
    ("ship_harrow_cruiser", "engines"),
    ("ship_harrow_cruiser", "steering"),
    ("ship_harrow_cruiser", "torpedo_tube[bow_port]"),
    ("ship_harrow_cruiser", "torpedo_tube[bow_starboard]"),
    // The lance run, and the one hull in the fleet whose AI engages boost. It is
    // also the only hull that flies the attack pass's PRESSED short-pass loop
    // (#789), which #878 moved into the fragment as an opt-in leg: the class
    // default authors `pressed_min_progress` at a value no separation reading can
    // fall under and withholds `pressed_window_ticks` entirely, so the arm is
    // declined twice over for a hull that has not asked for it.
    ("ship_harrow_destroyer", "engines"),
    ("ship_harrow_destroyer", "steering"),
    ("ship_harrow_destroyer", "boost"),
    // The artillery platform: it holds position rather than closing, which is
    // why its impulse axis is IDLE rather than permitting. That declaration now
    // arrives with `movement_artillery.toml` instead of being authored on the
    // hull — switching the drive off is a property of the DOCTRINE, since the
    // hold band lies inside the impulse cruise window on every hull small enough
    // to want one — so two hulls depart from the impulse baseline identically.
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
    // ── The one hull that presents a different gun (issue #956) ──────────────
    //
    // Which weapon family a ship TURNS TO BRING TO BEAR used to be a Rust array
    // — `[Phasers, Blasters, Torpedoes]` in `tick_weapons_arc_request` — and is
    // now `[weapons_console.ai]`, three rank channels naming a family each. The
    // fleet baseline authors that same order unconditionally, so nothing about
    // how the fleet flies moved with the decision.
    //
    // This hull is the departure and it is the issue's own worked example:
    // while the arc a round from it would strike is not blocking, the family
    // worth turning for is its 24-degree bow TUBES, and it falls back to the
    // fleet order term for term once the gap closes. That conditional is the
    // whole reason the order stopped being a constant, so it is exactly the
    // regression this list exists to catch: a doctrine that quietly collapsed
    // back onto the baseline would still validate, still spawn, still fly its
    // ring — and would simply never present its tubes again.
    ("ship_harrow_cruiser", "weapons_doctrine"),
    ("ship_harrow_destroyer", "blaster_bank[harrow-lance-port]"),
    (
        "ship_harrow_destroyer",
        "blaster_bank[harrow-lance-starboard]",
    ),
    ("ship_harrow_patrol", "phaser_bank[port]"),
    ("ship_harrow_patrol", "phaser_bank[starboard]"),
    ("ship_harrow_warhawk", "phaser_bank[port]"),
    ("ship_harrow_warhawk", "phaser_bank[starboard]"),
    ("ship_harrow_warhawk", "blaster_bank[bow_artillery]"),
    // ── The captains that do NOT open engagements (issue #912) ───────────────
    //
    // The fleet-baseline Captain policy raises Red Alert on two independent
    // readings: recent combat (`secs_since_combat`, the returning-fire half) and
    // a hostile inside the authored `alert_on_hostile_within` (the first-contact
    // half #912 added, without which a backfilled Alliance hull whose guns are
    // gated on the alert could only ever return fire).
    //
    // These five author the first rule and NOT the second, and it is the same
    // doctrine as the always-armed gun line above rather than an oversight. A
    // Harrow needs no captain's permission to shoot — its banks author
    // `min_alert_to_fire = 0` — so giving its captain a first-contact rule would
    // change the hull's red-alert STATE (and with it every fact, comms and
    // repair reading keyed on it, and #874's arc overlay) while changing nothing
    // about when it fires: a real behavioural change dressed up as a no-op. The
    // Requiem courier is unarmed and merely carries the same stand-down doctrine.
    //
    // Listed here in the direction that keeps the entry meaningful: if one of
    // these five quietly acquires the first-contact rule it collapses onto the
    // baseline and this list notices, and if an Alliance hull quietly loses it
    // the unanimity requirement notices instead.
    ("ship_harrow_cruiser", "captain"),
    ("ship_harrow_destroyer", "captain"),
    ("ship_harrow_patrol", "captain"),
    ("ship_harrow_warhawk", "captain"),
    ("ship_requiem_courier", "captain"),
    // The civilian hauler (issue #1028) departs further than any of the five
    // above: its one captain rule is `when = "true"` → `set_red_alert(false)`,
    // an UNCONDITIONAL stand-down. That is the doctrine, not a stub. A hauler
    // has no guns for an alert to arm, and raising one would still change the
    // posture every fact, comms and repair reading is keyed on — so the honest
    // declaration is that it never raises one, said here rather than by quietly
    // matching a baseline it does not mean.
    //
    // Listed in the direction that keeps the entry meaningful: a hauler that
    // acquired the returning-fire rule would be a civilian craft that goes to
    // battlestations, and only this list would notice.
    ("ship_civilian_hauler", "captain"),
    // ── The reactors whose drive is not gated by the alert ───────────────────
    //
    // These five author `power` inline and their `helm` ELEVATE reads
    // `thrust >= thrust_threshold and battery_pct >= min_restore_helm`, and the
    // HOLD above it reads `battery_pct >= min_reserve_helm` instead — both with
    // NO `red_alert` term; the fleet baseline's carries one on both. That guard
    // exists because `plan_helm_travel` commands near-max throttle for any ordinary
    // transit, so an ungated rule holds an Alliance hull's drive elevated for
    // its whole cruise and browns the reactor out with no combat involved.
    //
    // The reactor arithmetic is NOT what distinguishes them, and it is worth
    // saying so because an earlier revision of this note claimed it was. An
    // Alliance hull authors the same canonical trio at `helm 2 + weapons 2 +
    // shields 2` = 6; these five author no `[power_groups.*]` at all, so
    // `PowerSystem::from_authored_groups` seeds that same trio at level 2 —
    // also 6. Elevating helm puts BOTH at 7, and `PowerSystem::tick` indexes
    // `config.rates[total - 3]`, which is -2 on the fleet baseline's
    // `[5, 4, 3, 2, -2, -5]` and -2 on the patrol's `[6, 5, 4, 2, -2, -6]`
    // alike. The same three groups cost the same drain either way.
    //
    // What distinguishes them is WHEN THE ALERT IS UP. An Alliance captain
    // raises it on first contact (#912), so an alert-gated helm rule still
    // releases the drive for the whole engagement. These five author the
    // returning-fire rule ONLY — their captains raise the alert inside a short
    // `combat_window_secs` of the last exchange of fire and stand it down
    // otherwise, which is also why their guns are always-armed
    // (`min_alert_to_fire = 0`, see the gun line above). A Harrow spends almost
    // all of its life in transit with the alert down, so inheriting the guard
    // would pin its drive at 2 for exactly the leg that needs it and release it
    // only once the shooting had already started.
    //
    // These entries were first added by #923 with a different reason (the
    // `sensors` channel the Alliance baseline then carried). #955 removed that
    // channel again, so the departure is now the helm guard alone — a narrower
    // claim, and the one that is actually true of the shipped files.
    //
    // Listed in the direction that keeps the entry meaningful: if one of these
    // quietly acquires the alert guard its transit doctrine has changed and only
    // this list notices, and if an Alliance hull quietly loses it the unanimity
    // requirement notices instead.
    ("ship_harrow_cruiser", "power"),
    ("ship_harrow_destroyer", "power"),
    ("ship_harrow_patrol", "power"),
    ("ship_harrow_warhawk", "power"),
    ("ship_requiem_courier", "power"),
];

fn is_bespoke(hull: &str, slot: &str) -> bool {
    BESPOKE_DOCTRINES.contains(&(hull, slot))
}

/// **The replacement for the deleted not-equal-to-the-synthesiser assertion.**
///
/// For every one of the fifteen policy kinds: the hulls NOT on
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

    // The fifteen policy kinds are all represented, so no kind slipped out of
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
            "weapons_doctrine",
        ],
        "all fifteen policy kinds must be authored somewhere in the fleet, or a kind \
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

    // ── First contact (issue #912) ──────────────────────────────────────────
    //
    // The half that does not depend on having been shot at yet. `operate_captain_ai`
    // seeds BOTH readings unconditionally, so every row below is a snapshot the
    // host really produces rather than a synthetic one.
    let reach = param(&p, "alert_on_hostile_within");
    assert!(reach > 0.0, "the engagement reach must be a live threshold");

    assert_eq!(
        resolve(
            &p,
            "red_alert",
            &facts(&[("hostile_contact", 1.0), ("hostile_range", reach / 2.0)])
        ),
        Some(AiPolicyVerb::SetRedAlert(true)),
        "a hostile inside the authored reach ⇒ Red Alert raised, with NO combat \
         history at all. This is the row #912 exists for: `secs_since_combat` is \
         absent — nobody has fired yet, and since #872 this hull's guns cannot \
         fire until the alert is up — so if this read false the hull could only \
         ever return fire and the aggressive half of every class doctrine would \
         be unreachable without a human captain."
    );
    assert_eq!(
        resolve(
            &p,
            "red_alert",
            &facts(&[("hostile_contact", 1.0), ("hostile_range", reach)])
        ),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "the comparison is STRICTLY less-than, so a contact exactly at the \
         authored reach is still outside it ⇒ stand down."
    );
    assert_eq!(
        resolve(
            &p,
            "red_alert",
            &facts(&[("hostile_contact", 1.0), ("hostile_range", reach * 3.0)])
        ),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "a hostile well beyond the reach ⇒ stand down. Presence alone never \
         raises the alert — the range clause is what makes the reach a designer's \
         lever rather than decoration."
    );
    // The genuine no-contact snapshot: the host seeds `hostile_range` as 0.0
    // when it found nobody, so WITHOUT the presence clause this row would read
    // "a hostile at range zero" and every ship in the fleet would sit at
    // permanent Red Alert. This is the row that makes `hostile_contact`
    // load-bearing.
    assert_eq!(
        resolve(
            &p,
            "red_alert",
            &facts(&[("hostile_contact", 0.0), ("hostile_range", 0.0)])
        ),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "no contact ⇒ stand down, even though the always-seeded range reads 0.0 \
         and would otherwise satisfy the threshold on its own."
    );

    // ── The captains that deliberately do NOT open engagements ──────────────
    //
    // The other direction, on a real Harrow: same first-contact snapshot, and it
    // still stands down, because the hull authors no such rule. Its guns are
    // ungated (`min_alert_to_fire = 0`), so an alert would buy it nothing and
    // would move every other reading keyed on red alert for free.
    let harrow = entity("ship_harrow_patrol");
    let hp = policy(
        harrow
            .captain_console
            .as_ref()
            .and_then(|c| c.ai.as_ref())
            .expect("the Harrow patrol authors `[captain_console.ai]`"),
    );
    assert!(
        hp.params.get("alert_on_hostile_within").is_none(),
        "a Harrow must not carry the first-contact reach at all — its entry on \
         BESPOKE_DOCTRINES says the rule is absent, not merely retuned."
    );
    assert_eq!(
        resolve(
            &hp,
            "red_alert",
            &facts(&[("hostile_contact", 1.0), ("hostile_range", reach / 2.0)])
        ),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "the Harrow sees the same hostile at the same range and stands down. It \
         is always armed, so first contact is not its captain's decision."
    );
    assert_eq!(
        resolve(
            &hp,
            "red_alert",
            &facts(&[("secs_since_combat", window / 2.0)])
        ),
        Some(AiPolicyVerb::SetRedAlert(true)),
        "…and its returning-fire half is untouched, so this is an ABSENT rule \
         rather than a broken policy."
    );
}

/// The shipped cruiser as TEXT, for the mutation proof below — the RESOLVED
/// document rather than the file (issue #876).
///
/// It was `include_str!` on the argument that the mutation models an edit a
/// designer would make to that file. Since #876 the cruiser is COMPOSED and the
/// rule being deleted lives in `fragments/ai/captain_alliance.toml`, so the file
/// a designer would edit is the FRAGMENT — and the resolved document is where
/// that edit lands. It is also the only text `EntityConfig::from_toml` will
/// accept, since the raw file now carries an `includes` key the parser rejects.
fn cruiser_toml() -> String {
    let path = crate_root().join("assets/entities/alliance_cruiser.toml");
    let key = path.to_string_lossy().replace('\\', "/");
    crate::entities::include_resolve::resolve_from_disk(&key)
        .expect("alliance_cruiser must compose")
        .toml
}

/// The whole first-contact rule (issue #912), rule header included, because
/// deleting a rule is what a designer choosing a passive hull actually does.
///
/// Spelled as the RESOLVED document renders it (issue #876), not as
/// `fragments/ai/captain_alliance.toml` authors it: a composed template is
/// re-serialised through `toml::to_string`, which orders a table's keys
/// alphabetically. `without_block` asserts the text is present exactly once, so
/// a rendering change fails loudly here rather than silently deleting nothing.
const FIRST_CONTACT_RULE: &str = "\n[[captain_console.ai.rule]]\nchannel = \"red_alert\"\npriority = 5\nvalue = true\nverb = \"set_red_alert\"\nwhen = \"fact(hostile_contact) > 0 and fact(hostile_range) < param(alert_on_hostile_within)\"\n";

/// Delete one authored block from a shipped hull, asserting it was actually
/// there — a silently-missed removal would turn the mutation below into a
/// vacuous pass (the same reason `ai_flag_hosts::with_guard` asserts its count).
fn without_block(hull: &str, block: &str) -> String {
    assert_eq!(
        hull.matches(block).count(),
        1,
        "the block being removed must appear exactly once in the hull"
    );
    hull.replace(block, "\n")
}

/// The Captain policy a hull's TOML text decodes to, through the real load path.
fn captain_policy_of(toml: &str) -> AiPolicy {
    let config = EntityConfig::from_toml(toml).expect("the hull loads");
    policy(
        config
            .captain_console
            .as_ref()
            .and_then(|c| c.ai.as_ref())
            .expect("the hull authors `[captain_console.ai]`"),
    )
}

/// **AC4, the data-driven proof.** Delete the first-contact rule from a real
/// shipped hull's TOML and the hull goes back to return-fire-only — so the
/// aggression is data, not code.
///
/// This is a *resolution* assertion rather than a load assertion, which is why
/// it lives beside the truth table rather than in `ai_flag_hosts`: the mutated
/// hull still loads perfectly well (a captain with only the returning-fire rule
/// is valid content — five Harrows and the Requiem courier ship exactly that),
/// and the whole claim is about what it then DECIDES.
///
/// Nothing in `src/console/captain/server.rs` tests hostility, range or alert
/// state to decide anything: it seeds `hostile_contact` and `hostile_range` and
/// hands both to `resolve_channel`. So if this predicate is removed from the
/// TOML there is no Rust path left that could raise the alert on first contact —
/// which is what the two halves below show.
#[test]
fn removing_the_authored_first_contact_rule_restores_return_fire_only() {
    let cruiser = cruiser_toml();
    let shipped = captain_policy_of(&cruiser);
    let reach = param(&shipped, "alert_on_hostile_within");
    let window = param(&shipped, "combat_window_secs");
    let first_contact = facts(&[("hostile_contact", 1.0), ("hostile_range", reach / 2.0)]);
    let recent_combat = facts(&[("secs_since_combat", window / 2.0)]);

    assert_eq!(
        resolve(&shipped, "red_alert", &first_contact),
        Some(AiPolicyVerb::SetRedAlert(true)),
        "as shipped, the cruiser opens an engagement on a hostile inside its \
         authored reach."
    );

    let passive = captain_policy_of(&without_block(&cruiser, FIRST_CONTACT_RULE));
    assert_eq!(
        resolve(&passive, "red_alert", &first_contact),
        Some(AiPolicyVerb::SetRedAlert(false)),
        "with the rule deleted the SAME hostile at the SAME range no longer \
         raises the alert — and since #872 the hull's guns are gated on that \
         alert, so it is back to return-fire-only. If this still read true, the \
         aggression would be coming from Rust rather than from the TOML."
    );
    assert_eq!(
        resolve(&passive, "red_alert", &recent_combat),
        Some(AiPolicyVerb::SetRedAlert(true)),
        "…and the returning-fire half survives the deletion untouched, so what \
         was removed is the first-contact decision and nothing else."
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

// ─────────────────────────────────────────────────────────────────────────────
// The movement POSTURE gate (issue #875)
// ─────────────────────────────────────────────────────────────────────────────

/// The three copies of the composed attack-pass machine, keyed by the axis they
/// fly. Read off the SHIPPED hull, so a retune in TOML retunes this pin with it.
fn attack_pass_policies() -> Vec<(&'static str, AiPolicy)> {
    let hull = entity("alliance_destroyer");
    let helm = hull
        .helm_console
        .as_ref()
        .expect("the player destroyer authors `[helm_console]`");
    vec![
        (
            "engines",
            policy(
                helm.engines_ai
                    .as_ref()
                    .expect("composed from movement_attack_pass.toml"),
            ),
        ),
        (
            "steering",
            policy(
                helm.steering_ai
                    .as_ref()
                    .expect("composed from movement_attack_pass.toml"),
            ),
        ),
        (
            "boost",
            policy(
                helm.boost_ai
                    .as_ref()
                    .expect("composed from movement_attack_pass.toml"),
            ),
        ),
    ]
}

/// A memory bag with the machine's authored initial values and a state clock
/// reading, for resolving a transition the way the host does.
fn machine_memory(p: &AiPolicy, state_time_secs: f64) -> crate::world::flags::AiPolicyMemory {
    let mut m = p
        .machine()
        .expect("a stateful policy")
        .initial_memory
        .clone();
    m.set_state_time_secs(state_time_secs);
    m
}

/// **The posture truth table (AC2).** The gate fires at red alert AND reads
/// FALSE — not absent — when the alert is clear, on all three axes of the
/// composed class doctrine.
///
/// This is the assertion the whole feature rests on, and it is the #779 shape:
/// `posture` is a fact name, so a misspelling here or in
/// `helm_ai::seed_helm_actuator_facts` parses, validates, and reads false for
/// ever — which presents as a destroyer that shadows politely and never once
/// attacks, with every other test still green. Proving the guard reads BOTH ways
/// against a really-seeded name is the only thing that catches it.
///
/// The threshold comes out of the authored `param` rather than a literal, so a
/// designer who adds an intermediate rung to the posture ladder and retunes
/// `press_posture` retunes this test with it (AGENTS.md rule #11).
///
/// The host-side half — that the fact is really seeded, on every one of the
/// seven policy hosts, and unconditionally — is
/// `helm_ai::tests::posture_is_seeded_unconditionally_from_red_alert`.
#[test]
fn posture_guard_truth_table() {
    for (axis, p) in attack_pass_policies() {
        let press = param(&p, "press_posture");
        assert!(
            press > 0.0,
            "{axis}: `press_posture` must be a live threshold, not zero — at zero \
             the defensive rung (0.0) would satisfy `>=` and the gate would be open \
             for ever."
        );
        let machine = p.machine().expect("{axis} is a state machine");
        assert_eq!(
            machine.initial, "shadow",
            "{axis}: the doctrine RESTS defensive. A machine that booted into an \
             aggressive leg would press once before the first posture reading \
             arrived."
        );

        let pressed = facts(&[("posture", press)]);
        let clear = facts(&[("posture", press - 1.0)]);
        let unseeded = facts(&[]);
        let memory = machine_memory(&p, 0.0);

        // ── shadow: the alert opens the gate, and nothing else does ──────────
        assert_eq!(
            p.resolve_transition("shadow", &pressed, &memory, &[])
                .map(|t| t.to.as_str()),
            Some("acquire"),
            "{axis}/shadow: red alert ⇒ the class doctrine is licensed and the hull \
             leaves the standoff ring."
        );
        assert_eq!(
            p.resolve_transition("shadow", &clear, &memory, &[]),
            None,
            "{axis}/shadow: alert DOWN ⇒ hold the ring. This is the half that makes \
             the assertion above mean something — without it `shadow` could be a \
             state the hull leaves unconditionally."
        );
        assert_eq!(
            p.resolve_transition("shadow", &unseeded, &memory, &[]),
            None,
            "{axis}/shadow: with NO posture reading at all the comparison reads false \
             and the hull stays defensive. The gate fails CLOSED, so a typo in the \
             fact name cannot make a hull aggressive."
        );

        // ── acquire / inbound: the alert going down breaks the hull off ──────
        for state in ["acquire", "inbound"] {
            assert_eq!(
                p.resolve_transition(state, &clear, &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("shadow"),
                "{axis}/{state}: the alert going down outranks everything else in \
                 this leg — a hull whose captain has stood down must not press home \
                 a run it is no longer licensed to make."
            );
            assert_ne!(
                p.resolve_transition(state, &pressed, &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("shadow"),
                "{axis}/{state}: at red alert the break-off guard must NOT fire, or \
                 the doctrine could never reach the merge at all."
            );
        }

        // ── escape: the commitment is not cut short, by posture or anything ──
        //
        // The one leg where a posture drop is deliberately DEFERRED. The escape
        // flies a frozen heading and the doctrine's own invariant is that only
        // the authored dwell ends it; the posture branch shares the `state_time`
        // conjunct with the other two so it cannot shorten the commitment.
        let dwell = param(&p, "escape_duration_secs");
        let mid_escape = machine_memory(&p, dwell * 0.5);
        let dwell_done = machine_memory(&p, dwell);
        assert_eq!(
            p.resolve_transition("escape", &clear, &mid_escape, &[]),
            None,
            "{axis}/escape: the alert going down MID-escape changes nothing. \
             Committing to the outward heading means the dwell runs; a hull that \
             turned here would curl back through the target it just passed."
        );
        assert_eq!(
            p.resolve_transition("escape", &clear, &dwell_done, &[])
                .map(|t| t.to.as_str()),
            Some("shadow"),
            "{axis}/escape: at the END of the dwell a dropped alert wins — and it \
             outranks both the recovery branch and the next-pass branch, which are \
             the other two things the dwell can end into."
        );
        assert_eq!(
            p.resolve_transition("escape", &pressed, &dwell_done, &[])
                .map(|t| t.to.as_str()),
            Some("acquire"),
            "{axis}/escape: still at red alert with the shields up ⇒ line up for \
             another pass."
        );
    }
}

/// **The pressed short-pass loop's DECLINING half, on the player destroyer
/// (issue #789, generalised by #878).**
///
/// `alliance_destroyer` composes the same `movement_attack_pass.toml` fragment
/// `ship_harrow_destroyer` does, but never authors a real `pressed_min_progress`
/// or a `pressed_window_ticks` on top of it — so it gets the class default, and
/// the class default's whole claim is that the pressed branch cannot win. The
/// Harrow opts IN to both scalars (`ship_harrow_destroyer.toml`); this hull
/// declining both is the other half of the same doctrine, and until now nothing
/// resolved the escape leg against a fact set that actually FAVOURS the pressed
/// branch to check it still loses.
///
/// First the structural half: `pressed_min_progress` stays below zero — the
/// fragment's own comment on the value is that a threshold this far under zero
/// is one no separation reading can ever fall under — and `pressed_window_ticks`
/// is simply absent, which is the second, independent opt-in the host reads off
/// the STEERING axis by name to publish the arm at all (see that axis's own
/// comment in the fragment). Either one alone declines the loop; this hull
/// declines both.
///
/// Then the behavioural half, resolved through the real transition resolver at
/// the end of the dwell with every OTHER conjunct of the pressed branch set to
/// favour it — shields spent, still inside the target's own reach, zero net
/// separation — so `separation_progress < pressed_min_progress` is the only
/// conjunct left that can fail. On the class default it does, and recovery wins
/// by priority instead. A hull that quietly inherited a real threshold (or
/// `pressed_window_ticks`) would reach `pressed_pivot` here instead, and only
/// this assertion would notice.
#[test]
fn the_player_destroyer_declines_the_pressed_short_pass_loop() {
    let steering = attack_pass_policies()
        .into_iter()
        .find(|(axis, _)| *axis == "steering")
        .expect("the destroyer composes a steering policy from the fragment")
        .1;

    assert!(
        param(&steering, "pressed_min_progress") < 0.0,
        "the player destroyer's steering axis must keep the class default below \
         zero, or the pressed branch's guard could actually be reachable by a \
         real separation reading"
    );
    assert!(
        steering.params.get("pressed_window_ticks").is_none(),
        "the player destroyer must NOT author `pressed_window_ticks` — the host \
         reads it off this axis by name to publish the pressed arm at all, so \
         authoring it (even alongside a declined threshold) would turn the arm \
         on for a hull that never asked for it"
    );

    let dwell = param(&steering, "escape_duration_secs");
    let dwell_done = machine_memory(&steering, dwell);
    // Every conjunct but the pressed one is set to FAVOUR the pressed branch:
    // posture still pressed, shields spent, still inside the target's own
    // reach, and no ground gained. If the pressed transition were ever
    // reachable on this hull, this is the fact set that would reach it.
    let pressed_favouring = facts(&[
        ("posture", param(&steering, "press_posture")),
        ("shield_fraction", 0.0),
        ("inside_threat_range", 1.0),
        ("separation_progress", 0.0),
    ]);
    assert_eq!(
        steering
            .resolve_transition("escape", &pressed_favouring, &dwell_done, &[])
            .map(|t| t.to.as_str()),
        Some("recover"),
        "the player destroyer's escape leg must fall through to ordinary \
         recovery even when every other conjunct of the pressed branch is \
         satisfied — the declined threshold is what has to stop it, and a \
         `pressed_pivot` here means the class default has quietly become live"
    );
}

/// **The targeting half of issue #875 AC5, on the COMPOSED hull.** A Sensors
/// designation still redirects the backfilled destroyer's guns, and still loses
/// to a named mission objective.
///
/// PRD #774 stories 10/11 are inherited rather than new — the designation
/// reaches Tactical as an advisory channel-3 candidate carrying
/// `source_sensors_designation`, weighted 500 against a radar contact's 1 — and
/// the point of asserting it here is that this hull's Tactical selector is now
/// COMPOSED. Its whole ranking arrives through `includes`, so an override that
/// used to be authored in the file is now a property of a merge, and a merge
/// that dropped the selector would leave a hull that still validates, still
/// spawns, and quietly ignores its own crew.
///
/// Run through the real `TargetSelector::select` against the shipped block, and
/// both directions are asserted: the designation must WIN over the radar
/// contact, and must LOSE to a mission objective, or "advisory" would be the
/// wrong word for it.
#[test]
fn a_sensors_designation_still_redirects_the_composed_destroyers_guns() {
    let hull = entity("alliance_destroyer");
    let sel = selector(
        hull.weapons_console
            .as_ref()
            .expect("the destroyer carries weapons")
            .selector
            .as_ref()
            .expect("its Tactical selector is composed from the library"),
    );
    let ctx = self_ctx(&[]);

    // The designation carries the LARGER uuid, so the smallest-uuid tie-break
    // cannot be what makes it win.
    let radar_contact = candidate(
        "aaa-nearest-hostile",
        &[("detectable", 1.0), ("hostile", 1.0), ("source_radar", 1.0)],
    );
    let designated = candidate(
        "zzz-designated-by-sensors",
        &[
            ("detectable", 1.0),
            ("hostile", 1.0),
            ("source_sensors_designation", 1.0),
        ],
    );
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[radar_contact.clone(), designated.clone()],
            None
        )
        .as_deref(),
        Some("zzz-designated-by-sensors"),
        "the crew's designation must beat the AI's own nearest-hostile pick, or a \
         backfilled destroyer cannot be told what to shoot at."
    );
    // …and it must still overcome hysteresis retention of the AI's own pick,
    // which is the half a reweight is most likely to break: the switch margin
    // is applied against the CURRENT lock.
    assert_eq!(
        pick(
            &sel,
            &ctx,
            &[radar_contact.clone(), designated.clone()],
            Some("aaa-nearest-hostile")
        )
        .as_deref(),
        Some("zzz-designated-by-sensors"),
        "a designation issued while the ship is already locked on must still \
         redirect it — 'at any time' means mid-engagement."
    );
    // The advisory half: a named mission objective outranks it.
    let objective = candidate(
        "mmm-mission-objective",
        &[("detectable", 1.0), ("source_objective", 1.0)],
    );
    assert_eq!(
        pick(&sel, &ctx, &[designated, objective], None).as_deref(),
        Some("mmm-mission-objective"),
        "the designation is ADVISORY: it redirects the ship's own choice, not a \
         mission order."
    );
}

/// The composed doctrine is genuinely POSTURE-gated: every aggressive leg
/// carries a way back to the defensive one.
///
/// A separate pin from the truth table because it is a structural claim about
/// the graph rather than about one guard. A leg that gained an aggressive
/// transition but no break-off — easy to do when adding a state — would trap an
/// unmanned hull in a fight its captain has called off, and no per-guard table
/// would notice.
#[test]
fn every_aggressive_leg_of_the_class_doctrine_can_return_to_the_defensive_one() {
    for (axis, p) in attack_pass_policies() {
        let machine = p.machine().expect("a state machine");
        for state in &machine.states {
            if state.id == "shadow" {
                continue;
            }
            assert!(
                state.transitions.iter().any(|t| t.to == "shadow"),
                "{axis}/{}: an aggressive leg with no transition back to `shadow`. \
                 A captain standing the alert down would leave the hull pressing an \
                 attack it is no longer licensed to make, for ever.",
                state.id
            );
        }
        // …and the reverse: the defensive leg's ONLY way out is the posture gate.
        let shadow = machine.state("shadow").expect("the defensive leg");
        assert_eq!(
            shadow.transitions.len(),
            1,
            "{axis}/shadow: the standoff leg must have exactly one exit, and it must \
             be the posture gate. A second exit is a way to start a fight without \
             the captain."
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The composed BROADSIDE-ORBIT doctrine (issue #876 AC1)
// ─────────────────────────────────────────────────────────────────────────────

/// The two copies of the composed broadside-orbit machine, keyed by the axis they
/// fly. Read off the SHIPPED hull, so a retune in TOML retunes these pins with it.
///
/// Two, not three: no leg of a ring engages the boost drive, so
/// `fragments/ai/movement_broadside_orbit.toml` leaves `[helm_console.boost_ai]`
/// alone and this hull keeps the fleet baseline's `idle = true` — which is why
/// `boost_is_the_only_idle_baseline_and_two_hulls_depart_from_it` still names two
/// hulls after AC1 rather than three.
fn broadside_orbit_policies() -> Vec<(&'static str, AiPolicy)> {
    let hull = entity("alliance_cruiser");
    let helm = hull
        .helm_console
        .as_ref()
        .expect("the player cruiser authors `[helm_console]`");
    vec![
        (
            "engines",
            policy(
                helm.engines_ai
                    .as_ref()
                    .expect("composed from movement_broadside_orbit.toml"),
            ),
        ),
        (
            "steering",
            policy(
                helm.steering_ai
                    .as_ref()
                    .expect("composed from movement_broadside_orbit.toml"),
            ),
        ),
    ]
}

/// **The posture truth table for the broadside-orbit doctrine (issue #876 AC1).**
///
/// The same claim `the_artillery_doctrine_rests_defensive_until_red_alert` makes
/// about the gun line, and it is here for the same reason: `posture` is a fact
/// NAME, so a misspelling in the fragment or in
/// `helm_ai::seed_helm_actuator_facts` parses, validates and reads false for ever
/// — presenting as a cruiser that holds a polite standoff ring and never once
/// fights, with every other test still green.
///
/// It is also the half of AC1 the live probes structurally cannot check. In both
/// `probe_duel` and `probe_aggressor` a hostile sits inside the captain's
/// `alert_on_hostile_within` from the opening seconds, so a hull with the gate
/// mutated out reaches its ring on very nearly the same tick as one without —
/// which is exactly why the gate needs a truth table and not only a live run.
///
/// Nothing in a ring is a commitment the hull is owed (unlike the attack pass's
/// escape leg, which flies a frozen heading), so the posture drop is asserted as
/// the HIGHEST-priority exit from every aggressive leg.
///
/// The threshold comes out of the authored `param` rather than a literal, so a
/// designer who adds an intermediate rung to the posture ladder and retunes
/// `press_posture` retunes this test with it (AGENTS.md rule #11).
#[test]
fn the_broadside_orbit_doctrine_rests_defensive_until_red_alert() {
    for (axis, p) in broadside_orbit_policies() {
        let press = param(&p, "press_posture");
        assert!(
            press > 0.0,
            "{axis}: `press_posture` must be a live threshold, not zero — at zero \
             the defensive rung (0.0) would satisfy `>=` and the gate would be open \
             for ever."
        );
        let machine = p.machine().expect("a state machine");
        assert_eq!(
            machine.initial, "shadow",
            "{axis}: the doctrine RESTS defensive. A machine that booted into an \
             aggressive leg would press once before the first posture reading \
             arrived."
        );

        let pressed = facts(&[("posture", press)]);
        let clear = facts(&[("posture", press - 1.0)]);
        let unseeded = facts(&[]);
        let memory = machine_memory(&p, 0.0);

        assert_eq!(
            p.resolve_transition("shadow", &pressed, &memory, &[])
                .map(|t| t.to.as_str()),
            Some("acquire"),
            "{axis}/shadow: red alert ⇒ the class doctrine is licensed and the hull \
             leaves the standoff ring."
        );
        assert_eq!(
            p.resolve_transition("shadow", &clear, &memory, &[]),
            None,
            "{axis}/shadow: alert DOWN ⇒ maintain range. This is the half that makes \
             the assertion above mean something — without it `shadow` could be a \
             state the hull leaves unconditionally."
        );
        assert_eq!(
            p.resolve_transition("shadow", &unseeded, &memory, &[]),
            None,
            "{axis}/shadow: with NO posture reading at all the comparison reads \
             false and the hull stays defensive. The gate fails CLOSED, so a typo \
             in the fact name cannot make a hull aggressive."
        );

        for state in &machine.states {
            if state.id == "shadow" {
                continue;
            }
            assert_eq!(
                p.resolve_transition(&state.id, &clear, &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("shadow"),
                "{axis}/{}: the alert going down must outrank everything else in \
                 this leg. Nothing in a ring is a commitment the hull is owed — \
                 there is no frozen heading to protect — so a captain standing the \
                 alert down breaks it off at the next tick.",
                state.id
            );
            assert_ne!(
                p.resolve_transition(&state.id, &pressed, &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("shadow"),
                "{axis}/{}: at red alert the break-off guard must NOT fire, or the \
                 doctrine could never join its ring at all.",
                state.id
            );
        }

        let shadow = machine.state("shadow").expect("the defensive leg");
        assert_eq!(
            shadow.transitions.len(),
            1,
            "{axis}/shadow: the standoff leg must have exactly one exit, and it \
             must be the posture gate. A second exit is a way to start a fight \
             without the captain."
        );
    }
}

/// **The broadside-orbit doctrine's legs are the ones the HOST reads (issue #876
/// AC1).**
///
/// The host never learns a state's NAME: it reads which leg is being flown off the
/// Steering axis's yaw verb, and gates each leg on its own complete authored
/// scalar set. So a fragment whose states are all present but whose verbs drifted
/// would validate, spawn, and fly ordinary doctrine travel for ever — which is
/// precisely the state this hull was left in when AC1 was withdrawn, and it looked
/// identical from outside.
///
/// `hold_torpedo_bearing` in particular is not interchangeable with
/// `pivot_to_reengage`, whose geometry is the same: that verb is a leg of the
/// shield-RECOVERY doctrine and the host pairs it with `reengage_speed`, so a
/// drift onto it would fly the bow hold at the wrong throttle and publish the
/// wrong leg on the pass surface.
#[test]
fn the_broadside_orbit_doctrine_rings_when_pressed_and_points_for_a_salvo() {
    let (_, steering) = broadside_orbit_policies()
        .into_iter()
        .find(|(axis, _)| *axis == "steering")
        .expect("the doctrine authors a Steering machine");
    let memory = machine_memory(&steering, 0.0);
    let yaw = |state: &str, f: &AiFacts| {
        steering
            .resolve_channel_in_state(state, "yaw", f, &memory, &[])
            .cloned()
    };
    let with_target = facts(&[("target_valid", 1.0)]);
    let no_target = facts(&[("target_valid", 0.0)]);

    assert_eq!(
        yaw("shadow", &with_target),
        Some(AiPolicyVerb::HoldRecoveryOrbit),
        "clear, with something to stand off FROM ⇒ hold the wide ring the hull's \
         `safe_range_margin` puts beyond the target's own reach."
    );
    assert_eq!(
        yaw("shadow", &no_target),
        Some(AiPolicyVerb::ActuateDesiredFacing),
        "clear, with NOTHING to stand off from ⇒ ordinary doctrine travel. A ring \
         needs a centre, and without this second rule the channel resolves to a \
         hold and a targetless hull coasts on its last steering input for ever."
    );
    assert_eq!(
        yaw("acquire", &with_target),
        Some(AiPolicyVerb::ActuateDesiredFacing),
        "acquire: the bow goes on the target on the way in, and ordinary doctrine \
         travel carries the hull to `engage_range`."
    );
    assert_eq!(
        yaw("orbit", &with_target),
        Some(AiPolicyVerb::HoldCombatOrbit),
        "THE manoeuvre: a ring at the hull's OWN fighting radius with the \
         broadsides bearing. This is the verb the host gates the whole \
         combat-orbit arm on."
    );
    assert_eq!(
        yaw("torpedo_run", &with_target),
        Some(AiPolicyVerb::HoldTorpedoBearing),
        "THE torpedo run: a live bow-on tracking solution, re-solved every tick. \
         Not `pivot_to_reengage`, whose geometry is identical but which the host \
         pairs with the shield-recovery doctrine's `reengage_speed`."
    );
}

/// One authored leg's declared answer to a channel-3 arc-bearing request
/// (issue #918).
fn leg_yields_to_arc_requests(p: &AiPolicy, leg: &str) -> bool {
    p.machine()
        .expect("the doctrine authors a state machine")
        .state(leg)
        .unwrap_or_else(|| panic!("the doctrine declares the `{leg}` leg"))
        .yields_to_arc_requests
}

/// The `steering` copy of a class movement doctrine.
fn steering_of(policies: Vec<(&'static str, AiPolicy)>) -> AiPolicy {
    policies
        .into_iter()
        .find(|(axis, _)| *axis == "steering")
        .expect("the doctrine authors a Steering machine")
        .1
}

/// **The two COMMITTED legs in the fragment library decline channel-3
/// arc-bearing requests; every travelling leg beside them still yields (issue
/// #918).**
///
/// The precedence rule is authored data, not a Rust branch on the verb, and this
/// is where that claim is checked against the files the fleet actually flies. A
/// host that hardcoded "a combat orbit outranks Channel 3" would pass every
/// behavioural test in the repo and would silently decide for the next doctrine
/// somebody writes; the declaration is what keeps the decision the designer's.
///
/// The YIELDING half is the load-bearing one and is asserted leg by leg rather
/// than in the aggregate. A fragment that declined on every leg would look
/// correct in a duel and would quietly retire #673-#684: a cruiser that has not
/// reached its ring yet, or a destroyer still lining up, must still turn to bring
/// a family that cannot bear onto its target, exactly as a hull with no doctrine
/// at all does.
#[test]
fn only_the_committed_legs_decline_a_channel_three_arc_bearing_request() {
    let ring = steering_of(broadside_orbit_policies());
    assert!(
        !leg_yields_to_arc_requests(&ring, "orbit"),
        "the broadside ring holds the target on the beam by construction, which no \
         fixed fore tube can ever satisfy: a ring that yielded would be overwritten \
         bow-on on every tick it was flown"
    );
    // `shadow` flies the identical `plan_recovery_orbit` tangent solver as
    // `orbit`, so it is just as vulnerable to the bow-on overwrite and declines
    // for the same reason (issue #918 followed up).
    assert!(
        !leg_yields_to_arc_requests(&ring, "shadow"),
        "the standoff ring shares `orbit`'s tangent solver and would sawtooth the \
         same way if left to yield"
    );
    for travelling in ["acquire", "torpedo_run"] {
        assert!(
            leg_yields_to_arc_requests(&ring, travelling),
            "`{travelling}` is not a committed heading — it must leave the default \
             standing and turn to bring a family to bear (#673-#684)"
        );
    }

    let pass = steering_of(attack_pass_policies());
    assert!(
        !leg_yields_to_arc_requests(&pass, "escape"),
        "the escape leg's whole point is the frozen heading, and #875 wrote that \
         nothing about the target may cut its dwell short. A request that turns the \
         hull back onto the ship it just passed is exactly that, by another route"
    );
    // `shadow` and `recover` both fly rings on the same `plan_recovery_orbit`
    // tangent solver `escape`'s sibling doctrines fight on, so both decline too
    // (issue #918 followed up).
    for ring_leg in ["shadow", "recover"] {
        assert!(
            !leg_yields_to_arc_requests(&pass, ring_leg),
            "`{ring_leg}` holds a ring via `hold_recovery_orbit`, which shares the \
             fighting doctrines' tangent solver and would sawtooth the same way if \
             left to yield"
        );
    }
    for travelling in ["acquire", "inbound", "reenter"] {
        assert!(
            leg_yields_to_arc_requests(&pass, travelling),
            "`{travelling}` tracks or holds against the target anyway — it has no \
             separate heading to defend and must keep yielding"
        );
    }

    // The declaration belongs to the axis that STEERS. The other copies of the
    // same machine run the same legs and answer no facing request, and content
    // validation rejects the declaration on them outright — so a fragment that
    // mirrored it across the axes would fail to LOAD rather than diverge quietly.
    for (axis, policy) in attack_pass_policies()
        .into_iter()
        .chain(broadside_orbit_policies())
        .filter(|(axis, _)| *axis != "steering")
    {
        for leg in &policy.machine().expect("a doctrine machine").states {
            assert!(
                leg.yields_to_arc_requests,
                "{axis}: leg `{}` declares an arc-request disposition on an axis that \
                 does not steer",
                leg.id
            );
        }
    }
}

/// **Every shipped hull whose RESOLVED steering machine flies a ring or a
/// frozen heading declines a channel-3 arc-bearing request on that leg,
/// full stop (issue #918 followed up).**
///
/// [`only_the_committed_legs_decline_a_channel_three_arc_bearing_request`]
/// checks two specific fragments by NAME — `orbit` on the broadside doctrine,
/// `escape`/`shadow`/`recover` on the attack-pass one. This pin checks the
/// same property the other way round and structurally, so it does not need to
/// know a fragment's state names or even that the fragment exists: for every
/// AI-bearing hull (`ai_hulls`, i.e. every hull that authors `[behaviour]`),
/// walk every axis of its COMPOSED `[helm_console]` — the same six `*_ai`
/// blocks `harrow_warhawk_authors_the_artillery_machine_on_both_travel_axes`
/// (`src/entities/config.rs`) iterates by hand for one hull — and for every
/// state any of whose rules emits `HoldCombatOrbit`, `HoldRecoveryOrbit` or
/// `HoldCommittedHeading`, assert the state itself declares
/// `yields_to_arc_requests = false`.
///
/// Walking the composed config rather than hand-listing "`orbit` on
/// broadside, `escape`/`shadow`/`recover` on attack-pass, `shadow` on
/// artillery, `recover` on the Harrow destroyer's own hand-authored
/// doctrine, ..." is the whole point (review finding 6): a THIRD movement
/// fragment, or a bespoke hull that authors one of these three verbs on a
/// state and forgets the decline, fails HERE — on the hull it was authored
/// on — rather than only in a fragment-specific pin that has never heard of
/// it. `leg.rules` rather than the state's `id` is what is inspected,
/// because the host reads which leg is being flown off the VERB and never
/// off the name (every doctrine file in the fleet says so), so a pin keyed
/// on ids would silently stop checking the day a state was renamed.
#[test]
fn every_ring_or_frozen_heading_leg_declines_arc_bearing_requests() {
    let committed_verbs = |leg: &AiPolicyState| {
        leg.rules.iter().any(|r| {
            matches!(
                &r.verb,
                AiPolicyVerb::HoldCombatOrbit
                    | AiPolicyVerb::HoldRecoveryOrbit
                    | AiPolicyVerb::HoldCommittedHeading
            )
        })
    };
    for (stem, cfg) in ai_hulls() {
        let Some(hc) = cfg.helm_console.as_ref() else {
            continue;
        };
        for (axis, ai) in [
            ("engines_ai", hc.engines_ai.as_ref()),
            ("steering_ai", hc.steering_ai.as_ref()),
            ("lateral_ai", hc.lateral_ai.as_ref()),
            ("vertical_ai", hc.vertical_ai.as_ref()),
            ("impulse_ai", hc.impulse_ai.as_ref()),
            ("boost_ai", hc.boost_ai.as_ref()),
        ] {
            let Some(ai) = ai else { continue };
            let Some(machine) = policy(ai).machine().cloned() else {
                continue;
            };
            for leg in &machine.states {
                if committed_verbs(leg) {
                    assert!(
                        !leg.yields_to_arc_requests,
                        "{stem}: {axis} leg `{}` flies a ring or a frozen heading \
                         (`hold_combat_orbit` / `hold_recovery_orbit` / \
                         `hold_committed_heading`) and must decline channel-3 \
                         arc-bearing requests (issue #918) — left to yield it would \
                         sawtooth in and out of its ring, or be dragged off its \
                         committed heading, by a bow-on solution written over the \
                         planner's own",
                        leg.id
                    );
                }
            }
        }
    }
}

/// **The torpedo run opens on the hull's own readiness and is bounded on it too
/// (issue #876 AC1).**
///
/// The leg exists to own the channel-3 `ArcBearingRequest` where a doctrine can:
/// while a loaded tube cannot bear, a request is raised, and before issue #918
/// `apply_arc_bearing_request` overwrote the doctrine's steering with a bow-on
/// solution after the planner had already solved the ring tangent. A hull that
/// points its own bow when it has a salvo satisfies that request by flying its
/// own doctrine instead — still the better answer on the ticks it covers, and it
/// covers ticks the ring's #918 declaration does not: a hull with a salvo worth
/// spending should point it, not merely refuse to be turned.
///
/// Every guard below fails silently if it drifts:
///
/// * a run that cannot be ENTERED hands the ring back to Channel 3, which is the
///   withdrawn state this issue was reopened from;
/// * a run entered while the hull is still CLOSING cuts thrust at whatever range
///   the tubes happened to finish loading at — measured parking this hull just
///   outside its own beam reach for the rest of the engagement;
/// * a run that cannot be LEFT is a hull that stops orbiting for ever, and it is
///   reachable rather than theoretical: `auto_fire_torpedo` refuses every launch
///   while the striking arc is up, so `tubes_full` can stay true indefinitely
///   against a target this hull cannot strip.
///
/// Both armament exits carry `torpedoes_in_flight` for the same reason: firing
/// empties the tubes immediately, so without it the salvo-spent guard would
/// release the hull on the very tick it launched.
#[test]
fn the_torpedo_run_opens_on_a_loaded_salvo_and_closes_on_the_hulls_own_armament() {
    // The two copies must AGREE about the threshold before anything else is
    // asserted about it. `torpedo_run_range` is referenced by a guard on BOTH
    // axes, so a hull that retunes it on one and leaves the other on the class
    // default breaks its ring at two different ranges — the axes reach different
    // legs on the same tick, which is the silent failure two independent copies of
    // one machine exist to make impossible.
    let ranges: Vec<f64> = broadside_orbit_policies()
        .iter()
        .map(|(_, p)| param(p, "torpedo_run_range"))
        .collect();
    assert_eq!(
        ranges[0], ranges[1],
        "the Engines and Steering copies disagree about the range a loaded salvo is \
         worth breaking the ring for ({} vs {}). Both guards read this name, so a \
         hull retuning it owes it to both axes.",
        ranges[0], ranges[1]
    );

    for (axis, p) in broadside_orbit_policies() {
        let press = param(&p, "press_posture");
        let run_range = param(&p, "torpedo_run_range");
        assert!(
            run_range > 0.0,
            "{axis}: `torpedo_run_range` must be a live threshold. At zero the leg \
             can never open and the ring is handed straight back to Channel 3."
        );
        // The striking-arc reading the composing hull DEMANDS (issue #878). The
        // class default is 0.0 — "any reading will do", which is what the player
        // cruiser measured best — and a hull that wants the Harrow's stricter
        // entry authors 1.0. Seeding the fact at exactly that threshold keeps
        // every assertion below a statement about the armament guards whichever
        // reading the hull composing this fragment has chosen, and the parameter
        // itself is pinned as a switch by
        // `the_harrow_cruiser_breaks_its_ring_only_for_a_struck_down_arc`.
        let gap = param(&p, "torpedo_run_shield_gap");
        let memory = machine_memory(&p, 0.0);
        let at = |range: f64, full: f64, fillable: f64, in_flight: f64| {
            facts(&[
                ("posture", press),
                ("target_valid", 1.0),
                ("range_to_target", range),
                ("tubes_full", full),
                ("tubes_fillable", fillable),
                ("torpedoes_in_flight", in_flight),
                ("target_facing_shield_down", gap),
            ])
        };

        assert_eq!(
            p.resolve_transition("orbit", &at(run_range, 1.0, 1.0, 0.0), &memory, &[])
                .map(|t| t.to.as_str()),
            Some("torpedo_run"),
            "{axis}/orbit: a loaded, reachable salvo inside `torpedo_run_range` is \
             what the ring breaks for."
        );
        for (why, f) in [
            ("no salvo loaded", at(run_range, 0.0, 1.0, 0.0)),
            ("the battery shot out", at(run_range, 1.0, 0.0, 0.0)),
            ("the hull still closing", at(run_range + 1.0, 1.0, 1.0, 0.0)),
        ] {
            assert_ne!(
                p.resolve_transition("orbit", &f, &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("torpedo_run"),
                "{axis}/orbit: the ring must NOT break with {why}. It spends the \
                 broadside geometry for a shot that is not there — and outside the \
                 run range it also cuts thrust at whatever range the hull reached."
            );
        }

        assert_eq!(
            p.resolve_transition("torpedo_run", &at(run_range, 0.0, 1.0, 0.0), &memory, &[])
                .map(|t| t.to.as_str()),
            Some("orbit"),
            "{axis}/torpedo_run: the salvo is spent ⇒ back to the ring while the \
             tubes reload. THE bound, and it is drawn on this hull's own armament \
             rather than on the target doing anything."
        );
        assert_eq!(
            p.resolve_transition("torpedo_run", &at(run_range, 1.0, 0.0, 0.0), &memory, &[])
                .map(|t| t.to.as_str()),
            Some("orbit"),
            "{axis}/torpedo_run: the battery is gone ⇒ back to the ring. This is \
             what a rounds-in-the-tube reading structurally cannot see — a tube \
             destroyed mid-run keeps its rounds, so `tubes_full` stays true."
        );
        for full in [0.0, 1.0] {
            assert_ne!(
                p.resolve_transition("torpedo_run", &at(run_range, full, 0.0, 1.0), &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("orbit"),
                "{axis}/torpedo_run: a hull does not turn away from rounds it has \
                 committed, airborne or still owed to a burst."
            );
        }
        assert_eq!(
            p.resolve_transition(
                "torpedo_run",
                &facts(&[("posture", press), ("target_valid", 0.0)]),
                &memory,
                &[]
            )
            .map(|t| t.to.as_str()),
            Some("acquire"),
            "{axis}/torpedo_run: the target is gone ⇒ `acquire`, not `orbit`. There \
             is no bow to hold on nothing, and no ring to hold either."
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The composed HARROW hulls (issue #878)
// ─────────────────────────────────────────────────────────────────────────────

/// Every Harrow hull that composes a class MOVEMENT fragment, paired with the
/// travelling leg its doctrine's defensive `shadow` state opens into.
const COMPOSED_HARROW_HULLS: &[(&str, &str)] = &[
    ("ship_harrow_destroyer", "acquire"),
    ("ship_harrow_cruiser", "acquire"),
    ("ship_harrow_warhawk", "acquire"),
];

/// **The whole of issue #878's authoring claim, as a truth table: a Harrow
/// unlocks its class doctrine by POSTURE PARAMETER and by nothing else.**
///
/// The shared movement fragments rest in `shadow` — a standoff ring outside the
/// target's own guns — and open the aggressive half on `fact(posture) >=
/// param(press_posture)`. The Alliance hulls leave `press_posture` at the class
/// default of 1 and wait for their captain to call red alert. A Harrow has no
/// bridge crew to call one and its captain policy (#912) stands the alert down
/// out of combat, so it authors the LOWEST rung instead and is permanently
/// pressed.
///
/// That single number is the entire difference between the player fleet's
/// doctrine and the Harrow's, so it is asserted in both directions on every axis
/// of every composed hull:
///
/// * the gate OPENS at the seeded defensive reading (`posture = 0`), which is
///   what makes the hull fight at all — the failure it catches is a Harrow that
///   quietly acquires the class default and then shadows politely for ever,
///   outside its own weapons range, with every structural test still green;
/// * every break-off guard is UNREACHABLE, which is what makes the composed
///   machine reduce to exactly the inline one it replaced (#883/#790/#792) —
///   there is no posture reading at which a Harrow gives up its manoeuvre,
///   because there is no rung below the one it presses at.
///
/// Read off the shipped files, so a retune in TOML retunes this pin with it
/// (AGENTS.md rule #11).
#[test]
fn the_harrow_hulls_unlock_their_class_doctrine_by_posture_alone() {
    for (stem, opens_into) in COMPOSED_HARROW_HULLS {
        let cfg = entity(stem);
        let helm = cfg
            .helm_console
            .as_ref()
            .unwrap_or_else(|| panic!("{stem} authors [helm_console]"));
        let axes = [
            ("engines", helm.engines_ai.as_ref()),
            ("steering", helm.steering_ai.as_ref()),
            ("boost", helm.boost_ai.as_ref()),
        ];
        let mut machines = 0usize;
        for (axis, ai) in axes {
            let Some(ai) = ai else { continue };
            let p = policy(ai);
            let Some(machine) = p.machine().cloned() else {
                continue;
            };
            machines += 1;
            assert_eq!(
                machine.initial, "shadow",
                "{stem}/{axis}: the class doctrine RESTS defensive, and a hull that \
                 booted into an aggressive leg would press once before the first \
                 posture reading arrived"
            );
            let press = param(&p, "press_posture");
            assert_eq!(
                press, 0.0,
                "{stem}/{axis}: a Harrow is ALWAYS pressed and says so by authoring \
                 the lowest rung of the posture ladder. Anything above it and this \
                 hull holds a standoff ring outside its own guns until a captain it \
                 does not have calls red alert."
            );

            // The seeded DEFENSIVE reading — the only one an unmanned Harrow ever
            // produces, since its captain stands the alert down out of combat.
            let defensive = facts(&[("posture", 0.0)]);
            let memory = machine_memory(&p, 0.0);
            assert_eq!(
                p.resolve_transition("shadow", &defensive, &memory, &[])
                    .map(|t| t.to.as_str()),
                Some(*opens_into),
                "{stem}/{axis}: the gate must be OPEN at the resting posture — this \
                 is what makes the hull leave the ring on its first evaluation and \
                 start the fight it exists to start"
            );

            // …and NO leg anywhere can be broken off by posture, because no
            // reading is below the rung this hull presses at. Resolved through the
            // real transition resolver from every leg, with the state clock run
            // well past any authored dwell so a break-off deferred to the end of a
            // commitment is eligible too, and with every other fact ABSENT so the
            // only guards that can fire are the posture-only ones. A future
            // fragment adding a break-off to a new leg is covered without this pin
            // being edited.
            let long_dwell = machine_memory(&p, 1.0e6);
            for leg in &machine.states {
                if leg.id == "shadow" {
                    continue;
                }
                assert_ne!(
                    p.resolve_transition(&leg.id, &defensive, &long_dwell, &[])
                        .map(|t| t.to.as_str()),
                    Some("shadow"),
                    "{stem}/{axis}/{}: the class doctrine's posture break-off must be \
                     unreachable on an always-pressed hull. If it can fire, the \
                     composed doctrine no longer reduces to the inline manoeuvre it \
                     replaced — the hull would abandon its attack at the resting \
                     posture, which is every tick of an unmanned Harrow's life.",
                    leg.id
                );
            }
        }
        assert!(
            machines >= 2,
            "{stem}: a composed movement doctrine is a machine on BOTH travel axes \
             or it is not a class doctrine — found {machines}"
        );
    }
}

/// **The Harrow cruiser's ring still breaks only for a struck-down arc, and the
/// player cruiser's still does not (issue #878).**
///
/// `movement_broadside_orbit.toml` gained `torpedo_run_shield_gap` so one file
/// could carry both readings, and the parameter is a SWITCH rather than a tuning:
/// at `0.0` the bow hold opens on range and readiness alone (what the player
/// cruiser measured best) and the arc-recovered exit is unreachable; at `1.0` the
/// entry demands the striking arc be down and the exit fires when it comes back.
///
/// Asserted through the real transition resolver on both shipped hulls, because
/// the failure it catches is silent in either direction: a Harrow that acquired
/// the permissive reading would cut thrust and hold its bow on a target it cannot
/// hurt (its rounds author `damage_shields = 0`), and a player cruiser that
/// acquired the strict one would wait bow-on for a window its own beams open at
/// about one damage per second into a regenerating arc — measured on
/// `probe_aggressor` at 0 opened runs in 901 ticks.
#[test]
fn the_harrow_cruiser_breaks_its_ring_only_for_a_struck_down_arc() {
    for (stem, gap, arc_up_opens_the_run) in [
        ("ship_harrow_cruiser", 1.0, false),
        ("alliance_cruiser", 0.0, true),
    ] {
        let cfg = entity(stem);
        let helm = cfg.helm_console.as_ref().expect("[helm_console]");
        for (axis, ai) in [
            ("engines", helm.engines_ai.as_ref()),
            ("steering", helm.steering_ai.as_ref()),
        ] {
            let p = policy(ai.expect("a composed travel axis"));
            assert_eq!(
                param(&p, "torpedo_run_shield_gap"),
                gap,
                "{stem}/{axis}: the two readings differ by this parameter and \
                 nothing else"
            );
            let memory = machine_memory(&p, 0.0);
            let ready = |arc_down: f64| {
                facts(&[
                    ("posture", param(&p, "press_posture")),
                    ("target_valid", 1.0),
                    ("range_to_target", param(&p, "torpedo_run_range")),
                    ("tubes_full", 1.0),
                    ("tubes_fillable", 1.0),
                    ("torpedoes_in_flight", 0.0),
                    ("target_facing_shield_down", arc_down),
                ])
            };
            assert_eq!(
                p.resolve_transition("orbit", &ready(1.0), &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("torpedo_run"),
                "{stem}/{axis}: a struck-down arc with a loaded salvo opens the bow \
                 hold on BOTH readings — that is the case the leg exists for"
            );
            assert_eq!(
                p.resolve_transition("orbit", &ready(0.0), &memory, &[])
                    .map(|t| t.to.as_str())
                    == Some("torpedo_run"),
                arc_up_opens_the_run,
                "{stem}/{axis}: with the arc UP the permissive reading must still \
                 open the run and the strict one must refuse it. This is the whole \
                 of the parameter."
            );
            // …and the exit that owes the strict entry its bound: the arc coming
            // back releases the hull, but only where an arc was demanded.
            assert_eq!(
                p.resolve_transition("torpedo_run", &ready(0.0), &memory, &[])
                    .map(|t| t.to.as_str())
                    == Some("orbit"),
                !arc_up_opens_the_run,
                "{stem}/{axis}: a hull that breaks its ring FOR a shield gap must \
                 resume the ring when the gap closes; one that never asked for a gap \
                 has nothing to be released by"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The composed ARTILLERY doctrine (issue #876)
// ─────────────────────────────────────────────────────────────────────────────

/// The two copies of the composed artillery machine, keyed by the axis they fly.
/// Read off the SHIPPED hull, so a retune in TOML retunes these pins with it.
///
/// Two, not three: a gun line has no leg that engages the boost drive, so
/// `fragments/ai/movement_artillery.toml` leaves `[helm_console.boost_ai]` alone
/// and this hull keeps the fleet baseline's `idle = true`. That absence is
/// asserted by `boost_is_the_only_idle_baseline_and_two_hulls_depart_from_it`
/// continuing to name two hulls rather than three.
fn artillery_policies() -> Vec<(&'static str, AiPolicy)> {
    let hull = entity("alliance_battleship");
    let helm = hull
        .helm_console
        .as_ref()
        .expect("the player battleship authors `[helm_console]`");
    vec![
        (
            "engines",
            policy(
                helm.engines_ai
                    .as_ref()
                    .expect("composed from movement_artillery.toml"),
            ),
        ),
        (
            "steering",
            policy(
                helm.steering_ai
                    .as_ref()
                    .expect("composed from movement_artillery.toml"),
            ),
        ),
    ]
}

/// **The posture truth table for the artillery doctrine (issue #876 AC2/AC4).**
///
/// The same claim `posture_guard_truth_table` makes about the attack pass, and
/// the same #779 failure shape it exists to catch: `posture` is a fact NAME, so a
/// misspelling here or in `helm_ai::seed_helm_actuator_facts` parses, validates
/// and reads false for ever — presenting as a battleship that holds a polite
/// standoff ring and never once takes up a firing position, with every other test
/// still green.
///
/// One difference from the attack pass, and it is doctrine rather than oversight:
/// NO leg of a gun line is a commitment. The pass defers its posture drop to the
/// end of the escape dwell because the escape flies a frozen heading; nothing
/// here flies anything the hull is owed, so the drop is the highest-priority exit
/// from every aggressive leg and is asserted as such below.
///
/// The threshold comes out of the authored `param` rather than a literal, so a
/// designer who adds an intermediate rung to the posture ladder and retunes
/// `press_posture` retunes this test with it (AGENTS.md rule #11).
#[test]
fn the_artillery_doctrine_rests_defensive_until_red_alert() {
    for (axis, p) in artillery_policies() {
        let press = param(&p, "press_posture");
        assert!(
            press > 0.0,
            "{axis}: `press_posture` must be a live threshold, not zero — at zero \
             the defensive rung (0.0) would satisfy `>=` and the gate would be open \
             for ever."
        );
        let machine = p.machine().expect("a state machine");
        assert_eq!(
            machine.initial, "shadow",
            "{axis}: the doctrine RESTS defensive. A machine that booted into an \
             aggressive leg would press once before the first posture reading \
             arrived."
        );

        let pressed = facts(&[("posture", press)]);
        let clear = facts(&[("posture", press - 1.0)]);
        let unseeded = facts(&[]);
        let memory = machine_memory(&p, 0.0);

        assert_eq!(
            p.resolve_transition("shadow", &pressed, &memory, &[])
                .map(|t| t.to.as_str()),
            Some("acquire"),
            "{axis}/shadow: red alert ⇒ the class doctrine is licensed and the hull \
             leaves the standoff ring."
        );
        assert_eq!(
            p.resolve_transition("shadow", &clear, &memory, &[]),
            None,
            "{axis}/shadow: alert DOWN ⇒ hold position at range. This is the half \
             that makes the assertion above mean something — without it `shadow` \
             could be a state the hull leaves unconditionally."
        );
        assert_eq!(
            p.resolve_transition("shadow", &unseeded, &memory, &[]),
            None,
            "{axis}/shadow: with NO posture reading at all the comparison reads \
             false and the hull stays defensive. The gate fails CLOSED, so a typo \
             in the fact name cannot make a hull aggressive."
        );

        // Every aggressive leg, and the drop outranks everything in each of them.
        for state in &machine.states {
            if state.id == "shadow" {
                continue;
            }
            assert_eq!(
                p.resolve_transition(&state.id, &clear, &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("shadow"),
                "{axis}/{}: the alert going down must outrank everything else in \
                 this leg. Nothing in a gun line is a commitment the hull is owed, \
                 so a captain standing the alert down breaks it off at the next \
                 tick.",
                state.id
            );
            assert_ne!(
                p.resolve_transition(&state.id, &pressed, &memory, &[])
                    .map(|t| t.to.as_str()),
                Some("shadow"),
                "{axis}/{}: at red alert the break-off guard must NOT fire, or the \
                 doctrine could never take up its position at all.",
                state.id
            );
        }

        // …and the reverse: the defensive leg's ONLY way out is the posture gate.
        let shadow = machine.state("shadow").expect("the defensive leg");
        assert_eq!(
            shadow.transitions.len(),
            1,
            "{axis}/shadow: the standoff leg must have exactly one exit, and it \
             must be the posture gate. A second exit is a way to start a fight \
             without the captain."
        );
    }
}

/// **The artillery doctrine's legs are the ones the HOST reads (issue #876
/// AC2).**
///
/// The host never learns a state's NAME: it reads which leg is being flown off
/// the Steering axis's yaw verb, and gates each leg on its own complete authored
/// scalar set. So a fragment whose states are all present but whose verbs drifted
/// would validate, spawn, and fly ordinary doctrine travel for ever.
///
/// The `shadow` fallback is asserted in both directions on purpose. With a target
/// the leg is a ring; WITHOUT one there is no centre to hold a ring around and no
/// reach to derive its radius from, so the axis has to hand back to ordinary
/// doctrine travel — and if that second rule were missing the channel would
/// resolve to "hold" and a targetless hull would coast on its last steering input
/// for ever.
#[test]
fn the_artillery_doctrine_flies_a_ring_when_clear_and_a_gun_line_when_pressed() {
    let (_, steering) = artillery_policies()
        .into_iter()
        .find(|(axis, _)| *axis == "steering")
        .expect("the doctrine authors a Steering machine");
    let memory = machine_memory(&steering, 0.0);
    let yaw = |state: &str, f: &AiFacts| {
        steering
            .resolve_channel_in_state(state, "yaw", f, &memory, &[])
            .cloned()
    };
    let with_target = facts(&[("target_valid", 1.0)]);
    let no_target = facts(&[("target_valid", 0.0)]);

    assert_eq!(
        yaw("shadow", &with_target),
        Some(AiPolicyVerb::HoldRecoveryOrbit),
        "clear, with something to stand off FROM ⇒ hold the wide ring the hull's \
         `safe_range_margin` puts beyond the target's own reach."
    );
    assert_eq!(
        yaw("shadow", &no_target),
        Some(AiPolicyVerb::ActuateDesiredFacing),
        "clear, with NOTHING to stand off from ⇒ ordinary doctrine travel. A ring \
         needs a centre."
    );
    for leg in ["acquire", "reposition"] {
        assert_eq!(
            yaw(leg, &with_target),
            Some(AiPolicyVerb::ActuateDesiredFacing),
            "{leg}: the bow goes on the TARGET on the way in, not on a lead — \
             nothing is being fired at this range, and a run-in aimed at an \
             intercept arrives off to one side of it."
        );
    }
    assert_eq!(
        yaw("hold", &with_target),
        Some(AiPolicyVerb::HoldArtilleryPosition),
        "THE manoeuvre: translational station on the authored throttle with the \
         bow on a PREDICTED intercept. This is the verb the host gates the whole \
         artillery arm on."
    );
}

/// **Issue #876 AC3, as a relation rather than as a number.** The battleship
/// specialises the shared artillery doctrine by `param` alone, and what it
/// specialises is its own GUN.
///
/// Nothing here restates an authored value (AGENTS.md #11). Every assertion is a
/// relation between two things the content already says, so a designer retuning
/// the envelope or the weapon retunes the test with it — and each relation is one
/// the doctrine would silently half-fly if it broke:
///
/// * the two axes must AGREE, because they are two independent copies of one
///   machine that reach their legs by reading the same facts. A copy left on the
///   fragment's default would take up its position at a different range from the
///   other and the axes would disagree about which leg they are flying.
/// * the inner edge must sit below the outer one, because the gap between them IS
///   the hysteresis; equal, the hull would chatter between closing and holding
///   every time the target drifted across the line.
/// * the outer edge must not reach past the hull's own artillery piece, or the
///   doctrine would hold a gun line from outside the range at which its bolt
///   still exists.
#[test]
fn the_battleship_tunes_the_artillery_envelope_to_its_own_gun() {
    let hull = entity("alliance_battleship");
    let policies = artillery_policies();
    let hold: Vec<f64> = policies
        .iter()
        .map(|(_, p)| param(p, "artillery_hold_range"))
        .collect();
    let max: Vec<f64> = policies
        .iter()
        .map(|(_, p)| param(p, "max_artillery_range"))
        .collect();
    assert_eq!(
        hold[0], hold[1],
        "the Engines and Steering copies disagree about the inner edge of the \
         band, so they will take up the firing position at different ranges"
    );
    assert_eq!(
        max[0], max[1],
        "the Engines and Steering copies disagree about the outer edge of the band"
    );
    assert!(
        hold[0] < max[0],
        "the inner edge ({}) must sit below the outer one ({}) — the gap between \
         them is the hysteresis that stops the hull chattering between closing \
         and holding",
        hold[0],
        max[0]
    );

    let longest_bolt = hull
        .weapons_console
        .as_ref()
        .expect("the battleship carries weapons")
        .blaster_banks
        .iter()
        .map(|b| b.range)
        .fold(0.0_f32, f32::max);
    assert!(longest_bolt > 0.0, "the battleship mounts a blaster bank");
    assert!(
        max[0] <= longest_bolt as f64,
        "the gun line's outer edge ({}) reaches past this hull's longest blaster \
         ({longest_bolt}), so the doctrine would hold a firing position from \
         outside the range at which its own bolt still exists. The envelope \
         belongs to the WEAPON.",
        max[0]
    );
}

/// **The artillery doctrine switches the impulse drive off, and it has to.**
///
/// The sibling of `config::tests::harrow_warhawk_holds_its_impulse_drive_idle`,
/// pointed at the hull that takes the same doctrine through `includes` instead of
/// authoring it inline. Both halves matter and both are asserted:
///
/// * the declaration is an explicit IDLE carrying no rules and no states. The
///   fragment has to author `rule = []` as well as `idle = true`, because `idle`
///   deep-merges as a plain scalar over the fleet baseline's unconditional
///   permit and an idle policy carrying an inherited rule is a contradiction
///   content validation rejects — so this is where a merge that dropped that one
///   line shows up.
/// * the band still lies inside the drive's default cruise window, which is the
///   reason the idle is needed at all: an engaged drive hard-overrides commanded
///   throttle, so the hull would sail through its own gun line. If a future
///   retune moved the band clear, this assertion is what says so rather than
///   leaving the idle looking like superstition.
#[test]
fn the_composed_battleship_holds_its_impulse_drive_idle() {
    let hull = entity("alliance_battleship");
    let helm = hull
        .helm_console
        .as_ref()
        .expect("the battleship authors `[helm_console]`");
    let impulse = helm.impulse_ai.as_ref().expect(
        "the battleship must resolve a `[helm_console.impulse_ai]`: since #885b \
         stage 5d an absent block is a load error, and the fleet baseline's \
         unconditional permit is exactly what the artillery fragment replaces",
    );
    let decoded = policy(impulse);
    assert!(
        decoded.idle,
        "the declaration must be an explicit idle — the impulse channel resolving \
         to nothing, whatever geometry or doctrine the host is handed"
    );
    assert!(
        impulse.rule.is_empty() && impulse.state.is_empty(),
        "an idle declaration carries no rules and no states (content validation \
         rejects the contradiction), so anything here is the fleet baseline's \
         permit surviving the merge"
    );

    let (_, steering) = artillery_policies()
        .into_iter()
        .find(|(axis, _)| *axis == "steering")
        .expect("the doctrine authors a Steering machine");
    let band = param(&steering, "artillery_hold_range");
    assert!(
        (helm.impulse_cancel_distance as f64) < band
            && band < (helm.impulse_engage_distance as f64),
        "the hold range ({band}) sits inside the impulse cruise window (engage {}, \
         cancel {}) — if a retune ever moves it clear, revisit whether the idle \
         above is still earning its place",
        helm.impulse_engage_distance,
        helm.impulse_cancel_distance
    );
}

/// Power: the elevate rules need BOTH their trigger and their battery reserve;
/// the hold rules need the channel to be elevated ALREADY; the baseline rules
/// hold the line whenever a battery reading exists at all.
///
/// The brownout property is structural rather than a separate branch: an elevate
/// guard cannot fire below its reserve, so allocation never rises when the
/// battery cannot sustain it.
///
/// Since issue #1003 each channel carries two floors instead of one, and the
/// hysteresis section at the bottom is where the pair earns its keep: between
/// them the answer depends on `fact(power_<group>)` — where the channel already
/// is — and on nothing else.
#[test]
fn power_guard_truth_table() {
    let p = fleet_baseline_policy("power");
    let thrust_threshold = param(&p, "thrust_threshold");
    let helm_reserve = param(&p, "min_reserve_helm");
    let helm_restore = param(&p, "min_restore_helm");
    let weapons_reserve = param(&p, "min_reserve_weapons");
    let weapons_restore = param(&p, "min_restore_weapons");

    let elevated = |level: u8| Some(AiPolicyVerb::SetPowerGroupAllocation(level));
    let (hi, lo) = (3u8, 2u8);

    // ── helm ────────────────────────────────────────────────────────────────
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_reserve + 30.0)
            ])
        ),
        elevated(hi),
        "red alert, sustained thrust AND battery above the helm reserve ⇒ elevate."
    );
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_reserve + 30.0)
            ])
        ),
        elevated(lo),
        "same thrust and battery, but away from red alert ⇒ baseline. \
         `plan_helm_travel` commands near-max thrust for ordinary transit, not just \
         combat manoeuvring, so without this guard a cruising ship held the elevated \
         allocation for its whole transit and browned out with no combat involved."
    );
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_reserve - 20.0)
            ])
        ),
        elevated(lo),
        "red alert and thrust, battery BELOW the reserve ⇒ the elevate guard reads \
         false and helm holds baseline. This is the brownout guard, and it is the \
         reserve param that enforces it — not a global emergency branch."
    );
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold - 0.5),
                ("battery_pct", helm_reserve + 30.0)
            ])
        ),
        elevated(lo),
        "thrust below the threshold ⇒ baseline, however full the battery, even at \
         red alert."
    );

    // ── weapons ─────────────────────────────────────────────────────────────
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[("red_alert", 1.0), ("battery_pct", weapons_reserve + 70.0)])
        ),
        elevated(hi),
        "red alert AND battery above the weapons restore floor ⇒ elevate."
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

    // ── the hysteresis band, on both channels (issue #1003) ─────────────────
    //
    // Between a channel's shed floor and its restore floor the two rules
    // disagree, and `fact(power_<group>)` is the tie-break. Every row below
    // holds the battery at the SAME charge and changes only where the channel
    // already is — which is exactly the property a single-threshold ladder
    // cannot express, and the reason it flips at tick rate.
    assert!(
        helm_reserve < helm_restore && weapons_reserve < weapons_restore,
        "each channel must restore above where it sheds ({helm_reserve}/{helm_restore}, \
         {weapons_reserve}/{weapons_restore}), or there is no band to test"
    );
    let helm_band = (helm_reserve + helm_restore) / 2.0;
    let weapons_band = (weapons_reserve + weapons_restore) / 2.0;

    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_band),
                ("power_helm", 3.0)
            ])
        ),
        elevated(hi),
        "in the band, helm ALREADY at 3 ⇒ the hold rule keeps it there. Shedding \
         here would give the point back ten percent above the authored floor."
    );
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_band),
                ("power_helm", 2.0)
            ])
        ),
        elevated(lo),
        "same charge, helm already SHED ⇒ baseline. It may not come back until \
         `min_restore_helm`, which is what stops the shed and the re-elevate \
         landing on one number and flipping every tick."
    );
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[
                ("red_alert", 1.0),
                ("battery_pct", weapons_band),
                ("power_weapons", 3.0)
            ])
        ),
        elevated(hi),
        "in the band, weapons ALREADY at 3 ⇒ hold."
    );
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[
                ("red_alert", 1.0),
                ("battery_pct", weapons_band),
                ("power_weapons", 2.0)
            ])
        ),
        elevated(lo),
        "same charge, weapons already SHED ⇒ baseline. This is the rung the \
         battery actually oscillates on, so this is the row that matters."
    );
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[
                ("red_alert", 0.0),
                ("battery_pct", weapons_band),
                ("power_weapons", 3.0)
            ])
        ),
        elevated(lo),
        "the hold rule carries the SAME trigger clauses as the elevate: holding is \
         not a licence to keep the point once the alert is down."
    );

    // ── the exact boundary, on all four authored floors (issue #1003) ───────
    //
    // Every row above sits well clear of a floor; these sit exactly ON one, in
    // both directions. Evaluated against the DECODED policy via the facts bag
    // directly, like the rest of this test, rather than through the reactor's
    // own `battery_pct` computation — that widens an f32 quotient before
    // comparing, which is not the same number as a `param()` read as f64 when
    // the assertion depends on landing exactly on the line.
    let eps = 1e-6;

    // helm SHED floor: the hold rule's guard is `>=`, so exactly on the floor
    // still holds; one epsilon under it, the channel sheds.
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_reserve),
                ("power_helm", 3.0)
            ])
        ),
        elevated(hi),
        "helm already elevated, battery exactly ON the {helm_reserve} shed floor \
         ⇒ the `>=` hold guard still reads true."
    );
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_reserve - eps),
                ("power_helm", 3.0)
            ])
        ),
        elevated(lo),
        "helm already elevated, battery one epsilon BELOW the {helm_reserve} \
         shed floor ⇒ sheds."
    );

    // helm RESTORE floor: the elevate rule's guard is also `>=`.
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_restore)
            ])
        ),
        elevated(hi),
        "helm not yet elevated, battery exactly ON the {helm_restore} restore \
         floor ⇒ the `>=` elevate guard reads true and it climbs."
    );
    assert_eq!(
        resolve(
            &p,
            "helm",
            &facts(&[
                ("red_alert", 1.0),
                ("thrust", thrust_threshold + 0.2),
                ("battery_pct", helm_restore - eps)
            ])
        ),
        elevated(lo),
        "helm not yet elevated, battery one epsilon BELOW the {helm_restore} \
         restore floor ⇒ stays at baseline."
    );

    // weapons SHED floor.
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[
                ("red_alert", 1.0),
                ("battery_pct", weapons_reserve),
                ("power_weapons", 3.0)
            ])
        ),
        elevated(hi),
        "weapons already elevated, battery exactly ON the {weapons_reserve} \
         shed floor ⇒ holds."
    );
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[
                ("red_alert", 1.0),
                ("battery_pct", weapons_reserve - eps),
                ("power_weapons", 3.0)
            ])
        ),
        elevated(lo),
        "weapons already elevated, battery one epsilon BELOW the \
         {weapons_reserve} shed floor ⇒ sheds."
    );

    // weapons RESTORE floor.
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[("red_alert", 1.0), ("battery_pct", weapons_restore)])
        ),
        elevated(hi),
        "weapons not yet elevated, battery exactly ON the {weapons_restore} \
         restore floor ⇒ climbs."
    );
    assert_eq!(
        resolve(
            &p,
            "weapons",
            &facts(&[("red_alert", 1.0), ("battery_pct", weapons_restore - eps)])
        ),
        elevated(lo),
        "weapons not yet elevated, battery one epsilon BELOW the \
         {weapons_restore} restore floor ⇒ stays at baseline."
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

        // `target_facing_shields` rides along for the torpedo row and is inert
        // for the two beam ones: since issue #956 the tube's launch guard also
        // requires the striking arc to be down, so a snapshot carrying only the
        // alert would hold for a reason this row is not about. Its own truth
        // table is `torpedo_launch_shield_gate_truth_table` below.
        assert_eq!(
            resolve(
                &p,
                channel,
                &facts(&[("red_alert", 1.0), ("target_facing_shields", 0.0)])
            )
            .as_ref(),
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

/// **The tactical restraint lever, against the SHIPPED fire gates (issue
/// #1041).**
///
/// The claim the whole slice rests on: a weapons hold suppresses fire on every
/// armed hull in the fleet **without one character of authored doctrine
/// changing**. It is proved here rather than in the weapons module because the
/// thing being proved is a property of the CONTENT — every shipped bank's
/// authored `min_alert_to_fire` — and content is what this file pins.
///
/// Read the rows as a ladder. `min_alert_to_fire` is a floor on how hot the ship
/// must be before a bank opens up; the hold seeds a rung below stood-down, so it
/// sits under every floor the fleet authors, the always-armed `0` included. That
/// last row is the one that matters: seeding a plain `0.0` for a hold would have
/// left the Harrow gun line firing through the captain's order, because
/// `0 >= 0`.
#[test]
fn a_weapons_hold_closes_every_shipped_fire_gate() {
    use crate::console::weapons::{WeaponsAlertPosture, WEAPONS_HOLD_ALERT_FACT};

    let held = WeaponsAlertPosture {
        red_alert: true,
        weapons_hold: true,
        stance_high_alert: None,
    };
    assert_eq!(
        held.alert_fact_value(),
        WEAPONS_HOLD_ALERT_FACT,
        "a hold outranks the alert in the seeded value — the whole point is that \
         a ship can be AT stations with its guns cold"
    );
    // The released half of the byte-identical claim, at the source: with no
    // hold the seeded value is exactly the 1.0/0.0 every host inlined before
    // this issue, so a run in which nobody holds fire cannot have moved.
    assert_eq!(WeaponsAlertPosture::alert(true).alert_fact_value(), 1.0);
    assert_eq!(WeaponsAlertPosture::alert(false).alert_fact_value(), 0.0);

    let held_snapshot = |extra: &[(&str, f64)]| {
        let mut pairs = vec![("red_alert", held.alert_fact_value())];
        pairs.extend_from_slice(extra);
        facts(&pairs)
    };

    // ── The fleet baseline: a hull WITH a captain (threshold 1) ─────────────
    for (kind, channel) in [
        ("phaser_bank", "phaser_fire"),
        ("blaster_bank", "blaster_fire"),
        ("torpedo_tube", "torpedo_launch"),
    ] {
        let p = fleet_baseline_policy(kind);
        assert!(
            param(&p, "min_alert_to_fire") >= 0.0,
            "{kind}: the hold sits below every AUTHORABLE floor, so a hull that \
             authored a negative threshold would shoot through the captain's \
             order. No shipped hull does, and this is the assertion that keeps \
             it that way."
        );
        assert_eq!(
            resolve(
                &p,
                channel,
                &held_snapshot(&[
                    ("target_valid", 1.0),
                    ("in_range", 1.0),
                    ("in_arc", 1.0),
                    ("loaded", 1.0),
                    ("tubes_full", 1.0),
                    ("target_facing_shields", 0.0),
                ])
            ),
            None,
            "{kind}/{channel}: EVERY readiness reading favourable, the alert \
             raised, and the ship still holds — because the captain called a \
             weapons hold. This is the suppression AC, resolved through the \
             shipped predicate with no new vocabulary in it."
        );
    }

    // ── The Harrow gun line: always armed (threshold 0) ─────────────────────
    //
    // The hull with no captain to call an alert is also the hull a scenario is
    // most likely to order to hold fire, and it is the one a naive "seed zero"
    // implementation would have missed entirely.
    let harrow = entity("ship_harrow_patrol");
    let bank = harrow
        .weapons_console
        .as_ref()
        .expect("the Harrow patrol carries phasers")
        .phaser_banks
        .first()
        .expect("…at least one bank");
    let hp = policy(
        bank.ai
            .as_ref()
            .expect("every shipped bank authors a policy"),
    );
    assert_eq!(param(&hp, "min_alert_to_fire"), 0.0);
    assert_eq!(
        resolve(&hp, "phaser_fire", &held_snapshot(&[])),
        None,
        "the always-armed threshold of 0 is exactly what a hold has to beat, and \
         it does — `-1 >= 0` is false. A hold seeded as a plain 0.0 would have \
         left this hull shooting."
    );
    // …and releasing it restores the always-armed behaviour byte for byte.
    for alert in [0.0, 1.0] {
        assert_eq!(
            resolve(&hp, "phaser_fire", &facts(&[("red_alert", alert)])),
            Some(AiPolicyVerb::FirePhaser),
            "released, the Harrow fires with the alert at {alert} exactly as it \
             did before this issue existed"
        );
    }
}

/// **The weapon-family arc order, now that it is content (issue #956) — the
/// FALLBACK half.**
///
/// `tick_weapons_arc_request` used to choose which family to turn for from a
/// Rust array, `[Phasers, Blasters, Torpedoes]`. The fleet baseline authors
/// exactly that order, unconditionally, so a hull with no preference of its own
/// resolves the AUTHORED baseline rather than an inline constant — which is the
/// issue's third acceptance criterion.
///
/// Resolved through the REAL host helper over the REAL shipped block, so this
/// is the order ships actually fly, and it is asserted on an EMPTY fact snapshot
/// as well: the baseline must not depend on a reading that might be absent, or
/// the fleet would silently stop turning its guns onto anything (the #779
/// failure mode).
#[test]
fn the_fleet_baseline_arc_order_is_unconditional_on_facts() {
    let p = fleet_baseline_policy("weapons_doctrine");
    let order =
        |snapshot: &AiFacts| crate::console::weapons::resolve_arc_bearing_order(&p, snapshot, &[]);

    let baseline = order(&facts(&[]));
    assert!(
        !baseline.is_empty(),
        "the fleet baseline must resolve an arc-bearing order with NO facts at all, so a \
         hull that has not yet seen a target still knows which gun it would rather present."
    );
    for hp in [-10.0, 0.0, 40.0] {
        assert_eq!(
            order(&facts(
                &[("target_facing_shields", hp), ("red_alert", 1.0),]
            )),
            baseline,
            "target_facing_shields = {hp}: the BASELINE is unconditional. Reordering \
             on the target's screen is a doctrine a hull opts into (see \
             `the_harrow_cruiser_leads_with_its_tubes_into_a_shield_gap`), not \
             something the fleet does by default — that is what makes the departure \
             below a departure."
        );
    }
}

/// **…and the DOCTRINE half: "torpedoes when the target's shields are down".**
///
/// The issue's worked example, on the one shipped hull that authors it. The
/// Harrow cruiser fights with two 24-degree bow tubes; nothing but a deliberate
/// turn ever puts them on a target, so while the arc a round would strike is not
/// blocking, the family worth asking Helm to present is the TUBES.
///
/// Both directions, and the second is the one that matters: with the screen back
/// up the hull resolves the fleet baseline TERM FOR TERM, so the doctrine is a
/// conditional promotion rather than a permanently different ship.
#[test]
fn the_harrow_cruiser_leads_with_its_tubes_into_a_shield_gap() {
    use crate::core::messages::WeaponFamily::{Blasters, Phasers, Torpedoes};
    let hull = entity("ship_harrow_cruiser");
    let p = policy(
        hull.weapons_console
            .as_ref()
            .expect("the Harrow cruiser carries weapons")
            .ai
            .as_ref()
            .expect("…and authors `[weapons_console.ai]`"),
    );
    let order = |hp: f64| {
        crate::console::weapons::resolve_arc_bearing_order(
            &p,
            &facts(&[("target_facing_shields", hp), ("red_alert", 1.0)]),
            &[],
        )
    };

    assert_eq!(
        order(0.0),
        vec![Torpedoes, Phasers],
        "the striking arc is DOWN ⇒ the hull turns for its tubes first, then its \
         beams. The third rank names the tubes again and the host drops the repeat, \
         which is what keeps the baseline rules underneath authorable unchanged."
    );
    assert_eq!(
        order(-10.0),
        vec![Torpedoes, Phasers],
        "an arc driven past zero is still a gap — the guard is `<= 0`, the same \
         comparison the tubes' own launch predicate uses, over the same per-arc HP \
         reading."
    );
    assert_eq!(
        order(40.0),
        vec![Phasers, Blasters, Torpedoes],
        "the screen is UP ⇒ the fleet baseline, term for term. Without this row the \
         hull could be flying a third order nobody wrote down; with it, the \
         departure is exactly the promotion and nothing else."
    );
    assert_eq!(
        crate::console::weapons::resolve_arc_bearing_order(&p, &facts(&[]), &[]),
        vec![Phasers, Blasters, Torpedoes],
        "with NO striking-arc reading at all the promotion's guard reads false and \
         the hull falls back to the baseline. The doctrine fails CLOSED, so a \
         misspelled fact name costs the hull its bow-tube opening rather than \
         wedging it permanently onto the tubes."
    );
    assert_ne!(
        order(0.0),
        crate::console::weapons::resolve_arc_bearing_order(
            &fleet_baseline_policy("weapons_doctrine"),
            &facts(&[("target_facing_shields", 0.0), ("red_alert", 1.0)]),
            &[],
        ),
        "…and on the SAME snapshot the fleet baseline resolves something else \
         entirely, which is what makes this hull's BESPOKE_DOCTRINES entry mean \
         something rather than restating the default."
    );
}

/// **The torpedo shields-down gate, now that it is content (issue #956).**
///
/// `auto_fire_torpedo` used to open with
/// `if !target_locked || target_facing_shields > 0 { return vec![] }`. That
/// second clause is the fleet's entire torpedo doctrine — "phasers strip the
/// shields, torpedoes finish the hull" — and it sat in Rust, unconditionally,
/// UPSTREAM of every authored policy: a tube's `torpedo_launch` guard could only
/// ever narrow it (AND), never authorise a round while the striking arc was up
/// and never retune the threshold. #956 deleted the clause and every armed tube
/// in the fleet now authors it.
///
/// This is the truth table that guarantees the move cost nothing, and it is
/// stronger than what it replaced (four `auto_fire_torpedo` unit tests over a
/// hand-built input): it runs the REAL shipped policy, on both sides of the
/// fleet, over the same per-arc HP reading `seed_torpedo_tube_launch_facts`
/// seeds — so a hull that quietly dropped the clause fails here, and so does one
/// whose guard reads a fact name nobody seeds.
///
/// The `<= 0` threshold is read as a literal rather than a `param`, because that
/// is how the shipped content authors it — the two Harrow doctrines have carried
/// it as a literal since #791, and the baseline now matches them. It is still a
/// designer's lever: it is one number in a TOML guard.
#[test]
fn torpedo_launch_shield_gate_truth_table() {
    // (label, the hull's tube policy). The baseline hull holds fire until its
    // captain calls the alert; the Harrow warhawk is always armed, so the two
    // together show the shield clause is independent of the alert clause.
    let baseline = fleet_baseline_policy("torpedo_tube");
    let warhawk = entity("ship_harrow_warhawk");
    let warhawk_tube = policy(
        warhawk
            .torpedoes
            .as_ref()
            .expect("the warhawk carries tubes")
            .tubes
            .first()
            .expect("…at least one")
            .ai
            .as_ref()
            .expect("every shipped tube authors a policy"),
    );

    for (hull, p) in [
        ("the fleet baseline", &baseline),
        ("the warhawk", &warhawk_tube),
    ] {
        let alert = param(p, "min_alert_to_fire");
        // The other conjuncts each hull's own doctrine adds (`tubes_full`,
        // `loaded`, `in_arc`) are seeded favourable throughout, so the ONLY
        // thing moving between the rows below is the striking arc.
        let snapshot = |hp: f64| {
            facts(&[
                ("red_alert", alert.max(1.0)),
                ("tubes_full", 1.0),
                ("loaded", 1.0),
                ("in_arc", 1.0),
                ("target_facing_shields", hp),
            ])
        };

        assert_eq!(
            resolve(p, "torpedo_launch", &snapshot(0.0)).as_ref(),
            Some(&AiPolicyVerb::LaunchTorpedo),
            "{hull}: the striking arc is DOWN ⇒ launch. A target with no shield \
             arcs at all (asteroid, debris, unshielded NPC) reads 0 through the \
             same fact and is torpedo-eligible for the same reason."
        );
        assert_eq!(
            resolve(p, "torpedo_launch", &snapshot(-5.0)).as_ref(),
            Some(&AiPolicyVerb::LaunchTorpedo),
            "{hull}: an arc driven PAST zero is still down — the comparison is \
             `<= 0`, not `== 0`."
        );
        assert_eq!(
            resolve(p, "torpedo_launch", &snapshot(50.0)),
            None,
            "{hull}: the striking arc is UP ⇒ hold, with every other reading \
             favourable and the alert raised. This is the half that makes the \
             rows above mean something: without it the gate would be gone rather \
             than moved, and an AI crew would empty its magazine into a healthy \
             screen."
        );
        // A SECOND healthy reading, well clear of the first: the guard is a
        // comparison against zero, not a match on one number, so a fatter arc
        // holds the shot for the same reason a thin one does.
        //
        // Note what this table deliberately does NOT claim. The policy sees one
        // already-resolved scalar, so WHICH arc that number came from is not a
        // question it can be asked, and no row here can prove the reading is
        // per-arc. That is the HOST's half, pinned end to end by
        // `console::weapons::server_tests::
        // ai_torpedo_auto_fire_gates_on_the_arc_the_torpedo_would_strike`.
        assert_eq!(
            resolve(p, "torpedo_launch", &snapshot(120.0)),
            None,
            "{hull}: a healthy arc holds the shot at any HP — the guard compares \
             against zero rather than matching a particular reading."
        );
        assert_eq!(
            resolve(
                p,
                "torpedo_launch",
                &facts(&[
                    ("red_alert", alert.max(1.0)),
                    ("tubes_full", 1.0),
                    ("loaded", 1.0),
                    ("in_arc", 1.0),
                ])
            ),
            None,
            "{hull}: with NO striking-arc reading at all the comparison reads \
             false and the tube holds. The gate fails CLOSED, so a typo in the \
             fact name cannot turn a doctrine into an ungated launcher (#779)."
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

/// Boost is the only baseline that is an explicit IDLE, and the two hulls whose
/// doctrine engages the drive are not.
///
/// "Explicit policy or explicit idle" (#794 AC1) has a different authored shape
/// for each, and getting it backwards is not a validation error — an idle system
/// simply never acts.
///
/// # Why this says TWO hulls now (issue #875)
///
/// It said one until the player destroyer composed
/// `fragments/ai/movement_attack_pass.toml`. That fragment's escape leg burns
/// the drive, so the hull's Boost policy is a state machine rather than the
/// baseline's `idle = true` — and a fragment REPLACING an inherited idle
/// declaration is the exact case where getting it backwards is silent: `idle`
/// deep-merges as a scalar, so a movement fragment that forgot to clear it would
/// compose into a hull that validates, spawns, and never boosts.
///
/// The premise did not weaken; it widened. Both hulls are still asserted
/// individually, and the baseline is still asserted idle.
#[test]
fn boost_is_the_only_idle_baseline_and_two_hulls_depart_from_it() {
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
    for (hull, why) in [
        (
            "ship_harrow_destroyer",
            "authors its lance run inline, and was the first",
        ),
        (
            "alliance_destroyer",
            "COMPOSES `fragments/ai/movement_attack_pass.toml`, whose `idle = false` \
             is what clears the fleet baseline's inherited idle Boost. If that line \
             is ever dropped from the fragment this is the assertion that notices",
        ),
    ] {
        let cfg = entity(hull);
        let boost = cfg
            .helm_console
            .as_ref()
            .and_then(|h| h.boost_ai.as_ref())
            .unwrap_or_else(|| panic!("{hull} must resolve a `[helm_console.boost_ai]`"));
        let decoded = policy(boost);
        assert!(
            !decoded.idle,
            "{hull} is a hull whose AI engages boost — it {why} — and its entry on \
             BESPOKE_DOCTRINES means nothing if the policy is idle."
        );
        assert!(
            decoded.machine.is_some(),
            "{hull}'s boost doctrine is a state machine: the drive burns on the \
             escape leg and nowhere else, which cannot be said with a stateless rule."
        );
    }
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
        checked, 169,
        "the fifteen policy kinds account for 169 of the fleet's 229 AI-capable \
         fine-system slots (the other 60 are the selectors). A change in this number \
         means a hull, weapon or kind moved. 169-of-229 since #1163, which added the \
         ELEVENTH and TWELFTH AI-bearing hulls — `ship_harrow_tug` and \
         `alliance_tender`, eleven policy slots and five selectors EACH, both fully \
         composed from the fleet baseline with no bespoke captain; 147-of-197 since \
         #1028, which added the TENTH AI-bearing hull — `ship_civilian_hauler`, \
         eleven policy slots and five selectors, every one of them composed from the \
         fleet baseline except its stand-down captain; 136-of-181 since #956, which \
         added the ship-level `weapons_doctrine` kind — one slot on each of the nine \
         AI-bearing hulls; 127-of-172 after #954 moved the three-weapon RNG-coverage \
         escort out of `assets/entities/` to the test-fixture directory, and \
         141-of-191 before that."
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
///
/// The `tier_ordinal == 3` row is the deliberate exception: it is a POSITIVE
/// case since issue #1013, pinning that the worst candidate possible is
/// eligible rather than refused. The tier clause is still proved gating by the
/// Operational (`tier_ordinal == 0`) row above it.
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
        )
        .as_deref(),
        Some("station"),
        "a DESTROYED station (tier 3) IS selected since issue #1013. The old guard \
         `tier_ordinal < 3` refused exactly the worst-damaged candidate possible, \
         because a repair team could not then lift the latch; the on-site sweep \
         repairs destroyed systems now, so refusing them would strand a station \
         nothing else in the game can clear. The `tier_ordinal > 0` clause still \
         gates the tier reading independently — the Operational row above is the \
         case that reads it false."
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

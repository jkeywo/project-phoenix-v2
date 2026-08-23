use super::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Drop everything from the file's `#[cfg(test)] mod ...` marker onwards.
///
/// Unit tests pass `&[]` everywhere by design, so scanning them would drown
/// the signal. Item-level `#[cfg(test)] use ...` lines (which `src/ship/helm_ai.rs`
/// has near the top) are deliberately NOT treated as the marker — only the
/// attribute immediately followed by a `mod` item is.
fn strip_test_module(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start() != "#[cfg(test)]" {
            continue;
        }
        if lines
            .get(i + 1)
            .is_some_and(|next| next.trim_start().starts_with("mod "))
        {
            return lines[..i].join("\n");
        }
    }
    src.to_string()
}

fn read_non_test_source(rel: &str) -> String {
    let path = crate_root().join(rel);
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("scanned file {rel} must be readable: {e}"));
    strip_test_module(&src)
}

/// The body of `fn <name>`, from the definition to the closing brace of the
/// enclosing item, by brace counting from the signature's opening `{`.
fn function_body<'a>(src: &'a str, func: &str) -> &'a str {
    let needle = format!("fn {func}");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("no `{needle}` in the scanned source"));
    let open = start
        + src[start..]
            .find('{')
            .unwrap_or_else(|| panic!("`{needle}` has no body"));
    let mut depth = 0usize;
    for (offset, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("`{needle}` body is unbalanced");
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("src/ must be readable") {
        let path = entry.expect("readable dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
                // Relocated sibling test modules (issue #1180/#1190 pattern:
                // `#[path = "<name>_tests.rs"] mod tests;`) are entirely
                // `#[cfg(test)]` content — never a production call site — but
                // carry no `#[cfg(test)] mod` header of their own for
                // `strip_test_module` to find, since they ARE the module body
                // rather than a wrapper around it. Skip them here the same
                // way `strip_test_module` skips an inline `mod tests { .. }`,
                // so a fixture string that happens to contain
                // `HISTORY_FOLD_CALL` textually (as this file's own const
                // does) can't misattribute to the nearest `fn` above it.
                && !path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with("_tests.rs"))
        {
            out.push(path);
        }
    }
}

// ── The history fold (issue #890) ───────────────────────────────────────

/// The one method name that advances a bounded history window
/// (`AiPolicyMemory::fold_history` / `AiHistory::fold_history`), as it
/// appears at a CALL site. The two methods share one deliberately
/// distinctive name — not `fold` or `advance` — so this scan can be exact.
const HISTORY_FOLD_CALL: &str = ".fold_history(";

/// AC: a host that declares a fold site really folds there.
///
/// Without this the classification would be an assertion in a comment, and
/// the load-time rejection would let a `history(...)` guard through onto a
/// host whose window nothing advances — which is the exact failure the
/// rejection exists to prevent.
#[test]
fn every_declared_history_fold_site_really_folds() {
    for host in AI_HOSTS {
        let Some(site) = host.history_fold else {
            continue;
        };
        let src = read_non_test_source(site.file);
        let body = function_body(&src, site.func);
        assert!(
            body.contains(HISTORY_FOLD_CALL),
            "{} ({}) declares {}::{} as its history fold site, but that function \
                 never calls `{HISTORY_FOLD_CALL}`. Either point the entry at the \
                 function that advances the window, or set history_fold = None — a \
                 host that folds nothing must reject history(...) at load rather \
                 than let it read absent for ever",
            host.system,
            host.block,
            site.file,
            site.func
        );
    }
}

/// AC (the once-per-shared-tick guarantee, structurally): there is exactly
/// ONE fold site in the crate, and a host claims it.
///
/// This is the guard against the sharp edge #789 documented. The per-axis
/// helm actuator systems all resolve guards off the same ship in the same
/// tick; a fold added to one of them would advance every authored window
/// several times per shared tick, so `history(net_change, x, 30)` would
/// silently measure a fraction of the span the file says — with every other
/// assertion in the suite still green. A new fold site now has to be
/// declared here, where the reviewer can see whether it runs once per tick.
#[test]
fn every_history_fold_in_the_crate_belongs_to_a_host() {
    let root = crate_root();
    let claimed: BTreeSet<(&str, &str)> = AI_HOSTS
        .iter()
        .filter_map(|h| h.history_fold)
        .map(|s| (s.file, s.func))
        .collect();
    assert!(
        !claimed.is_empty(),
        "no host claims a fold site, so this scan would pass vacuously"
    );

    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let mut unclaimed: Vec<String> = Vec::new();
    let mut seen = 0usize;
    for path in files {
        let rel = path
            .strip_prefix(&root)
            .expect("scanned file is under the crate root")
            .to_string_lossy()
            .replace('\\', "/");
        let src = strip_test_module(&std::fs::read_to_string(&path).expect("readable source"));
        let mut from = 0usize;
        while let Some(hit) = src[from..].find(HISTORY_FOLD_CALL) {
            let at = from + hit;
            from = at + HISTORY_FOLD_CALL.len();
            let Some(fn_at) = src[..at].rfind("fn ") else {
                continue;
            };
            let name: String = src[fn_at + 3..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            // `AiPolicyMemory::fold_history` delegates to
            // `AiHistory::fold_history`. A call from the method of the same
            // name is that delegation, not a second place a window advances.
            if name == "fold_history" {
                continue;
            }
            seen += 1;
            if !claimed.contains(&(rel.as_str(), name.as_str())) {
                unclaimed.push(format!("{rel}::{name}"));
            }
        }
    }
    unclaimed.sort();
    unclaimed.dedup();
    assert!(
        unclaimed.is_empty(),
        "these functions advance a bounded history window but no AiHost claims \
             them as its fold site: {unclaimed:?}. A window must be folded EXACTLY \
             ONCE per shared AI tick — four per-axis hosts folding the same ship's \
             window would quarter every authored span. Add the host's history_fold \
             entry, or move the fold to the shared per-tick host."
    );
    assert_eq!(
        seen,
        claimed.len(),
        "expected exactly one fold call per claimed site; {seen} were found"
    );
}

/// The hosts that fold a window today, pinned by name — the reading of how
/// far the operator has reached, exactly as the flag-chain roll call is.
#[test]
fn exactly_the_three_helm_machine_axes_fold_history_today() {
    let folding: Vec<&str> = AI_HOSTS
        .iter()
        .filter(|h| h.history_fold.is_some())
        .map(|h| h.system)
        .collect();
    assert_eq!(
        folding,
        vec!["Helm engines", "Helm steering", "Helm boost"],
        "the set of history-folding hosts changed. Widening it is how a new \
             host gains the operator — update this list, and check the new fold \
             really runs once per shared AI tick. Anything else means a host \
             silently lost its window and its authored guards have gone quiet."
    );
}

#[test]
fn an_unfolded_host_rejects_history_and_nothing_else() {
    let windowed =
        crate::world::flags::parse_predicate("history(min, range_to_target, 30) >= 40").unwrap();
    let err = CAPTAIN_RED_ALERT
        .check_guard("rule 0", &windowed)
        .unwrap_err();
    assert!(err.contains("Captain"), "must name the system: {err}");
    assert!(
        err.contains("history(min, range_to_target, 30)"),
        "must quote the atom: {err}"
    );
    assert!(
        err.contains("folds a bounded history window"),
        "must say plainly why: {err}"
    );

    // Nested under `not`/`and` is still a reference.
    let nested =
        crate::world::flags::parse_predicate("fact(a) > 0 and not history(max, b, 4) < 1").unwrap();
    assert!(CAPTAIN_RED_ALERT.check_guard("rule 0", &nested).is_err());

    // Everything else on an unfolded host is untouched by this check.
    let plain = crate::world::flags::parse_predicate("fact(a) > param(b)").unwrap();
    assert!(CAPTAIN_RED_ALERT.check_guard("rule 0", &plain).is_ok());
}

#[test]
fn folding_hosts_accept_history() {
    let windowed = crate::world::flags::parse_predicate(
        "history(min, range_to_target, param(w)) >= param(safe)",
    )
    .unwrap();
    for host in [HELM_ENGINES, HELM_STEERING, HELM_BOOST] {
        assert!(
            host.check_guard("rule 0", &windowed).is_ok(),
            "{} folds a window once per shared tick, so a history guard on it is \
                 valid content",
            host.system
        );
    }
}

/// The three helm hosts fold, but the three per-axis ACTUATOR axes sitting
/// beside them do not — and that asymmetry is the whole point. Their guards
/// are resolved through the stateless `decide` path (no state component, no
/// per-system bag to fold into), so a window authored there would read absent
/// for ever.
#[test]
fn the_per_axis_actuator_hosts_reject_history() {
    let windowed =
        crate::world::flags::parse_predicate("history(min, hazard_urgency, 8) >= 1").unwrap();
    for host in [HELM_LATERAL, HELM_VERTICAL, HELM_IMPULSE] {
        let err = host
            .check_guard("rule 0", &windowed)
            .expect_err("an unfolded helm axis must reject a windowed guard");
        assert!(err.contains(host.system), "{err}");
    }
}

/// EVERY host reads world flags since #891 stage 2 — pinned as a property
/// rather than a name list, so adding a twentieth plumbed host needs no
/// edit here while a host that declares `FlagChain::Empty` still fails,
/// its authored `flag()`/`counter()` guards rejected at load by
/// [`AiHost::check_world_state`].
#[test]
fn every_host_reads_world_flags_today() {
    let unplumbed: Vec<&str> = AI_HOSTS
        .iter()
        .filter(|h| h.flag_chain != FlagChain::Plumbed)
        .map(|h| h.system)
        .collect();
    assert_eq!(
        unplumbed,
        Vec::<&str>::new(),
        "a host lost its world-flag chain: authored flag()/counter() \
             guards on it now reject at load, and #891 stage 2 promised the \
             feature works everywhere the grammar says it does."
    );
}

#[test]
fn every_host_block_is_named_once() {
    let mut blocks: Vec<&str> = AI_HOSTS.iter().map(|h| h.block).collect();
    blocks.sort_unstable();
    let unique: BTreeSet<&str> = blocks.iter().copied().collect();
    assert_eq!(
        blocks.len(),
        unique.len(),
        "two hosts claim the same authored block, so a rejection would name \
             the wrong system"
    );
}

/// The rejection machinery outlives stage 2 — a FUTURE host added with an
/// empty chain must still reject authored world-state guards at load. No
/// shipped host is `Empty` any more, so the surface is pinned through a
/// synthetic one.
const UNPLUMBED_PROBE: AiHost = AiHost {
    system: "Probe",
    block: "[probe.ai]",
    flag_chain: FlagChain::Empty,
    history_fold: None,
    // A single seeded candidate fact, so the flag()/counter() selector
    // rejection tests below can use `candidate_fact(detectable)` in their
    // eligibility WITHOUT tripping the #1210 fact check first — the probe
    // exists to exercise the empty flag chain, not an empty fact vocabulary.
    // Any OTHER fact name on it is still rejected, which is what
    // `an_unplumbed_probe_rejects_an_unseeded_fact` proves.
    facts: &[cand(
        DETECTABLE,
        "probe",
        "1.0 for a detectable candidate",
        "false",
        "test fixture",
    )],
};

#[test]
fn an_unplumbed_host_rejects_flag_and_counter_and_nothing_else() {
    let flag = crate::world::flags::parse_predicate("flag(aphelion_armed)").unwrap();
    let err = UNPLUMBED_PROBE.check_guard("rule 0", &flag).unwrap_err();
    assert!(err.contains("Probe"), "message must name the system: {err}");
    assert!(
        err.contains("flag(aphelion_armed)"),
        "message must quote the atom: {err}"
    );
    assert!(
        err.contains("NO world-flag chain plumbed"),
        "message must say the chain is not plumbed: {err}"
    );

    let counter = crate::world::flags::parse_predicate("counter(evacuation_rounds) >= 3").unwrap();
    let err = UNPLUMBED_PROBE.check_guard("rule 0", &counter).unwrap_err();
    assert!(err.contains("counter(evacuation_rounds)"), "{err}");

    // Nested under `not`/`and` is still a reference.
    let nested = crate::world::flags::parse_predicate("fact(a) > 0 and not flag(b)").unwrap();
    assert!(UNPLUMBED_PROBE.check_guard("rule 0", &nested).is_err());

    // Facts, params, memory and literals are untouched by this check.
    let facts =
        crate::world::flags::parse_predicate("fact(a) > param(b) and memory(c) < 1").unwrap();
    assert!(UNPLUMBED_PROBE.check_guard("rule 0", &facts).is_ok());
}

#[test]
fn plumbed_hosts_accept_flag_and_counter() {
    let flag = crate::world::flags::parse_predicate(
        "flag(containment_started) and counter(evacuation_rounds) >= 1",
    )
    .unwrap();
    for host in AI_HOSTS {
        assert!(
            host.check_guard("rule 0", &flag).is_ok(),
            "{} passes a real flag chain (#891 stage 2), so its guards must \
                 be accepted",
            host.system
        );
    }
}

// ── Through the real validators ─────────────────────────────────────────

fn policy(src: &str) -> crate::entities::config::FineSystemAiConfigToml {
    toml::from_str(src).expect("fixture policy parses")
}

fn selector(src: &str) -> crate::entities::config::FineSystemAiSelectorToml {
    toml::from_str(src).expect("fixture selector parses")
}

const RED_ALERT: &[&str] = &[crate::entities::config::CAPTAIN_RED_ALERT_CHANNEL];
const SET_RED_ALERT: &[&str] = &[crate::entities::config::CAPTAIN_SET_RED_ALERT_VERB];

/// Every place a policy can carry a guard is walked, not just the first.
///
/// A stateful policy hides guards in two more positions than a stateless
/// one — per-state rules and transitions — and a check that only walked the
/// top-level list would leave both silently trapped, which is the whole
/// defect being closed.
#[test]
fn a_flag_guard_is_rejected_in_every_authorable_position() {
    let stateless = policy(
        r#"
            [[rule]]
            priority = 0
            channel = "red_alert"
            when = "flag(battle_stations)"
            verb = "set_red_alert"
            value = true
            "#,
    );
    let err = crate::entities::config::validate_fine_system_ai_policy_for(
        &UNPLUMBED_PROBE,
        &stateless,
        RED_ALERT,
        SET_RED_ALERT,
    )
    .expect_err("a top-level rule guard must be walked");
    assert!(err.contains("rule 0") && err.contains("Probe"), "{err}");

    // The same policy on a PLUMBED host — every shipped host today — is
    // valid content: the stage-1 rejection is lifted (#891 stage 2).
    crate::entities::config::validate_fine_system_ai_policy_for(
        &CAPTAIN_RED_ALERT,
        &stateless,
        RED_ALERT,
        SET_RED_ALERT,
    )
    .expect("a flag() guard on a plumbed host validates");

    let machine = policy(
        r#"
            initial_state = "calm"

            [[state]]
            id = "calm"
            [[state.rule]]
            priority = 0
            channel = "red_alert"
            when = "counter(alert_level) >= 2"
            verb = "set_red_alert"
            value = true
            [[state.transition]]
            priority = 0
            to = "hot"
            when = "true"

            [[state]]
            id = "hot"
            "#,
    );
    let err = crate::entities::config::validate_fine_system_ai_policy_for(
        &UNPLUMBED_PROBE,
        &machine,
        RED_ALERT,
        SET_RED_ALERT,
    )
    .expect_err("a per-state rule guard must be walked");
    assert!(err.contains("counter(alert_level)"), "{err}");

    let transition = policy(
        r#"
            initial_state = "calm"

            [[state]]
            id = "calm"
            [[state.transition]]
            priority = 0
            to = "hot"
            when = "flag(battle_stations)"

            [[state]]
            id = "hot"
            [[state.rule]]
            priority = 0
            channel = "red_alert"
            when = "true"
            verb = "set_red_alert"
            value = true
            "#,
    );
    let err = crate::entities::config::validate_fine_system_ai_policy_for(
        &UNPLUMBED_PROBE,
        &transition,
        RED_ALERT,
        SET_RED_ALERT,
    )
    .expect_err("a transition guard must be walked");
    assert!(
        err.contains("transition 0") && err.contains("flag(battle_stations)"),
        "{err}"
    );
}

/// The same walk over the selector surface: eligibility AND every score
/// term, since a flag-weighted score term reads false for ever just as
/// quietly as a flag-gated eligibility.
#[test]
fn a_flag_guard_is_rejected_in_both_selector_positions() {
    let sources = crate::entities::config::SENSORS_SELECTOR_SOURCES;
    let eligibility = selector(
        r#"
            horizon = 100.0
            switch_margin = 0.0
            eligibility = "candidate_fact(detectable) > 0 and flag(sensors_online)"
            "#,
    );
    let err = crate::entities::config::validate_fine_system_ai_selector_for(
        &UNPLUMBED_PROBE,
        &eligibility,
        sources,
    )
    .expect_err("the eligibility expression must be walked");
    assert!(
        err.contains("Probe") && err.contains("flag(sensors_online)"),
        "{err}"
    );

    // The same selector on the (now plumbed, #891 stage 2) Sensors host is
    // valid content.
    crate::entities::config::validate_fine_system_ai_selector_for(
        &SENSORS_SELECTOR,
        &eligibility,
        sources,
    )
    .expect("a flag() eligibility on the plumbed Sensors selector validates");

    let scored = selector(
        r#"
            horizon = 100.0
            switch_margin = 0.0
            eligibility = "candidate_fact(detectable) > 0"

            [[score]]
            when = "counter(threat_level) >= 1"
            weight = 5.0
            "#,
    );
    let err = crate::entities::config::validate_fine_system_ai_selector_for(
        &UNPLUMBED_PROBE,
        &scored,
        sources,
    )
    .expect_err("every score term's `when` must be walked");
    assert!(
        err.contains("score term 0") && err.contains("counter(threat_level)"),
        "{err}"
    );

    // The one selector host that CAN read flags keeps accepting both.
    let ok = selector(
        r#"
            horizon = 100.0
            switch_margin = 0.0
            eligibility = "candidate_fact(in_range) > 0 and flag(diplomatic_clearance)"

            [[score]]
            when = "counter(hails_sent) >= 1"
            weight = 1.0
            "#,
    );
    crate::entities::config::validate_fine_system_ai_selector_for(
        &COMMS_SELECTOR,
        &ok,
        crate::entities::config::COMMS_SELECTOR_SOURCES,
    )
    .expect("the Comms hail selector passes a real flag chain");
}

// ── Through the real entity-load path, on a real shipped hull ───────────

/// The shipped cruiser as loadable TEXT — the RESOLVED document, not the
/// file (issue #876).
///
/// `include_str!` bakes bytes at compile time, so a baked site can never see
/// include resolution, and `alliance_cruiser` is a COMPOSED hull since #876:
/// its `[captain_console.ai]` comes from `fragments/ai/captain_alliance.toml`
/// and its `[power.ai_policy]` from `fragments/ai/fleet_baseline.toml`. Both
/// mutations below substitute a guard inside one of those blocks, so the
/// resolved document is the only text that carries them — and it is also the
/// only text `EntityConfig::from_toml` accepts, since the raw file now
/// carries an `includes` key the parser rejects.
fn cruiser() -> String {
    crate::entities::include_resolve::resolve_from_disk("assets/entities/alliance_cruiser.toml")
        .expect("alliance_cruiser must compose")
        .toml
}

/// Substitute one authored guard in a shipped hull, asserting the target was
/// actually there — a silently-missed replacement would turn the mutation
/// tests below into vacuous passes.
fn with_guard(hull: &str, from: &str, to: &str) -> String {
    assert_eq!(
        hull.matches(from).count(),
        1,
        "the guard being mutated must appear exactly once in the hull"
    );
    hull.replace(from, to)
}

/// AC (#891 stage 2, replacing the stage-1 rejection test): a `flag()`
/// guard authored on the Captain host of a REAL hull loads — and WORKS,
/// in both directions, through the loaded policy. Fires with the flag set
/// and reads false with it clear: a guard that only ever read false would
/// pass the load half and fail the fired half.
#[test]
fn a_flag_guard_on_a_shipped_hull_loads_and_reads_the_world() {
    // Sanity: the unmutated hull loads, so any difference below is the guard.
    let cruiser = cruiser();
    crate::entities::config::EntityConfig::from_toml(&cruiser)
        .expect("alliance_cruiser loads as shipped");

    let mutated = with_guard(
        &cruiser,
        r#"when = "fact(secs_since_combat) < param(combat_window_secs)""#,
        r#"when = "fact(secs_since_combat) < param(combat_window_secs) and flag(battle_stations)""#,
    );
    let cfg = crate::entities::config::EntityConfig::from_toml(&mutated)
        .expect("a flag() guard on the Captain host is valid content since #891 stage 2");
    let policy = cfg
        .captain_console
        .as_ref()
        .and_then(|c| c.ai.as_ref())
        .expect("the mutated hull still authors [captain_console.ai]")
        .to_policy()
        .expect("and it decodes");

    // Freshly in combat, so the mutated rule's fact half is TRUE and the
    // outcome is decided by the flag alone.
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("secs_since_combat", 1.0);
    facts.set(crate::entities::config::CAPTAIN_HOSTILE_CONTACT_FACT, 0.0);
    facts.set(crate::entities::config::CAPTAIN_HOSTILE_RANGE_FACT, 0.0);

    let mut world = crate::world::flags::FlagStore::default();
    world.set_flag("battle_stations");
    assert_eq!(
        policy.resolve_channel(
            crate::entities::config::CAPTAIN_RED_ALERT_CHANNEL,
            &facts,
            &[&world],
        ),
        Some(&crate::ai::policy::AiPolicyVerb::SetRedAlert(true)),
        "with the world flag SET the guard fires and the alert goes up"
    );

    let clear = crate::world::flags::FlagStore::default();
    assert_eq!(
        policy.resolve_channel(
            crate::entities::config::CAPTAIN_RED_ALERT_CHANNEL,
            &facts,
            &[&clear],
        ),
        Some(&crate::ai::policy::AiPolicyVerb::SetRedAlert(false)),
        "with the world flag CLEAR the guard reads false and the stand-down \
             rule wins instead"
    );
}

/// AC: the hosts that CAN evaluate flags still accept them — same hull, same
/// load path, a flag added to its `[power.ai_policy]` instead.
///
/// The guard mutated is the weapons ELEVATE rule, which since issue #1003
/// reads the RESTORE floor (`min_restore_weapons`) — its sibling hold rule
/// is the one that reads `min_reserve_weapons`. Either would do; naming the
/// restore floor is what keeps [`with_guard`]'s exactly-once assertion true.
#[test]
fn a_flag_guard_on_a_plumbed_host_of_a_shipped_hull_still_loads() {
    let mutated = with_guard(
        &cruiser(),
        r#"when = "fact(red_alert) > 0 and fact(battery_pct) >= param(min_restore_weapons)""#,
        r#"when = "fact(red_alert) > 0 and fact(battery_pct) >= param(min_restore_weapons) and flag(weapons_free)""#,
    );
    crate::entities::config::EntityConfig::from_toml(&mutated).expect(
        "the Power reactor passes a real flag chain, so a flag() guard on it is valid content",
    );
}

// ── History, through the real entity-load path (issue #890) ─────────────

/// The shipped destroyer as the loader sees it — RESOLVED, because the hull
/// has composed its movement doctrine out of the fragment library since issue
/// #878 and the guards mutated below are authored in the fragment now.
fn destroyer() -> String {
    crate::entities::include_resolve::resolve_from_disk(
        "assets/entities/ship_harrow_destroyer.toml",
    )
    .expect("ship_harrow_destroyer must resolve")
    .toml
}

/// The destroyer's own re-entry gate, which #788 could only express as a
/// bespoke host-folded fact. The window is authored on the same param the
/// bespoke plumbing already reads.
const AUTHORED_WINDOW: &str = "history(min, range_to_target, param(safe_distance_window_ticks)) \
                                   >= param(safe_range_margin)";

/// The destroyer's UNCONDITIONAL recovery-orbit rule, in the resolved
/// document's rendering.
///
/// Anchored `verb`-then-`when` because a composed hull's resolved text is
/// re-rendered from the merged value rather than concatenated (issue #878):
/// keys come out sorted and the authored indentation is gone. The `when =
/// "true"` half is what makes this the `recover` leg's rule and not the
/// `shadow` leg's, whose own `hold_recovery_orbit` is guarded on
/// `target_valid`.
const RECOVERY_ORBIT_RULE: &str = "verb = \"hold_recovery_orbit\"\nwhen = \"true\"";

/// The destroyer's `recover` re-entry transition, which all THREE of its
/// machine axes author an independent copy of. Substituted in every copy, so
/// the mutation exercises each folded host rather than only the first.
const RECOVER_GUARD: &str = r#"when = "fact(shield_fraction) >= param(reentry_shield_fraction) and fact(safe_distance_held) > 0""#;

/// [`with_guard`] for a guard the hull authors once PER AXIS. Asserts the
/// expected count for the same reason `with_guard` asserts one: a
/// silently-missed substitution turns the test into a vacuous pass.
fn with_guard_everywhere(hull: &str, from: &str, to: &str, expected: usize) -> String {
    assert_eq!(
        hull.matches(from).count(),
        expected,
        "the guard being mutated must appear on every axis that authors it"
    );
    hull.replace(from, to)
}

/// AC: the two window shapes are authorable in a real hull's policy guard
/// and the hull LOADS — the mechanism reaches content, not just unit tests.
#[test]
fn a_history_guard_on_a_shipped_hull_loads() {
    crate::entities::config::EntityConfig::from_toml(&destroyer())
        .expect("ship_harrow_destroyer loads as shipped");

    // A transition guard (the position #788's bespoke fact was confined to),
    // on all three of the hull's machine axes. A LITERAL window here because
    // only the Steering axis declares the standoff length as a param — the
    // authored-param spelling is exercised on the rule below.
    let transition = with_guard_everywhere(
        &destroyer(),
        RECOVER_GUARD,
        r#"when = "fact(shield_fraction) >= param(reentry_shield_fraction) and history(min, range_to_target, 60) >= 0""#,
        3,
    );
    crate::entities::config::EntityConfig::from_toml(&transition).expect(
        "the helm machine axes fold their window once per shared tick, so a \
             windowed transition guard is valid content",
    );

    // And a per-state RULE guard — the position the bespoke facts could NOT
    // be authored in, because the per-axis hosts never seeded them. Anchored
    // on the recovery-orbit verb, which only the Steering axis authors, and
    // with the window length AUTHORED as one of that axis's own params.
    let rule = with_guard(
        &destroyer(),
        RECOVERY_ORBIT_RULE,
        &format!("verb = \"hold_recovery_orbit\"\nwhen = \"{AUTHORED_WINDOW}\""),
    );
    crate::entities::config::EntityConfig::from_toml(&rule)
        .expect("a windowed per-state rule guard is valid content on the same host");
}

/// The NET-CHANGE shape too, over the hull's own authored `pressed_window_ticks`
/// — both operators the shipped doctrines hand-rolled are authorable, not
/// just the threshold one.
#[test]
fn a_net_change_guard_on_a_shipped_hull_loads() {
    let mutated = with_guard(
        &destroyer(),
        RECOVERY_ORBIT_RULE,
        "verb = \"hold_recovery_orbit\"\nwhen = \"history(net_change, range_to_target, \
             param(pressed_window_ticks)) > param(pressed_min_progress)\"",
    );
    crate::entities::config::EntityConfig::from_toml(&mutated)
        .expect("a net-change window over the hull's authored span is valid content");
}

/// AC: a history operator authored where it cannot be evaluated is rejected
/// at LOAD, not silently false — on a real hull, through the real path.
#[test]
fn a_history_guard_on_an_unfolded_host_fails_to_load() {
    let mutated = with_guard(
        &cruiser(),
        r#"when = "fact(secs_since_combat) < param(combat_window_secs)""#,
        r#"when = "history(min, secs_since_combat, 30) < param(combat_window_secs)""#,
    );
    let err = crate::entities::config::EntityConfig::from_toml(&mutated)
        .expect_err("a history() guard on the Captain host must fail the load")
        .to_string();
    assert!(
        err.contains("Captain") && err.contains("[captain_console.ai]"),
        "the load error must name the system and its block: {err}"
    );
    assert!(
        err.contains("history(min, secs_since_combat, 30)")
            && err.contains("folds a bounded history window"),
        "the load error must quote the atom and say plainly why: {err}"
    );
}

/// AC: a malformed window length is a load error naming the problem — the
/// half of the check the parser cannot make, because only the hull knows
/// what its parameter is worth.
#[test]
fn a_fractional_window_param_fails_to_load() {
    let windowed = with_guard(
        &destroyer(),
        RECOVERY_ORBIT_RULE,
        &format!("verb = \"hold_recovery_orbit\"\nwhen = \"{AUTHORED_WINDOW}\""),
    );
    let mutated = with_guard(
        &windowed,
        "safe_distance_window_ticks = 60.0",
        "safe_distance_window_ticks = 60.5",
    );
    let err = crate::entities::config::EntityConfig::from_toml(&mutated)
        .expect_err("a fractional window length must fail the load")
        .to_string();
    assert!(
        err.contains("positive whole number") && err.contains("60.5"),
        "the load error must name the problem and the offending value: {err}"
    );
}

// ── The red-alert fire gate is DATA (issue #872) ────────────────────────

/// The shipped cruiser's authored fire gate, exactly as every armed hull in
/// the fleet writes it. One predicate text; only `min_alert_to_fire`
/// differs between a hull with a captain and a hull without.
const FIRE_GATE: &str = r#"when = "fact(red_alert) >= param(min_alert_to_fire)""#;

/// …and the TUBE form of the same gate. Since issue #956 a tube's launch
/// guard conjoins the fleet's torpedo doctrine — the arc a round would
/// strike must not be blocking — onto the same shared alert clause, so the
/// text is longer while the alert half is identical. Mutated alongside the
/// beam form so the counts below still cover every armed weapon on the hull.
///
/// The shield half is the fleet's literal `<= 0`. It was a `param` for one
/// release — issue #929's first pass gave this hull a `max_striking_shield_hp`
/// past any arc reading; its second pass put the fleet text back and raised the
/// hull's phaser banks instead. Either way it is incidental to what this
/// constant is for: only the ALERT clause is removed below, and whatever the
/// shield clause says rides through the mutation unchanged.
const TUBE_FIRE_GATE: &str =
    r#"when = "fact(red_alert) >= param(min_alert_to_fire) and fact(target_facing_shields) <= 0""#;

/// The cruiser authors the gate on two phaser banks (beam form) and three
/// torpedo tubes (tube form). Stated as numbers so a refit that adds or
/// removes a weapon fails here rather than silently leaving one of them
/// unmutated — which is how the mutation below would turn into a vacuous
/// pass.
const CRUISER_GATED_BEAMS: usize = 2;
const CRUISER_GATED_TUBES: usize = 3;

/// Resolve every phaser bank on a hull's `phaser_fire` channel over a fact
/// snapshot with NO red alert.
fn phaser_banks_fire_with_the_alert_down(hull: &str) -> Vec<bool> {
    let cfg = crate::entities::config::EntityConfig::from_toml(hull)
        .expect("the hull under test must load");
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set(crate::entities::config::POWER_RED_ALERT_FACT, 0.0);
    cfg.weapons_console
        .as_ref()
        .expect("the hull carries phasers")
        .phaser_banks
        .iter()
        .map(|bank| {
            let policy = bank
                .ai
                .as_ref()
                .expect("every shipped bank authors a policy")
                .to_policy()
                .expect("and it decodes");
            policy.resolve_channel(crate::entities::config::PHASER_FIRE_CHANNEL, &facts, &[])
                == Some(&crate::ai::policy::AiPolicyVerb::FirePhaser)
        })
        .collect()
}

/// **AC4, the data-driven proof.** A hull whose authored fire predicate is
/// removed has NO fire gate.
///
/// The claim under test is not "red alert gates fire" — it is that the
/// gating lives in `assets/entities/*.toml` and nowhere else. So this runs
/// the real load path over the real shipped cruiser twice: once as shipped,
/// where the alert is down and every bank holds; and once with the guard
/// string-substituted back to the unconditional `when = "true"` the hulls
/// carried before #872, where every bank fires on the same snapshot. The
/// `min_alert_to_fire` param is deliberately LEFT DECLARED in the mutated
/// copy: it is the predicate that gates, and an unreferenced param gates
/// nothing.
///
/// If any Rust host were to test red alert itself, the second half would
/// still hold and the behavioural tests would still pass — which is why
/// `no_weapons_host_decides_fire_from_red_alert_in_rust` sits beside this
/// one.
#[test]
fn removing_the_authored_fire_gate_removes_the_gate() {
    let cruiser = cruiser();
    let shipped = phaser_banks_fire_with_the_alert_down(&cruiser);
    assert_eq!(
        shipped.len(),
        2,
        "the cruiser's two phaser banks are the subject of this test"
    );
    assert!(
        shipped.iter().all(|fired| !fired),
        "as shipped, with the alert down, every cruiser bank holds fire"
    );

    // Both forms of the one gate, so the mutated hull has no alert clause
    // left anywhere. The tube form keeps its striking-arc conjunct — that
    // is the TORPEDO doctrine (issues #956, #929) and a different decision;
    // only the alert half is being removed.
    let ungated = with_guard_everywhere(
        &cruiser,
        TUBE_FIRE_GATE,
        r#"when = "fact(target_facing_shields) <= 0""#,
        CRUISER_GATED_TUBES,
    );
    let ungated =
        with_guard_everywhere(&ungated, FIRE_GATE, r#"when = "true""#, CRUISER_GATED_BEAMS);
    let mutated = phaser_banks_fire_with_the_alert_down(&ungated);
    assert!(
        mutated.iter().all(|fired| *fired),
        "with the authored predicate removed the SAME hull, through the SAME \
             load path, on the SAME fact snapshot, fires with no red alert. The \
             gate is the authored predicate — nothing in Rust reimposes it."
    );
}

/// The other half of AC4: no weapons host decides fire from red alert in
/// Rust. The hosts may only SEED the fact.
///
/// Without this, a Rust-side `if red_alert` could be added beside the
/// authored predicate and every other test in the issue would still pass —
/// the behaviour would look identical right up until a designer removed the
/// predicate and found the gate still there.
#[test]
fn no_weapons_host_decides_fire_from_red_alert_in_rust() {
    for file in [
        "src/console/weapons/beam.rs",
        "src/console/weapons/blaster.rs",
        "src/console/weapons/torpedo.rs",
        "src/console_ai/server.rs",
    ] {
        // The ONE permitted branch on the value: turning the bool into the
        // `1.0`/`0.0` the fact carries. Removed before the scan so the scan
        // itself can be blunt.
        let src = read_non_test_source(file).replace("if red_alert { 1.0 } else { 0.0 }", "");
        // Everything else that branches on it — `if red_alert`,
        // `&& red_alert`, `!red_alert` — is a Rust rule and belongs in TOML.
        for forbidden in [
            "if red_alert",
            "&& red_alert",
            "|| red_alert",
            "!red_alert",
            "if !red_alert",
            "red_alert {",
        ] {
            assert!(
                !src.contains(forbidden),
                "{file} contains `{forbidden}`: a weapons host is deciding fire \
                     from red alert in Rust. The host's only job is to SEED \
                     `fact(red_alert)`; the decision is the hull's authored \
                     predicate (issue #872, AC4)."
            );
        }
    }
}

/// Every production validation call must name its host, or the rejection
/// simply does not run for that block.
///
/// The host-less `validate_fine_system_ai_*` entry points exist for unit
/// tests (and for `src/ship/helm_ai.rs`'s own fixtures), so they cannot be
/// removed — which makes "production accidentally calls the host-less one"
/// a live and completely silent way to reopen the trap.
#[test]
fn production_validation_names_its_host() {
    // The host-less `validate_fine_system_ai_*` entry points DEFINE in the
    // extracted `ai_policy_schema` module (issue #1196); production validation
    // still lives in `config`'s `EntityConfig::from_toml`. Scan both: every
    // occurrence must be the definition itself (never a call), and the
    // definition must be seen exactly once across the two sources.
    let sources = [
        read_non_test_source("src/entities/ai_policy_schema.rs"),
        read_non_test_source("src/entities/config.rs"),
    ];
    for hostless in [
        "validate_fine_system_ai_policy(",
        "validate_fine_system_ai_selector(",
    ] {
        let mut definitions = 0;
        for src in &sources {
            let mut from = 0usize;
            while let Some(hit) = src[from..].find(hostless) {
                let at = from + hit;
                from = at + hostless.len();
                let is_definition = src[..at].ends_with("fn ");
                assert!(
                    is_definition,
                    "an entity-config source calls the host-less `{hostless}` outside \
                         a definition. Production validation must use the `_for` variant \
                         with the owning ai_flag_hosts::AiHost, or a flag()/counter() \
                         guard on that block goes back to reading false in silence."
                );
                definitions += 1;
            }
        }
        // Non-vacuity: the definition itself must have been seen, or the
        // scan found nothing and this test proves nothing.
        assert_eq!(
            definitions, 1,
            "expected to find exactly the definition of `{hostless}` across the \
                 non-test entity-config sources; the scan is looking in the wrong place"
        );
    }
}

// ── The typed fact registry (issue #1210) ───────────────────────────────

/// Every descriptor across every host, as `(name, scope, shape)`.
fn all_descriptors() -> Vec<(&'static str, FactScope, FactShape)> {
    AI_HOSTS
        .iter()
        .flat_map(|h| h.facts.iter())
        .map(|d| (d.name.name(), d.scope, d.shape))
        .collect()
}

/// The drift check, small-table form (the AC3 "pins literal<->registry"):
/// every [`FACT_CATALOGUE`] constant is seeded by some host, and every host
/// descriptor names a catalogue constant. Neither side can gain or lose a
/// fact without the other being updated — the same guard STYLE the flag-chain
/// and history tables use, over a table rather than the crate's own source.
#[test]
fn the_fact_catalogue_and_the_host_registry_agree() {
    let catalogue: BTreeSet<&str> = FACT_CATALOGUE.iter().map(|f| f.name()).collect();
    assert_eq!(
        catalogue.len(),
        FACT_CATALOGUE.len(),
        "FACT_CATALOGUE lists the same fact name twice"
    );

    let registered: BTreeSet<&str> = all_descriptors().iter().map(|(n, _, _)| *n).collect();

    let unseeded: Vec<&str> = catalogue.difference(&registered).copied().collect();
    assert!(
        unseeded.is_empty(),
        "these catalogued facts are seeded by NO host, so validation would reject \
             an authored guard that reads them: {unseeded:?}. Add each to the host that \
             seeds it, or drop it from FACT_CATALOGUE."
    );

    let unlisted: Vec<&str> = registered.difference(&catalogue).copied().collect();
    assert!(
        unlisted.is_empty(),
        "these host descriptors name a fact absent from FACT_CATALOGUE, so the \
             catalogue no longer speaks for the registry: {unlisted:?}. Add each to \
             FACT_CATALOGUE."
    );
}

/// The ten catalogue entries borrowed from an `entities::config` `*_FACT`
/// const carry that const's exact string, pinned against both the const and
/// the literal so neither definition can drift.
#[test]
fn the_catalogue_matches_the_config_fact_consts() {
    use crate::entities::config as cfg;
    // Qualified with `super::` because the test module also binds a
    // `RED_ALERT` (the captain channel list), which would otherwise shadow
    // the catalogue constant of the same name.
    let pairs: &[(FactId, &str, &str)] = &[
        (
            super::HOSTILE_CONTACT,
            cfg::CAPTAIN_HOSTILE_CONTACT_FACT,
            "hostile_contact",
        ),
        (
            super::HOSTILE_RANGE,
            cfg::CAPTAIN_HOSTILE_RANGE_FACT,
            "hostile_range",
        ),
        (
            super::BATTERY_PCT,
            cfg::POWER_BATTERY_PCT_FACT,
            "battery_pct",
        ),
        (super::THRUST, cfg::POWER_THRUST_FACT, "thrust"),
        (super::RED_ALERT, cfg::POWER_RED_ALERT_FACT, "red_alert"),
        (
            super::TARGET_FACING_SHIELDS,
            cfg::TARGET_FACING_SHIELDS_FACT,
            "target_facing_shields",
        ),
        (
            super::ROUNDS_ABOARD,
            cfg::TORPEDO_ROUNDS_ABOARD_FACT,
            "rounds_aboard",
        ),
        (
            super::MISSION_THREAT_REMAINING,
            cfg::TORPEDO_MISSION_THREAT_FACT,
            "mission_threat_remaining",
        ),
        (
            super::ROUNDS_PER_THREAT,
            cfg::TORPEDO_ROUNDS_PER_THREAT_FACT,
            "rounds_per_threat",
        ),
        (
            super::TARGETED_OBJECTIVE_COUNT,
            cfg::TORPEDO_TARGETED_OBJECTIVE_COUNT_FACT,
            "targeted_objective_count",
        ),
    ];
    for (id, konst, literal) in pairs {
        assert_eq!(
            id.name(),
            *konst,
            "catalogue entry diverged from its config const"
        );
        assert_eq!(
            id.name(),
            *literal,
            "catalogue entry diverged from its expected string"
        );
    }
}

/// `referenced_facts` collects the three WORLD contexts and nothing else —
/// `memory(...)` and `state_time` are private, validated against their own
/// declarations, and must not be treated as host-seeded facts.
#[test]
fn referenced_facts_collects_the_world_contexts_only() {
    let pred = crate::world::flags::parse_predicate(
        "fact(a) > 0 and candidate_fact(b) > 0 and target_fact(c) > 0 and memory(d) > 0 \
             and state_time > 0 and flag(e) and counter(f) >= 1",
    )
    .unwrap();
    let mut refs = Vec::new();
    pred.referenced_facts(&mut refs);
    assert_eq!(
        refs,
        vec![
            (FactContext::SelfCtx, "a".to_string()),
            (FactContext::Candidate, "b".to_string()),
            (FactContext::Target, "c".to_string()),
        ]
    );
}

/// `check_facts` accepts a fact the host seeds and rejects a typo, naming the
/// host, the block, the atom, and the nearest real fact.
#[test]
fn check_facts_accepts_a_seeded_fact_and_suggests_the_nearest_on_a_typo() {
    let ok = crate::world::flags::parse_predicate("fact(secs_since_combat) < param(w)").unwrap();
    assert!(CAPTAIN_RED_ALERT.check_facts("rule 0", &ok).is_ok());

    let typo = crate::world::flags::parse_predicate("fact(secs_since_kombat) < param(w)").unwrap();
    let err = CAPTAIN_RED_ALERT.check_facts("rule 0", &typo).unwrap_err();
    assert!(
        err.contains("Captain") && err.contains("[captain_console.ai]"),
        "{err}"
    );
    assert!(
        err.contains("fact(secs_since_kombat)"),
        "must quote the atom: {err}"
    );
    assert!(
        err.contains("Did you mean `secs_since_combat`?"),
        "must suggest: {err}"
    );
}

/// The scope matters: a `candidate_fact(...)` is checked against the host's
/// CANDIDATE descriptors, so a candidate name read as a self `fact(...)` is
/// rejected, and vice versa.
#[test]
fn a_fact_is_checked_against_its_own_scope() {
    // `detectable` is a candidate fact on the Sensors selector, not a self one.
    let as_candidate =
        crate::world::flags::parse_predicate("candidate_fact(detectable) > 0").unwrap();
    assert!(SENSORS_SELECTOR
        .check_facts("eligibility", &as_candidate)
        .is_ok());

    let as_self = crate::world::flags::parse_predicate("fact(detectable) > 0").unwrap();
    assert!(
        SENSORS_SELECTOR
            .check_facts("eligibility", &as_self)
            .is_err(),
        "detectable is a candidate fact; a bare self fact(detectable) is not seeded"
    );
}

/// The two data-driven families match by prefix: any `power_<group>` and any
/// `recent_damage_<facing>` is accepted, but a mistyped stem is not.
#[test]
fn the_data_driven_families_match_by_prefix() {
    for name in [
        "fact(power_helm)",
        "fact(power_weapons)",
        "fact(power_shields)",
    ] {
        let pred = crate::world::flags::parse_predicate(&format!("{name} >= 3")).unwrap();
        assert!(
            POWER_ALLOCATION.check_facts("rule 0", &pred).is_ok(),
            "{name}"
        );
    }
    let bad = crate::world::flags::parse_predicate("fact(powr_helm) >= 3").unwrap();
    assert!(
        POWER_ALLOCATION.check_facts("rule 0", &bad).is_err(),
        "a mistyped family stem is still rejected"
    );

    let arc = crate::world::flags::parse_predicate("fact(recent_damage_fore) >= 1").unwrap();
    assert!(SHIELDS_FOCUS.check_facts("rule 0", &arc).is_ok());
}

/// An unseeded fact reaches the load path as a LOAD ERROR — the whole point:
/// a mistyped `fact(...)` on a real shipped hull no longer parses, validates,
/// and then reads false for ever, but fails the load naming the host.
#[test]
fn a_mistyped_fact_on_a_shipped_hull_fails_to_load() {
    // The unmutated hull loads, so the difference below is the typo alone.
    crate::entities::config::EntityConfig::from_toml(&cruiser())
        .expect("alliance_cruiser loads as shipped");

    let mutated = with_guard(
        &cruiser(),
        r#"when = "fact(secs_since_combat) < param(combat_window_secs)""#,
        r#"when = "fact(secs_since_kombat) < param(combat_window_secs)""#,
    );
    let err = crate::entities::config::EntityConfig::from_toml(&mutated)
        .expect_err("a mistyped fact() name must fail the load")
        .to_string();
    assert!(
        err.contains("Captain") && err.contains("[captain_console.ai]"),
        "the load error must name the system and its block: {err}"
    );
    assert!(
        err.contains("fact(secs_since_kombat)") && err.contains("never seeds a fact"),
        "the load error must quote the atom and say plainly why: {err}"
    );
}

/// The probe seeds exactly one fact, so an unseeded name on it is rejected —
/// the rejection surface is real, not a property only shipped hosts have.
#[test]
fn an_unplumbed_probe_rejects_an_unseeded_fact() {
    let pred = crate::world::flags::parse_predicate("candidate_fact(nonexistent) > 0").unwrap();
    let err = UNPLUMBED_PROBE
        .check_facts("eligibility", &pred)
        .unwrap_err();
    assert!(
        err.contains("Probe") && err.contains("candidate_fact(nonexistent)"),
        "{err}"
    );
}

//! Which fine-system AI hosts can actually evaluate `flag(...)` and
//! `counter(...)` guards — and the load-time rejection for the ones that
//! cannot (issue #891, stage 1).
//!
//! # The trap this closes
//!
//! `flag(name)` and `counter(name) CMP n` are full citizens of the shared
//! `world::flags` predicate grammar, and every fine-system policy/selector API
//! (`AiPolicy::resolve_channel`, `resolve_channel_in_state`,
//! `resolve_transition`, `TargetSelector::select`) takes a
//! `flags: &[&FlagStore]` chain. But a chain is only as real as what the HOST
//! passes: three of the nineteen hosts build one from
//! `WorldContentRuntime.flags`, and the other sixteen pass a literal `&[]`.
//!
//! On those sixteen a `flag(...)` guard parses, validates, and then reads
//! `false` for ever — the same silent-nothing failure mode as an unseeded
//! `fact(...)` name, except here the grammar advertises the feature as
//! available. Stage 1 (this module) converts that silent trap into a load
//! error. Stage 2 threads the real chain into every host and lifts the
//! rejection; when it does, the [`FlagChain`] on each host below flips to
//! [`FlagChain::Plumbed`] and the check stops firing for it — no rewrite of the
//! rejection itself is needed, one host at a time.
//!
//! No shipped hull authors a `flag()` or `counter()` guard on any AI policy or
//! selector today (the only `flag(`/`counter(` atoms under `assets/` are in
//! `assets/worlds/*.toml` world TRIGGERS, which evaluate through
//! `WorldContentRuntime` and are unaffected), so the rejection changes nothing
//! that runs.
//!
//! # Why the table is not simply hand-maintained
//!
//! A hardcoded "these hosts can, those cannot" list is drift-bait: the moment
//! stage 2 plumbs one host, the list is wrong and wrong SILENTLY — back to the
//! failure mode being fixed. So every host below records `eval_sites`: the
//! exact function(s) whose flag argument decides the answer. `flag_chain` is
//! declared, but it is not the source of truth — `tests::flag_chain_matches_the_hosts_source`
//! RE-DERIVES it by reading each of those functions out of the crate's own
//! source and inspecting the last argument of the resolve/select call. Change a
//! host's chain without updating its entry (or the other way round) and that
//! test fails naming the host. A second test walks every resolve/select call
//! site in the crate and fails on one that no host claims, so a NEW host cannot
//! be added without appearing here either.

use crate::world::flags::Predicate;

/// Whether a host's runtime evaluation call receives a populated world-flag
/// chain, and therefore whether `flag(...)`/`counter(...)` in one of its guards
/// can ever read true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagChain {
    /// The host builds its chain from `WorldContentRuntime.flags`.
    Plumbed,
    /// The host passes a literal `&[]`.
    Empty,
}

/// One runtime evaluation call site: the function whose resolve/select call
/// fixes a host's [`FlagChain`].
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
    /// The chain the host passes at runtime. Pinned against the real source by
    /// `tests::flag_chain_matches_the_hosts_source`.
    pub flag_chain: FlagChain,
    /// Every function whose resolve/select call fixes `flag_chain`. More than
    /// one when a host resolves several channels from several places; ALL of
    /// them must agree, so a stage-2 change that plumbs one and misses another
    /// fails the drift test rather than half-working.
    pub eval_sites: &'static [EvalSite],
}

// ── The nineteen hosts ───────────────────────────────────────────────────────
//
// Fourteen policy hosts, then five selector hosts. Ordering mirrors the
// validation blocks in `EntityConfig::from_toml`.

/// The three secondary helm axes collapse their channel to a bare "actuate this
/// tick?" through one shared helper.
const HELM_ACTUATOR_SITES: &[EvalSite] = &[site("src/ship/helm_ai.rs", "helm_policy_actuates")];
/// The three helm axes that may author a #882 state machine resolve their
/// channel on either policy path, and their TRANSITIONS through the shared
/// machine tick — three call sites, all of which must agree.
const HELM_MACHINE_SITES: &[EvalSite] = &[
    site("src/ship/helm_ai.rs", "resolve_helm_channel"),
    site("src/ship/helm_ai.rs", "tick_policy_machine"),
];

pub const CAPTAIN_RED_ALERT: AiHost = AiHost {
    system: "Captain",
    block: "[captain_console.ai]",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site("src/console/captain/server.rs", "operate_captain_ai")],
};

pub const HELM_ENGINES: AiHost = AiHost {
    system: "Helm engines",
    block: "[helm_console.engines_ai]",
    flag_chain: FlagChain::Empty,
    eval_sites: HELM_MACHINE_SITES,
};

pub const HELM_STEERING: AiHost = AiHost {
    system: "Helm steering",
    block: "[helm_console.steering_ai]",
    flag_chain: FlagChain::Empty,
    eval_sites: HELM_MACHINE_SITES,
};

pub const HELM_LATERAL: AiHost = AiHost {
    system: "Helm lateral thrust",
    block: "[helm_console.lateral_ai]",
    flag_chain: FlagChain::Empty,
    eval_sites: HELM_ACTUATOR_SITES,
};

pub const HELM_VERTICAL: AiHost = AiHost {
    system: "Helm vertical thrust",
    block: "[helm_console.vertical_ai]",
    flag_chain: FlagChain::Empty,
    eval_sites: HELM_ACTUATOR_SITES,
};

pub const HELM_IMPULSE: AiHost = AiHost {
    system: "Helm impulse",
    block: "[helm_console.impulse_ai]",
    flag_chain: FlagChain::Empty,
    eval_sites: HELM_ACTUATOR_SITES,
};

pub const HELM_BOOST: AiHost = AiHost {
    system: "Helm boost",
    block: "[helm_console.boost_ai]",
    flag_chain: FlagChain::Empty,
    eval_sites: HELM_MACHINE_SITES,
};

pub const PHASER_BANK: AiHost = AiHost {
    system: "Phaser bank",
    block: "[[weapons_console.phaser_banks]].ai",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site(
        "src/console/weapons/beam.rs",
        "phaser_bank_policy_fires",
    )],
};

pub const BLASTER_BANK: AiHost = AiHost {
    system: "Blaster bank",
    block: "[[weapons_console.blaster_banks]].ai",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site(
        "src/console/weapons/blaster.rs",
        "blaster_bank_policy_fires",
    )],
};

pub const TORPEDO_TUBE: AiHost = AiHost {
    system: "Torpedo tube",
    block: "[[torpedoes.tubes]].ai",
    flag_chain: FlagChain::Empty,
    eval_sites: &[
        site(
            "src/console/weapons/torpedo.rs",
            "torpedo_tube_load_policy_fires",
        ),
        site(
            "src/console/weapons/torpedo.rs",
            "torpedo_tube_launch_policy_fires",
        ),
    ],
};

pub const TORPEDO_MAGAZINE: AiHost = AiHost {
    system: "Torpedo magazine",
    block: "[torpedoes].ai",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site(
        "src/console/weapons/torpedo.rs",
        "torpedo_magazine_grant_policy_fires",
    )],
};

pub const SHIELDS_FOCUS: AiHost = AiHost {
    system: "Shields focus",
    block: "[shields_console.ai_policy]",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site("src/console_ai/server.rs", "ai_shield_focus")],
};

pub const POWER_ALLOCATION: AiHost = AiHost {
    system: "Power reactor",
    block: "[power.ai_policy]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/console_ai/server.rs", "ai_power_allocation")],
};

pub const COMMS_RESPONSE: AiHost = AiHost {
    system: "Comms dialogue response",
    block: "[comms_console.ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site(
        "src/console/comms/server.rs",
        "operate_comms_response_ai",
    )],
};

pub const SENSORS_SELECTOR: AiHost = AiHost {
    system: "Sensors target selector",
    block: "[sensors_console.selector]",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site("src/ship/sensors.rs", "operate_sensors_ai")],
};

pub const TACTICAL_SELECTOR: AiHost = AiHost {
    system: "Tactical target selector",
    block: "[weapons_console.selector]",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site("src/console/weapons/mod.rs", "ai_target_selection")],
};

pub const NAVIGATION_SELECTOR: AiHost = AiHost {
    system: "Navigation target selector",
    block: "[navigation_console.selector]",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site(
        "src/console/navigation/mod.rs",
        "operate_navigation_ai",
    )],
};

pub const REPAIR_SELECTOR: AiHost = AiHost {
    system: "Repair target selector",
    block: "[repair.selector]",
    flag_chain: FlagChain::Empty,
    eval_sites: &[site("src/console/repair/server.rs", "operate_repair_ai")],
};

pub const COMMS_SELECTOR: AiHost = AiHost {
    system: "Comms hail selector",
    block: "[comms_console.selector]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/console/comms/server.rs", "operate_comms_ai")],
};

/// Roll call. The drift tests iterate this, so a host added above and left out
/// here is caught by `every_eval_site_in_the_crate_belongs_to_a_host`.
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

impl AiHost {
    /// Reject a guard expression that reads world state on a host whose runtime
    /// evaluation gets no flag chain (issue #891 stage 1).
    ///
    /// `what` is the validator's own rule/transition/term label, so the message
    /// reads as one sentence with the rest of the content-error surface. The
    /// offending atom is quoted back verbatim: with `flag(...)` and
    /// `counter(...)` both rejected here, "which one did I write?" is the
    /// author's immediate next question.
    pub fn check_guard(&self, what: &str, pred: &Predicate) -> Result<(), String> {
        if self.flag_chain == FlagChain::Plumbed {
            return Ok(());
        }
        let mut refs = Vec::new();
        pred.referenced_world_state(&mut refs);
        let Some(atom) = refs.first() else {
            return Ok(());
        };
        let sites = self
            .eval_sites
            .iter()
            .map(|s| format!("{}::{}", s.file, s.func))
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "{what} references {atom}, but the {} system ({}) evaluates its AI \
             guards with NO world-flag chain plumbed — the chain is empty at \
             {sites} — so {atom} would read false for ever. Remove it, or plumb \
             the flag chain into that host first (issue #891 stage 2)",
            self.system, self.block
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// Method-call forms of the four evaluation entry points. A host's flag
    /// chain is the last argument of whichever of these it calls.
    const EVAL_CALLS: &[&str] = &[
        ".resolve_channel(",
        ".resolve_channel_in_state(",
        ".resolve_transition(",
        ".select(",
    ];

    /// Files that call an evaluation entry point but host no authored content.
    ///
    /// `default_ai_policy_pins` is declared `#[cfg(test)] mod` in
    /// `src/entities/mod.rs` (so its whole body is test code, with no in-file
    /// `#[cfg(test)] mod tests` marker for `strip_test_module` to find), and it
    /// drives the synthesised defaults directly rather than hosting them.
    const NON_HOST_FILES: &[&str] = &["src/entities/default_ai_policy_pins.rs"];

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
            .unwrap_or_else(|e| panic!("eval-site file {rel} must be readable: {e}"));
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

    /// The last top-level argument of the call starting at `open` (the index of
    /// its `(`), with all whitespace squeezed out so a one-line call and a
    /// rustfmt-exploded one compare equal.
    ///
    /// rustfmt writes a TRAILING comma on the exploded form, so the last
    /// top-level comma is not necessarily the one before the last argument —
    /// separators are collected and the last non-empty span wins.
    fn last_argument(src: &str, open: usize) -> String {
        let mut depth = 0usize;
        let mut separators = vec![open];
        let mut end = None;
        for (offset, ch) in src[open..].char_indices() {
            match ch {
                '(' | '[' => depth += 1,
                ')' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + offset);
                        break;
                    }
                }
                ',' if depth == 1 => separators.push(open + offset),
                _ => {}
            }
        }
        let mut end = end.expect("evaluation call must have balanced parentheses");
        while let Some(sep) = separators.pop() {
            let arg: String = src[sep + 1..end].split_whitespace().collect();
            if !arg.is_empty() {
                return arg;
            }
            end = sep;
        }
        String::new()
    }

    /// Every evaluation call in `body`, as the flag chain each one passes.
    fn chains_in(body: &str) -> Vec<FlagChain> {
        let mut out = Vec::new();
        for call in EVAL_CALLS {
            let mut from = 0usize;
            while let Some(hit) = body[from..].find(call) {
                let open = from + hit + call.len() - 1;
                out.push(match last_argument(body, open).as_str() {
                    "&[]" => FlagChain::Empty,
                    _ => FlagChain::Plumbed,
                });
                from = open + 1;
            }
        }
        out
    }

    /// AC: the declared classification is not hand-maintained trivia — it is
    /// re-derived here from the hosts' own source, so plumbing a host in stage 2
    /// and forgetting this table fails the build.
    #[test]
    fn flag_chain_matches_the_hosts_source() {
        for host in AI_HOSTS {
            for site in host.eval_sites {
                let src = read_non_test_source(site.file);
                let body = function_body(&src, site.func);
                let chains = chains_in(body);
                assert!(
                    !chains.is_empty(),
                    "{} ({}): {}::{} declares itself this host's evaluation site \
                     but calls none of {EVAL_CALLS:?}. Point the entry at the \
                     function that actually resolves the policy/selector",
                    host.system,
                    host.block,
                    site.file,
                    site.func
                );
                for actual in chains {
                    assert_eq!(
                        actual, host.flag_chain,
                        "{} ({}) declares flag_chain = {:?}, but {}::{} passes a \
                         {:?} chain. If stage 2 just plumbed this host, flip the \
                         entry to FlagChain::Plumbed — the load-time rejection \
                         then stops firing for it. If it just LOST its chain, \
                         authored flag()/counter() guards on it have gone silent.",
                        host.system, host.block, host.flag_chain, site.file, site.func, actual
                    );
                }
            }
        }
    }

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("src/ must be readable") {
            let path = entry.expect("readable dir entry").path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// AC: a NEW host cannot appear without appearing in the table either.
    ///
    /// Walks every non-test evaluation call in the crate and requires the
    /// enclosing function to be claimed by some host. Without this, the drift
    /// test above would happily pass over a sixteenth silent host.
    #[test]
    fn every_eval_site_in_the_crate_belongs_to_a_host() {
        let root = crate_root();
        let claimed: BTreeSet<(&str, &str)> = AI_HOSTS
            .iter()
            .flat_map(|h| h.eval_sites.iter().map(|s| (s.file, s.func)))
            .collect();

        let mut files = Vec::new();
        rust_files(&root.join("src"), &mut files);
        let mut unclaimed: Vec<String> = Vec::new();
        for path in files {
            let rel = path
                .strip_prefix(&root)
                .expect("scanned file is under the crate root")
                .to_string_lossy()
                .replace('\\', "/");
            if NON_HOST_FILES.contains(&rel.as_str()) {
                continue;
            }
            let src = strip_test_module(&std::fs::read_to_string(&path).expect("readable source"));
            for call in EVAL_CALLS {
                let mut from = 0usize;
                while let Some(hit) = src[from..].find(call) {
                    let at = from + hit;
                    from = at + call.len();
                    // The enclosing `fn` is the last one defined before the call.
                    let Some(fn_at) = src[..at].rfind("fn ") else {
                        continue;
                    };
                    let name: String = src[fn_at + 3..]
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !claimed.contains(&(rel.as_str(), name.as_str())) {
                        unclaimed.push(format!("{rel}::{name}"));
                    }
                }
            }
        }
        unclaimed.sort();
        unclaimed.dedup();
        assert!(
            unclaimed.is_empty(),
            "these functions evaluate an AI policy/selector but no AiHost claims \
             them, so nothing pins whether flag()/counter() guards reaching them \
             can ever read true: {unclaimed:?}. Add the host (or its extra eval \
             site) to AI_HOSTS."
        );
    }

    /// The three hosts that CAN read world flags today, pinned by name. Reading
    /// this test tells a stage-2 author exactly how far the plumbing has got.
    #[test]
    fn exactly_three_hosts_can_read_world_flags_today() {
        let plumbed: Vec<&str> = AI_HOSTS
            .iter()
            .filter(|h| h.flag_chain == FlagChain::Plumbed)
            .map(|h| h.system)
            .collect();
        assert_eq!(
            plumbed,
            vec![
                "Power reactor",
                "Comms dialogue response",
                "Comms hail selector"
            ],
            "the set of flag-capable hosts changed. Stage 2 widening it is the \
             POINT — update this list. Anything else means a host silently lost \
             its chain."
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

    #[test]
    fn unplumbed_host_rejects_flag_and_counter_and_nothing_else() {
        let flag = crate::world::flags::parse_predicate("flag(aphelion_armed)").unwrap();
        let err = CAPTAIN_RED_ALERT.check_guard("rule 0", &flag).unwrap_err();
        assert!(
            err.contains("Captain"),
            "message must name the system: {err}"
        );
        assert!(
            err.contains("flag(aphelion_armed)"),
            "message must quote the atom: {err}"
        );
        assert!(
            err.contains("NO world-flag chain plumbed"),
            "message must say the chain is not plumbed: {err}"
        );

        let counter =
            crate::world::flags::parse_predicate("counter(evacuation_rounds) >= 3").unwrap();
        let err = CAPTAIN_RED_ALERT
            .check_guard("rule 0", &counter)
            .unwrap_err();
        assert!(err.contains("counter(evacuation_rounds)"), "{err}");

        // Nested under `not`/`and` is still a reference.
        let nested = crate::world::flags::parse_predicate("fact(a) > 0 and not flag(b)").unwrap();
        assert!(CAPTAIN_RED_ALERT.check_guard("rule 0", &nested).is_err());

        // Facts, params, memory and literals are untouched by this check.
        let facts =
            crate::world::flags::parse_predicate("fact(a) > param(b) and memory(c) < 1").unwrap();
        assert!(CAPTAIN_RED_ALERT.check_guard("rule 0", &facts).is_ok());
    }

    #[test]
    fn plumbed_hosts_accept_flag_and_counter() {
        let flag = crate::world::flags::parse_predicate(
            "flag(containment_started) and counter(evacuation_rounds) >= 1",
        )
        .unwrap();
        for host in [POWER_ALLOCATION, COMMS_RESPONSE, COMMS_SELECTOR] {
            assert!(
                host.check_guard("rule 0", &flag).is_ok(),
                "{} passes a real flag chain, so its guards must be accepted",
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
            &CAPTAIN_RED_ALERT,
            &stateless,
            RED_ALERT,
            SET_RED_ALERT,
        )
        .expect_err("a top-level rule guard must be walked");
        assert!(err.contains("rule 0") && err.contains("Captain"), "{err}");

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
            &CAPTAIN_RED_ALERT,
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
            &CAPTAIN_RED_ALERT,
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
            &SENSORS_SELECTOR,
            &eligibility,
            sources,
        )
        .expect_err("the eligibility expression must be walked");
        assert!(
            err.contains("Sensors target selector") && err.contains("flag(sensors_online)"),
            "{err}"
        );

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
            &SENSORS_SELECTOR,
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

    const CRUISER: &str = include_str!("../../assets/entities/alliance_cruiser.toml");

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

    /// AC: a `flag()` guard authored on an unplumbed host of a REAL hull fails
    /// the entity load, naming the system.
    #[test]
    fn a_flag_guard_on_a_shipped_hull_fails_to_load() {
        // Sanity: the unmutated hull loads, so the failure below is the guard.
        crate::entities::config::EntityConfig::from_toml(CRUISER)
            .expect("alliance_cruiser loads as shipped");

        let mutated = with_guard(
            CRUISER,
            r#"when = "fact(secs_since_combat) < param(combat_window_secs)""#,
            r#"when = "fact(secs_since_combat) < param(combat_window_secs) and flag(battle_stations)""#,
        );
        let err = crate::entities::config::EntityConfig::from_toml(&mutated)
            .expect_err("a flag() guard on the Captain host must fail the load")
            .to_string();
        assert!(
            err.contains("Captain") && err.contains("[captain_console.ai]"),
            "the load error must name the system and its block: {err}"
        );
        assert!(
            err.contains("flag(battle_stations)") && err.contains("NO world-flag chain plumbed"),
            "the load error must say plainly why: {err}"
        );
    }

    /// AC: the hosts that CAN evaluate flags still accept them — same hull, same
    /// load path, a flag added to its `[power.ai_policy]` instead.
    #[test]
    fn a_flag_guard_on_a_plumbed_host_of_a_shipped_hull_still_loads() {
        let mutated = with_guard(
            CRUISER,
            r#"when = "fact(red_alert) > 0 and fact(battery_pct) >= param(min_reserve_weapons)""#,
            r#"when = "fact(red_alert) > 0 and fact(battery_pct) >= param(min_reserve_weapons) and flag(weapons_free)""#,
        );
        crate::entities::config::EntityConfig::from_toml(&mutated).expect(
            "the Power reactor passes a real flag chain, so a flag() guard on it is valid content",
        );
    }

    /// The premise that makes stage 1 zero-risk: nothing shipped authors one, so
    /// every hull must still load. Rechecked here rather than assumed, because
    /// the rejection is only free while it stays true.
    #[test]
    fn every_shipped_entity_still_loads() {
        let dir = crate_root().join("assets/entities");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("assets/entities must be readable") {
            let path = entry.expect("readable dir entry").path();
            if path.extension().is_some_and(|e| e == "toml") {
                let src = std::fs::read_to_string(&path).expect("readable entity toml");
                crate::entities::config::EntityConfig::from_toml(&src).unwrap_or_else(|e| {
                    panic!("{} must still load: {e}", path.display());
                });
                checked += 1;
            }
        }
        assert!(checked > 0, "no entity TOMLs were checked");
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
        let src = read_non_test_source("src/entities/config.rs");
        for hostless in [
            "validate_fine_system_ai_policy(",
            "validate_fine_system_ai_selector(",
        ] {
            let mut from = 0usize;
            let mut definitions = 0;
            while let Some(hit) = src[from..].find(hostless) {
                let at = from + hit;
                from = at + hostless.len();
                let is_definition = src[..at].ends_with("fn ");
                assert!(
                    is_definition,
                    "src/entities/config.rs calls the host-less `{hostless}` outside a \
                     definition. Production validation must use the `_for` variant with \
                     the owning ai_flag_hosts::AiHost, or a flag()/counter() guard on \
                     that block goes back to reading false in silence."
                );
                definitions += 1;
            }
            // Non-vacuity: the definition itself must have been seen, or the
            // scan found nothing and this test proves nothing.
            assert_eq!(
                definitions, 1,
                "expected to find exactly the definition of `{hostless}` in the \
                 non-test source; the scan is looking in the wrong place"
            );
        }
    }
}

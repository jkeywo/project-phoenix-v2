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
//! (`AiPolicy::resolve_channel`, `resolve_channel_in_state`,
//! `resolve_transition`, `TargetSelector::select`) takes a
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
//! # Why the table is not simply hand-maintained
//!
//! A hardcoded "these hosts can, those cannot" list is drift-bait: the moment
//! a host gains or loses its chain, the list is wrong and wrong SILENTLY —
//! back to the failure mode being fixed. So every host below records `eval_sites`: the
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
    Some(site("src/ship/helm_ai.rs", "tick_policy_machine"));

pub const CAPTAIN_RED_ALERT: AiHost = AiHost {
    system: "Captain",
    block: "[captain_console.ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/console/captain/server.rs", "operate_captain_ai")],
    history_fold: None,
};

pub const HELM_ENGINES: AiHost = AiHost {
    system: "Helm engines",
    block: "[helm_console.engines_ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: HELM_MACHINE_SITES,
    history_fold: HELM_HISTORY_FOLD,
};

pub const HELM_STEERING: AiHost = AiHost {
    system: "Helm steering",
    block: "[helm_console.steering_ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: HELM_MACHINE_SITES,
    history_fold: HELM_HISTORY_FOLD,
};

pub const HELM_LATERAL: AiHost = AiHost {
    system: "Helm lateral thrust",
    block: "[helm_console.lateral_ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: HELM_ACTUATOR_SITES,
    history_fold: None,
};

pub const HELM_VERTICAL: AiHost = AiHost {
    system: "Helm vertical thrust",
    block: "[helm_console.vertical_ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: HELM_ACTUATOR_SITES,
    history_fold: None,
};

pub const HELM_IMPULSE: AiHost = AiHost {
    system: "Helm impulse",
    block: "[helm_console.impulse_ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: HELM_ACTUATOR_SITES,
    history_fold: None,
};

pub const HELM_BOOST: AiHost = AiHost {
    system: "Helm boost",
    block: "[helm_console.boost_ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: HELM_MACHINE_SITES,
    history_fold: HELM_HISTORY_FOLD,
};

pub const PHASER_BANK: AiHost = AiHost {
    system: "Phaser bank",
    block: "[[weapons_console.phaser_banks]].ai",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site(
        "src/console/weapons/beam.rs",
        "phaser_bank_policy_fires",
    )],
    history_fold: None,
};

pub const BLASTER_BANK: AiHost = AiHost {
    system: "Blaster bank",
    block: "[[weapons_console.blaster_banks]].ai",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site(
        "src/console/weapons/blaster.rs",
        "blaster_bank_policy_fires",
    )],
    history_fold: None,
};

pub const TORPEDO_TUBE: AiHost = AiHost {
    system: "Torpedo tube",
    block: "[[torpedoes.tubes]].ai",
    flag_chain: FlagChain::Plumbed,
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
    history_fold: None,
};

pub const TORPEDO_MAGAZINE: AiHost = AiHost {
    system: "Torpedo magazine",
    block: "[torpedoes].ai",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site(
        "src/console/weapons/torpedo.rs",
        "torpedo_magazine_grant_policy_fires",
    )],
    history_fold: None,
};

pub const SHIELDS_FOCUS: AiHost = AiHost {
    system: "Shields focus",
    block: "[shields_console.ai_policy]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/console_ai/server.rs", "ai_shield_focus")],
    history_fold: None,
};

pub const POWER_ALLOCATION: AiHost = AiHost {
    system: "Power reactor",
    block: "[power.ai_policy]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/console_ai/server.rs", "ai_power_allocation")],
    history_fold: None,
};

pub const COMMS_RESPONSE: AiHost = AiHost {
    system: "Comms dialogue response",
    block: "[comms_console.ai]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site(
        "src/console/comms/server.rs",
        "operate_comms_response_ai",
    )],
    history_fold: None,
};

pub const SENSORS_SELECTOR: AiHost = AiHost {
    system: "Sensors target selector",
    block: "[sensors_console.selector]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/ship/sensors.rs", "operate_sensors_ai")],
    history_fold: None,
};

pub const TACTICAL_SELECTOR: AiHost = AiHost {
    system: "Tactical target selector",
    block: "[weapons_console.selector]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/console/weapons/mod.rs", "ai_target_selection")],
    history_fold: None,
};

pub const NAVIGATION_SELECTOR: AiHost = AiHost {
    system: "Navigation target selector",
    block: "[navigation_console.selector]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site(
        "src/console/navigation/mod.rs",
        "operate_navigation_ai",
    )],
    history_fold: None,
};

pub const REPAIR_SELECTOR: AiHost = AiHost {
    system: "Repair target selector",
    block: "[repair.selector]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/console/repair/server.rs", "operate_repair_ai")],
    history_fold: None,
};

pub const COMMS_SELECTOR: AiHost = AiHost {
    system: "Comms hail selector",
    block: "[comms_console.selector]",
    flag_chain: FlagChain::Plumbed,
    eval_sites: &[site("src/console/comms/server.rs", "operate_comms_ai")],
    history_fold: None,
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
    /// `authored_ai_pins` is declared `#[cfg(test)] mod` in
    /// `src/entities/mod.rs` (so its whole body is test code, with no in-file
    /// `#[cfg(test)] mod tests` marker for `strip_test_module` to find), and it
    /// drives the shipped authored blocks directly rather than hosting them. It
    /// replaced `default_ai_policy_pins`, which sat here for the same reason,
    /// when #885b stage 5d deleted the synthesisers that suite pinned.
    const NON_HOST_FILES: &[&str] = &["src/entities/authored_ai_pins.rs"];

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

    /// The parameter-list text of `fn <name>` — from the signature's opening
    /// `(` through the matching `)`, by paren counting (the same shape as
    /// [`function_body`], one token earlier; a `Query<(A, B), C>` tuple's own
    /// parens balance correctly the same way nested braces do there). Lets
    /// [`chains_in`] check which resources a chain-building host actually
    /// DECLARES, independent of what its eval call PASSES — see `chains_in`
    /// for the drift this closes (issue #891 review finding 3).
    fn fn_signature<'a>(src: &'a str, func: &str) -> &'a str {
        let needle = format!("fn {func}");
        let start = src
            .find(&needle)
            .unwrap_or_else(|| panic!("no `{needle}` in the scanned source"));
        let open = start
            + src[start..]
                .find('(')
                .unwrap_or_else(|| panic!("`{needle}` has no parameter list"));
        let mut depth = 0usize;
        for (offset, ch) in src[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return &src[open..open + offset + 1];
                    }
                }
                _ => {}
            }
        }
        panic!("`{needle}` parameter list is unbalanced");
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
    ///
    /// A non-`&[]` last argument used to be sufficient to call a site
    /// [`FlagChain::Plumbed`] (issue #891 review finding 3: the drift guard
    /// this closes). That text-only rule has a hole: if the eval site BUILDS
    /// its own chain — its body calls `entity_flag_chain` directly, the shape
    /// every direct-system host uses (Captain, Shields, Power, the two Comms
    /// hosts, Sensors, Tactical, Navigation, Repair) — a non-`&[]` argument
    /// only means the source TEXT looks right. The site could still be a host
    /// that dropped its `Option<Res<WorldContentRuntime>>` /
    /// `Option<Res<WorldLayerMap>>` params and now calls
    /// `entity_flag_chain(None, None, None)`, which reads empty for ever at
    /// runtime while remaining textually non-empty. So a chain-BUILDING site
    /// is only Plumbed when `sig` — its own signature, from [`fn_signature`]
    /// — still declares both resources.
    ///
    /// Sites that instead RECEIVE an already-built chain as a `flags: &[&
    /// FlagStore]` parameter (the three helm machine axes' `resolve_helm_channel`
    /// / `tick_policy_machine`, the three helm actuator axes' `helm_policy_actuates`,
    /// and the per-bank/tube/magazine `*_policy_fires` helpers) have no
    /// resources of their own to declare — building the chain is the
    /// CALLER's job, not theirs — so those keep the original text-only rule.
    fn chains_in(body: &str, sig: &str) -> Vec<FlagChain> {
        let builds_own_chain = body.contains("entity_flag_chain(");
        let declares_chain_resources = (sig
            .contains("Option<Res<crate::world::server::WorldContentRuntime>>")
            || sig.contains("Option<Res<WorldContentRuntime>>"))
            && (sig.contains("Option<Res<crate::world::server::WorldLayerMap>>")
                || sig.contains("Option<Res<WorldLayerMap>>"));
        let can_be_plumbed = !builds_own_chain || declares_chain_resources;

        let mut out = Vec::new();
        for call in EVAL_CALLS {
            let mut from = 0usize;
            while let Some(hit) = body[from..].find(call) {
                let open = from + hit + call.len() - 1;
                let looks_non_empty = last_argument(body, open).as_str() != "&[]";
                out.push(if looks_non_empty && can_be_plumbed {
                    FlagChain::Plumbed
                } else {
                    FlagChain::Empty
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
                let sig = fn_signature(&src, site.func);
                let chains = chains_in(body, sig);
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
            crate::world::flags::parse_predicate("history(min, range_to_target, 30) >= 40")
                .unwrap();
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
            crate::world::flags::parse_predicate("fact(a) > 0 and not history(max, b, 4) < 1")
                .unwrap();
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
    /// are resolved by `helm_policy_actuates`, which has no per-system bag to
    /// fold into, so a window authored there would read absent for ever.
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
    /// edit here while a host silently losing its chain still fails (both here
    /// and in `flag_chain_matches_the_hosts_source`, which re-derives the
    /// classification from the host's own source).
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
        eval_sites: &[site("src/nowhere.rs", "probe_resolve")],
        history_fold: None,
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

        let counter =
            crate::world::flags::parse_predicate("counter(evacuation_rounds) >= 3").unwrap();
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
        crate::entity_includes::resolve_from_disk("assets/entities/alliance_cruiser.toml")
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
    #[test]
    fn a_flag_guard_on_a_plumbed_host_of_a_shipped_hull_still_loads() {
        // The reactor's one red-alert elevation, which since issue #923 is the
        // SENSORS one: the fleet buys nominal weapon REACH with the point the
        // weapons group used to spend on damage.
        let mutated = with_guard(
            &cruiser(),
            r#"when = "fact(red_alert) > 0 and fact(battery_pct) >= param(min_reserve_sensors)""#,
            r#"when = "fact(red_alert) > 0 and fact(battery_pct) >= param(min_reserve_sensors) and flag(weapons_free)""#,
        );
        crate::entities::config::EntityConfig::from_toml(&mutated).expect(
            "the Power reactor passes a real flag chain, so a flag() guard on it is valid content",
        );
    }

    // ── History, through the real entity-load path (issue #890) ─────────────

    const DESTROYER: &str = include_str!("../../assets/entities/ship_harrow_destroyer.toml");

    /// The destroyer's own re-entry gate, which #788 could only express as a
    /// bespoke host-folded fact. The window is authored on the same param the
    /// bespoke plumbing already reads.
    const AUTHORED_WINDOW: &str =
        "history(min, range_to_target, param(safe_distance_window_ticks)) \
                                   >= param(safe_range_margin)";

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
        crate::entities::config::EntityConfig::from_toml(DESTROYER)
            .expect("ship_harrow_destroyer loads as shipped");

        // A transition guard (the position #788's bespoke fact was confined to),
        // on all three of the hull's machine axes. A LITERAL window here because
        // only the Steering axis declares the standoff length as a param — the
        // authored-param spelling is exercised on the rule below.
        let transition = with_guard_everywhere(
            DESTROYER,
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
            DESTROYER,
            "when = \"true\"\n  verb = \"hold_recovery_orbit\"",
            &format!("when = \"{AUTHORED_WINDOW}\"\n  verb = \"hold_recovery_orbit\""),
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
            DESTROYER,
            "when = \"true\"\n  verb = \"hold_recovery_orbit\"",
            "when = \"history(net_change, range_to_target, param(pressed_window_ticks)) > \
             param(pressed_min_progress)\"\n  verb = \"hold_recovery_orbit\"",
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
            DESTROYER,
            "when = \"true\"\n  verb = \"hold_recovery_orbit\"",
            &format!("when = \"{AUTHORED_WINDOW}\"\n  verb = \"hold_recovery_orbit\""),
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

    /// The cruiser authors the gate on two phaser banks and three torpedo
    /// tubes. Stated as a number so a refit that adds or removes a weapon
    /// fails here rather than silently leaving one of them unmutated — which is
    /// how the mutation below would turn into a vacuous pass.
    const CRUISER_GATED_WEAPONS: usize = 5;

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

        let ungated = with_guard_everywhere(
            &cruiser,
            FIRE_GATE,
            r#"when = "true""#,
            CRUISER_GATED_WEAPONS,
        );
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
                // Through the include resolver (issue #906): a raw read would
                // assert on unresolved text the day a hull declares `includes`.
                let key = path.to_string_lossy().replace('\\', "/");
                crate::entity_includes::load_entity_config(&key).unwrap_or_else(|e| {
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

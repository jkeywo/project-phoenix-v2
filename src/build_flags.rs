//! Compile-time build flags the running page can read back (issue #939).
//!
//! `PHOENIX_DEMO_BUILD` marks a build destined for the public demo deploy
//! (`deploy-demo.yml`). It is deliberately a DIFFERENT signal from
//! `TRUNK_BUILD_RELEASE`, which is the size-optimisation switch read by
//! `scripts/wasm-opt-fixup.mjs` (see `Trunk.toml`) and is set by BOTH `ci.yml`
//! and `deploy-demo.yml`. Gating on the latter would strip the Debug/Cheat tab
//! from the GitHub Pages build too — and that build is the dev host, which has
//! to keep its debug tooling while staying size-optimised.
//!
//! `option_env!` bakes the value in at compile time. rustc records the
//! variable in the crate's dep-info, so Cargo rebuilds when it changes.
//!
//! Pure and Bevy-free; the `#[wasm_bindgen]` getter that exposes this to JS
//! lives in `crate::server::bridge` (`wasm_is_demo_build`).

// `DEMO_VALUE`, the exact value the demo deploy sets (`deploy-demo.yml`).
// `include!`d rather than declared here because `build.rs` needs the same
// literal and cannot `use` this crate; one source means the two halves of the
// gate cannot compare against different strings.
include!("demo_build_value.rs");

/// True when this binary was compiled with `PHOENIX_DEMO_BUILD=true`.
///
/// Deliberately NOT `cfg!(debug_assertions)` and NOT `TRUNK_BUILD_RELEASE`:
/// `trunk build --release` is used for local size experiments and for the dev
/// host's own deploy, and the flag we gate the Debug/Cheat tab on has to be
/// the one only the demo pipeline sets.
pub fn is_demo_build() -> bool {
    demo_flag_from_env(option_env!("PHOENIX_DEMO_BUILD"))
}

/// Pure decision the compile-time lookup feeds, split out so it is testable on
/// native without recompiling under a different environment.
pub fn demo_flag_from_env(value: Option<&str>) -> bool {
    value == Some(DEMO_VALUE)
}

/// The same answer as [`is_demo_build`], expressed as the `cfg` that `build.rs`
/// derives from the identical environment variable (issue #940).
///
/// `option_env!` can only be read at runtime; `#[cfg]` is what actually removes
/// code from the binary. The phone client's debug/cheat route needs the second
/// kind of gate — a demo build must not merely refuse the route, it must not
/// contain it — so `command_admission::debug_route` is `#[cfg]`-split on
/// `phoenix_demo_build`. This function exists so the test below can assert the
/// two gates never disagree.
pub const fn is_demo_cfg() -> bool {
    cfg!(phoenix_demo_build)
}

/// Does this build offer the host mod-pack upload? (PRD #855.)
///
/// The public build ships a curated catalogue — combat_test with the Alliance
/// Destroyer and Alliance Cruiser, per `assets/scenarios.demo.toml`. A mod-pack
/// upload ADDS scenarios and hulls to that catalogue at runtime, so it is the one
/// control that undoes the restriction, and a demo binary contains no
/// `wasm_add_mod_pack` to reach: that export carries
/// `#[cfg(not(phoenix_demo_build))]`, and `gui/build-flags.js`'s
/// `offersModPackUpload` removes the button that would call it.
///
/// Stated here, as a predicate over the same cfg, so the rule is asserted by a
/// test that runs in BOTH builds — `ci.yml`'s demo-build gate step already
/// filters on `build_flags`, so this needs no new CI step to be exercised with
/// the flag actually set.
pub const fn accepts_mod_pack_uploads() -> bool {
    !cfg!(phoenix_demo_build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_is_a_dev_build() {
        assert!(!demo_flag_from_env(None));
    }

    #[test]
    fn only_the_exact_ci_value_counts_as_a_demo_build() {
        assert!(demo_flag_from_env(Some("true")));
        // Anything else — including the shapes a hand-typed export produces —
        // stays a dev build rather than silently hiding the debug menu.
        assert!(!demo_flag_from_env(Some("TRUE")));
        assert!(!demo_flag_from_env(Some("1")));
        assert!(!demo_flag_from_env(Some("")));
        assert!(!demo_flag_from_env(Some("false")));
    }

    /// The dev host (`ci.yml`) sets `TRUNK_BUILD_RELEASE=true` and nothing
    /// else, and must keep its Debug/Cheat tab. Nothing in this module may
    /// read that variable — this test is the reminder, since the two flags
    /// travelled together before the split.
    #[test]
    fn the_size_optimisation_flag_is_not_the_demo_flag() {
        assert!(
            !is_demo_build() || option_env!("PHOENIX_DEMO_BUILD") == Some("true"),
            "is_demo_build() must answer to PHOENIX_DEMO_BUILD alone"
        );
    }

    /// The runtime answer (`option_env!`, read back by JS through
    /// `wasm_is_demo_build()`) and the compile-time answer (`build.rs`'s cfg,
    /// which removes the client debug route) come from the same environment
    /// variable and must never disagree. If they did, a demo build could ship a
    /// menu whose gate was compiled out — or, worse, a route whose UI was
    /// hidden but which still admitted commands.
    ///
    /// What this can and cannot catch, honestly: with `PHOENIX_DEMO_BUILD`
    /// unset both sides answer `false` regardless of the literals they compare
    /// against, so an unset run only pins the *sense* of the gate. The other
    /// half — that the two sides compare against the same string — is now a
    /// structural fact rather than an assertion, because `DEMO_VALUE` is one
    /// `include!`d literal (`src/demo_build_value.rs`). And the demo half of
    /// this assertion is really exercised because `ci.yml`'s "demo-build gate
    /// tests" step re-runs it with the variable set; before that step existed,
    /// no job in the repo ever compiled a `#[cfg(phoenix_demo_build)]` body.
    #[test]
    fn the_cfg_gate_and_the_runtime_flag_agree() {
        assert_eq!(
            is_demo_cfg(),
            is_demo_build(),
            "build.rs's phoenix_demo_build cfg must track PHOENIX_DEMO_BUILD \
             exactly, or the compiled-out route and the hidden tab disagree"
        );
    }

    /// The catalogue restriction and the mod-pack upload are the same decision
    /// (PRD #855): a build that curates its public catalogue must not also ship
    /// the control that adds arbitrary scenarios and hulls to it.
    ///
    /// Like the assertion above, this is exercised with the flag genuinely set
    /// by `ci.yml`'s demo-build gate step, which already filters on
    /// `build_flags` — so the demo arm is compiled and run, not merely written.
    #[test]
    fn a_demo_build_that_curates_its_catalogue_offers_no_mod_pack_upload() {
        assert_eq!(
            accepts_mod_pack_uploads(),
            !is_demo_build(),
            "a demo build curates its catalogue; offering a mod-pack upload \
             would hand any player at the host page the lever that undoes it"
        );
    }
}

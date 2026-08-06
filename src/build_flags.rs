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

/// The exact value the demo deploy sets (`deploy-demo.yml`).
const DEMO_VALUE: &str = "true";

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
}

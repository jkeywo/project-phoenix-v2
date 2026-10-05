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
include!("../../../src/demo_build_value.rs");

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
#[path = "build_flags_tests.rs"]
mod tests;

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
/// `include!`d literal (`crates/phoenix-simulation/src/demo_build_value.rs`). And the demo half of
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

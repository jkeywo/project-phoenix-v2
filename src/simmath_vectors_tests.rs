use super::*;

/// The pinned cross-target digest (issue #909). This is what
/// `tests/smoke/simmath-vectors.spec.js` asserts the wasm build produces
/// too — see [`wasm_simmath_battery`]. If this fails after a deliberate
/// change (a `libm` upgrade, a widened battery), re-derive the new
/// expected value from this same function on native, update it here,
/// then verify the wasm side agrees — never re-bless just to make the
/// test pass without checking wasm.
const EXPECTED_DIGEST: u64 = 0xbbff_9333_2c3b_937e;
const EXPECTED_CASE_COUNT: usize = 1300;

#[test]
fn battery_is_deterministic_within_a_run() {
    let a = run_battery();
    let b = run_battery();
    assert_eq!(a, b, "the battery is not allowed to depend on run order");
}

/// Every tag the digest is supposed to fold, in first-seen order: one per
/// wrapped `simmath` function (with `sin_cos` split into its two
/// components) plus one per dependency probe. Pinned as a list rather
/// than a count so a renamed, reordered, or silently-emptied domain
/// builder fails here with the tag named, instead of hiding inside a
/// total that is still comfortably above some floor.
const EXPECTED_TAGS: &[&str] = &[
    "sin",
    "cos",
    "sin_cos.sin",
    "sin_cos.cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "powf",
    "exp",
    "ln",
    "nalgebra.sin",
    "nalgebra.cos",
    "nalgebra.powf",
    "nalgebra.atan2",
    "glam.vec2_to_angle",
    "glam.quat_from_rotation_y",
    "glam.vec3_angle_between",
];

#[test]
fn battery_covers_every_wrapped_function() {
    let run = run_battery_detailed();
    let observed: Vec<&str> = run.cases_per_tag.iter().map(|(t, _)| *t).collect();
    assert_eq!(
        observed, EXPECTED_TAGS,
        "the battery's tag set changed — a function or dependency probe \
             was added, renamed, reordered, or lost its entire domain"
    );
    for (tag, count) in &run.cases_per_tag {
        assert!(
            *count > 0,
            "tag `{tag}` contributed no cases — its domain builder \
                 regressed to empty"
        );
    }
    assert_eq!(
        run.result.case_count,
        run.cases_per_tag.iter().map(|(_, n)| n).sum::<usize>(),
        "case_count and the per-tag breakdown disagree"
    );
}

/// The tripwire: this is the actual cross-target proof for issue #909
/// AC-3. `tests/smoke/simmath-vectors.spec.js` re-asserts the same two
/// constants against the wasm build.
#[test]
fn native_battery_matches_the_pinned_cross_target_digest() {
    let result = run_battery();
    assert_eq!(
        result.case_count, EXPECTED_CASE_COUNT,
        "case count drifted — the battery shape changed; if intentional, \
             re-derive EXPECTED_DIGEST and EXPECTED_CASE_COUNT together and \
             update tests/smoke/simmath-vectors.spec.js to match"
    );
    assert_eq!(
        result.digest, EXPECTED_DIGEST,
        "digest {:016x} != pinned {EXPECTED_DIGEST:016x} — a simmath \
             function's output changed on native. If this is wasm too (see \
             tests/smoke/simmath-vectors.spec.js), the libm crate output \
             changed and both pins need a deliberate, reviewed update. If it \
             is native-only, native has drifted off shared libm — that is \
             the exact regression this file exists to catch.",
        result.digest
    );
}

/// Runtime proof, not just a Cargo-feature-flag proof, for issue #909
/// AC-2: nalgebra's own scalar transcendentals (via the `simba`
/// `ComplexField` impl it re-exports) must land on the exact same libm
/// output as `crate::simmath`, bit for bit — otherwise nalgebra's
/// `Cargo.toml` features could be "on" while nalgebra still silently
/// computes through `std` (see the long comment on the `nalgebra`/
/// `parry3d`/`rapier3d` dependency block in `Cargo.toml` for why that is
/// the *plausible* failure, not a hypothetical one: `simba`'s `libm`
/// feature alone is a verified no-op on a std target, and only
/// `libm_force` — wired in here via `rapier3d`/`parry3d`'s
/// `enhanced-determinism` feature — actually overrides it).
#[test]
fn nalgebra_scalar_math_routes_through_the_same_libm_as_simmath() {
    use nalgebra::ComplexField;
    for x in [0.3_f32, 1.0, -2.5, 10.0, 0.6] {
        assert_eq!(
            ComplexField::sin(x).to_bits(),
            simmath::sin(x).to_bits(),
            "nalgebra::ComplexField::sin({x}) disagreed with \
                 crate::simmath::sin({x}) — nalgebra is not actually routing \
                 through shared libm despite its Cargo.toml feature"
        );
        assert_eq!(
            ComplexField::cos(x).to_bits(),
            simmath::cos(x).to_bits(),
            "nalgebra::ComplexField::cos({x}) disagreed with \
                 crate::simmath::cos({x})"
        );
        assert_eq!(
            ComplexField::powf(x, 1.3_f32).to_bits(),
            simmath::powf(x, 1.3).to_bits(),
            "nalgebra::ComplexField::powf({x}, 1.3) disagreed with \
                 crate::simmath::powf({x}, 1.3)"
        );
    }
}

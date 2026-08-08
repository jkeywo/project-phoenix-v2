//! Cross-target vector battery for `crate::simmath` and the dependency math
//! it shares a libm with (issue #909).
//!
//! # Why this exists
//!
//! `src/simmath.rs` (issue #908) pins 12 hand-picked cases as a tripwire —
//! enough to catch a wrapper regressing back onto `std`, not enough to prove
//! native and wasm agree across each function's *domain*. This module is the
//! systematic version: an enumerated, deterministic battery (no RNG — a
//! fixed grid plus explicit edge cases) run through every `simmath` function
//! — plus probes into `nalgebra`/`glam`, which is where the crates issue #909
//! is actually named after do their transcendental math (see *Dependency
//! probes* below) — all folded into one canonical digest. The native test
//! inline below and the wasm export at the bottom of this file both call
//! [`run_battery`] and assert the exact same digest, so drift between targets
//! collapses to one comparable `u64` instead of "some case somewhere
//! differs".
//!
//! # What is and is not in the digest
//!
//! Every input is either a fixed Rust literal, or derived from literals
//! using nothing but IEEE-exact `+ − × ÷` (that is all `linspace` does), so
//! the *inputs* are byte-identical on every target already: `f32` literals —
//! including `f32::NAN` — are compile-time bit patterns rather than machine
//! instructions, and IEEE-754 pins the four basic operations exactly
//! everywhere. What might legitimately differ is the *output*, and that is
//! exactly what libm-vs-std routing (issue #908/#909) is about.
//!
//! One deliberate exception: NaN *outputs* are folded in by their `is_nan()`
//! shape, not their raw bits. Every case here is finite-input arithmetic
//! computed by pure-Rust `libm`, so in principle the payload bits should
//! already be reproducible cross-target — but NaN payload bits are exactly
//! the kind of thing a LLVM backend is allowed to canonicalize differently
//! per target ABI, and that is a wasm32-vs-x86_64 codegen question, not a
//! libm-routing one. Pinning raw NaN bits here would risk failing this guard
//! for a reason unrelated to what it exists to catch. The *shape* (NaN or
//! not) is still asserted, so a function that stops propagating NaN
//! correctly is still caught.
//!
//! # Domain coverage
//!
//! Per function, see its `*_domain()` builder below for the rationale; in
//! summary:
//!
//! - `sin`/`cos`/`sin_cos`: a linear sweep, quadrant-boundary points (with
//!   nearby perturbations), large-magnitude argument-reduction stress
//!   (`1e3..1e8`), sim-realistic moderate magnitudes, and the shared edge
//!   scalars (±0, subnormals, ±∞, NaN, `MIN`/`MAX`).
//! - `tan`: the trig sweep plus points clustered on both sides of each
//!   asymptote (odd multiples of π/2).
//! - `asin`/`acos`: a sweep across `[-1, 1]`, points hugging both endpoints
//!   (where the derivative diverges), and just-out-of-domain values (NaN
//!   result expected).
//! - `atan`: a wide sweep plus saturating large magnitudes.
//! - `atan2`: axis-aligned and signed-zero quadrant boundaries (the
//!   `atan2(±0, ±0)` branch points explicitly), infinities, NaN, and a
//!   quadrant × aspect-ratio grid.
//! - `powf`: a base × exponent grid at sim-realistic magnitudes, plus the
//!   IEEE-754 special cases (zero/negative/infinite base, integer vs.
//!   fractional exponent, `x^0`, `1^x`, NaN propagation).
//! - `exp`: a sweep plus the f32 overflow/underflow boundary (~88 / ~-104).
//! - `ln`: a log-spaced positive sweep plus the domain boundary at zero and
//!   negative inputs (NaN result expected).
//!
//! # Dependency probes — why `crate::simmath` alone is not the proof
//!
//! Issue #909 is named after the *dependencies*: `nalgebra`, `parry3d`,
//! `rapier3d`/`simba` and `glam`. A battery that only calls
//! `crate::simmath` would pass identically whether or not those crates got
//! rewired, because `crate::simmath` calls the `libm` crate directly and
//! always has. So the same digest also folds in values computed *by the
//! dependencies*, under their own tags:
//!
//! - `nalgebra.sin` / `nalgebra.cos` / `nalgebra.powf` —
//!   [`nalgebra::ComplexField`], and `nalgebra.atan2` —
//!   [`nalgebra::RealField`] (`atan2` lives on `RealField`, not
//!   `ComplexField`). These are the `simba` scalar impls the whole
//!   collision/dynamics stack computes through, and the exact thing
//!   `simba/libm_force` (via `rapier3d`/`parry3d`'s `enhanced-determinism`)
//!   redirects. A wasm build where `simba` silently fell back to `std` math
//!   changes these outputs and therefore changes the digest.
//! - `glam.vec2_to_angle` (`Vec2::to_angle` → `math::atan2`) and
//!   `glam.quat_from_rotation_y` (`Quat::from_rotation_y` →
//!   `math::sin_cos`) — glam's `libm` feature swaps exactly those two
//!   `math::*` shims, so losing the feature moves the digest.
//! - `glam.vec3_angle_between` is folded in for completeness but is *not* a
//!   libm probe: glam implements `Vec3::angle_between` with
//!   `acos_approx`, a 7-degree minimax polynomial over `abs`/`sqrt`, which
//!   is IEEE-exact with or without the `libm` feature. It is here as a
//!   composite-arithmetic canary (dot/length/divide ordering), and the
//!   distinction is recorded so nobody later reads a passing digest as
//!   proof that glam's `acos` path is pinned.
//!
//! What this still does not cover: `rapier3d`/`parry3d` are not called
//! directly here. They are covered transitively — their scalar
//! transcendentals *are* the `simba` impls probed above, and their
//! `enhanced-determinism` feature is what forces those impls onto
//! `libm_force`. Solver-level bit-exactness is a separate problem (thread
//! reduction order — issue #896) and deliberately out of scope.

use crate::simmath;

// ── Canonical digest ────────────────────────────────────────────────────────
// Hand-rolled FNV-1a/64: no new crate dependency for a fold this small, and
// the whole point is "one comparable value" — the algorithm just needs to be
// identical on every target, which a few lines of integer arithmetic already
// guarantees without pulling in a hashing crate's own version drift.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

struct Digest {
    hash: u64,
    /// How many cases each tag contributed, in first-seen order. Exactly one
    /// [`Digest::feed_tag`] call opens each case, so this doubles as the
    /// case count *and* as the per-function coverage record the
    /// `battery_covers_every_wrapped_function` test asserts on — a domain
    /// builder that regresses to empty drops its tag from this list rather
    /// than merely shrinking a total.
    tag_counts: Vec<(&'static str, usize)>,
}

impl Digest {
    fn new() -> Self {
        Digest {
            hash: FNV_OFFSET,
            tag_counts: Vec::new(),
        }
    }

    fn feed_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash ^= b as u64;
            self.hash = self.hash.wrapping_mul(FNV_PRIME);
        }
    }

    /// Open a case, tagged with which function produced it, so two functions
    /// that happen to agree on a value at the same list position can't cancel
    /// out and hide a divergence. Called exactly once per case.
    fn feed_tag(&mut self, tag: &'static str) {
        self.feed_bytes(tag.as_bytes());
        self.feed_bytes(&[0]);
        match self.tag_counts.iter_mut().find(|(t, _)| *t == tag) {
            Some((_, n)) => *n += 1,
            None => self.tag_counts.push((tag, 1)),
        }
    }

    /// An input value: fed as its exact bits. Inputs are literals, or derived
    /// from literals with IEEE-exact arithmetic only, so this is only ever
    /// hashing a compile-time-identical constant — it is here for the fold,
    /// not because inputs could plausibly drift.
    fn feed_input(&mut self, x: f32) {
        self.feed_bytes(&x.to_bits().to_le_bytes());
    }

    /// An output value: NaN folds in by shape (see module docs — NaN payload
    /// bits are a codegen question, not a libm-routing one); everything else
    /// (including the two signed infinities and signed zero) folds in by its
    /// exact bit pattern.
    fn feed_output(&mut self, x: f32) {
        if x.is_nan() {
            self.feed_bytes(b"NaN");
        } else {
            self.feed_bytes(&x.to_bits().to_le_bytes());
        }
    }
}

/// The result of running the whole battery: a single comparable digest, plus
/// a case count so a battery that silently shrank to zero cases can't still
/// "pass" by matching an empty digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryResult {
    pub digest: u64,
    pub case_count: usize,
}

/// [`BatteryResult`] plus the per-tag breakdown behind it, in first-seen
/// order. Only the native coverage test needs the breakdown; the wasm export
/// and the digest comparison work off [`BatteryResult`] alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryRun {
    pub result: BatteryResult,
    pub cases_per_tag: Vec<(&'static str, usize)>,
}

// ── Vector builders ─────────────────────────────────────────────────────────

fn linspace(start: f32, end: f32, n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![start];
    }
    let step = (end - start) / (n as f32 - 1.0);
    (0..n).map(|i| start + step * i as f32).collect()
}

/// Values every function's domain needs to answer for at least once: signed
/// zero, the smallest normal and subnormal (both signs), `EPSILON`, the
/// finite extremes, both infinities, and NaN.
fn edge_scalars() -> Vec<f32> {
    vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(1),  // smallest positive subnormal
        -f32::from_bits(1), // smallest negative subnormal
        f32::EPSILON,
        f32::MAX,
        f32::MIN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
    ]
}

/// Shared sweep for `sin`/`cos`/`sin_cos`/`tan`: a linear pass across
/// [-2π, 2π], every quarter-π boundary with small perturbations either side
/// (argument reduction is most error-prone right at those boundaries), a
/// large-magnitude reduction stress ladder up to 1e8 (both signs — this is
/// where naive reduction algorithms lose precision fastest), a moderate
/// "sim-realistic" range for accumulated headings/turn rates, and the shared
/// edge scalars.
fn trig_domain() -> Vec<f32> {
    use std::f32::consts::PI;
    let mut v = linspace(-2.0 * PI, 2.0 * PI, 41);
    for k in [-2.0_f32, -1.5, -1.0, -0.5, 0.5, 1.0, 1.5, 2.0] {
        let base = k * PI;
        for delta in [-1e-3_f32, -1e-5, 0.0, 1e-5, 1e-3] {
            v.push(base + delta);
        }
    }
    v.extend(linspace(-100.0, 100.0, 21)); // sim-realistic moderate range
    for mag in [1e3_f32, 1e4, 1e5, 1e6, 1e7, 1e8] {
        v.push(mag);
        v.push(-mag);
    }
    v.extend(edge_scalars());
    v
}

/// `trig_domain` plus points clustered tightly on both sides of every
/// asymptote (odd multiples of π/2) inside the swept range — `tan` blows up
/// there, and that is exactly where a reduction-algorithm disagreement
/// between targets would show up first.
fn tan_domain() -> Vec<f32> {
    use std::f32::consts::PI;
    let mut v = trig_domain();
    let half_pi = PI / 2.0;
    for k in [-3.0_f32, -1.0, 1.0, 3.0] {
        let asymptote = k * half_pi;
        v.push(asymptote);
        for delta in [-1e-3_f32, -1e-4, -1e-5, 1e-5, 1e-4, 1e-3] {
            v.push(asymptote + delta);
        }
    }
    v
}

/// `asin`/`acos` share a domain: a sweep across `[-1, 1]`, points hugging
/// both endpoints from inside (derivative diverges there — the classic
/// precision-loss spot), the endpoints themselves, and just-out-of-domain
/// values (`asin`/`acos` of a value outside `[-1, 1]` is NaN by definition;
/// exercising that is a shape check, folded via `feed_output`'s NaN rule).
fn asin_acos_domain() -> Vec<f32> {
    let mut v = linspace(-1.0, 1.0, 41);
    v.extend([-1.0, 1.0, 0.0, -0.0]);
    for delta in [1e-3_f32, 1e-4, 1e-5, 1e-6, 1e-7] {
        v.push(1.0 - delta);
        v.push(-1.0 + delta);
    }
    v.extend([1.0000001_f32, -1.0000001, 1.5, -1.5, f32::NAN]);
    v
}

/// `atan` is defined everywhere, so the interesting behaviour is the
/// saturation toward ±π/2 at large magnitude rather than a domain edge.
fn atan_domain() -> Vec<f32> {
    let mut v = linspace(-1000.0, 1000.0, 41);
    v.extend(edge_scalars());
    v.extend([1e10_f32, -1e10, 1e20, -1e20]);
    v
}

/// `atan2(y, x)` pairs: the signed-zero quadrant boundaries explicitly
/// (`atan2(±0, ±0)` picks a different branch for each of the four sign
/// combinations — this is the textbook case the issue calls out), the four
/// axis-aligned half-planes, the four diagonal quadrants, infinities, NaN,
/// and a quadrant × aspect-ratio grid (steep and shallow angles, small and
/// large magnitudes) built from plain literals rather than from `sin`/`cos`
/// so this domain never depends on the functions under test elsewhere in the
/// battery.
fn atan2_domain() -> Vec<(f32, f32)> {
    let mut v = vec![
        (0.0, 0.0),
        (-0.0, 0.0),
        (0.0, -0.0),
        (-0.0, -0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (1.0, 0.0),
        (-1.0, 0.0),
        (1.0, 1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (-1.0, -1.0),
        (f32::INFINITY, f32::INFINITY),
        (f32::INFINITY, f32::NEG_INFINITY),
        (f32::NEG_INFINITY, f32::INFINITY),
        (f32::NEG_INFINITY, f32::NEG_INFINITY),
        (f32::NAN, 1.0),
        (1.0, f32::NAN),
    ];
    for &y in &[0.1_f32, 1.0, 3.0, 10.0, -0.1, -1.0, -3.0, -10.0] {
        for &x in &[0.1_f32, 1.0, 3.0, 10.0, -0.1, -1.0, -3.0, -10.0] {
            v.push((y, x));
        }
    }
    v
}

/// `powf(base, exponent)` pairs: a base × exponent grid at sim-realistic
/// magnitudes (falloff curves, drag exponents — small bases, exponents in
/// roughly [-3, 4] including a non-integer one), plus the IEEE-754 special
/// cases: zero/negative-zero base, `x^0 == 1` (even for NaN/∞ bases),
/// `1^x == 1` (even for NaN/∞ exponents), negative base with integer vs.
/// fractional exponent (the latter is NaN — no real result), and infinite
/// base or exponent.
fn powf_domain() -> Vec<(f32, f32)> {
    let mut v = Vec::new();
    for &base in &[0.5_f32, 1.0, 1.5, 2.0, 2.5, 10.0, 0.1] {
        for &exp in &[-3.0_f32, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0, 3.0, 3.7] {
            v.push((base, exp));
        }
    }
    v.extend([
        (0.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (-0.0, 0.0),
        (-0.0, 3.0),
        (-0.0, 2.0),
        (1.0, f32::INFINITY),
        (1.0, f32::NAN),
        (f32::NAN, 0.0),
        (f32::INFINITY, 0.0),
        (-1.0, 2.0),
        (-1.0, 3.0),
        (-2.0, 0.5),
        (f32::INFINITY, 1.0),
        (f32::INFINITY, -1.0),
        (f32::NEG_INFINITY, 3.0),
        (f32::NEG_INFINITY, 2.0),
        (2.0, f32::INFINITY),
        (2.0, f32::NEG_INFINITY),
        (0.5, f32::INFINITY),
        (0.5, f32::NEG_INFINITY),
    ]);
    v
}

/// `exp` sweep plus the f32 overflow (~88.7) / underflow (~-103.97)
/// boundary, both sides.
fn exp_domain() -> Vec<f32> {
    let mut v = linspace(-20.0, 20.0, 41);
    v.extend(edge_scalars());
    v.extend([88.0_f32, 88.8, 89.0, -103.0, -104.0, -105.0, 200.0, -200.0]);
    v
}

/// `ln` domain is `x > 0`; a log-spaced sweep across ~70 orders of magnitude,
/// plus the boundary at zero (both signs — `ln` of either is -∞) and
/// negative inputs (NaN result).
fn ln_domain() -> Vec<f32> {
    let mut v = vec![
        1e-40_f32,
        1e-30,
        1e-20,
        1e-10,
        1e-5,
        1e-3,
        0.1,
        0.5,
        0.9,
        1.0,
        1.1,
        2.0,
        10.0,
        100.0,
        1e5,
        1e10,
        1e20,
        1e30,
        f32::MAX,
    ];
    v.extend([
        0.0,
        -0.0,
        -1.0,
        -0.5,
        f32::MIN_POSITIVE,
        f32::from_bits(1),
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ]);
    v
}

// ── Dependency probe domains ────────────────────────────────────────────────
// Deliberately modest next to the `simmath` domains above: these exist to
// prove *routing* (does nalgebra/glam land on the same libm as
// `crate::simmath`?), not to re-explore each function's domain — that job is
// already done by the builders above, and the wrapped functions and the
// dependency impls bottom out in the same `libm` entry points. A handful of
// ordinary in-range values is enough to move the digest if a target silently
// falls back to `std`; edge cases here would only re-test `libm` itself.

/// Shared single-argument inputs for the `nalgebra`/`glam` probes: ordinary
/// sim-scale magnitudes across both signs, including signed zero.
fn dep_scalar_domain() -> Vec<f32> {
    vec![
        -100.0, -10.0, -3.5, -2.5, -1.5, -1.0, -0.75, -0.5, -0.25, -0.0, 0.0, 0.25, 0.5, 0.75, 1.0,
        1.5, 2.5, 3.5, 10.0, 100.0,
    ]
}

/// `(base, exponent)` pairs for `nalgebra`'s `ComplexField::powf`. Bases are
/// positive: a negative base with a fractional exponent is NaN, and NaN
/// outputs fold in by shape, which would blunt the probe.
fn dep_powf_domain() -> Vec<(f32, f32)> {
    let mut v = Vec::new();
    for &base in &[0.5_f32, 1.5, 2.0, 2.5, 10.0] {
        for &exp in &[-2.0_f32, -0.5, 0.5, 1.3, 2.0, 3.7] {
            v.push((base, exp));
        }
    }
    v
}

/// `(y, x)` pairs for `nalgebra`'s `RealField::atan2` and glam's
/// `Vec2::to_angle`: all four quadrants, both axes, and the origin.
fn dep_atan2_domain() -> Vec<(f32, f32)> {
    let mut v = Vec::new();
    for &y in &[-3.0_f32, -1.0, -0.25, 0.0, 0.25, 1.0, 3.0] {
        for &x in &[-3.0_f32, -1.0, -0.25, 0.0, 0.25, 1.0, 3.0] {
            v.push((y, x));
        }
    }
    v
}

/// Vector pairs for glam's `Vec3::angle_between`: parallel, antiparallel,
/// perpendicular, and assorted oblique pairs at mixed magnitudes.
fn dep_vec3_pair_domain() -> Vec<([f32; 3], [f32; 3])> {
    vec![
        ([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([1.0, 1.0, 0.0], [1.0, 0.0, 1.0]),
        ([1.0, 2.0, 3.0], [-3.0, 2.0, -1.0]),
        ([0.5, -0.25, 2.0], [2.0, 0.25, -0.5]),
        ([-2.5, 3.5, -10.0], [10.0, -3.5, 2.5]),
        ([100.0, 0.0, 0.0], [0.0, 0.0, 0.25]),
    ]
}

// ── The battery ──────────────────────────────────────────────────────────

/// Run every `simmath` function — and the `nalgebra`/`glam` probes that stand
/// in for the rest of the dependency stack — across its battery, folding every
/// (function, input, output) tuple into one canonical digest in a fixed order.
/// Deterministic: no RNG, no wall clock, no allocation order that could vary —
/// just enumerated literals folded in a fixed sequence, so two runs (same
/// target or cross-target) either produce the same `u64` or the battery has
/// genuinely found a divergence.
pub fn run_battery() -> BatteryResult {
    run_battery_detailed().result
}

/// [`run_battery`], keeping the per-tag breakdown. Same fold, same digest —
/// the breakdown is bookkeeping the coverage test reads, not an input to the
/// hash.
pub fn run_battery_detailed() -> BatteryRun {
    use nalgebra::{ComplexField, RealField};

    let mut d = Digest::new();

    for x in trig_domain() {
        d.feed_tag("sin");
        d.feed_input(x);
        d.feed_output(simmath::sin(x));

        d.feed_tag("cos");
        d.feed_input(x);
        d.feed_output(simmath::cos(x));

        let (s, c) = simmath::sin_cos(x);
        d.feed_tag("sin_cos.sin");
        d.feed_input(x);
        d.feed_output(s);

        d.feed_tag("sin_cos.cos");
        d.feed_input(x);
        d.feed_output(c);
    }

    for x in tan_domain() {
        d.feed_tag("tan");
        d.feed_input(x);
        d.feed_output(simmath::tan(x));
    }

    for x in asin_acos_domain() {
        d.feed_tag("asin");
        d.feed_input(x);
        d.feed_output(simmath::asin(x));

        d.feed_tag("acos");
        d.feed_input(x);
        d.feed_output(simmath::acos(x));
    }

    for x in atan_domain() {
        d.feed_tag("atan");
        d.feed_input(x);
        d.feed_output(simmath::atan(x));
    }

    for (y, x) in atan2_domain() {
        d.feed_tag("atan2");
        d.feed_input(y);
        d.feed_input(x);
        d.feed_output(simmath::atan2(y, x));
    }

    for (base, exp) in powf_domain() {
        d.feed_tag("powf");
        d.feed_input(base);
        d.feed_input(exp);
        d.feed_output(simmath::powf(base, exp));
    }

    for x in exp_domain() {
        d.feed_tag("exp");
        d.feed_input(x);
        d.feed_output(simmath::exp(x));
    }

    for x in ln_domain() {
        d.feed_tag("ln");
        d.feed_input(x);
        d.feed_output(simmath::ln(x));
    }

    // ── Dependency probes (see the module docs) ─────────────────────────
    // Same digest, own tags: if `simba` or `glam` stops routing through the
    // shared libm on one target, these values move and the single pinned
    // `u64` stops matching — which is the whole point of folding them in
    // here rather than asserting them in a separate native-only test that
    // the wasm build never runs.

    for x in dep_scalar_domain() {
        d.feed_tag("nalgebra.sin");
        d.feed_input(x);
        d.feed_output(ComplexField::sin(x));

        d.feed_tag("nalgebra.cos");
        d.feed_input(x);
        d.feed_output(ComplexField::cos(x));
    }

    for (base, exp) in dep_powf_domain() {
        d.feed_tag("nalgebra.powf");
        d.feed_input(base);
        d.feed_input(exp);
        d.feed_output(ComplexField::powf(base, exp));
    }

    for (y, x) in dep_atan2_domain() {
        d.feed_tag("nalgebra.atan2");
        d.feed_input(y);
        d.feed_input(x);
        d.feed_output(RealField::atan2(y, x));

        // `Vec2::to_angle` is glam's `math::atan2(self.y, self.x)`, so this
        // is the same question asked of glam's shim instead of simba's.
        d.feed_tag("glam.vec2_to_angle");
        d.feed_input(y);
        d.feed_input(x);
        d.feed_output(glam::Vec2::new(x, y).to_angle());
    }

    for angle in dep_scalar_domain() {
        // `Quat::from_rotation_y` is `math::sin_cos(angle * 0.5)` packed into
        // xyzw; all four components fold in under one case.
        d.feed_tag("glam.quat_from_rotation_y");
        d.feed_input(angle);
        for component in glam::Quat::from_rotation_y(angle).to_array() {
            d.feed_output(component);
        }
    }

    for (a, b) in dep_vec3_pair_domain() {
        // NOT a libm probe — glam's `acos_approx` is a minimax polynomial
        // over `abs`/`sqrt`, IEEE-exact either way. Here as a
        // composite-arithmetic canary; see the module docs.
        d.feed_tag("glam.vec3_angle_between");
        for v in a.iter().chain(b.iter()) {
            d.feed_input(*v);
        }
        d.feed_output(glam::Vec3::from_array(a).angle_between(glam::Vec3::from_array(b)));
    }

    let case_count = d.tag_counts.iter().map(|(_, n)| n).sum();
    BatteryRun {
        result: BatteryResult {
            digest: d.hash,
            case_count,
        },
        cases_per_tag: d.tag_counts,
    }
}

// ── WASM export ──────────────────────────────────────────────────────────
// The browser-side half of the proof: a smoke spec calls this from inside
// the running wasm page and asserts the returned digest against the same
// pinned constant the native test below asserts against (see
// `tests/smoke/simmath-vectors.spec.js`). `server.html` has to promote it
// onto `window` alongside the other exports for the spec to reach it — an
// explicit allowlist, not an automatic re-export.
//
// Deliberately NOT feature-gated, and yes, that ships test scaffolding in
// the release wasm. Accepted on purpose: the claim being made is about the
// *deployed* artifact, and an export gated behind a test-only feature would
// only ever prove a binary nobody serves. The cost is one small function
// plus the vector tables it walks — noise beside Bevy in the same module.
// Revisit if a wasm size budget ever lands (issue #868's budgets cover
// assets, not the wasm blob).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn wasm_simmath_battery() -> String {
    let result = run_battery();
    format!(
        "{{\"digest\":\"{:016x}\",\"case_count\":{}}}",
        result.digest, result.case_count
    )
}

#[cfg(test)]
mod tests {
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
}

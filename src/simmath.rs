//! The only sanctioned source of transcendental float math in simulation
//! code (issue #908).
//!
//! # Why this module exists
//!
//! Native↔wasm bit-exactness is a product requirement: multi-peer lockstep
//! runs the same simulation on every peer and compares nothing but inputs,
//! so a last-bit float difference compounds into visible divergence within
//! seconds. IEEE-754 guarantees `+ - * / sqrt` (and `powi`, `to_radians`,
//! `to_degrees` by construction) are bit-identical everywhere — those stay
//! on `std`. It guarantees **nothing** about `sin`, `cos`, `tan`, `atan2`,
//! `powf`, `exp`, `ln` and friends: `std` routes them to a libm, native
//! links the *system* libm (glibc, MSVCRT, …) while wasm gets Rust's, and
//! their last bits differ. Routing every simulation call site through this
//! one pure-Rust [`libm`] crate makes the answer identical on every target.
//!
//! # Enforcement
//!
//! `clippy.toml` lists the raw `f32`/`f64` methods under
//! `disallowed-methods`, and CI denies clippy warnings — a new bare
//! `.cos()` fails the build rather than silently desyncing two peers a
//! month later.
//!
//! # Sanctioned exclusions (presentation-only float math)
//!
//! Rendering, audio panning, and dev-tool camera math never feed back into
//! simulation state, so platform-varying `std` transcendentals are harmless
//! there. Three singleton call sites carry a scoped per-fn
//! `#[allow(clippy::disallowed_methods)]` with a one-line justification:
//!
//! * `audio_config.rs` (`listener_relative`) — Web Audio panning
//! * `entities/star.rs` (`uv_sphere_mesh`) — render mesh generation
//! * `weapons/beam_render.rs` — beam gizmo geometry
//!
//! Four modules carry the allow at module scope
//! (`#![allow(clippy::disallowed_methods)]`) because the entire module is
//! render/presentation path, not just one function:
//!
//! * `server/pfx.rs` — particle effects
//! * `server/renderer.rs` — render pipeline
//! * `gui/radar.rs` — server viewscreen radar widget
//! * `viewer/camera.rs` — standalone model-viewer camera
//!
//! Any new sim-feeding helper must not be added to those four modules —
//! sim math belongs in this file, not behind a presentation-path allow.
//!
//! # Adding a function
//!
//! Only what simulation code actually uses is wrapped. If sim code needs
//! `ln`, `log2`, `sinh`, an `f64` variant, … add a delegating wrapper here
//! (backed by `libm`, never `std`) instead of allowing the lint locally.

/// `x.sin()`, routed through the shared pure-Rust libm.
#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}

/// `x.cos()`, routed through the shared pure-Rust libm.
#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}

/// `x.sin_cos()`, routed through the shared pure-Rust libm.
#[inline]
pub fn sin_cos(x: f32) -> (f32, f32) {
    libm::sincosf(x)
}

/// `x.tan()`, routed through the shared pure-Rust libm.
#[inline]
pub fn tan(x: f32) -> f32 {
    libm::tanf(x)
}

/// `x.asin()`, routed through the shared pure-Rust libm.
#[inline]
pub fn asin(x: f32) -> f32 {
    libm::asinf(x)
}

/// `x.acos()`, routed through the shared pure-Rust libm.
#[inline]
pub fn acos(x: f32) -> f32 {
    libm::acosf(x)
}

/// `x.atan()`, routed through the shared pure-Rust libm.
#[inline]
pub fn atan(x: f32) -> f32 {
    libm::atanf(x)
}

/// `y.atan2(x)`, routed through the shared pure-Rust libm.
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

/// `base.powf(exponent)`, routed through the shared pure-Rust libm.
#[inline]
pub fn powf(base: f32, exponent: f32) -> f32 {
    libm::powf(base, exponent)
}

/// `x.exp()`, routed through the shared pure-Rust libm.
#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}

/// `x.ln()`, routed through the shared pure-Rust libm.
#[inline]
pub fn ln(x: f32) -> f32 {
    libm::logf(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin a handful of wrapper outputs to exact bit patterns.
    ///
    /// These are the values `libm` 0.2 produces; they must be identical on
    /// every platform (that is the whole point of the module). If this test
    /// fails, either a wrapper silently fell back to a `std` method whose
    /// system libm disagrees with these bits on this platform, or a `libm`
    /// upgrade changed an answer — both mean cross-target lockstep would
    /// desync, and both need a deliberate decision, not a re-bless. The
    /// full cross-target vector battery is issue #909; this is only the
    /// tripwire.
    #[test]
    fn wrapper_outputs_are_bit_exact() {
        let cases: [(f32, u32); 12] = [
            (sin(1.0), 0x3f576aa4),
            (sin(-2.5), 0xbf193578),
            (cos(1.0), 0x3f0a5140),
            (cos(-2.5), 0xbf4d17bf),
            (tan(0.7), 0x3f57a036),
            (asin(0.6), 0x3f24bc7e),
            (acos(0.6), 0x3f6d6338),
            (atan(1.5), 0x3f7b985f),
            (atan2(1.0, -2.0), 0x402b6374),
            (powf(2.5, 1.3), 0x40529f03),
            (exp(1.0), 0x402df854),
            (ln(2.0), 0x3f317218),
        ];
        for (i, (got, want)) in cases.iter().enumerate() {
            assert_eq!(
                got.to_bits(),
                *want,
                "case {i}: got {got} ({:#010x}), want bits {want:#010x}",
                got.to_bits(),
            );
        }
    }

    /// `sin_cos` must agree bit-for-bit with the individual `sin`/`cos`
    /// wrappers, so call sites can use either form interchangeably.
    #[test]
    fn sin_cos_matches_sin_and_cos() {
        for x in [0.0_f32, 1.0, -2.5, std::f32::consts::PI, 100.0] {
            let (s, c) = sin_cos(x);
            assert_eq!(s.to_bits(), sin(x).to_bits());
            assert_eq!(c.to_bits(), cos(x).to_bits());
        }
    }
}

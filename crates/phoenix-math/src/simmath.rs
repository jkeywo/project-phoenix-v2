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

/// `x.hypot(y)`, routed through the shared pure-Rust libm.
#[inline]
pub fn hypot(x: f32, y: f32) -> f32 {
    libm::hypotf(x, y)
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

/// `x.log10()`, routed through the shared pure-Rust libm.
#[inline]
pub fn log10(x: f32) -> f32 {
    libm::log10f(x)
}

#[cfg(test)]
#[path = "simmath_tests.rs"]
mod tests;

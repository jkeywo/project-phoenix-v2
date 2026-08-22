//! Phoenix's deterministic-math foundation (issue #1184).
//!
//! Extracted from the root `project-phoenix` crate into its own workspace
//! member so "the deterministic foundation depends on nothing" is a
//! compiler-checked property: this crate has **no Bevy dependency**, and its
//! whole dependency surface is `libm` and `serde`. The root crate re-exports
//! every module here (`pub use phoenix_math::simmath;` and friends), so every
//! existing `crate::simmath::…` / `project_phoenix::simmath::…` import path
//! still resolves unchanged.
//!
//! Modules that could NOT move — because they carry a hard Bevy dependency
//! (`sim_rng`, `world_id`, `sim_sets`, `core::balance`) or a non-Bevy math
//! dependency this crate deliberately excludes (`simmath_vectors` needs
//! `nalgebra`) — stay in the root crate. See issue #1184.
#![forbid(unsafe_code)]

pub mod audio_config;
/// Fixed-capacity history window (issue #788). Pure, Bevy-free, domain-neutral.
pub mod bounded_history;
/// Composite-key deterministic value derivation (issue #788). Pure, Bevy-free,
/// domain-neutral.
pub mod composite_rng;
/// Pure math behind the automatic LOD switch-range tuner (`tune-lods`): the
/// alpha-aware image difference and the knee-of-curve rule. Ungated so its unit
/// tests run under the default `cargo test`.
pub mod lod_tune;
/// Shared pure-Rust libm wrappers — the only sanctioned transcendental float
/// math in simulation code (issue #908; enforced via clippy.toml).
pub mod simmath;

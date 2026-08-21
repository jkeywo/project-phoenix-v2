//! The tractor beam (issue #1156) — the linchpin slice of PRD #1143's
//! distributed ship operations.
//!
//! Engineering gets a tractor beam and a tow becomes a chain no single seat can
//! complete. The tractor is a first-class engineering-owned `[[system]]`: it
//! declares a power group, carries a damage entry, is admission-gated, registers
//! as an admitted consumer and publishes its own blackboard. Engaging it couples
//! the ship to whatever Tactical currently has locked; releasing it, losing the
//! lock, drifting out of the authored range, losing the power allocation or
//! having the tractor knocked out all drop the coupling.
//!
//! Split the same way `operations` is (rule 10):
//!
//! * [`coupling`] — the pure, Bevy-free half: the coupling-position module
//!   (where the held target sits, from transforms and the authored offset), the
//!   hold verdict, the authored `[tractor]` config and its validation, and the
//!   refusal vocabulary. Unit-tested in isolation.
//! * [`server`] — the Bevy adapter: the per-ship [`server::TractorBeam`]
//!   component, the fixed-tick systems that take the engage/release commands,
//!   decide the hold, move the held target, and publish the blackboard.
//!
//! This is the shape the umbilical (#1160), dock (#1159) and external
//! repair-dispatch (#1161) slices copy.

/// The pure, Bevy-free coupling geometry, hold verdict, authored config and
/// refusal vocabulary.
pub mod coupling;
/// The pure, Bevy-free held-response vocabulary (issue #1158): what being held
/// DOES to a target, authored on the target itself.
pub mod held_response;
/// The Bevy adapter: the component, the fixed-tick systems and the blackboard
/// publisher.
pub mod server;

pub use coupling::{
    coupled_position, hold_status, tow_load_penalty, TowLoadCurve, TractorConfig, TractorRefusal,
};
pub use held_response::{
    condition_delta, held_offset, HeldResponse, HeldResponseConfig, HeldResponseKind,
};
pub use server::{
    apply_tow_load_penalty, arrest_held_declines, handle_tractor_commands, move_coupled_target,
    operate_tractor_ai, publish_tractor_blackboard, tick_tractor, tractor_blackboard_key,
    HeldResponseSection, TractorBeam, TractorPlugin, TractorSaveState,
};

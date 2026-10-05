//! The shared typed input path every system AI operator emits through
//! (issue #738).
//!
//! Before this module, seven console/system AI decide systems each carried a
//! byte-identical private `emit_*_ai_command` helper: build this ship's
//! `ai:<uuid>` token, fabricate an empty [`crate::ship::config::ShipConfig`]
//! when the entity has no `ShipConfigComponent`, then call
//! [`super::validate_and_admit`]. They now all route through
//! [`emit_ai_command`], so there is exactly one place that decides what an AI
//! operator's token looks like and what an unconfigured ship's admission
//! context is.
//!
//! This is a de-duplication, not a policy change: the token shape, the
//! fallback config, and the validation call are all unchanged from the copies
//! it replaces.

/// Validate-and-enqueue one AI decision into this ship's own
/// `AdmittedCommands` through [`super::validate_and_admit`] — the same seam
/// network `ControlSystem` messages pass through.
///
/// The command is checked against *this* entity's own `ControlSourceResolver`
/// (`operate_ai` must hold on `target`). The write happens in the same tick,
/// so the paired applier — scheduled after the decide system — sees it without
/// a one-tick queue lag.
///
/// `ship_config` is `Option` because NPC ships spawned without a
/// `ShipConfigComponent` still emit: an empty [`crate::ship::config::ShipConfig`]
/// stands in, which grants no station tenure and so only ever admits `ai:`
/// tokens on systems whose control source already says `operate_ai`.
pub fn emit_ai_command(
    entity_uuid: Option<&crate::entities::spawner::EntityUuid>,
    target: crate::core::messages::SystemId,
    payload: crate::core::messages::SystemControlPayload,
    sources: &crate::ship_plugin::ShipSystemControlSources,
    _sessions: &crate::lobby::Sessions,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
    admitted: &mut crate::core::messages::AdmittedCommands,
) -> bool {
    phoenix_sim_gameplay::command_admission::ai_emit::emit_ai_command(
        entity_uuid,
        target,
        payload,
        sources,
        ship_config,
        admitted,
    )
}

#[cfg(test)]
#[path = "ai_emit_tests.rs"]
mod tests;

pub use phoenix_sim_gameplay::command_admission::ai_emit::{
    ai_token_for, is_ai_token, AI_BACKFILL_TOKEN, AI_TOKEN_PREFIX,
};

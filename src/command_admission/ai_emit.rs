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

/// The token an AI operator on a ship with no [`crate::entity_spawner::EntityUuid`]
/// emits under.
///
/// Deliberately *not* registered in `crate::ai::server::AiTokenRegistry`: an
/// unregistered `ai:` token falls through the routing branch in
/// [`super::admit_system_commands`] to the `LocalShip`, which is exactly what
/// the player ship's Backfill AI wants. Registered NPCs never reach this
/// branch — they always carry an `EntityUuid`.
pub const AI_BACKFILL_TOKEN: &str = "ai:backfill";

/// Build the `ai:` token for one ship's AI operator: `ai:<uuid>` when the
/// entity carries an [`crate::entity_spawner::EntityUuid`], else
/// [`AI_BACKFILL_TOKEN`].
pub fn ai_token_for(entity_uuid: Option<&crate::entity_spawner::EntityUuid>) -> String {
    entity_uuid
        .map(|u| format!("ai:{}", u.0))
        .unwrap_or_else(|| AI_BACKFILL_TOKEN.to_string())
}

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
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    target: crate::messages::SystemId,
    payload: crate::messages::SystemControlPayload,
    sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
    admitted: &mut crate::messages::AdmittedCommands,
) -> bool {
    let token = ai_token_for(entity_uuid);
    let default_config;
    let config = match ship_config {
        Some(c) => &c.0,
        None => {
            default_config = crate::ship::config::ShipConfig {
                stations: vec![],
                systems: vec![],
                power_groups: std::collections::HashMap::new(),
                coordination_lag_secs: 0.0,
            };
            &default_config
        }
    };
    super::validate_and_admit(&token, target, payload, sources, sessions, config, admitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_token_uses_the_entity_uuid_when_present() {
        let uuid = crate::entity_spawner::EntityUuid("abc-123".to_string());
        assert_eq!(ai_token_for(Some(&uuid)), "ai:abc-123");
    }

    #[test]
    fn ai_token_falls_back_to_the_unregistered_backfill_token() {
        assert_eq!(ai_token_for(None), AI_BACKFILL_TOKEN);
        // The backfill token must stay unregistered-by-shape: admission routes
        // any `ai:` token it cannot resolve to the LocalShip.
        assert!(AI_BACKFILL_TOKEN.starts_with("ai:"));
    }
}

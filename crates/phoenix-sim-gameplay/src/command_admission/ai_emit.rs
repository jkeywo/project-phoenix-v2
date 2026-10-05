/// The prefix every AI operator's token carries, and the one thing that tells a
/// reader an admitted command came from an AI decision rather than from a seat.
///
/// Named here because it is already load-bearing in three places
/// ([`super::policy::is_command_authorized`] routes on it,
/// [`super::admit_system_commands`] resolves its route from it, and
/// `lobby::handler` exempts it from lobby identity), and because issue #1436's
/// crew-activity adapter is the first reader OUTSIDE admission to depend on it.
pub const AI_TOKEN_PREFIX: &str = "ai:";

/// Is this the token of an AI operator rather than a seat?
///
/// The complement is deliberately broad: a crew session token, the native local
/// console token, and the absent token a peer-relayed or GM-puppeted command
/// carries are all "not an AI decision". Advisory reads only — command authority
/// is decided by [`super::policy::is_command_authorized`], never by this.
pub fn is_ai_token(token: Option<&str>) -> bool {
    token.is_some_and(|token| token.starts_with(AI_TOKEN_PREFIX))
}

/// The token an AI operator on a ship with no [`crate::entities::spawner::EntityUuid`]
/// emits under.
///
/// Deliberately *not* registered in `crate::ai::server::AiTokenRegistry`: an
/// unregistered `ai:` token falls through the routing branch in
/// [`super::admit_system_commands`] to the `LocalShip`, which is exactly what
/// the player ship's Backfill AI wants. Registered NPCs never reach this
/// branch — they always carry an `EntityUuid`.
pub const AI_BACKFILL_TOKEN: &str = "ai:backfill";

/// Build the `ai:` token for one ship's AI operator: `ai:<uuid>` when the
/// entity carries an [`crate::entities::spawner::EntityUuid`], else
/// [`AI_BACKFILL_TOKEN`].
pub fn ai_token_for(entity_uuid: Option<&crate::entities::spawner::EntityUuid>) -> String {
    entity_uuid
        .map(|u| format!("ai:{}", u.0))
        .unwrap_or_else(|| AI_BACKFILL_TOKEN.to_string())
}

/// Authorize with the common policy and enqueue immediately in this ship's own queue.
pub fn emit_ai_command(
    entity_uuid: Option<&crate::entities::spawner::EntityUuid>,
    target: crate::core::messages::SystemId,
    payload: crate::core::messages::SystemControlPayload,
    sources: &crate::ship_plugin::ShipSystemControlSources,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
    admitted: &mut crate::core::messages::AdmittedCommands,
) -> bool {
    use phoenix_sim_contracts::authority::{
        authorize_command, CommandAuthorization, CommandClaimant, CommandTargetPolicy,
    };
    let empty = crate::ship::config::ShipConfig {
        stations: vec![],
        systems: vec![],
        power_groups: std::collections::HashMap::new(),
        coordination_lag_secs: 0.0,
    };
    let config = ship_config.map(|c| &c.0).unwrap_or(&empty);
    let effective =
        crate::command_admission::policy::effective_target_for_command(config, &target, &payload);
    if authorize_command(
        CommandClaimant::Ai,
        CommandTargetPolicy {
            control: sources.0.policy_for(&effective),
            summary: false,
            debug_route: false,
        },
    ) != CommandAuthorization::Allowed
    {
        return false;
    }
    admitted.0.push(crate::core::messages::AdmittedCommand {
        target,
        payload,
        response_token: Some(ai_token_for(entity_uuid)),
        feedback_correlation: None,
    });
    true
}

/// Basic authored Backfill decision. The applier and firing paths never know
/// who issued this ordinary command. Coordinated attack doctrine builds on it.
pub(crate) fn ai_strike_boost(
    sessions: bevy::prelude::Res<crate::lobby::Sessions>,
    mut ships: bevy::prelude::Query<
        (
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &crate::ship::power::ShipPowerSystem,
            &crate::ship::power::PowerConfigResource,
            &mut crate::core::messages::AdmittedCommands,
        ),
        bevy::prelude::With<crate::server_app::Ship>,
    >,
) {
    for (uuid, sources, ship, power, config, mut admitted) in &mut ships {
        let Some(boost) = power.0.strike_boost() else {
            continue;
        };
        let Some(threshold) = config
            .0
            .strike_reserve
            .as_ref()
            .and_then(|r| r.ai_enable_at)
        else {
            continue;
        };
        if !boost.enabled && power.0.battery_charge >= threshold {
            crate::command_admission::ai_emit::emit_ai_command(
                uuid,
                crate::core::messages::SystemId(
                    crate::ship::system_registry::PHASER_CONTROL_SYSTEM_ID.into(),
                ),
                crate::core::messages::SystemControlPayload::SetStrikeBoost { enabled: true },
                sources,
                &sessions,
                ship,
                &mut admitted,
            );
        }
    }
}

/// The ordinary admitted Gunnery command is the sole boost toggle writer.
pub(crate) fn handle_set_strike_boost(
    mut ships: bevy::prelude::Query<
        (
            &crate::core::messages::AdmittedCommands,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut crate::ship::power::ShipPowerSystem,
        ),
        bevy::prelude::With<crate::server_app::Ship>,
    >,
    mut outbound: Option<
        bevy::prelude::ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
) {
    use super::{WeaponActionRefusal, WeaponActionResult};
    use crate::core::messages::{SystemControlPayload, SystemId};
    let target = SystemId(crate::ship::system_registry::PHASER_CONTROL_SYSTEM_ID.into());
    for (admitted, sources, mut power) in &mut ships {
        for cmd in admitted.for_target(&target.0) {
            let SystemControlPayload::SetStrikeBoost { enabled } = cmd.payload else {
                continue;
            };
            let policy = sources.0.policy_for(&target);
            let result = if (policy.accept_human_input || policy.operate_ai)
                && power.0.set_strike_boost(enabled)
            {
                WeaponActionResult::Applied
            } else {
                WeaponActionResult::Refused(WeaponActionRefusal::Offline)
            };
            super::finish_action_feedback(cmd, &mut outbound, result);
        }
    }
}

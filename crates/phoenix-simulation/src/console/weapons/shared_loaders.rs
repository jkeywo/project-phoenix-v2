//! Correlated receipt for the existing paid magazine transaction, not a new executor.
use super::*;
use bevy::prelude::*;
pub(super) fn register(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        finish_load_receipts
            .in_set(crate::sim_sets::SimSet::Physics)
            .after(torpedo::handle_torpedo_magazine_inter_system),
    );
}
fn finish_load_receipts(
    ships: Query<
        (
            &crate::core::messages::AdmittedCommands,
            Option<&TorpedoSystemResource>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut outbound: Option<ResMut<Messages<crate::lobby::server::OutboundMessage>>>,
) {
    for (commands, torpedoes) in &ships {
        for command in &commands.0 {
            if !matches!(
                command.payload,
                crate::core::messages::SystemControlPayload::LoadTube
            ) {
                continue;
            }
            let started = torpedoes.is_some_and(|system| {
                system.0.tubes.iter().any(|tube| {
                    crate::ship::system_registry::torpedo_tube_system_id(&tube.id).as_ref()
                        == Some(&command.target)
                        && matches!(
                            tube.load_state,
                            crate::weapons::torpedo::TubeLoadState::Loading { .. }
                        )
                        && system.0.magazine.reserved.get(
                            &crate::weapons::torpedo::TorpedoSystem::magazine_claim_id(&tube.id),
                        ) == Some(&1)
                })
            });
            finish_action_feedback(
                command,
                &mut outbound,
                if started {
                    WeaponActionResult::Applied
                } else {
                    WeaponActionResult::Refused(WeaponActionRefusal::EmptyVolley)
                },
            );
        }
    }
}

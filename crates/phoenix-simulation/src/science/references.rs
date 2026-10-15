//! Deliberate correlation through one instrument's ordinary admission/knowledge.
use super::broad::{BroadProfile, BroadTracks};
use crate::{
    core::messages::{AdmittedCommands, SystemBlackboard, SystemControlPayload},
    server_app::ShipSystemBlackboards,
    ship_plugin::ShipConfigComponent,
};
use bevy::prelude::*;
pub use phoenix_sim_gameplay::science::references::{held_references, project};

pub(crate) fn register(app: &mut App) {
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    for kind in [
        crate::ship::system_registry::SENSOR_RADAR_KIND,
        crate::ship::system_registry::TACTICAL_RADAR_KIND,
        crate::ship::system_registry::HELM_RADAR_KIND,
    ] {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(kind, Payload::AttachSensorReference).with_feedback(
                FeedbackAddress::DeclaredKindOnly,
                &[Payload::AttachSensorReference],
            ),
        );
    }
    app.add_systems(
        FixedUpdate,
        apply
            .in_set(crate::sim_sets::SimSet::Modifiers)
            .after(super::sharing::deliver),
    );
    app.add_systems(
        FixedUpdate,
        publish
            .in_set(crate::sim_sets::SimSet::Publish)
            .in_set(crate::sim_sets::FixedStep::PublishSensorReferences)
            .after(crate::ship::sensors::publish_sensor_radar_blackboard)
            .after(crate::console::helm::publish_helm_blackboard)
            .after(crate::console::weapons::blackboard::publish_tactical_radar_blackboard)
            .after(crate::dossier::publish_dossier_blackboard),
    );
}

pub(crate) fn apply(
    tick: Res<crate::sim_tick::SimTick>,
    mut ships: Query<(
        &BroadProfile,
        &ShipConfigComponent,
        &AdmittedCommands,
        &ShipSystemBlackboards,
        &mut BroadTracks,
    )>,
    mut outbound: Option<ResMut<Messages<crate::lobby::OutboundMessage>>>,
) {
    for (profiles, config, admitted, boards, mut states) in &mut ships {
        for command in &admitted.0 {
            let SystemControlPayload::AttachSensorReference { contact, reference } =
                &command.payload
            else {
                continue;
            };
            let mut applied = false;
            if profiles.0.contains_key(&command.target) {
                let permitted = config.0.observations_for_system(&command.target, &boards.0);
                // This is a typed held feed; do not seek its original subject in
                // world entities, nor infer attachment from the source UUID.
                let held_contact = permitted
                    .get(&command.target)
                    .and_then(super::contact_routing::observations)
                    .is_some_and(|rows| rows.iter().any(|row| &row.contact_id == contact));
                if let Some(SystemBlackboard::Dossiers(dossiers)) = permitted
                    .get(&crate::dossier::dossier_blackboard_key())
                    .filter(|_| held_contact)
                {
                    if let Some(reference) = held_references(dossiers).get(reference) {
                        if reference.gathered_at_tick <= tick.0 {
                            if let Some(track) = states
                                .0
                                .get_mut(&command.target)
                                .and_then(|state| state.tracks.get_mut(contact))
                            {
                                applied = phoenix_sim_gameplay::science::references::attach(
                                    track,
                                    &reference.reference,
                                    tick.0,
                                )
                                .is_ok();
                            }
                        }
                    }
                }
            }
            if let Some(feedback) = crate::command_admission::feedback::admitted_action_feedback(
                command,
                if applied {
                    crate::core::messages::ActionFeedbackOutcome::Applied
                } else {
                    crate::core::messages::ActionFeedbackOutcome::Refused
                },
            ) {
                if let Some(outbound) = &mut outbound {
                    outbound.write(feedback);
                }
            }
        }
    }
}

/// Called on already-derived per-instrument track DTOs. The owning Console's
/// held feed is the only source; receiving another radar feed grants no library.
pub fn attach_views(
    config: &crate::ship::config::ShipConfig,
    boards: &ShipSystemBlackboards,
    owner: &crate::core::messages::SystemId,
    state: &phoenix_sim_gameplay::science::tracks::BroadTrackState,
    records: &mut [phoenix_model::sensing_tracks::SensorTrackSnapshot],
) {
    let permitted = config.observations_for_system(owner, &boards.0);
    let dossiers = match permitted.get(&crate::dossier::dossier_blackboard_key()) {
        Some(SystemBlackboard::Dossiers(dossiers)) => Some(dossiers),
        _ => None,
    };
    let routine_names = phoenix_sim_gameplay::science::references::routine_names(records);
    for record in records {
        record.references = match (state.tracks.get(&record.contact_id), dossiers) {
            (Some(track), Some(dossiers)) => project(
                track,
                dossiers,
                record
                    .identity
                    .as_ref()
                    .is_some_and(|identity| routine_names.contains(&identity.name)),
            ),
            _ => Vec::new(),
        };
    }
}

pub(crate) fn publish(
    mut ships: Query<(
        &ShipConfigComponent,
        &BroadTracks,
        &mut ShipSystemBlackboards,
    )>,
) {
    for (config, states, mut boards) in &mut ships {
        let mut next = Vec::new();
        for (owner, state) in &states.0 {
            let Some(board) = boards.0.get(owner) else {
                continue;
            };
            let mut board = board.clone();
            let rows = match &mut board {
                SystemBlackboard::SensorRadar(board) => board.tracks.as_mut(),
                SystemBlackboard::TacticalRadar(board) => board.tracks.as_mut(),
                SystemBlackboard::Helm(board) => board.tracks.as_mut(),
                _ => None,
            };
            if let Some(rows) = rows {
                attach_views(&config.0, &boards, owner, state, rows);
            }
            next.push((owner.clone(), board));
        }
        for (owner, board) in next {
            if boards.0.get(&owner) != Some(&board) {
                boards.0.insert(owner, board);
            }
        }
    }
}

//! Publication.
use super::*;

// -- Update systems ----------------------------------------------------------

/// Broadcast `ObjectiveSummary` when objectives change.
pub(crate) fn broadcast_objective_summary(
    local_ship: Query<&crate::entities::spawner::EntityUuid, With<crate::server_app::LocalShip>>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut objective_instances: Option<ResMut<ObjectiveInstanceManagerRes>>,
    mut outbox: ResMut<SimOutbox>,
) {
    if !objectives.0.is_dirty()
        && !objective_instances
            .as_ref()
            .is_some_and(|instances| instances.0.is_dirty())
    {
        return;
    }

    let ship_id = local_ship
        .iter()
        .next()
        .map(|uuid| uuid.0.as_str())
        .unwrap_or("");
    let legacy_snapshots = objectives.0.snapshots_for(ship_id);
    let objectives_snap = objective_instances
        .as_ref()
        .map(|instances| {
            instances
                .0
                .project_snapshots_for_ship(ship_id, legacy_snapshots.clone())
        })
        .unwrap_or(legacy_snapshots);

    outbox.push_reliable((
        Target::All,
        ServerMessage::ObjectiveSummary {
            objectives: objectives_snap,
        },
    ));

    objectives.0.mark_clean();
    if let Some(instances) = objective_instances.as_mut() {
        instances.0.mark_clean();
    }
}

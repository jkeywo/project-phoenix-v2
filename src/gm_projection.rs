//! Peer-local Game Master projection (issue #1291).
//!
//! This is a deliberately tiny read model over the authoritative ECS. It is
//! emitted only through the browser Host Channel; it is not a `ServerMessage`,
//! lockstep frame, or simulation outbox entry.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::console_bridge::GmEntityProjectionChanged;
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};

/// Marks the explicit production rendererless browser GM peer.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct BrowserGameMaster;

/// Authoritative status carried by the one-entity local projection.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmEntityStatus {
    pub hull_percent: u8,
    pub destroyed: bool,
}

/// Stable identity and status of the deterministic projection target.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmEntityProjection {
    pub entity_id: String,
    pub name: String,
    pub status: GmEntityStatus,
}

/// Absolute local projection. `entity: null` explicitly clears stale page state.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmEntityProjectionPayload {
    pub entity: Option<GmEntityProjection>,
}

pub struct GmProjectionPlugin;

impl Plugin for GmProjectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<GmEntityProjectionChanged>().add_systems(
            FixedLast,
            publish_local_projection.run_if(resource_exists::<BrowserGameMaster>),
        );
    }
}

fn publish_local_projection(
    entities: Query<(&EntityUuid, Option<&EntityName>, &EntitySystemHull)>,
    mut previous: Local<Option<GmEntityProjectionPayload>>,
    mut changed: MessageWriter<GmEntityProjectionChanged>,
) {
    let entity = entities
        .iter()
        .map(|(uuid, name, hull)| {
            let total_max = hull.0.total_max();
            let total_current = hull.0.total_current();
            let hull_percent = if total_max > 0.0 {
                ((total_current / total_max) * 100.0)
                    .clamp(0.0, 100.0)
                    .round() as u8
            } else {
                0
            };
            GmEntityProjection {
                entity_id: uuid.0.clone(),
                name: name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
                status: GmEntityStatus {
                    hull_percent,
                    destroyed: total_current <= 0.0,
                },
            }
        })
        .min_by(|left, right| left.entity_id.cmp(&right.entity_id));

    let next = GmEntityProjectionPayload { entity };
    if previous.as_ref() != Some(&next) {
        changed.write(GmEntityProjectionChanged {
            payload: next.clone(),
        });
        *previous = Some(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::SystemId;
    use crate::ship::damage::SystemHull;

    fn hull(hp: f32) -> EntitySystemHull {
        EntitySystemHull(SystemHull::from_config(&[(SystemId("captain".into()), hp)]))
    }

    fn take(app: &mut App) -> Vec<GmEntityProjectionPayload> {
        app.world_mut()
            .resource_mut::<Messages<GmEntityProjectionChanged>>()
            .drain()
            .map(|event| event.payload)
            .collect()
    }

    #[test]
    fn selects_the_lowest_stable_uuid_and_clears_or_reselects_after_removal() {
        let mut app = App::new();
        app.insert_resource(BrowserGameMaster)
            .add_plugins(GmProjectionPlugin);

        let high = app
            .world_mut()
            .spawn((
                EntityUuid("00000000-0000-0000-0000-000000000002".into()),
                EntityName("High".into()),
                hull(40.0),
            ))
            .id();
        let low = app
            .world_mut()
            .spawn((
                EntityUuid("00000000-0000-0000-0000-000000000001".into()),
                EntityName("Low".into()),
                hull(100.0),
            ))
            .id();

        app.world_mut().run_schedule(FixedLast);
        let first = take(&mut app);
        assert_eq!(first.len(), 1);
        let selected = first[0].entity.as_ref().expect("selected entity");
        assert_eq!(selected.entity_id, "00000000-0000-0000-0000-000000000001");
        assert_eq!(selected.status.hull_percent, 100);

        app.world_mut().despawn(low);
        app.world_mut().run_schedule(FixedLast);
        let second = take(&mut app);
        assert_eq!(second[0].entity.as_ref().unwrap().name, "High");

        app.world_mut().despawn(high);
        app.world_mut().run_schedule(FixedLast);
        assert_eq!(
            take(&mut app),
            vec![GmEntityProjectionPayload { entity: None }]
        );
    }
}

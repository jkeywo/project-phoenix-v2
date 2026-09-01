//! Peer-local Game Master map projection (issues #1291 and #1295).
//!
//! This is a deliberately narrow read model over the authoritative ECS. It is
//! emitted only through the browser Host Channel; it is not a `ServerMessage`,
//! lockstep frame, or simulation outbox entry. Every browser GM builds it from
//! the deterministic world already running in that peer.

use std::collections::BTreeMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::console::weapons::TacticalRadarSelection;
use crate::console_bridge::GmEntityProjectionChanged;
use crate::entities::config_cache::FactionRegistryResource;
use crate::entities::spawner::{
    EntityName, EntitySystemHull, EntityUuid, FactionComponent, StaticPointDefence,
};
use crate::lockstep::FleetSlotOf;
use crate::server_app::Ship;
use crate::ship::state::ShipPhysics;

/// Marks the explicit production rendererless browser GM peer.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct BrowserGameMaster;

/// Player/NPC classification projected without exposing ECS marker names.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GmEntityKind {
    PlayerShip,
    NpcShip,
}

/// Authoritative broad status carried by the local projection.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmEntityStatus {
    pub hull_percent: u8,
    pub destroyed: bool,
}

/// Stable reference used for faction and current-target links.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GmEntityReference {
    pub entity_id: String,
    pub name: String,
}

/// Stable identity plus the small M1 map/inspector surface for one ship.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct GmEntityProjection {
    pub entity_id: String,
    pub name: String,
    pub kind: GmEntityKind,
    pub position: [f32; 3],
    pub faction: Option<GmEntityReference>,
    pub status: GmEntityStatus,
    pub current_target: Option<GmEntityReference>,
}

/// Absolute local map projection. An empty list explicitly clears stale page
/// state when every selectable ship has left the world.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct GmEntityProjectionPayload {
    pub entities: Vec<GmEntityProjection>,
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

type GmShipProjectionQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        Option<&'static EntityName>,
        &'static EntitySystemHull,
        &'static ShipPhysics,
        Option<&'static FactionComponent>,
        Option<&'static FleetSlotOf>,
        Option<&'static TacticalRadarSelection>,
    ),
    (With<Ship>, Without<StaticPointDefence>),
>;

fn hull_status(hull: &EntitySystemHull) -> GmEntityStatus {
    let total_max = hull.0.total_max();
    let total_current = hull.0.total_current();
    let hull_percent = if total_max > 0.0 {
        ((total_current / total_max) * 100.0)
            .clamp(0.0, 100.0)
            .round() as u8
    } else {
        0
    };
    GmEntityStatus {
        hull_percent,
        destroyed: total_current <= 0.0,
    }
}

fn publish_local_projection(
    entities: GmShipProjectionQuery,
    factions: Option<Res<FactionRegistryResource>>,
    mut previous: Local<Option<GmEntityProjectionPayload>>,
    mut changed: MessageWriter<GmEntityProjectionChanged>,
) {
    // Resolve names in a separate deterministic lookup so target links never
    // leak a Bevy `Entity` and remain useful after a projection refresh.
    let names: BTreeMap<String, String> = entities
        .iter()
        .map(|(uuid, name, ..)| {
            (
                uuid.0.clone(),
                name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
            )
        })
        .collect();

    let mut projected: Vec<GmEntityProjection> = entities
        .iter()
        .map(|(uuid, name, hull, physics, faction, fleet_slot, target)| {
            let faction = faction.map(|faction| {
                let name = factions
                    .as_deref()
                    .and_then(|registry| registry.get(&faction.0))
                    .and_then(|config| config.display_name.clone())
                    .unwrap_or_else(|| faction.0.to_string());
                GmEntityReference {
                    entity_id: faction.0.to_string(),
                    name,
                }
            });
            let current_target =
                target
                    .and_then(|selection| selection.0.as_ref())
                    .map(|target_id| GmEntityReference {
                        entity_id: target_id.clone(),
                        name: names
                            .get(target_id)
                            .cloned()
                            .unwrap_or_else(|| target_id.clone()),
                    });
            GmEntityProjection {
                entity_id: uuid.0.clone(),
                name: name.map_or_else(|| uuid.0.clone(), |name| name.0.clone()),
                kind: if fleet_slot.is_some() {
                    GmEntityKind::PlayerShip
                } else {
                    GmEntityKind::NpcShip
                },
                position: [physics.x, physics.y, physics.z],
                faction,
                status: hull_status(hull),
                current_target,
            }
        })
        .collect();
    projected.sort_by(|left, right| left.entity_id.cmp(&right.entity_id));

    let next = GmEntityProjectionPayload {
        entities: projected,
    };
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
    use crate::ai::faction::{FactionConfig, FactionRegistry};
    use crate::command_admission::HostSlot;
    use crate::core::messages::SystemId;
    use crate::ship::damage::SystemHull;
    use uuid::Uuid;

    const PLAYER_ID: &str = "00000000-0000-4000-8000-000000000001";
    const NPC_ID: &str = "00000000-0000-4000-8000-000000000002";
    const FACTION_ID: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";

    fn hull(percent: f32) -> EntitySystemHull {
        let mut hull = SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);
        hull.apply_damage(100.0 - percent, &mut crate::sim_rng::unseeded_test_rng());
        EntitySystemHull(hull)
    }

    fn take(app: &mut App) -> Vec<GmEntityProjectionPayload> {
        app.world_mut()
            .resource_mut::<Messages<GmEntityProjectionChanged>>()
            .drain()
            .map(|event| event.payload)
            .collect()
    }

    fn app() -> App {
        let faction_uuid = Uuid::parse_str(FACTION_ID).unwrap();
        let mut factions = FactionRegistry::new();
        factions.insert(FactionConfig {
            uuid: faction_uuid,
            name: "Alliance".into(),
            display_name: Some("faction.alliance.display_name".into()),
            enemies: Vec::new(),
            compliance: None,
        });
        let mut app = App::new();
        app.insert_resource(BrowserGameMaster)
            .insert_resource(FactionRegistryResource(factions))
            .add_plugins(GmProjectionPlugin);
        app
    }

    #[test]
    fn projects_player_and_npc_in_stable_order_with_position_faction_health_and_target() {
        let mut app = app();
        let faction = FactionComponent(Uuid::parse_str(FACTION_ID).unwrap());
        app.world_mut().spawn((
            Ship,
            EntityUuid(NPC_ID.into()),
            EntityName("Raider".into()),
            hull(40.0),
            ShipPhysics {
                x: 80.0,
                y: 2.0,
                z: -30.0,
                ..Default::default()
            },
            faction.clone(),
            TacticalRadarSelection(Some(PLAYER_ID.into())),
        ));
        app.world_mut().spawn((
            Ship,
            EntityUuid(PLAYER_ID.into()),
            EntityName("Cruiser".into()),
            hull(100.0),
            ShipPhysics {
                x: -12.0,
                z: 7.0,
                ..Default::default()
            },
            faction,
            FleetSlotOf(HostSlot::SOLO),
            TacticalRadarSelection(Some(NPC_ID.into())),
        ));

        app.world_mut().run_schedule(FixedLast);
        let payload = take(&mut app).pop().expect("first absolute projection");
        assert_eq!(payload.entities.len(), 2);
        assert_eq!(payload.entities[0].entity_id, PLAYER_ID);
        assert_eq!(payload.entities[0].kind, GmEntityKind::PlayerShip);
        assert_eq!(payload.entities[0].position, [-12.0, 0.0, 7.0]);
        assert_eq!(payload.entities[0].status.hull_percent, 100);
        assert_eq!(
            payload.entities[0].faction.as_ref().unwrap().name,
            "faction.alliance.display_name"
        );
        assert_eq!(
            payload.entities[0].current_target,
            Some(GmEntityReference {
                entity_id: NPC_ID.into(),
                name: "Raider".into(),
            })
        );
        assert_eq!(payload.entities[1].kind, GmEntityKind::NpcShip);
        assert_eq!(payload.entities[1].position, [80.0, 2.0, -30.0]);
        assert_eq!(payload.entities[1].status.hull_percent, 40);
        assert!(!payload.entities[1].status.destroyed);
    }

    #[test]
    fn republishes_updates_and_removal_but_ignores_non_ship_entities() {
        let mut app = app();
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                EntityUuid(NPC_ID.into()),
                EntityName("Raider".into()),
                hull(40.0),
                ShipPhysics::default(),
                TacticalRadarSelection::default(),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid("00000000-0000-4000-8000-000000000099".into()),
            EntityName("Not a ship".into()),
            hull(100.0),
            ShipPhysics::default(),
        ));

        app.world_mut().run_schedule(FixedLast);
        assert_eq!(take(&mut app)[0].entities.len(), 1);
        app.world_mut().run_schedule(FixedLast);
        assert!(
            take(&mut app).is_empty(),
            "unchanged absolute state is deduped"
        );

        app.world_mut()
            .entity_mut(npc)
            .get_mut::<ShipPhysics>()
            .unwrap()
            .x = 5.0;
        app.world_mut()
            .entity_mut(npc)
            .get_mut::<TacticalRadarSelection>()
            .unwrap()
            .0 = Some("removed-target".into());
        app.world_mut().run_schedule(FixedLast);
        let updated = take(&mut app).pop().unwrap();
        assert_eq!(updated.entities[0].position[0], 5.0);
        assert_eq!(
            updated.entities[0].current_target.as_ref().unwrap().name,
            "removed-target",
            "a stale target remains a stable link without exposing ECS state"
        );

        app.world_mut().despawn(npc);
        app.world_mut().run_schedule(FixedLast);
        assert!(take(&mut app).pop().unwrap().entities.is_empty());
    }

    #[test]
    fn excludes_static_point_defence_even_when_it_carries_the_ship_substrate() {
        let mut app = app();
        app.world_mut().spawn((
            Ship,
            StaticPointDefence,
            EntityUuid("00000000-0000-4000-8000-000000000003".into()),
            EntityName("Axiom station".into()),
            hull(100.0),
            ShipPhysics::default(),
            TacticalRadarSelection::default(),
        ));

        app.world_mut().run_schedule(FixedLast);
        let payload = take(&mut app)
            .pop()
            .expect("first absolute projection must clear stale browser state");
        assert!(
            payload.entities.is_empty(),
            "a point-defence structure must not be mislabeled as an NPC ship"
        );
    }
}

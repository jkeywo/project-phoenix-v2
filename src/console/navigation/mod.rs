use bevy::prelude::*;

use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, SystemControlPayload, WaypointSnapshot};
use crate::ship::control_source::ControlTickPolicy;
use crate::ship::system_registry::{navigation_system_id, NAVIGATION_SYSTEM_ID};

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NavigationWaypoint>()
            .add_systems(
                Update,
                handle_navigation_waypoint.in_set(crate::sim_sets::SimSet::Input),
            )
            // Refresh anchored waypoints from the parent entity's live
            // Transform every tick, before the broadcaster reads the
            // waypoint into the SimSnapshot. Auto-clear when the parent
            // entity is no longer present.
            .add_systems(
                Update,
                refresh_anchored_waypoint.in_set(crate::sim_sets::SimSet::Modifiers),
            )
            .add_systems(
                Update,
                operate_navigation_ai.in_set(crate::sim_sets::SimSet::Physics),
            );
    }
}

/// Authoritative navigation waypoint state.
///
/// Stores either a free position chosen by tap-to-place, or an entity-anchored
/// waypoint that follows the named entity's transform until the entity
/// despawns.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct NavigationWaypoint(pub Option<WaypointMode>);

/// Storage variant of the navigation waypoint.
#[derive(Clone, Debug, PartialEq)]
pub enum WaypointMode {
    /// Tap-to-place: the waypoint is a fixed world position and never moves.
    Free { x: f32, z: f32 },
    /// Anchored to an entity by UUID. `last_x` / `last_z` mirror the entity's
    /// last-known transform; they are refreshed each tick by
    /// [`refresh_anchored_waypoint`]. When the parent entity is no longer
    /// present, the waypoint is auto-cleared.
    Anchored {
        source_uuid: String,
        last_x: f32,
        last_z: f32,
    },
}

impl NavigationWaypoint {
    /// Returns the broadcast-shaped snapshot for the current waypoint, or
    /// `None` if no waypoint is set.
    pub fn snapshot(&self) -> Option<WaypointSnapshot> {
        match &self.0 {
            None => None,
            Some(WaypointMode::Free { x, z }) => Some(WaypointSnapshot {
                x: *x,
                z: *z,
                source_uuid: None,
            }),
            Some(WaypointMode::Anchored {
                source_uuid,
                last_x,
                last_z,
            }) => Some(WaypointSnapshot {
                x: *last_x,
                z: *last_z,
                source_uuid: Some(source_uuid.clone()),
            }),
        }
    }
}

fn navigation_authorized(
    sessions: &Sessions,
    ship_config: &crate::ship_plugin::ShipConfigComponent,
    token: &str,
) -> bool {
    sessions
        .0
        .console_holder(&Console::Navigation, &ship_config.0)
        == Some(token)
}

/// Handle waypoint messages from the Navigation console.
///
/// Accepts `ClientMessage::ControlSystem { target: "navigation", payload: ... }`.
/// All messages are gated on:
///
/// 1. `ControlSourceResolver::policy_for(&navigation_system_id()).accept_human_input`
///    (rejects when the system is under AI control)
/// 2. Sender holds `Console::Navigation`.
fn handle_navigation_waypoint(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<(&crate::ship_plugin::ShipConfigComponent, &crate::ship_plugin::ShipSystemControlSources), With<crate::simulation::Ship>>,
    mut waypoint: ResMut<NavigationWaypoint>,
) {
    let Ok((ship_config, control_sources)) = ship_query.single() else {
        return;
    };

    let policy: ControlTickPolicy = control_sources.0.policy_for(&navigation_system_id());

    if !policy.accept_human_input {
        return;
    }

    for ev in reader.read() {
        if !navigation_authorized(&sessions, ship_config, &ev.token) {
            continue;
        }

        match &ev.msg {
            ClientMessage::ControlSystem { target, payload }
                if target.0 == NAVIGATION_SYSTEM_ID =>
            {
                match payload {
                    SystemControlPayload::SetNavigationWaypoint { x, z, source_uuid }
                        if x.is_finite() && z.is_finite() =>
                    {
                        waypoint.0 = Some(make_waypoint_mode(*x, *z, source_uuid.as_deref()));
                    }
                    SystemControlPayload::ClearNavigationWaypoint => {
                        waypoint.0 = None;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Build the appropriate `WaypointMode` from raw coordinates and an optional
/// anchor UUID. An empty UUID string is treated as "no anchor" (free waypoint).
fn make_waypoint_mode(x: f32, z: f32, source_uuid: Option<&str>) -> WaypointMode {
    match source_uuid {
        Some(uuid) if !uuid.is_empty() => WaypointMode::Anchored {
            source_uuid: uuid.to_string(),
            last_x: x,
            last_z: z,
        },
        _ => WaypointMode::Free { x, z },
    }
}

/// Each tick, if the navigation waypoint is anchored to an entity, look up
/// the entity's current `Transform` by `EntityUuid` and refresh the
/// waypoint's stored coordinates. If no entity carries the anchored UUID,
/// auto-clear the waypoint (per the despawn policy).
fn refresh_anchored_waypoint(
    mut waypoint: ResMut<NavigationWaypoint>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform)>,
) {
    let Some(WaypointMode::Anchored {
        source_uuid,
        last_x,
        last_z,
    }) = waypoint.0.as_mut()
    else {
        return;
    };

    let mut found = false;
    for (uuid, transform) in entity_q.iter() {
        if uuid.0 == *source_uuid {
            *last_x = transform.translation.x;
            *last_z = transform.translation.z;
            found = true;
            break;
        }
    }

    if !found {
        // Parent entity has despawned (or never existed). Auto-clear.
        waypoint.0 = None;
    }
}


// ── AI controller stub ─────────────────────────────────────────────────────────

/// Per-kind AI plugin for navigation.
///
/// Gated on policy.operate_ai for the Navigation system. No behaviour is
/// implemented yet — this is a compile-verified stub that will be filled in
/// when the Navigation AI controller is designed.
fn operate_navigation_ai(
    ships: Query<(&crate::ship_plugin::ShipSystemControlSources,), With<crate::simulation::Ship>>,
) {
    for (sources,) in &ships {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        if !policy.operate_ai {
            continue;
        }
        // TODO: implement navigation AI logic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::ServerMessage;
    use crate::server_app::{
        sim_state_broadcaster, LastBroadcastEntityPositions, LastBroadcastHull,
        LastBroadcastShields, ShipHullIntegrity, ShipImpulse,
    };
    use crate::ship_state::ShipState;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut sink: ResMut<Outbox>) {
        for msg in reader.read() {
            sink.0.push(msg.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            // Chain SimSet phases so handle (Input) → refresh (Modifiers) →
            // broadcast (Broadcast) run in the right order. Without this,
            // adding a second resource-touching system to a different set
            // makes the schedule non-deterministic and breaks the existing
            // broadcast assertions.
            .configure_sets(
                Update,
                (
                    crate::sim_sets::SimSet::Input,
                    crate::sim_sets::SimSet::Physics,
                    crate::sim_sets::SimSet::Damage,
                    crate::sim_sets::SimSet::Modifiers,
                    crate::sim_sets::SimSet::Broadcast,
                )
                    .chain(),
            )
            .add_plugins(NavigationPlugin)
            .add_plugins(sim_state_broadcaster())
            .init_resource::<crate::simulation::SimOutbox>()
            .init_resource::<Outbox>()
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[(
                Console::Navigation,
                25.0,
            )])))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(crate::modifiers::ShipModifiers::new())
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .add_systems(PostUpdate, collect);
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let out = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game_with_navigation(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push(
            app,
            "navigation",
            ClientMessage::Identify {
                token: "navigation".into(),
                name: "Decker".into(),
            },
        );
        tick(app);
        push(
            app,
            "navigation",
            ClientMessage::SelectStation {
                station: "Navigation".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "navigation", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn latest_sim_snapshot(out: &[OutboundMessage]) -> Option<crate::messages::SimSnapshot> {
        out.iter().rev().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        })
    }

    #[test]
    fn navigation_holder_can_set_and_clear_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 120.0,
                    z: -45.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<NavigationWaypoint>().0,
            Some(WaypointMode::Free { x: 120.0, z: -45.0 })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<NavigationWaypoint>().0.is_none());
    }

    #[test]
    fn non_navigation_sender_cannot_change_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 6.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<NavigationWaypoint>().0.is_none());
    }

    #[test]
    fn invalid_waypoint_coordinates_are_ignored() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: f32::NAN,
                    z: 1.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<NavigationWaypoint>().0.is_none());
    }

    #[test]
    fn sim_state_broadcast_includes_and_omits_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 10.0,
                    z: 20.0,
                    source_uuid: None,
                },
            },
        );
        let out = tick(&mut app);
        let snap = latest_sim_snapshot(&out).expect("expected SimState");
        assert_eq!(
            snap.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 10.0,
                z: 20.0,
                source_uuid: None,
            })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        let out = tick(&mut app);
        let snap = latest_sim_snapshot(&out).expect("expected SimState");
        assert!(snap.navigation_waypoint.is_none());
    }

    #[test]
    fn anchored_waypoint_tracks_moving_entity() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        // Spawn an entity carrying EntityUuid + Transform that the waypoint
        // will anchor to.
        let target_uuid = "target-1";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(50.0, 0.0, -100.0),
            ))
            .id();

        // Anchor the waypoint to that entity. The seed coords are the
        // entity's current position.
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 50.0,
                    z: -100.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        let out = tick(&mut app);
        let snap = latest_sim_snapshot(&out).expect("expected SimState");
        assert_eq!(
            snap.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 50.0,
                z: -100.0,
                source_uuid: Some(target_uuid.into()),
            })
        );

        // Move the entity. The next broadcast should reflect the new
        // position with source_uuid preserved.
        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_xyz(75.0, 0.0, -150.0));
        let out = tick(&mut app);
        let snap = latest_sim_snapshot(&out).expect("expected SimState");
        assert_eq!(
            snap.navigation_waypoint,
            Some(WaypointSnapshot {
                x: 75.0,
                z: -150.0,
                source_uuid: Some(target_uuid.into()),
            })
        );
    }

    #[test]
    fn anchored_waypoint_auto_clears_when_parent_despawns() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        let target_uuid = "target-despawn";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(10.0, 0.0, 20.0),
            ))
            .id();

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 10.0,
                    z: 20.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<NavigationWaypoint>().0.is_some());

        // Despawn the parent entity. The next tick must auto-clear.
        app.world_mut().entity_mut(target).despawn();
        let out = tick(&mut app);
        assert!(app.world().resource::<NavigationWaypoint>().0.is_none());
        let snap = latest_sim_snapshot(&out).expect("expected SimState");
        assert!(snap.navigation_waypoint.is_none());
    }

    #[test]
    fn empty_source_uuid_is_treated_as_free_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 1.0,
                    z: 2.0,
                    source_uuid: Some(String::new()),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<NavigationWaypoint>().0,
            Some(WaypointMode::Free { x: 1.0, z: 2.0 })
        );
    }

    // ── ControlSystem dispatch tests ─────────────────────────────────────────

    /// Navigation holder sends `ControlSystem` waypoint — accepted.
    #[test]
    fn control_system_navigation_holder_can_set_and_clear_waypoint() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 200.0,
                    z: -80.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<NavigationWaypoint>().0,
            Some(WaypointMode::Free { x: 200.0, z: -80.0 })
        );

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<NavigationWaypoint>().0.is_none());
    }

    /// Non-navigation sender sends `ControlSystem` waypoint — rejected.
    #[test]
    fn control_system_unauthorized_sender_rejected() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 6.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(
            app.world().resource::<NavigationWaypoint>().0.is_none(),
            "non-navigation sender should be rejected"
        );
    }

    /// When navigation system is AI-controlled, `ControlSystem` waypoint is rejected.
    #[test]
    fn control_system_rejected_when_ai_controlled() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::simulation::Ship>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::ship::system_registry::navigation_system_id(),
                    crate::ship::control_source::ControlSource::Ai,
                );
            }
        }

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 99.0,
                    z: 99.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert!(
            app.world().resource::<NavigationWaypoint>().0.is_none(),
            "should reject waypoint when navigation is AI-controlled"
        );
    }

    /// Anchored waypoint set via `ControlSystem` still tracks the entity.
    #[test]
    fn control_system_anchored_waypoint_tracks_entity() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        let target_uuid = "anchor-cs-test";
        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid(target_uuid.into()),
                Transform::from_xyz(30.0, 0.0, -60.0),
            ))
            .id();

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 30.0,
                    z: -60.0,
                    source_uuid: Some(target_uuid.into()),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<NavigationWaypoint>().0,
            Some(WaypointMode::Anchored {
                source_uuid: target_uuid.into(),
                last_x: 30.0,
                last_z: -60.0,
            })
        );

        // Move entity — next tick should update last_x/last_z.
        app.world_mut()
            .entity_mut(target)
            .insert(Transform::from_xyz(40.0, 0.0, -70.0));
        tick(&mut app);
        assert_eq!(
            app.world().resource::<NavigationWaypoint>().0,
            Some(WaypointMode::Anchored {
                source_uuid: target_uuid.into(),
                last_x: 40.0,
                last_z: -70.0,
            })
        );
    }

    #[test]
    fn control_system_set_navigation_waypoint_works() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 15.0,
                    z: 25.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<NavigationWaypoint>().0,
            Some(WaypointMode::Free { x: 15.0, z: 25.0 })
        );
    }

    #[test]
    fn control_system_clear_navigation_waypoint_works() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::SetNavigationWaypoint {
                    x: 5.0,
                    z: 5.0,
                    source_uuid: None,
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "navigation",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("navigation".into()),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<NavigationWaypoint>().0.is_none());
    }
}

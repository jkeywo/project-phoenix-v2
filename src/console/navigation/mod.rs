use bevy::prelude::*;

use crate::messages::{
    AdmittedCommands, NavigationBlackboard, SystemBlackboard, SystemControlPayload, SystemId,
    WaypointSnapshot,
};
use crate::ship::system_registry::NAVIGATION_SYSTEM_ID;

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
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
        )
        .add_systems(
            Update,
            publish_navigation_blackboard.in_set(crate::sim_sets::SimSet::Publish),
        );
    }
}

/// Authoritative navigation waypoint state.
///
/// Stores either a free position chosen by tap-to-place, or an entity-anchored
/// waypoint that follows the named entity's transform until the entity
/// despawns.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own waypoint.
#[derive(Component, Default, Clone, Debug, PartialEq)]
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

fn handle_navigation_waypoint(
    mut ship_query: Query<
        (&AdmittedCommands, &mut NavigationWaypoint),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, mut waypoint) in ship_query.iter_mut() {
        for cmd in admitted.for_target(NAVIGATION_SYSTEM_ID) {
            match &cmd.payload {
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

/// Each tick, if any ship's navigation waypoint is anchored to an entity,
/// look up the entity's current `Transform` by `EntityUuid` and refresh
/// the waypoint's stored coordinates. If no entity carries the anchored
/// UUID, auto-clear the waypoint (per the despawn policy). Iterates every
/// ship so both player and NPC waypoints track their anchors.
fn refresh_anchored_waypoint(
    mut waypoint_q: Query<&mut NavigationWaypoint, With<crate::server_app::Ship>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform)>,
) {
    for mut waypoint in waypoint_q.iter_mut() {
        let Some(WaypointMode::Anchored {
            source_uuid,
            last_x,
            last_z,
        }) = waypoint.0.as_mut()
        else {
            continue;
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
}

// ── Blackboard publish ────────────────────────────────────────────────────────

fn publish_navigation_blackboard(
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    waypoint_q: Query<&NavigationWaypoint, With<crate::server_app::LocalShip>>,
    mut ship_bbs_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    let cfg = &ship_config.0;
    let navigation_waypoint = waypoint_q.single().ok().and_then(|w| w.snapshot());
    let bb = NavigationBlackboard {
        nav_chart_range: cfg.nav_chart_range,
        nav_chart_shows: cfg.nav_chart_shows.clone(),
        nav_chart_selects: cfg.nav_chart_selects.clone(),
        navigation_waypoint,
    };

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(NAVIGATION_SYSTEM_ID.to_string()),
            SystemBlackboard::Navigation(bb),
        );
    }
}

// ── AI controller stub ─────────────────────────────────────────────────────────

/// Per-entity AI loop for navigation. Loops over ALL ship entities (player and NPC)
/// where the Navigation system is `ControlSource::Ai`.
///
/// Currently a compile-verified stub — Navigation AI sets waypoints based on
/// doctrine objectives (deferred to later fine-grained decomposition).
pub fn operate_navigation_ai(ships: Query<&crate::ship_plugin::ShipSystemControlSources>) {
    for sources in &ships {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        if !policy.operate_ai {
            continue;
        }
        // TODO: implement navigation AI logic (set waypoints from doctrine objectives)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::{ClientMessage, ServerMessage};
    use crate::server_app::{
        sim_state_broadcaster, LastBroadcastEntityPositions, LastBroadcastHull,
        LastBroadcastShields, ShipImpulse,
    };

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
            .add_plugins(crate::server_app::AdmissionPlugin)
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
                    crate::sim_sets::SimSet::Publish,
                    crate::sim_sets::SimSet::PublishAggregate,
                    crate::sim_sets::SimSet::Broadcast,
                )
                    .chain(),
            )
            .init_resource::<crate::server_app::LastBroadcastBlackboards>()
            .init_resource::<crate::lobby::server::ShipClientConfigResource>()
            .add_plugins(NavigationPlugin)
            .add_plugins(sim_state_broadcaster())
            .add_plugins(crate::server_app::sim_outbox_broadcaster())
            .init_resource::<crate::simulation::SimOutbox>()
            .add_systems(
                Update,
                crate::server_app::broadcast_blackboard_updates
                    .in_set(crate::sim_sets::SimSet::PublishAggregate),
            )
            .init_resource::<Outbox>()
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(crate::modifiers::ShipModifiers::new())
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .add_systems(PostUpdate, collect);
        // Spawn the player ship entity so handle_navigation_waypoint can query it.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::simulation::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            // PR 7 (issue #597) — NavigationWaypoint is now a per-entity Component.
            NavigationWaypoint::default(),
        ));
        app
    }

    /// PR 7 test helper — read the LocalShip's `NavigationWaypoint` component.
    fn get_nav_waypoint(app: &mut App) -> Option<WaypointMode> {
        let mut q = app
            .world_mut()
            .query_filtered::<&NavigationWaypoint, With<crate::server_app::LocalShip>>();
        q.single(app.world()).ok().and_then(|w| w.0.clone())
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

    fn latest_navigation_blackboard(
        out: &[OutboundMessage],
    ) -> Option<crate::messages::NavigationBlackboard> {
        out.iter().rev().find_map(|m| match &m.msg {
            ServerMessage::BlackboardUpdate { updates } => {
                updates.iter().find_map(|(_, bb)| match bb {
                    crate::messages::SystemBlackboard::Navigation(nav) => Some(nav.clone()),
                    _ => None,
                })
            }
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
            get_nav_waypoint(&mut app),
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
        assert!(get_nav_waypoint(&mut app).is_none());
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
        assert!(get_nav_waypoint(&mut app).is_none());
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
        assert!(get_nav_waypoint(&mut app).is_none());
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
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
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
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert!(bb.navigation_waypoint.is_none());
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
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
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
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert_eq!(
            bb.navigation_waypoint,
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
        assert!(get_nav_waypoint(&mut app).is_some());

        // Despawn the parent entity. The next tick must auto-clear.
        app.world_mut().entity_mut(target).despawn();
        let out = tick(&mut app);
        assert!(get_nav_waypoint(&mut app).is_none());
        let bb = latest_navigation_blackboard(&out).expect("expected NavigationBlackboard");
        assert!(bb.navigation_waypoint.is_none());
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
            get_nav_waypoint(&mut app),
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
            get_nav_waypoint(&mut app),
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
        assert!(get_nav_waypoint(&mut app).is_none());
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
            get_nav_waypoint(&mut app).is_none(),
            "non-navigation sender should be rejected"
        );
    }

    /// When navigation system is AI-controlled, `ControlSystem` waypoint is rejected.
    #[test]
    fn control_system_rejected_when_ai_controlled() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);

        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
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
            get_nav_waypoint(&mut app).is_none(),
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
            get_nav_waypoint(&mut app),
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
            get_nav_waypoint(&mut app),
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
            get_nav_waypoint(&mut app),
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
        assert!(get_nav_waypoint(&mut app).is_none());
    }

    /// Verifies operate_navigation_ai runs per-entity for AI-controlled ships (issue #592 AC).
    #[test]
    fn operate_navigation_ai_per_entity_ai_gate() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        use crate::ship_plugin::ShipSystemControlSources;

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::navigation_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let policy = ai_sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        assert!(
            policy.operate_ai,
            "AI Navigation must gate through operate_ai"
        );

        // Human-controlled navigation must not operate AI.
        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::navigation_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::navigation_system_id());
        assert!(
            !human_policy.operate_ai,
            "Human Navigation must not operate AI"
        );
    }
}

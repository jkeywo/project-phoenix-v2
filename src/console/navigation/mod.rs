use bevy::prelude::*;

use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, WaypointSnapshot};

pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NavigationWaypoint>().add_systems(
            Update,
            handle_navigation_waypoint.in_set(crate::sim_sets::SimSet::Input),
        );
    }
}

#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub struct NavigationWaypoint(pub Option<WaypointSnapshot>);

fn navigation_authorized(sessions: &Sessions, token: &str) -> bool {
    sessions.0.console_holder(Console::Navigation) == Some(token)
}

fn handle_navigation_waypoint(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut waypoint: ResMut<NavigationWaypoint>,
) {
    for ev in reader.read() {
        match &ev.msg {
            ClientMessage::SetNavigationWaypoint { x, z }
                if navigation_authorized(&sessions, &ev.token) && x.is_finite() && z.is_finite() =>
            {
                waypoint.0 = Some(WaypointSnapshot { x: *x, z: *z });
            }
            ClientMessage::ClearNavigationWaypoint
                if navigation_authorized(&sessions, &ev.token) =>
            {
                waypoint.0 = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::{GamePhase, ServerMessage};
    use crate::server_app::{sim_state_broadcaster, ShipImpulse};
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
            .add_plugins(NavigationPlugin)
            .add_plugins(sim_state_broadcaster())
            .init_resource::<Outbox>()
            .insert_resource(ShipState::new())
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .add_systems(PostUpdate, collect);
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage { token: token.into(), msg });
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
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
        app.world_mut().insert_resource(GamePhase::InProgress);
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
            ClientMessage::SetNavigationWaypoint { x: 120.0, z: -45.0 },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<NavigationWaypoint>().0,
            Some(WaypointSnapshot { x: 120.0, z: -45.0 })
        );

        push(&mut app, "navigation", ClientMessage::ClearNavigationWaypoint);
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
            ClientMessage::SetNavigationWaypoint { x: 5.0, z: 6.0 },
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
            ClientMessage::SetNavigationWaypoint {
                x: f32::NAN,
                z: 1.0,
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
            ClientMessage::SetNavigationWaypoint { x: 10.0, z: 20.0 },
        );
        let out = tick(&mut app);
        let snap = latest_sim_snapshot(&out).expect("expected SimState");
        assert_eq!(
            snap.navigation_waypoint,
            Some(WaypointSnapshot { x: 10.0, z: 20.0 })
        );

        push(&mut app, "navigation", ClientMessage::ClearNavigationWaypoint);
        let out = tick(&mut app);
        let snap = latest_sim_snapshot(&out).expect("expected SimState");
        assert!(snap.navigation_waypoint.is_none());
    }
}

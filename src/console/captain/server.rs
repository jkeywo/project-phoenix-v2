use bevy::prelude::*;

use crate::lobby::{CurrentPhase, InboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::ship_state::ShipState;

pub struct CaptainPlugin;

impl Plugin for CaptainPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (
            handle_toggle_red_alert.in_set(crate::sim_sets::SimSet::Input),
            handle_set_view.in_set(crate::sim_sets::SimSet::Input),
        ));
    }
}

fn handle_toggle_red_alert(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if matches!(ev.msg, ClientMessage::ToggleRedAlert)
            && sessions.0.console_holder(Console::CaptainChair) == Some(ev.token.as_str())
        {
            ship.toggle_red_alert();
        }
    }
}

fn handle_set_view(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if let ClientMessage::SetView { mode } = ev.msg.clone() {
            // Authorization is per-variant: Camera views are the captain's call,
            // Radar is the helm's call. A request from the wrong console is
            // silently ignored.
            let required = match &mode {
                ViewMode::Camera(_) => Console::CaptainChair,
                ViewMode::Radar => Console::Helm,
                ViewMode::ScienceRadar | ViewMode::SensorsRadar => Console::Sensors,
                ViewMode::SystemChart | ViewMode::NavigationChart => Console::Navigation,
                ViewMode::Comms => Console::Comms,
            };
            if sessions.0.console_holder(required) == Some(ev.token.as_str()) {
                ship.view_mode = mode;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::ViewDirection;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(CaptainPlugin)
            .init_resource::<Outbox>()
            .insert_resource(ShipState::new())
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

    fn start_game(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    // ── ToggleRedAlert tests ────────────────────────────────────────────────

    #[test]
    fn toggle_red_alert_during_lobby_is_ignored() {
        let mut app = test_app();
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(&mut app);
        // Still in Lobby — game not started
        push(&mut app, "captain", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(!app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn non_captain_toggle_red_alert_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "crew", ClientMessage::Identify { token: "crew".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "crew", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(!app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn captain_toggle_red_alert_works() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn captain_toggle_red_alert_twice_returns_to_off() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(app.world().resource::<ShipState>().red_alert());
        push(&mut app, "captain", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(!app.world().resource::<ShipState>().red_alert());
    }

    // ── SetView tests ───────────────────────────────────────────────────────

    #[test]
    fn set_view_during_lobby_is_ignored() {
        let mut app = test_app();
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(&mut app);
        // Still in Lobby — game not started
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn non_captain_set_view_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "crew", ClientMessage::Identify { token: "crew".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "crew", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Port) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn captain_set_view_changes_direction() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Aft)
        );
    }
}

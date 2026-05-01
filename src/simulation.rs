use bevy::prelude::*;

use crate::lobby::{CaptainToken, CurrentPhase, InboundMessage, OutboundMessage, Target};
use crate::messages::{ClientMessage, GamePhase, ServerMessage, SimSnapshot};

// ── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct RedAlertState {
    pub active: bool,
}

#[derive(Resource)]
struct SimBroadcastTimer(Timer);

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RedAlertState { active: false })
            .insert_resource(SimBroadcastTimer(Timer::from_seconds(
                0.1,
                TimerMode::Repeating,
            )))
            .add_systems(Update, (handle_toggle, broadcast_sim_state));
    }
}

// ── Systems ────────────────────────────────────────────────────────────────

fn handle_toggle(
    mut reader: MessageReader<InboundMessage>,
    mut red_alert: ResMut<RedAlertState>,
    captain: Res<CaptainToken>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if matches!(ev.msg, ClientMessage::ToggleRedAlert)
            && captain.0.as_deref() == Some(ev.token.as_str())
        {
            red_alert.active = !red_alert.active;
        }
    }
}

fn broadcast_sim_state(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    mut writer: MessageWriter<OutboundMessage>,
    red_alert: Res<RedAlertState>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if timer.0.tick(time.delta()).just_finished() {
        writer.write(OutboundMessage {
            target: Target::All,
            msg: ServerMessage::SimState {
                snapshot: SimSnapshot { red_alert: red_alert.active },
            },
        });
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::LobbyPlugin;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::time::TimePlugin, LobbyPlugin, SimulationPlugin));
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage { token: token.into(), msg });
    }

    fn tick(app: &mut App) {
        app.update();
    }

    fn red_alert(app: &App) -> bool {
        app.world().resource::<RedAlertState>().active
    }

    fn setup_in_progress(app: &mut App) {
        push(app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(app);
        push(app, "t1", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn captain_can_toggle_red_alert_on() {
        let mut app = test_app();
        setup_in_progress(&mut app);
        assert!(!red_alert(&app));
        push(&mut app, "t1", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(red_alert(&app));
    }

    #[test]
    fn captain_can_toggle_red_alert_off() {
        let mut app = test_app();
        setup_in_progress(&mut app);
        push(&mut app, "t1", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(!red_alert(&app));
    }

    #[test]
    fn non_captain_cannot_toggle_red_alert() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::Identify { token: "t2".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::StartGame);
        tick(&mut app);
        push(&mut app, "t2", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(!red_alert(&app));
    }

    #[test]
    fn toggle_ignored_while_in_lobby() {
        let mut app = test_app();
        push(&mut app, "t1", ClientMessage::Identify { token: "t1".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "t1", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(!red_alert(&app));
    }
}

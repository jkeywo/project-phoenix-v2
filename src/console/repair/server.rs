use bevy::prelude::*;

use crate::lobby::{InboundMessage, Sessions};
use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{ClientMessage, Console, ServerMessage};
use crate::repair_teams::RepairTeams;
use crate::simulation::{ShipHullIntegrity, SimOutbox};
use crate::modifiers::ShipModifiers;
use crate::messages::ModifierSlot;

// ── Resources ─────────────────────────────────────────────────────────────────

/// Bevy resource wrapping the pure `RepairTeams` state machine.
#[derive(Resource)]
pub struct ShipRepairTeams(pub RepairTeams);

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShipRepairTeams(RepairTeams::default()))
            .add_systems(Update, (
                handle_dispatch_repair_team.in_set(crate::sim_sets::SimSet::Input),
                tick_repair_teams.in_set(crate::sim_sets::SimSet::Physics),
            ))
            .add_plugins(repair_state_broadcaster());
    }
}

// ── Broadcaster ───────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `RepairState` producer.
///
/// Broadcasts `RepairState` at 10 Hz to the `Repair` console holder only.
/// Registered by [`RepairPlugin`].
pub fn repair_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::Holding(Console::Repair),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let teams = world.resource::<ShipRepairTeams>();
            let slots = teams.0.slots().to_vec();
            vec![ServerMessage::RepairState { teams: slots }]
        },
    )
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Handle `DispatchRepairTeam { console }` messages from the Repair console.
///
/// Validates: game is in-progress, sender holds `Console::Repair`.
/// - If no free team exists: message ignored.
/// - Otherwise: dispatch the lowest-numbered free team to the named console.
pub fn handle_dispatch_repair_team(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut teams: ResMut<ShipRepairTeams>,
) {
    for ev in reader.read() {
        let target_console = match &ev.msg {
            ClientMessage::DispatchRepairTeam { console } => console.clone(),
            _ => continue,
        };
        // Only the Repair console holder may dispatch teams.
        let Some(repair_token) = sessions.0.console_holder(Console::Repair) else {
            continue;
        };
        if ev.token.as_str() != repair_token {
            continue;
        }
        // Must have a free team.
        let Some(team_idx) = teams.0.lowest_free_team() else {
            continue;
        };
        teams.0.dispatch(team_idx, target_console);
    }
}

/// Tick repair teams each frame: advance timers and apply HP restoration.
pub fn tick_repair_teams(
    time: Res<Time>,
    mut teams: ResMut<ShipRepairTeams>,
    mut hull: ResMut<ShipHullIntegrity>,
    modifiers: Res<ShipModifiers>,
) {
    let dt = time.delta_secs();
    let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
    teams.0.tick(dt * repair_mult, &mut hull.0);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::simulation::{ShipImpulse, ShipShields};
    use crate::shield::ShieldSystem;

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
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource(crate::ship_state::ShipState::new())
            .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                (Console::Helm, 25.0),
                (Console::Tactical, 25.0),
                (Console::Power, 25.0),
                (Console::Shields, 25.0),
            ])))
            .insert_resource(ShipShields(ShieldSystem::default()))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(crate::modifiers::ShipModifiers::new())
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .add_plugins(RepairPlugin)
            .add_plugins(repair_state_broadcaster())
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
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage { target, msg });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "eng", ClientMessage::Identify { token: "eng".into(), name: "Bob".into() });
        tick(app);
        push(app, "eng", ClientMessage::SelectStation { station: "Repair".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn team_is_travelling(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Travelling { .. })
    }

    fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Idle)
    }

    // ── Dispatch tests ──────────────────────────────────────────────────────

    /// Non-Repair console holder sending `DispatchRepairTeam` is ignored.
    #[test]
    fn non_repair_sender_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);

        push(&mut app, "captain", ClientMessage::DispatchRepairTeam { console: Console::Helm });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_idle(&teams, 0), "team 0 should remain idle after non-Repair dispatch");
    }

    /// Repair holder dispatches team to a console → team enters Travelling.
    #[test]
    fn dispatch_sends_team_to_travelling() {
        let mut app = test_app();
        start_game(&mut app);

        push(&mut app, "eng", ClientMessage::DispatchRepairTeam { console: Console::Helm });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_travelling(&teams, 0), "team 0 should be travelling after dispatch");
    }

    /// When all teams are busy, further dispatches are ignored.
    #[test]
    fn all_busy_teams_ignore_further_dispatches() {
        let mut app = test_app();
        start_game(&mut app);

        // Dispatch both teams (default is 2).
        push(&mut app, "eng", ClientMessage::DispatchRepairTeam { console: Console::Helm });
        tick(&mut app);
        push(&mut app, "eng", ClientMessage::DispatchRepairTeam { console: Console::Tactical });
        tick(&mut app);

        // Third dispatch — no free team.
        push(&mut app, "eng", ClientMessage::DispatchRepairTeam { console: Console::Power });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        // Both teams still busy (Travelling), third dispatch was a no-op.
        assert!(team_is_travelling(&teams, 0));
        assert!(team_is_travelling(&teams, 1));
    }

    /// RepairState broadcast includes the team slot states.
    #[test]
    fn repair_state_broadcast_includes_team_slots() {
        let mut app = test_app();
        start_game(&mut app);

        push(&mut app, "eng", ClientMessage::DispatchRepairTeam { console: Console::Helm });
        let out1 = tick(&mut app);
        let out2 = tick(&mut app);

        let has_repair_state = out1.iter().chain(out2.iter()).any(|m| {
            matches!(&m.msg, ServerMessage::RepairState { teams } if
                teams.iter().any(|t| matches!(t, crate::messages::TeamSlot::Travelling { .. })))
        });
        assert!(has_repair_state, "RepairState should include a Travelling team after dispatch");
    }
}

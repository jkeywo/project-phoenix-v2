use bevy::prelude::*;

use crate::console_bridge::ConsoleStateChanged;
use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::ModifierSlot;
use crate::messages::{
    ClientMessage, Console, ConsoleHullStatus, RepairConsoleState, RepairTarget, ServerMessage,
    SystemControlPayload, TeamSlot,
};
use crate::modifiers::ShipModifiers;
use crate::repair_teams::RepairTeams;
use crate::ship::system_registry::{repair_system_id, REPAIR_SYSTEM_ID};
use crate::ship_plugin::ShipSystemControlSources;
use crate::simulation::ShipHullIntegrity;

// ── Resources ─────────────────────────────────────────────────────────────────

/// Bevy resource wrapping the pure `RepairTeams` state machine.
#[derive(Resource)]
pub struct ShipRepairTeams(pub RepairTeams);

// ── Console state component ────────────────────────────────────────────────────

/// Bevy component that caches the current `RepairConsoleState` for the HTML panel.
///
/// Spawned once at `Startup`, recomputed each broadcast frame, and pushed via
/// `ConsoleStateChanged` whenever the value changes.
#[derive(Component, Clone, PartialEq)]
pub struct RepairConsoleStateComp(pub RepairConsoleState);

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ConsoleStateChanged>();
        app.insert_resource(ShipRepairTeams(RepairTeams::default()))
            .add_systems(Startup, spawn_repair_console_state_entity)
            .add_systems(
                Update,
                (
                    handle_dispatch_repair_team.in_set(crate::sim_sets::SimSet::Input),
                    tick_repair_teams.in_set(crate::sim_sets::SimSet::Physics),
                    recompute_repair_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
                    push_repair_console_state
                        .in_set(crate::sim_sets::SimSet::Broadcast)
                        .after(recompute_repair_console_state),
                ),
            )
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

/// Handle `DispatchRepairTeam` messages from the Repair console.
///
/// Accepts both the legacy `ClientMessage::DispatchRepairTeam` path and the
/// new `ClientMessage::ControlSystem { target: "repair", payload:
/// DispatchRepairTeam { .. } }` path. Both are gated on:
///
/// 1. `ControlSourceResolver::policy_for(&repair_system_id()).accept_human_input`
///    (rejects when the system is under AI control)
/// 2. Sender holds `Console::Repair`.
///
/// `RepairTarget::Core` is accepted by the wire but has no runtime effect yet
/// (per-core repair targeting is tracked by PRD C slice C7).
pub fn handle_dispatch_repair_team(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_config: Res<crate::ship_plugin::ShipConfigResource>,
    control_sources: Res<ShipSystemControlSources>,
    mut teams: ResMut<ShipRepairTeams>,
) {
    let policy = control_sources.0.policy_for(&repair_system_id());

    for ev in reader.read() {
        // Extract (team_idx, target_console) from either message form.
        let (team_idx, target_console): (usize, Console) = match &ev.msg {
            // ── Legacy path ───────────────────────────────────────────────
            ClientMessage::DispatchRepairTeam { team_idx, console } => {
                (*team_idx as usize, console.clone())
            }
            // ── ControlSystem path ────────────────────────────────────────
            ClientMessage::ControlSystem { target, payload } if target.0 == REPAIR_SYSTEM_ID => {
                match payload {
                    SystemControlPayload::DispatchRepairTeam { team_idx, target } => {
                        let console = match target {
                            RepairTarget::Station(station_id) => {
                                match Console::from_console_id(&station_id.0) {
                                    Some(c) => c,
                                    None => continue, // unknown station id
                                }
                            }
                            RepairTarget::Core => Console::Core,
                        };
                        (*team_idx as usize, console)
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };

        // Gate: reject if the repair system is under AI control.
        if !policy.accept_human_input {
            continue;
        }

        // Gate: only the Repair console holder may dispatch teams.
        let Some(repair_token) = sessions.0.console_holder(&Console::Repair, &ship_config.0) else {
            warn!(
                "[repair-auth] ignored repair action from token={} holder=None",
                ev.token,
            );
            continue;
        };
        if ev.token.as_str() != repair_token {
            warn!(
                "[repair-auth] ignored repair action from token={} holder={}",
                ev.token, repair_token,
            );
            continue;
        }

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

// ── HTML console state push ────────────────────────────────────────────────────

pub fn spawn_repair_console_state_entity(mut commands: Commands) {
    commands.spawn(RepairConsoleStateComp(RepairConsoleState::default()));
}

/// Recompute `RepairConsoleStateComp` from live resources each broadcast frame.
pub fn recompute_repair_console_state(
    teams: Res<ShipRepairTeams>,
    hull: Res<ShipHullIntegrity>,
    mut q: Query<&mut RepairConsoleStateComp>,
) {
    let team_slots: Vec<TeamSlot> = teams.0.slots().to_vec();

    let console_hull: Vec<ConsoleHullStatus> = hull
        .0
        .entries()
        .iter()
        .map(|(c, cur, max)| ConsoleHullStatus {
            console: c.clone(),
            current: *cur,
            max_hp: *max,
        })
        .collect();

    let damageable_consoles: Vec<Console> =
        hull.0.entries().iter().map(|(c, _, _)| c.clone()).collect();

    let next = RepairConsoleState {
        teams: team_slots,
        console_hull,
        travel_duration_secs: teams.0.timings().travel_duration,
        damageable_consoles,
    };

    for mut comp in q.iter_mut() {
        if comp.0 != next {
            comp.0 = next.clone();
        }
    }
}

/// Push `RepairConsoleState` as a `ConsoleStateChanged` whenever it changes.
pub fn push_repair_console_state(
    q: Query<&RepairConsoleStateComp, Changed<RepairConsoleStateComp>>,
    mut writer: MessageWriter<ConsoleStateChanged>,
) {
    for comp in q.iter() {
        if let Ok(json) = crate::core::codec::encode_console_state(&comp.0) {
            writer.write(ConsoleStateChanged {
                name: "Repair".into(),
                json,
            });
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::shield::ShieldSystem;
    use crate::ship_plugin::ShipSystemControlSources;
    use crate::simulation::SimOutbox;
    use crate::simulation::{ShipImpulse, ShipShields};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.configure_sets(
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
        .add_plugins(LobbyPlugin)
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
            (Console::Core, 50.0),
        ])))
        .insert_resource(ShipShields(ShieldSystem::default()))
        .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
        .insert_resource(crate::modifiers::ShipModifiers::new())
        .init_resource::<crate::lobby::WorldResource>()
        .init_resource::<SimOutbox>()
        .init_resource::<ShipSystemControlSources>()
        .init_resource::<Outbox>()
        .add_plugins(RepairPlugin)
        .add_plugins(repair_state_broadcaster())
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
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut out = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            out.push(OutboundMessage { target, msg });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn start_game(app: &mut App) {
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
            "eng",
            ClientMessage::Identify {
                token: "eng".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "eng",
            ClientMessage::SelectStation {
                station: "Repair".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "eng", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn team_is_travelling(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(
            teams.0.slots()[idx],
            crate::messages::TeamSlot::Travelling { .. }
        )
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

        push(
            &mut app,
            "captain",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_idle(teams, 0),
            "team 0 should remain idle after non-Repair dispatch"
        );
    }

    /// Repair holder dispatches team to a console → team enters Travelling.
    #[test]
    fn dispatch_sends_team_to_travelling() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
            "team 0 should be travelling after dispatch"
        );
    }

    /// When team is busy, dispatching to a different console redirects it.
    #[test]
    fn all_busy_teams_ignore_further_dispatches() {
        let mut app = test_app();
        start_game(&mut app);

        // Dispatch both teams (default is 2).
        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 1,
                console: Console::Tactical,
            },
        );
        tick(&mut app);

        // Redirect team 0 to Power (different console) — now team 0 is Returning with queue
        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Power,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        // team 0 should be Returning (redirected), team 1 still Travelling
        assert!(matches!(
            &teams.0.slots()[0],
            crate::messages::TeamSlot::Returning { .. }
        ));
        assert!(team_is_travelling(teams, 1));
    }

    /// RepairState broadcast includes the team slot states.
    #[test]
    fn repair_state_broadcast_includes_team_slots() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Helm,
            },
        );
        let out1 = tick(&mut app);
        let out2 = tick(&mut app);

        let has_repair_state = out1.iter().chain(out2.iter()).any(|m| {
            matches!(&m.msg, ServerMessage::RepairState { teams } if
                teams.iter().any(|t| matches!(t, crate::messages::TeamSlot::Travelling { .. })))
        });
        assert!(
            has_repair_state,
            "RepairState should include a Travelling team after dispatch"
        );
    }

    // ── ControlSystem dispatch tests ─────────────────────────────────────────

    /// Repair holder dispatches via `ControlSystem` → team enters Travelling.
    #[test]
    fn control_system_dispatch_authorized_sends_team_to_travelling() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
            "team 0 should be travelling after ControlSystem dispatch"
        );
    }

    /// Non-Repair console holder sending `ControlSystem` dispatch is rejected.
    #[test]
    fn control_system_dispatch_unauthorized_sender_is_rejected() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_idle(teams, 0),
            "team 0 should remain idle when non-Repair sender uses ControlSystem"
        );
    }

    /// `ControlSystem` dispatch is blocked when the repair system is AI-controlled.
    #[test]
    fn control_system_dispatch_rejected_when_ai_controlled() {
        let mut app = test_app();
        start_game(&mut app);

        // Set repair system to AI control.
        {
            let mut cs = app.world_mut().resource_mut::<ShipSystemControlSources>();
            cs.0.set(
                crate::ship::system_registry::repair_system_id(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Station(StationId("helm".into())),
                },
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_idle(teams, 0),
            "team 0 should remain idle when repair system is AI-controlled"
        );
    }

    #[test]
    fn control_system_dispatch_repair_target_core_dispatches_team() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("repair".into()),
                payload: SystemControlPayload::DispatchRepairTeam {
                    team_idx: 0,
                    target: RepairTarget::Core,
                },
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
            "team 0 should be travelling to Core after RepairTarget::Core dispatch"
        );
    }

    /// Legacy `ClientMessage::DispatchRepairTeam` still works.
    #[test]
    fn legacy_dispatch_still_works_after_control_system_migration() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "eng",
            ClientMessage::DispatchRepairTeam {
                team_idx: 0,
                console: Console::Tactical,
            },
        );
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(
            team_is_travelling(teams, 0),
            "legacy DispatchRepairTeam should still dispatch team 0"
        );
    }

    /// End-to-end TOML-driven wiring check: build the runtime `RepairTeams`
    /// the same way `spawn_game_start_entities` does (parse player_ship.toml
    /// → RepairConfig::to_runtime → RepairTeams::new_with_timings) and
    /// assert the timings match the TOML. Changing
    /// `travel_duration_secs = 5.0` to e.g. `99.0` in player_ship.toml
    /// would fail this test.
    #[test]
    fn repair_teams_resource_reflects_player_ship_toml_repair_block() {
        let toml_str = include_str!("../../../assets/entities/player_ship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("player_ship.toml must parse");
        let rc = config.repair.expect("player_ship must declare [repair]");
        let timings = rc.to_runtime();
        let teams = crate::repair_teams::RepairTeams::new_with_timings(2, timings);
        assert_eq!(teams.timings().travel_duration, rc.travel_duration_secs);
        assert_eq!(
            teams.timings().repair_rate_hp_per_sec,
            rc.repair_rate_hp_per_sec
        );
        // And the runtime defaults still match (until someone intentionally
        // diverges them).
        let baseline = crate::repair_teams::RepairTimings::default();
        assert_eq!(teams.timings().travel_duration, baseline.travel_duration);
        assert_eq!(
            teams.timings().repair_rate_hp_per_sec,
            baseline.repair_rate_hp_per_sec
        );
    }

    // ── HTML push tests ─────────────────────────────────────────────────────

    #[derive(Resource, Default)]
    struct PushOutbox(Vec<ConsoleStateChanged>);

    fn collect_pushes(
        mut reader: MessageReader<ConsoleStateChanged>,
        mut box_: ResMut<PushOutbox>,
    ) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn push_test_app() -> App {
        let mut app = App::new();
        app.add_message::<ConsoleStateChanged>()
            .insert_resource(ShipRepairTeams(RepairTeams::default()))
            .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                (Console::Helm, 25.0),
                (Console::Tactical, 25.0),
                (Console::Power, 25.0),
            ])))
            .insert_resource(crate::modifiers::ShipModifiers::new())
            .init_resource::<PushOutbox>()
            .add_systems(Startup, spawn_repair_console_state_entity)
            .add_systems(
                Update,
                (
                    recompute_repair_console_state,
                    push_repair_console_state.after(recompute_repair_console_state),
                    collect_pushes.after(push_repair_console_state),
                ),
            );
        app
    }

    #[test]
    fn push_emits_repair_console_state_on_first_update() {
        let mut app = push_test_app();
        app.update();

        let pushes = &app.world().resource::<PushOutbox>().0;
        assert!(
            !pushes.is_empty(),
            "expected at least one ConsoleStateChanged on startup"
        );
        let push = pushes
            .iter()
            .find(|p| p.name == "Repair")
            .expect("expected push named 'Repair'");
        assert!(
            push.json.contains("\"teams\""),
            "json should contain teams: {}",
            push.json
        );
        assert!(
            push.json.contains("\"console_hull\""),
            "json should contain console_hull: {}",
            push.json
        );
        assert!(
            push.json.contains("\"travel_duration_secs\""),
            "json should contain travel_duration_secs: {}",
            push.json
        );
    }

    #[test]
    fn push_emits_on_dispatch_and_not_without_change() {
        let mut app = push_test_app();
        // First update: spawned component is Changed → push fires.
        app.update();
        app.world_mut().resource_mut::<PushOutbox>().0.clear();

        // No state change → no push.
        app.update();
        assert!(
            app.world().resource::<PushOutbox>().0.is_empty(),
            "no push expected when state has not changed"
        );

        // Dispatch team 0 → state changes → push fires.
        app.world_mut()
            .resource_mut::<ShipRepairTeams>()
            .0
            .dispatch(0, Console::Helm);
        app.update();
        let pushes = &app.world().resource::<PushOutbox>().0;
        assert!(!pushes.is_empty(), "expected push after dispatch");
        let push = pushes.iter().find(|p| p.name == "Repair").unwrap();
        assert!(
            push.json.contains("Travelling"),
            "Travelling state should appear in json: {}",
            push.json
        );
    }

    #[test]
    fn push_json_contains_damageable_consoles() {
        let mut app = push_test_app();
        app.update();

        let pushes = &app.world().resource::<PushOutbox>().0;
        let push = pushes.iter().find(|p| p.name == "Repair").unwrap();
        assert!(
            push.json.contains("\"damageable_consoles\""),
            "json should contain damageable_consoles: {}",
            push.json
        );
        assert!(
            push.json.contains("\"Helm\""),
            "Helm should appear in damageable_consoles: {}",
            push.json
        );
        assert!(
            push.json.contains("\"Tactical\""),
            "Tactical should appear in damageable_consoles: {}",
            push.json
        );
    }
}

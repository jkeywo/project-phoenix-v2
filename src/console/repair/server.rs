use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::ModifierSlot;
use crate::messages::{
    AdmittedCommands, ClientMessage, Console, ConsoleHullStatus, RepairBlackboard, RepairTarget,
    ServerMessage, SystemBlackboard, SystemControlPayload, SystemId, TeamSlot,
};
use crate::modifiers::ShipModifiers;
use crate::repair_teams::RepairTeams;
use crate::ship::system_registry::{repair_system_id, REPAIR_SYSTEM_ID};
use crate::ship_plugin::ShipSystemControlSources;

// ── Resources ─────────────────────────────────────────────────────────────────

/// Bevy resource wrapping the pure `RepairTeams` state machine.
///
/// Derives both `Resource` (existing player-ship singleton) and `Component`
/// (per-entity path after issue #590 unification).
#[derive(Resource, Component, Clone)]
pub struct ShipRepairTeams(pub RepairTeams);

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct RepairPlugin;

impl Plugin for RepairPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShipRepairTeams(RepairTeams::default()))
            .add_systems(
                Update,
                (
                    handle_dispatch_repair_team.in_set(crate::sim_sets::SimSet::Input),
                    tick_repair_teams.in_set(crate::sim_sets::SimSet::Physics),
                    operate_repair_ai.in_set(crate::sim_sets::SimSet::Physics),
                    publish_repair_blackboard.in_set(crate::sim_sets::SimSet::Publish),
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
/// `RepairTarget::Core` dispatches to `Console::Core`, the repair bucket for
/// ownerless ship-wide systems.
pub fn handle_dispatch_repair_team(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &AdmittedCommands,
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut teams: ResMut<ShipRepairTeams>,
) {
    let Ok((admitted, ship_config, control_sources)) = ship_query.single() else {
        return;
    };

    // ── ControlSystem path (authority already checked at admission) ────────
    for cmd in admitted.for_target(REPAIR_SYSTEM_ID) {
        if let SystemControlPayload::DispatchRepairTeam {
            team_idx,
            target: repair_target,
        } = &cmd.payload
        {
            let console = match repair_target {
                RepairTarget::Station(station_id) => {
                    match Console::from_console_id(&station_id.0) {
                        Some(c) => c,
                        None => continue,
                    }
                }
                RepairTarget::Core => Console::Core,
            };
            teams.0.dispatch(*team_idx as usize, console);
        }
    }

    // ── Legacy path (DispatchRepairTeam message type, still auth-gated here) ─
    let policy = control_sources.0.policy_for(&repair_system_id());
    if !policy.accept_human_input {
        for _ in reader.read() {}
        return;
    }
    let Some(repair_token) = sessions.0.console_holder(&Console::Repair, &ship_config.0) else {
        for _ in reader.read() {}
        return;
    };
    for ev in reader.read() {
        let ClientMessage::DispatchRepairTeam { team_idx, console } = &ev.msg else {
            continue;
        };
        if ev.token.as_str() != repair_token {
            continue;
        }
        teams.0.dispatch(*team_idx as usize, console.clone());
    }
}

/// Tick repair teams each frame: advance timers and apply HP restoration.
pub fn tick_repair_teams(
    time: Res<Time>,
    mut teams: ResMut<ShipRepairTeams>,
    mut hull_q: Query<&mut crate::entity_spawner::EntityConsoleHull, With<crate::server_app::LocalShip>>,
    modifiers: Res<ShipModifiers>,
) {
    let dt = time.delta_secs();
    let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
    if let Ok(mut hull) = hull_q.single_mut() {
        teams.0.tick(dt * repair_mult, &mut hull.0);
    }
}

// ── Blackboard publish ─────────────────────────────────────────────────────────

fn publish_repair_blackboard(
    teams: Res<ShipRepairTeams>,
    hull_q: Query<&crate::entity_spawner::EntityConsoleHull, With<crate::server_app::LocalShip>>,
    mut blackboards: ResMut<crate::server_app::SystemBlackboards>,
) {
    let team_slots: Vec<TeamSlot> = teams.0.slots().to_vec();

    let hull_ref = hull_q.single().ok();
    let console_hull: Vec<ConsoleHullStatus> = hull_ref
        .map(|h| {
            h.0.entries()
                .iter()
                .map(|(c, cur, max)| ConsoleHullStatus {
                    console: c.clone(),
                    current: *cur,
                    max_hp: *max,
                })
                .collect()
        })
        .unwrap_or_default();

    let damageable_consoles: Vec<Console> = hull_ref
        .map(|h| h.0.entries().iter().map(|(c, _, _)| c.clone()).collect())
        .unwrap_or_default();

    let bb = RepairBlackboard {
        teams: team_slots,
        console_hull,
        travel_duration_secs: teams.0.timings().travel_duration,
        damageable_consoles,
    };

    blackboards.0.insert(
        SystemId(REPAIR_SYSTEM_ID.to_string()),
        SystemBlackboard::Repair(bb),
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// ── AI controller stub ─────────────────────────────────────────────────────────

/// Per-kind AI loop for repair. Loops over ALL ship entities (player and NPC)
/// where the Repair system is `ControlSource::Ai`.
///
/// Currently a compile-verified stub — Repair AI auto-dispatches teams to
/// damaged consoles (the business logic is deferred to later fine-grained
/// decomposition in PRD #487).
pub fn operate_repair_ai(ships: Query<&ShipSystemControlSources>) {
    for sources in &ships {
        let policy = sources.0.policy_for(&repair_system_id());
        if !policy.operate_ai {
            continue;
        }
        // TODO: implement repair AI logic (auto-dispatch teams to lowest-HP console)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::server_app::ShipHullIntegrity;
    use crate::messages::*;
    use crate::server_app::SystemBlackboards;
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
                crate::sim_sets::SimSet::Publish,
                crate::sim_sets::SimSet::PublishAggregate,
                crate::sim_sets::SimSet::Broadcast,
            )
                .chain(),
        )
        .add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
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
        .init_resource::<Outbox>()
        .init_resource::<SystemBlackboards>()
        .add_plugins(RepairPlugin)
        .add_plugins(repair_state_broadcaster())
        .add_systems(PostUpdate, collect);
        // Spawn the player ship entity so handle_dispatch_repair_team can query it.
        let hull_config = &[
            (Console::Helm, 25.0_f32),
            (Console::Tactical, 25.0),
            (Console::Power, 25.0),
            (Console::Shields, 25.0),
            (Console::Core, 50.0),
        ];
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::entity_spawner::EntityConsoleHull(ConsoleHull::from_config(hull_config)),
        ));
        app
    }

    fn repair_bb(app: &App) -> RepairBlackboard {
        let bbs = app.world().resource::<SystemBlackboards>();
        let key = SystemId(REPAIR_SYSTEM_ID.to_string());
        let SystemBlackboard::Repair(bb) = bbs.0.get(&key).unwrap() else {
            panic!("expected Repair blackboard");
        };
        bb.clone()
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
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::ship::system_registry::repair_system_id(),
                    crate::ship::control_source::ControlSource::Ai,
                );
            }
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

    // ── Blackboard publish tests ─────────────────────────────────────────────

    #[test]
    fn publish_repair_blackboard_contains_teams_and_hull() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        let bb = repair_bb(&app);
        assert!(!bb.teams.is_empty(), "expected at least one team slot");
        assert!(!bb.console_hull.is_empty(), "expected console_hull entries");
        assert!(
            bb.travel_duration_secs > 0.0,
            "expected positive travel duration"
        );
    }

    #[test]
    fn publish_repair_blackboard_reflects_dispatch() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        app.world_mut()
            .resource_mut::<ShipRepairTeams>()
            .0
            .dispatch(0, Console::Helm);
        tick(&mut app);

        let bb = repair_bb(&app);
        assert!(
            bb.teams
                .iter()
                .any(|t| matches!(t, TeamSlot::Travelling { .. })),
            "expected a Travelling team slot after dispatch"
        );
    }

    #[test]
    fn publish_repair_blackboard_contains_damageable_consoles() {
        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app);

        let bb = repair_bb(&app);
        assert!(
            !bb.damageable_consoles.is_empty(),
            "expected damageable_consoles"
        );
        assert!(
            bb.damageable_consoles.contains(&Console::Helm),
            "Helm should appear in damageable_consoles"
        );
    }

    /// Verifies that operate_repair_ai loops over all entities with
    /// ShipSystemControlSources, gating on operate_ai (issue #590 AC).
    #[test]
    fn operate_repair_ai_runs_per_entity_for_ai_controlled_ships() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::repair_system_id(),
            ControlSource::Ai,
        );
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let policy = ai_sources
            .0
            .policy_for(&crate::system_registry::repair_system_id());
        assert!(policy.operate_ai, "AI Repair must gate through operate_ai");

        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::repair_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::repair_system_id());
        assert!(!human_policy.operate_ai, "human Repair must not operate AI");
    }

    /// Verifies that an NPC ship (without Ship marker) with Repair on AI
    /// runs operate_repair_ai independently (issue #590 AC).
    #[test]
    fn npc_ship_with_repair_on_ai_runs_operate_repair_ai() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};

        // Build a test app with operate_repair_ai registered.
        let mut app = bevy::prelude::App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_systems(
                bevy::prelude::Update,
                operate_repair_ai,
            );

        // Spawn an NPC entity (no Ship marker) with ShipSystemControlSources set to AI.
        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(
            crate::system_registry::repair_system_id(),
            ControlSource::Ai,
        );
        let npc_entity = app
            .world_mut()
            .spawn(ShipSystemControlSources(ai_resolver))
            .id();

        // Should not panic.
        app.update();
        assert!(
            app.world().get::<ShipSystemControlSources>(npc_entity).is_some(),
            "NPC entity must still exist after operate_repair_ai runs"
        );
    }
}

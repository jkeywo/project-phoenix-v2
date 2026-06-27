use bevy::prelude::*;

use crate::messages::{
    AdmittedCommands, CaptainBlackboard, ObjectiveSnapshot, ObjectiveSource, SystemBlackboard,
    SystemControlPayload, SystemId, ViewDirection, ViewMode,
};
use crate::objectives::WorldConditions;
use crate::ship::combat_activity::RecentCombatActivity;
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipState;
use crate::simulation::Ship;
use crate::world::server::ObjectiveManagerRes;

pub struct CaptainPlugin;

impl Plugin for CaptainPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RecentCombatActivity>();
        app.init_resource::<crate::server_app::WeaponFiredThisTick>();
        app.init_resource::<crate::messages::AdmittedCommands>();
        app.init_resource::<crate::server_app::CaptainPriorityBoost>();
        app.add_systems(
            Update,
            (
                operate_captain_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .before(handle_toggle_red_alert),
                handle_toggle_red_alert.in_set(crate::sim_sets::SimSet::Input),
                handle_set_view.in_set(crate::sim_sets::SimSet::Input),
                handle_set_objective_priority.in_set(crate::sim_sets::SimSet::Input),
                crate::ship::combat_activity::update_combat_activity
                    .in_set(crate::sim_sets::SimSet::Broadcast),
                publish_captain_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

// ── Input handlers ───────────────────────────────────────────────────────────

fn handle_toggle_red_alert(admitted: Res<AdmittedCommands>, mut ship: ResMut<ShipState>) {
    for cmd in admitted.for_target(crate::system_registry::RED_ALERT_SYSTEM_ID) {
        if matches!(cmd.payload, SystemControlPayload::ToggleRedAlert) {
            ship.toggle_red_alert();
        }
    }
}

fn view_request_from_admitted(
    cmd: &crate::messages::AdmittedCommand,
) -> Option<(SystemId, ViewMode)> {
    match &cmd.payload {
        SystemControlPayload::SetView { mode }
            if cmd.target.0 == crate::system_registry::VIEWSCREEN_SYSTEM_ID =>
        {
            Some((
                crate::ship::viewscreen::source_system_for_view_mode(mode),
                mode.clone(),
            ))
        }
        SystemControlPayload::SetView { mode }
            if cmd.target.0 == crate::system_registry::HELM_SYSTEM_ID =>
        {
            Some((crate::system_registry::helm_system_id(), mode.clone()))
        }
        _ => None,
    }
}

fn handle_set_view(admitted: Res<AdmittedCommands>, mut ship: ResMut<ShipState>) {
    for cmd in admitted.0.iter() {
        if let Some((source, mode)) = view_request_from_admitted(cmd) {
            ship.request_view_mode_from(source, mode);
        }
    }
}

/// Toggle the captain's priority boost for a doctrine objective.
/// Sending the same id twice clears the boost.
fn handle_set_objective_priority(
    admitted: Res<AdmittedCommands>,
    boost: Option<ResMut<crate::server_app::CaptainPriorityBoost>>,
) {
    let Some(mut boost) = boost else { return };
    for cmd in admitted.for_target(crate::system_registry::CAPTAIN_SYSTEM_ID) {
        if let SystemControlPayload::SetObjectivePriority { id } = &cmd.payload {
            if boost.boosted_id.as_deref() == Some(id.as_str()) {
                boost.boosted_id = None;
            } else {
                boost.boosted_id = Some(id.clone());
            }
        }
    }
}

/// AI system: if the captain system is AI-controlled, run `CaptainAi::operate`
/// and emit `ToggleRedAlert` into `AdmittedCommands` when the desired state
/// differs from the current state. Runs before `handle_toggle_red_alert` so
/// the command is visible to the handler in the same tick.
fn operate_captain_ai(
    time: Res<Time>,
    ship: Res<ShipState>,
    mut admitted: ResMut<AdmittedCommands>,
    ship_query: Query<&ShipSystemControlSources, With<Ship>>,
    activity: Res<RecentCombatActivity>,
) {
    let Ok(control_sources) = ship_query.single() else {
        return;
    };
    let policy = control_sources
        .0
        .policy_for(&crate::system_registry::red_alert_system_id());
    if !policy.operate_ai {
        return;
    }

    let now = time.elapsed_secs();
    let ai = crate::ai::core::CaptainAi;
    if let Some(should_be_red_alert) =
        ai.operate(now, activity.last_damage_taken, activity.last_weapon_fired)
    {
        if should_be_red_alert != ship.red_alert() {
            admitted.0.push(crate::messages::AdmittedCommand {
                target: SystemId(crate::system_registry::RED_ALERT_SYSTEM_ID.to_string()),
                payload: SystemControlPayload::ToggleRedAlert,
                response_token: None,
            });
        }
    }
}

// ── Blackboard publish ───────────────────────────────────────────────────────

fn publish_captain_blackboard(
    ship: Res<ShipState>,
    hull: Option<Res<crate::server_app::ShipHullIntegrity>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    boost: Option<Res<crate::server_app::CaptainPriorityBoost>>,
    ship_query: Query<&ShipSystemControlSources, With<Ship>>,
    mut blackboards: ResMut<crate::server_app::SystemBlackboards>,
) {
    let red_alert = ship.red_alert();
    let hull_fraction = hull
        .as_ref()
        .map(|h| {
            let max = h.0.total_max();
            if max > 0.0 {
                h.0.total_current() / max
            } else {
                1.0
            }
        })
        .unwrap_or(1.0);
    let control_sources = ship_query.single().ok();
    let red_alert_auto = control_sources.is_some_and(|cs| {
        cs.0.source_for(&crate::system_registry::red_alert_system_id()) == ControlSource::Ai
    });
    let viewscreen_auto = control_sources.is_some_and(|cs| {
        cs.0.source_for(&crate::system_registry::viewscreen_system_id()) == ControlSource::Ai
    });
    let view_direction = match &ship.view_mode {
        ViewMode::Camera(ViewDirection::Fore) => "Fore",
        ViewMode::Camera(ViewDirection::Port) => "Port",
        ViewMode::Camera(ViewDirection::Starboard) => "Starboard",
        ViewMode::Camera(ViewDirection::Aft) => "Aft",
        _ => "",
    }
    .to_string();

    // Score objectives to determine which doctrine ones are currently relevant.
    // Mission objectives are always shown; doctrine objectives are hidden when
    // their utility score is zero (conditions not met).
    let conditions = WorldConditions {
        red_alert,
        hull_fraction,
    };
    let captain_boost = boost.as_ref().and_then(|b| {
        b.boosted_id
            .as_deref()
            .map(|id| (id, crate::server_app::CaptainPriorityBoost::BOOST_AMOUNT))
    });
    let objectives_snap: Vec<ObjectiveSnapshot> = objectives
        .as_ref()
        .map(|obj| {
            let scored = obj.0.scored_pool_with_boost(&conditions, captain_boost);
            scored
                .into_iter()
                .filter(|o| o.source == ObjectiveSource::Mission || o.score > 0.0)
                .map(|o| o.snapshot)
                .collect()
        })
        .unwrap_or_default();

    let hull_integrity_pct = hull
        .as_ref()
        .map(|h| {
            let max_hp = h.0.total_max();
            if max_hp > 0.0 {
                (h.0.total_current() / max_hp * 100.0).clamp(0.0, 100.0)
            } else {
                100.0
            }
        })
        .unwrap_or(100.0);

    let game_status = if red_alert {
        "RED ALERT — All hands to battlestations."
    } else {
        "Standing by. All systems nominal."
    }
    .to_string();

    let bb = CaptainBlackboard {
        red_alert,
        red_alert_system_id: crate::system_registry::red_alert_system_id(),
        red_alert_auto,
        viewscreen_system_id: crate::system_registry::viewscreen_system_id(),
        viewscreen_auto,
        view_direction,
        view_mode: ship.view_mode.clone(),
        objectives: objectives_snap,
        hull_integrity_pct,
        game_status,
        boosted_objective_id: boost.as_ref().and_then(|b| b.boosted_id.clone()),
    };

    blackboards.0.insert(
        SystemId(crate::system_registry::CAPTAIN_SYSTEM_ID.to_string()),
        SystemBlackboard::Captain(bb),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Sessions};
    use crate::messages::{ClientMessage, ViewDirection};
    use crate::server_app::SystemBlackboards;
    use crate::ship::control_source::ControlSource;
    use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
    use crate::simulation::Ship;
    use crate::system_registry::CAPTAIN_SYSTEM_ID;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(LobbyPlugin)
            .add_plugins(CaptainPlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .init_resource::<Outbox>()
            .init_resource::<SystemBlackboards>()
            .insert_resource(ShipState::new())
            .add_systems(PostUpdate, collect);
        app.world_mut().spawn((
            Ship,
            ShipConfigComponent::default(),
            ShipSystemControlSources::default(),
        ));
        app
    }

    fn captain_bb(app: &App) -> CaptainBlackboard {
        let bbs = app.world().resource::<SystemBlackboards>();
        let key = SystemId(CAPTAIN_SYSTEM_ID.to_string());
        let SystemBlackboard::Captain(bb) = bbs.0.get(&key).unwrap() else {
            panic!("expected Captain blackboard");
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
        let out = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn set_control_source(
        app: &mut App,
        system_id: crate::messages::SystemId,
        source: ControlSource,
    ) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<Ship>>();
        let mut cs = q.single_mut(app.world_mut()).unwrap();
        cs.0.set(system_id, source);
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
        push(app, "captain", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    // ── ToggleRedAlert tests ────────────────────────────────────────────────

    #[test]
    fn toggle_red_alert_during_lobby_is_processed_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level (`.run_if(in_state(GamePhase::InProgress))`).
        // In test configurations without SimSet, the system processes messages during Lobby.
        // The production gate is enforced by the SimSet chain, not individual system logic.
        let mut app = test_app();
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn non_captain_toggle_red_alert_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "crew",
            ClientMessage::Identify {
                token: "crew".into(),
                name: "Bob".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "crew",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn captain_toggle_red_alert_works() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn captain_control_system_red_alert_works() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn ai_controlled_red_alert_ignores_human_control_system_toggle() {
        let mut app = test_app();
        // Set both the captain system and the red-alert system to AI control.
        // In a real game, the TOML sets every system under a console to the same
        // source; the admission gate checks the red-alert system's policy, so
        // both must be Ai for human commands targeting red-alert to be rejected.
        set_control_source(
            &mut app,
            crate::system_registry::captain_system_id(),
            ControlSource::Ai,
        );
        set_control_source(
            &mut app,
            crate::system_registry::red_alert_system_id(),
            ControlSource::Ai,
        );
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn non_captain_control_system_red_alert_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "crew",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn admission_gate_rejects_unauthorized_network_command() {
        // Verifies that the authority-at-admission gate (admit_system_commands)
        // rejects a ControlSystem message from a token that doesn't hold the target
        // console. The command must not appear in AdmittedCommands.
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "rando",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(
            !app.world().resource::<ShipState>().red_alert(),
            "unauthorized command must have no effect"
        );
        assert!(
            app.world()
                .resource::<crate::messages::AdmittedCommands>()
                .0
                .is_empty(),
            "unauthorized command must be rejected by the admission gate (AdmittedCommands should be empty)"
        );
    }

    #[test]
    fn admission_gate_rejects_npc_ai_token() {
        // An ai:<uuid> token whose entity is not the player Ship must be rejected
        // even when the target system has operate_ai = true (Backfill).
        let mut app = test_app();
        // Put Tactical under AI control so operate_ai is true — without the NPC
        // check the token would pass is_command_authorized.
        set_control_source(
            &mut app,
            crate::system_registry::tactical_system_id(),
            ControlSource::Ai,
        );
        start_game(&mut app);

        // Spawn a fake NPC entity (no Ship component) and register its AI token.
        let npc_uuid = "npc-test-001";
        let npc_entity = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<crate::ai::server::AiTokenRegistry>()
            .register_with_entity(npc_uuid, npc_entity);
        let npc_token = format!("ai:{}", npc_uuid);

        push(
            &mut app,
            &npc_token,
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "enemy-ship".into(),
                },
            },
        );
        tick(&mut app);
        assert!(
            app.world()
                .resource::<crate::messages::AdmittedCommands>()
                .0
                .is_empty(),
            "NPC ai: token must be rejected by the admission gate"
        );
    }

    #[test]
    fn wrong_control_system_target_does_not_toggle_red_alert() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("not-red-alert".into()),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!app.world().resource::<ShipState>().red_alert());
    }

    #[test]
    fn captain_toggle_red_alert_twice_returns_to_off() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(app.world().resource::<ShipState>().red_alert());
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!app.world().resource::<ShipState>().red_alert());
    }

    // ── SetView tests ───────────────────────────────────────────────────────

    #[test]
    fn set_view_during_lobby_is_processed_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level (`.run_if(in_state(GamePhase::InProgress))`).
        // In test configurations without SimSet, the system processes messages during Lobby.
        let mut app = test_app();
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(ViewDirection::Starboard),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Starboard)
        );
    }

    #[test]
    fn non_captain_set_view_is_ignored() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "crew",
            ClientMessage::Identify {
                token: "crew".into(),
                name: "Bob".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "crew",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(ViewDirection::Port),
                },
            },
        );
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
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(ViewDirection::Aft),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Aft)
        );
    }

    #[test]
    fn active_non_captain_view_toggles_back_to_last_captain_camera() {
        let mut app = test_app();
        push(
            &mut app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Hoshi".into(),
            },
        );
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SetReady { ready: true });
        push(&mut app, "helm", ClientMessage::SetReady { ready: true });
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(ViewDirection::Aft),
                },
            },
        );
        tick(&mut app);
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .set_station("helm", Some(crate::messages::StationId("helm".into())));
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Radar
        );
        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Aft)
        );
    }

    #[test]
    fn helm_control_system_set_view_can_request_radar() {
        let mut app = test_app();
        start_game(&mut app);

        // Radar is the helm's call, so seat a helm holder and send from it.
        push(
            &mut app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Hoshi".into(),
            },
        );
        tick(&mut app);
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .set_station("helm", Some(crate::messages::StationId("helm".into())));

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::helm_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Radar
        );
    }

    #[test]
    fn viewscreen_channel_2_set_view_can_request_radar() {
        let mut app = test_app();
        start_game(&mut app);

        push(
            &mut app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Hoshi".into(),
            },
        );
        tick(&mut app);
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .set_station("helm", Some(crate::messages::StationId("helm".into())));

        push(
            &mut app,
            "helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Radar
        );
    }

    #[test]
    fn ai_controlled_helm_can_drive_viewscreen_without_human_seat() {
        let mut app = test_app();
        set_control_source(
            &mut app,
            crate::system_registry::helm_system_id(),
            ControlSource::Ai,
        );

        push(
            &mut app,
            "ai:helm",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Radar,
                },
            },
        );
        tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Radar
        );
    }

    // ── operate_captain_ai tests ─────────────────────────────────────────────

    #[test]
    fn operate_captain_ai_activates_red_alert_when_in_combat() {
        let mut app = test_app();
        start_game(&mut app);
        // Set captain to AI mode
        set_control_source(
            &mut app,
            crate::system_registry::red_alert_system_id(),
            ControlSource::Ai,
        );
        app.world_mut()
            .resource_mut::<RecentCombatActivity>()
            .last_damage_taken = Some(0.0);
        tick(&mut app);
        assert!(
            app.world().resource::<ShipState>().red_alert(),
            "AI should activate red alert when damage was recent"
        );
    }

    #[test]
    fn operate_captain_ai_deactivates_red_alert_when_combat_ends() {
        let mut app = test_app();
        start_game(&mut app);
        // Put ship in red alert
        app.world_mut()
            .resource_mut::<ShipState>()
            .toggle_red_alert();
        assert!(app.world().resource::<ShipState>().red_alert());
        // Set captain to AI mode with no recent activity
        set_control_source(
            &mut app,
            crate::system_registry::red_alert_system_id(),
            ControlSource::Ai,
        );
        // No recent damage or weapons fire — activity is default (None)
        tick(&mut app);
        assert!(
            !app.world().resource::<ShipState>().red_alert(),
            "AI should deactivate red alert when no recent combat activity"
        );
    }

    #[test]
    fn operate_captain_ai_does_nothing_when_human_controlled() {
        let mut app = test_app();
        start_game(&mut app);
        app.world_mut()
            .resource_mut::<RecentCombatActivity>()
            .last_damage_taken = Some(0.0);
        tick(&mut app);
        // Human-controlled: AI system should not fire
        assert!(
            !app.world().resource::<ShipState>().red_alert(),
            "AI system must not fire when captain is human-controlled"
        );
    }

    #[test]
    fn operate_captain_ai_uses_red_alert_control_source_not_captain() {
        let mut app = test_app();
        start_game(&mut app);
        set_control_source(
            &mut app,
            crate::system_registry::captain_system_id(),
            ControlSource::Ai,
        );
        app.world_mut()
            .resource_mut::<RecentCombatActivity>()
            .last_damage_taken = Some(0.0);

        tick(&mut app);

        assert!(
            !app.world().resource::<ShipState>().red_alert(),
            "AI must only operate red alert when the red-alert system is automated"
        );
    }

    // ── Blackboard publish tests ─────────────────────────────────────────────

    use crate::damage::ConsoleHull;
    use crate::messages::Console;
    use crate::objectives::ObjectiveManager;
    use crate::server_app::ShipHullIntegrity;
    use crate::world::server::ObjectiveManagerRes;

    /// Minimal app: just publish_captain_blackboard + ShipState + SystemBlackboards.
    fn bb_test_app() -> App {
        let mut app = App::new();
        app.add_systems(Update, publish_captain_blackboard);
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[(
            Console::CaptainChair,
            100.0,
        )])));
        app.init_resource::<SystemBlackboards>();
        app
    }

    #[test]
    fn publish_captain_blackboard_reflects_red_alert() {
        let mut app = bb_test_app();
        app.world_mut()
            .resource_mut::<ShipState>()
            .toggle_red_alert();
        app.update();

        let bb = captain_bb(&app);
        assert!(bb.red_alert);
        assert_eq!(bb.game_status, "RED ALERT — All hands to battlestations.");
    }

    #[test]
    fn publish_captain_blackboard_marks_ai_red_alert_auto() {
        let mut app = bb_test_app();
        let mut cs = ShipSystemControlSources::default();
        cs.0.set(
            crate::system_registry::red_alert_system_id(),
            ControlSource::Ai,
        );
        app.world_mut()
            .spawn((Ship, ShipConfigComponent::default(), cs));
        app.update();

        let bb = captain_bb(&app);
        assert!(bb.red_alert_auto);
        assert_eq!(
            bb.red_alert_system_id,
            crate::system_registry::red_alert_system_id()
        );
    }

    #[test]
    fn publish_captain_blackboard_marks_ai_viewscreen_auto() {
        let mut app = bb_test_app();
        let mut cs = ShipSystemControlSources::default();
        cs.0.set(
            crate::system_registry::viewscreen_system_id(),
            ControlSource::Ai,
        );
        app.world_mut()
            .spawn((Ship, ShipConfigComponent::default(), cs));
        app.update();

        let bb = captain_bb(&app);
        assert!(
            bb.viewscreen_auto,
            "viewscreen_auto should be true when viewscreen is AI"
        );
        assert_eq!(
            bb.viewscreen_system_id,
            crate::system_registry::viewscreen_system_id()
        );
    }

    #[test]
    fn publish_captain_blackboard_viewscreen_auto_false_by_default() {
        let mut app = bb_test_app();
        app.update();
        assert!(
            !captain_bb(&app).viewscreen_auto,
            "viewscreen_auto should default to false"
        );
    }

    #[test]
    fn publish_captain_blackboard_reflects_hull_integrity() {
        let mut app = bb_test_app();
        {
            let mut hull = app.world_mut().resource_mut::<ShipHullIntegrity>();
            let mut rng = rand::rng();
            hull.0.apply_damage(25.0, &mut rng);
        }
        app.update();

        let bb = captain_bb(&app);
        assert!(
            (bb.hull_integrity_pct - 75.0).abs() < 1.0,
            "expected ~75% hull integrity, got {}",
            bb.hull_integrity_pct
        );
    }

    #[test]
    fn publish_captain_blackboard_reflects_objectives() {
        let mut app = bb_test_app();
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Test objective", true, vec![]);
        app.world_mut().insert_resource(ObjectiveManagerRes(mgr));
        app.update();

        let bb = captain_bb(&app);
        assert_eq!(bb.objectives.len(), 1);
        assert_eq!(bb.objectives[0].id, "obj-1");
        assert_eq!(bb.objectives[0].text, "Test objective");
    }

    #[test]
    fn publish_captain_blackboard_clears_direction_for_non_camera_view() {
        let mut app = bb_test_app();
        app.world_mut().resource_mut::<ShipState>().view_mode = ViewMode::Radar;
        app.update();
        assert_eq!(captain_bb(&app).view_direction, "");
    }

    // ── #574 objective filtering + priority boost tests ──────────────────────

    use crate::messages::{ObjectiveSource, ObjectiveStatus};
    use crate::objectives::{UtilityConfig, ZeroGateCondition};

    fn doctrine_objective_manager() -> ObjectiveManager {
        let mut mgr = ObjectiveManager::new();
        // Doctrine objective gated on red_alert — score=0 when not at red alert.
        mgr.add_full(
            "destroy-hostiles",
            "Destroy hostiles",
            false,
            vec![],
            crate::messages::AiDirective::None,
            UtilityConfig {
                base_priority: 30.0,
                zero_gates: vec![ZeroGateCondition {
                    condition: "red_alert".into(),
                    threshold: None,
                }],
                ..Default::default()
            },
            ObjectiveSource::Doctrine,
        );
        mgr
    }

    #[test]
    fn doctrine_objective_hidden_from_captain_bb_when_score_zero() {
        let mut app = bb_test_app();
        app.world_mut()
            .insert_resource(ObjectiveManagerRes(doctrine_objective_manager()));
        app.update();

        let bb = captain_bb(&app);
        assert!(
            bb.objectives.is_empty(),
            "doctrine objective with zero score must be hidden from the captain panel"
        );
    }

    #[test]
    fn doctrine_objective_shown_in_captain_bb_when_score_positive() {
        let mut app = bb_test_app();
        app.world_mut()
            .resource_mut::<ShipState>()
            .toggle_red_alert();
        app.world_mut()
            .insert_resource(ObjectiveManagerRes(doctrine_objective_manager()));
        app.update();

        let bb = captain_bb(&app);
        assert_eq!(
            bb.objectives.len(),
            1,
            "doctrine objective should be visible when conditions met"
        );
        assert_eq!(bb.objectives[0].id, "destroy-hostiles");
        assert_eq!(bb.objectives[0].source, ObjectiveSource::Doctrine);
    }

    #[test]
    fn mission_objective_always_shown_in_captain_bb() {
        // Mission objectives are never filtered regardless of utility score.
        let mut app = bb_test_app();
        let mut mgr = ObjectiveManager::new();
        mgr.add("reach-station", "Reach station", true, vec![]);
        app.world_mut().insert_resource(ObjectiveManagerRes(mgr));
        app.update();

        let bb = captain_bb(&app);
        assert_eq!(bb.objectives.len(), 1);
        assert_eq!(bb.objectives[0].source, ObjectiveSource::Mission);
    }

    #[test]
    fn boosted_objective_id_propagates_to_captain_bb() {
        let mut app = bb_test_app();
        app.world_mut()
            .insert_resource(crate::server_app::CaptainPriorityBoost {
                boosted_id: Some("destroy-hostiles".into()),
            });
        app.update();

        assert_eq!(
            captain_bb(&app).boosted_objective_id.as_deref(),
            Some("destroy-hostiles"),
        );
    }

    #[test]
    fn captain_priority_boost_makes_gated_doctrine_objective_visible() {
        // A doctrine objective gated on red_alert would normally be hidden (score=0).
        // The captain's priority boost overcomes the zero-score so it appears.
        let mut app = bb_test_app();
        app.world_mut()
            .insert_resource(ObjectiveManagerRes(doctrine_objective_manager()));
        app.world_mut()
            .insert_resource(crate::server_app::CaptainPriorityBoost {
                boosted_id: Some("destroy-hostiles".into()),
            });
        app.update();

        let bb = captain_bb(&app);
        assert_eq!(
            bb.objectives.len(),
            1,
            "boosted objective must appear despite zero-gate"
        );
        assert_eq!(bb.objectives[0].id, "destroy-hostiles");
    }

    #[test]
    fn set_objective_priority_command_sets_boost() {
        let mut app = test_app();
        app.world_mut()
            .insert_resource(crate::server_app::CaptainPriorityBoost::default());
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::captain_system_id(),
                payload: SystemControlPayload::SetObjectivePriority {
                    id: "destroy-hostiles".into(),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world()
                .resource::<crate::server_app::CaptainPriorityBoost>()
                .boosted_id
                .as_deref(),
            Some("destroy-hostiles"),
        );
    }

    #[test]
    fn set_objective_priority_command_toggles_off_when_same_id() {
        let mut app = test_app();
        app.world_mut()
            .insert_resource(crate::server_app::CaptainPriorityBoost {
                boosted_id: Some("destroy-hostiles".into()),
            });
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::captain_system_id(),
                payload: SystemControlPayload::SetObjectivePriority {
                    id: "destroy-hostiles".into(),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world()
                .resource::<crate::server_app::CaptainPriorityBoost>()
                .boosted_id,
            None,
            "sending the same id again should clear the boost"
        );
    }
}

use bevy::prelude::*;

use crate::messages::{
    AdmittedCommands, CameraView, CaptainBlackboard, ObjectiveSnapshot, ObjectiveSource,
    SystemBlackboard, SystemControlPayload, SystemId, ViewMode,
};
use crate::objectives::WorldConditions;
use crate::ship::combat_activity::RecentCombatActivity;
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::ShipSystemControlSources;
use crate::world::server::ObjectiveManagerRes;

pub struct CaptainPlugin;

impl Plugin for CaptainPlugin {
    fn build(&self, app: &mut App) {
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

/// Applies `ToggleRedAlert` commands from every ship's own
/// `AdmittedCommands` to that ship's own `ShipRedAlert`.
///
/// Iterates every ship (player + NPC) because `operate_captain_ai` writes
/// `ToggleRedAlert` into each ship's own `AdmittedCommands` when its
/// Captain system is AI-controlled. Without per-entity dispatch, NPC
/// captain-AI red-alert toggles would be silently dropped.
fn handle_toggle_red_alert(
    mut ship_query: Query<
        (&AdmittedCommands, &mut crate::ship_state::ShipRedAlert),
        With<crate::server_app::Ship>,
    >,
) {
    for (admitted, mut ra) in ship_query.iter_mut() {
        for cmd in admitted.for_target(crate::system_registry::RED_ALERT_SYSTEM_ID) {
            if matches!(cmd.payload, SystemControlPayload::ToggleRedAlert) {
                ra.toggle();
            }
        }
    }
}

fn view_request_from_admitted(
    cmd: &crate::messages::AdmittedCommand,
) -> Option<(SystemId, ViewMode)> {
    /// Map a "cinematic" marker name to the Cinematic view mode.
    fn resolve(mode: &ViewMode) -> ViewMode {
        match mode {
            ViewMode::Camera(cv) if cv.marker_name == "cinematic" => ViewMode::Cinematic,
            _ => mode.clone(),
        }
    }
    match &cmd.payload {
        // `SetView` arrives either on the viewscreen target or (legacy helm
        // console path) on the `"helm"` station-id target — the coarse helm
        // system is gone (#801), but the wire string is unchanged and resolves
        // through the station-name admission fallback. Either way the
        // requesting system is derived from the view mode itself.
        SystemControlPayload::SetView { mode }
            if cmd.target.0 == crate::system_registry::VIEWSCREEN_SYSTEM_ID
                || cmd.target.0 == crate::system_registry::HELM_STATION_ID =>
        {
            Some((
                crate::ship::viewscreen::source_system_for_view_mode(mode),
                resolve(mode),
            ))
        }
        _ => None,
    }
}

fn handle_set_view(
    ship_query: Query<&AdmittedCommands, With<crate::server_app::LocalShip>>,
    mut view_mode_q: Query<
        &mut crate::ship_state::ShipViewMode,
        With<crate::server_app::LocalShip>,
    >,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    let Some(mut vm) = view_mode_q.iter_mut().next() else {
        return;
    };
    for cmd in admitted.0.iter() {
        if let Some((source, mode)) = view_request_from_admitted(cmd) {
            vm.request_view_mode_from(source, mode);
        }
    }
}

/// Toggle the captain's priority boost for a doctrine objective.
/// Sending the same id twice clears the boost.
fn handle_set_objective_priority(
    ship_query: Query<&AdmittedCommands, With<crate::server_app::LocalShip>>,
    mut boost: ResMut<crate::server_app::CaptainPriorityBoost>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
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
///
/// After PRD #597 PR 10: reads combat timers from each ship's own
/// per-entity `RecentCombatActivity` component — no global resource. Loops over
/// all ship entities (player and NPC) where the Captain system is
/// `ControlSource::Ai`.
fn operate_captain_ai(
    time: Res<Time>,
    sessions: Res<crate::lobby::Sessions>,
    mut ship_query: Query<(
        &mut AdmittedCommands,
        &ShipSystemControlSources,
        &RecentCombatActivity,
        Option<&crate::ship_state::ShipRedAlert>,
        Option<&crate::entity_spawner::EntityUuid>,
        Option<&crate::ship_plugin::ShipConfigComponent>,
    )>,
) {
    let now = time.elapsed_secs();
    let ai = crate::ai::core::CaptainAi;

    for (mut admitted, control_sources, activity, red_alert_opt, entity_uuid, ship_config) in
        ship_query.iter_mut()
    {
        let policy = control_sources
            .0
            .policy_for(&crate::system_registry::red_alert_system_id());
        if !policy.operate_ai {
            continue;
        }

        // Read this ship's own combat activity.
        let last_under_attack =
            most_recent(activity.last_damage_taken, activity.last_hostile_fire_taken);
        let last_weapon_fired = activity.last_weapon_fired;

        if let Some(should_be_red_alert) = ai.operate(now, last_under_attack, last_weapon_fired) {
            let current_red_alert = red_alert_opt.map(|ra| ra.0).unwrap_or(false);
            if should_be_red_alert != current_red_alert {
                // Route through the shared admission seam with this ship's own
                // `ai:<uuid>` token (issue #830) rather than pushing straight
                // into `AdmittedCommands` — true AI/human symmetry, mirroring
                // `emit_sensors_ai_command`. The decision above (CaptainAi, 10s
                // window) and its change-guard are unchanged.
                emit_captain_ai_command(
                    entity_uuid,
                    SystemControlPayload::ToggleRedAlert,
                    control_sources,
                    &sessions,
                    ship_config,
                    &mut admitted,
                );
            }
        }
    }
}

/// Emit an admitted Captain AI command targeting the red-alert system through
/// the shared [`crate::command_admission::validate_and_admit`] seam, using this
/// ship's own `ai:<uuid>` token (mirrors `emit_sensors_ai_command`).
fn emit_captain_ai_command(
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    payload: SystemControlPayload,
    sources: &ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
    admitted: &mut AdmittedCommands,
) -> bool {
    let token = entity_uuid
        .map(|u| format!("ai:{}", u.0))
        .unwrap_or_else(|| "ai:backfill".to_string());
    let default_config;
    let config = match ship_config {
        Some(c) => &c.0,
        None => {
            default_config = crate::ship::config::ShipConfig {
                stations: vec![],
                systems: vec![],
                power_groups: std::collections::HashMap::new(),
                coordination_lag_secs: 0.0,
            };
            &default_config
        }
    };
    crate::command_admission::validate_and_admit(
        &token,
        crate::system_registry::red_alert_system_id(),
        payload,
        sources,
        sessions,
        config,
        admitted,
    )
}

fn most_recent(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

// ── Blackboard publish ───────────────────────────────────────────────────────

/// Per-`Ship` publisher (issue #830). Ship-wide fields (red_alert, auto flags,
/// hull integrity, game_status) are computed for every ship from its own
/// per-entity `ShipRedAlert` + `ShipSystemControlSources` + `EntitySystemHull`.
/// Player-only fields — camera views (from the local `ModelMarkers` /
/// `CinematicCameraSection`), view direction/mode (from the local
/// `ShipViewMode`), and the objectives list + boost (from `ObjectiveManagerRes`
/// / `CaptainPriorityBoost`) — are gated on `Has<LocalShip>`; NPCs get the
/// empty/default equivalents (nothing reads an NPC captain blackboard, and the
/// wire broadcaster is `LocalShip`-filtered).
fn publish_captain_blackboard(
    objectives: Option<Res<ObjectiveManagerRes>>,
    boost: Res<crate::server_app::CaptainPriorityBoost>,
    markers_q: Query<&crate::model_rig::ModelMarkers, With<crate::server_app::LocalShip>>,
    cinematic_q: Query<
        Option<&crate::entity_spawner::CinematicCameraSection>,
        With<crate::server_app::LocalShip>,
    >,
    mut ship_query: Query<
        (
            &ShipSystemControlSources,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::ship_state::ShipViewMode>,
            Option<&crate::entity_spawner::EntitySystemHull>,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (control_sources, red_alert_comp, view_mode_comp, hull_opt, is_local, mut bbs) in
        ship_query.iter_mut()
    {
        let red_alert = red_alert_comp.map(|ra| ra.0).unwrap_or(false);

        let (hull_fraction, hull_integrity_pct) = hull_opt
            .map(|h| {
                let max = h.0.total_max();
                if max > 0.0 {
                    let frac = h.0.total_current() / max;
                    (frac, (frac * 100.0).clamp(0.0, 100.0))
                } else {
                    (1.0, 100.0)
                }
            })
            .unwrap_or((1.0, 100.0));

        let red_alert_auto = control_sources
            .0
            .source_for(&crate::system_registry::red_alert_system_id())
            == ControlSource::Ai;
        let viewscreen_auto = control_sources
            .0
            .source_for(&crate::system_registry::viewscreen_system_id())
            == ControlSource::Ai;

        // ── Player-only fields (LocalShip) ────────────────────────────────────
        // View mode / camera list / objectives are player camera + doctrine
        // surfaces. NPCs get the same defaults the pre-#830 `.single()` error
        // arms produced for a ship missing the component.
        let view_mode = if is_local {
            view_mode_comp
                .map(|vm| vm.view_mode.clone())
                .unwrap_or(ViewMode::Camera(CameraView::default()))
        } else {
            ViewMode::Camera(CameraView::default())
        };
        let view_direction = match &view_mode {
            ViewMode::Camera(cv) => cv.marker_name.clone(),
            ViewMode::Cinematic => "cinematic".to_string(),
            _ => String::new(),
        };

        let mut camera_views: Vec<String> = Vec::new();
        let mut objectives_snap: Vec<ObjectiveSnapshot> = Vec::new();
        let mut boosted_objective_id: Option<String> = None;
        if is_local {
            camera_views = markers_q
                .single()
                .ok()
                .map(|mm| {
                    mm.marker_names()
                        .filter(|n| n.starts_with("camera_"))
                        .map(|n| n.to_string())
                        .collect()
                })
                .unwrap_or_default();
            let has_cinematic = cinematic_q.single().ok().is_some_and(|c| c.is_some());
            if has_cinematic {
                camera_views.push("cinematic".to_string());
            }

            let conditions = WorldConditions {
                red_alert,
                hull_fraction,
                attacked: false,
            };
            let captain_boost = boost
                .boosted_id
                .as_deref()
                .map(|id| (id, crate::server_app::CaptainPriorityBoost::BOOST_AMOUNT));
            objectives_snap = objectives
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
            boosted_objective_id = boost.boosted_id.clone();
        }

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
            view_mode,
            camera_views,
            objectives: objectives_snap,
            hull_integrity_pct,
            game_status,
            boosted_objective_id,
        };

        bbs.0.insert(
            SystemId(crate::system_registry::CAPTAIN_SYSTEM_ID.to_string()),
            SystemBlackboard::Captain(bb),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Sessions};
    use crate::messages::{CameraView, ClientMessage};
    use crate::server_app::LocalShip;
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
            .add_systems(PostUpdate, collect);
        app.world_mut().spawn((
            Ship,
            LocalShip,
            ShipConfigComponent::default(),
            ShipSystemControlSources::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::ship_state::ShipRedAlert::default(),
            crate::ship_state::ShipViewMode::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            // Per-entity combat activity trackers (PRD #597 PR 10).
            RecentCombatActivity::default(),
            crate::server_app::WeaponFiredThisTick::default(),
            crate::server_app::ShipAttackedThisTick::default(),
            crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[(
                crate::messages::SystemId("captain".into()),
                100.0,
            )])),
        ));
        app
    }

    fn get_red_alert(app: &mut App) -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipRedAlert, With<LocalShip>>();
        q.single(app.world()).map(|ra| ra.0).unwrap_or(false)
    }

    fn get_view_mode(app: &mut App) -> ViewMode {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipViewMode, With<LocalShip>>();
        q.single(app.world())
            .map(|vm| vm.view_mode.clone())
            .unwrap_or(ViewMode::Camera(CameraView::default()))
    }

    fn set_red_alert(app: &mut App, red: bool) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut crate::ship_state::ShipRedAlert, With<LocalShip>>();
        if let Ok(mut ra) = q.single_mut(app.world_mut()) {
            ra.0 = red;
        }
    }

    fn captain_bb(app: &mut App) -> CaptainBlackboard {
        let mut q = app.world_mut().query_filtered::<&crate::server_app::ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let bbs = q.single(app.world()).unwrap();
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
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
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
                station: "Captain".into(),
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
                station: "Captain".into(),
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
        assert!(get_red_alert(&mut app));
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
        assert!(!get_red_alert(&mut app));
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
        assert!(get_red_alert(&mut app));
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
        assert!(get_red_alert(&mut app));
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
        assert!(!get_red_alert(&mut app));
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
        assert!(!get_red_alert(&mut app));
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
            !get_red_alert(&mut app),
            "unauthorized command must have no effect"
        );
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::messages::AdmittedCommands, With<crate::server_app::LocalShip>>();
        let admitted = q
            .single(app.world())
            .expect("LocalShip must carry AdmittedCommands");
        assert!(
            admitted.0.is_empty(),
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
            crate::system_registry::tactical_radar_system_id(),
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
                target: crate::system_registry::tactical_radar_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "enemy-ship".into(),
                },
            },
        );
        tick(&mut app);
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::messages::AdmittedCommands, With<crate::server_app::LocalShip>>();
        let admitted = q
            .single(app.world())
            .expect("LocalShip must carry AdmittedCommands");
        assert!(
            admitted.0.is_empty(),
            "NPC ai: token must be rejected by the admission gate"
        );
    }

    #[test]
    fn config_defined_system_controllable_by_owning_console() {
        // A config-defined system (red-alert → station "captain") is controllable
        // by the holder of its owning console, and denied for non-holders.
        let mut app = test_app();
        start_game(&mut app);

        // "captain" holds the captain station → should be admitted.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(get_red_alert(&mut app), "captain should control red-alert");

        // Toggle back off.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!get_red_alert(&mut app));

        // "rando" (not identified, does not hold captain station) → denied.
        push(
            &mut app,
            "rando",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!get_red_alert(&mut app), "rando must not control red-alert");
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::messages::AdmittedCommands, With<crate::server_app::LocalShip>>();
        let admitted = q
            .single(app.world())
            .expect("LocalShip must carry AdmittedCommands");
        assert!(
            !admitted.0.iter().any(|cmd| cmd.target.0 == "red-alert"),
            "rando command targeting red-alert must be rejected"
        );
    }

    #[test]
    fn unknown_system_id_is_denied() {
        // A system ID not present in the ship config is denied (not conservatively
        // allowed) by the admission gate.
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::messages::SystemId("foobar".into()),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!get_red_alert(&mut app));
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::messages::AdmittedCommands, With<crate::server_app::LocalShip>>();
        let admitted = q
            .single(app.world())
            .expect("LocalShip must carry AdmittedCommands");
        assert!(
            admitted.0.is_empty(),
            "unknown system id must be rejected by the admission gate"
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
        assert!(!get_red_alert(&mut app));
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
        assert!(get_red_alert(&mut app));
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);
        assert!(!get_red_alert(&mut app));
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
                station: "Captain".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::viewscreen_system_id(),
                payload: SystemControlPayload::SetView {
                    mode: ViewMode::Camera(CameraView::new("camera_starboard")),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::new("camera_starboard"))
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
                    mode: ViewMode::Camera(CameraView::new("camera_port")),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::default())
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
                    mode: ViewMode::Camera(CameraView::new("camera_aft")),
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::new("camera_aft"))
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
                station: "Captain".into(),
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
                    mode: ViewMode::Camera(CameraView::new("camera_aft")),
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
        assert_eq!(get_view_mode(&mut app), ViewMode::Radar);
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
            get_view_mode(&mut app),
            ViewMode::Camera(CameraView::new("camera_aft"))
        );
    }

    // (Issue #832) The former `helm_control_system_set_view_can_request_radar`
    // test drove SetView through the bare `"helm"` station-id wire target,
    // relying on `station_for_system`'s step-3 station-name fallback. That
    // fallback was removed (no client emits a station-name target — every
    // SetView goes through the `viewscreen` system). The production path is
    // covered by `viewscreen_channel_2_set_view_can_request_radar` below.

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

        assert_eq!(get_view_mode(&mut app), ViewMode::Radar);
    }

    #[test]
    fn ai_controlled_helm_can_drive_viewscreen_without_human_seat() {
        let mut app = test_app();
        // Radar view authority derives from the helm-radar fine system
        // (issue #801) — the coarse helm system no longer exists.
        set_control_source(
            &mut app,
            crate::system_registry::helm_radar_system_id(),
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

        assert_eq!(get_view_mode(&mut app), ViewMode::Radar);
    }

    fn set_activity_last_damage(app: &mut App, secs: Option<f32>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut RecentCombatActivity, With<LocalShip>>();
        if let Ok(mut a) = q.single_mut(app.world_mut()) {
            a.last_damage_taken = secs;
        }
    }

    fn set_activity_hostile_fire(app: &mut App, secs: Option<f32>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut RecentCombatActivity, With<LocalShip>>();
        if let Ok(mut a) = q.single_mut(app.world_mut()) {
            a.last_hostile_fire_taken = secs;
        }
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
        set_activity_last_damage(&mut app, Some(0.0));
        tick(&mut app);
        assert!(
            get_red_alert(&mut app),
            "AI should activate red alert when damage was recent"
        );
    }

    #[test]
    fn operate_captain_ai_deactivates_red_alert_when_combat_ends() {
        let mut app = test_app();
        start_game(&mut app);
        // Put ship in red alert
        {
            let cur = get_red_alert(&mut app);
            set_red_alert(&mut app, !cur);
        }
        assert!(get_red_alert(&mut app));
        // Set captain to AI mode with no recent activity
        set_control_source(
            &mut app,
            crate::system_registry::red_alert_system_id(),
            ControlSource::Ai,
        );
        // No recent damage or weapons fire — activity is default (None)
        tick(&mut app);
        assert!(
            !get_red_alert(&mut app),
            "AI should deactivate red alert when no recent combat activity"
        );
    }

    #[test]
    fn operate_captain_ai_does_nothing_when_human_controlled() {
        let mut app = test_app();
        start_game(&mut app);
        set_activity_last_damage(&mut app, Some(0.0));
        tick(&mut app);
        // Human-controlled: AI system should not fire
        assert!(
            !get_red_alert(&mut app),
            "AI system must not fire when captain is human-controlled"
        );
    }

    #[test]
    fn operate_captain_ai_activates_red_alert_when_under_hostile_fire() {
        let mut app = test_app();
        start_game(&mut app);
        set_control_source(
            &mut app,
            crate::system_registry::red_alert_system_id(),
            ControlSource::Ai,
        );
        set_activity_hostile_fire(&mut app, Some(0.0));

        tick(&mut app);

        assert!(
            get_red_alert(&mut app),
            "AI should activate red alert when hostile fire targets the ship"
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
        set_activity_last_damage(&mut app, Some(0.0));

        tick(&mut app);

        assert!(
            !get_red_alert(&mut app),
            "AI must only operate red alert when the red-alert system is automated"
        );
    }

    // ── Blackboard publish tests ─────────────────────────────────────────────

    use crate::damage::SystemHull;
    use crate::messages::SystemId;
    use crate::objectives::ObjectiveManager;
    use crate::world::server::ObjectiveManagerRes;

    /// Minimal app: just publish_captain_blackboard + per-entity components.
    fn bb_test_app() -> App {
        let mut app = App::new();
        // `publish_captain_blackboard` now takes a plain `Res<CaptainPriorityBoost>`
        // (issue #830); this harness does not add `CaptainPlugin`, so init it here.
        app.init_resource::<crate::server_app::CaptainPriorityBoost>();
        app.add_systems(Update, publish_captain_blackboard);
        let hull = SystemHull::from_config(&[(SystemId("captain".into()), 100.0)]);
        // Spawn LocalShip entity with required components for publish_captain_blackboard.
        // Ship marker required: the publisher now iterates `With<Ship>` (issue #830).
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            crate::ship_state::ShipRedAlert::default(),
            crate::ship_state::ShipViewMode::default(),
            ShipSystemControlSources::default(),
            crate::entity_spawner::EntitySystemHull(hull),
            crate::server_app::ShipSystemBlackboards::default(),
        ));
        app
    }

    fn apply_hull_damage(app: &mut App, amount: f32) {
        let mut rng = rand::rng();
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .get_mut::<crate::entity_spawner::EntitySystemHull>()
            .unwrap()
            .0
            .apply_damage(amount, &mut rng);
    }

    #[test]
    fn publish_captain_blackboard_reflects_red_alert() {
        let mut app = bb_test_app();
        {
            let cur = get_red_alert(&mut app);
            set_red_alert(&mut app, !cur);
        }
        app.update();

        let bb = captain_bb(&mut app);
        assert!(bb.red_alert);
        assert_eq!(bb.game_status, "RED ALERT — All hands to battlestations.");
    }

    #[test]
    fn publish_captain_blackboard_marks_ai_red_alert_auto() {
        let mut app = bb_test_app();
        // Set the LocalShip entity's ShipSystemControlSources to AI for red-alert.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<LocalShip>>();
            if let Ok(mut cs) = q.single_mut(app.world_mut()) {
                cs.0.set(
                    crate::system_registry::red_alert_system_id(),
                    ControlSource::Ai,
                );
            }
        }
        app.update();

        let bb = captain_bb(&mut app);
        assert!(bb.red_alert_auto);
        assert_eq!(
            bb.red_alert_system_id,
            crate::system_registry::red_alert_system_id()
        );
    }

    #[test]
    fn publish_captain_blackboard_marks_ai_viewscreen_auto() {
        let mut app = bb_test_app();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipSystemControlSources, With<LocalShip>>();
            if let Ok(mut cs) = q.single_mut(app.world_mut()) {
                cs.0.set(
                    crate::system_registry::viewscreen_system_id(),
                    ControlSource::Ai,
                );
            }
        }
        app.update();

        let bb = captain_bb(&mut app);
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
            !captain_bb(&mut app).viewscreen_auto,
            "viewscreen_auto should default to false"
        );
    }

    #[test]
    fn publish_captain_blackboard_reflects_hull_integrity() {
        let mut app = bb_test_app();
        apply_hull_damage(&mut app, 25.0);
        app.update();

        let bb = captain_bb(&mut app);
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

        let bb = captain_bb(&mut app);
        assert_eq!(bb.objectives.len(), 1);
        assert_eq!(bb.objectives[0].id, "obj-1");
        assert_eq!(bb.objectives[0].text, "Test objective");
    }

    #[test]
    fn publish_captain_blackboard_clears_direction_for_non_camera_view() {
        let mut app = bb_test_app();
        // Set view mode via per-entity component
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::ship_state::ShipViewMode, With<LocalShip>>();
            if let Ok(mut vm) = q.single_mut(app.world_mut()) {
                vm.view_mode = ViewMode::Radar;
            }
        }
        app.update();
        assert_eq!(captain_bb(&mut app).view_direction, "");
    }

    // ── #574 objective filtering + priority boost tests ──────────────────────

    use crate::messages::ObjectiveSource;
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

        let bb = captain_bb(&mut app);
        assert!(
            bb.objectives.is_empty(),
            "doctrine objective with zero score must be hidden from the captain panel"
        );
    }

    #[test]
    fn doctrine_objective_shown_in_captain_bb_when_score_positive() {
        let mut app = bb_test_app();
        {
            let cur = get_red_alert(&mut app);
            set_red_alert(&mut app, !cur);
        }
        app.world_mut()
            .insert_resource(ObjectiveManagerRes(doctrine_objective_manager()));
        app.update();

        let bb = captain_bb(&mut app);
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

        let bb = captain_bb(&mut app);
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
            captain_bb(&mut app).boosted_objective_id.as_deref(),
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

        let bb = captain_bb(&mut app);
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

    /// Verifies that operate_captain_ai reads combat state from the ship's
    /// per-entity `RecentCombatActivity` component (PRD #597 PR 10).
    #[test]
    fn operate_captain_ai_activates_red_alert_when_attacker_in_blackboard() {
        // Build using the standard test_app.
        let mut app = test_app();

        // Set last_damage_taken on the LocalShip's per-entity RecentCombatActivity.
        set_activity_last_damage(&mut app, Some(0.0));

        start_game(&mut app);
        set_control_source(
            &mut app,
            crate::system_registry::red_alert_system_id(),
            ControlSource::Ai,
        );

        tick(&mut app);
        assert!(
            get_red_alert(&mut app),
            "AI should activate red alert when last_damage_taken is set on the ship's RecentCombatActivity"
        );
    }

    // ── NPC red-alert parity regression (audit follow-up) ─────────────────
    //
    // Regression test for the audit-report bug: `operate_captain_ai` iterates
    // every ship (player + NPC) and pushes `ToggleRedAlert` into each ship's
    // own `AdmittedCommands`, but `handle_toggle_red_alert` previously read
    // only the LocalShip's `AdmittedCommands`, so NPC red-alert toggles were
    // silently dropped. This test spawns an NPC ship with AI-controlled
    // red-alert, gives it recent combat activity, and asserts the NPC's own
    // `ShipRedAlert` flips while the LocalShip's does not.

    #[test]
    fn npc_captain_ai_toggles_own_red_alert_via_admitted_commands() {
        let mut app = test_app();
        start_game(&mut app);

        // Build an NPC ship with the same essential components as the
        // LocalShip, but without the LocalShip marker. Set its red-alert
        // system to AI control.
        let npc_control_sources = {
            let mut cs = ShipSystemControlSources::default();
            cs.0.set(
                crate::system_registry::red_alert_system_id(),
                ControlSource::Ai,
            );
            cs
        };
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                ShipConfigComponent::default(),
                npc_control_sources,
                crate::messages::AdmittedCommands::default(),
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::ship_state::ShipRedAlert::default(),
                crate::ship_state::ShipViewMode::default(),
                crate::server_app::ShipSystemBlackboards::default(),
                RecentCombatActivity {
                    last_damage_taken: Some(0.0),
                    ..Default::default()
                },
                crate::server_app::WeaponFiredThisTick::default(),
                crate::server_app::ShipAttackedThisTick::default(),
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[
                    (crate::messages::SystemId("captain".into()), 100.0),
                ])),
            ))
            .id();

        // Player red-alert is Human-controlled and has no combat activity —
        // the AI must not toggle it.
        tick(&mut app);

        let npc_red_alert = app
            .world()
            .entity(npc)
            .get::<crate::ship_state::ShipRedAlert>()
            .expect("NPC must carry ShipRedAlert")
            .0;
        assert!(
            npc_red_alert,
            "operate_captain_ai should have activated the NPC's own red-alert (its AI is under combat)"
        );
        assert!(
            !get_red_alert(&mut app),
            "player's red-alert must be unaffected by NPC captain-AI toggle"
        );
    }

    #[test]
    fn handle_toggle_red_alert_applies_admitted_commands_per_entity() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn an NPC ship without LocalShip whose red-alert system is
        // AI-held, register its `ai:` token, and send a `ToggleRedAlert`
        // through the inbound queue. Since #824 admission is ship-aware —
        // `AdmittedCommands` is cleared per-entity every tick and a
        // registered `ai:` token routes to its own ship's queue — so a
        // pre-seeded component would be wiped before the handler ran; the
        // wire path is the honest way in, and it exercises the routing too.
        let npc_control_sources = {
            let mut cs = ShipSystemControlSources::default();
            cs.0.set(
                crate::system_registry::red_alert_system_id(),
                ControlSource::Ai,
            );
            cs
        };
        let npc = app
            .world_mut()
            .spawn((
                Ship,
                ShipConfigComponent::default(),
                npc_control_sources,
                crate::messages::AdmittedCommands::default(),
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::ship_state::ShipRedAlert::default(),
                crate::ship_state::ShipViewMode::default(),
                crate::server_app::ShipSystemBlackboards::default(),
                RecentCombatActivity::default(),
                crate::server_app::WeaponFiredThisTick::default(),
                crate::server_app::ShipAttackedThisTick::default(),
                crate::entity_spawner::EntitySystemHull(crate::damage::SystemHull::from_config(&[
                    (crate::messages::SystemId("captain".into()), 100.0),
                ])),
            ))
            .id();
        let npc_uuid = uuid::Uuid::new_v4().to_string();
        app.world_mut()
            .resource_mut::<crate::ai::server::AiTokenRegistry>()
            .register_with_entity(&npc_uuid, npc);

        push(
            &mut app,
            &format!("ai:{npc_uuid}"),
            ClientMessage::ControlSystem {
                target: SystemId(crate::system_registry::RED_ALERT_SYSTEM_ID.to_string()),
                payload: SystemControlPayload::ToggleRedAlert,
            },
        );
        tick(&mut app);

        let npc_red_alert = app
            .world()
            .entity(npc)
            .get::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0;
        assert!(
            npc_red_alert,
            "handle_toggle_red_alert must apply ToggleRedAlert from the NPC's own AdmittedCommands"
        );
        assert!(
            !get_red_alert(&mut app),
            "handle_toggle_red_alert must not touch the LocalShip when an NPC's AdmittedCommands drives the toggle"
        );
    }
}

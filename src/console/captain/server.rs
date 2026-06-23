use bevy::prelude::*;

use crate::console_bridge::ConsoleStateChanged;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{
    CaptainConsoleState, ClientMessage, Console, ObjectiveSnapshot, SystemControlPayload, SystemId,
    ViewDirection, ViewMode,
};
use crate::ship::combat_activity::RecentCombatActivity;
use crate::ship::control_source::ControlSource;
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipState;
use crate::world::server::ObjectiveManagerRes;

pub struct CaptainPlugin;

impl Plugin for CaptainPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ConsoleStateChanged>();
        app.init_resource::<RecentCombatActivity>();
        app.init_resource::<crate::server_app::WeaponFiredThisTick>();
        app.add_systems(
            Update,
            (
                handle_toggle_red_alert.in_set(crate::sim_sets::SimSet::Input),
                handle_set_view.in_set(crate::sim_sets::SimSet::Input),
                operate_captain_ai.in_set(crate::sim_sets::SimSet::Input),
            ),
        );
        // HTML console state push (mirrors the WeaponsPlugin pattern from issue #422).
        app.add_systems(Startup, spawn_captain_console_state_entity);
        app.add_systems(
            Update,
            (
                crate::ship::combat_activity::update_combat_activity
                    .in_set(crate::sim_sets::SimSet::Broadcast),
                recompute_captain_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
                push_captain_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
            ),
        );
    }
}

// ── Input handlers ───────────────────────────────────────────────────────────

fn handle_toggle_red_alert(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    control_sources: Option<Res<ShipSystemControlSources>>,
) {
    let policy = control_sources
        .as_deref()
        .map(|cs| {
            cs.0.policy_for(&crate::system_registry::captain_system_id())
        })
        .unwrap_or(crate::ship::control_source::control_tick_policy(
            ControlSource::Human,
        ));
    for ev in reader.read() {
        if !is_red_alert_toggle(&ev.msg) {
            continue;
        }
        if !policy.accept_human_input {
            continue;
        }
        if sessions.0.console_holder(Console::CaptainChair) != Some(ev.token.as_str()) {
            continue;
        }
        ship.toggle_red_alert();
    }
}

fn is_red_alert_toggle(msg: &ClientMessage) -> bool {
    match msg {
        ClientMessage::ToggleRedAlert => true,
        ClientMessage::ControlSystem { target, payload } => {
            target.0 == crate::system_registry::RED_ALERT_SYSTEM_ID
                && matches!(payload, SystemControlPayload::ToggleRedAlert)
        }
        _ => false,
    }
}

fn view_request_from_message(msg: &ClientMessage) -> Option<(SystemId, ViewMode)> {
    match msg {
        ClientMessage::SetView { mode } => Some((
            crate::ship::viewscreen::source_system_for_view_mode(mode),
            mode.clone(),
        )),
        ClientMessage::ControlSystem { target, payload }
            if target.0 == crate::system_registry::VIEWSCREEN_SYSTEM_ID =>
        {
            match payload {
                SystemControlPayload::SetView { mode } => Some((
                    crate::ship::viewscreen::source_system_for_view_mode(mode),
                    mode.clone(),
                )),
                _ => None,
            }
        }
        ClientMessage::ControlSystem { target, payload }
            if target.0 == crate::system_registry::HELM_SYSTEM_ID =>
        {
            match payload {
                SystemControlPayload::SetView { mode } => {
                    Some((crate::system_registry::helm_system_id(), mode.clone()))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn handle_set_view(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
    control_sources: Option<Res<ShipSystemControlSources>>,
) {
    for ev in reader.read() {
        if let Some((source, mode)) = view_request_from_message(&ev.msg) {
            if source_can_request_view(&source, &ev.token, &sessions, control_sources.as_deref()) {
                ship.request_view_mode_from(source, mode);
            }
        }
    }
}

fn source_can_request_view(
    source: &SystemId,
    token: &str,
    sessions: &Sessions,
    control_sources: Option<&ShipSystemControlSources>,
) -> bool {
    let policy = control_sources.map(|cs| cs.0.policy_for(source)).unwrap_or(
        crate::ship::control_source::control_tick_policy(ControlSource::Human),
    );
    if policy.operate_ai {
        return true;
    }
    if !policy.accept_human_input {
        return false;
    }
    let Some(console) = console_for_view_source(source) else {
        return false;
    };
    sessions.0.console_holder(console) == Some(token)
}

fn console_for_view_source(source: &SystemId) -> Option<Console> {
    match source.0.as_str() {
        crate::system_registry::CAPTAIN_SYSTEM_ID => Some(Console::CaptainChair),
        crate::system_registry::HELM_SYSTEM_ID => Some(Console::Helm),
        crate::system_registry::SENSORS_SYSTEM_ID => Some(Console::Sensors),
        crate::system_registry::NAVIGATION_SYSTEM_ID => Some(Console::Navigation),
        crate::system_registry::COMMS_SYSTEM_ID => Some(Console::Comms),
        _ => None,
    }
}

/// AI system: if the captain system is AI-controlled, run `CaptainAi::operate`
/// and toggle red alert when the result differs from the current state.
fn operate_captain_ai(
    mut ship: ResMut<ShipState>,
    control_sources: Option<Res<ShipSystemControlSources>>,
    activity: Option<Res<RecentCombatActivity>>,
    time: Res<Time>,
) {
    let policy = control_sources
        .as_deref()
        .map(|cs| {
            cs.0.policy_for(&crate::system_registry::captain_system_id())
        })
        .unwrap_or(crate::ship::control_source::control_tick_policy(
            ControlSource::Human,
        ));
    if !policy.operate_ai {
        return;
    }
    let activity = match activity.as_deref() {
        Some(a) => a,
        None => return,
    };
    let ai = crate::ai::core::CaptainAi;
    if let Some(should_be_red_alert) = ai.operate(activity, time.elapsed_secs()) {
        if should_be_red_alert != ship.red_alert() {
            ship.toggle_red_alert();
        }
    }
}

// ── HTML console state push ──────────────────────────────────────────────────
//
// Mirrors the WeaponsPlugin pattern from issue #422: writes Captain console
// state into a single entity component so a Changed<...> system can encode and
// emit a ConsoleStateChanged message. The wasm forwarding to the JS callback
// lives in bridge::flush_console_state.

/// Single-entity component carrying the latest serialised Captain console state.
/// Bevy change-detection drives the JS push.
#[derive(Component, Clone, PartialEq)]
pub struct CaptainConsoleStateComp(pub CaptainConsoleState);

/// Startup system: spawn the single entity carrying the Captain console state.
fn spawn_captain_console_state_entity(mut commands: Commands) {
    commands.spawn(CaptainConsoleStateComp(CaptainConsoleState {
        red_alert: false,
        red_alert_system_id: crate::system_registry::red_alert_system_id(),
        red_alert_auto: false,
        viewscreen_system_id: crate::system_registry::viewscreen_system_id(),
        viewscreen_auto: false,
        view_direction: "Fore".into(),
        objectives: Vec::new(),
        hull_integrity_pct: 100.0,
        game_status: String::new(),
    }));
}

/// Recompute the Captain console state from live resources, writing into
/// `CaptainConsoleStateComp` only on change so `Changed<...>` fires on actual
/// state change.
fn recompute_captain_console_state(
    ship: Res<ShipState>,
    hull: Option<Res<crate::server_app::ShipHullIntegrity>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    control_sources: Option<Res<ShipSystemControlSources>>,
    mut comp_q: Query<&mut CaptainConsoleStateComp>,
) {
    let red_alert = ship.red_alert();
    let red_alert_auto = control_sources.as_deref().is_some_and(|cs| {
        cs.0.source_for(&crate::system_registry::captain_system_id()) == ControlSource::Ai
    });
    let viewscreen_auto = control_sources.as_deref().is_some_and(|cs| {
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

    let objectives_snap: Vec<ObjectiveSnapshot> = objectives
        .as_ref()
        .map(|obj| obj.0.sorted_snapshots())
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

    // Compute a game-status string (matches what renderGame(s) in client.html
    // would show for the captain — but computed server-side so the HTML panel
    // receives it even before the client-side state arrives).
    let game_status = if red_alert {
        "RED ALERT — All hands to battlestations."
    } else {
        "Standing by. All systems nominal."
    }
    .to_string();

    let next = CaptainConsoleState {
        red_alert,
        red_alert_system_id: crate::system_registry::red_alert_system_id(),
        red_alert_auto,
        viewscreen_system_id: crate::system_registry::viewscreen_system_id(),
        viewscreen_auto,
        view_direction,
        objectives: objectives_snap,
        hull_integrity_pct,
        game_status,
    };

    for mut comp in comp_q.iter_mut() {
        if comp.0 != next {
            comp.0 = next.clone();
        }
    }
}

/// `Changed<CaptainConsoleStateComp>` system: encode the state and emit a
/// `ConsoleStateChanged { name: "CaptainChair", json }` message for the wasm bridge
/// to forward to the JS `__updateConsole` callback.
fn push_captain_console_state(
    comp_q: Query<&CaptainConsoleStateComp, Changed<CaptainConsoleStateComp>>,
    mut writer: MessageWriter<ConsoleStateChanged>,
) {
    for comp in comp_q.iter() {
        if let Ok(json) = crate::core::codec::encode_console_state(&comp.0) {
            writer.write(ConsoleStateChanged {
                name: "CaptainChair".into(),
                json,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::ViewDirection;
    use crate::ship::control_source::ControlSource;
    use crate::ship_plugin::ShipSystemControlSources;

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
            .init_resource::<Outbox>()
            .init_resource::<ShipSystemControlSources>()
            .insert_resource(ShipState::new())
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
        push(app, "captain", ClientMessage::StartGame);
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
        push(&mut app, "captain", ClientMessage::ToggleRedAlert);
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
        // Set captain system to AI control
        {
            let mut cs = app.world_mut().resource_mut::<ShipSystemControlSources>();
            cs.0.set(
                crate::system_registry::captain_system_id(),
                ControlSource::Ai,
            );
        }
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
        push(&mut app, "captain", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(app.world().resource::<ShipState>().red_alert());
        push(&mut app, "captain", ClientMessage::ToggleRedAlert);
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
            ClientMessage::SetView {
                mode: ViewMode::Camera(ViewDirection::Starboard),
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
            ClientMessage::SetView {
                mode: ViewMode::Camera(ViewDirection::Port),
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
            ClientMessage::SetView {
                mode: ViewMode::Camera(ViewDirection::Aft),
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
        push(&mut app, "captain", ClientMessage::StartGame);
        tick(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SetView {
                mode: ViewMode::Camera(ViewDirection::Aft),
            },
        );
        tick(&mut app);
        app.world_mut()
            .resource_mut::<Sessions>()
            .0
            .toggle_console("helm", Console::Helm)
            .unwrap();
        push(
            &mut app,
            "helm",
            ClientMessage::SetView {
                mode: ViewMode::Radar,
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
            ClientMessage::SetView {
                mode: ViewMode::Radar,
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
            .toggle_console("helm", Console::Helm)
            .unwrap();

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
            .toggle_console("helm", Console::Helm)
            .unwrap();

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
        {
            let mut cs = app.world_mut().resource_mut::<ShipSystemControlSources>();
            cs.0.set(crate::system_registry::helm_system_id(), ControlSource::Ai);
        }

        push(
            &mut app,
            "ai",
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
        {
            let mut cs = app.world_mut().resource_mut::<ShipSystemControlSources>();
            cs.0.set(
                crate::system_registry::captain_system_id(),
                ControlSource::Ai,
            );
        }
        // Simulate recent damage (at t=0, now is ~0 so within 10s window)
        {
            let mut activity = app.world_mut().resource_mut::<RecentCombatActivity>();
            activity.last_damage_taken = Some(0.0);
        }
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
        {
            let mut cs = app.world_mut().resource_mut::<ShipSystemControlSources>();
            cs.0.set(
                crate::system_registry::captain_system_id(),
                ControlSource::Ai,
            );
        }
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
        // Simulate damage — but captain is human-controlled (default)
        {
            let mut activity = app.world_mut().resource_mut::<RecentCombatActivity>();
            activity.last_damage_taken = Some(0.0);
        }
        tick(&mut app);
        // Human-controlled: AI system should not fire
        assert!(
            !app.world().resource::<ShipState>().red_alert(),
            "AI system must not fire when captain is human-controlled"
        );
    }

    // ── Console state push tests ─────────────────────────────────────────────
    //
    // Follows the exact pattern from `weapons/server.rs` (issue #422):
    //   • Recompute tests add only `recompute_captain_console_state` (no push).
    //   • Push tests add recompute + push + collector, wiring them directly
    //     (no SimSet — the test app does not call `add_simulation_plugins`).

    use crate::damage::ConsoleHull;
    use crate::messages::Console;
    use crate::objectives::ObjectiveManager;
    use crate::server_app::ShipHullIntegrity;
    use crate::world::server::ObjectiveManagerRes;

    /// Helper app that adds only the recompute system (no push, no message bus).
    /// Used for recompute-assertion tests that inspect the component directly.
    fn recompute_test_app() -> App {
        let mut app = App::new();
        app.add_systems(Startup, spawn_captain_console_state_entity);
        app.add_systems(Update, recompute_captain_console_state);
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[(
            Console::CaptainChair,
            100.0,
        )])));
        app
    }

    /// Collect `ConsoleStateChanged` messages into a resource for assertions.
    #[derive(Resource, Default)]
    struct ConsolePushes(Vec<ConsoleStateChanged>);

    fn collect_console_pushes(
        mut reader: MessageReader<ConsoleStateChanged>,
        mut sink: ResMut<ConsolePushes>,
    ) {
        for m in reader.read() {
            sink.0.push(m.clone());
        }
    }

    /// Helper app that exercises the push pipeline (no recompute —
    /// callers mutate the component directly, matching the weapons push test
    /// pattern from issue #422).
    fn push_test_app() -> App {
        let mut app = App::new();
        app.add_message::<ConsoleStateChanged>()
            .init_resource::<ConsolePushes>()
            .add_systems(
                Update,
                (
                    push_captain_console_state,
                    collect_console_pushes.after(push_captain_console_state),
                ),
            );
        // Spawn the component in the world directly (not via Startup system)
        // so that the first update sees it as Changed (spawn → new).
        app.world_mut()
            .spawn(CaptainConsoleStateComp(CaptainConsoleState {
                red_alert: false,
                red_alert_system_id: crate::system_registry::red_alert_system_id(),
                red_alert_auto: false,
                viewscreen_system_id: crate::system_registry::viewscreen_system_id(),
                viewscreen_auto: false,
                view_direction: "Fore".into(),
                objectives: Vec::new(),
                hull_integrity_pct: 100.0,
                game_status: String::new(),
            }));
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[(
            Console::CaptainChair,
            100.0,
        )])));
        app
    }

    // ── Spawn tests ───────────────────────────────────────────────────────────

    #[test]
    fn spawn_entity_exists_with_defaults() {
        let mut app = App::new();
        app.add_systems(Startup, spawn_captain_console_state_entity);
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let state = q.single(app.world()).unwrap();
        assert!(!state.0.red_alert);
        assert_eq!(state.0.hull_integrity_pct, 100.0);
        assert!(state.0.objectives.is_empty());
    }

    // ── Recompute tests (no push bus) ─────────────────────────────────────────

    #[test]
    fn recompute_reflects_red_alert() {
        let mut app = recompute_test_app();
        app.world_mut()
            .resource_mut::<ShipState>()
            .toggle_red_alert();
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(comp.0.red_alert);
        assert_eq!(
            comp.0.game_status,
            "RED ALERT — All hands to battlestations."
        );
    }

    #[test]
    fn recompute_marks_ai_controlled_red_alert_auto() {
        let mut app = recompute_test_app();
        // Set captain system to AI control via ShipSystemControlSources
        app.init_resource::<ShipSystemControlSources>();
        {
            let mut cs = app.world_mut().resource_mut::<ShipSystemControlSources>();
            cs.0.set(
                crate::system_registry::captain_system_id(),
                ControlSource::Ai,
            );
        }
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(comp.0.red_alert_auto);
        assert_eq!(
            comp.0.red_alert_system_id,
            crate::system_registry::red_alert_system_id()
        );
    }

    #[test]
    fn recompute_marks_ai_controlled_viewscreen_auto() {
        let mut app = recompute_test_app();
        app.init_resource::<ShipSystemControlSources>();
        {
            let mut cs = app.world_mut().resource_mut::<ShipSystemControlSources>();
            cs.0.set(
                crate::system_registry::viewscreen_system_id(),
                ControlSource::Ai,
            );
        }
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(comp.0.viewscreen_auto, "viewscreen_auto should be true when viewscreen is AI");
        assert_eq!(
            comp.0.viewscreen_system_id,
            crate::system_registry::viewscreen_system_id()
        );
    }

    #[test]
    fn recompute_viewscreen_auto_is_false_by_default() {
        let mut app = recompute_test_app();
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(
            !comp.0.viewscreen_auto,
            "viewscreen_auto should default to false"
        );
    }

    #[test]
    fn recompute_reflects_hull_integrity() {
        let mut app = recompute_test_app();
        // Apply 25 damage to the 100-HP hull.
        {
            let mut hull = app.world_mut().resource_mut::<ShipHullIntegrity>();
            let mut rng = rand::rng();
            hull.0.apply_damage(25.0, &mut rng);
        }
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(
            (comp.0.hull_integrity_pct - 75.0).abs() < 1.0,
            "expected ~75% hull integrity, got {}",
            comp.0.hull_integrity_pct
        );
    }

    #[test]
    fn recompute_reflects_objectives() {
        let mut app = recompute_test_app();
        let mut mgr = ObjectiveManager::new();
        mgr.add("obj-1", "Test objective", true, vec![]);
        app.world_mut().insert_resource(ObjectiveManagerRes(mgr));
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert_eq!(comp.0.objectives.len(), 1);
        assert_eq!(comp.0.objectives[0].id, "obj-1");
        assert_eq!(comp.0.objectives[0].text, "Test objective");
    }

    // ── Push tests (full pipeline) ────────────────────────────────────────────

    #[test]
    fn recompute_clears_direction_for_non_camera_view() {
        let mut app = recompute_test_app();
        app.world_mut().resource_mut::<ShipState>().view_mode = ViewMode::Radar;
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert_eq!(comp.0.view_direction, "");
    }

    #[test]
    fn push_emits_one_message_with_expected_values() {
        let mut app = push_test_app();

        // First update: freshly spawned component is Changed → push fires.
        app.update();
        app.world_mut().resource_mut::<ConsolePushes>().0.clear();

        // Mutate the component → next update should push exactly one message.
        {
            let mut q = app.world_mut().query::<&mut CaptainConsoleStateComp>();
            let mut comp = q.single_mut(app.world_mut()).unwrap();
            comp.0 = CaptainConsoleState {
                red_alert: true,
                red_alert_system_id: crate::system_registry::red_alert_system_id(),
                red_alert_auto: false,
                viewscreen_system_id: crate::system_registry::viewscreen_system_id(),
                viewscreen_auto: false,
                view_direction: "Aft".into(),
                objectives: Vec::new(),
                hull_integrity_pct: 75.0,
                game_status: "RED ALERT".into(),
            };
        }
        app.update();

        let pushes = &app.world().resource::<ConsolePushes>().0;
        assert_eq!(pushes.len(), 1, "expected exactly one push after a change");
        let push = &pushes[0];
        assert_eq!(push.name, "CaptainChair");
        assert!(
            push.json.contains("\"red_alert\":true"),
            "json: {}",
            push.json
        );
        assert!(
            push.json.contains("\"hull_integrity_pct\":75.0"),
            "json: {}",
            push.json
        );
        assert!(push.json.contains("\"RED ALERT\""), "json: {}", push.json);
        assert!(
            push.json.contains("\"view_direction\":\"Aft\""),
            "json: {}",
            push.json
        );

        // No further change → no further pushes.
        app.world_mut().resource_mut::<ConsolePushes>().0.clear();
        app.update();

        assert!(
            app.world().resource::<ConsolePushes>().0.is_empty(),
            "no push expected without a change"
        );
    }
}

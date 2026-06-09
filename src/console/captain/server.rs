use bevy::prelude::*;

use crate::console_bridge::ConsoleStateChanged;
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{CaptainConsoleState, ClientMessage, Console, ObjectiveSnapshot, ViewDirection, ViewMode};
use crate::ship_state::ShipState;
use crate::world::server::ObjectiveManagerRes;

pub struct CaptainPlugin;

impl Plugin for CaptainPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ConsoleStateChanged>();
        app.add_systems(Update, (
            handle_toggle_red_alert.in_set(crate::sim_sets::SimSet::Input),
            handle_set_view.in_set(crate::sim_sets::SimSet::Input),
        ));
        // HTML console state push (mirrors the WeaponsPlugin pattern from issue #422).
        app.add_systems(Startup, spawn_captain_console_state_entity);
        app.add_systems(Update, (
            recompute_captain_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
            push_captain_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
        ));
    }
}

// ── Input handlers ───────────────────────────────────────────────────────────

/// Returns `true` when the caller is either the real Captain-chair holder or
/// the local console bridge (HTML / native wry server).
fn captain_authorized(sessions: &Sessions, token: &str) -> bool {
    sessions.0.console_holder(Console::CaptainChair) == Some(token)
        || token == crate::console_bridge::LOCAL_CONSOLE_TOKEN
}

fn handle_toggle_red_alert(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
) {
    for ev in reader.read() {
        if matches!(ev.msg, ClientMessage::ToggleRedAlert)
            && captain_authorized(&sessions, &ev.token)
        {
            ship.toggle_red_alert();
        }
    }
}

fn handle_set_view(
    mut reader: MessageReader<InboundMessage>,
    mut ship: ResMut<ShipState>,
    sessions: Res<Sessions>,
) {
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
            let authorized = if required == Console::CaptainChair {
                captain_authorized(&sessions, &ev.token)
            } else {
                sessions.0.console_holder(required) == Some(ev.token.as_str())
            };
            if authorized {
                ship.view_mode = mode;
            }
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
    mut comp_q: Query<&mut CaptainConsoleStateComp>,
) {
    let red_alert = ship.red_alert();
    let view_direction = match &ship.view_mode {
        ViewMode::Camera(ViewDirection::Fore)      => "Fore",
        ViewMode::Camera(ViewDirection::Port)      => "Port",
        ViewMode::Camera(ViewDirection::Starboard) => "Starboard",
        ViewMode::Camera(ViewDirection::Aft)       => "Aft",
        _                                          => "Fore",
    }.to_string();

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
/// `ConsoleStateChanged { name: "Captain", json }` message for the wasm bridge
/// to forward to the JS `__updateConsole` callback.
fn push_captain_console_state(
    comp_q: Query<&CaptainConsoleStateComp, Changed<CaptainConsoleStateComp>>,
    mut writer: MessageWriter<ConsoleStateChanged>,
) {
    for comp in comp_q.iter() {
        if let Ok(json) = crate::core::codec::encode_console_state(&comp.0) {
            writer.write(ConsoleStateChanged {
                name: "Captain".into(),
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
    fn toggle_red_alert_during_lobby_is_processed_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level (`.run_if(in_state(GamePhase::InProgress))`).
        // In test configurations without SimSet, the system processes messages during Lobby.
        // The production gate is enforced by the SimSet chain, not individual system logic.
        let mut app = test_app();
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::ToggleRedAlert);
        tick(&mut app);
        assert!(app.world().resource::<ShipState>().red_alert());
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
    fn set_view_during_lobby_is_processed_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level (`.run_if(in_state(GamePhase::InProgress))`).
        // In test configurations without SimSet, the system processes messages during Lobby.
        let mut app = test_app();
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) });
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

    // ── Console state push tests ─────────────────────────────────────────────
    //
    // Follows the exact pattern from `weapons/server.rs` (issue #422):
    //   • Recompute tests add only `recompute_captain_console_state` (no push).
    //   • Push tests add recompute + push + collector, wiring them directly
    //     (no SimSet — the test app does not call `add_simulation_plugins`).

    use crate::damage::ConsoleHull;
    use crate::server_app::ShipHullIntegrity;
    use crate::world::server::ObjectiveManagerRes;
    use crate::objectives::ObjectiveManager;
    use crate::messages::Console;

    /// Helper app that adds only the recompute system (no push, no message bus).
    /// Used for recompute-assertion tests that inspect the component directly.
    fn recompute_test_app() -> App {
        let mut app = App::new();
        app.add_systems(Startup, spawn_captain_console_state_entity);
        app.add_systems(Update, recompute_captain_console_state);
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipHullIntegrity(
            ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)]),
        ));
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
            .add_systems(Update, (
                push_captain_console_state,
                collect_console_pushes.after(push_captain_console_state),
            ));
        // Spawn the component in the world directly (not via Startup system)
        // so that the first update sees it as Changed (spawn → new).
        app.world_mut().spawn(CaptainConsoleStateComp(CaptainConsoleState {
            red_alert: false,
            view_direction: "Fore".into(),
            objectives: Vec::new(),
            hull_integrity_pct: 100.0,
            game_status: String::new(),
        }));
        app.insert_resource(ShipState::new());
        app.insert_resource(ShipHullIntegrity(
            ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)]),
        ));
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
        app.world_mut().resource_mut::<ShipState>().toggle_red_alert();
        app.update();

        let mut q = app.world_mut().query::<&CaptainConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(comp.0.red_alert);
        assert_eq!(comp.0.game_status, "RED ALERT — All hands to battlestations.");
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
        assert_eq!(push.name, "Captain");
        assert!(push.json.contains("\"red_alert\":true"), "json: {}", push.json);
        assert!(push.json.contains("\"hull_integrity_pct\":75.0"), "json: {}", push.json);
        assert!(push.json.contains("\"RED ALERT\""), "json: {}", push.json);
        assert!(push.json.contains("\"view_direction\":\"Aft\""), "json: {}", push.json);

        // No further change → no further pushes.
        app.world_mut().resource_mut::<ConsolePushes>().0.clear();
        app.update();

        assert!(
            app.world().resource::<ConsolePushes>().0.is_empty(),
            "no push expected without a change"
        );
    }
}

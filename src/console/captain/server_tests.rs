use super::*;
use crate::core::messages::{CameraView, ClientMessage};
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Sessions};
use crate::server_app::LocalShip;
use crate::server_app::Ship;
use crate::ship::control_source::ControlSource;
use crate::ship::system_registry::CAPTAIN_SYSTEM_ID;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

fn test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
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
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::ActiveStationRatings::default(),
        crate::ship_plugin::CoordinationQueue::default(),
        crate::ship::state::ShipRedAlert::default(),
        crate::ship::state::ShipViewMode::default(),
        crate::server_app::ShipSystemBlackboards::default(),
        // Per-entity combat activity trackers (PRD #597 PR 10).
        RecentCombatActivity::default(),
        crate::server_app::WeaponFiredThisTick::default(),
        crate::server_app::ShipAttackedThisTick::default(),
        crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::from_config(
            &[(crate::core::messages::SystemId("captain".into()), 100.0)],
        )),
        // The AUTHORED `[captain_console.ai]` block every shipped hull
        // carries. Since #885b stage 5d `operate_captain_ai` has no
        // synthesised fallback — a ship with no policy takes no Red Alert
        // decision — so a fixture that wants the behaviour must attach the
        // declaration a real hull writes.
        CaptainAiPolicy(
            crate::entities::authored_ai_pins::shipped_policy_toml("captain")
                .to_policy()
                .expect("the shipped Captain policy decodes"),
        ),
    ));
    // The restraint lever (issue #1041). Its own `insert` because the
    // bundle above is at Bevy's 15-element tuple ceiling — the same reason
    // the production spawner splits it out.
    {
        let mut q = app.world_mut().query_filtered::<Entity, With<LocalShip>>();
        let ship = q.single(app.world()).expect("the fixture ship");
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship::state::ShipWeaponsHold::default());
    }
    // One fixed step per update (issue #895): the plugin's systems run on
    // the logical tick, and each harness tick advances it once.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(1),
    );
    app
}

fn get_red_alert(app: &mut App) -> bool {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::ship::state::ShipRedAlert, With<LocalShip>>();
    q.single(app.world()).map(|ra| ra.0).unwrap_or(false)
}

fn get_view_mode(app: &mut App) -> ViewMode {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::ship::state::ShipViewMode, With<LocalShip>>();
    q.single(app.world())
        .map(|vm| vm.view_mode.clone())
        .unwrap_or(ViewMode::Camera(CameraView::default()))
}

fn set_red_alert(app: &mut App, red: bool) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut crate::ship::state::ShipRedAlert, With<LocalShip>>();
    if let Ok(mut ra) = q.single_mut(app.world_mut()) {
        ra.0 = red;
    }
}

fn get_weapons_hold(app: &mut App) -> bool {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::ship::state::ShipWeaponsHold, With<LocalShip>>();
    q.single(app.world()).map(|h| h.0).unwrap_or(false)
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
    // Issue #889: `operate_captain_ai` is gated by
    // `run_if(ai_snapshot_ready)`, so a fixture that wants a decision on
    // this update has to tick the latch. It used to be gated inside the
    // system body by an `Option<Res<_>>` that fell back to evaluating every
    // tick when absent — which is what every fixture below silently
    // exercised, so the shipped cadence was covered by no test at all.
    // Arming the latch by hand keeps these tests about decision CONTENT;
    // the cadence itself is covered in `ai::cadence`.
    crate::ai::cadence::arm_ai_tick(app);
    app.update();
    let out = app.world().resource::<Outbox>().0.clone();
    app.world_mut().resource_mut::<Outbox>().0.clear();
    out
}

fn set_control_source(
    app: &mut App,
    system_id: crate::core::messages::SystemId,
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

// ── SetRedAlert tests ───────────────────────────────────────────────────

#[test]
fn set_red_alert_during_lobby_is_processed_when_no_simset_gate() {
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
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(get_red_alert(&mut app));
}

#[test]
fn non_captain_set_red_alert_is_ignored() {
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
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(!get_red_alert(&mut app));
}

#[test]
fn captain_set_red_alert_works() {
    let mut app = test_app();
    start_game(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(get_red_alert(&mut app));
}

/// Issue #1041 AC1: the hold is a state the captain sets, LAYERED on the
/// binary alert rather than replacing it — so the two move independently
/// and Red Alert's own behaviour is untouched.
///
/// Both directions in one app, because "the captain can hold fire" is only
/// half a lever: a hold that could not be released would be a ship that had
/// disarmed itself.
#[test]
fn captain_holds_and_releases_fire_without_touching_the_alert() {
    let mut app = test_app();
    start_game(&mut app);
    // Stations first. The alert is the state the hold layers under, and
    // asserting it stays up throughout is the "does not replace" half.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(get_red_alert(&mut app));
    assert!(
        !get_weapons_hold(&mut app),
        "a ship at stations is weapons-free until someone says otherwise"
    );

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetWeaponsHold { held: true },
        },
    );
    tick(&mut app);
    assert!(get_weapons_hold(&mut app), "the captain's order lands");
    assert!(
        get_red_alert(&mut app),
        "and the ship is STILL at red alert — the hold layers under the \
         alert, it does not stand it down"
    );

    // Releasing it puts the ship back exactly where it started.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetWeaponsHold { held: false },
        },
    );
    tick(&mut app);
    assert!(!get_weapons_hold(&mut app));
    assert!(get_red_alert(&mut app));
}

/// The command carries the desired END state, so a retried or duplicated
/// press is idempotent — the handler assigns, it does not invert. Same
/// contract as `SetRedAlert`, and for the same reason: a console showing a
/// stale posture must not be able to flip the ship's guns back on.
#[test]
fn a_repeated_weapons_hold_order_is_idempotent() {
    let mut app = test_app();
    start_game(&mut app);
    for _ in 0..3 {
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::ship::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::SetWeaponsHold { held: true },
            },
        );
        tick(&mut app);
        assert!(get_weapons_hold(&mut app));
    }
}

/// The hold is replicated onto the same console that raises the alert, so
/// the captain reads one posture rather than inferring it.
#[test]
fn the_captain_blackboard_publishes_the_weapons_hold() {
    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app);
    assert!(!captain_bb(&mut app).weapons_hold);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetWeaponsHold { held: true },
        },
    );
    // Two ticks: one for the order to be admitted and applied, one for the
    // publisher to export the settled reading. The assertion is on the
    // PUBLISHED value rather than on the component deliberately — what is
    // pinned here is the captain's readout of the posture, which is what
    // makes the lever usable at all.
    tick(&mut app);
    tick(&mut app);
    assert!(captain_bb(&mut app).weapons_hold);
}

#[test]
fn captain_control_system_red_alert_works() {
    let mut app = test_app();
    start_game(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(get_red_alert(&mut app));
}

#[test]
fn ai_controlled_red_alert_ignores_human_control_system_set() {
    let mut app = test_app();
    // Set both the captain system and the red-alert system to AI control.
    // In a real game, the TOML sets every system under a console to the same
    // source; the admission gate checks the red-alert system's policy, so
    // both must be Ai for human commands targeting red-alert to be rejected.
    set_control_source(
        &mut app,
        crate::ship::system_registry::captain_system_id(),
        ControlSource::Ai,
    );
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Ai,
    );
    start_game(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
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
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
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
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(
        !get_red_alert(&mut app),
        "unauthorized command must have no effect"
    );
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::core::messages::AdmittedCommands, With<crate::server_app::LocalShip>>();
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
        crate::ship::system_registry::tactical_radar_system_id(),
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
            target: crate::ship::system_registry::tactical_radar_system_id(),
            payload: SystemControlPayload::SetTarget {
                uuid: "enemy-ship".into(),
            },
        },
    );
    tick(&mut app);
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::core::messages::AdmittedCommands, With<crate::server_app::LocalShip>>();
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
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(get_red_alert(&mut app), "captain should control red-alert");

    // Set back off with an explicit inactive request.
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: false },
        },
    );
    tick(&mut app);
    assert!(!get_red_alert(&mut app));

    // "rando" (not identified, does not hold captain station) → denied.
    push(
        &mut app,
        "rando",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(!get_red_alert(&mut app), "rando must not control red-alert");
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::core::messages::AdmittedCommands, With<crate::server_app::LocalShip>>();
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
            target: crate::core::messages::SystemId("foobar".into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(!get_red_alert(&mut app));
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::core::messages::AdmittedCommands, With<crate::server_app::LocalShip>>();
    let admitted = q
        .single(app.world())
        .expect("LocalShip must carry AdmittedCommands");
    assert!(
        admitted.0.is_empty(),
        "unknown system id must be rejected by the admission gate"
    );
}

#[test]
fn wrong_control_system_target_does_not_set_red_alert() {
    let mut app = test_app();
    start_game(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("not-red-alert".into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(!get_red_alert(&mut app));
}

#[test]
fn captain_set_red_alert_false_turns_off() {
    let mut app = test_app();
    start_game(&mut app);
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);
    assert!(get_red_alert(&mut app));
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: false },
        },
    );
    tick(&mut app);
    assert!(!get_red_alert(&mut app));
}

// ── Idempotency: the whole point of the set command (issue #748) ──────────

#[test]
fn captain_set_red_alert_true_twice_is_idempotent() {
    // A retried / duplicated activate must not flip the state back off —
    // this is the failure mode a toggle command has and a set command
    // fixes.
    let mut app = test_app();
    start_game(&mut app);
    for _ in 0..2 {
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::ship::system_registry::red_alert_system_id(),
                payload: SystemControlPayload::SetRedAlert { active: true },
            },
        );
        tick(&mut app);
        assert!(
            get_red_alert(&mut app),
            "repeated SetRedAlert{{active:true}} must remain active"
        );
    }
}

#[test]
fn captain_stale_set_red_alert_false_when_already_off_is_noop() {
    // A stale UI that still believes the ship is at red alert sends
    // active:false; the ship is already off, so the assignment is a no-op.
    let mut app = test_app();
    start_game(&mut app);
    assert!(!get_red_alert(&mut app));
    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active: false },
        },
    );
    tick(&mut app);
    assert!(
        !get_red_alert(&mut app),
        "SetRedAlert{{active:false}} on an already-off ship must stay off"
    );
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
            target: crate::ship::system_registry::viewscreen_system_id(),
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
            target: crate::ship::system_registry::viewscreen_system_id(),
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
            target: crate::ship::system_registry::viewscreen_system_id(),
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
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Camera(CameraView::new("camera_aft")),
            },
        },
    );
    tick(&mut app);
    app.world_mut().resource_mut::<Sessions>().0.set_station(
        "helm",
        Some(crate::core::messages::StationId("helm".into())),
    );
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
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
            target: crate::ship::system_registry::viewscreen_system_id(),
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
    app.world_mut().resource_mut::<Sessions>().0.set_station(
        "helm",
        Some(crate::core::messages::StationId("helm".into())),
    );

    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Radar,
            },
        },
    );
    tick(&mut app);

    assert_eq!(get_view_mode(&mut app), ViewMode::Radar);
}

#[test]
fn unauthorised_set_view_does_not_disturb_active_view() {
    // AC3 (issue #769): an unauthorised SetView is rejected at admission
    // and never reaches the arbiter, so the currently resolved view is
    // left unchanged. Here a helm-radar overlay is active and an
    // unauthorised console attempts to steal the screen with a camera view.
    let mut app = test_app();
    start_game(&mut app);

    // Authorised helm-radar overlay wins the screen.
    push(
        &mut app,
        "helm",
        ClientMessage::Identify {
            token: "helm".into(),
            name: "Hoshi".into(),
        },
    );
    tick(&mut app);
    app.world_mut().resource_mut::<Sessions>().0.set_station(
        "helm",
        Some(crate::core::messages::StationId("helm".into())),
    );
    push(
        &mut app,
        "helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Radar,
            },
        },
    );
    tick(&mut app);
    assert_eq!(get_view_mode(&mut app), ViewMode::Radar);

    // Unauthorised console (no station held) attempts a SetView.
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
            target: crate::ship::system_registry::viewscreen_system_id(),
            payload: SystemControlPayload::SetView {
                mode: ViewMode::Camera(CameraView::new("camera_port")),
            },
        },
    );
    tick(&mut app);

    // Rejected at admission → active view is unchanged.
    assert_eq!(get_view_mode(&mut app), ViewMode::Radar);
}

#[test]
fn ai_controlled_helm_can_drive_viewscreen_without_human_seat() {
    let mut app = test_app();
    // Radar view authority derives from the helm-radar fine system
    // (issue #801) — the coarse helm system no longer exists.
    set_control_source(
        &mut app,
        crate::ship::system_registry::helm_radar_system_id(),
        ControlSource::Ai,
    );

    push(
        &mut app,
        "ai:helm",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::viewscreen_system_id(),
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
        crate::ship::system_registry::red_alert_system_id(),
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
        crate::ship::system_registry::red_alert_system_id(),
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
        crate::ship::system_registry::red_alert_system_id(),
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
        crate::ship::system_registry::captain_system_id(),
        ControlSource::Ai,
    );
    set_activity_last_damage(&mut app, Some(0.0));

    tick(&mut app);

    assert!(
        !get_red_alert(&mut app),
        "AI must only operate red alert when the red-alert system is automated"
    );
}

// ── backfill_captain_prefers_cinematic_view ───────────────────────────────

#[test]
fn backfilled_captain_switches_to_cinematic_view() {
    let mut app = test_app();
    start_game(&mut app);
    assert_ne!(
        get_view_mode(&mut app),
        ViewMode::Cinematic,
        "fixture starts on the default camera view"
    );

    set_control_source(
        &mut app,
        crate::ship::system_registry::captain_system_id(),
        ControlSource::Ai,
    );
    tick(&mut app);

    assert_eq!(
        get_view_mode(&mut app),
        ViewMode::Cinematic,
        "an AI-operated Captain seat (a backfilled captain) must switch to Cinematic"
    );
}

#[test]
fn human_operated_captain_keeps_its_view_mode() {
    let mut app = test_app();
    start_game(&mut app);

    tick(&mut app);

    assert_ne!(
        get_view_mode(&mut app),
        ViewMode::Cinematic,
        "with the Captain seat still human-operated, nothing should switch the view"
    );
}

#[test]
fn cinematic_view_is_not_re_requested_once_reached() {
    // Regression guard against admission spam: once a backfilled ship is
    // showing Cinematic, further ticks must not keep emitting `SetView`.
    let mut app = test_app();
    start_game(&mut app);
    set_control_source(
        &mut app,
        crate::ship::system_registry::captain_system_id(),
        ControlSource::Ai,
    );
    tick(&mut app);
    assert_eq!(get_view_mode(&mut app), ViewMode::Cinematic);

    tick(&mut app);
    let admitted_len = {
        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<LocalShip>>();
        q.single(app.world()).unwrap().0.len()
    };
    assert_eq!(
        admitted_len, 0,
        "already-Cinematic must not keep re-admitting SetView every tick"
    );
}

// ── #775 inline stateless policy host integration tests ──────────────────

fn set_captain_policy(app: &mut App, policy: crate::ai::policy::AiPolicy) {
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(ship)
        .insert(CaptainAiPolicy(policy));
}

fn set_red_alert_offline(app: &mut App, offline: bool) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipSystemControlSources, With<LocalShip>>();
    let mut cs = q.single_mut(app.world_mut()).unwrap();
    cs.0.set_offline(crate::ship::system_registry::red_alert_system_id(), offline);
}

fn always_on_policy() -> crate::ai::policy::AiPolicy {
    crate::entities::config::FineSystemAiConfigToml {
        evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![crate::entities::config::FineSystemAiRuleToml {
            priority: 10,
            channel: "red_alert".into(),
            when: "true".into(),
            verb: "set_red_alert".into(),
            value: true,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
    .to_policy()
    .unwrap()
}

fn idle_policy() -> crate::ai::policy::AiPolicy {
    crate::ai::policy::AiPolicy {
        idle: true,
        ..Default::default()
    }
}

#[test]
fn authored_policy_drives_red_alert_output() {
    // An authored "always on" policy raises Red Alert under AI control even
    // with no combat activity — proving the data-authored policy, not a
    // hardcoded controller, decides the typed output (AC2/AC4).
    let mut app = test_app();
    start_game(&mut app);
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Ai,
    );
    set_captain_policy(&mut app, always_on_policy());
    // No combat activity at all.
    tick(&mut app);
    assert!(
        get_red_alert(&mut app),
        "authored always-on policy must raise Red Alert through the admitted path"
    );
}

/// Issue #891 stage 2, per-host both-directions proof for the Captain
/// host: a `flag()` guard holds the alert down while the scenario flag is
/// clear and raises it once the flag is set — through the full admitted
/// pipeline, in one app, so the only difference between the ticks is the
/// world flag.
#[test]
fn captain_flag_guard_reads_the_world_in_both_directions() {
    let flag_gated = crate::entities::config::FineSystemAiConfigToml {
        evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![crate::entities::config::FineSystemAiRuleToml {
            priority: 10,
            channel: "red_alert".into(),
            when: "flag(battle_stations)".into(),
            verb: "set_red_alert".into(),
            value: true,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    }
    .to_policy()
    .unwrap();

    let mut app = test_app();
    start_game(&mut app);
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Ai,
    );
    set_captain_policy(&mut app, flag_gated);

    // Flag CLEAR → the guard reads false and the alert stays down.
    tick(&mut app);
    assert!(
        !get_red_alert(&mut app),
        "with the world flag clear the guard must read false and hold"
    );

    // Flag SET → the SAME guard fires and the alert goes up next tick.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("battle_stations");
    tick(&mut app);
    assert!(
        get_red_alert(&mut app),
        "with the world flag set the same guard must raise Red Alert"
    );
}

#[test]
fn authored_idle_policy_takes_no_action() {
    // An explicit idle policy never raises Red Alert, even in combat —
    // proving policy-or-idle is honoured at runtime (AC1/AC2).
    let mut app = test_app();
    start_game(&mut app);
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Ai,
    );
    set_captain_policy(&mut app, idle_policy());
    set_activity_last_damage(&mut app, Some(0.0));
    tick(&mut app);
    assert!(
        !get_red_alert(&mut app),
        "idle policy must take no AI action even under recent damage"
    );
}

/// Issue #869 content fixture: a Captain policy authored in a reusable
/// FRAGMENT, composed into a hull through `includes`, drives Red Alert down
/// the ordinary admitted-command path.
///
/// The point is that nothing here knows about composition. The runtime is
/// handed one fully resolved configuration; the fragment boundary exists
/// only while the template is being loaded.
#[test]
fn a_policy_authored_in_an_included_fragment_drives_red_alert() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/fragments/composed_escort.toml",
    )
    .expect("the composed fixture hull must resolve and validate");
    let composed = config
        .captain_console
        .as_ref()
        .and_then(|c| c.ai.as_ref())
        .expect(
            "the resolved hull declares an inline Captain policy — one it never \
             authored itself, and which reached it through two levels of include",
        )
        .to_policy()
        .expect("a policy that passed load validation must convert");
    assert_ne!(
        composed,
        crate::entities::authored_ai_pins::shipped_policy_toml("captain")
            .to_policy()
            .unwrap(),
        "the fragment's policy must differ from the synthesised default, or this \
         test would still pass with composition removed entirely"
    );

    let mut app = test_app();
    start_game(&mut app);
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Ai,
    );
    set_captain_policy(&mut app, composed);

    // No combat activity whatsoever: the synthesised default stands down in
    // this state, so a raised Red Alert can only have come from the
    // fragment's unconditional rule.
    tick(&mut app);
    assert!(
        get_red_alert(&mut app),
        "the composed fragment's policy must reach ShipRedAlert through \
         AdmittedCommands, exactly as an inline policy does"
    );
}

#[test]
fn human_takeover_stops_ai_then_reacquisition_resets_from_facts() {
    // AC5 lifecycle: AI raises Red Alert in combat; a human takes the
    // console (Control Source → Human) and combat ends — the AI stops and
    // does not force the state; when the AI reacquires with no combat it
    // recomputes statelessly from current facts and stands down.
    let mut app = test_app();
    start_game(&mut app);
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Ai,
    );
    set_activity_last_damage(&mut app, Some(0.0));
    tick(&mut app);
    assert!(get_red_alert(&mut app), "AI raises Red Alert in combat");

    // Human takes over; combat ends.
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Human,
    );
    set_activity_last_damage(&mut app, None);
    set_activity_hostile_fire(&mut app, None);
    tick(&mut app);
    assert!(
        get_red_alert(&mut app),
        "AI must stop under human control and not force Red Alert off"
    );

    // AI reacquires with no combat history → stateless recompute stands down.
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Ai,
    );
    tick(&mut app);
    assert!(
        !get_red_alert(&mut app),
        "reacquired AI must recompute from current facts and stand down"
    );
}

#[test]
fn recovery_from_unavailability_resumes_policy() {
    // AC4/AC5: while the Red Alert system is unavailable (offline) the AI
    // cannot act even in combat (admission + operate_ai both deny); when it
    // recovers, the stateless policy re-evaluates and raises Red Alert.
    let mut app = test_app();
    start_game(&mut app);
    set_control_source(
        &mut app,
        crate::ship::system_registry::red_alert_system_id(),
        ControlSource::Ai,
    );
    set_red_alert_offline(&mut app, true);
    set_activity_last_damage(&mut app, Some(0.0));
    tick(&mut app);
    assert!(
        !get_red_alert(&mut app),
        "offline Red Alert system must block AI action"
    );

    // Recover: system available again → policy resumes.
    set_red_alert_offline(&mut app, false);
    tick(&mut app);
    assert!(
        get_red_alert(&mut app),
        "recovered system must let the policy raise Red Alert"
    );
}

// ── Blackboard publish tests ─────────────────────────────────────────────

use crate::core::messages::SystemId;
use crate::objectives::ObjectiveManager;
use crate::ship::damage::SystemHull;
use crate::world::server::ObjectiveManagerRes;

/// Minimal app: just publish_captain_blackboard + per-entity components.
fn bb_test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
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
        crate::ship::state::ShipRedAlert::default(),
        crate::ship::state::ShipViewMode::default(),
        ShipSystemControlSources::default(),
        crate::entities::spawner::EntitySystemHull(hull),
        crate::server_app::ShipSystemBlackboards::default(),
    ));
    app
}

fn apply_hull_damage(app: &mut App, amount: f32) {
    let mut rng = crate::sim_rng::unseeded_test_rng();
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    app.world_mut()
        .entity_mut(ship)
        .get_mut::<crate::entities::spawner::EntitySystemHull>()
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
                crate::ship::system_registry::red_alert_system_id(),
                ControlSource::Ai,
            );
        }
    }
    app.update();

    let bb = captain_bb(&mut app);
    assert!(bb.red_alert_auto);
    assert_eq!(
        bb.red_alert_system_id,
        crate::ship::system_registry::red_alert_system_id()
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
                crate::ship::system_registry::viewscreen_system_id(),
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
        crate::ship::system_registry::viewscreen_system_id()
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
            .query_filtered::<&mut crate::ship::state::ShipViewMode, With<LocalShip>>();
        if let Ok(mut vm) = q.single_mut(app.world_mut()) {
            vm.view_mode = ViewMode::Radar;
        }
    }
    app.update();
    assert_eq!(captain_bb(&mut app).view_direction, "");
}

// ── #574 objective filtering + priority boost tests ──────────────────────

use crate::core::messages::ObjectiveSource;
use crate::objectives::{UtilityConfig, ZeroGateCondition};

fn doctrine_objective_manager() -> ObjectiveManager {
    let mut mgr = ObjectiveManager::new();
    // Doctrine objective gated on red_alert — score=0 when not at red alert.
    mgr.add_full(
        "destroy-hostiles",
        "Destroy hostiles",
        false,
        vec![],
        crate::core::messages::AiDirective::None,
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

/// Build a boost resource with `id` boosted in the bb test app's local
/// (uuid-less) scope.
fn local_boost(id: &str) -> crate::server_app::CaptainPriorityBoost {
    let mut b = crate::server_app::CaptainPriorityBoost::default();
    b.toggle(crate::server_app::CaptainPriorityBoost::LOCAL_SCOPE, id);
    b
}

#[test]
fn boosted_objective_id_propagates_to_captain_bb() {
    let mut app = bb_test_app();
    app.world_mut()
        .insert_resource(local_boost("destroy-hostiles"));
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
        .insert_resource(local_boost("destroy-hostiles"));
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
fn captain_boost_does_not_leak_to_another_ships_scope() {
    // A boost set in one ship's scope must never appear in another ship's
    // scope (issue #752 scoped-objective-priority, no-leak proof).
    let mut boost = crate::server_app::CaptainPriorityBoost::default();
    boost.toggle("ship-a", "destroy-hostiles");
    assert_eq!(boost.boosted_for("ship-a"), Some("destroy-hostiles"));
    assert_eq!(
        boost.boosted_for("ship-b"),
        None,
        "ship-a's boost must not bleed into ship-b's scope"
    );
    assert!(boost.boost_arg("ship-b").is_none());
    // ...and ship-a's own consumer still sees it.
    assert_eq!(boost.boost_arg("ship-a"), Some("destroy-hostiles"));
}

#[test]
fn captain_boost_prune_objective_clears_matching_scopes_only() {
    let mut boost = crate::server_app::CaptainPriorityBoost::default();
    boost.toggle("ship-a", "gone");
    boost.toggle("ship-b", "stays");
    boost.prune_objective("gone");
    assert_eq!(
        boost.boosted_for("ship-a"),
        None,
        "removed objective pruned"
    );
    assert_eq!(
        boost.boosted_for("ship-b"),
        Some("stays"),
        "unrelated scope's boost is untouched"
    );
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
            target: crate::ship::system_registry::captain_system_id(),
            payload: SystemControlPayload::SetObjectivePriority {
                id: "destroy-hostiles".into(),
            },
        },
    );
    tick(&mut app);
    assert!(
        app.world()
            .resource::<crate::server_app::CaptainPriorityBoost>()
            .contains_objective("destroy-hostiles"),
        "the command must boost the objective in the captain's ship scope",
    );
}

#[test]
fn set_objective_priority_command_toggles_off_when_same_id() {
    let mut app = test_app();
    app.world_mut()
        .insert_resource(crate::server_app::CaptainPriorityBoost::default());
    start_game(&mut app);
    let set_priority = || ClientMessage::ControlSystem {
        target: crate::ship::system_registry::captain_system_id(),
        payload: SystemControlPayload::SetObjectivePriority {
            id: "destroy-hostiles".into(),
        },
    };
    // First send sets the boost, second send toggles it back off.
    push(&mut app, "captain", set_priority());
    tick(&mut app);
    assert!(app
        .world()
        .resource::<crate::server_app::CaptainPriorityBoost>()
        .contains_objective("destroy-hostiles"));
    push(&mut app, "captain", set_priority());
    tick(&mut app);
    assert!(
        app.world()
            .resource::<crate::server_app::CaptainPriorityBoost>()
            .is_empty(),
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
        crate::ship::system_registry::red_alert_system_id(),
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
// every ship (player + NPC) and pushes `SetRedAlert` into each ship's
// own `AdmittedCommands`, but `handle_set_red_alert` previously read
// only the LocalShip's `AdmittedCommands`, so NPC red-alert changes were
// silently dropped. This test spawns an NPC ship with AI-controlled
// red-alert, gives it recent combat activity, and asserts the NPC's own
// `ShipRedAlert` flips while the LocalShip's does not.

#[test]
fn npc_captain_ai_sets_own_red_alert_via_admitted_commands() {
    let mut app = test_app();
    start_game(&mut app);

    // Build an NPC ship with the same essential components as the
    // LocalShip, but without the LocalShip marker. Set its red-alert
    // system to AI control.
    let npc_control_sources = {
        let mut cs = ShipSystemControlSources::default();
        cs.0.set(
            crate::ship::system_registry::red_alert_system_id(),
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
            crate::core::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::ship::state::ShipRedAlert::default(),
            crate::ship::state::ShipViewMode::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            RecentCombatActivity {
                last_damage_taken: Some(0.0),
                ..Default::default()
            },
            crate::server_app::WeaponFiredThisTick::default(),
            crate::server_app::ShipAttackedThisTick::default(),
            crate::entities::spawner::EntitySystemHull(
                crate::ship::damage::SystemHull::from_config(&[(
                    crate::core::messages::SystemId("captain".into()),
                    100.0,
                )]),
            ),
            // The NPC authors its own Captain policy too — the declaration
            // is per-ENTITY and nothing is synthesised for it since #885b
            // stage 5d.
            CaptainAiPolicy(
                crate::entities::authored_ai_pins::shipped_policy_toml("captain")
                    .to_policy()
                    .expect("the shipped Captain policy decodes"),
            ),
        ))
        .id();

    // Player red-alert is Human-controlled and has no combat activity —
    // the AI must not toggle it.
    tick(&mut app);

    let npc_red_alert = app
        .world()
        .entity(npc)
        .get::<crate::ship::state::ShipRedAlert>()
        .expect("NPC must carry ShipRedAlert")
        .0;
    assert!(
        npc_red_alert,
        "operate_captain_ai should have activated the NPC's own red-alert (its AI is under combat)"
    );
    assert!(
        !get_red_alert(&mut app),
        "player's red-alert must be unaffected by NPC captain-AI set"
    );
}

#[test]
fn handle_set_red_alert_applies_admitted_commands_per_entity() {
    let mut app = test_app();
    start_game(&mut app);

    // Spawn an NPC ship without LocalShip whose red-alert system is
    // AI-held, register its `ai:` token, and send a `SetRedAlert`
    // through the inbound queue. Since #824 admission is ship-aware —
    // `AdmittedCommands` is cleared per-entity every tick and a
    // registered `ai:` token routes to its own ship's queue — so a
    // pre-seeded component would be wiped before the handler ran; the
    // wire path is the honest way in, and it exercises the routing too.
    let npc_control_sources = {
        let mut cs = ShipSystemControlSources::default();
        cs.0.set(
            crate::ship::system_registry::red_alert_system_id(),
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
            crate::core::messages::AdmittedCommands::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::ship::state::ShipRedAlert::default(),
            crate::ship::state::ShipViewMode::default(),
            crate::server_app::ShipSystemBlackboards::default(),
            RecentCombatActivity::default(),
            crate::server_app::WeaponFiredThisTick::default(),
            crate::server_app::ShipAttackedThisTick::default(),
            crate::entities::spawner::EntitySystemHull(
                crate::ship::damage::SystemHull::from_config(&[(
                    crate::core::messages::SystemId("captain".into()),
                    100.0,
                )]),
            ),
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
            target: SystemId(crate::ship::system_registry::RED_ALERT_SYSTEM_ID.to_string()),
            payload: SystemControlPayload::SetRedAlert { active: true },
        },
    );
    tick(&mut app);

    let npc_red_alert = app
        .world()
        .entity(npc)
        .get::<crate::ship::state::ShipRedAlert>()
        .unwrap()
        .0;
    assert!(
        npc_red_alert,
        "handle_set_red_alert must apply SetRedAlert from the NPC's own AdmittedCommands"
    );
    assert!(
        !get_red_alert(&mut app),
        "handle_set_red_alert must not touch the LocalShip when an NPC's AdmittedCommands drives the set"
    );
}

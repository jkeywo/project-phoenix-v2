//! Independent System availability through the canonical GM boundary.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::ship::{
    components::{ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources},
    control_source::{ControlSource, ControlSourceResolver},
    damage::SystemHull,
};
use phoenix::{
    command_admission::HostSlot,
    core::messages::SystemId,
    entities::spawner::{EntitySystemHull, EntityUuid},
    gm_action::*,
    sim_tick::SimTick,
};
use project_phoenix as phoenix;

fn grant(sequence: u64, target: &str, system: &str, disabled: bool) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-one".into(),
        correlation: GmActionId::new(format!("system-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: 42,
        order: GmActionOrder::new(HostSlot(1), sequence),
        action: GmAction::SetSystemDisabled {
            target: target.into(),
            system: SystemId(system.into()),
            disabled,
        },
    }
}
fn apply(app: &mut App, request: GmActionGrant) {
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(request)
        .unwrap();
    app.world_mut().run_system_once(apply_due_actions).unwrap();
}
fn bare() -> (App, Entity) {
    let mut app = App::new();
    app.insert_resource(SimTick(42))
        .init_resource::<SimulationPaused>()
        .init_resource::<GmActionJournal>()
        .init_resource::<GmActionLog>();
    let config = phoenix::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "captain"
name = "Captain"
description = "Command"
rank = "Captain"
[[system]]
id = "red-alert"
kind = "red_alert"
station = "captain"
[[system]]
id = "other-alert"
kind = "red_alert"
station = "captain"
"#,
        &["red_alert"],
    )
    .unwrap();
    let mut sources = ControlSourceResolver::new();
    sources.set(SystemId("red-alert".into()), ControlSource::Ai);
    let ship = app
        .world_mut()
        .spawn((
            phoenix::server_app::Ship,
            EntityUuid("ship-a".into()),
            ShipConfigComponent(config),
            ActiveStationRatings::default(),
            ShipSystemControlSources(sources),
            EntitySystemHull(SystemHull::from_config(&[
                (SystemId("red-alert".into()), 100.0),
                (SystemId("other-alert".into()), 100.0),
            ])),
        ))
        .id();
    (app, ship)
}

#[test]
fn healthy_damaged_and_destroyed_systems_keep_hp_and_normal_admission_policy() {
    let sid = SystemId("red-alert".into());
    for hp in [100.0, 45.0, 5.0, 0.0] {
        let (mut app, ship) = bare();
        app.world_mut()
            .get_mut::<EntitySystemHull>(ship)
            .unwrap()
            .0
            .set_hp(&sid, hp);
        app.world_mut()
            .run_system_once(phoenix::ship::damage_sync::sync_console_damage_tiers)
            .unwrap();
        let before = app.world().get::<EntitySystemHull>(ship).unwrap().0.clone();
        let baseline = app
            .world()
            .get::<ShipSystemControlSources>(ship)
            .unwrap()
            .0
            .policy_for(&sid);
        apply(&mut app, grant(1, "ship-a", "red-alert", true));
        apply(&mut app, grant(2, "ship-a", "red-alert", true));
        let sources = &app.world().get::<ShipSystemControlSources>(ship).unwrap().0;
        assert!(!sources.policy_for(&sid).accept_human_input);
        assert!(!sources.policy_for(&sid).operate_ai);
        assert!(!sources.policy_for(&sid).coordinate);
        for token in ["ai:red-alert", phoenix::console_bridge::LOCAL_CONSOLE_TOKEN] {
            assert!(!phoenix::command_admission::policy::is_command_authorized(
                token,
                &sid,
                &phoenix::core::messages::SystemControlPayload::SetRedAlert { active: true },
                app.world().get::<ShipSystemControlSources>(ship).unwrap(),
                &phoenix::lobby::Sessions(phoenix::lobby::session::SessionManager::new()),
                &app.world().get::<ShipConfigComponent>(ship).unwrap().0,
                None,
            ));
        }
        assert!(
            sources
                .policy_for(&SystemId("other-alert".into()))
                .coordinate
        );
        // Damage synchronisation and a new rating cannot erase the GM latch.
        app.world_mut()
            .run_system_once(phoenix::ship::damage_sync::sync_console_damage_tiers)
            .unwrap();
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(ship)
            .unwrap()
            .0
            .set(sid.clone(), ControlSource::Human);
        let sources = &app.world().get::<ShipSystemControlSources>(ship).unwrap().0;
        assert!(sources.is_gm_disabled(&sid));
        assert!(!sources.policy_for(&sid).accept_human_input);
        apply(&mut app, grant(3, "ship-a", "red-alert", false));
        apply(&mut app, grant(4, "ship-a", "red-alert", false));
        assert_eq!(app.world().get::<EntitySystemHull>(ship).unwrap().0, before);
        assert_eq!(
            app.world()
                .get::<ShipSystemControlSources>(ship)
                .unwrap()
                .0
                .policy_for(&sid)
                .coordinate,
            baseline.coordinate
        );
        assert_eq!(
            app.world()
                .resource::<GmActionLog>()
                .entries()
                .iter()
                .map(|r| r.outcome)
                .collect::<Vec<_>>(),
            [
                GmActionOutcome::Applied,
                GmActionOutcome::NoOp,
                GmActionOutcome::Applied,
                GmActionOutcome::NoOp
            ]
        );
    }
}

#[test]
fn repair_does_not_restore_gm_latch_and_stale_or_same_tick_requests_keep_exact_results() {
    let (mut app, ship) = bare();
    let sid = SystemId("red-alert".into());
    apply(&mut app, grant(1, "ship-a", "red-alert", true));
    app.world_mut()
        .get_mut::<EntitySystemHull>(ship)
        .unwrap()
        .0
        .set_hp(&sid, 0.0);
    app.world_mut()
        .get_mut::<EntitySystemHull>(ship)
        .unwrap()
        .0
        .restore(&sid, 100.0);
    app.world_mut()
        .run_system_once(phoenix::ship::damage_sync::sync_console_damage_tiers)
        .unwrap();
    assert!(app
        .world()
        .get::<ShipSystemControlSources>(ship)
        .unwrap()
        .0
        .is_gm_disabled(&sid));
    apply(&mut app, grant(2, "ship-a", "red-alert", false));
    apply(&mut app, grant(3, "ship-a", "red-alert", true));
    apply(&mut app, grant(4, "ship-a", "unknown", false));
    app.world_mut().despawn(ship);
    apply(&mut app, grant(5, "ship-a", "red-alert", false));
    let facts = app.world().resource::<GmActionLog>().entries();
    assert_eq!(facts[3].reason, Some(GmActionRefusalReason::UnknownSystem));
    assert_eq!(facts[4].reason, Some(GmActionRefusalReason::UnknownEntity));
    assert!(facts
        .iter()
        .all(|r| r.target.as_deref() == Some("ship-a") && r.effect_scope.is_some()));
    assert_eq!(facts[4].action_kind, GmActionKind::SystemRestore);
}

#[test]
fn system_latch_ingress_is_bounded_and_does_not_accept_hp_or_toggle_fields() {
    let wire = r#"{"operator_id":"gm","correlation":"one","action":"set_system_disabled","target":"ship-a","system":"red-alert","disabled":true}"#;
    assert!(phoenix::core::codec::decode_gm_action_request(wire).is_some());
    for bad in [
        wire.replace("\"disabled\":true", "\"toggle\":true"),
        wire.replace("\"disabled\":true", "\"disabled\":true,\"hp\":100"),
        wire.replace("red-alert", ""),
    ] {
        assert!(phoenix::core::codec::decode_gm_action_request(&bad).is_none());
    }
}

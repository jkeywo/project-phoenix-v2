//! Owner-level actual Radio proof, separate from joined player/whole-host tests.
use super::*;
use crate::{
    core::messages::{
        AdmittedCommand, AdmittedCommands, CaptainBlackboard, SystemControlPayload as Control,
    },
    entities::spawner::spawn_entity,
    sim_tick::SimTick,
    world_id::InstallWorldIdMint,
};
use bevy::ecs::system::RunSystemOnce;

fn authored() -> crate::entities::config::EntityConfig {
    let resolved = crate::entities::include_resolve::resolve_from_disk(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut raw: toml::Value = toml::from_str(&resolved.toml).unwrap();
    let console = raw["console"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|console| console["id"].as_str() == Some("comms"))
        .unwrap();
    console
        .as_table_mut()
        .unwrap()
        .entry("feed")
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .unwrap()
        .push(toml::from_str("source='captain'\nkind='Captain'").unwrap());
    let radio = raw["comms"]["radio"].as_table_mut().unwrap();
    radio.insert("traffic".into(), toml::from_str(
        "listening={simultaneous_channels=1, priority=['public','fleet']}\nalert={source='captain', channel='public', minimum_spacing_ticks=10, routine_text='We have selected routine alert.', red_alert_text='We have selected red alert.'}"
    ).unwrap());
    radio.insert(
        "capture".into(),
        toml::from_str("max_level=1\nticks_per_level=2").unwrap(),
    );
    crate::entities::config::EntityConfig::from_toml(&toml::to_string(&raw).unwrap()).unwrap()
}

fn fixture() -> (App, Entity, Entity) {
    let mut app = App::new();
    app.insert_resource(SimTick(10))
        .insert_resource(Time::<Fixed>::from_hz(30.0))
        .init_resource::<super::super::server::CommsInboxRes>()
        .init_resource::<super::super::server::CommsRuntime>()
        .init_resource::<crate::server_app::SimOutbox>();
    app.world_mut()
        .insert_world_id_mint(crate::world_id::WorldIdMint::default());
    let config = authored();
    let source = spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "source".into(),
        None,
    );
    let receiver = spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "receiver".into(),
        None,
    );
    app.world_mut().flush();
    (app, source, receiver)
}

fn command(app: &mut App, source: Entity, payload: Control) {
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0 = vec![AdmittedCommand {
        target: SystemId("comms".into()),
        payload,
        response_token: None,
        feedback_correlation: None,
    }];
}
fn run(app: &mut App) {
    app.world_mut()
        .run_system_once(crate::ship::energy_runtime::begin_work_interval)
        .unwrap();
    app.world_mut()
        .run_system_once(super::super::radio::tick_radio)
        .unwrap();
    app.world_mut().resource_mut::<SimTick>().0 += 1;
}
fn received(app: &App) -> usize {
    app.world()
        .resource::<super::super::server::CommsInboxRes>()
        .0
        .iter()
        .filter(|message| message.is_for_ship(Some("receiver")))
        .count()
}
fn sample_board(app: &mut App, source: Entity, red: bool, sampled: u64) {
    // Explicit permitted-publication precondition for this owner-level fixture;
    // full-host tests must earn it through the real Captain publisher/controls.
    let owner = SystemId("red-alert".into());
    app.world_mut()
        .get_mut::<ShipSystemBlackboards>(source)
        .unwrap()
        .0
        .insert(
            SystemId("captain".into()),
            SystemBlackboard::Captain(CaptainBlackboard {
                red_alert: red,
                red_alert_system_id: owner.clone(),
                alert_observation: Some(alert_observation(sampled, &owner)),
                ..Default::default()
            }),
        );
}

#[test]
fn real_receive_selection_drops_one_shot_without_retrospective_delivery() {
    let (mut app, source, receiver) = fixture();
    command(
        &mut app,
        receiver,
        Control::SetCommsListeningOrder {
            priority: vec!["fleet".into(), "public".into()],
        },
    );
    command(
        &mut app,
        source,
        Control::SendCommsText {
            channel: "public".into(),
            audience: phoenix_model::comms_channel::CommsAudience::All,
            body: "First actual public emission".into(),
        },
    );
    run(&mut app);
    assert_eq!(received(&app), 0);
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0
        .clear();
    command(
        &mut app,
        receiver,
        Control::SetCommsListeningOrder {
            priority: vec!["public".into(), "fleet".into()],
        },
    );
    run(&mut app);
    assert_eq!(
        received(&app),
        0,
        "an archive entry is not another emission"
    );
    command(
        &mut app,
        source,
        Control::SendCommsText {
            channel: "public".into(),
            audience: phoenix_model::comms_channel::CommsAudience::All,
            body: "Second actual public emission".into(),
        },
    );
    run(&mut app);
    assert_eq!(received(&app), 1);
}

#[test]
fn capture_observes_only_original_emissions_on_current_paid_receive_coverage() {
    let (mut app, source, receiver) = fixture();
    let captures = |app: &App| {
        app.world()
            .get::<super::super::capture::ShipCommsCapture>(receiver)
            .unwrap()
            .0
            .payloads()
            .count()
    };
    let emit = |app: &mut App, body: &str| {
        command(
            app,
            source,
            Control::SendCommsText {
                channel: "fleet".into(),
                audience: phoenix_model::comms_channel::CommsAudience::All,
                body: body.into(),
            },
        );
    };
    emit(&mut app, "First actual protected emission");
    run(&mut app);
    assert_eq!(captures(&app), 0, "unmonitored traffic is not a capture");
    app.world_mut()
        .get_mut::<AdmittedCommands>(source)
        .unwrap()
        .0
        .clear();
    command(
        &mut app,
        receiver,
        Control::SetCommsListeningOrder {
            priority: vec!["fleet".into(), "public".into()],
        },
    );
    run(&mut app);
    assert_eq!(
        captures(&app),
        0,
        "changing coverage cannot capture an archive"
    );
    emit(&mut app, "Second actual protected emission");
    run(&mut app);
    assert_eq!(
        captures(&app),
        1,
        "a new paid covered emission is genuine evidence"
    );
}

#[test]
fn successful_report_commits_only_current_permitted_paid_semantic_and_never_repeats() {
    let (mut app, source, _) = fixture();
    for fault in 0..4 {
        let now = app.world().resource::<SimTick>().0;
        sample_board(
            &mut app,
            source,
            false,
            if fault == 1 { now - 2 } else { now },
        );
        if fault == 0 {
            app.world_mut()
                .get_mut::<ShipSystemBlackboards>(source)
                .unwrap()
                .0
                .remove(&SystemId("captain".into()));
        } else if fault == 2 {
            app.world_mut()
                .get_mut::<crate::ship::power::ShipPowerSystem>(source)
                .unwrap()
                .0
                .network_mut()
                .consumers
                .get_mut(&SystemId("comms".into()))
                .unwrap()
                .energy = 0.0;
        } else {
            app.world_mut()
                .get_mut::<crate::ship::power::ShipPowerSystem>(source)
                .unwrap()
                .0
                .network_mut()
                .consumers
                .get_mut(&SystemId("comms".into()))
                .unwrap()
                .energy = 1.0;
        }
        command(&mut app, source, Control::ReportCommsAlert);
        run(&mut app);
        assert_eq!(received(&app), usize::from(fault == 3));
        assert_eq!(
            app.world()
                .get::<ShipCommsTraffic>(source)
                .unwrap()
                .state
                .last_report
                .is_some(),
            fault == 3
        );
    }
    let saved = app
        .world()
        .get::<ShipCommsTraffic>(source)
        .unwrap()
        .state
        .clone();
    let now = app.world().resource::<SimTick>().0;
    sample_board(&mut app, source, false, now);
    run(&mut app);
    assert_eq!(
        received(&app),
        1,
        "fresh unchanged intent does not chatter again"
    );
    assert_eq!(
        app.world().get::<ShipCommsTraffic>(source).unwrap().state,
        saved
    );
}

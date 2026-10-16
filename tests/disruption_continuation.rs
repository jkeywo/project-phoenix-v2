//! Real Comms-hosted shared EMP bolt: acquisition/admission/payment/conditions.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{
        AdmittedCommands, ClientMessage, SystemBlackboard, SystemControlPayload as Control,
        SystemId,
    },
    entities::spawner::{EntityId, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::{server::InboundMessage, Sessions},
    server_app::{LocalShip, ShipSystemBlackboards},
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    ship_plugin::{HumanSeekingHosts, ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const BANK: &str = "blaster-disruptor";
#[derive(Resource, Default)]
struct AiWitness(bool);
fn observe_ai(
    rows: Query<(&AdmittedCommands, &ShipSystemControlSources), With<LocalShip>>,
    mut witness: ResMut<AiWitness>,
) {
    for (commands, sources) in &rows {
        witness.0 |= sources.0.source_for(&id(BANK))
            == project_phoenix::ship::control_source::ControlSource::Ai
            && commands.for_target(BANK).any(|command| {
                matches!(
                    command.payload,
                    Control::FireBlaster | Control::ChargeBlasterStart
                )
            });
    }
}
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn steps(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}
fn send(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: token.into(),
        msg,
    });
}
fn boot() -> (App, Entity, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/disruption-world.toml".into(),
        ship_path: "assets/entities/disruptor_cruiser.toml".into(),
        deterministic: true,
        seed: Some(15621721),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<AiWitness>();
    app.add_systems(
        FixedUpdate,
        observe_ai.in_set(project_phoenix::sim_sets::SimSet::Publish),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    steps(&mut app, 10);
    let source = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    let target = app
        .world_mut()
        .query::<(Entity, &EntityId)>()
        .iter(app.world())
        .find_map(|(entity, id)| (id.0 == "disruption-target").then_some(entity))
        .unwrap();
    let stations = app
        .world()
        .get::<ShipConfigComponent>(source)
        .unwrap()
        .0
        .stations
        .clone();
    for station in stations {
        let token = format!("disruption-{}", station.id.0);
        for msg in [
            ClientMessage::Identify {
                token: token.clone(),
                name: token.clone(),
            },
            ClientMessage::SelectStation {
                station: station.id.0,
            },
            ClientMessage::SetReady { ready: true },
            ClientMessage::SetStationRating {
                rating_name: station.ratings[0].name.clone(),
            },
        ] {
            send(&mut app, &token, msg);
            steps(&mut app, 4);
        }
    }
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(source)
            .unwrap()
            .0
            .source_for(&id(BANK)),
        project_phoenix::ship::control_source::ControlSource::Human
    );
    (app, source, target)
}
fn control(app: &mut App, source: Entity, owner: &str, payload: Control) {
    let sessions = app.world().resource::<Sessions>();
    let sources = app.world().get::<ShipSystemControlSources>(source).unwrap();
    let config = &app.world().get::<ShipConfigComponent>(source).unwrap().0;
    let token = sessions
        .0
        .players()
        .iter()
        .find(|player| {
            project_phoenix::command_admission::policy::is_command_authorized(
                &player.token,
                &id(owner),
                &payload,
                sources,
                sessions,
                config,
                app.world().get::<HumanSeekingHosts>(source),
            )
        })
        .unwrap_or_else(|| panic!("actual Human authority for {owner}"))
        .token
        .clone();
    send(
        app,
        &token,
        ClientMessage::ControlSystem {
            target: id(owner),
            payload,
        },
    );
    steps(app, 4);
}
fn alias(app: &App, source: Entity) -> Option<String> {
    let boards = app.world().get::<ShipSystemBlackboards>(source)?;
    let config = &app.world().get::<ShipConfigComponent>(source)?.0;
    let permitted = config.observations_for_system(&id(BANK), &boards.0);
    let SystemBlackboard::TacticalRadar(board) = permitted.get(&id("tactical-radar"))? else {
        return None;
    };
    board
        .tracks
        .as_ref()?
        .iter()
        .find(|track| {
            track.selectable
                && !track.predicted
                && track
                    .identity
                    .as_ref()
                    .is_some_and(|identity| identity.name == "disruption-target")
        })
        .map(|track| track.contact_id.clone())
}
fn wait(app: &mut App, mut predicate: impl FnMut(&App) -> bool) {
    for _ in 0..600 {
        if predicate(app) {
            return;
        }
        app.update();
    }
    panic!("actual supported disruption must progress");
}
fn energy(app: &App, source: Entity) -> f32 {
    app.world()
        .get::<ShipPowerSystem>(source)
        .unwrap()
        .0
        .network()
        .consumers[&id(BANK)]
        .energy
}
fn emp(app: &App, target: Entity) -> Option<phoenix_model::emp::ComponentEmpState> {
    app.world()
        .get::<ShipDamage>(target)?
        .state
        .components
        .values()
        .flat_map(|components| components.values())
        .find_map(|component| component.emp.clone())
}
fn factor(app: &App, target: Entity) -> f32 {
    app.world().get::<ShipDamage>(target).unwrap().factor(
        &id("sensor-radar"),
        phoenix_model::damage::DamageBinding::Operation,
    )
}
fn same(a: &App, b: &App, step: usize) {
    assert_eq!(
        serde_json::to_value(capture(a.world())).unwrap(),
        serde_json::to_value(capture(b.world())).unwrap(),
        "full capture at {step}"
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest at {step}"
    );
}
#[test]
fn human_comms_bank_launches_real_paid_emp_and_active_condition_resumes_json360() {
    let (mut live, source, target) = boot();
    wait(&mut live, |app| alias(app, source).is_some());
    assert_eq!(factor(&live, target), 1.0);
    assert!(emp(&live, target).is_none());
    let handle = alias(&live, source).unwrap();
    control(
        &mut live,
        source,
        BANK,
        Control::SetWeaponTarget { uuid: Some(handle) },
    );
    let before = energy(&live, source);
    control(&mut live, source, BANK, Control::ChargeBlasterStart);
    assert!(
        energy(&live, source) < before,
        "ordinary admitted source spends actual bank reserve"
    );
    wait(&mut live, |app| emp(app, target).is_some());
    let actual = emp(&live, target).unwrap();
    assert!(actual.effect_until_tick > actual.applied_tick);
    assert_eq!(
        factor(&live, target),
        0.0,
        "actual hit disables existing eligible Operation component"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let uuid = live.world().get::<EntityUuid>(target).unwrap().0.clone();
    let (mut resumed, _, _) = boot();
    assert!(ready_to_restore(resumed.world(), &saved));
    let original = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut invalid = saved.clone();
        let damage = invalid
            .entities
            .iter_mut()
            .find(|row| row.uuid == uuid)
            .unwrap()
            .physical_damage
            .as_mut()
            .unwrap();
        let receipt = damage
            .components
            .values_mut()
            .flat_map(|components| components.values_mut())
            .find_map(|component| component.emp.as_mut())
            .unwrap();
        match fault {
            0 => receipt.effect_until_tick = receipt.applied_tick,
            1 => receipt.resistance_stacks = 0,
            _ => {
                receipt.applied_tick = saved.tick + 1;
                receipt.effect_until_tick = saved.tick + 10;
                receipt.next_decay_tick = saved.tick + 10000;
            }
        }
        assert!(!restore(resumed.world_mut(), &invalid).is_complete());
        assert_eq!(capture(resumed.world()), original);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    assert!(restore(resumed.world_mut(), &saved).is_complete());
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 360);
    assert_eq!(
        factor(&live, target),
        1.0,
        "EMP expires without repair or an actor takeover"
    );
    assert!(emp(&live, target).is_some_and(|receipt| receipt.resistance_stacks > 0));
}

#[test]
fn ordinary_individual_bank_ai_uses_same_permitted_target_and_returns_control_after_afk() {
    let (mut app, source, target) = boot();
    wait(&mut app, |app| alias(app, source).is_some());
    let handle = alias(&app, source).unwrap();
    control(
        &mut app,
        source,
        BANK,
        Control::SetWeaponTarget { uuid: Some(handle) },
    );
    control(
        &mut app,
        source,
        "red-alert",
        Control::SetRedAlert { active: true },
    );
    // Backfill can act on its first ordinary interval. Observe the reserve
    // before handing authority over, rather than after the first paid launch.
    let before = energy(&app, source);
    send(
        &mut app,
        "disruption-comms",
        ClientMessage::SetAfk { afk: true },
    );
    steps(&mut app, 4);
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(source)
            .unwrap()
            .0
            .source_for(&id(BANK)),
        project_phoenix::ship::control_source::ControlSource::Ai
    );
    wait(&mut app, |app| emp(app, target).is_some());
    assert!(app.world().resource::<AiWitness>().0);
    assert!(energy(&app, source) < before);
    send(
        &mut app,
        "disruption-comms",
        ClientMessage::SetAfk { afk: false },
    );
    steps(&mut app, 4);
    control(
        &mut app,
        source,
        BANK,
        Control::SetWeaponTarget { uuid: None },
    );
    assert_eq!(
        app.world()
            .get::<ShipSystemControlSources>(source)
            .unwrap()
            .0
            .source_for(&id(BANK)),
        project_phoenix::ship::control_source::ControlSource::Human
    );
    wait(&mut app, |app| factor(app, target) == 1.0);
    assert!(emp(&app, target).is_some_and(|receipt| receipt.resistance_stacks > 0));
}

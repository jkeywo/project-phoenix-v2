//! Real paid transponder -> independently acquired identity -> ordinary controls.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::{
    core::messages::{ClientMessage, SystemControlPayload as Control, SystemId},
    entities::spawner::{EntityName, EntityUuid},
    headless::{build_headless_app, HeadlessArgs},
    lobby::server::InboundMessage,
    science::{
        broad::BroadTracks,
        identity::{IdentityEmissions, ShipIdentityEmitters},
    },
    server_app::LocalShip,
    ship::power::ShipPowerSystem,
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
    sim_digest::world_digest,
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::WorldConfig,
};
const OWNER: &str = "identity-emitter";
const TOKEN: &str = "identity-operator";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
#[derive(Resource, Default, Debug)]
struct PaidWitness {
    tick: Option<u64>,
    work: f64,
    claim: Option<String>,
}
fn observe(
    tick: Res<project_phoenix::sim_tick::SimTick>,
    owners: Query<(&ShipPowerSystem, &IdentityEmissions), With<LocalShip>>,
    mut witness: ResMut<PaidWitness>,
) {
    *witness = PaidWitness {
        tick: Some(tick.0),
        ..Default::default()
    };
    for (power, emissions) in &owners {
        witness.work = power.0.emitted_work(&id(OWNER));
        witness.claim = emissions.claim(tick.0).map(|claim| claim.callsign.clone());
    }
}
fn boot() -> (App, Entity) {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "tests/fixtures/identity-world.toml".into(),
        ship_path: "assets/entities/identity_cruiser.toml".into(),
        deterministic: true,
        seed: Some(1562171),
        ..Default::default()
    })
    .unwrap();
    app.init_resource::<PaidWitness>();
    app.add_systems(
        FixedUpdate,
        observe
            .in_set(project_phoenix::sim_sets::SimSet::Input)
            .after_ignore_deferred(project_phoenix::sim_sets::FixedStep::TickJamming)
            .before_ignore_deferred(project_phoenix::sim_sets::FixedStep::RefreshJammedCommsRange),
    );
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    let actor = app
        .world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap();
    (app, actor)
}
fn send(app: &mut App, msg: ClientMessage) {
    app.world_mut().write_message(InboundMessage {
        token: TOKEN.into(),
        msg,
    });
}
fn claim(app: &mut App) {
    for msg in [
        ClientMessage::Identify {
            token: TOKEN.into(),
            name: TOKEN.into(),
        },
        ClientMessage::SelectStation {
            station: "comms".into(),
        },
        ClientMessage::SetReady { ready: true },
        ClientMessage::SetStationRating {
            rating_name: "Std".into(),
        },
    ] {
        send(app, msg);
        for _ in 0..4 {
            app.update();
        }
    }
}
fn control(app: &mut App, enabled: bool) {
    send(
        app,
        ClientMessage::ControlSystem {
            target: id(OWNER),
            payload: Control::SetIdentityEmitterEnabled { enabled },
        },
    );
    for _ in 0..4 {
        app.update();
    }
}
fn paid(app: &App) -> bool {
    let value = app.world().resource::<PaidWitness>();
    value.tick.and_then(|tick| tick.checked_add(1))
        == Some(
            app.world()
                .resource::<project_phoenix::sim_tick::SimTick>()
                .0,
        )
        && value.work > 0.0
        && value.claim.as_deref() == Some("Transit courier")
}
fn picture(app: &mut App, source: &str) -> Option<(String, String)> {
    let mut query = app.world_mut().query::<(&EntityUuid, &BroadTracks)>();
    for (uuid, tracks) in query.iter(app.world()) {
        if uuid.0 == source {
            continue;
        }
        let Some(state) = tracks.0.get(&id("sensor-radar")) else {
            continue;
        };
        for (alias, track) in &state.tracks {
            if state.resolve_current(alias) == Some(source) {
                return track
                    .identity
                    .as_ref()
                    .map(|identity| (alias.clone(), identity.name.clone()));
            }
        }
    }
    None
}
fn await_identity(app: &mut App, source: &str, name: &str) -> String {
    for _ in 0..1800 {
        if let Some((alias, current)) = picture(app, source) {
            if current == name {
                return alias;
            }
        }
        app.update();
    }
    panic!(
        "ordinary physical Broad never measured expected identity {name:?}; witness {:?}",
        app.world().resource::<PaidWitness>()
    );
}
fn same(a: &App, b: &App, step: usize) {
    assert_eq!(
        capture(a.world()),
        capture(b.world()),
        "complete capture at {step}"
    );
    assert_eq!(
        world_digest(a.world()),
        world_digest(b.world()),
        "digest at {step}"
    );
}
#[test]
fn actual_ai_human_identity_keeps_physical_contact_and_resumes_360_complete_steps() {
    let (mut live, actor) = boot();
    let source = live.world().get::<EntityUuid>(actor).unwrap().0.clone();
    assert_eq!(
        live.world().get::<EntityName>(actor).unwrap().0,
        "True survey ship"
    );
    for _ in 0..240 {
        let config = &live.world().get::<ShipConfigComponent>(actor).unwrap().0;
        if live
            .world()
            .get::<ShipIdentityEmitters>(actor)
            .unwrap()
            .enabled(config, &id(OWNER))
        {
            break;
        }
        live.update();
    }
    let config = &live.world().get::<ShipConfigComponent>(actor).unwrap().0;
    assert!(
        live.world()
            .get::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .policy_for(&id(OWNER))
            .operate_ai
    );
    assert!(
        live.world()
            .get::<ShipIdentityEmitters>(actor)
            .unwrap()
            .enabled(config, &id(OWNER)),
        "ordinary authored tree must enable from false initial"
    );
    let alias = await_identity(&mut live, &source, "Transit courier");
    assert!(paid(&live));
    claim(&mut live);
    control(&mut live, false);
    assert!(!live
        .world()
        .get::<ShipIdentityEmitters>(actor)
        .unwrap()
        .enabled(
            &live.world().get::<ShipConfigComponent>(actor).unwrap().0,
            &id(OWNER)
        ));
    assert_eq!(live.world().resource::<PaidWitness>().work, 0.0);
    assert!(live.world().resource::<PaidWitness>().claim.is_none());
    assert_eq!(
        await_identity(&mut live, &source, "True survey ship"),
        alias,
        "normal refresh preserves earned real-body alias"
    );
    control(&mut live, true);
    assert_eq!(await_identity(&mut live, &source, "Transit courier"), alias);
    assert!(paid(&live));
    assert_eq!(
        live.world().get::<EntityName>(actor).unwrap().0,
        "True survey ship"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let (mut resumed, restored) = boot();
    claim(&mut resumed);
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut bad = saved.clone();
        let row = bad
            .entities
            .iter_mut()
            .find(|row| row.uuid == source)
            .unwrap();
        if fault == 0 {
            row.identity_emitters = None;
        } else {
            let state = row.identity_emitters.as_mut().unwrap();
            if fault == 1 {
                state.0.get_mut(&id(OWNER)).unwrap().0.insert(
                    "enabled".into(),
                    project_phoenix::ship::composition::CommandChannelValue::EntityTarget(None),
                );
            } else {
                let value = state.0.remove(&id(OWNER)).unwrap();
                state.0.insert(id("foreign-owner"), value);
            }
        }
        assert!(!restore(resumed.world_mut(), &bad).is_complete());
        assert_eq!(capture(resumed.world()), before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    assert!(
        resumed
            .world()
            .get::<IdentityEmissions>(restored)
            .unwrap()
            .claim(saved.tick)
            .is_none(),
        "restore cannot grant prepaid emissions"
    );
    for step in 0..360 {
        same(&live, &resumed, step);
        live.update();
        resumed.update();
        assert!(paid(&live), "live finite work at {step}");
        assert!(paid(&resumed), "restored finite work at {step}");
    }
    same(&live, &resumed, 360);
    assert_eq!(
        picture(&mut resumed, &source).unwrap(),
        (alias, "Transit courier".into())
    );
}

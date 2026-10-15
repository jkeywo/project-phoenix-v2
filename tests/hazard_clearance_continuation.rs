//! Actual hazard restoration + paid report + authored clearance, full-host continuation.
//! Incident setup uses the ordinary physical Damage owner; this is not authored
//! encounter pacing, weapon admission, or a measured two-/six-player trial.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use phoenix_model::{
    activity::{ActivityExecutionIdentity, ActivityOutcome},
    damage::{DamageHit, DamageProfile, HitSample},
};
use project_phoenix::{
    comms::server::CommsRuntime,
    core::messages::SystemId,
    headless::{build_headless_app, HeadlessArgs},
    server_app::LocalShip,
    ship::{compartment_damage::ShipDamage, power::ShipPowerSystem},
    sim_digest::world_digest,
    sim_rng::{with_event_generator, SimRng},
    sim_tick::sim_tick_period,
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::{config::WorldConfig, server::WorldContentRuntime},
};
const WORLD: &str = "tests/fixtures/hazard-clearance-world.toml";
fn boot() -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(1562224),
        ..Default::default()
    })
    .unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..10 {
        app.update();
    }
    app
}
fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}
fn hp(app: &App, actor: Entity) -> f64 {
    app.world()
        .get::<ShipDamage>(actor)
        .unwrap()
        .state
        .compartments["engineering"]
        .hp
}
fn incident(app: &mut App, actor: Entity) -> f64 {
    let healthy = hp(app, actor);
    app.world_mut().resource_scope(|world, rng: Mut<SimRng>| {
        let mut damage = world.get_mut::<ShipDamage>(actor).unwrap();
        let deck = damage
            .definition
            .interior
            .compartments
            .iter()
            .find(|room| room.id == "engineering")
            .unwrap()
            .deck
            .clone();
        let result = with_event_generator(Some(&rng), "hazard-clearance-incident", 0, |rng| {
            damage.apply_hit(
                0.0,
                DamageHit {
                    incoming: [1.0, 0.0],
                    profile: DamageProfile::default(),
                    payload: 1.0,
                    aim: Some("engineering".into()),
                    source: "hazard-clearance-incident".into(),
                    sample: Some(HitSample {
                        deck,
                        profile_fraction: 0.5,
                    }),
                },
                rng,
            )
        })
        .unwrap();
        assert!(result
            .deposits
            .iter()
            .any(|row| row.compartment == "engineering" && row.structural_loss > 0.0));
    });
    assert!(hp(app, actor) < healthy);
    healthy
}
fn same(live: &App, resumed: &App, step: usize) {
    assert!(
        capture(live.world()) == capture(resumed.world()),
        "complete capture differs at step {step}"
    );
    assert_eq!(
        world_digest(live.world()),
        world_digest(resumed.world()),
        "digest at step {step}"
    );
}
#[test]
fn actual_hazard_and_explicit_clearance_keep_original_clocks_through_1200_complete_steps() {
    let mut live = boot();
    let actor = local(&mut live);
    let healthy = incident(&mut live, actor);
    let mut checkpoint = None;
    for _ in 0..240 {
        live.update();
        if live
            .world()
            .resource::<CommsRuntime>()
            .pending_ai_responses
            .iter()
            .any(|(key, row)| {
                key.planned_request
                    && row.request_reading.is_some_and(|reading| {
                        reading.remaining_ticks > 0
                            && reading.remaining_ticks < reading.required_ticks
                    })
            })
        {
            checkpoint = Some(capture(live.world()));
            break;
        }
    }
    let saved: PhoenixSnapshot = serde_json::from_slice(
        &serde_json::to_vec(
            &checkpoint
                .expect("actual ordinary Comms AI partially reads the released clearance request"),
        )
        .unwrap(),
    )
    .unwrap();
    let world = saved.scenario.as_ref().unwrap();
    let permission = &world.situations.instances["htn/hazard-clearance/root/permission"];
    assert!(permission.accepted.is_none());
    assert!(permission.activations.is_empty());
    let deadline = world
        .plan_choice
        .as_ref()
        .unwrap()
        .temporal
        .as_ref()
        .unwrap()
        .deadline_tick;
    let mut resumed = boot();
    for _ in 0..240 {
        if ready_to_restore(resumed.world(), &saved) {
            break;
        }
        resumed.update();
    }
    assert!(ready_to_restore(resumed.world(), &saved));
    let report = restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    let mut paid_repair = false;
    let mut paid_comms = false;
    for step in 0..1200 {
        same(&live, &resumed, step);
        let power = &live.world().get::<ShipPowerSystem>(actor).unwrap().0;
        paid_repair |= power.emitted_work(&SystemId("repair".into())) > 0.0;
        paid_comms |= power.emitted_work(&SystemId("comms".into())) > 0.0;
        live.update();
        resumed.update();
    }
    same(&live, &resumed, 1200);
    assert!(
        paid_repair && paid_comms,
        "both ordinary owners must buy their actual work"
    );
    assert!(
        hp(&live, actor) >= healthy,
        "actual structural restoration must occur"
    );
    let state = capture(live.world());
    let world = state.scenario.as_ref().unwrap();
    assert_eq!(
        world
            .plan_choice
            .as_ref()
            .unwrap()
            .temporal
            .as_ref()
            .unwrap()
            .deadline_tick,
        deadline
    );
    let hazard = &world.situations.instances["htn/hazard-clearance/root/hazard"];
    assert!(hazard
        .accepted
        .as_ref()
        .is_some_and(|scope| matches!(scope.identity, ActivityExecutionIdentity::Physical { .. })));
    assert!(matches!(
        hazard.achieved.as_ref().unwrap().outcome,
        ActivityOutcome::RepairAchieved { .. }
    ));
    let permission = &world.situations.instances["htn/hazard-clearance/root/permission"];
    assert!(permission.accepted.as_ref().is_some_and(|scope| matches!(
        scope.identity,
        ActivityExecutionIdentity::Communication { .. }
    )));
    let receipt_tick = permission.settled.as_ref().unwrap().tick;
    assert!(receipt_tick <= permission.activation_ticks["reported"]);
    assert!(permission.activation_ticks["reported"] <= permission.activation_ticks["clearance"]);
    let runtime = live.world().resource::<WorldContentRuntime>();
    assert!(runtime.flags.flag("clearance_granted"));
    assert_eq!(runtime.flags.counter("clearance_applied"), 1);
    assert_eq!(runtime.flags.counter("hazard_stabilised"), 1);
    // Full-candidate rejection must happen before any effect or physical write.
    let before = capture(resumed.world());
    let digest = world_digest(resumed.world());
    for fault in 0..3 {
        let mut bad = state.clone();
        let permission = bad
            .scenario
            .as_mut()
            .unwrap()
            .situations
            .instances
            .get_mut("htn/hazard-clearance/root/permission")
            .unwrap();
        match fault {
            0 => {
                permission.activation_ticks.remove("reported");
            }
            1 => {
                permission
                    .activation_ticks
                    .insert("reported".into(), receipt_tick.saturating_sub(1));
            }
            _ => {
                permission
                    .activation_ticks
                    .insert("clearance".into(), bad.tick + 1);
            }
        }
        assert!(
            !restore(resumed.world_mut(), &bad).is_complete(),
            "forged callback clock {fault}"
        );
        assert!(capture(resumed.world()) == before);
        assert_eq!(world_digest(resumed.world()), digest);
    }
}

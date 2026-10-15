//! Same ordinary Logistics tree, admitted dispatch and Receive settlement.
use super::*;
#[test]
fn imported_delivery_instruction_uses_permitted_demand_and_real_accepted_batch_only() {
    let (mut app, actor) = fixture(ControlSource::Ai);
    send(
        &mut app,
        "gunner",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 0,
        },
    );
    app.update();
    let mut base = crate::world::activities::declared_actor_fixture(
        "logistics-fixture",
        "assets/entities/alliance_cruiser.toml",
    );
    let root = crate::world::config::parse_world(
        &crate::content_fs::read_to_string("tests/fixtures/maintenance-htn-world.toml").unwrap(),
    )
    .unwrap();
    let mut planning = root.planning.unwrap();
    planning.task = "maintenance::supply-rounds".into();
    planning.imports[0].names = vec!["supply-rounds".into()];
    planning.arguments.insert(
        "actor".into(),
        phoenix_sim_world::world::planning::Value::Actor("logistics-fixture".into()),
    );
    let (compiled, _) =
        phoenix_sim_world::world::planning::CompiledPlanning::load(planning.clone(), |path| {
            crate::content_fs::read_to_string(path).ok().map(|source| {
                source
                    .replace(
                        "exports=[\"maintain\"]",
                        "exports=[\"maintain\",\"supply-rounds\"]",
                    )
                    .replace("kind=\"quantity\",value=4", "kind=\"quantity\",value=2")
            })
        })
        .unwrap();
    base.planning = Some(planning);
    base.compiled_planning = Some(compiled);
    let profile = app
        .world()
        .get::<CompiledShipLogistics>(actor)
        .unwrap()
        .0
        .clone();
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    let released = app.world().resource::<crate::sim_tick::SimTick>().0;
    crate::world::planning::advance_for_actor(
        &mut runtime,
        &base,
        "logistics-fixture",
        None,
        released,
        |request| crate::world::activities::delivery_binding_supported(request, &profile),
    )
    .unwrap();
    app.insert_resource(base).insert_resource(runtime);
    let total = quantity(&app, actor);
    for _ in 0..3 {
        app.update();
        assert!(
            app.world()
                .resource::<crate::world::server::WorldContentRuntime>()
                .situations
                .instances
                .values()
                .next()
                .unwrap()
                .accepted
                .is_none(),
            "instruction does not fabricate local demand"
        );
    }
    send(
        &mut app,
        "gunner",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 6,
        },
    );
    let mut accepted = None;
    for _ in 0..30 {
        app.update();
        let runtime = app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>();
        let instance = runtime.situations.instances.values().next().unwrap();
        if let Some(job) = &instance.accepted {
            accepted = Some(job.clone());
            break;
        }
    }
    let accepted = accepted.expect("the existing ordinary tree admits actual demanded delivery");
    assert!(app
        .world()
        .get::<ShipLogistics>(actor)
        .unwrap()
        .0
        .batches
        .contains_key(&accepted.physical_job().unwrap()));
    assert!(app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .situations
        .instances
        .values()
        .next()
        .unwrap()
        .settled
        .is_none());
    assert_eq!(
        quantity(&app, actor).0 + quantity(&app, actor).1 + quantity(&app, actor).2,
        total.0 + total.1 + total.2
    );
    let mut received = false;
    for _ in 0..600 {
        app.update();
        let runtime = app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>();
        if runtime.pending_world_events.iter().any(|event|matches!(event,crate::world::content::WorldEvent::Activity(fact) if fact.scope==accepted && matches!(fact.outcome,phoenix_model::activity::ActivityOutcome::Delivery{returned:false,amount:2,..}))) {received=true;break;}
    }
    assert!(received, "only actual Receive produced the reserved fact");
    let final_stock = quantity(&app, actor);
    assert_eq!(
        final_stock.0 + final_stock.1 + final_stock.2,
        total.0 + total.1 + total.2
    );
}

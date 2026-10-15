//! Real admitted delivery/return outcomes, never feedback or level polling.
use super::*;
use phoenix_sim_world::world::activities::*;

fn prepare() -> (App, Entity) {
    let (mut app, actor) = fixture(ControlSource::Human);
    app.insert_resource(crate::world::activities::declared_actor_fixture(
        "logistics-fixture",
        "assets/entities/alliance_cruiser.toml",
    ));
    let definition = SituationDefinition {
        id: "supply".into(),
        initial: "received".into(),
        state: vec![SituationState {
            id: "received".into(),
            when: SituationCondition::Received,
            call: None,
            next: None,
        }],
    };
    let config = SituationConfig {
        id: "two-rounds".into(),
        definition: definition.id.clone(),
        request: ActivityRequest::Delivery {
            actor: "logistics-fixture".into(),
            owner: SystemId("logistics".into()),
            source: "hold-ammunition".into(),
            destination: "magazine".into(),
            commodity: "torpedoes".into(),
            amount: 2,
        },
    };
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    runtime.situation_definitions = vec![definition];
    runtime.situation_configs = vec![config];
    app.insert_resource(runtime);
    // Keep this test to one explicit ordinary request rather than the separate
    // maintained-stock policy. Actual handling/power/stock owners are unchanged.
    send(
        &mut app,
        "gunner",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 2,
        },
    );
    app.update();
    (app, actor)
}

#[test]
fn admitted_delivery_binds_its_committed_id_and_retains_only_actual_receive_once() {
    let (mut app, actor) = prepare();
    let before = quantity(&app, actor);
    // Normal delivery cannot exceed the real local demand. Raise it through
    // its actual operator in the SAME step as the explicit two-unit request.
    send(
        &mut app,
        "gunner",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 4,
        },
    );
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::DispatchDelivery {
            source: "hold-ammunition".into(),
            destination: "magazine".into(),
            amount: 2,
        },
    );
    app.update();
    let runtime = app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>();
    let scope = runtime.situations.instances["two-rounds"]
        .accepted
        .clone()
        .unwrap();
    assert!(
        runtime.pending_world_events.is_empty(),
        "accepted request is not delivery success"
    );
    assert!(app
        .world()
        .get::<ShipLogistics>(actor)
        .unwrap()
        .0
        .batches
        .contains_key(&scope.physical_job().unwrap()));
    for _ in 0..300 {
        app.update();
        if app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_world_events
            .iter()
            .any(|event| matches!(event, crate::world::content::WorldEvent::Activity(_)))
        {
            break;
        }
    }
    let runtime = app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>();
    let facts: Vec<_> = runtime
        .pending_world_events
        .iter()
        .filter_map(|event| {
            if let crate::world::content::WorldEvent::Activity(fact) = event {
                Some(fact)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].scope, scope);
    assert!(
        matches!(&facts[0].outcome,phoenix_model::activity::ActivityOutcome::Delivery {source,destination,amount:2,returned:false,outcome:phoenix_model::job::JobOutcome::Completed,..} if source=="hold-ammunition"&&destination=="magazine")
    );
    let pending = runtime.pending_world_events.clone();
    let after = quantity(&app, actor);
    assert_eq!(before.0 + before.1 + before.2, after.0 + after.1 + after.2);
    assert!(after.1 >= before.1 + 2);
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(
        app.world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_world_events,
        pending,
        "collector absence cannot evict or replay retained evidence"
    );
    let now = app.world().resource::<crate::sim_tick::SimTick>().0;
    let mut runtime = app
        .world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>();
    crate::world::activities::acknowledge_pending(&mut runtime, now);
    assert_eq!(
        runtime
            .situations
            .ready(&runtime.situation_definitions)
            .len(),
        1
    );
}

#[test]
fn rejected_request_has_no_binding_and_actual_return_does_not_credit_receive() {
    let (mut app, actor) = prepare();
    let before = quantity(&app, actor);
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::DispatchDelivery {
            source: "hold-ammunition".into(),
            destination: "magazine".into(),
            amount: 0,
        },
    );
    app.update();
    assert!(app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .situations
        .instances["two-rounds"]
        .accepted
        .is_none());
    assert_eq!(quantity(&app, actor), before);
    send(
        &mut app,
        "gunner",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 4,
        },
    );
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::DispatchDelivery {
            source: "hold-ammunition".into(),
            destination: "magazine".into(),
            amount: 2,
        },
    );
    app.update();
    let job = app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>()
        .situations
        .instances["two-rounds"]
        .accepted
        .as_ref()
        .unwrap()
        .physical_job()
        .unwrap();
    // Recall this physical batch and withdraw its standing local demand, so
    // ordinary Human machinery does not correctly schedule a replacement.
    send(
        &mut app,
        "gunner",
        "torpedo-magazine",
        Control::SetStockTarget {
            store: "magazine".into(),
            amount: 2,
        },
    );
    send(
        &mut app,
        "engineer",
        "logistics",
        Control::ReturnDelivery { job: job.0 },
    );
    for _ in 0..300 {
        app.update();
        if app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_world_events
            .iter()
            .any(|event| matches!(event, crate::world::content::WorldEvent::Activity(_)))
        {
            break;
        }
    }
    let now = app.world().resource::<crate::sim_tick::SimTick>().0;
    let mut runtime = app
        .world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>();
    crate::world::activities::acknowledge_pending(&mut runtime, now);
    let fact = runtime.situations.instances["two-rounds"]
        .settled
        .as_ref()
        .unwrap();
    assert!(matches!(
        fact.outcome,
        phoenix_model::activity::ActivityOutcome::Delivery {
            returned: true,
            outcome: phoenix_model::job::JobOutcome::Cancelled,
            ..
        }
    ));
    assert!(runtime
        .situations
        .ready(&runtime.situation_definitions)
        .is_empty());
    drop(runtime);
    let after = quantity(&app, actor);
    assert_eq!(before.0 + before.1 + before.2, after.0 + after.1 + after.2);
}

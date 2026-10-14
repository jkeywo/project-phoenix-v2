use super::*;
use crate::resources::Stock;
use phoenix_model::{interior::RouteLocation, job::JobId, messages::SystemId};

fn fixture() -> (
    CompiledLogistics,
    LogisticsState,
    BTreeMap<ResourceEndpoint, EndpointStock>,
) {
    let authored =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let mut ship = authored.ship_config.unwrap();
    let mut interior = authored.interior.unwrap();
    let magazine = ship
        .systems
        .iter()
        .find(|s| s.kind == crate::ship::system_registry::TORPEDO_MAGAZINE_KIND)
        .unwrap()
        .id
        .clone();
    let local = interior.placements[&magazine].clone();
    let home = interior
        .compartments
        .iter()
        .find(|r| r.id != local)
        .unwrap()
        .id
        .clone();
    let mut handling = ship.systems[0].clone();
    handling.id = SystemId("logistics".into());
    handling.kind = LOGISTICS_KIND.into();
    interior
        .placements
        .insert(handling.id.clone(), home.clone());
    ship.systems.push(handling);
    let graph = InteriorGraph::compile(&interior, &ship, interior.team_homes.len()).unwrap();
    let hold = ResourceEndpoint::Capacity {
        id: "hold-ammo".into(),
    };
    let mag = ResourceEndpoint::TorpedoMagazine {};
    let config = LogisticsConfig {
        stores: vec![
            ManagedStoreConfig {
                id: "hold".into(),
                endpoint: hold.clone(),
                resource: "torpedoes".into(),
                compartment: home.clone(),
                role: StoreRole::Main,
                system: None,
                source: None,
                target: 4,
                reserve: 0,
                priority: 0,
            },
            ManagedStoreConfig {
                id: "magazine".into(),
                endpoint: mag.clone(),
                resource: "torpedoes".into(),
                compartment: local.clone(),
                role: StoreRole::Local,
                system: Some(magazine),
                source: Some("hold".into()),
                target: 6,
                reserve: 0,
                priority: 0,
            },
        ],
        storage: vec![
            StorageBayConfig {
                compartment: home,
                space: 8,
                unit_space: BTreeMap::from([("torpedoes".into(), 1)]),
            },
            StorageBayConfig {
                compartment: local,
                space: 6,
                unit_space: BTreeMap::from([("torpedoes".into(), 1)]),
            },
        ],
        handling: vec![HandlingConfig {
            system: SystemId("logistics".into()),
            max_active: 2,
            max_batch: 3,
            units_per_sec: 2.0,
            reorder_fraction: 1.0,
        }],
    };
    let definitions = BTreeMap::from([
        (hold.clone(), EndpointDefinition { ceiling: 8 }),
        (mag.clone(), EndpointDefinition { ceiling: 6 }),
    ]);
    let profile = CompiledLogistics::compile(&config, &ship, &graph, &definitions).unwrap();
    let stocks = BTreeMap::from([
        (
            hold,
            EndpointStock {
                stock: Stock::new(4, 8).unwrap(),
                held: 0,
            },
        ),
        (
            mag,
            EndpointStock {
                stock: Stock::new(2, 6).unwrap(),
                held: 0,
            },
        ),
    ]);
    let state = LogisticsState::new(&profile);
    state.validate(&profile, &stocks).unwrap();
    (profile, state, stocks)
}
fn dispatch(
    profile: &CompiledLogistics,
    state: &mut LogisticsState,
    stocks: &mut BTreeMap<ResourceEndpoint, EndpointStock>,
    amount: i64,
) -> JobId {
    state
        .dispatch(profile, state.last_tick, "hold", "magazine", amount, |a| {
            apply_stock_action(profile, a, stocks)
        })
        .unwrap()
}
fn step(
    profile: &CompiledLogistics,
    state: &mut LogisticsState,
    stocks: &mut BTreeMap<ResourceEndpoint, EndpointStock>,
    dt: f32,
    blocked: &BTreeSet<String>,
    fraction: f32,
) -> f32 {
    let mut paid = 0.0;
    state
        .tick(
            profile,
            state.last_tick + 1,
            dt,
            blocked,
            |a| apply_stock_action(profile, a, stocks),
            |_, time| {
                paid += time * fraction;
                Ok(fraction)
            },
        )
        .unwrap();
    state.validate(profile, stocks).unwrap();
    paid
}
fn total(state: &LogisticsState, stocks: &BTreeMap<ResourceEndpoint, EndpointStock>) -> i64 {
    stocks.values().map(|s| s.stock.level + s.held).sum::<i64>()
        + state.batches.values().map(|b| b.amount).sum::<i64>()
}

#[test]
fn dispatch_conserves_real_cargo_and_reserves_space_against_other_loading() {
    let (profile, mut state, mut stocks) = fixture();
    let first = dispatch(&profile, &mut state, &mut stocks, 3);
    assert_eq!(total(&state, &stocks), 6);
    assert_eq!(stocks[&ResourceEndpoint::TorpedoMagazine {}].stock.level, 2);
    assert_eq!(state.batches[&first].amount, 3);
    dispatch(&profile, &mut state, &mut stocks, 3);
    assert_eq!(state.batches.values().map(|b| b.amount).sum::<i64>(), 4);
    let destination = &mut stocks
        .get_mut(&ResourceEndpoint::TorpedoMagazine {})
        .unwrap()
        .stock;
    assert_eq!(
        destination.adjust(100),
        0,
        "ordinary loading cannot take promised space"
    );
    assert_eq!(total(&state, &stocks), 6);
    state.validate(&profile, &stocks).unwrap();
    for _ in 0..100 {
        step(
            &profile,
            &mut state,
            &mut stocks,
            1.0,
            &BTreeSet::new(),
            1.0,
        );
    }
    assert!(state.batches.is_empty());
    assert_eq!(stocks[&ResourceEndpoint::TorpedoMagazine {}].stock.level, 6);
    assert_eq!(total(&state, &stocks), 6);
}
#[test]
fn protected_reserve_and_failed_dispatch_leave_owners_unchanged() {
    let (profile, mut state, mut stocks) = fixture();
    state.set_reserve(&profile, "hold", 4).unwrap();
    let before = (state.clone(), stocks.clone());
    assert!(state
        .dispatch(&profile, 0, "hold", "magazine", 2, |a| apply_stock_action(
            &profile,
            a,
            &mut stocks
        ))
        .is_err());
    assert_eq!((state.clone(), stocks.clone()), before);
    assert!(state.set_target(&profile, "magazine", 7).is_err());
    state.set_reserve(&profile, "hold", 2).unwrap();
    let job = dispatch(&profile, &mut state, &mut stocks, 3);
    assert_eq!(state.batches[&job].amount, 2);
    assert_eq!(
        stocks[&profile.store("hold").unwrap().endpoint].stock.level,
        2
    );
}

#[test]
fn future_dispatch_cannot_mutate_stock_into_an_invalid_saved_frontier() {
    let (profile, mut state, stocks) = fixture();
    let before = (state.clone(), stocks.clone());
    assert!(state
        .dispatch(&profile, 2, "hold", "magazine", 1, |_| panic!(
            "future dispatch must refuse before touching actual stock"
        ))
        .is_err());
    assert_eq!((state, stocks), before);
}
#[test]
fn routing_buys_only_actual_work_and_a_blocked_path_never_pays_for_waiting() {
    let (profile, mut state, mut stocks) = fixture();
    let job = dispatch(&profile, &mut state, &mut stocks, 1);
    let blocked = profile
        .graph()
        .config
        .connections
        .iter()
        .map(|c| c.id.clone())
        .collect();
    assert_eq!(
        step(&profile, &mut state, &mut stocks, 1000.0, &blocked, 1.0),
        0.5
    );
    assert_eq!(state.batches[&job].stage, DeliveryStage::Travelling);
    assert_eq!(
        step(&profile, &mut state, &mut stocks, 1000.0, &blocked, 1.0),
        0.0
    );
    assert!(state.batches[&job].navigation.blocked);
    let route = profile
        .graph()
        .route(
            &profile.store("hold").unwrap().compartment,
            &profile.store("magazine").unwrap().compartment,
            &BTreeSet::new(),
        )
        .unwrap();
    let duration: f32 = route
        .iter()
        .map(|id| {
            profile
                .graph()
                .config
                .connections
                .iter()
                .find(|c| &c.id == id)
                .unwrap()
                .travel_secs
        })
        .sum();
    let paid = step(
        &profile,
        &mut state,
        &mut stocks,
        1000.0,
        &BTreeSet::new(),
        1.0,
    );
    assert!((paid - duration).abs() < 0.0001, "{paid} != {duration}");
    assert_eq!(state.batches[&job].stage, DeliveryStage::Unloading);
    assert_eq!(
        step(
            &profile,
            &mut state,
            &mut stocks,
            1000.0,
            &BTreeSet::new(),
            1.0
        ),
        0.5
    );
    assert!(state.batches.is_empty());
    assert_eq!(total(&state, &stocks), 6);
}
#[test]
fn cancellation_retains_cargo_and_original_claim_until_a_real_return_can_fit() {
    let (profile, mut state, mut stocks) = fixture();
    let job = dispatch(&profile, &mut state, &mut stocks, 3);
    let source = profile.store("hold").unwrap().endpoint.clone();
    stocks.get_mut(&source).unwrap().stock.adjust(7);
    state.request_return(job).unwrap();
    let before = stocks.clone();
    assert_eq!(
        step(
            &profile,
            &mut state,
            &mut stocks,
            2.0,
            &BTreeSet::new(),
            1.0
        ),
        0.0
    );
    assert_eq!(stocks, before);
    assert!(!state.batches[&job].returning);
    stocks.get_mut(&source).unwrap().stock.adjust(-3);
    for _ in 0..100 {
        step(
            &profile,
            &mut state,
            &mut stocks,
            1.0,
            &BTreeSet::new(),
            1.0,
        );
    }
    assert!(state.batches.is_empty());
    assert_eq!(stocks[&source].stock.level, 8);
    assert_eq!(stocks[&ResourceEndpoint::TorpedoMagazine {}].stock.level, 2);
    assert_eq!(
        state.last_outcome.as_ref().unwrap().outcome,
        phoenix_model::job::JobOutcome::Cancelled
    );
}
#[test]
fn committed_crossing_and_json_state_continue_with_exact_claims() {
    let (profile, mut state, mut stocks) = fixture();
    let job = dispatch(&profile, &mut state, &mut stocks, 1);
    step(
        &profile,
        &mut state,
        &mut stocks,
        1.0,
        &BTreeSet::new(),
        1.0,
    );
    step(
        &profile,
        &mut state,
        &mut stocks,
        0.01,
        &BTreeSet::new(),
        1.0,
    );
    assert!(
        matches!(state.batches[&job].navigation.location,RouteLocation::Crossing{elapsed,..} if elapsed>0.0)
    );
    let mut restored: LogisticsState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    let mut restored_stocks = stocks.clone();
    for value in restored_stocks.values_mut() {
        value.stock = serde_json::from_slice(&serde_json::to_vec(&value.stock).unwrap()).unwrap();
    }
    let blocked = profile
        .graph()
        .config
        .connections
        .iter()
        .map(|c| c.id.clone())
        .collect();
    for _ in 0..100 {
        step(&profile, &mut state, &mut stocks, 0.1, &blocked, 1.0);
        step(
            &profile,
            &mut restored,
            &mut restored_stocks,
            0.1,
            &blocked,
            1.0,
        );
        assert_eq!(state, restored);
        assert_eq!(stocks, restored_stocks);
    }
    for _ in 0..100 {
        step(
            &profile,
            &mut state,
            &mut stocks,
            1.0,
            &BTreeSet::new(),
            0.5,
        );
        step(
            &profile,
            &mut restored,
            &mut restored_stocks,
            1.0,
            &BTreeSet::new(),
            0.5,
        );
        assert_eq!(state, restored);
        assert_eq!(stocks, restored_stocks);
    }
    assert!(state.batches.is_empty());
}
#[test]
fn malformed_claims_locations_and_nonfinite_work_refuse_without_state_commit() {
    let (profile, mut state, mut stocks) = fixture();
    let job = dispatch(&profile, &mut state, &mut stocks, 1);
    let mut invalid = state.clone();
    invalid.batches.get_mut(&job).unwrap().amount = 2;
    assert!(invalid.validate(&profile, &stocks).is_err());
    let mut invalid = state.clone();
    invalid.batches.get_mut(&job).unwrap().navigation.location = RouteLocation::Compartment {
        compartment: "missing-room".into(),
    };
    assert!(invalid.validate(&profile, &stocks).is_err());
    let before = state.clone();
    assert!(state
        .tick(
            &profile,
            1,
            1.0,
            &BTreeSet::new(),
            |a| apply_stock_action(&profile, a, &mut stocks),
            |_, _| Ok(f32::NAN)
        )
        .is_err());
    assert_eq!(state, before);
    assert!(state
        .tick(
            &profile,
            1,
            f32::NAN,
            &BTreeSet::new(),
            |_| unreachable!(),
            |_, _| unreachable!()
        )
        .is_err());
    assert_eq!(state, before);
}
#[test]
fn disabled_and_duplicate_ticks_preserve_committed_work_and_quantities() {
    let (profile, mut state, mut stocks) = fixture();
    dispatch(&profile, &mut state, &mut stocks, 1);
    state.set_enabled(false);
    assert_eq!(
        step(
            &profile,
            &mut state,
            &mut stocks,
            3.0,
            &BTreeSet::new(),
            1.0
        ),
        0.0
    );
    let before = state.clone();
    state
        .tick(
            &profile,
            state.last_tick,
            3.0,
            &BTreeSet::new(),
            |_| unreachable!(),
            |_, _| unreachable!(),
        )
        .unwrap();
    assert_eq!(state, before);
    state.set_enabled(true);
    for _ in 0..100 {
        step(
            &profile,
            &mut state,
            &mut stocks,
            1.0,
            &BTreeSet::new(),
            1.0,
        );
    }
    assert!(state.batches.is_empty());
    assert_eq!(total(&state, &stocks), 6);
}

#[test]
fn concurrent_batches_share_one_work_budget_and_blocked_work_frees_it() {
    let (profile, mut state, mut stocks) = fixture();
    let first = dispatch(&profile, &mut state, &mut stocks, 1);
    let second = dispatch(&profile, &mut state, &mut stocks, 1);
    assert_eq!(
        step(
            &profile,
            &mut state,
            &mut stocks,
            0.5,
            &BTreeSet::new(),
            1.0
        ),
        0.5
    );
    assert_eq!(state.batches[&first].stage, DeliveryStage::Travelling);
    assert_eq!(
        state.batches[&second].work_progress, 0.0,
        "second reservation does not double handling throughput"
    );
    let blocked = profile
        .graph()
        .config
        .connections
        .iter()
        .map(|c| c.id.clone())
        .collect();
    assert_eq!(
        step(&profile, &mut state, &mut stocks, 0.5, &blocked, 1.0),
        0.5
    );
    assert_eq!(state.blocked[&first], DeliveryBlock::Access);
    assert_eq!(
        state.batches[&second].stage,
        DeliveryStage::Travelling,
        "blocked first batch consumes no work budget"
    );
    let restored: LogisticsState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(restored.blocked, state.blocked);
    restored.validate(&profile, &stocks).unwrap();
    let mut invalid = restored;
    invalid.blocked.insert(JobId(999), DeliveryBlock::Disabled);
    assert!(invalid.validate(&profile, &stocks).is_err());
}
#[test]
fn shared_bay_accounting_counts_other_resources_and_incoming_without_mirrors() {
    let (mut profile, state, mut stocks) = fixture();
    let local = profile.store("magazine").unwrap().compartment.clone();
    let endpoint = ResourceEndpoint::Capacity {
        id: "local-parts".into(),
    };
    let bay = profile
        .config
        .storage
        .iter_mut()
        .find(|b| b.compartment == local)
        .unwrap();
    bay.unit_space.insert("parts".into(), 2);
    profile.config.stores.push(ManagedStoreConfig {
        id: "parts".into(),
        endpoint: endpoint.clone(),
        resource: "parts".into(),
        compartment: local,
        role: StoreRole::Main,
        system: None,
        source: None,
        target: 0,
        reserve: 0,
        priority: 0,
    });
    profile
        .endpoints
        .insert(endpoint.clone(), EndpointDefinition { ceiling: 2 });
    stocks.insert(
        endpoint,
        EndpointStock {
            stock: Stock::new(2, 2).unwrap(),
            held: 0,
        },
    );
    profile.validate_stocks(&stocks).unwrap();
    assert_eq!(profile.receiving_headroom(&stocks, "magazine").unwrap(), 0);
    let before = stocks.clone();
    let action = LogisticsStockAction::Dispatch {
        source: "hold".into(),
        destination: "magazine".into(),
        requested: 1,
        floor: 0,
        claim: state.claim(JobId(1)),
    };
    assert_eq!(
        apply_stock_action(&profile, &action, &mut stocks).unwrap(),
        0
    );
    assert_eq!(stocks, before);
}

#[test]
fn ordinary_logistics_tree_uses_only_closed_console_bindings() {
    let bindings = crate::resources::logistics_ai::bindings();
    crate::ai::state_tree::compile(&crate::resources::logistics_ai::initial_tree(), &bindings)
        .unwrap();
    assert_eq!(bindings.facts.len(), 2);
    assert_eq!(bindings.channels, BTreeSet::from(["dispatch".into()]));
}

#[test]
fn authoring_refuses_duplicate_locations_wrong_local_owner_and_nonfinite_limits() {
    let (profile, _, _) = fixture();
    let mut ship =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml")
            .unwrap()
            .ship_config
            .unwrap();
    let mut handling = ship.systems[0].clone();
    handling.id = profile.handling().system.clone();
    handling.kind = LOGISTICS_KIND.into();
    ship.systems.push(handling);
    for mode in 0..8 {
        let mut config = profile.config.clone();
        match mode {
            0 => config.stores[1].endpoint = config.stores[0].endpoint.clone(),
            1 => config.stores[1].system = Some(profile.handling().system.clone()),
            2 => config.handling[0].units_per_sec = f32::NAN,
            3 => config.stores[1].target = 7,
            4 => config.stores[1].source = Some("missing".into()),
            5 => config.handling[0].reorder_fraction = f32::NAN,
            6 => config.handling[0].reorder_fraction = -0.1,
            _ => config.handling[0].reorder_fraction = 1.1,
        }
        assert!(
            CompiledLogistics::compile(&config, &ship, profile.graph(), profile.endpoints())
                .is_err()
        );
    }
}

#[test]
fn priority_edits_and_json_continuation_preserve_committed_jobs_and_require_exact_store_keys() {
    let (mut profile, _, mut stocks) = fixture();
    profile.config.stores[0].priority = 7;
    let mut state = LogisticsState::new(&profile);
    assert_eq!(state.priorities["hold"], 7);
    dispatch(&profile, &mut state, &mut stocks, 1);
    let jobs = state.batches.clone();
    let stock_before = stocks.clone();
    state.set_priority(&profile, "hold", 0).unwrap();
    state.set_priority(&profile, "magazine", u16::MAX).unwrap();
    assert_eq!(state.batches, jobs);
    assert_eq!(stocks, stock_before);
    let unchanged = state.clone();
    assert!(state.set_priority(&profile, "absent", 1).is_err());
    assert_eq!(state, unchanged);
    let restored: LogisticsState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(state, restored);
    restored.validate(&profile, &stocks).unwrap();
    let mut bad = restored.clone();
    bad.priorities.remove("hold");
    assert!(bad.validate(&profile, &stocks).is_err());
    let mut bad = restored;
    bad.priorities.insert("foreign".into(), 0);
    assert!(bad.validate(&profile, &stocks).is_err());
}

#[test]
fn old_authored_policy_defaults_are_explicit_and_new_values_stay_closed() {
    let (profile, _, _) = fixture();
    let mut authored = serde_json::to_value(profile.config()).unwrap();
    for store in authored["store"].as_array_mut().unwrap() {
        store.as_object_mut().unwrap().remove("priority");
    }
    authored["handling"][0]
        .as_object_mut()
        .unwrap()
        .remove("reorder_fraction");
    let old: LogisticsConfig = serde_json::from_value(authored.clone()).unwrap();
    assert!(old.stores.iter().all(|store| store.priority == 0));
    assert_eq!(old.handling[0].reorder_fraction, 1.0);
    authored["store"][0]["priority"] = serde_json::json!(65536);
    assert!(serde_json::from_value::<LogisticsConfig>(authored.clone()).is_err());
    authored["store"][0]["priority"] = serde_json::json!(2);
    authored["handling"][0]["unrestricted_policy"] = serde_json::json!(true);
    assert!(serde_json::from_value::<LogisticsConfig>(authored).is_err());
}

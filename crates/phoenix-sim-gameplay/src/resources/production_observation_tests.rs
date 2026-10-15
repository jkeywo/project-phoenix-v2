use super::*;

fn reports(f: &Fixture) -> Vec<ProductionStockObservation> {
    let logistics = f.profile.logistics();
    logistics
        .config()
        .stores
        .iter()
        .map(|store| {
            let stock = &f.stocks[&store.endpoint];
            let bay = logistics
                .config()
                .storage
                .iter()
                .find(|b| b.compartment == store.compartment)
                .unwrap();
            ProductionStockObservation {
                store: store.id.clone(),
                level: stock.stock.level,
                available: stock.stock.available(),
                incoming: stock.stock.incoming_total(),
                held: stock.held,
                reserve: f.floors.get(&store.id).copied().unwrap_or(0),
                headroom: logistics.receiving_headroom(&f.stocks, &store.id).unwrap(),
                ceiling: stock.stock.ceiling,
                bay_free_space: bay.space
                    - logistics
                        .occupied_space(&f.stocks, &store.compartment)
                        .unwrap(),
            }
        })
        .collect()
}

#[test]
fn only_selected_recipe_reports_are_required_and_policy_has_no_stock_side_effects() {
    let f = fixture();
    let state = ProductionState::new(&f.profile);
    let before = f.stocks.clone();
    let mut observed = reports(&f);
    observed.retain(|r| r.store != "magazine");
    assert!(state
        .start_available_from_report(&f.profile, 0, &observed)
        .unwrap());
    assert!(state
        .start_available(&f.profile, 0, &f.floors, &f.stocks)
        .unwrap());
    // Unrelated reports neither grant knowledge nor become required validation.
    observed.push(ProductionStockObservation {
        store: "not-a-recipe-store".into(),
        level: -1,
        ..observed[0].clone()
    });
    assert!(state
        .start_available_from_report(&f.profile, 0, &observed)
        .unwrap());
    observed.retain(|r| r.store != "parts");
    assert!(!state
        .start_available_from_report(&f.profile, 0, &observed)
        .unwrap());
    assert_eq!(f.stocks, before);
    assert_eq!(state, ProductionState::new(&f.profile));
}

#[test]
fn floors_available_incoming_and_held_match_truth_side_start_decisions() {
    for mode in 0..5 {
        let mut f = fixture();
        let mut state = ProductionState::new(&f.profile);
        match mode {
            0 => {
                f.floors.insert("parts".into(), 6);
            }
            1 => {
                f.stocks
                    .get_mut(&endpoint("alloy"))
                    .unwrap()
                    .stock
                    .reserve("other-work", 5)
                    .unwrap();
            }
            2 => {
                state.set_target(&f.profile, 1).unwrap();
                f.stocks
                    .get_mut(&endpoint("hold"))
                    .unwrap()
                    .stock
                    .reserve_incoming("other-work", 1)
                    .unwrap();
            }
            3 => {
                state.set_target(&f.profile, 1).unwrap();
                f.stocks.get_mut(&endpoint("hold")).unwrap().held = 1;
            }
            _ => {}
        }
        let decision = state
            .start_available_from_report(&f.profile, 0, &reports(&f))
            .unwrap();
        assert_eq!(decision, mode == 4);
        assert_eq!(
            decision,
            state
                .start_available(&f.profile, 0, &f.floors, &f.stocks)
                .unwrap()
        );
        let mut actual = state.clone();
        assert_eq!(decision, start(&mut f, &mut actual, 1.0, 1.0));
    }
}

#[test]
fn multiple_outputs_use_exact_shared_space_freed_by_consumed_inputs() {
    for free in [1, 2] {
        let mut f = fixture();
        let mut logistics = f.profile.logistics().config().clone();
        let bay = logistics
            .storage
            .iter_mut()
            .find(|b| b.compartment == logistics.stores[0].compartment)
            .unwrap();
        bay.unit_space = BTreeMap::from([
            ("alloy".into(), 3),
            ("machinery_parts".into(), 2),
            ("torpedoes".into(), 4),
        ]);
        bay.space = 30 + free;
        let compiled = CompiledLogistics::compile(
            &logistics,
            &f.ship,
            f.profile.logistics().graph(),
            f.profile.logistics().endpoints(),
        )
        .unwrap();
        let mut config = f.profile.config().clone();
        config.recipes[0].outputs[0].amount = 2;
        config.recipes[0].outputs.push(ProductionAmount {
            store: "parts".into(),
            amount: 1,
        });
        f.profile = CompiledProduction::compile(&config, &compiled, &f.ship).unwrap();
        let state = ProductionState::new(&f.profile);
        let observed = reports(&f);
        assert_eq!(
            observed
                .iter()
                .find(|r| r.store == "hold")
                .unwrap()
                .headroom,
            0
        );
        let decision = state
            .start_available_from_report(&f.profile, 0, &observed)
            .unwrap();
        // The batch frees eight space units and needs ten. Integer headroom
        // alone would lose the difference between one and two free units.
        assert_eq!(decision, free == 2);
        assert_eq!(
            decision,
            state
                .start_available(&f.profile, 0, &f.floors, &f.stocks)
                .unwrap()
        );
        let before = f.stocks.clone();
        let mut actual = state.clone();
        assert_eq!(decision, start(&mut f, &mut actual, 1.0, 1.0));
        if !decision {
            assert_eq!(f.stocks, before);
        }
    }
}

#[test]
fn malformed_relevant_reports_refuse_without_touching_stock_or_state() {
    let f = fixture();
    let state = ProductionState::new(&f.profile);
    let good = reports(&f);
    for mode in 0..7 {
        let mut observed = good.clone();
        match mode {
            0 => observed.push(observed[0].clone()),
            1 => observed[0].ceiling += 1,
            2 => observed[0].available = observed[0].level + 1,
            3 => observed[0].bay_free_space -= 1,
            4 => observed[0].headroom += 1,
            5 => {
                observed[0].level = i64::MAX;
                observed[0].incoming = 1;
            }
            _ => {
                for row in observed.iter_mut().filter(|r| r.store != "magazine") {
                    row.bay_free_space = 24;
                    row.headroom = row.ceiling - row.level - row.incoming - row.held;
                }
            }
        }
        assert!(
            state
                .start_available_from_report(&f.profile, 0, &observed)
                .is_err(),
            "mode {mode}"
        );
    }
    assert_eq!(state, ProductionState::new(&f.profile));
}

#[test]
fn observation_readiness_obeys_owner_frontier_and_standing_machine_intent() {
    let mut f = fixture();
    let mut state = ProductionState::new(&f.profile);
    let observed = reports(&f);
    assert!(state
        .start_available_from_report(&f.profile, 2, &observed)
        .is_err());
    state.owner = SystemId("other".into());
    assert!(state
        .start_available_from_report(&f.profile, 0, &observed)
        .is_err());
    state.owner = f.profile.config().system.clone();
    state.set_enabled(false);
    assert!(!state
        .start_available_from_report(&f.profile, 0, &observed)
        .unwrap());
    state.set_enabled(true);
    state.set_target(&f.profile, 0).unwrap();
    assert!(!state
        .start_available_from_report(&f.profile, 0, &observed)
        .unwrap());
    state.set_target(&f.profile, 4).unwrap();
    assert!(start(&mut f, &mut state, 1.0, 1.0));
    assert!(!state
        .start_available_from_report(&f.profile, 0, &reports(&f))
        .unwrap());
}

#[test]
fn compilation_refuses_an_extra_fitted_fabrication_owner() {
    let f = fixture();
    let mut ship = f.ship.clone();
    let mut extra = ship.system(&f.profile.config().system).unwrap().clone();
    extra.id = SystemId("unconfigured-fabricator".into());
    ship.systems.push(extra);
    assert!(CompiledProduction::compile(f.profile.config(), f.profile.logistics(), &ship).is_err());
}

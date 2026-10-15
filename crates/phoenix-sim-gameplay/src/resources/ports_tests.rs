use super::*;

fn profile() -> PortServiceConfig {
    PortServiceConfig {
        policy: PortServicePolicy::ManualOffer,
        eligible_factions: vec![uuid::Uuid::from_u128(1)],
        offers: vec![PortOfferConfig {
            id: "parts".into(),
            resource: "machinery_parts".into(),
            quota: 4,
            unit_amount: 5,
            costs: vec![PortCost {
                resource: "alloy".into(),
                amount: 2,
            }],
        }],
    }
}
fn visitor(id: u128) -> String {
    uuid::Uuid::from_u128(id).to_string()
}
fn order() -> PortOrderState {
    let mut state = PortOrderState::default();
    state
        .approve(
            &profile(),
            &visitor(2),
            1,
            vec![PortOrderLine {
                offer: "parts".into(),
                store: "hold-parts".into(),
                lots: 3,
            }],
        )
        .unwrap();
    state
}
fn advance(state: &mut PortOrderState, tick: u64, dt: f32, supply: i64, grant: f32) -> i64 {
    state
        .advance(
            &profile(),
            &SystemId("umbilical".into()),
            tick,
            dt,
            5.0,
            supply,
            |system, seconds| {
                assert_eq!(system, &SystemId("umbilical".into()));
                assert!(seconds > 0.0 && seconds <= dt);
                Ok(grant)
            },
        )
        .unwrap()
}

#[test]
fn provider_visits_bound_each_actual_visitor_and_revoke_on_undock() {
    let config = profile();
    let mut state = PortVisitState::default();
    let visitors = BTreeSet::from([visitor(2), visitor(3)]);
    state.refresh(&config, 1, &visitors).unwrap();
    state
        .record_delivery(&config, &visitor(2), 1, "parts", 3)
        .unwrap();
    assert_eq!(
        state.remaining(&config, &visitor(2), 1, "parts").unwrap(),
        1
    );
    assert_eq!(
        state.remaining(&config, &visitor(3), 1, "parts").unwrap(),
        4
    );
    let before = state.clone();
    assert!(state
        .record_delivery(&config, &visitor(2), 1, "parts", 2)
        .is_err());
    assert_eq!(state, before);
    state
        .refresh(&config, 2, &BTreeSet::from([visitor(3)]))
        .unwrap();
    assert!(state.remaining(&config, &visitor(2), 1, "parts").is_err());
    state.refresh(&config, 3, &visitors).unwrap();
    assert_eq!(state.visitors[&visitor(2)].visit, 3);
    assert_eq!(
        state.remaining(&config, &visitor(2), 3, "parts").unwrap(),
        4
    );
    assert!(state
        .record_delivery(&config, &visitor(2), 1, "parts", 1)
        .is_err());
    let encoded = serde_json::to_vec(&state).unwrap();
    assert_eq!(
        serde_json::from_slice::<PortVisitState>(&encoded).unwrap(),
        state
    );
    state.validate(&config, 3).unwrap();
}

#[test]
fn provider_invalid_refresh_or_quota_never_partially_commits() {
    let config = profile();
    let mut state = PortVisitState::default();
    state
        .refresh(&config, 2, &BTreeSet::from([visitor(2)]))
        .unwrap();
    for (tick, visitors) in [
        (1, BTreeSet::new()),
        (3, BTreeSet::from([visitor(3), "invalid".into()])),
        (2, BTreeSet::new()),
    ] {
        let before = state.clone();
        assert!(state.refresh(&config, tick, &visitors).is_err());
        assert_eq!(state, before);
    }
    for mode in 0..4 {
        let mut bad = state.clone();
        match mode {
            0 => {
                bad.visitors.get_mut(&visitor(2)).unwrap().delivered.clear();
            }
            1 => {
                bad.visitors
                    .get_mut(&visitor(2))
                    .unwrap()
                    .delivered
                    .insert("parts".into(), 5);
            }
            2 => {
                bad.visitors.get_mut(&visitor(2)).unwrap().visit = 3;
            }
            _ => bad.last_tick = 3,
        }
        assert!(bad.validate(&config, 2).is_err());
    }
}

#[test]
fn first_physical_visit_can_begin_at_tick_zero_without_reissuing_its_quota() {
    let config = profile();
    let visitors = BTreeSet::from([visitor(2)]);
    let mut state = PortVisitState::default();
    state.refresh(&config, 0, &visitors).unwrap();
    assert!(state.initialized);
    let visit = state.visitors[&visitor(2)].visit;
    state
        .record_delivery(&config, &visitor(2), visit, "parts", 1)
        .unwrap();
    let before = state.clone();
    state.refresh(&config, 0, &visitors).unwrap();
    assert_eq!(state, before);
    assert!(state.refresh(&config, 0, &BTreeSet::new()).is_err());
    assert_eq!(state, before);
    state.refresh(&config, 1, &visitors).unwrap();
    assert_eq!(state.visitors[&visitor(2)].visit, visit);
    assert_eq!(state.visitors[&visitor(2)].delivered["parts"], 1);
    state.refresh(&config, 2, &BTreeSet::new()).unwrap();
    state.refresh(&config, 3, &visitors).unwrap();
    assert_ne!(state.visitors[&visitor(2)].visit, visit);
}

#[test]
fn lot_work_uses_actual_units_paid_fraction_and_exact_immutable_receipts() {
    let mut state = order();
    assert_eq!(advance(&mut state, 1, 1.0, 3, 0.5), 0);
    assert_eq!(state.fraction, 0.5);
    assert_eq!(state.lines[0].delivered_lots, 0);
    assert_eq!(advance(&mut state, 2, 1.0, 3, 0.5), 1);
    assert!(
        state.validate(&profile(), 2).is_err(),
        "unsettled work cannot cross the save barrier"
    );
    assert!(state.record_receipt(2, 2).is_err());
    state.record_receipt(2, 1).unwrap();
    state.record_receipt(2, 1).unwrap();
    assert_eq!(
        state.lines[0].delivered_lots, 1,
        "duplicate receipt cannot duplicate goods or progress"
    );
    assert!(state.record_receipt(2, 0).is_err());
    state.validate(&profile(), 2).unwrap();
    assert_eq!(advance(&mut state, 3, 100.0, 20, 1.0), 2);
    state.record_receipt(3, 2).unwrap();
    assert_eq!(state.lines[0].delivered_lots, 3);
    assert!(!state.active);
    assert_eq!(state.fraction, 0.0);
}

#[test]
fn unavailable_supply_pausing_and_json_preserve_already_paid_work() {
    let mut state = order();
    assert_eq!(advance(&mut state, 1, 1.0, 3, 0.25), 0);
    let encoded = serde_json::to_vec(&state).unwrap();
    let mut resumed: PortOrderState = serde_json::from_slice(&encoded).unwrap();
    for tick in 2..5 {
        for candidate in [&mut state, &mut resumed] {
            assert_eq!(
                candidate
                    .advance(
                        &profile(),
                        &SystemId("umbilical".into()),
                        tick,
                        1.0,
                        5.0,
                        0,
                        |_, _| panic!("no actual supply cannot buy machinery work")
                    )
                    .unwrap(),
                0
            );
            assert_eq!(candidate.fraction, 0.25);
        }
        assert_eq!(state, resumed);
    }
    assert_eq!(advance(&mut state, 5, 1.0, 3, 0.75), 1);
    state.record_receipt(5, 0).unwrap();
    assert_eq!(
        state.lines[0].delivered_lots, 0,
        "actual refusal never invents delivered goods"
    );
    state.stop();
    assert_eq!(state.fraction, 0.0);
    assert_eq!(
        state
            .advance(
                &profile(),
                &SystemId("umbilical".into()),
                6,
                1.0,
                5.0,
                3,
                |_, _| panic!("cancelled order cannot buy work")
            )
            .unwrap(),
        0
    );
}

#[test]
fn replay_errors_and_replacement_keep_a_monotonic_settled_frontier() {
    let mut state = order();
    assert_eq!(advance(&mut state, 2, 1.0, 3, 1.0), 1);
    let pending = state.clone();
    assert!(state
        .approve(
            &profile(),
            &visitor(3),
            2,
            vec![PortOrderLine {
                offer: "parts".into(),
                store: "other".into(),
                lots: 1
            }]
        )
        .is_err());
    assert_eq!(state, pending);
    state.stop();
    state.record_receipt(2, 1).unwrap();
    state.validate(&profile(), 2).unwrap();
    state
        .approve(
            &profile(),
            &visitor(3),
            2,
            vec![PortOrderLine {
                offer: "parts".into(),
                store: "other".into(),
                lots: 1,
            }],
        )
        .unwrap();
    assert_eq!(state.last_tick, 2);
    assert_eq!(state.last_outcome.as_ref().unwrap().offered_lots, 0);
    assert_eq!(
        state
            .advance(
                &profile(),
                &SystemId("umbilical".into()),
                2,
                1.0,
                5.0,
                3,
                |_, _| panic!("duplicate tick cannot buy work")
            )
            .unwrap(),
        0
    );
    for grant in [f32::NAN, -0.1, 1.1] {
        let before = state.clone();
        assert!(state
            .advance(
                &profile(),
                &SystemId("umbilical".into()),
                3,
                1.0,
                5.0,
                3,
                |_, _| Ok(grant)
            )
            .is_err());
        assert_eq!(state, before);
    }
    for mode in 0..5 {
        let mut bad = state.clone();
        match mode {
            0 => bad.fraction = f64::NAN,
            1 => bad.partner = Some("invalid".into()),
            2 => bad.last_outcome = None,
            3 => bad.last_outcome.as_mut().unwrap().tick = 1,
            _ => bad.lines[0].delivered_lots = 2,
        }
        assert!(bad.validate(&profile(), 3).is_err());
    }
}

#[test]
fn malformed_approval_is_atomic_and_positive_costs_do_not_create_authority() {
    let mut state = order();
    for mode in 0..6 {
        let mut lines = vec![PortOrderLine {
            offer: "parts".into(),
            store: "hold".into(),
            lots: 1,
        }];
        match mode {
            0 => lines.clear(),
            1 => lines[0].offer = "absent".into(),
            2 => lines[0].lots = 0,
            3 => lines[0].lots = 5,
            4 => lines[0].store.clear(),
            _ => lines.push(lines[0].clone()),
        }
        let before = state.clone();
        assert!(state.approve(&profile(), &visitor(2), 1, lines).is_err());
        assert_eq!(state, before);
    }
    // This state has no human token, inventory, cost debit or permission flag.
    // Only the actual admitted adapter can turn approved work into goods.
    assert_eq!(state.lines[0].delivered_lots, 0);
    let json = serde_json::to_value(&state).unwrap();
    assert!(json.get("stock").is_none());
    assert!(json.get("authority").is_none());
    let mut invented = order();
    invented.lines[0].delivered_lots = 1;
    assert!(
        invented.validate(&profile(), 3).is_err(),
        "no execution/receipt frontier can never have delivered goods"
    );
}

#[test]
fn future_visit_cannot_buy_work_before_its_actual_frontier() {
    let mut state = PortOrderState::default();
    state
        .approve(
            &profile(),
            &visitor(2),
            5,
            vec![PortOrderLine {
                offer: "parts".into(),
                store: "hold".into(),
                lots: 1,
            }],
        )
        .unwrap();
    let before = state.clone();
    assert!(state
        .advance(
            &profile(),
            &SystemId("umbilical".into()),
            4,
            1.0,
            5.0,
            1,
            |_, _| panic!("future physical visit cannot buy work")
        )
        .is_err());
    assert_eq!(state, before);
}

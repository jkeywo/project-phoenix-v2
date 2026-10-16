//! Pure demand/transaction probes; full paid-owner continuation is a host test.
use super::*;
use crate::resources::logistics_server::{dispatch_order, endpoint_view};

fn fixture() -> (
    CompiledShipLogistics,
    ShipLogistics,
    crate::infrastructure::InfrastructureCondition,
    crate::console::weapons::TorpedoSystemResource,
    PlannedDeliveryRequest,
) {
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = crate::resources::logistics_profile::compile_entity_logistics(&content)
        .unwrap()
        .unwrap();
    let mut state = LogisticsState::new(&profile);
    state.set_target(&profile, "repair-structural", 0).unwrap();
    let mut inventory = crate::infrastructure::InfrastructureCondition(
        crate::infrastructure::InfrastructureState::from_config(
            content.infrastructure.as_ref().unwrap(),
        ),
    );
    // This is a disposable pure transaction input, not the actual host cause.
    inventory.0.adjust_capacity("structural_parts", -8).unwrap();
    let row = PlannedDeliveryRequest {
        id: "htn-optional/maintenance/1/supply".into(),
        released_tick: 1,
        source: "hold-structural-parts".into(),
        destination: "repair-structural".into(),
        commodity: "structural_parts".into(),
        amount: 1,
    };
    let torpedoes = content.torpedoes.as_ref().unwrap();
    let magazine = crate::console::weapons::TorpedoSystemResource(
        crate::weapons::torpedo::TorpedoSystem::from_configs(
            &torpedoes.tubes,
            torpedoes.to_runtime(),
        ),
    );
    (
        CompiledShipLogistics(profile),
        ShipLogistics(state),
        inventory,
        magazine,
        row,
    )
}
fn request(row: &PlannedDeliveryRequest) -> DeliveryRequest {
    DeliveryRequest {
        source: row.source.clone(),
        destination: row.destination.clone(),
        amount: row.amount,
    }
}
fn views(
    profile: &CompiledShipLogistics,
    state: &ShipLogistics,
    stocks: &BTreeMap<ResourceEndpoint, EndpointStock>,
) -> Vec<LogisticsStoreView> {
    profile
        .0
        .config()
        .stores
        .iter()
        .map(|store| {
            let value = &stocks[&store.endpoint];
            LogisticsStoreView {
                id: store.id.clone(),
                resource: store.resource.clone(),
                role: if store.role == phoenix_model::logistics::StoreRole::Main {
                    "main"
                } else {
                    "local"
                }
                .into(),
                system: store.system.clone(),
                level: value.stock.level,
                available: value.stock.available(),
                held: value.held,
                ceiling: value.stock.ceiling,
                headroom: profile.0.receiving_headroom(stocks, &store.id).unwrap(),
                bay_free_space: i64::MAX,
                incoming: value.stock.incoming_total(),
                target: state.0.goals[&store.id],
                reserve: state.0.floors[&store.id],
                priority: state.0.priorities[&store.id],
                compartment: store.compartment.clone(),
            }
        })
        .collect()
}
#[test]
fn current_frontier_and_permitted_original_scope_must_match_exactly() {
    let (_, _, _, _, row) = fixture();
    assert_eq!(
        intersect(vec![row.clone()], &[row.clone()]),
        vec![row.clone()]
    );
    for mutation in 0..5 {
        let mut other = row.clone();
        match mutation {
            0 => other.id.push_str("/other"),
            1 => other.released_tick += 1,
            2 => other.commodity = "machinery_parts".into(),
            3 => other.amount += 1,
            _ => other.source = "hold-machinery-parts".into(),
        }
        assert!(intersect(vec![row.clone()], &[other]).is_empty());
    }
    assert!(
        intersect(vec![row.clone()], &[]).is_empty(),
        "no permitted row grants no demand"
    );
    assert!(
        intersect(vec![], &[row]).is_empty(),
        "stale accepted/terminal row grants no demand"
    );
}
#[test]
fn exact_mission_dispatch_uses_real_stock_and_cannot_repeat_or_raise_stored_target() {
    let (profile, mut state, mut inventory, mut magazine, row) = fixture();
    let mut flags = Vec::new();
    let before = inventory
        .0
        .capacity_stock("hold-structural-parts")
        .unwrap()
        .level;
    assert!(
        dispatch_order(
            &profile.0,
            &mut state.0,
            1,
            &request(&row),
            &[],
            &BTreeSet::new(),
            Some(&mut inventory),
            Some(&mut magazine),
            &mut flags
        )
        .is_err(),
        "ordinary target-zero dispatch retains its existing refusal"
    );
    let job = dispatch_order(
        &profile.0,
        &mut state.0,
        1,
        &request(&row),
        &[row.clone()],
        &BTreeSet::new(),
        Some(&mut inventory),
        Some(&mut magazine),
        &mut flags,
    )
    .unwrap();
    assert_eq!(state.0.goals["repair-structural"], 0);
    assert_eq!(state.0.batches[&job].amount, 1);
    assert_eq!(
        inventory
            .0
            .capacity_stock("hold-structural-parts")
            .unwrap()
            .level
            + state.0.batches[&job].amount,
        before,
        "dispatch moves finite source quantity into actual transit"
    );
    let after = inventory.clone();
    let next_id = state.0.next_id;
    assert!(dispatch_order(
        &profile.0,
        &mut state.0,
        1,
        &request(&row),
        &[row],
        &BTreeSet::new(),
        Some(&mut inventory),
        Some(&mut magazine),
        &mut flags
    )
    .is_err());
    assert_eq!(inventory, after);
    assert_eq!(state.0.next_id, next_id);
    assert_eq!(
        state.0.batches.len(),
        1,
        "incoming/active batch cannot be topped up by a stale same scope"
    );
}
#[test]
fn missing_stock_feed_partial_supply_incoming_and_closed_route_cannot_enable_mission_demand() {
    let (profile, state, inventory, magazine, row) = fixture();
    let stocks = endpoint_view(&profile.0, Some(&inventory), Some(&magazine)).unwrap();
    let permitted = views(&profile, &state, &stocks);
    assert_eq!(
        candidate(&profile, &state, &permitted, &[row.clone()]),
        Some(request(&row))
    );
    let missing = permitted
        .iter()
        .filter(|store| store.id != row.source)
        .cloned()
        .collect::<Vec<_>>();
    assert!(candidate(&profile, &state, &missing, &[row.clone()]).is_none());
    let from = profile.0.store(&row.source).unwrap().endpoint.clone();
    let to = profile.0.store(&row.destination).unwrap().endpoint.clone();
    let mut unavailable = stocks.clone();
    let stock = &mut unavailable.get_mut(&from).unwrap().stock;
    stock.reserve("other-actual-work", stock.level).unwrap();
    assert!(dispatch_amount(
        &profile.0,
        &state.0,
        &unavailable,
        &BTreeSet::new(),
        &request(&row),
        &[row.clone()]
    )
    .is_none());
    let mut incoming = stocks.clone();
    incoming
        .get_mut(&to)
        .unwrap()
        .stock
        .reserve_incoming("other-delivery", 1)
        .unwrap();
    assert!(dispatch_amount(
        &profile.0,
        &state.0,
        &incoming,
        &BTreeSet::new(),
        &request(&row),
        &[row.clone()]
    )
    .is_none());
    assert!(candidate(
        &profile,
        &state,
        &views(&profile, &state, &incoming),
        &[row.clone()]
    )
    .is_none());
    let blocked = profile
        .0
        .graph()
        .config
        .connections
        .iter()
        .map(|link| link.id.clone())
        .collect::<BTreeSet<_>>();
    assert!(dispatch_amount(
        &profile.0,
        &state.0,
        &stocks,
        &blocked,
        &request(&row),
        &[row.clone()]
    )
    .is_none());
    let mut wrong_amount = request(&row);
    wrong_amount.amount += 1;
    assert!(dispatch_amount(
        &profile.0,
        &state.0,
        &stocks,
        &BTreeSet::new(),
        &wrong_amount,
        &[row.clone()]
    )
    .is_none());
    let mut wrong_resource = row.clone();
    wrong_resource.commodity = "machinery_parts".into();
    assert!(candidate(&profile, &state, &permitted, &[wrong_resource]).is_none());
    assert_eq!(state.0.goals["repair-structural"], 0);
}

#[test]
fn own_visible_row_without_current_world_authority_grants_no_mission_demand() {
    let content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let (_, state, _, _, row) = fixture();
    let config = crate::ship_plugin::ShipConfigComponent(content.ship_config.unwrap());
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        state.0.owner.clone(),
        SystemBlackboard::Logistics(phoenix_model::logistics_ui::LogisticsBlackboard {
            planned_requests: vec![row],
            ..Default::default()
        }),
    );
    for actor in ["logistics-fixture", "foreign-actor"] {
        assert!(
            permitted_current(None, None, actor, &state.0.owner, &config, &boards, 100).is_empty()
        );
    }
}

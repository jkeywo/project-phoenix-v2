use super::*;
use crate::{
    resources::{logistics::*, Stock},
    ship::interior::InteriorGraph,
};
use phoenix_model::messages::SystemId;

struct Fixture {
    profile: CompiledProduction,
    logistics: LogisticsState,
    stocks: EndpointMap,
    floors: BTreeMap<String, i64>,
    ship: ShipConfig,
}
fn endpoint(id: &str) -> ResourceEndpoint {
    ResourceEndpoint::Capacity { id: id.into() }
}
fn fixture() -> Fixture {
    let entity =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let mut ship = entity.ship_config.unwrap();
    let mut layout = entity.interior.unwrap();
    let magazine = ship
        .systems
        .iter()
        .find(|s| s.kind == crate::ship::system_registry::TORPEDO_MAGAZINE_KIND)
        .unwrap()
        .id
        .clone();
    let local = layout.placements[&magazine].clone();
    let home = layout
        .compartments
        .iter()
        .find(|r| r.id != local)
        .unwrap()
        .id
        .clone();
    for (id, kind) in [
        ("logistics", LOGISTICS_KIND),
        ("fabrication", FABRICATION_KIND),
    ] {
        let mut system = ship.systems[0].clone();
        system.id = SystemId(id.into());
        system.kind = kind.into();
        layout.placements.insert(system.id.clone(), home.clone());
        ship.systems.push(system);
    }
    let graph = InteriorGraph::compile(&layout, &ship, layout.team_homes.len()).unwrap();
    let mut stores = Vec::new();
    for (id, resource) in [
        ("alloy", "alloy"),
        ("parts", "machinery_parts"),
        ("hold", "torpedoes"),
    ] {
        stores.push(ManagedStoreConfig {
            id: id.into(),
            endpoint: endpoint(id),
            resource: resource.into(),
            compartment: home.clone(),
            role: StoreRole::Main,
            system: None,
            source: None,
            target: 0,
            reserve: 0,
            priority: 0,
        });
    }
    stores.push(ManagedStoreConfig {
        id: "magazine".into(),
        endpoint: ResourceEndpoint::TorpedoMagazine {},
        resource: "torpedoes".into(),
        compartment: local.clone(),
        role: StoreRole::Local,
        system: Some(magazine),
        source: Some("hold".into()),
        target: 6,
        reserve: 0,
        priority: 0,
    });
    let config = LogisticsConfig {
        stores,
        storage: vec![
            StorageBayConfig {
                compartment: home,
                space: 24,
                unit_space: BTreeMap::from([
                    ("alloy".into(), 1),
                    ("machinery_parts".into(), 1),
                    ("torpedoes".into(), 1),
                ]),
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
    let stocks = BTreeMap::from([
        (
            endpoint("alloy"),
            EndpointStock {
                stock: Stock::new(6, 8).unwrap(),
                held: 0,
            },
        ),
        (
            endpoint("parts"),
            EndpointStock {
                stock: Stock::new(6, 8).unwrap(),
                held: 0,
            },
        ),
        (
            endpoint("hold"),
            EndpointStock {
                stock: Stock::new(0, 8).unwrap(),
                held: 0,
            },
        ),
        (
            ResourceEndpoint::TorpedoMagazine {},
            EndpointStock {
                stock: Stock::new(2, 6).unwrap(),
                held: 0,
            },
        ),
    ]);
    let definitions = stocks
        .iter()
        .map(|(id, s)| {
            (
                id.clone(),
                EndpointDefinition {
                    ceiling: s.stock.ceiling,
                },
            )
        })
        .collect();
    let logistics = CompiledLogistics::compile(&config, &ship, &graph, &definitions).unwrap();
    let recipe = ProductionRecipeConfig {
        id: "torpedoes".into(),
        inputs: vec![
            ProductionAmount {
                store: "alloy".into(),
                amount: 2,
            },
            ProductionAmount {
                store: "parts".into(),
                amount: 1,
            },
        ],
        outputs: vec![ProductionAmount {
            store: "hold".into(),
            amount: 1,
        }],
        work_secs: 4.0,
    };
    let config = ProductionConfig {
        system: SystemId("fabrication".into()),
        recipes: vec![recipe],
        initial_recipe: Some("torpedoes".into()),
        initial_target: 4,
    };
    let profile = CompiledProduction::compile(&config, &logistics, &ship).unwrap();
    Fixture {
        logistics: LogisticsState::new(&logistics),
        profile,
        stocks,
        floors: BTreeMap::from([("alloy".into(), 0), ("parts".into(), 1)]),
        ship,
    }
}
fn start(f: &mut Fixture, s: &mut ProductionState, dt: f32, fraction: f32) -> bool {
    s.try_start(
        &f.profile,
        s.last_tick,
        dt,
        &f.floors,
        &mut f.stocks,
        |_, _| Ok(fraction),
    )
    .unwrap()
}
fn tick(f: &mut Fixture, s: &mut ProductionState, dt: f32, fraction: f32) {
    s.tick(&f.profile, s.last_tick + 1, dt, &mut f.stocks, |_, _| {
        Ok(fraction)
    })
    .unwrap();
    s.validate(&f.profile, &f.stocks, &f.logistics).unwrap();
}

#[test]
fn zero_work_has_no_inputs_claims_or_job_and_first_partial_work_commits_once() {
    let mut f = fixture();
    let mut s = ProductionState::new(&f.profile);
    let before = f.stocks.clone();
    assert!(!start(&mut f, &mut s, 1.0, 0.0));
    assert_eq!(f.stocks, before);
    assert_eq!(s.next_id, 1);
    assert!(s.active.is_none());
    assert!(start(&mut f, &mut s, 1.0, 0.5));
    assert_eq!(s.active.as_ref().unwrap().progress, 0.5);
    assert_eq!(f.stocks[&endpoint("alloy")].stock.level, 4);
    assert_eq!(f.stocks[&endpoint("parts")].stock.level, 5);
    assert_eq!(f.stocks[&endpoint("hold")].stock.level, 0);
    assert_eq!(f.stocks[&endpoint("hold")].stock.incoming_total(), 1);
    s.validate(&f.profile, &f.stocks, &f.logistics).unwrap();
    tick(&mut f, &mut s, 1.0, 0.5);
    assert_eq!(s.active.as_ref().unwrap().progress, 1.0);
    assert_eq!(f.stocks[&endpoint("alloy")].stock.level, 4);
    tick(&mut f, &mut s, 99.0, 1.0);
    assert!(s.active.is_none());
    assert_eq!(s.target, 4);
    assert_eq!(f.stocks[&endpoint("hold")].stock.level, 1);
    assert_eq!(f.stocks[&endpoint("hold")].stock.incoming_total(), 0);
}
#[test]
fn floors_source_claims_and_all_output_room_are_checked_before_any_paid_work() {
    let mut f = fixture();
    let mut s = ProductionState::new(&f.profile);
    f.floors.insert("parts".into(), 6);
    let before = f.stocks.clone();
    assert!(!s
        .try_start(&f.profile, 0, 1.0, &f.floors, &mut f.stocks, |_, _| panic!(
            "must not pay while blocked"
        ))
        .unwrap());
    assert_eq!(s.blocked, Some(ProductionBlock::Inputs));
    assert_eq!(f.stocks, before);
    f.floors.insert("parts".into(), 1);
    f.stocks
        .get_mut(&endpoint("alloy"))
        .unwrap()
        .stock
        .reserve("repair", 5)
        .unwrap();
    assert!(!start(&mut f, &mut s, 1.0, 1.0));
    f.stocks
        .get_mut(&endpoint("alloy"))
        .unwrap()
        .stock
        .release("repair");
    f.stocks
        .get_mut(&endpoint("hold"))
        .unwrap()
        .stock
        .reserve_incoming("other", 8)
        .unwrap();
    s.set_target(&f.profile, 8).unwrap();
    assert!(!start(&mut f, &mut s, 1.0, 1.0));
    assert_eq!(s.blocked, Some(ProductionBlock::Target));
    assert_eq!(f.stocks[&endpoint("alloy")].stock.level, 6);
}
#[test]
fn explicit_recipe_change_discards_only_consumed_wip_and_releases_only_own_output_claims() {
    let mut f = fixture();
    let mut s = ProductionState::new(&f.profile);
    f.stocks
        .get_mut(&endpoint("hold"))
        .unwrap()
        .stock
        .reserve_incoming("delivery", 1)
        .unwrap();
    assert!(start(&mut f, &mut s, 1.0, 1.0));
    let used = f.stocks[&endpoint("alloy")].stock.level;
    let before = f.stocks.clone();
    let old = s.clone();
    assert!(s
        .select_recipe(&f.profile, Some("absent"), 0, &mut f.stocks)
        .is_err());
    assert_eq!(f.stocks, before);
    assert_eq!(s, old);
    s.select_recipe(&f.profile, None, 0, &mut f.stocks).unwrap();
    assert_eq!(f.stocks[&endpoint("alloy")].stock.level, used);
    assert_eq!(
        f.stocks[&endpoint("hold")].stock.incoming,
        BTreeMap::from([("delivery".into(), 1)])
    );
    assert!(s.active.is_none());
    assert!(s.last_outcome.as_ref().unwrap().discarded);
}
#[test]
fn target_is_maintained_stock_not_batch_counter_and_incoming_or_held_prevents_duplicates() {
    let mut f = fixture();
    let mut s = ProductionState::new(&f.profile);
    s.set_target(&f.profile, 1).unwrap();
    assert!(start(&mut f, &mut s, 4.0, 1.0));
    tick(&mut f, &mut s, 0.0, 1.0);
    assert_eq!(s.target, 1);
    assert!(!start(&mut f, &mut s, 4.0, 1.0));
    assert_eq!(
        f.stocks
            .get_mut(&endpoint("hold"))
            .unwrap()
            .stock
            .adjust(-1),
        -1
    );
    assert!(start(&mut f, &mut s, 4.0, 1.0));
    tick(&mut f, &mut s, 0.0, 1.0);
    f.stocks
        .get_mut(&endpoint("hold"))
        .unwrap()
        .stock
        .adjust(-1);
    f.stocks.get_mut(&endpoint("hold")).unwrap().held = 1;
    assert!(!start(&mut f, &mut s, 1.0, 1.0));
}
#[test]
fn pause_same_tick_and_json_resume_preserve_wip_and_combined_claims() {
    let mut f = fixture();
    let mut s = ProductionState::new(&f.profile);
    f.stocks.get_mut(&endpoint("hold")).unwrap().stock.adjust(2);
    f.logistics
        .dispatch(f.profile.logistics(), 0, "hold", "magazine", 1, |a| {
            apply_stock_action(f.profile.logistics(), a, &mut f.stocks)
        })
        .unwrap();
    assert!(start(&mut f, &mut s, 1.0, 1.0));
    s.validate(&f.profile, &f.stocks, &f.logistics).unwrap();
    assert!(f
        .logistics
        .validate(f.profile.logistics(), &f.stocks)
        .is_err());
    let mut restored: ProductionState =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    let mut stocks: EndpointMap = f.stocks.clone();
    s.set_enabled(false);
    restored.set_enabled(false);
    s.tick(&f.profile, 1, 9.0, &mut f.stocks, |_, _| panic!("paused"))
        .unwrap();
    restored
        .tick(&f.profile, 1, 9.0, &mut stocks, |_, _| panic!("paused"))
        .unwrap();
    assert_eq!(s, restored);
    assert_eq!(f.stocks, stocks);
    s.set_enabled(true);
    restored.set_enabled(true);
    s.tick(&f.profile, 1, 99.0, &mut f.stocks, |_, _| {
        panic!("duplicate tick")
    })
    .unwrap();
    for next in 2..=5 {
        s.tick(&f.profile, next, 1.0, &mut f.stocks, |_, _| Ok(1.0))
            .unwrap();
        restored
            .tick(&f.profile, next, 1.0, &mut stocks, |_, _| Ok(1.0))
            .unwrap();
        assert_eq!(s, restored);
        assert_eq!(f.stocks, stocks);
        s.validate(&f.profile, &f.stocks, &f.logistics).unwrap();
    }
    let mut unknown = f.stocks.clone();
    unknown
        .get_mut(&endpoint("hold"))
        .unwrap()
        .stock
        .reserve_incoming("unknown", 1)
        .unwrap();
    assert!(s.validate(&f.profile, &unknown, &f.logistics).is_err());
}
#[test]
fn invalid_time_fraction_recipe_and_saved_wip_refuse_without_stock_commit() {
    let mut f = fixture();
    let mut s = ProductionState::new(&f.profile);
    let before = f.stocks.clone();
    assert!(s
        .try_start(&f.profile, 2, 1.0, &f.floors, &mut f.stocks, |_, _| panic!(
            "future"
        ))
        .is_err());
    assert!(s
        .try_start(
            &f.profile,
            0,
            f32::NAN,
            &f.floors,
            &mut f.stocks,
            |_, _| panic!("NaN")
        )
        .is_err());
    assert!(s
        .try_start(&f.profile, 0, 1.0, &f.floors, &mut f.stocks, |_, _| Ok(1.1))
        .is_err());
    assert_eq!(f.stocks, before);
    assert!(s.active.is_none());
    assert!(start(&mut f, &mut s, 1.0, 1.0));
    let mut bad = s.clone();
    bad.active.as_mut().unwrap().inputs[0].amount += 1;
    assert!(bad.validate(&f.profile, &f.stocks, &f.logistics).is_err());
    let mut cfg = f.profile.config().clone();
    cfg.recipes[0].work_secs = f32::INFINITY;
    assert!(CompiledProduction::compile(&cfg, f.profile.logistics(), &f.ship).is_err());
    cfg = f.profile.config().clone();
    let repeated = cfg.recipes[0].inputs[0].clone();
    cfg.recipes[0].inputs.push(repeated);
    assert!(CompiledProduction::compile(&cfg, f.profile.logistics(), &f.ship).is_err());
    cfg = f.profile.config().clone();
    cfg.recipes[0].outputs[0].store = "magazine".into();
    assert!(CompiledProduction::compile(&cfg, f.profile.logistics(), &f.ship).is_err());
}
#[test]
fn multi_output_commit_is_atomic_and_discard_does_not_refund_inputs() {
    let mut f = fixture();
    let mut cfg = f.profile.config().clone();
    cfg.recipes[0].outputs.push(ProductionAmount {
        store: "parts".into(),
        amount: 1,
    });
    f.profile = CompiledProduction::compile(&cfg, f.profile.logistics(), &f.ship).unwrap();
    let mut s = ProductionState::new(&f.profile);
    assert!(start(&mut f, &mut s, 1.0, 1.0));
    assert_eq!(f.stocks[&endpoint("parts")].stock.level, 5);
    assert_eq!(f.stocks[&endpoint("parts")].stock.incoming_total(), 1);
    let before = f.stocks.clone();
    let batch = s.active.as_ref().unwrap();
    let mut malformed = f.stocks.clone();
    let claim = s.claim(batch.id, "parts");
    malformed
        .get_mut(&endpoint("parts"))
        .unwrap()
        .stock
        .release_incoming(&claim);
    let old = malformed.clone();
    assert!(s
        .tick(&f.profile, 1, 5.0, &mut malformed, |_, _| Ok(1.0))
        .is_err());
    assert_eq!(malformed, old);
    assert_eq!(s.active.as_ref().unwrap().progress, 1.0);
    assert_eq!(f.stocks, before);
    tick(&mut f, &mut s, 5.0, 1.0);
    assert_eq!(f.stocks[&endpoint("hold")].stock.level, 1);
    assert_eq!(f.stocks[&endpoint("parts")].stock.level, 6);
    assert!(f.stocks.values().all(|s| s.stock.incoming.is_empty()));
}
#[test]
fn ordinary_production_tree_is_closed_and_uses_published_availability() {
    let bindings = crate::resources::production_ai::bindings();
    let program =
        crate::ai::state_tree::compile(&crate::resources::production_ai::initial_tree(), &bindings)
            .unwrap();
    let mut runtime = crate::ai::state_tree::TreeRuntime::default();
    for (tick, available) in [(1, false), (2, true), (3, false)] {
        let result = crate::ai::state_tree::step(
            &program,
            &BTreeMap::from([
                ("enabled".into(), true),
                ("production_available".into(), available),
            ]),
            &BTreeMap::from([("start".into(), false)]),
            &mut runtime,
            tick,
        )
        .unwrap();
        assert_eq!(result.intents.get("start"), Some(&available));
    }
}

#[test]
fn completed_batch_and_next_start_share_one_system_work_budget() {
    let mut f = fixture();
    let mut s = ProductionState::new(&f.profile);
    assert!(start(&mut f, &mut s, 3.5, 1.0));
    let mut offered = 0.0;
    let result = s
        .tick(&f.profile, 1, 1.0, &mut f.stocks, |_, seconds| {
            offered += seconds;
            Ok(1.0)
        })
        .unwrap();
    assert!(result.completed.is_some());
    assert_eq!(result.work_offered_secs, 0.5);
    assert!(s
        .try_start(
            &f.profile,
            1,
            1.0 - result.work_offered_secs,
            &f.floors,
            &mut f.stocks,
            |_, seconds| {
                offered += seconds;
                Ok(1.0)
            }
        )
        .unwrap());
    assert_eq!(offered, 1.0);
    assert_eq!(s.active.as_ref().unwrap().progress, 0.5);
    s.tick(&f.profile, 1, 10.0, &mut f.stocks, |_, _| {
        panic!("already advanced this tick")
    })
    .unwrap();
    assert_eq!(s.active.as_ref().unwrap().progress, 0.5);
    s.validate(&f.profile, &f.stocks, &f.logistics).unwrap();
}

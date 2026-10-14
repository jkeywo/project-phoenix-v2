use super::*;
use crate::entities::config::{EnergyGroupConfig, EnergyReactorConfig};
use crate::resources::energy::EnergyConfig;
use crate::resources::service::ResourceLink;
use crate::ship::config::{PowerGroupConfig, SystemInstanceConfig};

fn reserve() -> EnergyConfig {
    EnergyConfig {
        capacity: 200.0,
        max_intake_per_sec: 40.0,
        passive_loss_at_full_per_sec: 0.0,
        charge_heat_per_unit: 0.0,
        discharge_heat_per_unit: 0.1,
        passive_cooling_per_sec: 0.0,
        coolant_heat_per_unit: 10.0,
        trip_heat: 1_000.0,
        recover_heat: 100.0,
    }
}
fn group(id: &str, parent: Option<&str>, flow: f32, priority: u16) -> EnergyGroupConfig {
    EnergyGroupConfig {
        id: PowerGroupId(id.into()),
        parent: parent.map(|id| PowerGroupId(id.into())),
        max_flow_per_sec: 40.0,
        default_flow_per_sec: flow,
        max_cooling_per_sec: 10.0,
        default_cooling_per_sec: 2.0,
        priority,
        restore_delay_secs: 0.5,
    }
}
fn fixture(
    groups: Vec<EnergyGroupConfig>,
) -> (
    EnergyConfigSection,
    ShipConfig,
    EnergyCoolantBindings,
    BTreeMap<String, f32>,
) {
    let consumers: Vec<_> = groups
        .iter()
        .map(|group| EnergyConsumerConfig {
            system: SystemId(format!("consumer-{}", group.id.0)),
            initial_energy: 0.0,
            initial_heat: 0.0,
            continuous_cost_per_sec: 2.0,
            discrete_cost: 2.0,
            reserve: reserve(),
            coolant: ResourceLink::default(),
        })
        .collect();
    let ship = ShipConfig {
        consoles: vec![],
        stations: vec![],
        coordination_lag_secs: 0.0,
        systems: groups
            .iter()
            .zip(&consumers)
            .map(|(group, consumer)| SystemInstanceConfig {
                damage_components: Vec::new(),
                id: consumer.system.clone(),
                kind: "test-capability".into(),
                console: None,
                ai_only: true,
                power_group: Some(group.id.clone()),
                marker: None,
                config: None,
                components: vec![],
                channels: vec![],
            })
            .collect(),
        power_groups: groups
            .iter()
            .map(|group| {
                (
                    group.id.clone(),
                    PowerGroupConfig {
                        label: group.id.0.clone(),
                        default_level: 2,
                        min_level: 0,
                        max_level: 4,
                    },
                )
            })
            .collect(),
    };
    let bindings = EnergyCoolantBindings {
        reactor: "coolant".into(),
        consumers: consumers
            .iter()
            .map(|consumer| (consumer.system.clone(), "coolant".into()))
            .collect(),
    };
    let cfg = EnergyConfigSection {
        reactor: EnergyReactorConfig {
            sustainable_output_per_sec: 20.0,
            max_output_per_sec: 40.0,
            ramp_per_sec: 100.0,
            initial_output_per_sec: 20.0,
            initial_energy: 200.0,
            initial_heat: 0.0,
            reserve: reserve(),
            max_cooling_per_sec: 10.0,
            default_cooling_per_sec: 1.0,
            coolant: ResourceLink::default(),
        },
        consumers,
        groups,
    };
    (
        cfg,
        ship,
        bindings,
        BTreeMap::from([("coolant".into(), 100.0)]),
    )
}
fn run(
    state: &mut EnergyNetworkState,
    cfg: &EnergyConfigSection,
    ship: &ShipConfig,
    bindings: &EnergyCoolantBindings,
    coolant: &BTreeMap<String, f32>,
    dt: f32,
) -> EnergyNetworkResult {
    state
        .tick(cfg, ship, dt, bindings, coolant, &mut |_| 0)
        .unwrap()
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}

#[test]
fn random_whole_flow_cuts_preserve_requests_and_never_stop_local_work() {
    let (cfg, ship, bindings, coolant) = fixture(vec![
        group("life-support", None, 6.0, 99),
        group("weapons", None, 6.0, 1),
    ]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    let id = cfg.consumers[0].system.clone();
    state.consumers.get_mut(&id).unwrap().energy = 20.0;
    state.set_output(&cfg, 6.0).unwrap();
    let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(result.cut_groups, [PowerGroupId("life-support".into())]);
    let group = &state.groups[&PowerGroupId("life-support".into())];
    assert!(group.cut);
    assert_eq!(group.desired_flow_per_sec, 6.0);
    assert_eq!(group.effective_flow_per_sec, 0.0);
    assert!(state.try_spend_discrete(&cfg, &id).unwrap());
    assert_eq!(state.consumers[&id].energy, 18.0);
    assert!(!state.consumers[&id].thermal_stopped);
}

#[test]
fn recovery_requires_sustained_headroom_without_evicting_served_groups() {
    let (cfg, ship, bindings, coolant) =
        fixture(vec![group("a", None, 6.0, 0), group("b", None, 6.0, 0)]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.set_output(&cfg, 6.0).unwrap();
    run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    state.set_output(&cfg, 12.0).unwrap();
    let first = run(&mut state, &cfg, &ship, &bindings, &coolant, 0.25);
    assert!(first.restored_groups.is_empty());
    assert!(state.groups[&PowerGroupId("a".into())].cut);
    assert_eq!(
        state.groups[&PowerGroupId("b".into())].effective_flow_per_sec,
        6.0
    );
    let second = run(&mut state, &cfg, &ship, &bindings, &coolant, 0.25);
    assert_eq!(second.restored_groups, [PowerGroupId("a".into())]);
    assert_eq!(
        state.groups[&PowerGroupId("b".into())].effective_flow_per_sec,
        6.0
    );
    assert!(second.cut_groups.is_empty());
}

#[test]
fn minimal_priorities_share_real_supply_and_do_not_modify_manual_requests() {
    let (cfg, ship, bindings, coolant) = fixture(vec![
        group("low", None, 8.0, 1),
        group("high", None, 8.0, 0),
    ]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.mode = EnergyAllocationMode::Minimal;
    state.cooling_usage = CoolingUsage::None;
    state.set_output(&cfg, 10.0).unwrap();
    let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(
        state.groups[&PowerGroupId("high".into())].effective_flow_per_sec,
        8.0
    );
    assert_eq!(
        state.groups[&PowerGroupId("low".into())].effective_flow_per_sec,
        2.0
    );
    assert_eq!(
        state.groups[&PowerGroupId("low".into())].desired_flow_per_sec,
        8.0
    );
    assert_eq!(result.coolant_used["coolant"], 0.0);
    assert!(result.cut_groups.is_empty());
}

#[test]
fn parent_caps_children_and_direct_consumers_without_double_counting_power() {
    let (cfg, ship, bindings, coolant) = fixture(vec![
        group("root", None, 10.0, 0),
        group("port", Some("root"), 6.0, 0),
        group("starboard", Some("root"), 6.0, 0),
    ]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(result.cut_groups, [PowerGroupId("port".into())]);
    assert_eq!(
        state.groups[&PowerGroupId("root".into())].effective_flow_per_sec,
        10.0
    );
    assert_eq!(
        state.consumers[&SystemId("consumer-port".into())].energy,
        0.0
    );
    assert_eq!(
        state.consumers[&SystemId("consumer-starboard".into())].energy,
        6.0
    );
    assert_eq!(
        state.consumers[&SystemId("consumer-root".into())].energy,
        4.0
    );
    assert_eq!(result.delivered_energy, 10.0);
    assert_eq!(state.reactor.energy, 190.0);
}

#[test]
fn headroom_rejection_stays_at_reactor_and_is_not_donated_to_other_groups() {
    let (cfg, ship, bindings, coolant) = fixture(vec![
        group("full", None, 10.0, 0),
        group("empty", None, 10.0, 0),
    ]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state
        .consumers
        .get_mut(&SystemId("consumer-full".into()))
        .unwrap()
        .energy = 200.0;
    let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(
        state.consumers[&SystemId("consumer-empty".into())].energy,
        10.0
    );
    assert_eq!(state.reactor.energy, 190.0);
    assert_eq!(result.delivered_energy, 10.0);
}

#[test]
fn generation_ramps_and_extra_export_uses_the_same_capacitor() {
    let (mut cfg, ship, bindings, coolant) = fixture(vec![group("a", None, 30.0, 0)]);
    cfg.reactor.initial_output_per_sec = 0.0;
    cfg.reactor.ramp_per_sec = 10.0;
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.mode = EnergyAllocationMode::Minimal;
    state.set_output(&cfg, 40.0).unwrap();
    let a = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(state.output_actual_per_sec, 10.0);
    assert_eq!(a.delivered_energy, 10.0);
    run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    let c = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(state.output_actual_per_sec, 30.0);
    assert_eq!(c.generation_accepted, 20.0);
    assert_eq!(c.delivered_energy, 30.0);
    assert!(state.reactor.energy < cfg.reactor.initial_energy);
    close(state.reactor.heat, 3.0);
    close(c.coolant_used["coolant"], 0.1);
}

#[test]
fn coolant_is_shared_real_budget_and_cool_equipment_does_not_spend() {
    let (cfg, ship, bindings, _) =
        fixture(vec![group("a", None, 0.0, 0), group("b", None, 0.0, 0)]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    let coolant = BTreeMap::from([("coolant".into(), 0.25)]);
    for consumer in state.consumers.values_mut() {
        consumer.heat = 10.0;
    }
    state.reactor.heat = 10.0;
    let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(result.coolant_used["coolant"], 0.25);
    close(
        state
            .consumers
            .values()
            .map(|state| state.heat)
            .sum::<f32>()
            + state.reactor.heat,
        27.5,
    );
    for consumer in state.consumers.values_mut() {
        consumer.heat = 0.0;
    }
    state.reactor.heat = 0.0;
    assert_eq!(
        run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0).coolant_used["coolant"],
        0.0
    );
}

#[test]
fn reactor_trip_outputs_zero_and_recovers_without_resetting_the_requested_output() {
    let (mut cfg, ship, bindings, coolant) = fixture(vec![group("a", None, 10.0, 0)]);
    cfg.reactor.reserve.trip_heat = 10.0;
    cfg.reactor.reserve.recover_heat = 2.0;
    cfg.reactor.initial_heat = 12.0;
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.set_reactor_cooling(&cfg, 10.0).unwrap();
    let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(result.delivered_energy, 0.0);
    assert_eq!(state.output_actual_per_sec, 0.0);
    assert_eq!(state.output_target_per_sec, 20.0);
    assert!(!state.reactor.thermal_stopped);
    run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert!(state.output_actual_per_sec > 0.0);
}

#[test]
fn fine_settings_and_affordable_work_use_the_existing_consumer_reserve() {
    let (cfg, ship, bindings, coolant) = fixture(vec![group("a", None, 10.0, 0)]);
    let id = cfg.consumers[0].system.clone();
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.set_fine_flow(&cfg, &ship, &id, Some(3.0)).unwrap();
    run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(state.consumers[&id].energy, 3.0);
    assert!(state.try_spend_discrete(&cfg, &id).unwrap());
    assert_eq!(
        state.spend_continuous(&cfg, &id, 2.0).unwrap().fraction,
        0.5
    );
    assert!(!state.try_spend_discrete(&cfg, &id).unwrap());
    assert!(state.set_fine_flow(&cfg, &ship, &id, Some(50.0)).is_err());
    state.set_fine_flow(&cfg, &ship, &id, None).unwrap();
    assert!(!state.fine_flows.contains_key(&id));
}

#[test]
fn snapshot_restore_preserves_cut_recovery_reserves_output_and_cooling() {
    let (cfg, ship, bindings, coolant) =
        fixture(vec![group("a", None, 6.0, 0), group("b", None, 6.0, 0)]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.set_output(&cfg, 6.0).unwrap();
    run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    state.set_output(&cfg, 12.0).unwrap();
    run(&mut state, &cfg, &ship, &bindings, &coolant, 0.25);
    let mut restored: EnergyNetworkState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    restored.validate_against(&cfg, &ship).unwrap();
    for _ in 0..10 {
        let a = run(&mut state, &cfg, &ship, &bindings, &coolant, 0.25);
        let b = run(&mut restored, &cfg, &ship, &bindings, &coolant, 0.25);
        assert_eq!(a, b);
        assert_eq!(state, restored);
    }
}

#[test]
fn malformed_inputs_and_unknown_saved_equipment_refuse_before_mutation() {
    let (cfg, ship, bindings, coolant) = fixture(vec![group("a", None, 6.0, 0)]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    let original = state.clone();
    for dt in [f32::NAN, f32::INFINITY, -1.0] {
        assert!(state
            .tick(&cfg, &ship, dt, &bindings, &coolant, &mut |_| 0)
            .is_err());
        assert_eq!(state, original);
    }
    assert!(state
        .tick(&cfg, &ship, 1.0, &bindings, &BTreeMap::new(), &mut |_| 0)
        .is_err());
    assert_eq!(state, original);
    let mut bad = state.clone();
    bad.consumers.remove(&cfg.consumers[0].system);
    assert!(bad.validate_against(&cfg, &ship).is_err());
    assert!(state
        .set_group(&cfg, &PowerGroupId("a".into()), f32::NAN, 0.0)
        .is_err());
    assert_eq!(state, original);
}

#[test]
fn saved_child_budget_and_nonzero_tripped_output_are_refused() {
    let (cfg, ship, _, _) = fixture(vec![
        group("root", None, 10.0, 0),
        group("child", Some("root"), 6.0, 0),
    ]);
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state
        .groups
        .get_mut(&PowerGroupId("child".into()))
        .unwrap()
        .effective_flow_per_sec = 1.0;
    assert!(state
        .validate_against(&cfg, &ship)
        .unwrap_err()
        .contains("parent budget"));
    state
        .groups
        .get_mut(&PowerGroupId("child".into()))
        .unwrap()
        .effective_flow_per_sec = 0.0;
    state.reactor.thermal_stopped = true;
    assert!(state
        .validate_against(&cfg, &ship)
        .unwrap_err()
        .contains("actual output"));
}

#[test]
fn first_list_rank_receives_unequal_scarce_flow_and_shared_coolant_first() {
    // The command applier/UI rerank sibling list indexes with zero first.
    let (mut cfg, ship, bindings, _) = fixture(vec![
        group("last-alphabetically", None, 3.0, 0),
        group("first-alphabetically", None, 8.0, 1),
    ]);
    cfg.reactor.default_cooling_per_sec = 0.0;
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.mode = EnergyAllocationMode::Minimal;
    state.set_output(&cfg, 5.0).unwrap();
    for consumer in state.consumers.values_mut() {
        consumer.heat = 10.0;
    }
    let result = run(
        &mut state,
        &cfg,
        &ship,
        &bindings,
        &BTreeMap::from([("coolant".into(), 0.1)]),
        1.0,
    );
    assert_eq!(
        state.groups[&PowerGroupId("last-alphabetically".into())].effective_flow_per_sec,
        3.0
    );
    assert_eq!(
        state.groups[&PowerGroupId("first-alphabetically".into())].effective_flow_per_sec,
        2.0
    );
    close(
        state.consumers[&SystemId("consumer-last-alphabetically".into())].heat,
        9.0,
    );
    close(
        state.consumers[&SystemId("consumer-first-alphabetically".into())].heat,
        10.0,
    );
    assert_eq!(result.coolant_used["coolant"], 0.1);
}

#[test]
fn nested_first_list_rank_gets_parent_flow_and_cooling_without_double_counting() {
    let (mut cfg, ship, bindings, coolant) = fixture(vec![
        group("root", None, 7.0, 0),
        group("z-first", Some("root"), 3.0, 0),
        group("a-second", Some("root"), 8.0, 1),
    ]);
    cfg.groups[0].default_cooling_per_sec = 3.0;
    cfg.groups[1].default_cooling_per_sec = 5.0;
    cfg.groups[2].default_cooling_per_sec = 5.0;
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.mode = EnergyAllocationMode::Minimal;
    state.set_output(&cfg, 7.0).unwrap();
    state
        .consumers
        .get_mut(&SystemId("consumer-z-first".into()))
        .unwrap()
        .heat = 10.0;
    state
        .consumers
        .get_mut(&SystemId("consumer-a-second".into()))
        .unwrap()
        .heat = 10.0;
    let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    let first = &state.groups[&PowerGroupId("z-first".into())];
    let second = &state.groups[&PowerGroupId("a-second".into())];
    assert_eq!(
        (first.effective_flow_per_sec, second.effective_flow_per_sec),
        (3.0, 4.0)
    );
    assert_eq!(
        (
            first.effective_cooling_per_sec,
            second.effective_cooling_per_sec
        ),
        (3.0, 0.0)
    );
    assert_eq!(result.delivered_energy, 7.0);
    assert_eq!(
        result.consumer_reports[&SystemId("consumer-root".into())].accepted_energy,
        0.0
    );
    close(
        state.consumers[&SystemId("consumer-z-first".into())].heat,
        7.0,
    );
    close(
        state.consumers[&SystemId("consumer-a-second".into())].heat,
        10.0,
    );
    state.validate_against(&cfg, &ship).unwrap();
}

#[test]
fn repeated_shared_fractional_cooling_never_rounds_its_debit_down_or_above_budget() {
    for available in [1.0, 100.0] {
        let (mut cfg, ship, bindings, _) = fixture(
            (0..5)
                .map(|n| group(&format!("g{n}"), None, 0.0, n))
                .collect(),
        );
        cfg.reactor.default_cooling_per_sec = 0.0;
        let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
        let coolant = BTreeMap::from([("coolant".into(), available)]);
        for _ in 0..20 {
            for consumer in state.consumers.values_mut() {
                consumer.heat = 10.0;
            }
            let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
            let actual_reports = result
                .consumer_reports
                .values()
                .map(|report| report.coolant_used as f64)
                .sum::<f64>();
            let debit = result.coolant_used["coolant"];
            assert!(debit as f64 >= actual_reports);
            assert!(debit <= available);
            assert!(debit.is_finite());
        }
    }
}

#[test]
fn scarce_shared_coolant_respects_root_rank_before_equal_child_ranks() {
    let (mut cfg, ship, bindings, _) = fixture(vec![
        group("z-first-root", None, 0.0, 0),
        group("a-second-root", None, 0.0, 1),
        group("z-first-leaf", Some("z-first-root"), 0.0, 0),
        group("a-second-leaf", Some("a-second-root"), 0.0, 0),
    ]);
    cfg.reactor.default_cooling_per_sec = 0.0;
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.mode = EnergyAllocationMode::Minimal;
    let first = SystemId("consumer-z-first-leaf".into());
    let second = SystemId("consumer-a-second-leaf".into());
    state.consumers.get_mut(&first).unwrap().heat = 10.0;
    state.consumers.get_mut(&second).unwrap().heat = 10.0;
    let result = run(
        &mut state,
        &cfg,
        &ship,
        &bindings,
        &BTreeMap::from([("coolant".into(), 0.1)]),
        1.0,
    );
    close(state.consumers[&first].heat, 9.0);
    close(state.consumers[&second].heat, 10.0);
    assert_eq!(result.coolant_used["coolant"], 0.1);
    assert_eq!(result.delivered_energy, 0.0);
    state.validate_against(&cfg, &ship).unwrap();
}

#[test]
fn disabled_generation_still_exports_finite_reserve_and_preserves_cooling_and_continuation() {
    let (mut cfg, ship, bindings, coolant) = fixture(vec![group("work", None, 10.0, 0)]);
    cfg.reactor.reserve.passive_loss_at_full_per_sec = 2.0;
    cfg.reactor.default_cooling_per_sec = 5.0;
    let mut state = EnergyNetworkState::new(&cfg, &ship).unwrap();
    state.generation_enabled = false;
    state.reactor.heat = 80.0;
    let result = run(&mut state, &cfg, &ship, &bindings, &coolant, 1.0);
    assert_eq!(result.generation_accepted, 0.0);
    assert_eq!(result.generation_rejected, 0.0);
    assert!(result.delivered_energy > 0.0);
    assert!(state.consumers[&SystemId("consumer-work".into())].energy > 0.0);
    assert!(state.reactor.energy < cfg.reactor.initial_energy - result.delivered_energy);
    assert!(state.reactor.heat < 80.0);
    assert!(result.coolant_used["coolant"] > 0.0);
    let encoded = serde_json::to_string(&state).unwrap();
    let mut resumed: EnergyNetworkState = serde_json::from_str(&encoded).unwrap();
    resumed.validate_against(&cfg, &ship).unwrap();
    assert_eq!(state, resumed);
    run(&mut state, &cfg, &ship, &bindings, &coolant, 0.5);
    run(&mut resumed, &cfg, &ship, &bindings, &coolant, 0.5);
    assert_eq!(state, resumed);
    state.generation_enabled = true;
    assert!(run(&mut state, &cfg, &ship, &bindings, &coolant, 0.5).generation_accepted > 0.0);
}

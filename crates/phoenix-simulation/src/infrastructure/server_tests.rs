use super::*;
use crate::infrastructure::condition::{
    CapacityConfig, ConditionAdjustment, InfrastructureConfig, ThresholdConfig,
};

const FLAG: &str = "depot_transfer_capable";

fn depot_config(decay_per_sec: f32) -> InfrastructureConfig {
    InfrastructureConfig {
        condition_max: 100.0,
        decay_per_sec,
        capacities: vec![CapacityConfig {
            label: None,
            id: "depot_transfer_throughput".to_string(),
            amount: 40,
            ceiling: None,
        }],
        thresholds: vec![ThresholdConfig {
            label: None,
            flag: FLAG.to_string(),
            capacity: None,
            fails_below: 0.4,
            restores_above: None,
        }],
        ..Default::default()
    }
}

/// A bare app with the one system under test, ticked by hand. No
/// `TimePlugin`: a decay-free fixture must not depend on a clock, and the
/// decay tests insert `Time` themselves.
fn app_with(config: &InfrastructureConfig) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<WorldContentRuntime>();
    app.init_resource::<EffectQueue<ConditionAdjustment>>();
    app.init_resource::<EffectQueue<CapacityAdjustment>>();
    app.add_systems(Update, tick_infrastructure_condition);
    let entity = app
        .world_mut()
        .spawn((
            EntityUuid("depot-1".to_string()),
            InfrastructureCondition(InfrastructureState::from_config(config)),
        ))
        .id();
    (app, entity)
}

fn flag(app: &App, name: &str) -> bool {
    app.world()
        .resource::<WorldContentRuntime>()
        .flags
        .flag(name)
}

fn counter(app: &App, name: &str) -> i64 {
    app.world()
        .resource::<WorldContentRuntime>()
        .flags
        .counter(name)
}

fn drain_events(app: &mut App) -> Vec<WorldEvent> {
    std::mem::take(
        &mut app
            .world_mut()
            .resource_mut::<WorldContentRuntime>()
            .pending_world_events,
    )
}

// ── AC3: the flag surface ──

#[test]
fn a_structure_publishes_its_flags_and_capacities_on_its_first_tick() {
    let (mut app, _) = app_with(&depot_config(0.0));
    app.update();
    assert!(
        flag(&app, FLAG),
        "an intact depot's operational flag is up in the world store, where a script \
             predicate can read it"
    );
    assert_eq!(
        counter(&app, "depot_transfer_throughput"),
        40,
        "…and its authored capacity is a readable counter, so a scenario asks the depot \
             how much it moves instead of restating the number"
    );
    let events = drain_events(&mut app);
    assert_eq!(
        events,
        vec![WorldEvent::FlagSet {
            name: FLAG.to_string(),
            origin_layer: None,
        }],
        "exactly one world event: the flag going up. The capacity counter deliberately \
             fires none — a published quantity is not an operational event."
    );
}

#[test]
fn a_crossing_writes_the_store_and_queues_the_event_a_hook_reacts_to() {
    let (mut app, entity) = app_with(&depot_config(0.0));
    app.update();
    drain_events(&mut app);

    app.world_mut()
        .resource_mut::<EffectQueue<ConditionAdjustment>>()
        .0
        .push(ConditionAdjustment {
            uuid: "depot-1".to_string(),
            delta: -65.0,
        });
    app.update();

    assert!(
        !flag(&app, FLAG),
        "crossing the authored threshold clears the flag in the world store"
    );
    assert_eq!(
        drain_events(&mut app),
        vec![WorldEvent::FlagCleared {
            name: FLAG.to_string(),
            origin_layer: None,
        }],
        "…and queues the FlagCleared a scenario's on_flag_cleared hook fires from"
    );

    app.world_mut()
        .resource_mut::<EffectQueue<ConditionAdjustment>>()
        .0
        .push(ConditionAdjustment {
            uuid: "depot-1".to_string(),
            delta: 20.0,
        });
    app.update();
    assert!(flag(&app, FLAG), "and a repair puts it back");
    assert_eq!(
        drain_events(&mut app),
        vec![WorldEvent::FlagSet {
            name: FLAG.to_string(),
            origin_layer: None,
        }],
        "…with the matching FlagSet — the flag flips in BOTH directions"
    );
    let condition = app
        .world()
        .get::<InfrastructureCondition>(entity)
        .expect("the component is still attached");
    assert_eq!(condition.0.condition(), 55.0);
}

#[test]
fn a_capacity_threshold_crossing_queues_the_same_authoritative_flag_event() {
    let config = InfrastructureConfig {
        capacities: vec![CapacityConfig {
            id: "reserve_fuel".to_string(),
            amount: 0,
            ceiling: Some(100),
            label: None,
        }],
        thresholds: vec![ThresholdConfig {
            flag: "transfer_primed".to_string(),
            capacity: Some("reserve_fuel".to_string()),
            fails_below: 0.5,
            restores_above: Some(0.5),
            label: None,
        }],
        ..Default::default()
    };
    let (mut app, _) = app_with(&config);
    app.update();
    assert!(!flag(&app, "transfer_primed"));
    assert!(drain_events(&mut app).is_empty());

    app.world_mut()
        .resource_mut::<EffectQueue<CapacityAdjustment>>()
        .0
        .push(CapacityAdjustment {
            uuid: "depot-1".to_string(),
            capacity: "reserve_fuel".to_string(),
            delta: 50,
        });
    app.update();

    assert_eq!(counter(&app, "reserve_fuel"), 50);
    assert!(flag(&app, "transfer_primed"));
    assert_eq!(
        drain_events(&mut app),
        vec![WorldEvent::FlagSet {
            name: "transfer_primed".to_string(),
            origin_layer: None,
        }],
        "the receiving entity's own threshold supplies the on_flag_set seam"
    );
}

#[test]
fn a_first_tick_capacity_drain_publishes_both_sides_of_the_edge() {
    let config = InfrastructureConfig {
        capacities: vec![CapacityConfig {
            id: "reserve_fuel".to_string(),
            amount: 100,
            ceiling: Some(100),
            label: None,
        }],
        thresholds: vec![ThresholdConfig {
            flag: "transfer_primed".to_string(),
            capacity: Some("reserve_fuel".to_string()),
            fails_below: 0.5,
            restores_above: Some(0.5),
            label: None,
        }],
        ..Default::default()
    };
    let (mut app, _) = app_with(&config);
    app.world_mut()
        .resource_mut::<EffectQueue<CapacityAdjustment>>()
        .0
        .push(CapacityAdjustment {
            uuid: "depot-1".to_string(),
            capacity: "reserve_fuel".to_string(),
            delta: -100,
        });

    app.update();

    assert_eq!(counter(&app, "reserve_fuel"), 0);
    assert!(!flag(&app, "transfer_primed"));
    assert_eq!(
        drain_events(&mut app),
        vec![
            WorldEvent::FlagSet {
                name: "transfer_primed".to_string(),
                origin_layer: None,
            },
            WorldEvent::FlagCleared {
                name: "transfer_primed".to_string(),
                origin_layer: None,
            },
        ],
        "initial truth is published before the same tick's drain, preserving the exact edge"
    );
}

#[test]
fn a_queued_adjustment_for_an_unknown_entity_is_simply_not_applied() {
    let (mut app, entity) = app_with(&depot_config(0.0));
    app.update();
    app.world_mut()
        .resource_mut::<EffectQueue<ConditionAdjustment>>()
        .0
        .push(ConditionAdjustment {
            uuid: "no-such-depot".to_string(),
            delta: -90.0,
        });
    app.update();
    let condition = app.world().get::<InfrastructureCondition>(entity).unwrap();
    assert_eq!(
        condition.0.condition(),
        100.0,
        "an adjustment naming an entity that is not there must not land on whichever \
             structure happens to be first"
    );
    assert!(
        app.world()
            .resource::<EffectQueue<ConditionAdjustment>>()
            .0
            .is_empty(),
        "…and the queue is drained regardless, so a stale name cannot accumulate"
    );
}

// ── Decay ──

#[test]
fn authored_decay_walks_a_structure_down_through_its_threshold() {
    let mut app = App::new();
    app.init_resource::<WorldContentRuntime>();
    app.init_resource::<EffectQueue<ConditionAdjustment>>();
    app.init_resource::<EffectQueue<CapacityAdjustment>>();
    app.insert_resource(Time::<()>::default());
    app.add_systems(Update, tick_infrastructure_condition);
    app.world_mut().spawn((
        EntityUuid("depot-1".to_string()),
        InfrastructureCondition(InfrastructureState::from_config(&depot_config(10.0))),
    ));
    // One second per update, so ten condition points a tick.
    for _ in 0..7 {
        app.world_mut()
            .resource_mut::<Time<()>>()
            .advance_by(std::time::Duration::from_secs(1));
        app.update();
    }
    assert!(
        !flag(&app, FLAG),
        "seventy points of authored decay takes a depot below its 40 % threshold with no \
             damage and no script involved"
    );
}

#[test]
fn a_structure_with_nothing_to_do_leaves_the_runtime_unmarked() {
    let (mut app, _) = app_with(&depot_config(0.0));
    app.update();
    app.update();
    let changed = app
        .world()
        .resource_ref::<WorldContentRuntime>()
        .is_changed();
    assert!(
        !changed,
        "a static structure on a quiet tick must not mark WorldContentRuntime changed — \
             every world in the repo carries that resource, and a needless mark is a needless \
             wake-up for everything that watches it"
    );
}

// ── Determinism ──

#[test]
fn structures_are_walked_in_uuid_order_whatever_order_they_spawned_in() {
    let mut config = depot_config(0.0);
    config.thresholds[0].flag = "shared_flag".to_string();
    config.capacities.clear();
    let mut forward = App::new();
    forward.init_resource::<WorldContentRuntime>();
    forward.init_resource::<EffectQueue<ConditionAdjustment>>();
    forward.init_resource::<EffectQueue<CapacityAdjustment>>();
    forward.add_systems(Update, tick_infrastructure_condition);
    for uuid in ["depot-a", "depot-b", "depot-c"] {
        forward.world_mut().spawn((
            EntityUuid(uuid.to_string()),
            InfrastructureCondition(InfrastructureState::from_config(&config)),
        ));
    }
    forward.update();

    let mut reverse = App::new();
    reverse.init_resource::<WorldContentRuntime>();
    reverse.init_resource::<EffectQueue<ConditionAdjustment>>();
    reverse.init_resource::<EffectQueue<CapacityAdjustment>>();
    reverse.add_systems(Update, tick_infrastructure_condition);
    for uuid in ["depot-c", "depot-b", "depot-a"] {
        reverse.world_mut().spawn((
            EntityUuid(uuid.to_string()),
            InfrastructureCondition(InfrastructureState::from_config(&config)),
        ));
    }
    reverse.update();

    assert_eq!(
        drain_events(&mut forward),
        drain_events(&mut reverse),
        "the emitted event sequence is a function of the UUIDs, not of the order the \
             entities happen to sit in the archetype — two hosts that spawned the same \
             structures in different orders must agree"
    );
}

use super::*;
use std::collections::BTreeMap;

fn definition() -> AiTreeConfig {
    serde_json::from_value(serde_json::json!({
        "root":"reactor", "tree":[
            {"id":"reactor", "initial":"supply", "memory":{"rationing":false}, "state":[{
                "id":"supply", "task":[
                    {"id":"baseline", "action":{"kind":"write_number","channel":"output","value":6.0}},
                    {"id":"reserve-low", "when":[{"value":{"source":"number_compare","name":"reserve_fraction","comparison":"less","value":0.25},"equals":true}],
                     "action":{"kind":"remember","key":"rationing","value":{"source":"literal","value":true}}},
                    {"id":"cool", "action":{"kind":"call","tree":"cooling"}}
                ]
            }]},
            {"id":"base-cooling", "initial":"active", "state":[{
                "id":"active", "task":[{"id":"supply", "action":{"kind":"write_number","channel":"cooling","value":1.0}}]
            }]},
            {"id":"cooling", "extends":"base-cooling", "state":[{
                "id":"active", "task":[{"id":"hot", "when":[{"value":{"source":"number_compare","name":"heat","comparison":"greater_or_equal","value":80.0},"equals":true}],
                  "action":{"kind":"write_number","channel":"cooling","value":3.0}}]
            }]}
        ]
    })).unwrap()
}
fn bindings() -> NumericBindings {
    NumericBindings {
        facts: ["reserve_fraction".into(), "heat".into()].into(),
        channels: BTreeMap::from([
            ("output".into(), NumericBounds { min: 0.0, max: 8.0 }),
            ("cooling".into(), NumericBounds { min: 0.0, max: 3.0 }),
        ]),
    }
}

#[test]
fn bounded_numeric_tasks_preserve_inheritance_reference_order_and_continuation() {
    let program =
        compile_with_numbers(&definition(), &SupportedBindings::default(), &bindings()).unwrap();
    let mut runtime = TreeRuntime::default();
    let hot = BTreeMap::from([("heat".into(), 80.0), ("reserve_fraction".into(), 0.2)]);
    let result = step_with_numbers(
        &program,
        &BTreeMap::new(),
        &hot,
        &BTreeMap::new(),
        &mut runtime,
        1,
    )
    .unwrap();
    assert_eq!(
        result.numeric_intents,
        BTreeMap::from([("output".into(), 6.0), ("cooling".into(), 3.0)])
    );
    assert_eq!(runtime.memory("rationing"), Some(true));
    assert!(result.intents.is_empty());
    let mut restored: TreeRuntime =
        serde_json::from_slice(&serde_json::to_vec(&runtime).unwrap()).unwrap();
    let cool = BTreeMap::from([("heat".into(), 20.0), ("reserve_fraction".into(), 0.8)]);
    let next = step_with_numbers(
        &program,
        &BTreeMap::new(),
        &cool,
        &BTreeMap::new(),
        &mut runtime,
        2,
    )
    .unwrap();
    assert_eq!(
        next,
        step_with_numbers(
            &program,
            &BTreeMap::new(),
            &cool,
            &BTreeMap::new(),
            &mut restored,
            2
        )
        .unwrap()
    );
    assert_eq!(runtime, restored);
    assert_eq!(next.numeric_intents["cooling"], 1.0);
}

#[test]
fn numeric_access_and_outputs_are_closed_and_failure_is_transactional() {
    assert!(compile(&definition(), &SupportedBindings::default()).is_err());
    let mut config = definition();
    config.tree[0].state[0].task[0].action = TaskAction::WriteNumber {
        channel: "output".into(),
        value: 9.0,
    };
    assert!(compile_with_numbers(&config, &SupportedBindings::default(), &bindings()).is_err());
    config.tree[0].state[0].task[0].action = TaskAction::WriteNumber {
        channel: "unowned".into(),
        value: 1.0,
    };
    assert!(compile_with_numbers(&config, &SupportedBindings::default(), &bindings()).is_err());
    let program =
        compile_with_numbers(&definition(), &SupportedBindings::default(), &bindings()).unwrap();
    let mut runtime = TreeRuntime::default();
    let bad = BTreeMap::from([("heat".into(), f64::NAN), ("reserve_fraction".into(), 0.5)]);
    assert!(step_with_numbers(
        &program,
        &BTreeMap::new(),
        &bad,
        &BTreeMap::new(),
        &mut runtime,
        1
    )
    .is_err());
    assert_eq!(runtime, TreeRuntime::default());
    let hidden = BTreeMap::from([
        ("heat".into(), 20.0),
        ("reserve_fraction".into(), 0.5),
        ("enemy_private".into(), 1.0),
    ]);
    assert!(step_with_numbers(
        &program,
        &BTreeMap::new(),
        &hidden,
        &BTreeMap::new(),
        &mut runtime,
        1
    )
    .is_err());
    assert_eq!(runtime, TreeRuntime::default());
    let mut invalid_bounds = bindings();
    invalid_bounds.channels.get_mut("output").unwrap().max = f64::INFINITY;
    assert!(compile_with_numbers(
        &definition(),
        &SupportedBindings::default(),
        &invalid_bounds
    )
    .is_err());
}

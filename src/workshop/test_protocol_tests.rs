use super::*;

#[test]
fn flattened_trace_event_round_trips_through_native_status_json() {
    let record = TestTraceRecord {
        tick: 7,
        order: 2,
        source: TestTraceSource {
            path: Some("assets/worlds/test.rhai".into()),
            line: Some(4),
        },
        kind: TestTraceKind::CallbackScheduled {
            function: "later".into(),
            fire_tick: 12,
        },
    };
    let encoded = serde_json::to_string(&record).expect("encode trace record");
    assert_eq!(
        serde_json::from_str::<TestTraceRecord>(&encoded).expect("decode trace record"),
        record
    );
}

#[test]
fn typed_breakpoints_refuse_paths_and_expression_shaped_fields() {
    let breakpoint = TestBreakpoint {
        layer: Some("assets/worlds/arrival.toml".into()),
        condition: TestBreakpointCondition::Counter {
            name: "arrivals".into(),
            comparison: TestBreakpointComparison::Ge,
            value: 2,
        },
    };
    assert!(breakpoint.validate().is_ok());
    assert!(breakpoint.matches(2));
    let encoded = serde_json::to_string(&breakpoint).unwrap();
    assert_eq!(
        serde_json::from_str::<TestBreakpoint>(&encoded).unwrap(),
        breakpoint
    );
    assert!(serde_json::from_str::<TestBreakpoint>(
        r#"{"condition":{"kind":"flag","name":"ready","value":true,"expression":"debug()"}}"#,
    )
    .is_err());
    assert!(TestBreakpoint {
        layer: Some("assets/worlds/../private.toml".into()),
        condition: TestBreakpointCondition::Flag {
            name: "ready".into(),
            value: true
        },
    }
    .validate()
    .is_err());
}

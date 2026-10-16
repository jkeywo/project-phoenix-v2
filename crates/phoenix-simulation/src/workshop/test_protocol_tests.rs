use super::*;

#[test]
fn full_status_frame_includes_prefix_and_newline_and_refuses_oversize_without_truncating_success() {
    let launch = Launch {
        selection: TestSelection {
            world: "assets/worlds/test.toml".into(),
            slot: None,
            ship: "assets/entities/alliance_cruiser.toml".into(),
            seed: 13,
        },
        revision: "capture".into(),
        breakpoint: None,
        rehearsal: None,
        run_generation: 0,
    };
    let mut status = TestStatus::starting(&launch);
    let json = status.bounded_json().unwrap();
    assert!("PHOENIX_WORKSHOP_TEST ".len() + json.len() + 1 <= 65_536);
    status.trace.push(TestTraceRecord {
        tick: 1,
        order: 0,
        source: TestTraceSource {
            path: None,
            line: None,
        },
        kind: TestTraceKind::RecipientDiagnostic {
            action: "test".into(),
            message: "é".repeat(40_000),
        },
    });
    let refused: TestStatus = serde_json::from_str(&status.bounded_json().unwrap()).unwrap();
    assert_eq!(refused.error.as_deref(), Some("result-too-large"));
    assert!(!refused.running && refused.trace.is_empty());
    assert_eq!(refused.selection, launch.selection);
    assert_eq!(
        status.trace.len(),
        1,
        "encoding never mutates the child owner's original evidence"
    );
}

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

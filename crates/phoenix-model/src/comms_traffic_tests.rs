use super::*;
fn channels() -> Vec<String> {
    vec!["public".into(), "fleet".into(), "service".into()]
}

#[test]
fn optional_coverage_requires_exact_order_without_counting_traffic() {
    let channels = channels();
    let valid = CommsListeningConfig {
        simultaneous_channels: 2,
        priority: channels.clone(),
    };
    assert!(valid.validate(&channels).is_ok());
    for priority in [
        vec![],
        vec!["public".into()],
        vec!["public".into(), "public".into(), "fleet".into()],
        vec!["public".into(), "fleet".into(), "hidden".into()],
    ] {
        assert!(CommsListeningConfig {
            priority,
            ..valid.clone()
        }
        .validate(&channels)
        .is_err());
    }
    for simultaneous_channels in [0, 4] {
        assert!(CommsListeningConfig {
            simultaneous_channels,
            ..valid.clone()
        }
        .validate(&channels)
        .is_err());
    }
    assert!(!exact_channel_order(
        &["public".into(), "public".into()],
        &["public".into(), "public".into()]
    ));
}

#[test]
fn contextual_profile_is_closed_and_requires_distinct_bounded_intent_texts() {
    let valid = CommsAlertTrafficConfig {
        source: SystemId("captain".into()),
        channel: "public".into(),
        minimum_spacing_ticks: 120,
        routine_text: "We have selected routine alert.".into(),
        red_alert_text: "We have selected red alert.".into(),
    };
    assert!(valid.validate(&channels()).is_ok());
    for fault in 0..6 {
        let mut invalid = valid.clone();
        match fault {
            0 => invalid.source.0.clear(),
            1 => invalid.channel = "hidden".into(),
            2 => invalid.minimum_spacing_ticks = 0,
            3 => invalid.routine_text.clear(),
            4 => invalid.red_alert_text = invalid.routine_text.clone(),
            _ => invalid.red_alert_text = "x".repeat(4097),
        };
        assert!(invalid.validate(&channels()).is_err());
    }
    let mut extra = serde_json::to_value(&valid).unwrap();
    extra["private_actor"] = "secret".into();
    assert!(serde_json::from_value::<CommsAlertTrafficConfig>(extra).is_err());
    assert!(CommsTrafficConfig {
        listening: None,
        alert: None
    }
    .validate(&channels())
    .is_err());
    assert!(serde_json::from_str::<ReportedAlert>("\"work_completed\"").is_err());
}

use super::*;
use phoenix_model::{
    comms_traffic::{CommsAlertTrafficConfig, CommsListeningConfig},
    observation::ObservationProvenance,
};
fn channels() -> Vec<String> {
    vec!["public".into(), "fleet".into(), "service".into()]
}
fn profile() -> CommsTrafficConfig {
    CommsTrafficConfig {
        listening: Some(CommsListeningConfig {
            simultaneous_channels: 2,
            priority: channels(),
        }),
        alert: Some(CommsAlertTrafficConfig {
            source: SystemId("captain".into()),
            channel: "public".into(),
            minimum_spacing_ticks: 10,
            routine_text: "We have selected routine alert.".into(),
            red_alert_text: "We have selected red alert.".into(),
        }),
    }
}
fn metadata(tick: u64) -> ObservationMetadata {
    let owner = SystemId("alert-control".into());
    ObservationMetadata::at(
        tick,
        ObservationSubject::System { id: owner.clone() },
        ObservationProvenance {
            kind: ObservationSource::SystemReport,
            system: Some(owner),
            label: String::new(),
        },
        ObservationUncertainty::Unspecified,
    )
}
fn sample(red: bool, tick: u64) -> AlertSample {
    AlertSample::from_publication(
        red,
        &SystemId("alert-control".into()),
        &metadata(tick),
        tick,
    )
    .unwrap()
}
fn state() -> CommsTrafficState {
    CommsTrafficState::new(SystemId("radio-control".into()), &profile(), &channels()).unwrap()
}

#[test]
fn order_changes_only_receiving_intent_and_refused_updates_write_nothing() {
    let config = profile();
    let channels = channels();
    let mut state = state();
    assert_eq!(
        state
            .configured_receive_channels(&config, &channels)
            .unwrap(),
        vec!["public", "fleet"]
    );
    state
        .set_order(
            &config,
            &channels,
            vec!["service".into(), "fleet".into(), "public".into()],
        )
        .unwrap();
    assert_eq!(
        state
            .configured_receive_channels(&config, &channels)
            .unwrap(),
        vec!["service", "fleet"]
    );
    let before = state.clone();
    assert!(state
        .set_order(&config, &channels, vec!["public".into()])
        .is_err());
    assert_eq!(state, before);
    let legacy = CommsTrafficConfig {
        listening: None,
        alert: config.alert.clone(),
    };
    let state =
        CommsTrafficState::new(SystemId("radio-control".into()), &legacy, &channels).unwrap();
    assert_eq!(
        state
            .configured_receive_channels(&legacy, &channels)
            .unwrap(),
        vec!["public", "fleet", "service"]
    );
}

#[test]
fn context_requires_exact_local_source_and_original_current_clock() {
    let owner = SystemId("alert-control".into());
    assert!(AlertSample::from_publication(true, &owner, &metadata(9), 10).is_some());
    assert!(AlertSample::from_publication(true, &owner, &metadata(8), 10).is_none());
    assert!(AlertSample::from_publication(true, &owner, &metadata(11), 10).is_none());
    for fault in 0..6 {
        let mut bad = metadata(10);
        match fault {
            0 => bad.subject = ObservationSubject::Unknown,
            1 => bad.provenance.system = None,
            2 => bad.provenance.kind = ObservationSource::Scan,
            3 => bad.received_tick = Some(10),
            4 => bad.observed_tick = None,
            _ => bad.provenance.label = "foreign claim".into(),
        };
        assert!(AlertSample::from_publication(true, &owner, &bad, 10).is_none());
    }
}

#[test]
fn only_paid_commit_advances_semantic_and_clock_and_unchanged_context_never_repeats() {
    let config = profile();
    let channels = channels();
    let mut state = state();
    let before = state.clone();
    let prepared = state
        .prepare_report(&config, &channels, Some(sample(false, 10)), 10)
        .unwrap()
        .unwrap();
    assert_eq!(prepared.body(), config.alert.as_ref().unwrap().routine_text);
    drop(prepared);
    assert_eq!(state, before, "payment refusal leaves no report frontier");
    state
        .prepare_report(&config, &channels, Some(sample(false, 10)), 10)
        .unwrap()
        .unwrap()
        .commit();
    assert_eq!(
        state.last_report,
        Some(SuccessfulAlertReport {
            semantic: ReportedAlert::Routine,
            emitted_tick: 10
        })
    );
    assert!(
        state
            .prepare_report(&config, &channels, Some(sample(false, 1000)), 1000)
            .unwrap()
            .is_none(),
        "time does not turn unchanged intent into noise"
    );
    assert!(state
        .prepare_report(&config, &channels, Some(sample(true, 19)), 19)
        .unwrap()
        .is_none());
    assert!(state
        .prepare_report(&config, &channels, None, 20)
        .unwrap()
        .is_none());
    assert!(state
        .prepare_report(&config, &channels, Some(sample(true, 18)), 20)
        .unwrap()
        .is_none());
    assert!(
        state
            .prepare_report(&config, &channels, Some(sample(false, 20)), 20)
            .unwrap()
            .is_none(),
        "unsent changes back to reported state need no catchup"
    );
    let prepared = state
        .prepare_report(&config, &channels, Some(sample(true, 21)), 21)
        .unwrap()
        .unwrap();
    assert_eq!(
        prepared.body(),
        config.alert.as_ref().unwrap().red_alert_text
    );
    prepared.commit();
    assert_eq!(state.last_report.unwrap().emitted_tick, 21);
}

#[test]
fn json_preserves_report_frontier_and_bad_candidate_order_clock_refuses() {
    let config = profile();
    let channels = channels();
    let mut live = state();
    live.prepare_report(&config, &channels, Some(sample(true, 10)), 10)
        .unwrap()
        .unwrap()
        .commit();
    let mut resumed: CommsTrafficState =
        serde_json::from_slice(&serde_json::to_vec(&live).unwrap()).unwrap();
    for now in 10..=370 {
        let current = sample(now < 200, now);
        for state in [&mut live, &mut resumed] {
            if let Some(prepared) = state
                .prepare_report(&config, &channels, Some(current), now)
                .unwrap()
            {
                prepared.commit();
            }
        }
        assert_eq!(live, resumed);
    }
    for fault in 0..3 {
        let mut bad = live.clone();
        match fault {
            0 => bad.owner.0.clear(),
            1 => bad.priority.clear(),
            _ => bad.last_report.as_mut().unwrap().emitted_tick = 371,
        };
        assert!(bad.validate(&config, &channels, 370).is_err());
    }
}

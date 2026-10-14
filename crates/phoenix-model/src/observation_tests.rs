use super::*;

#[test]
fn forwarding_preserves_sample_age_and_evidence_limits() {
    let original = ObservationMetadata::at(17, ObservationSubject::Entity { uuid: "contact".into() },
        ObservationProvenance { kind: ObservationSource::Scan, system: Some(SystemId("scan".into())), label: String::new() },
        ObservationUncertainty::Quantized { field: "condition_fraction".into(), step: 0.25 });
    let delivered = original.clone().received(41);
    assert_eq!(delivered.age_ticks(50), Some(33));
    assert_eq!(delivered.observed_tick, original.observed_tick);
    assert_eq!(delivered.uncertainty, original.uncertainty);
    assert_eq!(delivered.provenance, original.provenance);
}

#[test]
fn missing_sample_time_is_unknown_while_tick_zero_is_real() {
    assert_eq!(ObservationMetadata::default().age_ticks(100), None);
    let at_zero = ObservationMetadata::at(0, ObservationSubject::Unknown,
        ObservationProvenance::default(), ObservationUncertainty::Unspecified);
    assert_eq!(at_zero.age_ticks(100), Some(100));
}

#[test]
fn precision_cannot_claim_invalid_quantization() {
    for step in [0.0, -0.1, f32::INFINITY, f32::NAN] {
        let metadata = ObservationMetadata { uncertainty: ObservationUncertainty::Quantized { field: "condition_fraction".into(), step }, ..Default::default() };
        assert!(!metadata.is_valid());
    }
    assert!(ObservationMetadata::default().is_valid());
}

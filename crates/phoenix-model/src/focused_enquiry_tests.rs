use super::*;
#[test]
fn subject_permissions_are_independent_and_absent_permissions_reveal_nothing() {
    let config: EnquiryObservationConfig =
        serde_json::from_str(r#"{"property":[{"property":{"kind":"field_exposure"},"step":0.1}]}"#)
            .unwrap();
    config.validate().unwrap();
    assert!(!config.property[0].normal && !config.property[0].intrusive);
    let mut duplicate = config.clone();
    duplicate.property.extend(config.property.clone());
    assert!(duplicate.validate().is_err());
    let encoded = serde_json::to_string(&config).unwrap();
    assert_eq!(
        serde_json::from_str::<EnquiryObservationConfig>(&encoded).unwrap(),
        config
    );
}
#[test]
fn acquired_quantities_refuse_forged_shapes_negative_stores_and_nonfinite_precision() {
    let mut quantity = ScanQuantity {
        kind: ScanQuantityKind::Capacity,
        label: "resource.known".into(),
        value: ScanQuantityValue::Count(3),
    };
    quantity.validate().unwrap();
    quantity.value = ScanQuantityValue::Count(-1);
    assert!(quantity.validate().is_err());
    quantity.value = ScanQuantityValue::Flag(true);
    assert!(quantity.validate().is_err());
    quantity.kind = ScanQuantityKind::FieldExposure;
    quantity.value = ScanQuantityValue::Scalar {
        value: 1.0,
        step: f64::INFINITY,
    };
    assert!(quantity.validate().is_err());
    quantity.value = ScanQuantityValue::Scalar {
        value: 1.0,
        step: f64::from_bits(1),
    };
    quantity.validate().unwrap();
    assert!(serde_json::from_str::<ScanQuantity>(r#"{"kind":"capacity","label":"resource.known","value":{"kind":"count","value":3},"secret":"x"}"#).is_err());
}

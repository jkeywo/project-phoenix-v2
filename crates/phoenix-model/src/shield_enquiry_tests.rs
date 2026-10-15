use super::*;
#[test]
fn ordinary_and_special_equipment_have_explicit_bounded_optional_benefits() {
    let ordinary = ShieldFrequencyResponse { mismatch_loss: 0.1 };
    let special = ShieldFrequencyResponse { mismatch_loss: 0.6 };
    assert_eq!(ordinary.multiplier(0.0, 1.0), 0.9);
    assert!(special.multiplier(0.0, 1.0) < 0.41);
    assert_eq!(ordinary.multiplier(1.0, 1.0), 1.0);
    assert_eq!(ShieldFrequencyResponse::default().multiplier(0.0, 1.0), 0.5);
}
#[test]
fn acquired_configuration_does_not_contain_unmeasured_frequency_or_future_schedule() {
    let a = ShieldFinding::acquire(0.701, Default::default(), 0.1).unwrap();
    let b = ShieldFinding::acquire(0.702, Default::default(), 0.1).unwrap();
    assert_eq!(a, b);
    a.validate().unwrap();
    let mut bad = a.clone();
    bad.frequency = 0.8;
    assert!(bad.validate().is_err());
    bad = a;
    bad.mismatch_loss = f32::NAN;
    assert!(bad.validate().is_err());
}
#[test]
fn cycle_uses_the_saved_causal_anchor_and_roundtrips_without_a_second_frequency() {
    let cycle = ShieldFrequencyCycle {
        frequencies: vec![0.2, 0.8],
        period_secs: 3.0,
    };
    assert_eq!(
        cycle
            .frequency_at(17, 46, std::time::Duration::from_millis(100))
            .unwrap(),
        0.2
    );
    assert_eq!(
        cycle
            .frequency_at(17, 47, std::time::Duration::from_millis(100))
            .unwrap(),
        0.8
    );
    let resumed: ShieldFrequencyCycle =
        serde_json::from_str(&serde_json::to_string(&cycle).unwrap()).unwrap();
    for tick in 17..500 {
        assert_eq!(
            cycle.frequency_at(17, tick, std::time::Duration::from_millis(100)),
            resumed.frequency_at(17, tick, std::time::Duration::from_millis(100))
        );
    }
    assert!(cycle
        .frequency_at(18, 17, std::time::Duration::from_millis(100))
        .is_err());
}
#[test]
fn invalid_precision_response_or_cycle_refuses() {
    for step in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-100, 2.0] {
        assert!(ShieldFinding::acquire(0.5, Default::default(), step).is_none());
    }
    for loss in [-0.1, 1.1, f32::NAN] {
        assert!(ShieldFrequencyResponse {
            mismatch_loss: loss
        }
        .validate()
        .is_err());
    }
    assert!(ShieldFrequencyCycle {
        frequencies: vec![0.2, 0.2],
        period_secs: 3.0
    }
    .validate()
    .is_err());
}

#[test]
fn adjacent_large_ticks_never_lose_their_cycle_phase() {
    let cycle = ShieldFrequencyCycle {
        frequencies: vec![0.2, 0.8],
        period_secs: 1.0,
    };
    let dt = std::time::Duration::from_secs(1);
    for tick in [(1u64 << 53), (1u64 << 53) + 1, u64::MAX - 1, u64::MAX] {
        assert_eq!(
            cycle.frequency_at(0, tick, dt).unwrap(),
            cycle.frequencies[(tick % 2) as usize]
        );
        let decoded: ShieldFrequencyCycle =
            serde_json::from_str(&serde_json::to_string(&cycle).unwrap()).unwrap();
        assert_eq!(
            cycle.frequency_at(0, tick, dt),
            decoded.frequency_at(0, tick, dt)
        );
    }
    // The product itself can exceed u128; its phase still remains exact.
    assert!(cycle
        .frequency_at(0, u64::MAX, std::time::Duration::new(u64::MAX, 999_999_999))
        .is_ok());
}

use super::*;
#[path = "power_payment_reference.rs"]
mod reference;
fn bytes(power: &PowerSystem) -> String {
    serde_json::to_string(&(
        power.capture_continuation(),
        &power.work_emissions,
        &power.strike_config,
    ))
    .unwrap()
}
#[test]
fn conditional_continuous_payment_matches_raw_reference_and_preserves_rejected_state() {
    for case in 0..13 {
        for acceptance in 0..3 {
            let mut actual = PowerSystem::default();
            let mut owner = actual.config().consumers[0].system.clone();
            actual.last_work_fraction.insert(owner.clone(), -0.0);
            actual.work_emissions.insert(owner.clone(), -0.0);
            let mut dt = 1.0;
            let mut activity = 1.0;
            match case {
                1 => {
                    actual.network.consumers.get_mut(&owner).unwrap().energy = 0.25;
                }
                2 => {
                    actual.network.consumers.get_mut(&owner).unwrap().energy = -0.0;
                }
                3 => {
                    actual
                        .network
                        .consumers
                        .get_mut(&owner)
                        .unwrap()
                        .thermal_stopped = true;
                }
                4 => dt = -0.0,
                5 => activity = -0.0,
                6 => owner = SystemId("unknown".into()),
                7 => {
                    actual.network.consumers.remove(&owner);
                }
                8 => {
                    actual.network.consumers.get_mut(&owner).unwrap().energy = -1.0;
                }
                9 => dt = f32::NAN,
                10 => {
                    dt = f32::MAX;
                    activity = f32::MAX;
                }
                11 => {
                    let mut hardware = actual.config().clone();
                    hardware.consumers[0].reserve.discharge_heat_per_unit = f32::MAX;
                    actual =
                        PowerSystem::from_energy(hardware, actual.ship_config().clone()).unwrap();
                    dt = 2.0;
                }
                12 => {
                    let mut hardware = actual.config().clone();
                    hardware.consumers[0].continuous_cost_per_sec *= 2.0;
                    actual.reconfigure_energy(hardware).unwrap();
                    let saved = actual.capture_continuation();
                    actual.restore_continuation(&saved).unwrap();
                }
                _ => {}
            }
            let mut expected = actual.clone();
            let before = actual.clone();
            let before_bytes = bytes(&actual);
            let mut actual_calls = 0;
            let result = actual.work_continuous_if(&owner, dt, activity, |grant| {
                actual_calls += 1;
                match acceptance {
                    0 => Ok(Some(grant)),
                    1 => Ok(None),
                    _ => Err("recipient refused".into()),
                }
            });
            let mut candidate = expected.clone();
            let mut expected_calls = 0;
            let oracle = candidate
                .work_continuous_reference(&owner, dt, activity)
                .and_then(|grant| {
                    expected_calls += 1;
                    match acceptance {
                        0 => {
                            expected = candidate;
                            Ok(Some(grant))
                        }
                        1 => Ok(None),
                        _ => Err("recipient refused".into()),
                    }
                });
            assert_eq!(result, oracle, "case {case}, policy {acceptance}");
            assert_eq!(actual_calls, expected_calls);
            assert_eq!(actual, expected);
            assert_eq!(bytes(&actual), bytes(&expected));
            if !matches!(result, Ok(Some(_))) {
                assert_eq!(actual, before);
                assert_eq!(bytes(&actual), before_bytes);
            }
            if (6..=11).contains(&case) {
                assert_eq!(actual_calls, 0);
            }
            // Existing unconditional API must preserve the old raw result and
            // accounting too, independently of the conditional accept policy.
            let mut public = before.clone();
            let mut raw = before;
            assert_eq!(
                public.work_continuous(&owner, dt, activity),
                raw.work_continuous_reference(&owner, dt, activity)
            );
            assert_eq!(public, raw);
            assert_eq!(bytes(&public), bytes(&raw));
        }
    }
}
#[test]
fn repeated_owner_payments_accumulate_and_duplicate_authored_owner_still_refuses() {
    let mut actual = PowerSystem::default();
    let owner = actual.config().consumers[0].system.clone();
    let mut expected = actual.clone();
    let mut direct = actual.clone();
    for _ in 0..4 {
        let grant = expected
            .work_continuous_reference(&owner, 0.25, 1.0)
            .unwrap();
        assert_eq!(
            actual
                .work_continuous_if(&owner, 0.25, 1.0, |grant| Ok(Some(grant)))
                .unwrap(),
            Some(grant)
        );
        assert_eq!(direct.work_continuous(&owner, 0.25, 1.0).unwrap(), grant);
        assert_eq!(direct, expected);
        assert_eq!(bytes(&direct), bytes(&expected));
        assert_eq!(actual, expected);
        assert_eq!(bytes(&actual), bytes(&expected));
    }
    let mut hardware = actual.config().clone();
    hardware.consumers.push(hardware.consumers[0].clone());
    assert!(PowerSystem::from_energy(hardware, actual.ship_config().clone()).is_err());
}

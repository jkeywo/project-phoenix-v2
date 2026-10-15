use super::*;

fn bay() -> BayConfig {
    BayConfig {
        compartment: "probe-bay".into(),
        capacity_space: 4,
        launch_work_secs: 2.0,
        recovery_work_secs: 3.0,
        recovery_range: 8.0,
        maximum_parent_speed: 0.0,
        release_offset: [0.0, 1.0, -3.0],
    }
}
#[test]
fn closed_probe_and_finite_stopped_parent_bay_roundtrip_without_equipment_copies() {
    let policy = bay();
    policy.validate().unwrap();
    let decoded: BayConfig =
        serde_json::from_str(&serde_json::to_string(&policy).unwrap()).unwrap();
    assert_eq!(decoded, policy);
    ProbeProfile {
        space: 1,
        passenger_berths: Vec::new(),
    }
    .validate()
    .unwrap();
    assert!(ProbeProfile {
        space: 0,
        passenger_berths: Vec::new()
    }
    .validate()
    .is_err());
    for fault in 0..7 {
        let mut invalid = policy.clone();
        match fault {
            0 => invalid.capacity_space = 0,
            1 => invalid.compartment.clear(),
            2 => invalid.launch_work_secs = 0.0,
            3 => invalid.recovery_work_secs = f32::NAN,
            4 => invalid.recovery_range = f32::INFINITY,
            5 => invalid.maximum_parent_speed = -1.0,
            _ => invalid.release_offset[1] = f32::INFINITY,
        }
        assert!(invalid.validate().is_err(), "case {fault}");
    }
    assert!(serde_json::from_str::<ProbeProfile>(r#"{"space":1,"battery":100}"#).is_err());
}
#[test]
fn one_location_keeps_custody_distinct_from_job_progress_and_refuses_self_parenting() {
    let location = CraftLocation::Carried {
        parent_uuid: "actual-parent".into(),
        bay: SystemId("probe-bay".into()),
    };
    location.validate("actual-probe").unwrap();
    let decoded: CraftLocation =
        serde_json::from_str(&serde_json::to_string(&location).unwrap()).unwrap();
    assert_eq!(decoded, location);
    assert!(location.validate("actual-parent").is_err());
    assert!(location.validate(&"é".repeat(65)).is_err());
    let initial = InitialCraftCustody {
        parent: "declared-parent".into(),
        bay: SystemId("probe-bay".into()),
    };
    initial.validate().unwrap();
    let mut invalid = initial;
    invalid.bay.0 = "bad\nowner".into();
    assert!(invalid.validate().is_err());
    assert!(
        serde_json::from_str::<CraftLocation>(r#"{"kind":"launching","progress":0.5}"#).is_err()
    );
    let deployed = CraftLocation::Deployed {};
    assert_eq!(
        serde_json::to_string(&deployed).unwrap(),
        r#"{"kind":"deployed"}"#
    );
    assert_eq!(
        serde_json::from_str::<CraftLocation>(r#"{"kind":"deployed"}"#).unwrap(),
        deployed
    );
    assert_eq!(CraftLocation::default(), deployed);
    assert!(
        serde_json::from_str::<CraftLocation>(r#"{"kind":"deployed","parent_uuid":"hidden"}"#)
            .is_err()
    );
}

#[test]
fn berths_are_finite_accepted_names_not_another_personnel_population() {
    let profile = ProbeProfile {
        space: 1,
        passenger_berths: vec!["one".into(), "two".into()],
    };
    profile.validate().unwrap();
    assert_eq!(
        serde_json::from_str::<ProbeProfile>(&serde_json::to_string(&profile).unwrap()).unwrap(),
        profile
    );
    for invalid in [
        vec!["".into()],
        vec!["one".into(), "one".into()],
        vec!["bad\nberth".into()],
    ] {
        assert!(ProbeProfile {
            space: 1,
            passenger_berths: invalid
        }
        .validate()
        .is_err());
    }
    let legacy: ProbeProfile = serde_json::from_str(r#"{"space":1}"#).unwrap();
    assert!(legacy.passenger_berths.is_empty());
}

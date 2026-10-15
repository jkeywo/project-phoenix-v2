use super::*;

#[test]
fn public_drive_reading_is_typed_owner_scoped_and_has_no_private_placement_recipe() {
    let view = TransitDriveView {
        owner: crate::identity::SystemId("actual-drive".into()),
        phase: TransitPhaseView::Preparing,
        preparation_fraction: 0.5,
        fuel_required: 2,
        can_begin: false,
        can_cancel: true,
        waiting: Some(TransitWaitingView::Unfunded),
        request: Some(TransitRequestKey {
            objective_id: "jump".into(),
            revision: 7,
        }),
        course: Some(TransitDestination::Local {
            point: [10.0, 0.0, 20.0],
            course_generation: 3,
        }),
        actual_arrival: None,
        last_step_tick: Some(12),
    };
    view.validate().unwrap();
    let board = crate::messages::SystemBlackboard::Transit(view.clone());
    assert_eq!(
        crate::console::ConsoleObservationKind::of(&board),
        crate::console::ConsoleObservationKind::Transit
    );
    let json = serde_json::to_vec(&board).unwrap();
    assert_eq!(
        serde_json::from_slice::<crate::messages::SystemBlackboard>(&json).unwrap(),
        board
    );
    assert!(!String::from_utf8(json).unwrap().contains("gravity"));
    for fault in 0..5 {
        let mut invalid = view.clone();
        match fault {
            0 => invalid.preparation_fraction = f32::NAN,
            1 => invalid.can_begin = true,
            2 => invalid.actual_arrival = Some([0.0; 3]),
            3 => invalid.course = None,
            _ => invalid.owner.0.clear(),
        }
        assert!(invalid.validate().is_err());
    }
    let manual = crate::messages::SystemControlPayload::BeginTransitToWaypoint { request: None };
    let encoded = serde_json::to_vec(&manual).unwrap();
    assert_eq!(
        serde_json::from_slice::<crate::messages::SystemControlPayload>(&encoded).unwrap(),
        manual
    );
    let mut private = serde_json::to_value(&view).unwrap();
    private["gravity"] = serde_json::json!([]);
    assert!(serde_json::from_value::<TransitDriveView>(private).is_err());
}
fn profile() -> AcceptedLocalJumpProfile {
    AcceptedLocalJumpProfile {
        config: LocalJumpConfig {
            preparation_work_secs: 2.0,
            max_distance: 1000.0,
            scatter_radius: 5.0,
            gravity_response: 1.0,
            world_travel: false,
            fuel: TransitFuel {
                store: "drive-fuel".into(),
                amount: 2,
            },
            reading_ticks: 120,
        },
        preparation_energy_per_sec: 1.0,
        commit_energy: 3.0,
    }
}
#[test]
fn accepted_fuel_profile_course_and_request_round_trip_without_receipt_refresh() {
    let records = (
        profile(),
        TransitDestination::Local {
            point: [10.0, 2.0, -30.0],
            course_generation: 7,
        },
        TransitRequestKey {
            objective_id: "jump-order".into(),
            revision: 17,
        },
    );
    records.0.validate().unwrap();
    records.1.validate().unwrap();
    records.2.validate().unwrap();
    let json = serde_json::to_vec(&records).unwrap();
    let restored: (
        AcceptedLocalJumpProfile,
        TransitDestination,
        TransitRequestKey,
    ) = serde_json::from_slice(&json).unwrap();
    assert_eq!(records, restored);
}
#[test]
fn finite_positive_profile_and_explicit_gravity_refuse_malformed_values() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
        let mut value = profile();
        value.config.preparation_work_secs = invalid;
        assert!(value.validate().is_err());
        let mut value = profile();
        value.config.max_distance = invalid;
        assert!(value.validate().is_err());
        let mut value = profile();
        value.preparation_energy_per_sec = invalid;
        assert!(value.validate().is_err());
        let mut value = profile();
        value.commit_energy = invalid;
        assert!(value.validate().is_err());
        assert!(ArrivalGravityConfig {
            exclusion_radius: invalid
        }
        .validate()
        .is_err());
    }
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
        let mut value = profile();
        value.config.scatter_radius = invalid;
        assert!(value.validate().is_err());
    }
    let mut value = profile();
    value.config.scatter_radius = 0.0;
    value.config.reading_ticks = 0;
    value.validate().unwrap();
    for amount in [0, -1, i64::MIN] {
        let mut value = profile();
        value.config.fuel.amount = amount;
        assert!(value.validate().is_err());
    }
}
#[test]
fn closed_geometry_and_identity_do_not_default_to_a_world_or_zero_point() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(TransitDestination::Local {
            point: [0.0, invalid, 0.0],
            course_generation: 0
        }
        .validate()
        .is_err());
    }
    for invalid in [
        "".to_owned(),
        " ".to_owned(),
        "order\nforged".to_owned(),
        "x".repeat(513),
    ] {
        assert!(TransitRequestKey {
            objective_id: invalid.clone(),
            revision: 0
        }
        .validate()
        .is_err());
        assert!(TransitFuel {
            store: invalid,
            amount: 1
        }
        .validate()
        .is_err());
    }
    assert!(serde_json::from_str::<LocalJumpConfig>(r#"{"preparation_work_secs":1,"max_distance":10,"scatter_radius":0,"fuel":{"store":"fuel","amount":1},"free_fuel":true}"#).is_err());
    assert!(serde_json::from_str::<TransitDestination>(
        r#"{"kind":"sector","sector":"elsewhere"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<ArrivalGravityConfig>(
        r#"{"exclusion_radius":10,"visual_mass":20}"#
    )
    .is_err());
}

#[test]
fn gravity_response_defaults_without_changing_legacy_serialized_scatter_inputs() {
    let legacy = serde_json::to_value(profile()).unwrap();
    assert!(legacy["config"].get("gravity_response").is_none());
    let restored: AcceptedLocalJumpProfile = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(restored.config.gravity_response, 1.0);
    assert_eq!(serde_json::to_value(restored).unwrap(), legacy);
    for response in [0.0, 0.25, 2.0] {
        let mut selected = profile();
        selected.config.gravity_response = response;
        selected.validate().unwrap();
        assert_eq!(
            serde_json::from_slice::<AcceptedLocalJumpProfile>(
                &serde_json::to_vec(&selected).unwrap()
            )
            .unwrap(),
            selected
        );
    }
    for response in [-1.0, f32::NAN, f32::INFINITY] {
        let mut selected = profile();
        selected.config.gravity_response = response;
        assert!(selected.validate().is_err());
    }
}

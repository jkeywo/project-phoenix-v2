use super::*;

fn profile() -> PersonnelConfig {
    PersonnelConfig {
        care: None,
        repair_teams: vec![DutyTeamConfig {
            id: "repair-alpha".into(),
            label: "Alpha".into(),
            team_idx: 0,
            competencies: vec![DutyCompetency::MechanicalRepair],
        }],
        officers: vec![DutyOfficerConfig {
            initial_condition: PersonnelCondition::Fit,
            id: "chief".into(),
            label: "Chief".into(),
            home_compartment: "engineering".into(),
            competencies: vec![DutyCompetency::MechanicalRepair],
            repair_work_multiplier: 1.25,
        }],
    }
}

#[test]
fn authored_roster_is_closed_and_cannot_duplicate_a_finite_team_or_officer() {
    let config = profile();
    config.validate().unwrap();
    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(
        serde_json::from_value::<PersonnelConfig>(value.clone()).unwrap(),
        config
    );
    let mut unknown = value;
    unknown
        .as_object_mut()
        .unwrap()
        .insert("extra_teams".into(), 100.into());
    assert!(serde_json::from_value::<PersonnelConfig>(unknown).is_err());
    let mut duplicate = config.clone();
    let mut other = duplicate.repair_teams[0].clone();
    other.id = "repair-beta".into();
    duplicate.repair_teams.push(other);
    assert!(duplicate.validate().is_err());
    for invalid in [f32::NAN, f32::INFINITY, 0.0, 0.99] {
        let mut candidate = config.clone();
        candidate.officers[0].repair_work_multiplier = invalid;
        assert!(candidate.validate().is_err());
    }
    let mut duplicate = config;
    duplicate.officers.push(duplicate.officers[0].clone());
    assert!(duplicate.validate().is_err());
}

#[test]
fn custody_has_one_closed_physical_location_and_no_independent_movement() {
    let state = DutyOfficerState {
        condition: PersonnelCondition::Fit,
        location: OfficerLocation::WithRepairTeam {
            actor: "ship".into(),
            team_idx: 1,
        },
        changed_tick: 12,
    };
    let mut value = serde_json::to_value(&state).unwrap();
    assert_eq!(
        serde_json::from_value::<DutyOfficerState>(value.clone()).unwrap(),
        state
    );
    value["location"]["elapsed"] = 100.into();
    assert!(serde_json::from_value::<DutyOfficerState>(value).is_err());
    assert!(serde_json::from_str::<OfficerLocation>(
        r#"{"kind":"with_repair_team","team_idx":0,"compartment":"bridge"}"#
    )
    .is_err());
}

#[test]
fn care_is_closed_and_old_conditionless_continuation_is_explicitly_refused() {
    assert!(serde_json::from_value::<DutyOfficerState>(serde_json::json!({"location":{"kind":"at_compartment","compartment":"engineering"},"changed_tick":0})).is_err());
    let mut config = profile();
    config.care = Some(CareConfig {
        system: crate::identity::SystemId("repair".into()),
        minor_work_secs: 3.0,
        severe_work_secs: 6.0,
        requirements: vec![crate::damage::PartRequirement {
            resource: "medication".into(),
            service: "medication".into(),
            amount: 1,
        }],
    });
    config.validate().unwrap();
    let mut value = serde_json::to_value(&config).unwrap();
    value["care"]["minor_work_secs"] = 0.into();
    assert!(serde_json::from_value::<PersonnelConfig>(value.clone())
        .unwrap()
        .validate()
        .is_err());
    value["care"] = false.into();
    assert!(serde_json::from_value::<PersonnelConfig>(value)
        .unwrap()
        .care
        .is_none());
}

#[test]
fn stable_person_ref_and_explicit_actual_holder_refuse_legacy_or_ambiguous_shapes() {
    let own = PersonRef::new("origin", "chief");
    let other = PersonRef::new("another-origin", "chief");
    own.validate().unwrap();
    assert_ne!(
        own, other,
        "local officer IDs do not identify people across ships"
    );
    assert_eq!(
        serde_json::from_str::<PersonRef>(&serde_json::to_string(&own).unwrap()).unwrap(),
        own
    );
    assert!(serde_json::from_value::<PersonRef>(serde_json::json!("chief")).is_err());
    assert!(serde_json::from_value::<PersonRef>(
        serde_json::json!({"origin":"origin","officer":"chief","holder":"other"})
    )
    .is_err());
    for invalid in [
        PersonRef::new("", "chief"),
        PersonRef::new(" origin", "chief"),
        PersonRef::new("origin", "Chief"),
    ] {
        assert!(invalid.validate().is_err());
    }
    for old in [
        serde_json::json!({"kind":"at_compartment","compartment":"engineering"}),
        serde_json::json!({"kind":"with_repair_team","team_idx":0}),
    ] {
        assert!(serde_json::from_value::<OfficerLocation>(old).is_err());
    }
    let held = OfficerLocation::AtCompartment {
        actor: "receiver".into(),
        compartment: "medical".into(),
    };
    assert_eq!(
        serde_json::from_value::<OfficerLocation>(serde_json::to_value(&held).unwrap()).unwrap(),
        held
    );
}
#[test]
fn compound_person_requests_round_trip_and_refuse_duplicate_unsorted_or_old_local_keys() {
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Orders {
        #[serde(with = "order_rows")]
        rows: std::collections::BTreeMap<PersonRef, PersonnelDestinationRequest>,
    }
    let request = PersonnelDestinationRequest {
        destination: PersonnelDestination::Home,
        requested_tick: 12,
    };
    let rows = Orders {
        rows: std::collections::BTreeMap::from([
            (PersonRef::new("a", "chief"), request.clone()),
            (PersonRef::new("b", "chief"), request.clone()),
        ]),
    };
    let value = serde_json::to_value(&rows).unwrap();
    assert_eq!(
        serde_json::from_value::<Orders>(value.clone()).unwrap(),
        rows
    );
    let mut duplicate = value.clone();
    duplicate["rows"][1] = duplicate["rows"][0].clone();
    assert!(serde_json::from_value::<Orders>(duplicate).is_err());
    let mut unsorted = value;
    unsorted["rows"].as_array_mut().unwrap().reverse();
    assert!(serde_json::from_value::<Orders>(unsorted).is_err());
    assert!(serde_json::from_value::<Orders>(
        serde_json::json!({"rows":{"chief":{"destination":{"kind":"home"},"requested_tick":12}}})
    )
    .is_err());
}

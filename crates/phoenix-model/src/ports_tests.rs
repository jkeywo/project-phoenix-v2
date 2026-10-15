use super::*;

#[test]
fn optional_actual_producer_is_closed_and_legacy_omission_stays_omitted() {
    let mut profile = service();
    assert!(!serde_json::to_value(&profile)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("system"));
    profile.system = Some(crate::identity::SystemId("dock".into()));
    let value = serde_json::to_value(&profile).unwrap();
    assert_eq!(value["system"], "dock");
    assert_eq!(
        serde_json::from_value::<PortServiceConfig>(value).unwrap(),
        profile
    );
    profile.system = Some(crate::identity::SystemId("".into()));
    assert!(profile.validate().is_err());
}

fn service() -> PortServiceConfig {
    PortServiceConfig {
        system: None,
        policy: PortServicePolicy::ManualOffer,
        eligible_factions: vec![Uuid::from_u128(1)],
        offers: vec![PortOfferConfig {
            id: "parts".into(),
            resource: "machinery_parts".into(),
            quota: 20,
            unit_amount: 5,
            costs: vec![PortCost {
                resource: "alloy".into(),
                amount: 1,
            }],
        }],
    }
}

#[test]
fn explicit_lot_terms_are_closed_and_round_trip_without_a_stock_counter() {
    let value = service();
    value.validate().unwrap();
    let encoded = serde_json::to_value(&value).unwrap();
    assert_eq!(encoded["offer"][0]["quota"], 20);
    assert_eq!(encoded["offer"][0]["cost"][0]["amount"], 1);
    assert!(encoded.get("stock").is_none());
    assert_eq!(
        serde_json::from_value::<PortServiceConfig>(encoded.clone()).unwrap(),
        value
    );
    let mut extra = encoded;
    extra["free_money"] = true.into();
    assert!(serde_json::from_value::<PortServiceConfig>(extra).is_err());
}

#[test]
fn free_permission_cannot_spend_and_empty_membership_remains_explicit_deny_all() {
    let mut value = service();
    value.policy = PortServicePolicy::FreeAlliedReplenishment;
    assert!(value.validate().is_err());
    value.offers[0].costs.clear();
    value.eligible_factions.clear();
    value.validate().unwrap();
    assert!(value.eligible_factions.is_empty());
    value.offers[0].quota = 0;
    value.validate().unwrap();
}

#[test]
fn invalid_or_overflowing_lot_terms_and_duplicate_resources_are_rejected() {
    for mode in 0..10 {
        let mut value = service();
        match mode {
            0 => value.offers[0].quota = -1,
            1 => value.offers[0].unit_amount = 0,
            2 => value.offers[0].quota = i64::MAX,
            3 => value.offers[0].costs[0].amount = 0,
            4 => value.offers[0].costs[0].amount = i64::MAX,
            5 => value.offers[0].resource.clear(),
            6 => value.offers.push(value.offers[0].clone()),
            7 => value.eligible_factions.push(value.eligible_factions[0]),
            8 => {
                let duplicate = value.offers[0].costs[0].clone();
                value.offers[0].costs.push(duplicate);
            }
            _ => value.offers.clear(),
        }
        assert!(value.validate().is_err(), "mode {mode}");
    }
}

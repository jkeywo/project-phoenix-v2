use super::*;

fn cfg() -> TransporterConfig {
    TransporterConfig {
        range: 600.0,
        seconds_per_civilian: 2.0,
        min_power_level: 2,
    }
}

fn inputs() -> TransportInputs<'static> {
    TransportInputs {
        selected: Some("derelict"),
        discovered: true,
        remaining: 3,
        separation: Some(300.0),
        range: 600.0,
        power_level: 3,
        min_power_level: 2,
        disabled: false,
    }
}

#[test]
fn a_well_formed_config_validates() {
    assert!(cfg().validate().is_ok());
}

#[test]
fn a_zero_range_or_duration_or_power_is_rejected() {
    let base = cfg();
    assert!(TransporterConfig {
        range: 0.0,
        ..base.clone()
    }
    .validate()
    .is_err());
    assert!(TransporterConfig {
        seconds_per_civilian: 0.0,
        ..base.clone()
    }
    .validate()
    .is_err());
    assert!(TransporterConfig {
        seconds_per_civilian: -1.0,
        ..base.clone()
    }
    .validate()
    .is_err());
    assert!(TransporterConfig {
        min_power_level: 0,
        ..base
    }
    .validate()
    .is_err());
}

#[test]
fn a_discovered_powered_in_range_contact_with_civilians_transports() {
    assert_eq!(transport_status(&inputs()), Ok(()));
    // Exactly at the range boundary still runs.
    assert_eq!(
        transport_status(&TransportInputs {
            separation: Some(600.0),
            ..inputs()
        }),
        Ok(())
    );
}

#[test]
fn nothing_selected_refuses_no_contact() {
    assert_eq!(
        transport_status(&TransportInputs {
            selected: None,
            separation: None,
            ..inputs()
        }),
        Err(TransportRefusal::NoContact)
    );
}

#[test]
fn an_unscanned_contact_refuses_not_discovered() {
    // The whole "announce only after the scan" gate: a selected contact that
    // has not been revealed cannot be transported.
    assert_eq!(
        transport_status(&TransportInputs {
            discovered: false,
            ..inputs()
        }),
        Err(TransportRefusal::NotDiscovered)
    );
}

#[test]
fn a_contact_past_the_authored_range_refuses_out_of_range() {
    assert_eq!(
        transport_status(&TransportInputs {
            separation: Some(600.1),
            ..inputs()
        }),
        Err(TransportRefusal::OutOfRange)
    );
    // A present selection whose entity cannot be found (no separation) is
    // also "nothing in range".
    assert_eq!(
        transport_status(&TransportInputs {
            separation: None,
            ..inputs()
        }),
        Err(TransportRefusal::OutOfRange)
    );
}

#[test]
fn a_fully_recovered_contact_refuses_no_civilians() {
    assert_eq!(
        transport_status(&TransportInputs {
            remaining: 0,
            ..inputs()
        }),
        Err(TransportRefusal::NoCivilians)
    );
}

#[test]
fn power_below_the_minimum_refuses_unpowered_before_acquisition() {
    // Even with no contact, the more-actionable power refusal wins.
    assert_eq!(
        transport_status(&TransportInputs {
            selected: None,
            separation: None,
            power_level: 1,
            ..inputs()
        }),
        Err(TransportRefusal::Unpowered)
    );
}

#[test]
fn a_disabled_transporter_refuses_first_of_all() {
    assert_eq!(
        transport_status(&TransportInputs {
            disabled: true,
            power_level: 0,
            selected: None,
            separation: None,
            ..inputs()
        }),
        Err(TransportRefusal::Disabled)
    );
}

#[test]
fn every_refusal_has_its_own_string_id() {
    let mut ids: Vec<&str> = TransportRefusal::ALL
        .iter()
        .map(|r| r.string_id())
        .collect();
    assert_eq!(ids.len(), 6);
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 6, "the six ids are distinct");
    for refusal in TransportRefusal::ALL {
        assert!(refusal.string_id().starts_with("transporter.refused."));
    }
}

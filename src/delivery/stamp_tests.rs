use super::*;

const BASE: &str = "[content]\nid = \"phoenix-base\"\nepoch = 1\n";

fn host() -> DeliveryStamp {
    DeliveryStamp::for_manifest(BASE)
}

#[test]
fn fleet_field_round_trips_through_the_shared_native_and_browser_checker() {
    let stamp = host();
    assert_eq!(
        crate::delivery::parse_stamp_field(&stamp.to_field()),
        Some(stamp.clone())
    );
    assert!(crate::delivery::check_host_stamp(&stamp, Some(&stamp.to_field())).is_ok());
    let missing = DeliveryStamp::for_manifest("");
    assert!(crate::delivery::check_host_stamp(&missing, Some(&missing.to_field())).is_err());
}

#[test]
fn a_host_stamp_pairs_the_compiled_protocol_with_the_served_content() {
    let s = host();
    assert_eq!(s.protocol, PROTOCOL_VERSION);
    assert_eq!(s.content_id, "phoenix-base");
    assert_eq!(s.content_epoch, 1);
}

#[test]
fn a_manifest_without_a_content_block_stamps_an_identity_nothing_can_match() {
    let s = DeliveryStamp::for_manifest("[[scenario]]\nid = \"x\"\nworld = \"w.toml\"\n");
    assert_eq!(s.content_id, "");
    assert_eq!(s.content_epoch, 0);
    // And that identity really does refuse a real client.
    let client = DeliveryStamp {
        protocol: PROTOCOL_VERSION,
        content_id: "phoenix-base".into(),
        content_epoch: 1,
    };
    assert_eq!(
        check_client_stamp(&s, Some(&client)).unwrap_err().code(),
        "content-id-mismatch"
    );
}

#[test]
fn a_matching_client_is_admitted() {
    let client = host();
    assert!(check_client_stamp(&host(), Some(&client)).is_ok());
}

#[test]
fn an_unstamped_client_is_refused_rather_than_assumed_compatible() {
    assert_eq!(
        check_client_stamp(&host(), None).unwrap_err().code(),
        "client-stamp-missing"
    );
}

#[test]
fn a_protocol_difference_is_reported_before_a_content_difference() {
    let client = DeliveryStamp {
        protocol: PROTOCOL_VERSION + 1,
        content_id: "something-else".into(),
        content_epoch: 99,
    };
    let err = check_client_stamp(&host(), Some(&client)).unwrap_err();
    assert_eq!(err.code(), "protocol-mismatch");
    assert!(err.detail().contains(&PROTOCOL_VERSION.to_string()));
    assert!(err.detail().contains(&(PROTOCOL_VERSION + 1).to_string()));
}

#[test]
fn a_content_epoch_bump_is_reported_with_both_epochs() {
    let client = DeliveryStamp {
        protocol: PROTOCOL_VERSION,
        content_id: "phoenix-base".into(),
        content_epoch: 2,
    };
    let err = check_client_stamp(&host(), Some(&client)).unwrap_err();
    assert_eq!(err.code(), "content-epoch-mismatch");
    assert!(err.detail().contains('1'));
    assert!(err.detail().contains('2'));
}

#[test]
fn a_bundle_serving_the_same_content_passes_the_startup_pin() {
    assert!(check_bundle_content(&host(), Some(BASE), "dist/assets/scenarios.toml").is_ok());
}

#[test]
fn a_bundle_built_for_other_content_fails_the_startup_pin() {
    let other = "[content]\nid = \"other-game\"\nepoch = 1\n";
    let err = check_bundle_content(&host(), Some(other), "dist/assets/scenarios.toml").unwrap_err();
    assert_eq!(err.code(), "content-id-mismatch");
}

#[test]
fn a_bundle_with_no_manifest_names_the_path_it_looked_at() {
    let err = check_bundle_content(&host(), None, "dist/assets/scenarios.toml").unwrap_err();
    assert_eq!(err.code(), "bundle-content-missing");
    assert!(err.detail().contains("dist/assets/scenarios.toml"));
}

#[test]
fn a_bundle_manifest_with_no_content_block_is_missing_not_empty() {
    let err = check_bundle_content(
        &host(),
        Some("[[scenario]]\nid = \"x\"\nworld = \"w.toml\"\n"),
        "dist/assets/scenarios.toml",
    )
    .unwrap_err();
    assert_eq!(err.code(), "bundle-content-missing");
}

#[test]
fn a_stamp_parses_from_all_three_params_and_from_nothing_less() {
    assert_eq!(
        DeliveryStamp::from_params(Some("1"), Some("phoenix-base"), Some("1")),
        Some(DeliveryStamp {
            protocol: 1,
            content_id: "phoenix-base".into(),
            content_epoch: 1,
        })
    );
    assert_eq!(
        DeliveryStamp::from_params(None, Some("phoenix-base"), Some("1")),
        None
    );
    assert_eq!(
        DeliveryStamp::from_params(Some("not-a-number"), Some("phoenix-base"), Some("1")),
        None
    );
}

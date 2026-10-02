use super::*;

fn decode(crew: serde_json::Value) -> Option<super::super::FleetRoster> {
    let raw = serde_json::json!({
        "local": 1, "owner": 1, "participants": [1],
        "ships": [{"host": 1, "crew": crew}]
    })
    .to_string();
    crate::core::codec::decode_fleet_roster(&raw).map(|(roster, _)| roster)
}

#[test]
fn fleet_crew_ingress_refuses_malformed_or_duplicate_seats_and_orders_exact_pairs() {
    use serde_json::json;
    for invalid in [
        json!(null),
        json!({}),
        json!([["helm"]]),
        json!([["helm", "Std", "private-token"]]),
        json!([["", "Std"]]),
        json!([["helm", ""]]),
        json!([["helm", "Std"], ["helm", "Manual"]]),
        json!([["helm", "Bad\nRating"]]),
        json!([["helm", "é".repeat(MAX_FLEET_CREW_FIELD_BYTES)]]),
        json!((0..=MAX_FLEET_CREW_SEATS)
            .map(|i| (format!("s{i}"), "Std"))
            .collect::<Vec<_>>()),
    ] {
        assert!(decode(invalid.clone()).is_none(), "accepted {invalid}");
    }
    let roster = decode(json!([
        ["\u{10000}", "Manual"],
        ["helm", "Assisted"],
        ["\u{e000}", "Std"]
    ]))
    .unwrap();
    assert_eq!(
        roster.ships()[0]
            .crew
            .iter()
            .map(|(id, _)| id.0.as_str())
            .collect::<Vec<_>>(),
        ["helm", "\u{e000}", "\u{10000}"]
    );
}

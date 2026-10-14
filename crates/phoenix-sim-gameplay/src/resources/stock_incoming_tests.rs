use super::*;

#[test]
fn incoming_is_room_not_quantity_and_arrival_consumes_only_its_own_claim() {
    let mut stock = Stock::new(2, 6).unwrap();
    stock.reserve("tube", 1).unwrap();
    stock.reserve_incoming("delivery", 3).unwrap();
    assert_eq!(stock.available(), 1);
    assert_eq!(stock.headroom(), 1);
    assert_eq!(stock.adjust(100), 1);
    let before = stock.clone();
    assert!(stock.receive("delivery", 2).is_err());
    assert_eq!(stock, before);
    stock.receive("delivery", 3).unwrap();
    assert_eq!(stock.level, 6);
    assert_eq!(stock.reserved["tube"], 1);
    assert!(stock.incoming.is_empty());
    assert!(stock.receive("delivery", 3).is_err());
}
#[test]
fn incoming_claims_reject_overbooking_and_continue_through_json() {
    let mut stock = Stock::new(2, 6).unwrap();
    stock.reserve_incoming("first", 3).unwrap();
    stock.reserve_incoming("first", 3).unwrap();
    assert!(stock.reserve_incoming("second", 2).is_err());
    let decoded: Stock = serde_json::from_slice(&serde_json::to_vec(&stock).unwrap()).unwrap();
    assert_eq!(stock, decoded);
    for text in [
        r#"{"level":2,"ceiling":6,"incoming":{"d":5}}"#,
        r#"{"level":2,"ceiling":6,"incoming":{"":1}}"#,
        r#"{"level":2,"ceiling":6,"incoming":{"d":-1}}"#,
    ] {
        assert!(serde_json::from_str::<Stock>(text).is_err());
    }
}

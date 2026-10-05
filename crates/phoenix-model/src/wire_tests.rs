use super::*;

#[test]
fn readiness_rejects_invalid_operands_before_aggregation() {
    let invalid = ReadinessTally {
        connected: 1,
        ready: 2,
    };
    let other = ReadinessTally::try_new(1, 0).unwrap();
    assert_eq!(
        invalid.checked_add(other),
        Err(ReadinessTallyError::ReadyExceedsConnected)
    );
    assert_eq!(
        other.checked_add(invalid),
        Err(ReadinessTallyError::ReadyExceedsConnected)
    );
    assert!(!ReadinessTally::default().all_ready());
    assert_eq!(
        ReadinessTally::try_new(u32::MAX, 0)
            .unwrap()
            .checked_add(other),
        Err(ReadinessTallyError::TotalOverflow)
    );
}

#[test]
fn readiness_decode_uses_the_same_invariant() {
    use serde::de::value::{Error, MapDeserializer};
    for (connected, ready) in [(0, 0), (3, 2), (3, 3), (1, 2)] {
        let decoder = MapDeserializer::<_, Error>::new(
            [("connected", connected), ("ready", ready)].into_iter(),
        );
        assert_eq!(
            ReadinessTally::deserialize(decoder).is_ok(),
            ready <= connected
        );
    }
}

use super::*;

#[test]
fn clearing_and_ai_retirement_only_change_the_issuing_ships_history() {
    let mut history = HailHistory::default();
    history.record("a", "station");
    history.record("b", "station");
    history.record("b", "other");
    history.clear_source("a");
    assert!(!history.contains("a", "station"));
    assert!(history.contains("b", "station"));
    history.retain_source("a", |_| false);
    assert_eq!(history.len(), 2);
    history.retain_source("b", |target| target == "station");
    assert_eq!(
        history.entries().collect::<Vec<_>>(),
        vec![("b", "station")]
    );
}

#[test]
fn actor_removal_retires_its_issued_and_received_hails_without_erasing_other_ships() {
    let mut history = HailHistory::default();
    history.record("a", "b");
    history.record("b", "station");
    history.record("c", "station");
    history.remove_actor("b");
    assert_eq!(
        history.entries().collect::<Vec<_>>(),
        vec![("c", "station")]
    );
    assert!(history.validate().is_ok());
}

#[test]
fn serialization_retains_ship_scope_and_stable_order() {
    let mut first = HailHistory::default();
    let mut second = HailHistory::default();
    for (source, target) in [("b", "z"), ("a", "y"), ("a", "x")] {
        first.record(source, target);
    }
    for (source, target) in first.entries().collect::<Vec<_>>().into_iter().rev() {
        second.record(source, target);
    }
    let json = serde_json::to_string(&first).unwrap();
    assert_eq!(json, serde_json::to_string(&second).unwrap());
    assert_eq!(first, serde_json::from_str(&json).unwrap());
    assert!(serde_json::from_str::<HailHistory>(r#"{"": ["target"]}"#)
        .unwrap()
        .validate()
        .is_err());
}

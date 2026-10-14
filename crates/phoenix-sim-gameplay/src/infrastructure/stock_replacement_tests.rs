use super::*;
#[test]
fn stock_replacement_is_atomic_and_preserves_threshold_publication() {
    let config:InfrastructureConfig=toml::from_str("[[capacity]]\nid='ammo'\namount=10\nceiling=10\n[[threshold]]\nflag='supplied'\ncapacity='ammo'\nfails_below=0.5\nrestores_above=0.6\n").unwrap();
    let mut state = InfrastructureState::from_config(&config);
    let before = state.clone();
    assert!(state
        .replace_capacity_stock("missing", crate::resources::Stock::new(1, 10).unwrap())
        .is_err());
    assert!(state
        .replace_capacity_stock("ammo", crate::resources::Stock::new(1, 11).unwrap())
        .is_err());
    assert_eq!(state, before);
    let mut replacement = crate::resources::Stock::new(2, 10).unwrap();
    replacement.reserve_incoming("delivery", 4).unwrap();
    assert!(!state
        .replace_capacity_stock("ammo", replacement)
        .unwrap()
        .is_empty());
    assert_eq!(state.capacity("ammo"), Some(2));
    assert_eq!(
        state.adjust_capacity("ammo", 100).unwrap().0,
        6,
        "legacy positive adjustments preserve receiving room"
    );
}

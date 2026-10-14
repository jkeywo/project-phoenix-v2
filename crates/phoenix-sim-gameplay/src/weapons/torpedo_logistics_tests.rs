use super::*;
#[test]
fn initial_magazine_quantity_is_distinct_from_ceiling_and_loading_occupancy() {
    let config: crate::entities::config::TorpedoesConfig =
        toml::from_str("count=6\ninitial_count=2").unwrap();
    config.validate_initial_stock().unwrap();
    let mut system = TorpedoSystem::new(config.to_runtime());
    assert_eq!(system.magazine.level, 2);
    assert_eq!(system.magazine.ceiling, 6);
    system.magazine.reserve_incoming("delivery", 3).unwrap();
    assert_eq!(system.replenish(100), 1);
    assert_eq!(system.magazine.level, 3);
    system.validate_magazine().unwrap();
    system.tubes[0].loaded_count = 1;
    assert!(
        system.validate_magazine().is_err(),
        "held rounds and incoming cannot share receiving space"
    );
    let invalid: crate::entities::config::TorpedoesConfig =
        toml::from_str("count=6\ninitial_count=7").unwrap();
    assert!(invalid.validate_initial_stock().is_err());
    let legacy: crate::entities::config::TorpedoesConfig = toml::from_str("count=6").unwrap();
    assert_eq!(TorpedoSystem::new(legacy.to_runtime()).magazine.level, 6);
}

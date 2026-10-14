use super::*;
#[test]
fn external_stage_release_never_releases_neighbouring_stage_or_consumed_stock() {
    let config: InfrastructureConfig =
        toml::from_str("[[capacity]]\nid='parts'\namount=10\nceiling=10\n").unwrap();
    let mut state = InfrastructureState::from_config(&config);
    for claim in [
        "external-repair/1/2/0",
        "external-repair/1/20/0",
        "repair/1/2/0",
    ] {
        state.capacities[0].stock.reserve(claim, 2).unwrap();
    }
    state.release_repair_claim("external-repair/1/2");
    assert!(!state.capacities[0]
        .stock
        .reserved
        .contains_key("external-repair/1/2/0"));
    assert_eq!(state.capacities[0].stock.reserved.len(), 2);
    assert_eq!(state.capacities[0].stock.level, 10);
}

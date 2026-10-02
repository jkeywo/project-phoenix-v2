use super::*;

#[test]
fn selection_is_reversible_until_commit() {
    let mut state = NativeSessionRoleState::default();
    assert!(state.request(NativeSessionRole::StandaloneGameMaster));
    assert!(state.back());
    assert_eq!(state.role(), NativeSessionRole::Undecided);
    assert!(state.request(NativeSessionRole::ShipHost));
    state.commit();
    assert!(!state.request(NativeSessionRole::FleetGameMaster));
    assert!(!state.back());
    assert_eq!(state.role(), NativeSessionRole::ShipHost);
}

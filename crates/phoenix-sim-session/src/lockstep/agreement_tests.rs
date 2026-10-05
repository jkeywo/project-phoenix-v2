use super::*;
#[test]
fn both_arrival_orders_discover_identical_disagreement_once() {
    let peer = HostSlot(2);
    let mut local_first = MeshAgreement::new(300);
    assert!(local_first.record_local(300, 11).is_empty());
    let found = local_first.record_peer(peer, 300, 22).unwrap();
    assert!(local_first.record_peer(peer, 300, 22).is_none());
    let mut peer_first = MeshAgreement::new(300);
    assert!(peer_first.record_peer(peer, 300, 22).is_none());
    assert_eq!(peer_first.record_local(300, 11), vec![found]);
    assert!(peer_first.record_local(300, 11).is_empty());
    peer_first.forget_through(300);
    assert!(peer_first.agreed());
    assert!(peer_first.record_local(300, 11).is_empty());
}

use super::*;
use crate::ship_slots::{FrozenShipSlots, LaunchSource, LaunchedSlot};

#[test]
fn selected_slots_become_immutable_only_at_the_shared_lobby_exit() {
    let expected = FrozenShipSlots(vec![LaunchedSlot {
        slot_id: "lead".into(),
        hull: "cruiser.toml".into(),
        claimant: Some("host-a".into()),
        source: LaunchSource::Claimed,
    }]);
    let mut world = World::new();
    world.insert_resource(PendingSlotFreeze(expected.clone()));

    assert!(
        !world.contains_resource::<FrozenShipSlots>(),
        "loading a world is not the roster-freeze boundary"
    );
    freeze_selected_ship_slots(&mut world);
    assert_eq!(world.resource::<FrozenShipSlots>(), &expected);
    assert!(
        !world.contains_resource::<PendingSlotFreeze>(),
        "the launch boundary consumes the mutable pre-start staging record"
    );
}

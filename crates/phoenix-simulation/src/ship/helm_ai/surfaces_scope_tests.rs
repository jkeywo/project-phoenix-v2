use super::*;

#[test]
fn viewscreen_identity_alone_does_not_reveal_world_contact_details() {
    use crate::core::messages::{HelmBlackboard, SystemBlackboard, ViewscreenBlackboard};
    let target = uuid::Uuid::from_u128(1);
    let other = uuid::Uuid::from_u128(2);
    let entities = vec![
        crate::ai::AiWorldEntity {
            uuid: target,
            position: [50.0, 0.0, 30.0],
            ..Default::default()
        },
        crate::ai::AiWorldEntity {
            uuid: other,
            position: [80.0, 0.0, 30.0],
            ..Default::default()
        },
    ];
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            combat_lock: Some(target.to_string()),
            ..Default::default()
        }),
    );
    let (hidden, identities) =
        helm_shared_target_view(Default::default(), &entities, &boards, None);
    assert_eq!(identities, vec![target]);
    assert!(
        hidden.entities.is_empty(),
        "designation must not restore a filtered world contact"
    );
    boards.0.insert(
        crate::core::messages::SystemId("helm".into()),
        SystemBlackboard::Helm(HelmBlackboard::default()),
    );
    let (known, _) = helm_shared_target_view(Default::default(), &entities, &boards, None);
    assert_eq!(
        known.entities.len(),
        1,
        "shared targeting must not grant unrelated contacts"
    );
    assert_eq!(known.entities[0].uuid, target);
    assert_eq!(known.entities[0].position, [50.0, 0.0, 30.0]);
}

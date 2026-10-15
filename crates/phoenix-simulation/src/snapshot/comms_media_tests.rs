use super::*;
use crate::comms::media_control::ShipCommsMediaIntent;
fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let (mut world, entity) = crate::science::identity::tests::fixture();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    assert!(world
        .get::<crate::comms::radio::RadioProfile>(entity)
        .is_some());
    world
        .get_mut::<ShipCommsMediaIntent>(entity)
        .unwrap()
        .origin_id = Some("old-selected-call".into());
    let saved = capture(&world);
    (world, entity, saved)
}
#[test]
fn required_radio_owner_and_malformed_single_row_are_rejected_without_writes() {
    let (mut world, entity, saved) = fixture();
    let row = saved
        .entities
        .iter()
        .find(|row| row.uuid == "identity-source")
        .unwrap();
    validate_entity(&world, entity, row).unwrap();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..3 {
        let mut bad = row.clone();
        match fault {
            0 => bad.comms_media = None,
            1 => bad.comms_media.as_mut().unwrap().origin_id = Some("x".repeat(129)),
            _ => bad.comms_media.as_mut().unwrap().origin_id = Some("bad\ncall".into()),
        }
        assert!(validate_entity(&world, entity, &bad).is_err());
        super::super::application::apply_entity_state(&mut world, entity, &bad, saved.tick + 1);
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
        let mut candidate = saved.clone();
        let candidate_row = candidate
            .entities
            .iter_mut()
            .find(|row| row.uuid == bad.uuid)
            .unwrap();
        *candidate_row = bad;
        assert!(validate_roster(&world, &candidate).is_err());
        assert!(!restore(&mut world, &candidate).is_complete());
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}
#[test]
fn missing_actor_recipe_keeps_required_intent_and_does_not_require_a_current_call() {
    let (mut world, entity, mut saved) = fixture();
    let row = saved
        .entities
        .iter_mut()
        .find(|row| row.uuid == "identity-source")
        .unwrap();
    row.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
        template_path: "assets/entities/identity_cruiser.toml".into(),
        name: "identity-source".into(),
        ..Default::default()
    });
    let serialized = serde_json::to_vec(&saved).unwrap();
    let decoded: PhoenixSnapshot = serde_json::from_slice(&serialized).unwrap();
    world.despawn(entity);
    validate_roster(&world, &decoded).unwrap();
    saved
        .entities
        .iter_mut()
        .find(|row| row.uuid == "identity-source")
        .unwrap()
        .comms_media = None;
    assert!(validate_roster(&world, &saved).is_err());
}

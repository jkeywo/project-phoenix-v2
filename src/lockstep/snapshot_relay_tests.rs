use super::*;

#[test]
fn reconnect_record_stages_exact_game_start_ids_before_requesting_the_phase() {
    let entity_uuid =
        crate::world_id::WorldId::new(crate::world_id::IdNamespace::Entity, 17, 3).render();
    let mut source = World::new();
    source.insert_resource(crate::lobby::SelectedShipResource(
        "assets/entities/alliance_cruiser.toml".into(),
    ));
    source.insert_resource(crate::lockstep::FleetRoster::default());
    source.insert_resource(crate::server_app::GameStartEntityUuids(vec![
        crate::snapshot::GameStartEntityUuid {
            authored_index: 0,
            entity_uuid: entity_uuid.clone(),
        },
    ]));
    let run = capture_run(&source, "assets/worlds/default.toml");
    let boot = run
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.state.boot_identity.clone())
        .expect("the source carries a complete boot identity");
    let text = crate::snapshot::export_artifact(&run).unwrap();
    let current = crate::snapshot::versions(&crate::content_ledger::frozen_or_live());

    let mut candidate = World::new();
    candidate.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
    candidate.insert_resource(NextState::<crate::core::messages::GamePhase>::default());
    let mut world_config = crate::world::config::WorldConfig::default();
    world_config
        .entities
        .push(crate::world::config::WorldEntity {
            spawn_on: crate::world::config::WorldEntitySpawnOn::GameStart,
            ..Default::default()
        });
    candidate.insert_resource(world_config);

    assert_eq!(
        gate_and_restore_against_with_readiness(&mut candidate, &text, &current, false, true,),
        MeshRestoreOutcome::NotReady,
    );
    assert!(matches!(
        candidate.resource::<NextState<crate::core::messages::GamePhase>>(),
        NextState::Pending(crate::core::messages::GamePhase::InProgress)
    ));
    assert!(!candidate.contains_resource::<crate::server_app::GameStartEntityUuids>());

    let mut expected = World::new();
    crate::server_app::stage_resume_game_start_entity_uuids(&mut expected, &boot);
    assert_eq!(
        candidate.get_resource::<crate::server_app::ResumeGameStartEntityUuids>(),
        expected.get_resource::<crate::server_app::ResumeGameStartEntityUuids>(),
        "the candidate must stage the canonical authored-index/UUID map exactly"
    );
    assert_eq!(
        boot.game_start_entity_uuids,
        vec![crate::snapshot::GameStartEntityUuid {
            authored_index: 0,
            entity_uuid,
        }]
    );
}

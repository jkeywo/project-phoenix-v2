use super::*;

#[test]
fn restore_outcomes_preserve_recovery_values_and_exact_diagnostics() {
    use MeshRestoreOutcome as Outcome;
    use RestoreResolution as Resolution;

    for (outcome, expected) in [
        (None, Resolution::Pending),
        (Some(Outcome::NotReady), Resolution::Pending),
        (
            Some(Outcome::Committed {
                tick: 567,
                digest: 0xfedc_ba98_7654_3210,
            }),
            Resolution::Recovered {
                tick: 567,
                digest: 0xfedc_ba98_7654_3210,
            },
        ),
        (
            Some(Outcome::RefusedGate("build moved: old -> new".into())),
            Resolution::Failed("build moved: old -> new".into()),
        ),
        (
            Some(Outcome::RefusedChunk("chunk checksum mismatch".into())),
            Resolution::Failed("chunk checksum mismatch".into()),
        ),
        (
            Some(Outcome::RefusedIntegrity {
                recorded: 0xab_cdef,
                restored: 0x12ab,
            }),
            Resolution::Failed(
                "the restored world folds to 0x00000000000012ab, not the 0x0000000000abcdef the record recorded"
                    .into(),
            ),
        ),
        (
            Some(Outcome::Incomplete { tick: 42, gaps: 3 }),
            Resolution::Failed("the restore left 3 gap(s) at tick 42".into()),
        ),
        (
            Some(Outcome::RefusedWrongSender {
                armed_from: HostSlot(1),
                from: Some(HostSlot(2)),
            }),
            Resolution::Failed("the record came from Some(HostSlot(2)), not the leader slot-1".into()),
        ),
        (
            Some(Outcome::RefusedWrongSender {
                armed_from: HostSlot(1),
                from: None,
            }),
            Resolution::Failed("the record came from None, not the leader slot-1".into()),
        ),
        (
            Some(Outcome::RefusedUnarmed),
            Resolution::Failed("this host was not armed to restore".into()),
        ),
    ] {
        assert_eq!(classify_restore_outcome(outcome.as_ref()), expected, "{outcome:?}");
    }
}

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

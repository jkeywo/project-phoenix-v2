use super::*;
use bevy::ecs::system::RunSystemOnce;

fn unarm(app: &mut App, entity: Entity) {
    app.world_mut()
        .entity_mut(entity)
        .remove::<PhaserCombatConfigResource>()
        .remove::<PhaserRenderConfig>()
        .remove::<TorpedoSystemResource>()
        .remove::<BlasterSystemResource>();
}

#[test]
fn absent_equipment_publishes_no_mounts_or_ammo_including_reconnect() {
    let mut app = test_app();
    register_reconnect_player(&mut app, "owner", Some("tactical"));
    let player = local_ship_entity(&mut app);
    unarm(&mut app, player);
    assert!(
        app.world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining
            > 0
    );
    let current = compute_current_weapons_update(app.world_mut());
    assert!(current.banks.is_empty());
    assert!(current.tubes.is_empty());
    assert_eq!(current.torpedo_count, 0);
    app.world_mut()
        .run_system_once(publish_weapons_core_blackboard)
        .unwrap();
    app.world_mut()
        .run_system_once(publish_phaser_bank_blackboards)
        .unwrap();
    app.world_mut()
        .run_system_once(publish_torpedo_tube_blackboards)
        .unwrap();
    app.world_mut()
        .run_system_once(publish_torpedo_magazine_blackboard)
        .unwrap();
    let bbs = &app
        .world()
        .get::<crate::server_app::ShipSystemBlackboards>(player)
        .unwrap()
        .0;
    let Some(SystemBlackboard::Weapons(bb)) =
        bbs.get(&crate::ship::system_registry::tactical_station_key())
    else {
        panic!("weapons overview");
    };
    assert!(bb.banks.is_empty() && bb.tubes.is_empty());
    assert_eq!(bb.torpedo_count, 0);
    assert!(!bbs.values().any(|bb| matches!(
        bb,
        SystemBlackboard::PhaserBank(_)
            | SystemBlackboard::TorpedoTube(_)
            | SystemBlackboard::TorpedoMagazine(_)
    )));
    crate::core::broadcast::resync_registered_replication_for_token(app.world_mut(), "owner");
    let entries = app.world_mut().resource_mut::<SimOutbox>().drain();
    let message = entries
        .iter()
        .find(|entry| matches!(entry.message, ServerMessage::WeaponsUpdate { .. }))
        .unwrap();
    assert_weapons_update_matches_live_state(&message.message, &current);
}

#[test]
fn absent_equipment_never_borrows_another_ships_or_global_weapons() {
    let mut app = test_app();
    let player = local_ship_entity(&mut app);
    let armed_torpedoes = app
        .world()
        .get::<TorpedoSystemResource>(player)
        .unwrap()
        .clone();
    let armed = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipSystemControlSources::default(),
            AdmittedCommands::default(),
            armed_torpedoes,
        ))
        .id();
    unarm(&mut app, player);
    let config = crate::entities::config::EntityConfig::from_toml_in_mode(
        "[behaviour]",
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .unwrap();
    let npc = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "unarmed-npc".into(),
        None,
    );
    app.world_mut().flush();
    // Keep the compatibility tubes loaded: a fallback would be able to fire
    // or unload a real round, rather than failing because its magazine is empty.
    {
        let mut global = app.world_mut().resource_mut::<TorpedoSystemResource>();
        for tube in &mut global.0.tubes {
            tube.load_state = crate::weapons::torpedo::TubeLoadState::Loaded;
            tube.loaded_count = 1;
            tube.target_count = 1;
        }
    }
    let global_before = app
        .world()
        .resource::<TorpedoSystemResource>()
        .0
        .torpedoes_remaining;
    let armed_before = app
        .world()
        .get::<TorpedoSystemResource>(armed)
        .unwrap()
        .0
        .torpedoes_remaining;
    for entity in [player, npc] {
        let commands = [
            ("phaser-port", SystemControlPayload::FirePhaser),
            (
                "torpedo-tube-fore-port",
                SystemControlPayload::FireTorpedo { target_uuid: None },
            ),
            ("torpedo-tube-fore-port", SystemControlPayload::LoadTube),
            ("torpedo-tube-fore-port", SystemControlPayload::UnloadTube),
        ]
        .into_iter()
        .map(|(target, payload)| AdmittedCommand {
            target: SystemId(target.into()),
            payload,
            response_token: Some("owner".into()),
            feedback_correlation: Some(ActionCorrelationId::new(target).unwrap()),
        })
        .collect();
        app.world_mut()
            .entity_mut(entity)
            .insert(AdmittedCommands(commands));
    }
    app.world_mut().run_system_once(handle_fire_phaser).unwrap();
    app.world_mut()
        .run_system_once(handle_fire_torpedo)
        .unwrap();
    app.world_mut().run_system_once(handle_load_tube).unwrap();
    app.world_mut().run_system_once(handle_unload_tube).unwrap();
    assert!(app.world().resource::<InterSystemQueue>().0.is_empty());
    let out: Vec<_> = app
        .world_mut()
        .resource_mut::<bevy::ecs::message::Messages<OutboundMessage>>()
        .drain()
        .collect();
    assert_eq!(
        out.iter()
            .filter(|entry| matches!(
                &entry.msg,
                ServerMessage::ActionFeedback {
                    outcome: ActionFeedbackOutcome::Refused,
                    ..
                }
            ))
            .count(),
        4,
        "both ships refuse both fire commands"
    );
    for source in [Some(player), Some(npc), None] {
        app.world_mut()
            .resource_mut::<InterSystemQueue>()
            .0
            .push(InterSystemMsg {
                target: crate::ship::system_registry::torpedo_magazine_system_id(),
                payload: InterSystemPayload::ClaimTorpedoRound {
                    tube: "fore_port".into(),
                },
                source_entity: source,
            });
    }
    app.world_mut()
        .run_system_once(handle_torpedo_magazine_inter_system)
        .unwrap();
    assert_eq!(
        app.world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining,
        global_before
    );
    let global = &app.world().resource::<TorpedoSystemResource>().0;
    assert!(global.in_flight.is_empty());
    for tube in &global.tubes {
        assert_eq!(
            tube.load_state,
            crate::weapons::torpedo::TubeLoadState::Loaded
        );
        assert_eq!(tube.loaded_count, 1);
        assert_eq!(tube.target_count, 1);
    }
    let torpedoes = &app.world().get::<TorpedoSystemResource>(armed).unwrap().0;
    assert_eq!(torpedoes.torpedoes_remaining, armed_before);
    assert!(torpedoes.in_flight.is_empty());
    for entity in [player, npc] {
        assert!(!app.world().get::<ActiveBeam>(entity).unwrap().is_firing());
    }
}

#[test]
fn resource_only_magazine_requires_no_ships_and_an_unaddressed_claim() {
    let mut app = test_app();
    let player = local_ship_entity(&mut app);
    unarm(&mut app, player);
    let before = app
        .world()
        .resource::<TorpedoSystemResource>()
        .0
        .torpedoes_remaining;
    app.world_mut()
        .resource_mut::<InterSystemQueue>()
        .0
        .push(InterSystemMsg {
            target: crate::ship::system_registry::torpedo_magazine_system_id(),
            payload: InterSystemPayload::ClaimTorpedoRound {
                tube: "fore_port".into(),
            },
            source_entity: None,
        });
    app.world_mut()
        .run_system_once(handle_torpedo_magazine_inter_system)
        .unwrap();
    assert_eq!(
        app.world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining,
        before
    );
    app.world_mut().despawn(player);
    app.world_mut().resource_mut::<InterSystemQueue>().0[0].source_entity = Some(player);
    app.world_mut()
        .run_system_once(handle_torpedo_magazine_inter_system)
        .unwrap();
    assert_eq!(
        app.world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining,
        before
    );
    app.world_mut().resource_mut::<InterSystemQueue>().0[0].source_entity = None;
    app.world_mut()
        .run_system_once(handle_torpedo_magazine_inter_system)
        .unwrap();
    assert_eq!(
        app.world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining,
        before - 1
    );
}

#[test]
fn authored_legacy_empty_bank_configuration_still_projects_its_mount() {
    let mut app = test_app();
    let player = local_ship_entity(&mut app);
    app.world_mut()
        .entity_mut(player)
        .insert(PhaserCombatConfigResource::default());
    let current = compute_current_weapons_update(app.world_mut());
    assert_eq!(current.banks.len(), 1);
    assert_eq!(current.banks[0].id, "");
}

#[test]
fn absent_phaser_equipment_cannot_apply_damage_from_leftover_beam_state() {
    let mut app = test_app();
    let player = local_ship_entity(&mut app);
    let target = setup_weapons_world(&mut app, 0.0, -20.0);
    unarm(&mut app, player);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(0.1));
    app.world_mut()
        .get_mut::<ActiveBeam>(player)
        .unwrap()
        .start("port", "target-uuid", 1.0, 100.0);
    app.world_mut().run_system_once(tick_beams_prepare).unwrap();
    app.world_mut()
        .run_system_once(tick_beams_apply_damage)
        .unwrap();
    assert!(app
        .world()
        .resource::<crate::console::weapons::shared::BeamContext>()
        .0
        .is_empty());
    assert_eq!(
        app.world()
            .get::<EntitySystemHull>(target)
            .unwrap()
            .0
            .total_current(),
        30.0
    );
}

#[test]
fn resource_only_lifecycle_requires_no_ship_entities() {
    use crate::weapons::torpedo::TubeLoadState;
    let mut app = test_app();
    let player = local_ship_entity(&mut app);
    unarm(&mut app, player);
    let loading = TubeLoadState::Loading {
        remaining: 2.0,
        total: 2.0,
    };
    app.world_mut()
        .resource_mut::<TorpedoSystemResource>()
        .0
        .tubes[0]
        .load_state = loading.clone();
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(0.25));
    app.world_mut()
        .run_system_once(tick_torpedo_lifecycle)
        .unwrap();
    assert_eq!(
        app.world().resource::<TorpedoSystemResource>().0.tubes[0].load_state,
        loading,
        "an unarmed Ship prevents the compatibility magazine from ticking"
    );
    app.world_mut().despawn(player);
    app.world_mut()
        .run_system_once(tick_torpedo_lifecycle)
        .unwrap();
    assert_eq!(
        app.world().resource::<TorpedoSystemResource>().0.tubes[0].load_state,
        TubeLoadState::Loading {
            remaining: 1.75,
            total: 2.0
        },
        "resource-only harness behavior remains supported"
    );
}

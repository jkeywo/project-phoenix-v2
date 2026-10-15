use super::*;
use crate::console::weapons::{
    guidance_link::{record_emitted, ShipGuidanceKnowledge},
    TorpedoSystemResource,
};
use phoenix_model::guided_orders::KnownGuidedLaunchCommand;
use std::collections::HashMap;

fn fixture() -> (World, Entity, PhoenixSnapshot) {
    let path = "assets/entities/alliance_battleship.toml";
    let config = crate::entities::include_resolve::load_entity_config(path).unwrap();
    let mut world = World::new();
    world.register_component::<EntitySystemHull>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<crate::console::repair::server::RepairRequestQueue>();
    world.register_component::<crate::ship_plugin::RepairHumanAlerted>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.insert_resource(SimTick(100));
    world.insert_resource(Time::<Fixed>::from_hz(60.0));
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "source".into(),
        None,
    );
    world.flush();
    world
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntitySpawnOrigin(
            crate::world::spawn_origin::SpawnOrigin {
                template_path: path.into(),
                name: "source".into(),
                ..Default::default()
            },
        ));
    let round = {
        let mut owner = world.get_mut::<TorpedoSystemResource>(entity).unwrap();
        let tube = owner.0.tubes[0].id.clone();
        owner.0.set_volley_target(&tube, 2);
        assert!(owner.0.start_load(&tube));
        let load_time = owner.0.tube(&tube).unwrap().load_time;
        for _ in 0..2 {
            owner
                .0
                .tick(load_time, &HashMap::new(), &mut || unreachable!());
        }
        assert!(matches!(
            owner.0.launch(
                &tube,
                "first-emitted".into(),
                0.0,
                0.0,
                0.0,
                0.0,
                Some("original-private-intent".into()),
                Some("source".into())
            ),
            crate::weapons::torpedo::LaunchResult::Launched { .. }
        ));
        assert_eq!(owner.0.burst_states.len(), 1);
        owner.0.in_flight[0].clone()
    };
    let tube = crate::ship::system_registry::torpedo_tube_system_id(&round.tube_id).unwrap();
    let mut knowledge = world.get_mut::<ShipGuidanceKnowledge>(entity).unwrap();
    record_emitted(
        &mut knowledge,
        &round,
        Some("public-contact".into()),
        Some(99),
        100,
    );
    assert!(
        phoenix_sim_gameplay::weapons::guided_orders::note_launch_command(
            &mut knowledge.0,
            round.uuid.clone(),
            KnownGuidedLaunchCommand {
                tube,
                issued_tick: 100,
                target: Some("public-contact".into()),
                sampled_tick: Some(99)
            },
            100
        )
    );
    drop(knowledge);
    let saved = capture(&world);
    assert!(saved.entities[0]
        .guided_source
        .as_ref()
        .unwrap()
        .launches
        .contains_key("first-emitted"));
    (world, entity, saved)
}

#[test]
fn actual_spawn_json_apply_preserves_own_source_knowledge_and_exact_pending_launch_binding() {
    let (mut world, entity, saved) = fixture();
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    validate_roster(&world, &saved).unwrap();
    world
        .get_mut::<ShipGuidanceKnowledge>(entity)
        .unwrap()
        .0
        .launches
        .clear();
    super::super::application::apply_entity_state(
        &mut world,
        entity,
        &saved.entities[0],
        saved.tick,
    );
    assert_eq!(
        capture(&world).entities[0].guided_source,
        saved.entities[0].guided_source
    );
    validate_roster(&world, &capture(&world)).unwrap();
    // Physical absence is ordinary unresolved source knowledge, not a bad save.
    world
        .get_mut::<TorpedoSystemResource>(entity)
        .unwrap()
        .0
        .apply_interception("first-emitted", f32::MAX)
        .unwrap();
    let absent = capture(&world);
    validate_roster(&world, &absent).unwrap();
    assert!(absent.entities[0]
        .guided_source
        .as_ref()
        .unwrap()
        .launches
        .contains_key("first-emitted"));
}

#[test]
fn forged_missing_future_foreign_and_pending_launch_claims_refuse_without_any_write() {
    for fault in 0..6 {
        let (mut world, _, mut bad) = fixture();
        let before = capture(&world);
        let digest = crate::sim_digest::world_digest(&world);
        if fault == 0 {
            bad.entities[0].guided_source = None;
        } else {
            let state = bad.entities[0].guided_source.as_mut().unwrap();
            match fault {
                1 => {
                    state.launches.get_mut("first-emitted").unwrap().tube =
                        SystemId("torpedo-tube-aft".into())
                }
                2 => {
                    state
                        .launches
                        .get_mut("first-emitted")
                        .unwrap()
                        .issued
                        .issued_tick = 101
                }
                3 => state.launch_commands.clear(),
                4 => state.launches.get_mut("first-emitted").unwrap().receiver = None,
                _ => {
                    state.launch_commands.get_mut("first-emitted").unwrap().tube =
                        SystemId("torpedo-tube-aft".into())
                }
            }
        }
        assert!(
            validate_roster(&world, &bad).is_err(),
            "forged source frontier{fault}"
        );
        assert!(!super::super::restore(&mut world, &bad).is_complete());
        assert_eq!(
            capture(&world),
            before,
            "rejected source frontier{fault} wrote state"
        );
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}

#[test]
fn authored_tube_identity_budget_matches_closed_source_system_provenance() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut tubes = config.torpedoes.unwrap().tubes;
    tubes[0].id = "a".repeat(115);
    crate::entities::config::validate_torpedo_tubes(&tubes).unwrap();
    assert_eq!(
        crate::ship::system_registry::torpedo_tube_system_id(&tubes[0].id)
            .unwrap()
            .0
            .len(),
        128
    );
    tubes[0].id.push('a');
    assert!(crate::entities::config::validate_torpedo_tubes(&tubes)
        .unwrap_err()
        .contains("115 UTF-8 bytes"));
}

#[test]
fn single_row_source_frontiers_are_checked_before_inverse_writes() {
    for fault in 0..3 {
        let (mut world, entity, mut saved) = fixture();
        match fault {
            0 => saved.entities[0].guided_source = None,
            1 => saved.entities[0]
                .guided_source
                .as_mut()
                .unwrap()
                .launch_commands
                .clear(),
            _ => {
                saved.entities[0]
                    .guided_source
                    .as_mut()
                    .unwrap()
                    .launches
                    .get_mut("first-emitted")
                    .unwrap()
                    .issued
                    .issued_tick = 101
            }
        }
        let before = capture(&world);
        let digest = crate::sim_digest::world_digest(&world);
        assert!(validate_entity(&world, Some(entity), &saved.entities[0], 100).is_err());
        super::super::application::apply_entity_state(&mut world, entity, &saved.entities[0], 100);
        assert_eq!(capture(&world), before);
        assert_eq!(crate::sim_digest::world_digest(&world), digest);
    }
}

#[test]
fn source_knowledge_presence_is_closed_even_with_a_tube_in_the_bare_system_graph() {
    let (mut world, entity, mut saved) = fixture();
    // A bare graph is not an accepted authored source capability. It must not
    // gain an injected ledger merely because one of its System kinds is a tube.
    world.entity_mut(entity).remove::<ShipGuidanceKnowledge>();
    saved.entities[0].spawn = None;
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    assert!(validate_entity(&world, Some(entity), &saved.entities[0], 100).is_err());
    assert!(
        super::super::preflight::validate_entity_resources(&world, entity, &saved.entities[0])
            .is_err()
    );
    super::super::application::apply_entity_state(&mut world, entity, &saved.entities[0], 100);
    assert_eq!(capture(&world), before);
    assert_eq!(crate::sim_digest::world_digest(&world), digest);
    saved.entities[0].guided_source = None;
    validate_entity(&world, Some(entity), &saved.entities[0], 100).unwrap();
}

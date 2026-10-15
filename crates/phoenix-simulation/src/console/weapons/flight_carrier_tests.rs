use super::*;
use crate::entities::spawner::EntityUuid;
use std::collections::HashMap;

fn host() -> App {
    host_with_registration(false)
}

fn assert_same_digest(left: &World, right: &World) {
    let expected = crate::sim_digest::digest_stages(left);
    assert_eq!(
        crate::sim_digest::world_digest(left),
        crate::sim_digest::world_digest(right),
        "first divergent scope: {:?}; left stages: {expected:?}; right stages: {:?}",
        crate::sim_digest::first_divergent_scope(right, &expected),
        crate::sim_digest::digest_stages(right),
    );
}

fn host_with_registration(reverse: bool) -> App {
    let mut app = App::new();
    if reverse {
        app.world_mut()
            .register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
        app.world_mut()
            .register_component::<TorpedoSystemResource>();
    } else {
        app.world_mut()
            .register_component::<TorpedoSystemResource>();
        app.world_mut()
            .register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    }
    register(&mut app);
    let world = app.world_mut();
    world.insert_resource(crate::sim_tick::SimTick(42));
    world.insert_resource(Time::<Fixed>::default());
    // Normal hosts own these run-scope defaults before capture. Restore keeps
    // their presence, which is deliberately represented in the causal digest.
    world.init_resource::<crate::gm::actions::action::SimulationPaused>();
    world.init_resource::<crate::gm::actions::action::GmActionJournal>();
    world.init_resource::<crate::gm::operations::puppet::StationPuppets>();
    world.init_resource::<crate::gm::operations::puppet::PendingGmStationCommands>();
    world.init_resource::<crate::core::report::MissionReport>();
    crate::world_id::install(world, crate::world_id::WorldIdMint::default());
    // Actual capture joins use optional registered owners even for bare hosts.
    world.register_component::<EntityUuid>();
    // The normal host registers the empty spatial namespaces before capture.
    // Restore's derived radar rebuild also registers these query components;
    // both sides must start from that same host roster, even with no asteroids.
    world.register_component::<crate::server_app::AsteroidUuid>();
    world.register_component::<Transform>();
    world.register_component::<crate::ship::state::ShipPhysics>();
    world.register_component::<crate::entities::spawner::EntitySystemHull>();
    world.register_component::<crate::ship::state::ShipRedAlert>();
    world.register_component::<crate::console::command::server::ShipStationStances>();
    world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    world.register_component::<crate::console::weapons::beam::ActiveBeam>();
    world.register_component::<crate::console::weapons::beam::PhaserCooldown>();
    world.register_component::<crate::entities::spawner::EntityShipArcHull>();
    world.register_component::<crate::console::repair::ShipRepairTeams>();
    world.register_component::<crate::console::repair::RepairRequestQueue>();
    world.register_component::<crate::ship::components::RepairHumanAlerted>();
    world.register_component::<crate::server_app::ShipSystemBlackboards>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<crate::ship::shields::ShipShields>();
    app
}

fn launched() -> TorpedoSystemResource {
    launched_id("physical-round")
}

fn launched_id(id: &str) -> TorpedoSystemResource {
    let mut owner = TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig {
        load_time: 0.1,
        seeker: Some(phoenix_model::guided_guidance::SeekerProfile {
            range: 2000.0,
            fire_arc_deg: 360.0,
            minimum_transmission: 0.01,
        }),
        ..Default::default()
    }));
    let tube = owner.0.tubes[0].id.clone();
    assert!(owner.0.start_load(&tube));
    owner.0.tick(0.1, &HashMap::new(), &mut || unreachable!());
    assert!(matches!(
        owner.0.launch(
            &tube,
            id.into(),
            1.0,
            2.0,
            3.0,
            0.0,
            Some("target".into()),
            Some("source".into())
        ),
        crate::weapons::torpedo::LaunchResult::Launched { .. }
    ));
    owner
}

fn carriers(world: &mut World) -> Vec<Entity> {
    let mut query = world.query_filtered::<Entity, With<WeaponFlightCarrier>>();
    query.iter(world).collect()
}

#[test]
fn actual_source_despawn_moves_flights_without_machinery_or_payload_loss() {
    let mut app = host();
    let owner = launched();
    let before = owner.0.in_flight.clone();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), crate::server_app::Ship, owner))
        .id();
    app.world_mut().despawn(source);
    app.world_mut().flush();
    let rows = carriers(app.world_mut());
    assert_eq!(rows.len(), 1);
    let carrier = rows[0];
    let world = app.world();
    let actual = world.get::<TorpedoSystemResource>(carrier).unwrap();
    assert_eq!(actual.0.in_flight, before);
    assert!(actual.0.tubes.is_empty());
    assert!(actual.0.burst_states.is_empty());
    assert_eq!(actual.0.magazine.level, 0);
    assert_eq!(actual.0.magazine.ceiling, 0);
    assert!(world.get::<crate::server_app::Ship>(carrier).is_none());
    assert!(world.get::<Transform>(carrier).is_none());
    assert!(world
        .get::<crate::ship_plugin::ShipConfigComponent>(carrier)
        .is_none());
    assert!(world
        .get::<crate::ship::power::ShipPowerSystem>(carrier)
        .is_none());
    // The old source attribution and launch facts live on the real round.
    assert_eq!(actual.0.in_flight[0].source_uuid.as_deref(), Some("source"));
}

#[test]
fn replacing_actual_launcher_keeps_existing_flights_once_and_cannot_upgrade_their_profile() {
    let mut app = host();
    let original = launched();
    let before = original.0.in_flight.clone();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), original.clone()))
        .id();
    let mut replacement = original;
    replacement.0.config.damage_hull = 100000;
    replacement.0.in_flight[0].profile.damage_hull = 100000;
    app.world_mut().entity_mut(source).insert(replacement);
    app.world_mut().flush();
    assert!(app
        .world()
        .get::<TorpedoSystemResource>(source)
        .unwrap()
        .0
        .in_flight
        .is_empty());
    let rows = carriers(app.world_mut());
    assert_eq!(rows.len(), 1);
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(rows[0])
            .unwrap()
            .0
            .in_flight,
        before
    );
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(source)
            .unwrap()
            .0
            .config
            .damage_hull,
        100000
    );
}

#[test]
fn carrier_json_restore_discards_bootstrap_flights_and_continues_same_vector_exactly() {
    let mut original = host();
    let source = original
        .world_mut()
        .spawn((EntityUuid("source".into()), launched()))
        .id();
    original.world_mut().despawn(source);
    original.world_mut().flush();
    let snapshot = crate::snapshot::capture(original.world());
    assert_eq!(snapshot.entities.len(), 1);
    assert!(snapshot.entities[0].weapon_flight_carrier);
    assert_eq!(
        snapshot.entities[0]
            .weapons
            .as_ref()
            .unwrap()
            .torpedoes_in_flight
            .len(),
        1
    );
    let snapshot = serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let mut resumed = host();
    resumed
        .world_mut()
        .spawn((EntityUuid("bootstrap-not-saved".into()), launched()));
    assert!(!crate::snapshot::ready_to_restore(
        resumed.world(),
        &snapshot
    ));
    assert!(crate::snapshot::ready_to_rebuild(
        resumed.world(),
        &snapshot
    ));
    let mut invalid = snapshot.clone();
    invalid.entities[0].physics = Some(Default::default());
    assert!(!crate::snapshot::ready_to_rebuild(
        resumed.world(),
        &invalid
    ));
    let report = crate::snapshot::restore(resumed.world_mut(), &snapshot);
    assert!(report.is_complete(), "{report:?}");
    resumed.world_mut().flush();
    assert_eq!(
        carriers(resumed.world_mut()).len(),
        1,
        "bootstrap removal must not rehome another flight"
    );
    let left = carriers(original.world_mut())[0];
    let right = carriers(resumed.world_mut())[0];
    let targets = HashMap::from([(
        "physical-round".into(),
        phoenix_model::guided_guidance::GuidanceObservation {
            target_uuid: "target".into(),
            position: [10.0, 8.0, -30.0],
            sampled_tick: 42,
            source: phoenix_model::guided_guidance::GuidanceSource::Seeker,
        },
    )]);
    for step in 0..360 {
        assert_eq!(
            crate::snapshot::capture(original.world()),
            crate::snapshot::capture(resumed.world()),
            "step{step}"
        );
        assert_same_digest(original.world(), resumed.world());
        for (app, entity) in [(&mut original, left), (&mut resumed, right)] {
            app.world_mut()
                .get_mut::<TorpedoSystemResource>(entity)
                .unwrap()
                .0
                .tick_with_work(
                    1.0 / 60.0,
                    &targets,
                    &mut |_| panic!("carrier must never request work"),
                    &mut || panic!("carrier must never mint a shot"),
                );
        }
    }
    // The exclusive overwrite guard must have ended: a subsequent real source
    // removal still preserves newly launched work through the ordinary hook.
    let source = resumed
        .world_mut()
        .spawn((EntityUuid("source".into()), launched_id("later-round")))
        .id();
    resumed.world_mut().despawn(source);
    resumed.world_mut().flush();
    assert_eq!(carriers(resumed.world_mut()).len(), 2);
}

#[test]
fn forged_carrier_machinery_and_single_actor_duplicate_inverses_do_not_write() {
    let mut app = host();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), launched()))
        .id();
    let saved_source = crate::snapshot::capture(app.world())
        .entities
        .into_iter()
        .find(|row| row.uuid == "source")
        .unwrap();
    assert_eq!(
        saved_source
            .weapons
            .as_ref()
            .unwrap()
            .torpedoes_in_flight
            .len(),
        1
    );
    app.world_mut().despawn(source);
    app.world_mut().flush();
    let before = crate::snapshot::capture(app.world());
    for case in 0..4 {
        let mut forged = before.clone();
        match case {
            0 => forged.entities[0].physics = Some([0.0; 8]),
            1 => forged.entities[0]
                .weapons
                .as_mut()
                .unwrap()
                .bursts
                .push(crate::snapshot::BurstState::default()),
            2 => {
                forged.entities[0]
                    .weapons
                    .as_mut()
                    .unwrap()
                    .magazine
                    .as_mut()
                    .unwrap()
                    .level = 1
            }
            3 => forged.entities[0].weapon_flight_carrier = false,
            _ => unreachable!(),
        }
        let report = crate::snapshot::restore(app.world_mut(), &forged);
        assert!(!report.is_complete(), "case{case}: {report:?}");
        assert_eq!(crate::snapshot::capture(app.world()), before);
    }
    let new_body = app
        .world_mut()
        .spawn((EntityUuid("source".into()), empty()))
        .id();
    crate::snapshot::apply_entity_state(app.world_mut(), new_body, &saved_source, 42);
    assert!(app
        .world()
        .get::<TorpedoSystemResource>(new_body)
        .unwrap()
        .0
        .in_flight
        .is_empty());
    assert_eq!(carriers(app.world_mut()).len(), 1);
}

#[test]
fn nonmission_cleanup_does_not_rehome_actual_flights() {
    let mut app = host();
    app.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), launched()))
        .id();
    app.world_mut().despawn(source);
    app.world_mut().flush();
    assert!(carriers(app.world_mut()).is_empty());
}

fn emitted_bolts() -> crate::console::weapons::blaster::BlasterSystemResource {
    let banks = ["port", "starboard"]
        .into_iter()
        .map(|id| {
            let mut bank = BlasterSystem::new(BlasterBankConfig {
                id: id.into(),
                volley_count: 2,
                volley_interval_secs: 10.0,
                projectile_speed: 6.0,
                range: 540.0,
                ..Default::default()
            });
            assert!(bank.request_fire());
            let events = bank.tick(
                0.1,
                &[(1.0, 3.0)],
                0.0,
                10.0,
                -20.0,
                0.0,
                0.0,
                "source",
                &mut || format!("bolt-{id}"),
            );
            assert_eq!(events.len(), 1);
            assert_eq!(
                bank.volley.pending_volley, 1,
                "source has an unfinished volley to discard"
            );
            bank
        })
        .collect();
    crate::console::weapons::blaster::BlasterSystemResource(banks)
}

#[test]
fn actual_bolt_owner_removal_preserves_emitted_rounds_and_discards_pending_fire() {
    use crate::console::weapons::blaster::BlasterSystemResource;
    let mut app = host();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), emitted_bolts()))
        .id();
    let before = crate::snapshot::capture(app.world()).entities[0]
        .weapons
        .as_ref()
        .unwrap()
        .blasters
        .clone();
    app.world_mut().despawn(source);
    app.world_mut().flush();
    let rows = carriers(app.world_mut());
    assert_eq!(rows.len(), 1);
    let saved = crate::snapshot::capture(app.world());
    assert_eq!(
        saved.entities[0].carrier_bolt_banks,
        vec!["port", "starboard"]
    );
    let stored = saved.entities[0].weapons.as_ref().unwrap();
    for (saved, original) in stored.blasters.iter().zip(&before) {
        assert_eq!(saved.in_flight, original.in_flight);
        assert_eq!(saved.pending_volley, 0);
        assert!(saved.schedule.is_empty());
    }
    let owner = app.world().get::<BlasterSystemResource>(rows[0]).unwrap();
    assert!(owner
        .0
        .iter()
        .all(|bank| bank.config.rounds.is_none() && bank.config.firing_schedule().is_empty()));
    assert_eq!(
        app.world()
            .get::<TorpedoSystemResource>(rows[0])
            .unwrap()
            .0
            .magazine
            .ceiling,
        0
    );
}

#[test]
fn replacing_bolt_equipment_cannot_duplicate_or_upgrade_already_emitted_rounds() {
    use crate::console::weapons::blaster::BlasterSystemResource;
    let mut app = host();
    let original = emitted_bolts();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), original.clone()))
        .id();
    let before = crate::snapshot::capture(app.world()).entities[0]
        .weapons
        .as_ref()
        .unwrap()
        .blasters
        .clone();
    let mut replacement = original;
    for bank in &mut replacement.0 {
        bank.config.damage = 90000;
        bank.in_flight[0].damage = 90000;
    }
    app.world_mut().entity_mut(source).insert(replacement);
    app.world_mut().flush();
    assert!(app
        .world()
        .get::<BlasterSystemResource>(source)
        .unwrap()
        .0
        .iter()
        .all(|bank| bank.in_flight.is_empty()));
    let saved = crate::snapshot::capture(app.world());
    let row = saved
        .entities
        .iter()
        .find(|row| row.weapon_flight_carrier)
        .unwrap();
    for (saved, original) in row.weapons.as_ref().unwrap().blasters.iter().zip(&before) {
        assert_eq!(saved.in_flight, original.in_flight);
    }
}

#[test]
fn bolt_carrier_recipe_json_continuation_and_forged_work_are_closed() {
    use crate::console::weapons::blaster::BlasterSystemResource;
    let mut original = host();
    let source = original
        .world_mut()
        .spawn((EntityUuid("source".into()), emitted_bolts()))
        .id();
    original.world_mut().despawn(source);
    original.world_mut().flush();
    let saved = crate::snapshot::capture(original.world());
    assert_eq!(saved.entities[0].carrier_bolt_banks.len(), 2);
    let saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    let mut resumed = host();
    resumed
        .world_mut()
        .spawn((EntityUuid("discarded-bootstrap".into()), emitted_bolts()));
    let report = crate::snapshot::restore(resumed.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    resumed.world_mut().flush();
    assert_eq!(carriers(resumed.world_mut()).len(), 1);
    let carrier = carriers(resumed.world_mut())[0];
    let expected_digest = crate::sim_digest::world_digest(resumed.world());
    resumed
        .world_mut()
        .get_mut::<BlasterSystemResource>(carrier)
        .unwrap()
        .0[0]
        .config
        .id = "other-bank".into();
    assert_ne!(
        crate::sim_digest::world_digest(resumed.world()),
        expected_digest,
        "actual attribution bank IDs are part of continuation"
    );
    resumed
        .world_mut()
        .get_mut::<BlasterSystemResource>(carrier)
        .unwrap()
        .0[0]
        .config
        .id = "port".into();
    let before = crate::snapshot::capture(resumed.world());
    for case in 0..5 {
        let mut bad = before.clone();
        match case {
            0 => {
                bad.entities[0].carrier_bolt_banks.pop();
            }
            1 => {
                bad.entities[0].carrier_bolt_banks[1] = "port".into();
            }
            2 => {
                bad.entities[0].weapons.as_mut().unwrap().blasters[0].pending_volley = 1;
            }
            3 => {
                bad.entities[0].carrier_bolt_banks[0].clear();
            }
            4 => {
                let mut duplicate = bad.entities[0].clone();
                for bank in &mut duplicate.weapons.as_mut().unwrap().blasters {
                    for round in &mut bank.in_flight {
                        round.id.push_str("-other");
                    }
                }
                bad.entities.push(duplicate);
            }
            _ => unreachable!(),
        }
        assert!(
            !crate::snapshot::restore(resumed.world_mut(), &bad).is_complete(),
            "case{case}"
        );
        assert_eq!(crate::snapshot::capture(resumed.world()), before);
    }
    let left = carriers(original.world_mut())[0];
    let right = carriers(resumed.world_mut())[0];
    for step in 0..360 {
        assert_eq!(
            crate::snapshot::capture(original.world()),
            crate::snapshot::capture(resumed.world()),
            "step{step}"
        );
        assert_same_digest(original.world(), resumed.world());
        for (app, entity) in [(&mut original, left), (&mut resumed, right)] {
            for bank in &mut app
                .world_mut()
                .get_mut::<BlasterSystemResource>(entity)
                .unwrap()
                .0
            {
                // The PDC owner supplies the shared flight-only progression;
                // a carrier must not visit charging, volley or payment code.
                bank.advance_flight_only(1.0 / 60.0);
                bank.finish_collision_interval();
            }
        }
    }
}

#[test]
fn forged_carrier_identity_is_refused_before_roster_or_clock_writes() {
    let mut app = host();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), emitted_bolts()))
        .id();
    app.world_mut().despawn(source);
    app.world_mut().flush();
    let before = crate::snapshot::capture(app.world());
    assert_eq!(before.entities.len(), 1);
    assert!(before.entities[0].weapon_flight_carrier);
    let before_digest = crate::sim_digest::world_digest(app.world());
    for invalid in [
        String::new(),
        "  ".into(),
        "carrier\nforged".into(),
        "x".repeat(129),
        "é".repeat(65),
    ] {
        let mut bad = before.clone();
        bad.tick = bad.tick.saturating_add(1);
        bad.entities[0].uuid = invalid.clone();
        let report = crate::snapshot::restore(app.world_mut(), &bad);
        assert!(!report.is_complete(), "invalid identity {invalid:?}");
        assert!(format!("{:?}", report.gaps).contains("bounded physical identity"));
        app.world_mut().flush();
        assert_eq!(crate::snapshot::capture(app.world()), before);
        assert_eq!(crate::sim_digest::world_digest(app.world()), before_digest);
        assert_eq!(carriers(app.world_mut()).len(), 1);
    }
    let mut fresh = host();
    let report = crate::snapshot::restore(fresh.world_mut(), &before);
    assert!(report.is_complete(), "{report:?}");
    fresh.world_mut().flush();
    assert_eq!(crate::snapshot::capture(fresh.world()), before);
    assert_same_digest(app.world(), fresh.world());
}

#[test]
fn settlement_drops_only_empty_bolt_banks_and_removes_only_fully_drained_carriers() {
    use crate::console::weapons::blaster::BlasterSystemResource;
    use bevy::ecs::system::RunSystemOnce;
    let mut app = host();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), emitted_bolts()))
        .id();
    app.world_mut().despawn(source);
    app.world_mut().flush();
    let carrier = carriers(app.world_mut())[0];
    // Actual collision consumption removes that one emitted vector entry.
    let id = app.world().get::<BlasterSystemResource>(carrier).unwrap().0[0].in_flight[0]
        .id
        .clone();
    app.world_mut()
        .get_mut::<BlasterSystemResource>(carrier)
        .unwrap()
        .0[0]
        .consume_hit(&id)
        .unwrap();
    app.world_mut().run_system_once(settle_carriers).unwrap();
    app.world_mut().flush();
    assert_eq!(carriers(app.world_mut()).len(), 1);
    let saved = crate::snapshot::capture(app.world());
    assert_eq!(saved.entities[0].carrier_bolt_banks, vec!["starboard"]);
    assert_eq!(
        saved.entities[0].weapons.as_ref().unwrap().blasters.len(),
        1
    );
    let mut resumed = host();
    assert!(crate::snapshot::restore(resumed.world_mut(), &saved).is_complete());
    assert_eq!(crate::snapshot::capture(resumed.world()), saved);
    let id = app.world().get::<BlasterSystemResource>(carrier).unwrap().0[0].in_flight[0]
        .id
        .clone();
    app.world_mut()
        .get_mut::<BlasterSystemResource>(carrier)
        .unwrap()
        .0[0]
        .consume_hit(&id)
        .unwrap();
    app.world_mut().run_system_once(settle_carriers).unwrap();
    app.world_mut().flush();
    assert!(carriers(app.world_mut()).is_empty());
}

#[test]
fn mixed_source_rehomes_once_independently_of_component_registration_order() {
    let run = |reverse| {
        let mut app = host_with_registration(reverse);
        let source = app
            .world_mut()
            .spawn((EntityUuid("source".into()), launched(), emitted_bolts()))
            .id();
        app.world_mut().despawn(source);
        app.world_mut().flush();
        assert_eq!(carriers(app.world_mut()).len(), 1);
        let capture = crate::snapshot::capture(app.world());
        let weapons = capture.entities[0].weapons.as_ref().unwrap();
        assert_eq!(weapons.torpedoes_in_flight.len(), 1);
        assert_eq!(weapons.blasters.len(), 2);
        (capture, crate::sim_digest::world_digest(app.world()))
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn sequential_equipment_replacements_before_flush_cannot_rehome_copies_twice() {
    for (bundled, flush_between) in [(false, false), (true, false), (false, true)] {
        let mut app = host();
        let guided = launched();
        let bolts = emitted_bolts();
        let source = app
            .world_mut()
            .spawn((EntityUuid("source".into()), guided.clone(), bolts.clone()))
            .id();
        let before = crate::snapshot::capture(app.world()).entities[0]
            .weapons
            .clone()
            .unwrap();
        if bundled {
            app.world_mut().entity_mut(source).insert((guided, bolts));
        } else {
            app.world_mut().entity_mut(source).insert(guided);
            if flush_between {
                app.world_mut().flush();
                assert_eq!(carriers(app.world_mut()).len(), 1);
            }
            app.world_mut().entity_mut(source).insert(bolts);
        }
        app.world_mut().flush();
        assert_eq!(carriers(app.world_mut()).len(), 1);
        let saved = crate::snapshot::capture(app.world());
        let carrier = saved
            .entities
            .iter()
            .find(|row| row.weapon_flight_carrier)
            .unwrap()
            .weapons
            .as_ref()
            .unwrap();
        assert_eq!(carrier.torpedoes_in_flight, before.torpedoes_in_flight);
        for (actual, old) in carrier.blasters.iter().zip(&before.blasters) {
            assert_eq!(actual.in_flight, old.in_flight);
        }
        let current = saved
            .entities
            .iter()
            .find(|row| row.uuid == "source")
            .unwrap()
            .weapons
            .as_ref()
            .unwrap();
        assert!(current.torpedoes_in_flight.is_empty());
        assert!(current
            .blasters
            .iter()
            .all(|bank| bank.in_flight.is_empty()));
    }
}

#[test]
fn restoring_existing_carrier_replaces_its_actual_remaining_bank_shape() {
    use crate::console::weapons::blaster::BlasterSystemResource;
    use bevy::ecs::system::RunSystemOnce;
    let mut app = host();
    let source = app
        .world_mut()
        .spawn((EntityUuid("source".into()), launched(), emitted_bolts()))
        .id();
    app.world_mut().despawn(source);
    app.world_mut().flush();
    let carrier = carriers(app.world_mut())[0];
    let full = crate::snapshot::capture(app.world());
    let full_digest = crate::sim_digest::world_digest(app.world());
    assert_eq!(
        full.entities[0].carrier_bolt_banks,
        vec!["port", "starboard"]
    );
    for _ in 0..2 {
        // Actual collision consumes one bank at a time, then the real carrier
        // settlement drops that now-empty bank from its physical recipe.
        let id = app.world().get::<BlasterSystemResource>(carrier).unwrap().0[0].in_flight[0]
            .id
            .clone();
        app.world_mut()
            .get_mut::<BlasterSystemResource>(carrier)
            .unwrap()
            .0[0]
            .consume_hit(&id)
            .unwrap();
        app.world_mut().run_system_once(settle_carriers).unwrap();
        app.world_mut().flush();
    }
    let guided_only = crate::snapshot::capture(app.world());
    let guided_digest = crate::sim_digest::world_digest(app.world());
    assert!(guided_only.entities[0].carrier_bolt_banks.is_empty());
    assert_eq!(
        guided_only.entities[0]
            .weapons
            .as_ref()
            .unwrap()
            .torpedoes_in_flight
            .len(),
        1
    );
    let encoded = serde_json::to_vec(&full).unwrap();
    let decoded = serde_json::from_slice(&encoded).unwrap();
    let report = crate::snapshot::restore(app.world_mut(), &decoded);
    assert!(report.is_complete(), "{report:?}");
    app.world_mut().flush();
    assert_eq!(carriers(app.world_mut()), vec![carrier]);
    assert_eq!(crate::snapshot::capture(app.world()), full);
    assert_eq!(crate::sim_digest::world_digest(app.world()), full_digest);
    // The inverse shape change must also discard the receiver's old bolts,
    // without rehoming them into an extra carrier during roster application.
    let report = crate::snapshot::restore(app.world_mut(), &guided_only);
    assert!(report.is_complete(), "{report:?}");
    app.world_mut().flush();
    assert!(app.world().get::<BlasterSystemResource>(carrier).is_none());
    assert_eq!(carriers(app.world_mut()), vec![carrier]);
    assert_eq!(crate::snapshot::capture(app.world()), guided_only);
    assert_eq!(crate::sim_digest::world_digest(app.world()), guided_digest);
    crate::snapshot::apply_entity_state(app.world_mut(), carrier, &full.entities[0], full.tick);
    app.world_mut().flush();
    assert_eq!(
        crate::snapshot::capture(app.world()),
        guided_only,
        "single-row inverse cannot replace a physical carrier outside bulk restore"
    );
}

use super::super::rehearsal_request::{FittingPlan, PlanSection};
use super::*;
use crate::resources::fitting::EquipmentLocation;

fn fixture() -> (World, Entity) {
    // Actual accepted include-resolved hull, never a toy hardware substitute.
    let config = crate::entities::include_resolve::load_entity_config(CRUISER).unwrap();
    let mut context = crate::resources::fitting_server::planner_context(&config).unwrap();
    context.hull_path = CRUISER.into();
    let mut sections = BTreeMap::<String, PlanSection>::new();
    for slot in &context.slots {
        sections
            .entry(slot.home_station.clone())
            .or_insert_with(|| PlanSection {
                explicit: false,
                choices: BTreeMap::new(),
            })
            .choices
            .insert(slot.slot.clone(), slot.default_equipment.clone());
    }
    let request = RehearsalRequest {
        version: 1,
        workload: Workload::SustainedEngagement,
        plan: FittingPlan {
            version: 1,
            hull_path: CRUISER.into(),
            content_id: context.content_id,
            scope: "*".into(),
            sections,
        },
    };
    let mut world = World::new();
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "00000000-0000-0000-0000-000000000013".into(),
        Some("rehearsal".into()),
    );
    world.flush();
    let subject = crate::entities::include_resolve::load_entity_config(
        "assets/entities/fitting_rehearsal_subject.toml",
    )
    .unwrap();
    crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &subject,
        Vec3::new(0.0, 0.0, -25.0),
        "00000000-0000-0000-0000-000000000014".into(),
        Some("rehearsal-subject".into()),
    );
    world.flush();
    world
        .entity_mut(entity)
        .insert(crate::ship_slots::AuthoredShipSlotId("rehearsal".into()));
    world
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntityTemplatePath::new(CRUISER));
    world.insert_resource(crate::ship_slots::FrozenShipSlots(vec![
        crate::ship_slots::LaunchedSlot {
            slot_id: "rehearsal".into(),
            hull: CRUISER.into(),
            claimant: Some("private-test".into()),
            source: crate::ship_slots::LaunchSource::Claimed,
        },
    ]));
    world.insert_resource(super::super::test_clock::DisposableTest);
    world.insert_resource(super::super::test_clock::TestClock::default());
    world.insert_resource(crate::gm::actions::action::SimulationPaused::default());
    world.insert_resource(crate::sim_tick::SimTick(0));
    world.insert_resource(Time::<Fixed>::from_hz(60.0));
    world.insert_resource(super::super::test_clock::TestControls::default());
    world.insert_resource(crate::effect_queue::EffectQueue::<
        crate::infrastructure::CapacityAdjustment,
    >::default());
    let choices = request
        .choices(
            &crate::resources::fitting_server::planner_context(&config)
                .map(|mut c| {
                    c.hull_path = CRUISER.into();
                    c
                })
                .unwrap(),
        )
        .unwrap();
    world.insert_resource(PreparedRehearsal {
        request,
        choices,
        accepted: config,
        profile: ReferenceProfile {
            workload: Workload::SustainedEngagement,
            warmup_intervals: 0,
            phases: vec![report::ReferencePhase {
                activity: report::Activity::Engagement,
                intervals: 60,
            }],
        },
        revision: "fixture".into(),
        workload_revision: "accepted-world".into(),
        run_generation: 1,
        selection: super::super::test_protocol::TestSelection {
            world: Workload::SustainedEngagement.world().into(),
            slot: Some("rehearsal".into()),
            ship: CRUISER.into(),
            seed: 13,
        },
        attempted: false,
        error: None,
        target: None,
        initialized_tick: None,
    });
    (world, entity)
}

#[test]
fn settled_sampler_skips_initialization_pause_and_duplicate_ticks_and_freezes_exact_window() {
    let (mut world, _) = fixture();
    world.insert_resource(State::new(crate::core::messages::GamePhase::InProgress));
    initialize(&mut world);
    report::sample(&mut world);
    assert_eq!(result(&world).unwrap().measured_intervals, 0);
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    world
        .resource_mut::<crate::gm::actions::action::SimulationPaused>()
        .0 = true;
    report::sample(&mut world);
    assert_eq!(result(&world).unwrap().measured_intervals, 0);
    world
        .resource_mut::<crate::gm::actions::action::SimulationPaused>()
        .0 = false;
    for tick in 1..=65 {
        world.resource_mut::<crate::sim_tick::SimTick>().0 = tick;
        report::sample(&mut world);
        report::sample(&mut world);
    }
    let actual = result(&world).unwrap();
    assert!(actual.complete && actual.insufficient_activity);
    assert_eq!(actual.measured_intervals, 60);
    assert_eq!(actual.last_measured_tick, Some(60));
    assert!(actual.buffers.values().all(|b| b.paid_energy == 0.0));
    assert_eq!(
        world
            .resource::<super::super::test_clock::TestControls>()
            .0
            .len(),
        1
    );
    assert_eq!(actual.charts.last().unwrap().measured_intervals, 60);
}

#[test]
fn sampler_refuses_unsettled_resource_effects_instead_of_applying_them_twice() {
    let (mut world, _) = fixture();
    world.insert_resource(State::new(crate::core::messages::GamePhase::InProgress));
    initialize(&mut world);
    world.resource_mut::<crate::sim_tick::SimTick>().0 = 1;
    world.resource_mut::<crate::effect_queue::EffectQueue<crate::infrastructure::CapacityAdjustment>>().0
        .push(crate::infrastructure::CapacityAdjustment { uuid: "unsettled-owner".into(), capacity: "coolant".into(), delta: -1 });
    report::sample(&mut world);
    let refused = result(&world).unwrap();
    assert_eq!(refused.measured_intervals, 0);
    assert!(refused.error.unwrap().contains("not settled"));
}

#[test]
fn accepted_cruiser_default_and_sidegrades_preserve_actual_assemblies_and_reserves() {
    for alternative in [false, true] {
        let (mut world, entity) = fixture();
        let fitting = world.get::<ShipFitting>(entity).unwrap().clone();
        let power = world
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .capture_continuation();
        let infrastructure = world
            .get::<crate::infrastructure::InfrastructureCondition>(entity)
            .unwrap()
            .clone();
        if alternative {
            let mut prepared = world.resource_mut::<PreparedRehearsal>();
            prepared
                .choices
                .insert("fore-beam".into(), "cruiser-beam-pulse".into());
            prepared
                .choices
                .insert("reactor".into(), "cruiser-reactor-responsive".into());
        }
        initialize(&mut world);
        assert_eq!(world.resource::<PreparedRehearsal>().error, None);
        assert_eq!(world.resource::<PreparedRehearsal>().target, Some(entity));
        assert_eq!(
            world
                .get::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .capture_continuation(),
            power
        );
        assert_eq!(
            world
                .get::<crate::infrastructure::InfrastructureCondition>(entity)
                .unwrap(),
            &infrastructure
        );
        let after = world.get::<ShipFitting>(entity).unwrap();
        assert_eq!(
            after.equipment.assemblies.keys().collect::<Vec<_>>(),
            fitting.equipment.assemblies.keys().collect::<Vec<_>>()
        );
        for (id, item) in &fitting.equipment.assemblies {
            let actual = &after.equipment.assemblies[id];
            assert_eq!(actual.location, item.location);
            assert_eq!(actual.condition, item.condition);
            if !matches!(item.location, EquipmentLocation::Installed { .. }) {
                assert_eq!(actual, item);
            }
        }
        if alternative {
            assert_eq!(
                after.equipment.installed("fore-beam").unwrap().definition,
                "cruiser-beam-pulse"
            );
            assert_eq!(
                after.equipment.installed("reactor").unwrap().definition,
                "cruiser-reactor-responsive"
            );
            assert_eq!(
                world
                    .get::<ShipPowerSystem>(entity)
                    .unwrap()
                    .0
                    .config()
                    .reactor
                    .sustainable_output_per_sec,
                28.53
            );
        } else {
            assert_eq!(*after, fitting);
        }
        // This constructor can never reapply a favourite to an operating run.
        initialize(&mut world);
        assert_eq!(
            world
                .get::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .capture_continuation(),
            power
        );
    }
}

#[test]
fn invalid_or_late_initial_fit_is_atomic_and_holds_child_before_default_work() {
    for fault in 0..5 {
        let (mut world, entity) = fixture();
        match fault {
            0 => world
                .resource_mut::<PreparedRehearsal>()
                .choices
                .insert("reactor".into(), "cruiser-beam-pulse".into()),
            1 => {
                world
                    .entity_mut(entity)
                    .remove::<crate::ship::compartment_damage::ShipDamage>();
                None
            }
            2 => {
                world.get_mut::<ShipFitting>(entity).unwrap().last_tick = Some(1);
                None
            }
            3 => {
                world
                    .resource_mut::<PreparedRehearsal>()
                    .request
                    .plan
                    .content_id = "0000000000000000".into();
                None
            }
            _ => {
                world.remove_resource::<super::super::test_clock::DisposableTest>();
                None
            }
        };
        let fitting = world.get::<ShipFitting>(entity).unwrap().clone();
        let power = world.get::<ShipPowerSystem>(entity).unwrap().clone();
        let infrastructure = world
            .get::<crate::infrastructure::InfrastructureCondition>(entity)
            .unwrap()
            .clone();
        initialize(&mut world);
        assert!(
            world.resource::<PreparedRehearsal>().error.is_some(),
            "fault {fault}"
        );
        assert_eq!(*world.get::<ShipFitting>(entity).unwrap(), fitting);
        assert_eq!(world.get::<ShipPowerSystem>(entity).unwrap().0, power.0);
        assert_eq!(
            *world
                .get::<crate::infrastructure::InfrastructureCondition>(entity)
                .unwrap(),
            infrastructure
        );
        assert!(
            world
                .resource::<super::super::test_clock::TestClock>()
                .paused
        );
    }
}

#[test]
fn private_request_refuses_duplicate_keys_unknown_fields_and_stale_choices() {
    let (world, entity) = fixture();
    let request = &world.resource::<PreparedRehearsal>().request;
    let selection = super::super::test_protocol::TestSelection {
        world: request.workload.world().into(),
        slot: Some("rehearsal".into()),
        ship: CRUISER.into(),
        seed: 13,
    };
    request.validate(&selection).unwrap();
    let launch = Launch {
        selection: selection.clone(),
        revision: "capture".into(),
        breakpoint: None,
        rehearsal: Some(request.clone()),
        run_generation: 1,
    };
    let encoded = launch.bounded_json().unwrap();
    Launch::decode(encoded.as_bytes()).unwrap();
    assert!(
        Launch::decode(format!("{}{}", " ".repeat(32_768), encoded).as_bytes()).is_err(),
        "wire whitespace cannot bypass the complete encoded rehearsal envelope"
    );
    let json = serde_json::to_string(request).unwrap();
    for malformed in [
        json.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
        json.replacen(
            "\"explicit\":false",
            "\"explicit\":false,\"actor\":\"other\"",
            1,
        ),
        json.replacen(
            "\"reactor\":\"cruiser-reactor-standard\"",
            "\"reactor\":\"cruiser-reactor-standard\",\"reactor\":\"cruiser-reactor-standard\"",
            1,
        ),
    ] {
        assert!(serde_json::from_str::<RehearsalRequest>(&malformed).is_err());
    }
    let context = world.get::<FittingPlannerCatalog>(entity).unwrap();
    let mut context = context.0.clone();
    context.hull_path = CRUISER.into();
    let mut stale = request.clone();
    stale.plan.content_id = "0000000000000000".into();
    assert!(stale.choices(&context).is_err());
    let mut wrong = request.clone();
    wrong
        .plan
        .sections
        .values_mut()
        .next()
        .unwrap()
        .choices
        .clear();
    assert!(wrong.choices(&context).is_err());
}

#[test]
fn reference_subject_matches_actual_accepted_utility_hardware_without_synthesized_extras() {
    // These are accepted EntityConfig values after the real loader's generated
    // System admission, rather than source/JavaScript composition alone.
    let mut expected = crate::entities::include_resolve::load_entity_config(
        "assets/entities/utility_shuttle.toml",
    )
    .unwrap();
    let subject = crate::entities::include_resolve::load_entity_config(
        "assets/entities/fitting_rehearsal_subject.toml",
    )
    .unwrap();
    expected.name = Some("entity.fitting_rehearsal_subject.name".into());
    expected.faction = Some("bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb".parse().unwrap());
    expected.collider.as_mut().unwrap().movable = false;
    let interior = expected.interior.as_mut().unwrap();
    assert_eq!(interior.compartments.len(), 1);
    interior.compartments[0].max_hp = 10000.0;
    let ship = subject.ship_config.as_ref().unwrap();
    assert_eq!(ship.systems.len(), 9);
    let installed: std::collections::BTreeSet<_> =
        ship.systems.iter().map(|system| &system.id).collect();
    let placements: std::collections::BTreeSet<_> = subject
        .interior
        .as_ref()
        .unwrap()
        .placements
        .keys()
        .collect();
    assert_eq!(installed, placements);
    assert_eq!(subject, expected);
}

#[test]
fn planner_content_id_ignores_map_insertion_order_but_binds_full_content_and_array_order() {
    let config = crate::entities::include_resolve::load_entity_config(CRUISER).unwrap();
    let id = |config: &crate::entities::config::EntityConfig| {
        crate::resources::fitting_server::planner_context(config)
            .unwrap()
            .content_id
    };
    let mut reordered = config.clone();
    let groups = &mut reordered.ship_config.as_mut().unwrap().power_groups;
    assert!(groups.len() > 1);
    let mut entries: Vec<_> = groups.drain().collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    *groups = std::collections::HashMap::new();
    for (key, value) in entries.into_iter().rev() {
        groups.insert(key, value);
    }
    assert_eq!(config, reordered);
    assert_eq!(id(&config), id(&reordered));

    let mut changed = config.clone();
    changed.name = Some("full-hull-content-id-regression".into());
    assert_ne!(id(&config), id(&changed));
    changed = config.clone();
    changed.ship_config.as_mut().unwrap().systems.swap(0, 1);
    assert_ne!(id(&config), id(&changed));
}

#[test]
fn actual_captured_reference_worlds_resolve_and_compile_complete_cruiser_equipment() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = BTreeMap::new();
    crate::native_capture::walk(&root.join("assets"), &mut |entry| {
        if entry.file_type().unwrap().is_dir() {
            return Ok(Some(entry.path()));
        }
        let path = crate::native_capture::relative_name(&entry.path(), &root)?;
        if path.ends_with(".toml") || path.ends_with(".rhai") {
            files.insert(
                path,
                std::fs::read_to_string(entry.path()).map_err(|e| e.to_string())?,
            );
        }
        Ok(None)
    })
    .unwrap();
    let disk = crate::entities::include_resolve::load_entity_config(CRUISER).unwrap();
    let captured = super::super::Sources(files.clone())
        .load_template(CRUISER)
        .unwrap();
    assert_eq!(disk, captured, "independent accepted loader parity");
    assert_eq!(
        crate::resources::fitting_server::planner_context(&disk),
        crate::resources::fitting_server::planner_context(&captured),
        "equal accepted hulls must bind the same favourite"
    );
    let (world, _) = fixture();
    for workload in [Workload::SustainedEngagement, Workload::BurstRecovery] {
        let mut request = world.resource::<PreparedRehearsal>().request.clone();
        request.workload = workload;
        let launch = Launch {
            selection: super::super::test_protocol::TestSelection {
                world: workload.world().into(),
                slot: Some("rehearsal".into()),
                ship: CRUISER.into(),
                seed: 13,
            },
            revision: "actual-source-capture".into(),
            breakpoint: None,
            rehearsal: Some(request),
            run_generation: 1,
        };
        let selected = prepare(files.clone(), &launch).unwrap().unwrap();
        assert_eq!(
            selected
                .accepted
                .ship_config
                .as_ref()
                .unwrap()
                .stations
                .len(),
            6
        );
        assert!(
            selected.accepted.energy.is_some()
                && selected.accepted.damage.is_some()
                && selected.accepted.interior.is_some()
        );
        let profile = crate::resources::fitting::CompiledFitting::compile(&selected.accepted)
            .unwrap()
            .unwrap();
        let mut fit = crate::resources::fitting::FittingState::from_profile(&profile).unwrap();
        for (slot, definition) in [
            ("fore-beam", "cruiser-beam-pulse"),
            ("reactor", "cruiser-reactor-responsive"),
        ] {
            profile.compatible(slot, definition).unwrap();
            let id = fit.installed(slot).unwrap().id.clone();
            fit.assemblies.get_mut(&id).unwrap().definition = definition.into();
            fit.desired.insert(slot.into(), definition.into());
        }
        let alternative = profile.effective_config(&fit).unwrap();
        assert_eq!(alternative.ship_config, selected.accepted.ship_config);
        assert_eq!(alternative.interior, selected.accepted.interior);
        assert_eq!(
            alternative.energy.as_ref().unwrap().consumers.len(),
            selected.accepted.energy.as_ref().unwrap().consumers.len()
        );
        let mut bad = launch.clone();
        bad.rehearsal.as_mut().unwrap().plan.content_id = "0000000000000000".into();
        assert!(prepare(files.clone(), &bad).is_err());
    }
}

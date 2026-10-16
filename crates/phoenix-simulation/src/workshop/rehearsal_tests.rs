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
        attempted: false,
        error: None,
        target: None,
        initialized_tick: None,
    });
    (world, entity)
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

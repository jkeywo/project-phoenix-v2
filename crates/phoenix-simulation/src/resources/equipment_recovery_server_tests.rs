use super::*;
use crate::resources::fitting::EquipmentLocation;
use phoenix_model::damage::{ComponentCondition, ComponentState};

fn sid(id: &str) -> SystemId {
    SystemId(id.into())
}
fn rig() -> (World, Entity, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    let actor = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "00000000-0000-0000-0000-000000000101".into(),
        None,
    );
    let source = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::new(4.0, 0.0, 0.0),
        "00000000-0000-0000-0000-000000000102".into(),
        None,
    );
    world.flush();
    let owner = config.recovery.as_ref().unwrap().system.clone();
    world.entity_mut(actor).insert(ShipEquipmentRecovery {
        owner,
        next_id: 1,
        job: None,
        last_tick: None,
    });
    let uuid = world.get::<EntityUuid>(source).unwrap().0.clone();
    world
        .get_mut::<crate::server_app::ShipSystemBlackboards>(actor)
        .unwrap()
        .0
        .insert(
            sid("sensor-radar"),
            crate::core::messages::SystemBlackboard::SensorRadar(
                crate::core::messages::SensorRadarBlackboard {
                    selected_target: Some(uuid),
                    ..Default::default()
                },
            ),
        );
    let mut damage = world.get_mut::<ShipDamage>(source).unwrap();
    damage.state.catastrophic = true;
    *damage
        .state
        .components
        .get_mut(&sid("phaser-fore"))
        .unwrap()
        .get_mut("operation")
        .unwrap() = ComponentState {
        condition: ComponentCondition::Damaged,
        dose: 0.5,
        last_source: Some("real-damage".into()),
    };
    (world, actor, source)
}
fn start_beam(world: &mut World, actor: Entity, source: Entity) -> Result<(), String> {
    let uuid = world.get::<EntityUuid>(source).unwrap().0.clone();
    let owner = world
        .get::<ShipEquipmentRecovery>(actor)
        .unwrap()
        .owner
        .clone();
    start(
        world,
        actor,
        &owner,
        &uuid,
        "fore-beam",
        "engineering-equipment",
    )
}
fn equipment(world: &World, actor: Entity) -> &ShipFitting {
    world.get::<ShipFitting>(actor).unwrap()
}
fn power(world: &World, actor: Entity) -> crate::modifiers::power_system::PowerState {
    world
        .get::<ShipPowerSystem>(actor)
        .unwrap()
        .0
        .capture_continuation()
}

#[test]
fn actual_paid_tool_moves_one_surviving_assembly_and_its_condition_without_refilling_buffers() {
    for control in [
        crate::ship::control_source::ControlSource::Human,
        crate::ship::control_source::ControlSource::Ai,
    ] {
        let (mut world, actor, source) = rig();
        let tool = world
            .get::<ShipEquipmentRecovery>(actor)
            .unwrap()
            .owner
            .clone();
        world
            .get_mut::<ShipSystemControlSources>(actor)
            .unwrap()
            .0
            .set(tool.clone(), control);
        let actual = equipment(&world, source)
            .equipment
            .installed("fore-beam")
            .unwrap()
            .id
            .clone();
        let initial = equipment(&world, actor).equipment.assemblies.len();
        let source_power = power(&world, source);
        let actor_power = power(&world, actor);
        let options = project(&world, actor).unwrap().options;
        assert!(options
            .iter()
            .any(|item| item.slot == "fore-beam"
                && item.destination_locker == "engineering-equipment"));
        start_beam(&mut world, actor, source).unwrap();
        for tick in 1..120 {
            advance(&mut world, actor, tick, 0.25);
        }
        let job = world
            .get::<ShipEquipmentRecovery>(actor)
            .unwrap()
            .job
            .as_ref()
            .unwrap();
        assert_eq!(job.outcome, Some(JobOutcome::Completed));
        assert_eq!(job.activity.progress, job.activity.work_secs);
        assert!(equipment(&world, source)
            .equipment
            .installed("fore-beam")
            .is_none());
        assert!(!equipment(&world, source)
            .equipment
            .assemblies
            .contains_key(&actual));
        let recovered = &equipment(&world, actor).equipment.assemblies[&actual];
        assert!(
            matches!(&recovered.location,EquipmentLocation::Stored { locker } if locker == "engineering-equipment")
        );
        assert_eq!(
            recovered.condition.as_ref().unwrap().condition,
            ComponentCondition::Damaged
        );
        assert_eq!(recovered.condition.as_ref().unwrap().dose, 0.5);
        assert_eq!(
            equipment(&world, actor).equipment.assemblies.len(),
            initial + 1
        );
        assert_eq!(power(&world, source), source_power);
        let after = power(&world, actor);
        assert!(
            after.network.consumers[&tool].energy < actor_power.network.consumers[&tool].energy
        );
        for (system, before) in &actor_power.network.consumers {
            if system != &tool {
                assert_eq!(&after.network.consumers[system], before);
            }
        }
        assert!(crate::resources::fitting_server::unavailable(
            world.get::<CompiledShipFitting>(source).unwrap(),
            equipment(&world, source),
            &sid("phaser-fore")
        ));
        assert!(!world
            .get::<crate::console::weapons::PhaserCombatConfigResource>(source)
            .unwrap()
            .0
            .banks
            .iter()
            .any(|bank| bank.id == "fore"));
        let completed = equipment(&world, actor).clone();
        advance(&mut world, actor, 121, 10.0);
        assert_eq!(equipment(&world, actor), &completed);
    }
}

#[test]
fn cancellation_retains_paid_cost_and_source_item_and_releases_actual_space_claim() {
    let (mut world, actor, source) = rig();
    let before = equipment(&world, source).clone();
    let owner = world
        .get::<ShipEquipmentRecovery>(actor)
        .unwrap()
        .owner
        .clone();
    start_beam(&mut world, actor, source).unwrap();
    advance(&mut world, actor, 1, 1.0);
    let spent = power(&world, actor);
    let profile = &world.get::<CompiledShipFitting>(actor).unwrap().0;
    assert_eq!(
        reserved_space(
            world.get::<ShipEquipmentRecovery>(actor).unwrap(),
            "engineering-equipment",
            profile
        ),
        1
    );
    assert!(source_claimed(
        &world,
        &world.get::<EntityUuid>(source).unwrap().0,
        &sid("phaser-fore")
    ));
    cancel(&mut world, actor, &owner).unwrap();
    advance(&mut world, actor, 2, 20.0);
    assert_eq!(equipment(&world, source), &before);
    assert_eq!(power(&world, actor), spent);
    assert!(!source_claimed(
        &world,
        &world.get::<EntityUuid>(source).unwrap().0,
        &sid("phaser-fore")
    ));
}

#[test]
fn major_assemblies_are_reported_but_require_a_supported_facility_extraction_adapter() {
    let (mut world, actor, source) = rig();
    let before = equipment(&world, source).clone();
    let paid = power(&world, actor);
    let owner = world
        .get::<ShipEquipmentRecovery>(actor)
        .unwrap()
        .owner
        .clone();
    let uuid = world.get::<EntityUuid>(source).unwrap().0.clone();
    let view = project(&world, actor).unwrap();
    assert!(view.options.iter().any(|option| option.slot == "reactor"
        && option.blocked_reason.as_deref() == Some("major_equipment_requires_facility")));
    assert_eq!(
        start(
            &mut world,
            actor,
            &owner,
            &uuid,
            "reactor",
            "engineering-equipment"
        )
        .unwrap_err(),
        "major_equipment_requires_facility"
    );
    assert_eq!(equipment(&world, source), &before);
    assert_eq!(power(&world, actor), paid);
}

#[test]
fn unknown_active_full_and_repair_claimed_sources_refuse_before_any_payment_or_item_write() {
    for mode in 0..4 {
        let (mut world, actor, source) = rig();
        match mode {
            0 => {
                world
                    .get_mut::<crate::server_app::ShipSystemBlackboards>(actor)
                    .unwrap()
                    .0
                    .clear();
            }
            1 => {
                world
                    .get_mut::<ShipDamage>(source)
                    .unwrap()
                    .state
                    .catastrophic = false;
            }
            2 => {
                let mut fitting = world.get_mut::<ShipFitting>(actor).unwrap();
                let item = fitting.equipment.assemblies.values_mut().find(|item| matches!(&item.location,EquipmentLocation::Stored { locker } if locker == "weapons-equipment")).unwrap();
                item.location = EquipmentLocation::Stored {
                    locker: "engineering-equipment".into(),
                };
            }
            _ => {
                world
                    .get_mut::<crate::console::repair::server::ShipRepairTeams>(source)
                    .unwrap()
                    .0
                    .dispatch(0, sid("phaser-fore"), "Fore".into());
            }
        }
        let before = (
            equipment(&world, actor).clone(),
            equipment(&world, source).clone(),
            power(&world, actor),
        );
        assert!(
            start_beam(&mut world, actor, source).is_err(),
            "mode {mode}"
        );
        assert_eq!(
            (
                equipment(&world, actor).clone(),
                equipment(&world, source).clone(),
                power(&world, actor)
            ),
            before
        );
        assert!(world
            .get::<ShipEquipmentRecovery>(actor)
            .unwrap()
            .job
            .is_none());
    }
}

#[test]
fn lost_source_or_range_pauses_without_payment_and_recovery_cannot_ignore_a_changed_actual_item() {
    let (mut world, actor, source) = rig();
    start_beam(&mut world, actor, source).unwrap();
    advance(&mut world, actor, 1, 0.5);
    let saved = world.get::<ShipEquipmentRecovery>(actor).unwrap().clone();
    let paid = power(&world, actor);
    world.get_mut::<Transform>(source).unwrap().translation.x = 1000.0;
    advance(&mut world, actor, 2, 10.0);
    assert_eq!(power(&world, actor), paid);
    assert_eq!(
        world
            .get::<ShipEquipmentRecovery>(actor)
            .unwrap()
            .job
            .as_ref()
            .unwrap()
            .activity
            .progress,
        saved.job.as_ref().unwrap().activity.progress
    );
    world.get_mut::<Transform>(source).unwrap().translation.x = 4.0;
    let actual = equipment(&world, source)
        .equipment
        .installed("fore-beam")
        .unwrap()
        .id
        .clone();
    world
        .get_mut::<ShipFitting>(source)
        .unwrap()
        .equipment
        .assemblies
        .remove(&actual);
    advance(&mut world, actor, 3, 10.0);
    assert_eq!(power(&world, actor), paid);
    assert!(!equipment(&world, actor)
        .equipment
        .assemblies
        .contains_key(&actual));
}

#[test]
fn ordinary_recovery_on_off_gate_can_pause_equipment_work_without_a_second_controller() {
    let (mut world, actor, source) = rig();
    start_beam(&mut world, actor, source).unwrap();
    advance(&mut world, actor, 1, 0.25);
    let progress = world
        .get::<ShipEquipmentRecovery>(actor)
        .unwrap()
        .job
        .as_ref()
        .unwrap()
        .activity
        .progress;
    let paid = power(&world, actor);
    world
        .get_mut::<crate::resources::recovery_server::ShipRecovery>(actor)
        .unwrap()
        .0
        .set_enabled(false);
    advance(&mut world, actor, 2, 20.0);
    assert_eq!(power(&world, actor), paid);
    assert_eq!(
        world
            .get::<ShipEquipmentRecovery>(actor)
            .unwrap()
            .job
            .as_ref()
            .unwrap()
            .activity
            .progress,
        progress
    );
    world
        .get_mut::<crate::resources::recovery_server::ShipRecovery>(actor)
        .unwrap()
        .0
        .set_enabled(true);
    for tick in 3..120 {
        advance(&mut world, actor, tick, 0.25);
    }
    assert_eq!(
        world
            .get::<ShipEquipmentRecovery>(actor)
            .unwrap()
            .job
            .as_ref()
            .unwrap()
            .outcome,
        Some(JobOutcome::Completed)
    );
    assert!(
        !world
            .get::<crate::resources::recovery_server::ShipRecovery>(actor)
            .unwrap()
            .0
            .enabled
    );
}

#[test]
fn paid_job_continues_through_json_and_forged_recipe_timers_or_ids_are_rejected() {
    let (mut world, actor, source) = rig();
    start_beam(&mut world, actor, source).unwrap();
    advance(&mut world, actor, 1, 0.25);
    let saved = world.get::<ShipEquipmentRecovery>(actor).unwrap().clone();
    let decoded: ShipEquipmentRecovery =
        serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert_eq!(saved, decoded);
    let tool = &world.get::<CompiledShipRecovery>(actor).unwrap().0;
    let profile = &world.get::<CompiledShipFitting>(actor).unwrap().0;
    validate_state(&decoded, tool, Some(profile), 1).unwrap();
    for mode in 0..5 {
        let mut bad = decoded.clone();
        match mode {
            0 => bad.job.as_mut().unwrap().activity.progress = f32::NAN,
            1 => bad.job.as_mut().unwrap().activity.work_secs += 1.0,
            2 => bad.job.as_mut().unwrap().id = JobId(bad.next_id),
            3 => bad.last_tick = Some(2),
            _ => bad.job.as_mut().unwrap().activity.destination_locker = "missing".into(),
        }
        assert!(
            validate_state(&bad, tool, Some(profile), 1).is_err(),
            "mode {mode}"
        );
    }
    world.entity_mut(actor).insert(decoded);
    for tick in 2..120 {
        advance(&mut world, actor, tick, 0.25);
    }
    assert_eq!(
        world
            .get::<ShipEquipmentRecovery>(actor)
            .unwrap()
            .job
            .as_ref()
            .unwrap()
            .outcome,
        Some(JobOutcome::Completed)
    );
}

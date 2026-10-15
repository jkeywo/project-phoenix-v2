use super::*;
use crate::console::repair::server::ShipRepairTeams;
use crate::resources::fitting_server::{CompiledShipFitting, ShipFitting};

fn fixture() -> (World, Entity, EntityState) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profile = CompiledFitting::compile(&config).unwrap().unwrap();
    let mut world = World::new();
    let entity = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "00000000-0000-0000-0000-000000000014".into(),
        None,
    );
    world.flush();
    let mut state = world.get::<ShipFitting>(entity).unwrap().clone();
    let item = state
        .equipment
        .assemblies
        .values_mut()
        .find(|item| item.definition == "cruiser-beam-pulse")
        .unwrap();
    item.location = EquipmentLocation::Stored {
        locker: "support-equipment".into(),
    };
    let item_id = item.id.clone();
    state
        .equipment
        .request(&profile, "fore-beam", "cruiser-beam-pulse")
        .unwrap();
    state.requested.insert("fore-beam".into());
    state.next_request = 2;
    state.last_tick = Some(1);
    let mut teams = world.get::<ShipRepairTeams>(entity).unwrap().clone();
    let job = teams
        .0
        .claim_refit(
            0,
            1,
            profile.slot("fore-beam").unwrap().system.clone(),
            "Fore".into(),
        )
        .unwrap();
    assert!(teams.0.visit_refit_compartment(job, "support"));
    for _ in 0..400 {
        teams.0.tick_refits(0.125, &BTreeSet::new());
        if teams.0.refit_effect_enabled(job) {
            break;
        }
    }
    assert!(teams.0.refit_effect_enabled(job));
    state
        .equipment
        .pick_up_from_locker(&profile, &item_id, job.0, "support")
        .unwrap();
    assert!(teams.0.visit_refit_compartment(job, "weapons"));
    teams.0.tick_refits(0.125, &BTreeSet::new());
    assert!(
        matches!(teams.0.refit_location(job), Some(crate::ship::interior::TeamLocation::Crossing { from, .. }) if from == "support")
    );
    state.work.insert(
        "fore-beam".into(),
        crate::resources::fitting_server::RefitWork {
            request: 1,
            slot: "fore-beam".into(),
            item: item_id,
            removed_item: state
                .equipment
                .installed("fore-beam")
                .map(|item| item.id.clone()),
            locker: "weapons-equipment".into(),
            source_locker: "support-equipment".into(),
            job,
            stage: RefitStage::Delivering,
            progress: 0.0,
            reserved: false,
            waiting: "team_travelling".into(),
        },
    );
    let row = EntityState {
        uuid: world.get::<EntityUuid>(entity).unwrap().0.clone(),
        fitting: Some(state.clone()),
        repair: Some(RepairState {
            jobs: Some(teams.0.save_state()),
            ..Default::default()
        }),
        infrastructure: Some(
            world
                .get::<crate::infrastructure::InfrastructureCondition>(entity)
                .unwrap()
                .0
                .clone(),
        ),
        power: Some(
            world
                .get::<crate::ship::power::ShipPowerSystem>(entity)
                .unwrap()
                .0
                .capture_continuation(),
        ),
        ..Default::default()
    };
    world.entity_mut(entity).insert((state, teams));
    (world, entity, row)
}

#[test]
fn carried_crossing_json_preflight_preserves_frontier_and_refuses_forged_room_or_stage_read_only() {
    let (world, entity, row) = fixture();
    let profile = &world.get::<CompiledShipFitting>(entity).unwrap().0;
    let decoded: EntityState = serde_json::from_str(&serde_json::to_string(&row).unwrap()).unwrap();
    validate_state(profile, decoded.fitting.as_ref().unwrap(), &decoded, 1).unwrap();
    fitted_power(profile, decoded.fitting.as_ref().unwrap(), &decoded).unwrap();
    let before = world.get::<ShipFitting>(entity).unwrap().clone();
    let before_jobs = world.get::<ShipRepairTeams>(entity).unwrap().0.save_state();
    for mode in 0..5 {
        let mut bad = decoded.clone();
        let state = bad.fitting.as_mut().unwrap();
        let work = state.work.get_mut("fore-beam").unwrap();
        match mode {
            0 => {
                if let EquipmentLocation::InWork { compartment, .. } = &mut state
                    .equipment
                    .assemblies
                    .get_mut(&work.item)
                    .unwrap()
                    .location
                {
                    *compartment = "weapons".into();
                }
            }
            1 => work.stage = RefitStage::Removing,
            2 => work.source_locker = "absent-equipment".into(),
            3 => {
                if let crate::jobs::repair::RepairActivity::Refit {
                    destination_compartment,
                    ..
                } = &mut bad.repair.as_mut().unwrap().jobs.as_mut().unwrap().teams[0]
                    .as_mut()
                    .unwrap()
                    .activity
                {
                    *destination_compartment = Some("support".into());
                }
            }
            _ => work.reserved = true,
        }
        assert!(
            validate_state(profile, bad.fitting.as_ref().unwrap(), &bad, 1).is_err(),
            "mode {mode}"
        );
        assert_eq!(world.get::<ShipFitting>(entity).unwrap(), &before);
        assert_eq!(
            world.get::<ShipRepairTeams>(entity).unwrap().0.save_state(),
            before_jobs
        );
    }
}

use super::*;
fn state(world: &World, actor: Entity) -> &phoenix_sim_gameplay::personnel::PersonnelState {
    world
        .resource::<PersonnelRoster>()
        .state(
            &world
                .get::<crate::entities::spawner::EntityUuid>(actor)
                .unwrap()
                .0,
        )
        .unwrap()
}
use crate::core::messages::{AdmittedCommand, SystemId};
use crate::ship::control_source::ControlSource;
use bevy::ecs::system::RunSystemOnce;

fn fixture() -> (World, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/personnel_cruiser.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(12));
    let ship = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "personnel-owner".into(),
        None,
    );
    world.flush();
    world.entity_mut(ship).insert(AdmittedCommands::default());
    (world, ship)
}
fn issue(world: &mut World, ship: Entity, target: &str, team: Option<u32>) {
    world.get_mut::<AdmittedCommands>(ship).unwrap().0 = vec![AdmittedCommand {
        target: SystemId(target.into()),
        payload: SystemControlPayload::SetDutyOfficerPosting {
            officer: "repair-chief".into(),
            team_idx: team,
        },
        response_token: None,
        feedback_correlation: None,
    }];
    world.run_system_once(apply).unwrap();
    world.get_mut::<AdmittedCommands>(ship).unwrap().0.clear();
}

#[test]
fn actual_admitted_human_and_ai_posting_share_one_custody_without_payment_or_movement() {
    for source in [ControlSource::Human, ControlSource::Ai] {
        let (mut world, ship) = fixture();
        world
            .get_mut::<ShipSystemControlSources>(ship)
            .unwrap()
            .0
            .set(SystemId("repair".into()), source);
        let jobs = world
            .get::<super::super::server::ShipRepairTeams>(ship)
            .unwrap()
            .0
            .save_state();
        let power = world
            .get::<crate::ship::power::ShipPowerSystem>(ship)
            .unwrap()
            .0
            .capture_continuation();
        let stock = world
            .get::<crate::infrastructure::InfrastructureCondition>(ship)
            .unwrap()
            .clone();
        issue(&mut world, ship, "repair", Some(0));
        let posted = state(&world, ship).clone();
        assert_eq!(
            posted.officers["repair-chief"].location,
            phoenix_model::personnel::OfficerLocation::WithRepairTeam { team_idx: 0 }
        );
        assert_eq!(posted.officers["repair-chief"].changed_tick, 12);
        world.resource_mut::<crate::sim_tick::SimTick>().0 = 13;
        issue(&mut world, ship, "repair", Some(0));
        assert_eq!(state(&world, ship), &posted);
        assert_eq!(
            world
                .get::<super::super::server::ShipRepairTeams>(ship)
                .unwrap()
                .0
                .save_state(),
            jobs
        );
        assert_eq!(
            world
                .get::<crate::ship::power::ShipPowerSystem>(ship)
                .unwrap()
                .0
                .capture_continuation(),
            power
        );
        assert_eq!(
            world
                .get::<crate::infrastructure::InfrastructureCondition>(ship)
                .unwrap(),
            &stock
        );
        issue(&mut world, ship, "repair", None);
        assert_eq!(
            state(&world, ship).officers["repair-chief"].location,
            phoenix_model::personnel::OfficerLocation::AtCompartment {
                compartment: "engineering".into()
            }
        );
    }
}

#[test]
fn unavailable_or_away_actual_team_cannot_teleport_custody() {
    for offline in [true, false] {
        let (mut world, ship) = fixture();
        world
            .get_mut::<ShipSystemControlSources>(ship)
            .unwrap()
            .0
            .set(
                SystemId("repair".into()),
                if offline {
                    ControlSource::Offline
                } else {
                    ControlSource::Human
                },
            );
        if !offline {
            world
                .get_mut::<super::super::server::ShipRepairTeams>(ship)
                .unwrap()
                .0
                .dispatch(0, SystemId("helm-engine-port".into()), "Port engine".into());
        }
        let before = state(&world, ship).clone();
        let jobs = world
            .get::<super::super::server::ShipRepairTeams>(ship)
            .unwrap()
            .0
            .save_state();
        issue(&mut world, ship, "repair", Some(0));
        assert_eq!(state(&world, ship), &before);
        assert_eq!(
            world
                .get::<super::super::server::ShipRepairTeams>(ship)
                .unwrap()
                .0
                .save_state(),
            jobs
        );
    }
}

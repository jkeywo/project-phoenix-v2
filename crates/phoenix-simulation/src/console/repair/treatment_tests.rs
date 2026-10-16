//! Actual accepted care machinery adapter; full host/device journeys are separate.
use super::*;
use crate::{infrastructure::InfrastructureCondition, ship::power::ShipPowerSystem};
use bevy::ecs::system::RunSystemOnce;
use phoenix_sim_gameplay::personnel::{CompiledPersonnel, PersonnelCondition, PersonnelRoster};
#[test]
fn actual_human_and_ai_care_share_finite_team_paid_equipment_medication_and_return() {
    for source in [
        crate::ship::control_source::ControlSource::Human,
        crate::ship::control_source::ControlSource::Ai,
    ] {
        let config = crate::entities::include_resolve::load_entity_config(
            "assets/entities/medical_cruiser.toml",
        )
        .unwrap();
        let mut app = App::new();
        let mut time: Time = Time::default();
        time.advance_by(std::time::Duration::from_millis(200));
        app.insert_resource(time);
        app.insert_resource(crate::sim_tick::SimTick(0));
        let entity = crate::entities::spawner::spawn_entity(
            &mut app.world_mut().commands(),
            &config,
            Vec3::ZERO,
            "medical-actor".into(),
            None,
        );
        app.world_mut().flush();
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(entity)
            .unwrap()
            .0
            .set(repair_system_id(), source);
        let initial_power = app
            .world()
            .get::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .network()
            .consumers[&repair_system_id()]
            .energy;
        let initial_stock = app
            .world()
            .get::<InfrastructureCondition>(entity)
            .unwrap()
            .0
            .capacity_stock("medical-supplies")
            .unwrap()
            .level;
        let profile = app
            .world()
            .get::<CompiledPersonnel>(entity)
            .unwrap()
            .clone();
        app.world_mut()
            .entity_mut(entity)
            .insert(crate::core::messages::AdmittedCommands(vec![
                crate::core::messages::AdmittedCommand {
                    target: repair_system_id(),
                    payload: crate::core::messages::SystemControlPayload::TreatDutyOfficer {
                        officer: phoenix_model::personnel::PersonRef::new(
                            "medical-actor",
                            "repair-chief",
                        ),
                        team_idx: 1,
                    },
                    response_token: None,
                    feedback_correlation: None,
                },
            ]));
        app.world_mut()
            .run_system_once(super::super::personnel::apply)
            .unwrap();
        app.world_mut()
            .get_mut::<crate::core::messages::AdmittedCommands>(entity)
            .unwrap()
            .0
            .clear();
        assert!(app
            .world()
            .get::<ShipRepairTeams>(entity)
            .unwrap()
            .0
            .is_treatment_team(1));
        let id = app
            .world()
            .get::<ShipRepairTeams>(entity)
            .unwrap()
            .0
            .work_list()[0]
            .id;
        let mut was_working = false;
        let mut was_returning = false;
        for tick in 1..=120 {
            app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = tick;
            app.world_mut().run_system_once(tick_repair_teams).unwrap();
            let jobs = &app.world().get::<ShipRepairTeams>(entity).unwrap().0;
            let job = jobs
                .work_list()
                .into_iter()
                .find(|job| job.id == id)
                .unwrap();
            was_working |= job.phase == phoenix_model::job::JobPhase::Working;
            was_returning |= job.phase == phoenix_model::job::JobPhase::Returning;
            if tick <= 10 {
                assert_eq!(
                    app.world()
                        .get::<InfrastructureCondition>(entity)
                        .unwrap()
                        .0
                        .capacity_stock("medical-supplies")
                        .unwrap()
                        .level,
                    initial_stock
                );
            }
            if job.outcome.is_some() {
                break;
            }
        }
        let teams = &app.world().get::<ShipRepairTeams>(entity).unwrap().0;
        let state = app
            .world()
            .resource::<PersonnelRoster>()
            .state("medical-actor")
            .unwrap();
        assert!(was_working && was_returning);
        assert_eq!(
            state.officers["repair-chief"].condition,
            PersonnelCondition::Fit
        );
        assert_eq!(teams.work_list()[0].id, id);
        assert_eq!(
            teams.work_list()[0].outcome,
            Some(phoenix_model::job::JobOutcome::Completed)
        );
        state
            .validate(
                &profile,
                teams.job_state(),
                "medical-actor",
                app.world().resource::<crate::sim_tick::SimTick>().0,
            )
            .unwrap();
        assert_eq!(
            app.world()
                .get::<InfrastructureCondition>(entity)
                .unwrap()
                .0
                .capacity_stock("medical-supplies")
                .unwrap()
                .level,
            initial_stock - 1
        );
        assert!(
            app.world()
                .get::<ShipPowerSystem>(entity)
                .unwrap()
                .0
                .network()
                .consumers[&repair_system_id()]
                .energy
                < initial_power
        );
    }
}

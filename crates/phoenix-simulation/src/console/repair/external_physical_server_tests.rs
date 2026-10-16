use super::*;
use crate::resources::server::ResourceServices;
use crate::ship::compartment_damage::ShipDamage;
use crate::ship::power::ShipPowerSystem;
use bevy::ecs::system::RunSystemOnce;
use std::time::Duration;

fn source(app: &mut App, name: &str) -> Entity {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let damage = ShipDamage::from_config(&config).unwrap().unwrap();
    let power = ShipPowerSystem(
        crate::modifiers::power_system::PowerSystem::from_energy(
            config.energy.clone().unwrap(),
            config.ship_config.clone().unwrap(),
        )
        .unwrap(),
    );
    let mut teams = crate::jobs::repair::RepairTeams::new(1);
    assert!(teams.claim_external(0, "facility".into()));
    app.world_mut()
        .spawn((
            EntityUuid(name.into()),
            Transform::default(),
            record(),
            ShipRepairTeams(teams),
            power,
            InfrastructureCondition(
                crate::infrastructure::condition::InfrastructureState::from_config(
                    config.infrastructure.as_ref().unwrap(),
                ),
            ),
            ResourceServices(config.resource_services),
            damage,
        ))
        .id()
}
fn target(
    app: &mut App,
    break_component: bool,
) -> (
    Entity,
    String,
    f64,
    Option<phoenix_model::damage::ComponentRef>,
) {
    let config =
        crate::entities::include_resolve::load_entity_config("assets/entities/station_axiom.toml")
            .unwrap();
    let mut damage = ShipDamage::from_config(&config).unwrap().unwrap();
    let room = damage
        .definition
        .interior
        .compartments
        .iter()
        .min_by_key(|room| &room.id)
        .unwrap();
    let room_id = room.id.clone();
    let full = room.max_hp as f64;
    let amount = damage.definition.config.remedies[&room.structural_remedy]
        .structural_hp
        .unwrap() as f64;
    damage.state.compartments.get_mut(&room_id).unwrap().hp = (full - amount).max(0.0);
    let component = break_component.then(|| {
        let (system, id) = damage
            .definition
            .components
            .iter()
            .flat_map(|(system, parts)| {
                parts
                    .iter()
                    .filter(|(_, part)| part.field_replaceable)
                    .map(move |(id, _)| (system, id))
            })
            .next()
            .unwrap();
        let reference = phoenix_model::damage::ComponentRef {
            system: system.clone(),
            component: id.clone(),
        };
        let row = damage
            .state
            .components
            .get_mut(system)
            .unwrap()
            .get_mut(id)
            .unwrap();
        row.condition = phoenix_model::damage::ComponentCondition::Destroyed;
        row.last_source = Some("external-test-hit".into());
        reference
    });
    let condition = InfrastructureCondition(
        crate::infrastructure::condition::InfrastructureState::from_config(
            config.infrastructure.as_ref().unwrap(),
        ),
    );
    let entity = app
        .world_mut()
        .spawn((
            EntityUuid("facility".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            damage,
            condition,
        ))
        .id();
    (entity, room_id, full, component)
}
fn app() -> App {
    let mut app = App::new();
    let mut time: Time = Time::default();
    time.advance_by(Duration::from_millis(100));
    app.insert_resource(time);
    app
}
fn reserved(app: &App, entity: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacities()
        .iter()
        .flat_map(|capacity| capacity.stock.reserved.values())
        .sum()
}
fn level(app: &App, entity: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacities()
        .iter()
        .map(|capacity| capacity.stock.level)
        .sum()
}

#[test]
fn actual_external_owner_restores_facility_structure_using_source_parts_only() {
    let mut app = app();
    let source = source(&mut app, "source-a");
    let (target, room, full, component) = target(&mut app, true);
    let before = level(&app, source);
    let condition = app
        .world()
        .get::<InfrastructureCondition>(target)
        .unwrap()
        .0
        .condition();
    for _ in 0..1000 {
        app.world_mut()
            .run_system_once(apply_external_repair)
            .unwrap();
        if app
            .world()
            .get::<ShipRepairTeams>(source)
            .unwrap()
            .0
            .external_target()
            .is_none()
        {
            break;
        }
    }
    assert_eq!(
        app.world()
            .get::<ShipDamage>(target)
            .unwrap()
            .state
            .compartments[&room]
            .hp,
        full
    );
    assert!(level(&app, source) < before);
    let component = component.unwrap();
    assert_eq!(
        app.world()
            .get::<ShipDamage>(target)
            .unwrap()
            .state
            .components[&component.system][&component.component]
            .condition,
        phoenix_model::damage::ComponentCondition::Healthy,
        "the same no-own-team target also receives actual component restoration"
    );
    assert_eq!(reserved(&app, source), 0);
    assert!(
        app.world().get::<ShipRepairTeams>(target).is_none(),
        "no target-side crew pool is created"
    );
    assert_eq!(
        app.world()
            .get::<InfrastructureCondition>(target)
            .unwrap()
            .0
            .condition(),
        condition,
        "physical repair does not also heal the obsolete condition track"
    );
}

#[test]
fn parallel_sources_share_target_claims_and_range_loss_releases_unconsumed_parts() {
    let mut app = app();
    let first = source(&mut app, "source-a");
    let second = source(&mut app, "source-b");
    let (target, room, _, _) = target(&mut app, false);
    let before = level(&app, first) + level(&app, second);
    app.world_mut()
        .run_system_once(apply_external_repair)
        .unwrap();
    assert!(reserved(&app, first) > 0);
    assert_eq!(
        reserved(&app, second),
        0,
        "second crew cannot reserve the same target stage"
    );
    let hp = app
        .world()
        .get::<ShipDamage>(target)
        .unwrap()
        .state
        .compartments[&room]
        .hp;
    app.world_mut()
        .get_mut::<Transform>(target)
        .unwrap()
        .translation
        .x = 700.0;
    app.world_mut()
        .run_system_once(tick_external_repair)
        .unwrap();
    app.world_mut()
        .run_system_once(apply_external_repair)
        .unwrap();
    assert_eq!(reserved(&app, first) + reserved(&app, second), 0);
    assert_eq!(level(&app, first) + level(&app, second), before);
    assert_eq!(
        app.world()
            .get::<ShipDamage>(target)
            .unwrap()
            .state
            .compartments[&room]
            .hp,
        hp
    );
}

fn source_state(app: &App, entity: Entity) -> serde_json::Value {
    serde_json::json!({
        "teams": app.world().get::<ShipRepairTeams>(entity).unwrap().0.save_state(),
        "power": app.world().get::<ShipPowerSystem>(entity).unwrap().0.capture_continuation(),
        "inventory": app.world().get::<InfrastructureCondition>(entity).unwrap().0,
        "damage": app.world().get::<ShipDamage>(entity).unwrap().state,
    })
}

#[test]
fn idle_external_application_preserves_an_internal_return_without_rewriting_owners() {
    let mut app = app();
    app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
    let source = source(&mut app, "idle-source");
    {
        let mut teams = app.world_mut().get_mut::<ShipRepairTeams>(source).unwrap();
        assert!(teams.0.release_external());
        teams.0.dispatch(
            0,
            crate::core::messages::SystemId("local-work".into()),
            "local-work".into(),
        );
        assert!(teams.0.recall(0));
        assert!(matches!(&teams.0.slots()[0], TeamSlot::Returning { .. }));
        assert!(teams.0.job_state().external_physical_work.is_empty());
    }
    let before = source_state(&app, source);
    app.world_mut().clear_trackers();
    app.world_mut()
        .run_system_once(apply_external_repair)
        .unwrap();
    assert_eq!(source_state(&app, source), before);
    macro_rules! unchanged {
        ($owner:ty) => {
            assert!(
                !app.world()
                    .entity(source)
                    .get_ref::<$owner>()
                    .unwrap()
                    .is_changed(),
                "idle external work must not rewrite {}",
                stringify!($owner)
            );
        };
    }
    unchanged!(ShipRepairTeams);
    unchanged!(ShipPowerSystem);
    unchanged!(InfrastructureCondition);
    assert!(app
        .world()
        .resource::<EffectQueue<TaskLifecycleRequest>>()
        .0
        .is_empty());
}

#[test]
fn dormant_source_does_not_change_active_work_payments_or_competing_claims() {
    fn rig(with_idle: bool) -> (App, Entity, Entity, Entity) {
        let mut app = app();
        if with_idle {
            let idle = source(&mut app, "aaa-idle");
            assert!(app
                .world_mut()
                .get_mut::<ShipRepairTeams>(idle)
                .unwrap()
                .0
                .release_external());
        }
        let first = source(&mut app, "source-a");
        let second = source(&mut app, "source-b");
        let target = target(&mut app, true).0;
        (app, first, second, target)
    }
    let (mut baseline, first, second, target) = rig(false);
    let (mut extra, extra_first, extra_second, extra_target) = rig(true);
    let initial = level(&baseline, first) + level(&baseline, second);
    let initial_target = baseline
        .world()
        .get::<ShipDamage>(target)
        .unwrap()
        .state
        .clone();
    for step in 0..1000 {
        baseline
            .world_mut()
            .run_system_once(apply_external_repair)
            .unwrap();
        extra
            .world_mut()
            .run_system_once(apply_external_repair)
            .unwrap();
        assert_eq!(
            source_state(&baseline, first),
            source_state(&extra, extra_first),
            "first {step}"
        );
        assert_eq!(
            source_state(&baseline, second),
            source_state(&extra, extra_second),
            "second {step}"
        );
        assert_eq!(
            baseline.world().get::<ShipDamage>(target).unwrap().state,
            extra.world().get::<ShipDamage>(extra_target).unwrap().state,
            "target {step}"
        );
        if step == 0 {
            assert!(reserved(&baseline, first) > 0);
            assert_eq!(
                reserved(&baseline, second),
                0,
                "ordinary competing claims remain exclusive"
            );
        }
        if baseline
            .world()
            .get::<ShipRepairTeams>(first)
            .unwrap()
            .0
            .external_target()
            .is_none()
            && baseline
                .world()
                .get::<ShipRepairTeams>(second)
                .unwrap()
                .0
                .external_target()
                .is_none()
        {
            break;
        }
    }
    for source in [first, second] {
        assert!(
            baseline
                .world()
                .get::<ShipRepairTeams>(source)
                .unwrap()
                .0
                .external_target()
                .is_none(),
            "both physical jobs must finish within the existing owner-test budget"
        );
    }
    assert!(level(&baseline, first) + level(&baseline, second) < initial);
    assert_ne!(
        baseline.world().get::<ShipDamage>(target).unwrap().state,
        initial_target
    );
    assert_eq!(reserved(&baseline, first) + reserved(&baseline, second), 0);
}

#[test]
fn recalled_and_lost_target_stages_still_release_parts_once_after_external_job_closes() {
    for lost in [false, true] {
        let mut app = app();
        app.init_resource::<EffectQueue<TaskLifecycleRequest>>();
        let source = source(&mut app, "source-a");
        let target = target(&mut app, false).0;
        let initial = level(&app, source);
        app.world_mut()
            .run_system_once(apply_external_repair)
            .unwrap();
        assert!(reserved(&app, source) > 0);
        if lost {
            assert!(app.world_mut().despawn(target));
            app.world_mut()
                .run_system_once(tick_external_repair)
                .unwrap();
        } else {
            app.world_mut()
                .entity_mut(source)
                .insert(AdmittedCommands(vec![correlated_external_command(
                    "cleanup-recall",
                    SystemControlPayload::RecallExternalRepair,
                )]));
            app.world_mut()
                .run_system_once(handle_external_repair_commands)
                .unwrap();
        }
        let teams = &app.world().get::<ShipRepairTeams>(source).unwrap().0;
        assert!(teams.external_target().is_none());
        assert!(
            !teams.job_state().external_physical_work.is_empty(),
            "closed Job still has a reserved stage"
        );
        let notices = std::mem::take(
            &mut app
                .world_mut()
                .resource_mut::<EffectQueue<TaskLifecycleRequest>>()
                .0,
        );
        assert_eq!(
            notices.len(),
            1,
            "normal recall/maintenance emits exactly one terminal notice"
        );
        let power = app
            .world()
            .get::<ShipPowerSystem>(source)
            .unwrap()
            .0
            .capture_continuation();
        app.world_mut()
            .run_system_once(apply_external_repair)
            .unwrap();
        assert_eq!(reserved(&app, source), 0);
        assert_eq!(
            level(&app, source),
            initial,
            "unconsumed parts are released, not consumed/refunded twice"
        );
        assert!(app
            .world()
            .get::<ShipRepairTeams>(source)
            .unwrap()
            .0
            .job_state()
            .external_physical_work
            .is_empty());
        assert_eq!(
            app.world()
                .get::<ShipPowerSystem>(source)
                .unwrap()
                .0
                .capture_continuation(),
            power
        );
        let cleaned = source_state(&app, source);
        app.world_mut()
            .run_system_once(apply_external_repair)
            .unwrap();
        assert_eq!(source_state(&app, source), cleaned);
        assert!(app
            .world()
            .resource::<EffectQueue<TaskLifecycleRequest>>()
            .0
            .is_empty());
    }
}

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

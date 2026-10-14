//! Actual internal Repair adapter coverage. Registered as a child of server.
use super::*;
use crate::infrastructure::InfrastructureCondition;
use crate::ship::compartment_damage::ShipDamage;
use crate::ship::power::ShipPowerSystem;
use bevy::ecs::system::RunSystemOnce;
use phoenix_model::damage::{ComponentCondition, ComponentRef, DamageBinding, RepairWorkTarget};
use std::time::Duration;

struct Fixture {
    app: App,
    entity: Entity,
    owner: SystemId,
    target: ComponentRef,
    manual: f32,
}
fn fixture(tool_condition: ComponentCondition, irreparable: bool) -> Fixture {
    let mut config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let initial = ShipDamage::from_config(&config).unwrap().unwrap();
    let home = &config.interior.as_ref().unwrap().team_homes[0];
    let owner = config
        .ship_config
        .as_ref()
        .unwrap()
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::REPAIR_KIND)
        .unwrap()
        .id
        .clone();
    let (target_system, parts) = initial
        .definition
        .components
        .iter()
        .find(|(id, parts)| {
            *id != &owner
                && initial.definition.interior.placements.get(*id) == Some(home)
                && parts.len() > 1
                && parts.values().all(|part| part.field_replaceable)
        })
        .expect("shipped Engineering has other repairable equipment");
    let target = ComponentRef {
        system: target_system.clone(),
        component: parts.keys().next().unwrap().clone(),
    };
    if irreparable {
        config
            .ship_config
            .as_mut()
            .unwrap()
            .systems
            .iter_mut()
            .find(|system| system.id == target.system)
            .unwrap()
            .damage_components
            .iter_mut()
            .find(|part| part.id == target.component)
            .unwrap()
            .field_replaceable = false;
    }
    let mut app = App::new();
    let mut time: Time = Time::default();
    time.advance_by(Duration::from_millis(100));
    app.insert_resource(time);
    let entity = {
        let mut commands = app.world_mut().commands();
        crate::entities::spawner::spawn_entity(
            &mut commands,
            &config,
            Vec3::ZERO,
            "repair-actor".into(),
            None,
        )
    };
    app.world_mut().flush();
    assert!(app
        .world()
        .entity(entity)
        .contains::<crate::server_app::Ship>());
    let manual = {
        let mut damage = app.world_mut().get_mut::<ShipDamage>(entity).unwrap();
        let tool_part = damage.definition.components[&owner]
            .iter()
            .find(|(_, part)| part.binding == DamageBinding::Operation)
            .unwrap()
            .0
            .clone();
        let tool = damage
            .state
            .components
            .get_mut(&owner)
            .unwrap()
            .get_mut(&tool_part)
            .unwrap();
        tool.condition = tool_condition;
        if tool.condition != ComponentCondition::Healthy {
            tool.last_source = Some("tool-hit".into());
        }
        let part = damage
            .state
            .components
            .get_mut(&target.system)
            .unwrap()
            .get_mut(&target.component)
            .unwrap();
        part.condition = if irreparable {
            ComponentCondition::Destroyed
        } else {
            ComponentCondition::Damaged
        };
        part.last_source = Some("equipment-hit".into());
        damage.state.validate(&damage.definition).unwrap();
        let remedy = &damage.definition.components[&target.system][&target.component].full_remedy;
        damage.definition.config.remedies[remedy].manual_work_fraction
    };
    // A charged canonical reserve proves tool failure, rather than energy
    // starvation. Stocks and recipes remain the actual finite authored ones.
    {
        let mut power = app.world_mut().get_mut::<ShipPowerSystem>(entity).unwrap();
        let capacity = power
            .0
            .config()
            .consumers
            .iter()
            .find(|profile| profile.system == owner)
            .unwrap()
            .reserve
            .capacity;
        let reserve = power.0.network_mut().consumers.get_mut(&owner).unwrap();
        reserve.energy = capacity;
        reserve.heat = 0.0;
    }
    assert!(app
        .world_mut()
        .get_mut::<ShipRepairTeams>(entity)
        .unwrap()
        .0
        .dispatch_physical(
            0,
            target.system.clone(),
            Some(RepairWorkTarget::Component {
                system: target.system.clone(),
                component: target.component.clone()
            }),
            target.system.0.clone()
        ));
    app.world_mut().run_system_once(tick_repair_teams).unwrap();
    assert!(
        matches!(
            app.world()
                .get::<ShipRepairTeams>(entity)
                .unwrap()
                .0
                .slots()[0],
            TeamSlot::Repairing { .. }
        ),
        "actual route reaches an on-site work phase before measurement"
    );
    Fixture {
        app,
        entity,
        owner,
        target,
        manual,
    }
}
fn energy(f: &Fixture) -> f32 {
    f.app
        .world()
        .get::<ShipPowerSystem>(f.entity)
        .unwrap()
        .0
        .network()
        .consumers[&f.owner]
        .energy
}
fn level(f: &Fixture) -> i64 {
    f.app
        .world()
        .get::<InfrastructureCondition>(f.entity)
        .unwrap()
        .0
        .capacities()
        .iter()
        .map(|capacity| capacity.stock.level)
        .sum()
}
fn reserved(f: &Fixture) -> i64 {
    f.app
        .world()
        .get::<InfrastructureCondition>(f.entity)
        .unwrap()
        .0
        .capacities()
        .iter()
        .flat_map(|capacity| capacity.stock.reserved.values())
        .sum()
}
fn progress(f: &Fixture) -> f32 {
    f.app
        .world()
        .get::<ShipRepairTeams>(f.entity)
        .unwrap()
        .0
        .save_state()
        .physical_work
        .values()
        .find(|work| {
            work.target.as_ref()
                == Some(&RepairWorkTarget::Component {
                    system: f.target.system.clone(),
                    component: f.target.component.clone(),
                })
        })
        .unwrap()
        .progress
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.00001, "{a} != {b}");
}

#[test]
fn unavailable_repair_tools_make_manual_progress_without_spending_charged_energy() {
    let mut f = fixture(ComponentCondition::Destroyed, false);
    let before = energy(&f);
    let stores = level(&f);
    f.app
        .world_mut()
        .run_system_once(tick_repair_teams)
        .unwrap();
    close(progress(&f), 0.1 * f.manual);
    assert!(progress(&f) > 0.0);
    assert_eq!(
        energy(&f),
        before,
        "unavailable tools cannot spend charge for zero acceleration"
    );
    assert!(
        reserved(&f) > 0,
        "the stage holds actual finite authored parts"
    );
    assert_eq!(
        level(&f),
        stores,
        "a pending stage has not consumed its reservation yet"
    );
    assert!(
        f.app
            .world()
            .get::<ShipDamage>(f.entity)
            .unwrap()
            .state
            .components[&f.target.system][&f.target.component]
            .condition
            == ComponentCondition::Damaged
    );
}

#[test]
fn partial_repair_tools_pay_the_same_work_for_less_actual_acceleration() {
    let run = |condition| {
        let mut f = fixture(condition, false);
        let before = energy(&f);
        let factor = f
            .app
            .world()
            .get::<ShipDamage>(f.entity)
            .unwrap()
            .factor(&f.owner, DamageBinding::Operation);
        f.app
            .world_mut()
            .run_system_once(tick_repair_teams)
            .unwrap();
        assert!(reserved(&f) > 0);
        (progress(&f), before - energy(&f), f.manual, factor)
    };
    let intact = run(ComponentCondition::Healthy);
    let partial = run(ComponentCondition::Damaged);
    close(intact.0, 0.1);
    close(partial.0, 0.1 * (partial.2 + (1.0 - partial.2) * partial.3));
    assert!(partial.0 < intact.0 && partial.0 > 0.1 * partial.2);
    assert!(intact.1 > 0.0);
    close(partial.1, intact.1);
}

#[test]
fn on_site_priority_skips_irreparable_components_and_retains_repairable_siblings() {
    let mut f = fixture(ComponentCondition::Healthy, true);
    let options = |f: &Fixture| {
        super::super::dispatch::physical_priority_options(
            &f.app.world().get::<ShipRepairTeams>(f.entity).unwrap().0,
            f.app.world().get::<ShipDamage>(f.entity).unwrap(),
        )
    };
    assert!(
        !options(&f).iter().any(|(_, id, _)| id == &f.target.system),
        "an irreparable component alone is not a useful priority"
    );
    let sibling = {
        let mut damage = f.app.world_mut().get_mut::<ShipDamage>(f.entity).unwrap();
        let sibling = damage.definition.components[&f.target.system]
            .keys()
            .find(|name| *name != &f.target.component)
            .unwrap()
            .clone();
        let part = damage
            .state
            .components
            .get_mut(&f.target.system)
            .unwrap()
            .get_mut(&sibling)
            .unwrap();
        part.condition = ComponentCondition::Damaged;
        part.last_source = Some("sibling-hit".into());
        damage.state.validate(&damage.definition).unwrap();
        sibling
    };
    assert!(options(&f)
        .iter()
        .any(|(_, id, target)| id == &f.target.system
            && target
                == &RepairWorkTarget::Component {
                    system: f.target.system.clone(),
                    component: sibling.clone()
                }));
}

//! Increment 11 integration: ordinary repair AI, finite parts and physical faults.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use phoenix_model::{
    damage::{ComponentCondition, ComponentRef, DamageBinding, RepairWorkTarget},
    interior::TeamLocation,
};
use project_phoenix::{
    console::repair::ShipRepairTeams,
    core::messages::{ModifierSlot, SystemId},
    entities::loader::{resolve_entity_via, FsTemplateLoader},
    headless::{build_headless_app, HeadlessArgs},
    infrastructure::InfrastructureCondition,
    modifiers::ShipModifiers,
    server_app::LocalShip,
    ship::compartment_damage::ShipDamage,
    sim_digest::world_digest,
    sim_tick::{sim_tick_period, SimTick},
    snapshot::{capture, ready_to_restore, restore, PhoenixSnapshot},
    world::config::{parse_world, WorldConfig},
};
use std::{collections::HashMap, path::Path};

fn boot() -> App {
    let args = HeadlessArgs {
        world_path: "tests/fixtures/physical-route-world.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        deterministic: true,
        seed: Some(156_210),
        ..Default::default()
    };
    let mut app = build_headless_app(&args).unwrap();
    let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    for _ in 0..30 {
        app.update();
    }
    app
}

fn local(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}

fn fault(app: &App, entity: Entity, target: &ComponentRef) -> ComponentCondition {
    app.world()
        .get::<ShipDamage>(entity)
        .unwrap()
        .state
        .components[&target.system][&target.component]
        .condition
        .clone()
}

fn parts(app: &App, entity: Entity) -> i64 {
    app.world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacities()
        .iter()
        .find(|capacity| capacity.id == "machinery_parts")
        .unwrap()
        .stock
        .level
}

fn damage_operation(app: &mut App, entity: Entity) -> (ComponentRef, f32, f32, i64) {
    let target = ComponentRef {
        system: SystemId("phaser-fore".into()),
        component: "operation".into(),
    };
    let mut damage = app.world_mut().get_mut::<ShipDamage>(entity).unwrap();
    let component = damage.definition.component(&target).unwrap();
    assert_eq!(component.binding, DamageBinding::Operation);
    assert!(component.field_replaceable);
    let recipe = &damage.definition.config.remedies[&component.full_remedy];
    let duration = recipe.work_secs;
    let manual = recipe.manual_work_fraction;
    let cost = recipe
        .requirements
        .iter()
        .filter(|part| part.resource == "machinery_parts")
        .map(|part| part.amount)
        .sum();
    assert!(cost > 0);
    let state = damage
        .state
        .components
        .get_mut(&target.system)
        .unwrap()
        .get_mut(&target.component)
        .unwrap();
    state.condition = ComponentCondition::Destroyed;
    state.last_source = Some("physical-route-integration".into());
    (target, duration, manual, cost)
}

fn manual_tick_budget(app: &App, entity: Entity, tick_secs: f32, manual: f32) -> f32 {
    tick_secs
        * app
            .world()
            .get::<ShipModifiers>(entity)
            .unwrap()
            .get(&ModifierSlot::RepairRate)
        * manual
}

fn progress(app: &App, entity: Entity, target: &ComponentRef) -> f32 {
    app.world()
        .get::<ShipRepairTeams>(entity)
        .unwrap()
        .0
        .save_state()
        .physical_work
        .values()
        .filter(|work| {
            work.target.as_ref()
                == Some(&RepairWorkTarget::Component {
                    system: target.system.clone(),
                    component: target.component.clone(),
                })
        })
        .map(|work| work.progress)
        .fold(0.0, f32::max)
}

fn frontier(app: &App, entity: Entity) -> serde_json::Value {
    let uuid = &app
        .world()
        .get::<project_phoenix::entities::spawner::EntityUuid>(entity)
        .unwrap()
        .0;
    let state = capture(app.world());
    let row = state.entities.iter().find(|row| &row.uuid == uuid).unwrap();
    serde_json::json!({"repair":row.repair,"access":row.access,"damage":row.physical_damage,"inventory":row.infrastructure})
}

#[test]
fn ordinary_ai_crosses_real_rooms_and_resumes_mid_route_into_actual_repair() {
    let mut live = boot();
    let entity = local(&mut live);
    let target = SystemId("phaser-fore".into());
    let teams = &live.world().get::<ShipRepairTeams>(entity).unwrap().0;
    let layout = teams.layout().unwrap();
    let target_room = &layout.placements[&target];
    assert!(layout.team_homes.iter().all(|home| home != target_room));
    assert!(teams
        .slots()
        .iter()
        .all(|slot| matches!(slot, project_phoenix::core::messages::TeamSlot::Idle)));
    let initial_parts = parts(&live, entity);
    let (component, duration, manual, cost) = damage_operation(&mut live, entity);
    assert!(initial_parts >= cost);
    // No direct job requests, controller calls, observations or team tick calls:
    // the installed ordinary AI must consume its actual reports and dispatch.
    let mut crossed = false;
    for _ in 0..600 {
        live.update();
        let teams = &live.world().get::<ShipRepairTeams>(entity).unwrap().0;
        if teams.navigation().unwrap().iter().any(|team| {
            matches!(&team.location,
            TeamLocation::Crossing { elapsed, .. } if *elapsed > 0.0)
        }) {
            crossed = true;
            break;
        }
    }
    assert!(
        crossed,
        "ordinary AI must actually enter an authored connection"
    );
    assert_eq!(
        fault(&live, entity, &component),
        ComponentCondition::Destroyed,
        "travel cannot remedy a remote component"
    );
    let saved: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(live.world())).unwrap()).unwrap();
    let mut resumed = boot();
    let restored_entity = local(&mut resumed);
    assert!(ready_to_restore(resumed.world(), &saved));
    assert_ne!(
        frontier(&resumed, restored_entity),
        frontier(&live, entity),
        "restore replaces a genuinely different bootstrap"
    );
    let result = restore(resumed.world_mut(), &saved);
    assert!(result.is_complete(), "{:?}", result.gaps);
    let tick_secs =
        sim_tick_period(live.world().resource::<WorldConfig>().global.sim_tick_hz).as_secs_f32();
    let mut funded_progress = false;
    // Retain the original 360-tick complete-world comparison, then allow the
    // authored manual-work worst case to finish the actual remedy and debit.
    // Component conditions are discrete; positive work replaces fractional HP.
    let remaining_bound = 360.max((duration / manual / tick_secs).ceil() as usize + 360);
    for step in 0..remaining_bound {
        assert_eq!(
            world_digest(live.world()),
            world_digest(resumed.world()),
            "whole-world continuation at step {step}"
        );
        assert_eq!(
            frontier(&live, entity),
            frontier(&resumed, restored_entity),
            "canonical route, job, parts, physical and access continuation at step {step}"
        );
        let before = progress(&live, entity, &component);
        let manual_budget = manual_tick_budget(&live, entity, tick_secs, manual);
        let tick = live.world().resource::<SimTick>().0;
        live.update();
        resumed.update();
        let after = progress(&live, entity, &component);
        funded_progress |= after - before > manual_budget * 1.5;
        assert_eq!(live.world().resource::<SimTick>().0, tick + 1);
        assert_eq!(resumed.world().resource::<SimTick>().0, tick + 1);
    }
    assert!(
        funded_progress,
        "ordinary on-site repair must advance faster than unpowered manual work"
    );
    assert_eq!(
        fault(&live, entity, &component),
        ComponentCondition::Healthy
    );
    assert_eq!(
        fault(&resumed, restored_entity, &component),
        ComponentCondition::Healthy
    );
    assert_eq!(
        initial_parts - parts(&live, entity),
        cost,
        "exactly one actual remedy consumes its finite parts"
    );
    assert_eq!(parts(&live, entity), parts(&resumed, restored_entity));
}

fn worlds_under(path: &Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            worlds_under(&path, found);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            found.push(path);
        }
    }
}

#[test]
fn all_shipped_world_entity_overrides_retain_valid_physical_ownership() {
    let mut worlds = Vec::new();
    worlds_under(Path::new("assets/worlds"), &mut worlds);
    worlds.sort();
    let (mut entities, mut overrides, mut layouts, mut zero_team_layouts) = (0, 0, 0, 0);
    let mut resolution_errors = Vec::new();
    for path in &worlds {
        let world = parse_world(&std::fs::read_to_string(path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        for entry in &world.entities {
            let config = match resolve_entity_via(entry, &HashMap::new(), &FsTemplateLoader) {
                Ok(config) => config,
                Err(error) => {
                    resolution_errors.push(format!(
                        "{} / {}: {error}",
                        path.display(),
                        entry.template_path
                    ));
                    continue;
                }
            };
            entities += 1;
            overrides += usize::from(entry.overrides.is_some());
            let Some(ship) = config
                .ship_config
                .as_ref()
                .filter(|ship| !ship.systems.is_empty())
            else {
                continue;
            };
            let geometry = config
                .interior
                .as_ref()
                .expect("strict resolver must retain mandatory geometry");
            let count = config
                .repair
                .as_ref()
                .map_or(0, |repair| repair.repair_team_count as usize);
            assert!(
                config.damage.is_some(),
                "equipped overrides must retain actual damage definitions"
            );
            assert_eq!(geometry.placements.len(), ship.systems.len());
            assert_eq!(geometry.team_homes.len(), count);
            layouts += 1;
            zero_team_layouts += usize::from(count == 0);
        }
    }
    assert!(
        !worlds.is_empty() && entities > 0 && overrides > 0 && layouts > 0 && zero_team_layouts > 0
    );
    assert!(
        resolution_errors.is_empty(),
        "every shipped override must resolve:\n{}",
        resolution_errors.join("\n")
    );
    println!("physical corpus: {} worlds, {entities} entity instances, {overrides} overrides, {layouts} layouts, {zero_team_layouts} zero-team layouts", worlds.len());
}

fn mesh_step(hosts: &mut [App; 2]) {
    use project_phoenix::lockstep::{MeshInbox, MeshOutbox};
    let frames = hosts
        .each_mut()
        .map(|app| app.world_mut().resource_mut::<MeshOutbox>().drain());
    for index in 0..2 {
        for frame in &frames[1 - index] {
            hosts[index]
                .world_mut()
                .resource_mut::<MeshInbox>()
                .push(frame.clone());
        }
    }
    for host in hosts.iter_mut() {
        host.update();
    }
    assert_eq!(
        hosts[0].world().resource::<SimTick>().0,
        hosts[1].world().resource::<SimTick>().0
    );
    assert_eq!(
        world_digest(hosts[0].world()),
        world_digest(hosts[1].world())
    );
}

#[test]
fn ordinary_ai_physical_routes_match_across_different_local_ship_peers() {
    use project_phoenix::{
        command_admission::HostSlot,
        entities::spawner::EntityUuid,
        lockstep::{authored_delay, join_fleet, FleetRoster, FleetShip, FleetSlotOf},
    };
    let mut hosts = [HostSlot(1), HostSlot(2)].map(|local_slot| {
        let mut app = build_headless_app(&HeadlessArgs {
            world_path: "tests/fixtures/physical-fleet-world.toml".into(),
            ship_path: "assets/entities/alliance_cruiser.toml".into(),
            deterministic: true,
            seed: Some(156_210),
            ..Default::default()
        })
        .unwrap();
        let roster = FleetRoster::new(
            [HostSlot(1), HostSlot(2)]
                .into_iter()
                .map(|host| FleetShip {
                    host,
                    ship_path: Some("assets/entities/alliance_cruiser.toml".into()),
                    authored_slot_id: None,
                    crew: vec![],
                })
                .collect(),
            local_slot,
        );
        let delay = authored_delay(app.world());
        assert!(join_fleet(app.world_mut(), roster, delay));
        let period = sim_tick_period(app.world().resource::<WorldConfig>().global.sim_tick_hz);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
        app.finish();
        app.cleanup();
        app
    });
    for _ in 0..40 {
        mesh_step(&mut hosts);
    }
    let mut local_slots = Vec::new();
    let mut initial = std::collections::BTreeMap::new();
    for (index, app) in hosts.iter_mut().enumerate() {
        let local_entity = local(app);
        local_slots.push(app.world().get::<FleetSlotOf>(local_entity).unwrap().0);
        let actors: Vec<_> = app
            .world_mut()
            .query::<(Entity, &EntityUuid, &FleetSlotOf)>()
            .iter(app.world())
            .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
            .collect();
        assert_eq!(
            actors.len(),
            2,
            "both full fleet hulls must exist on each peer"
        );
        for (entity, uuid) in actors {
            let stock = parts(app, entity);
            let (component, _, manual, cost) = damage_operation(app, entity);
            if index == 0 {
                initial.insert(uuid, (component, stock, manual, cost));
            } else {
                assert_eq!(initial[&uuid], (component, stock, manual, cost));
            }
        }
    }
    assert_ne!(
        local_slots[0], local_slots[1],
        "hosts must project different LocalShip owners"
    );
    let mut crossing = std::collections::BTreeSet::new();
    let mut paid_work = std::collections::BTreeSet::new();
    let tick_secs = sim_tick_period(
        hosts[0]
            .world()
            .resource::<WorldConfig>()
            .global
            .sim_tick_hz,
    )
    .as_secs_f32();
    for _ in 0..720 {
        let prior: [std::collections::BTreeMap<_, _>; 2] = hosts.each_mut().map(|app| {
            let actors: Vec<_> = app
                .world_mut()
                .query::<(Entity, &EntityUuid, &FleetSlotOf)>()
                .iter(app.world())
                .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
                .collect();
            actors
                .into_iter()
                .map(|(entity, uuid)| {
                    let value = progress(app, entity, &initial[&uuid].0);
                    (uuid, value)
                })
                .collect()
        });
        mesh_step(&mut hosts);
        for (index, host) in hosts.iter_mut().enumerate() {
            let mut teams = host
                .world_mut()
                .query::<(Entity, &EntityUuid, &FleetSlotOf, &ShipRepairTeams)>();
            for (entity, uuid, _, teams) in teams.iter(host.world()) {
                if teams.0.navigation().unwrap().iter().any(|team| matches!(&team.location, TeamLocation::Crossing { elapsed, .. } if *elapsed > 0.0)) {
                    crossing.insert((index, uuid.0.clone()));
                }
                let (component, _, manual, _) = &initial[&uuid.0];
                if progress(host, entity, component) - prior[index][&uuid.0]
                    > manual_tick_budget(host, entity, tick_secs, *manual) * 1.5
                {
                    paid_work.insert((index, uuid.0.clone()));
                }
            }
        }
    }
    assert_eq!(
        crossing.len(),
        4,
        "both ordinary AIs must physically cross on both peers"
    );
    assert_eq!(
        paid_work.len(),
        4,
        "both crews must perform positive powered on-site work on both peers"
    );
    for app in &mut hosts {
        let actors: Vec<_> = app
            .world_mut()
            .query::<(Entity, &EntityUuid, &FleetSlotOf)>()
            .iter(app.world())
            .map(|(entity, uuid, _)| (entity, uuid.0.clone()))
            .collect();
        for (entity, uuid) in actors {
            let (component, stock, _, cost) = &initial[&uuid];
            assert_eq!(
                fault(app, entity, component),
                ComponentCondition::Healthy,
                "{uuid} must actually restore its component"
            );
            assert_eq!(
                *stock - parts(app, entity),
                *cost,
                "{uuid} must consume its own exact finite parts once"
            );
        }
    }
}

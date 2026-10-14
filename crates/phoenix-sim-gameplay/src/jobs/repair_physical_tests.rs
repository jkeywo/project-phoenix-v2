use super::*;
use crate::infrastructure::condition::{InfrastructureConfig, InfrastructureState};
use crate::resources::service::ResourceEndpoint;
use crate::ship::interior::{InteriorConfig, InteriorGraph};
use phoenix_model::damage::DamageConfig;
use serde_json::json;

fn fixture(
    parts: i64,
) -> (
    RepairTeams,
    DamageState,
    InfrastructureState,
    BTreeMap<String, ResourceEndpoint>,
) {
    let interior:InteriorConfig=serde_json::from_value(json!({
        "deck":[{"id":"main","label":"Main","damage_weight":1.0}],
        "compartment":[
            {"id":"front","deck":"main","label":"Front","x":0,"z":0,"width":2,"depth":2,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structure"},
            {"id":"rear","deck":"main","label":"Rear","x":2,"z":0,"width":2,"depth":2,"material":"steel","max_hp":100,"recovery_fraction":0.25,"structural_remedy":"structure"}],
        "connection":[{"id":"door","from":"front","to":"rear","travel_secs":2}],
        "placements":{"engine":"front","scanner":"rear"},"team_homes":["front","front"]
    })).unwrap();
    let config:DamageConfig=serde_json::from_value(json!({
        "materials":{"steel":{"resistance":{"physical":1,"laser":1},"residual_fraction":0.5}},
        "remedies":{
            "structure":{"work_secs":1,"manual_work_fraction":0.25,"structural_hp":10,"outcome":{"kind":"structural"},"requirements":[{"resource":"parts","service":"parts","amount":1}]},
            "full":{"work_secs":2,"manual_work_fraction":0.25,"outcome":{"kind":"full"},"requirements":[{"resource":"parts","service":"parts","amount":1}]},
            "patch":{"work_secs":1,"manual_work_fraction":0.25,"outcome":{"kind":"patch","factor":0.25},"requirements":[]}
        }
    })).unwrap();
    let component = json!({"id":"operation","binding":"operation","susceptibility":{"physical":1,"laser":1},"fault_dose_threshold":4,"degraded_factor":0.5,"full_remedy":"full","patch_remedy":"patch","field_replaceable":true});
    let ship: ShipConfig = serde_json::from_value(json!({"system":[
        {"id":"engine","kind":"helm_thrust","damage_component":[component.clone()]},
        {"id":"scanner","kind":"sensors","damage_component":[component]}
    ]}))
    .unwrap();
    let definition = DamageDefinition::compile(&interior, &config, &ship).unwrap();
    let state = DamageState::new(&definition);
    let mut teams = RepairTeams::new(2);
    teams.install_layout(InteriorGraph::compile(&interior, &ship, 2).unwrap());
    teams.install_damage_definition(definition);
    let inventory = InfrastructureState::from_config(
        &toml::from_str::<InfrastructureConfig>(&format!(
            "[[capacity]]\nid='parts'\namount={parts}\nceiling=100\n"
        ))
        .unwrap(),
    );
    (
        teams,
        state,
        inventory,
        BTreeMap::from([(
            "parts".into(),
            ResourceEndpoint::Capacity { id: "parts".into() },
        )]),
    )
}
fn advance(
    teams: &mut RepairTeams,
    state: &mut DamageState,
    stock: &mut InfrastructureState,
    services: &BTreeMap<String, ResourceEndpoint>,
    dt: f32,
) {
    teams.tick_physical(
        dt,
        state,
        &BTreeSet::new(),
        |action, id, recipe| stock.repair_parts(action, id, recipe, services).is_ok(),
        |_, _| 0.0,
    );
}

#[test]
fn unpowered_team_restores_its_room_with_real_finite_parts_and_parallel_component_work() {
    let (mut teams, mut damage, mut stock, services) = fixture(8);
    damage.compartments.get_mut("front").unwrap().hp = 0.0;
    damage.compartments.get_mut("front").unwrap().disabled = true;
    damage
        .components
        .get_mut(&SystemId("engine".into()))
        .unwrap()
        .get_mut("operation")
        .unwrap()
        .condition = ComponentCondition::Destroyed;
    teams.dispatch_physical(0, SystemId("engine".into()), None, "Engine".into());
    teams.dispatch_physical(1, SystemId("engine".into()), None, "Engine".into());
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    assert_eq!(
        damage.compartments["front"].hp, 0.0,
        "reservation is not completed work"
    );
    assert_eq!(
        stock.capacity_stock("parts").unwrap().level,
        8,
        "claims do not consume early"
    );
    assert_eq!(stock.capacity_stock("parts").unwrap().reserved.len(), 2);
    for _ in 0..48 {
        advance(&mut teams, &mut damage, &mut stock, &services, 1.0);
    }
    assert!(
        !damage.compartments["front"].disabled,
        "manual work can lift the zero-HP room latch"
    );
    assert!(damage.compartments["front"].hp > 0.0);
    assert_eq!(
        damage.components[&SystemId("engine".into())]["operation"].condition,
        ComponentCondition::Healthy
    );
    assert_eq!(
        stock.capacity_stock("parts").unwrap().level,
        0,
        "finite stores bound restoration"
    );
    assert!(teams.work_list().iter().any(|row| row.waiting_parts));
}

#[test]
fn redirect_releases_stage_claims_and_preserves_job_identity_and_crossing() {
    let (mut teams, mut damage, mut stock, services) = fixture(4);
    damage.compartments.get_mut("front").unwrap().hp = 50.0;
    teams.dispatch_physical(0, SystemId("engine".into()), None, "Engine".into());
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    let id = teams.work_list()[0].id;
    assert!(!stock.capacity_stock("parts").unwrap().reserved.is_empty());
    teams.dispatch_physical(0, SystemId("scanner".into()), None, "Scanner".into());
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    assert_eq!(teams.work_list()[0].id, id);
    assert!(stock.capacity_stock("parts").unwrap().reserved.is_empty());
    assert_eq!(stock.capacity_stock("parts").unwrap().level, 4);
    assert!(matches!(
        teams.navigation().unwrap()[0].location,
        crate::ship::interior::TeamLocation::Crossing { .. }
    ));
    teams.recall(0);
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    assert!(
        matches!(
            teams.navigation().unwrap()[0].location,
            crate::ship::interior::TeamLocation::Crossing { .. }
        ),
        "recall cannot teleport out of a committed crossing"
    );
}

#[test]
fn typed_tasks_are_exclusive_and_continue_with_exact_claim_and_work_progress() {
    let (mut teams, mut damage, mut stock, services) = fixture(4);
    damage.compartments.get_mut("front").unwrap().hp = 50.0;
    let target = Some(RepairWorkTarget::Compartment {
        compartment: "front".into(),
    });
    assert!(teams.dispatch_physical(0, SystemId("engine".into()), target.clone(), "Front".into()));
    assert!(!teams.dispatch_physical(1, SystemId("engine".into()), target, "Front".into()));
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    let saved: RepairJobState =
        serde_json::from_str(&serde_json::to_string(&teams.save_state()).unwrap()).unwrap();
    let mut restored = teams.clone();
    restored.restore_state(&saved).unwrap();
    let mut other_damage = damage.clone();
    let mut other_stock = stock.clone();
    for _ in 0..10 {
        advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
        advance(
            &mut restored,
            &mut other_damage,
            &mut other_stock,
            &services,
            0.5,
        );
        assert_eq!(teams.save_state(), restored.save_state());
        assert_eq!(damage, other_damage);
        assert_eq!(stock, other_stock);
    }
}

#[test]
fn patched_component_accepts_later_full_work_but_never_another_patch() {
    let (mut teams, mut damage, mut stock, services) = fixture(0);
    damage
        .components
        .get_mut(&SystemId("engine".into()))
        .unwrap()
        .get_mut("operation")
        .unwrap()
        .condition = ComponentCondition::Patched { factor: 0.25 };
    let target = Some(RepairWorkTarget::Component {
        system: SystemId("engine".into()),
        component: "operation".into(),
    });
    assert!(teams.dispatch_physical(0, SystemId("engine".into()), target, "Engine".into()));
    for _ in 0..10 {
        advance(&mut teams, &mut damage, &mut stock, &services, 1.0);
    }
    assert!(teams.work_list()[0].waiting_parts);
    assert_eq!(
        damage.components[&SystemId("engine".into())]["operation"].condition,
        ComponentCondition::Patched { factor: 0.25 }
    );
    stock.adjust_capacity("parts", 1);
    for _ in 0..10 {
        advance(&mut teams, &mut damage, &mut stock, &services, 1.0);
    }
    assert_eq!(
        damage.components[&SystemId("engine".into())]["operation"].condition,
        ComponentCondition::Healthy
    );
    assert_eq!(stock.capacity_stock("parts").unwrap().level, 0);
}

#[test]
fn continuation_rejects_incompatible_recipe_and_remote_component_work() {
    let (mut teams, mut damage, mut stock, services) = fixture(4);
    damage.compartments.get_mut("front").unwrap().hp = 50.0;
    teams.dispatch_physical(0, SystemId("engine".into()), None, "Engine".into());
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    let before = teams.save_state();
    let mut forged = before.clone();
    forged.physical_work.values_mut().next().unwrap().recipe = Some("full".into());
    assert!(teams.restore_state(&forged).is_err());
    assert_eq!(teams.save_state(), before);
    let mut forged = before.clone();
    forged.physical_work.values_mut().next().unwrap().target = Some(RepairWorkTarget::Component {
        system: SystemId("scanner".into()),
        component: "operation".into(),
    });
    forged.physical_work.values_mut().next().unwrap().recipe = Some("full".into());
    assert!(teams.restore_state(&forged).is_err());
    assert_eq!(teams.save_state(), before);
}

#[test]
fn same_anchor_retarget_to_another_compartment_remains_saveable_before_departure() {
    let (mut teams, mut damage, mut stock, services) = fixture(4);
    damage.compartments.get_mut("front").unwrap().hp = 50.0;
    damage.compartments.get_mut("rear").unwrap().hp = 50.0;
    teams.dispatch_physical(0, SystemId("engine".into()), None, "Front".into());
    advance(&mut teams, &mut damage, &mut stock, &services, 0.5);
    teams.dispatch_physical(
        0,
        SystemId("engine".into()),
        Some(RepairWorkTarget::Compartment {
            compartment: "rear".into(),
        }),
        "Rear".into(),
    );
    advance(&mut teams, &mut damage, &mut stock, &services, 0.1);
    let saved = teams.save_state();
    assert!(matches!(teams.slots()[0], TeamSlot::Travelling { .. }));
    assert!(saved
        .physical_work
        .values()
        .all(|work| work.recipe.is_some() == work.reserved));
    teams.clone().restore_state(&saved).unwrap();
    assert_eq!(damage.compartments["rear"].hp, 50.0);
}

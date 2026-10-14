use super::*;
use crate::infrastructure::condition::{InfrastructureConfig, InfrastructureState};
use crate::resources::service::ResourceEndpoint;
use phoenix_model::damage::ComponentCondition;

fn fixture(
    parts: i64,
) -> (
    RepairTeams,
    DamageDefinition,
    DamageState,
    InfrastructureState,
    BTreeMap<String, ResourceEndpoint>,
) {
    let config =
        crate::test_fixtures::load_entity_config("assets/entities/alliance_cruiser.toml").unwrap();
    let damage = crate::ship::physical_damage::ShipDamage::from_config(&config)
        .unwrap()
        .unwrap();
    let mut services = BTreeMap::new();
    let mut resources = BTreeSet::new();
    for recipe in damage.definition.config.remedies.values() {
        for part in &recipe.requirements {
            services.insert(
                part.service.clone(),
                ResourceEndpoint::Capacity {
                    id: part.resource.clone(),
                },
            );
            resources.insert(part.resource.clone());
        }
    }
    let capacity = resources
        .into_iter()
        .map(|id| serde_json::json!({"id":id,"amount":parts,"ceiling":1000}))
        .collect::<Vec<_>>();
    let authored: InfrastructureConfig =
        serde_json::from_value(serde_json::json!({"capacity":capacity})).unwrap();
    (
        RepairTeams::new(1),
        damage.definition,
        damage.state,
        InfrastructureState::from_config(&authored),
        services,
    )
}
fn damaged_component(definition: &DamageDefinition, state: &mut DamageState) -> RepairWorkTarget {
    let (system, component) = definition
        .components
        .iter()
        .flat_map(|(system, components)| {
            components
                .iter()
                .filter(|(_, component)| component.field_replaceable)
                .map(move |(id, _)| (system, id))
        })
        .next()
        .unwrap();
    let row = state
        .components
        .get_mut(system)
        .unwrap()
        .get_mut(component)
        .unwrap();
    row.condition = ComponentCondition::Destroyed;
    row.last_source = Some("external-test-hit".into());
    RepairWorkTarget::Component {
        system: system.clone(),
        component: component.clone(),
    }
}
fn advance(
    teams: &mut RepairTeams,
    definition: &DamageDefinition,
    state: &mut DamageState,
    stock: &mut InfrastructureState,
    services: &BTreeMap<String, ResourceEndpoint>,
    dt: f32,
    tools: f32,
    claimed: &BTreeSet<RepairWorkTarget>,
) {
    teams.tick_external_physical(
        dt,
        "foreign",
        definition,
        state,
        claimed,
        |action, claim, recipe| {
            if action == RepairStockAction::Release {
                stock.release_repair_claim(claim);
                return true;
            }
            let recipe = recipe.unwrap();
            stock.repair_parts(action, claim, recipe, services).is_ok()
        },
        |_, _| tools,
    );
}
fn remaining(stock: &InfrastructureState) -> i64 {
    stock
        .capacities()
        .iter()
        .map(|capacity| capacity.stock.level)
        .sum()
}
fn reserved(stock: &InfrastructureState) -> i64 {
    stock
        .capacities()
        .iter()
        .flat_map(|capacity| capacity.stock.reserved.values())
        .sum()
}

#[test]
fn a_visited_zero_structure_actor_is_restored_manually_with_real_source_parts() {
    let (mut teams, definition, mut damage, mut stock, services) = fixture(500);
    let room = definition
        .interior
        .compartments
        .iter()
        .min_by_key(|room| &room.id)
        .unwrap();
    let room_id = room.id.clone();
    damage.compartments.get_mut(&room_id).unwrap().hp = 0.0;
    damage.compartments.get_mut(&room_id).unwrap().disabled = true;
    assert!(teams.claim_external(0, "foreign".into()));
    let before = remaining(&stock);
    let work_secs = definition.config.remedies[&room.structural_remedy].work_secs;
    let manual = definition.config.remedies[&room.structural_remedy].manual_work_fraction;
    advance(
        &mut teams,
        &definition,
        &mut damage,
        &mut stock,
        &services,
        work_secs / manual,
        0.0,
        &BTreeSet::new(),
    );
    assert!(damage.compartments[&room_id].hp > 0.0);
    assert!(
        remaining(&stock) < before,
        "actual source parts are consumed by the completed stage"
    );
    assert!(!damage.catastrophic);
    assert_eq!(
        teams.save_state().teams.len(),
        1,
        "the target never gains a second crew pool"
    );
}

#[test]
fn external_component_stage_continues_through_json_and_consumes_exactly_once() {
    let (mut teams, definition, mut damage, mut stock, services) = fixture(500);
    let target = damaged_component(&definition, &mut damage);
    assert!(teams.claim_external(0, "foreign".into()));
    advance(
        &mut teams,
        &definition,
        &mut damage,
        &mut stock,
        &services,
        0.1,
        0.0,
        &BTreeSet::new(),
    );
    let saved = teams.save_state();
    assert_eq!(saved.external_physical_work.len(), 1);
    assert_eq!(
        saved
            .external_physical_work
            .values()
            .next()
            .unwrap()
            .target
            .as_ref(),
        Some(&target)
    );
    assert!(reserved(&stock) > 0);
    let encoded = serde_json::to_vec(&(saved, stock.clone(), damage.clone())).unwrap();
    let (jobs, mut resumed_stock, mut resumed_damage): (
        RepairJobState,
        InfrastructureState,
        DamageState,
    ) = serde_json::from_slice(&encoded).unwrap();
    let mut resumed = RepairTeams::new(1);
    resumed
        .validate_external_physical_targets(
            &jobs,
            &BTreeMap::from([("foreign".into(), definition.clone())]),
            &BTreeSet::from(["foreign".into()]),
        )
        .unwrap();
    resumed.restore_state(&jobs).unwrap();
    let before = remaining(&stock);
    for _ in 0..300 {
        advance(
            &mut teams,
            &definition,
            &mut damage,
            &mut stock,
            &services,
            0.2,
            1.0,
            &BTreeSet::new(),
        );
        advance(
            &mut resumed,
            &definition,
            &mut resumed_damage,
            &mut resumed_stock,
            &services,
            0.2,
            1.0,
            &BTreeSet::new(),
        );
        assert_eq!(teams.save_state(), resumed.save_state());
        assert_eq!(stock, resumed_stock);
        assert_eq!(damage, resumed_damage);
    }
    assert!(remaining(&stock) < before);
    let after = remaining(&stock);
    advance(
        &mut teams,
        &definition,
        &mut damage,
        &mut stock,
        &services,
        1.0,
        1.0,
        &BTreeSet::new(),
    );
    assert_eq!(
        remaining(&stock),
        after,
        "completed stage cannot consume twice"
    );
    assert!(!needs_work(&definition, &damage, &target));
}

#[test]
fn recall_retarget_and_vanished_target_release_actual_claims_without_recipe_lookup() {
    for mode in [0, 1, 2] {
        let (mut teams, definition, mut damage, mut stock, services) = fixture(500);
        damaged_component(&definition, &mut damage);
        teams.claim_external(0, "foreign".into());
        advance(
            &mut teams,
            &definition,
            &mut damage,
            &mut stock,
            &services,
            0.1,
            0.0,
            &BTreeSet::new(),
        );
        let before = remaining(&stock);
        assert!(reserved(&stock) > 0);
        if mode == 0 {
            teams.release_external();
        }
        if mode == 1 {
            assert!(teams.claim_external(0, "replacement".into()));
        }
        teams.cleanup_external_physical(|_| mode != 2, |claim| stock.release_repair_claim(claim));
        assert_eq!(reserved(&stock), 0);
        assert_eq!(
            remaining(&stock),
            before,
            "release never refunds an already consumed stage"
        );
        assert!(teams.save_state().external_physical_work.is_empty());
    }
}

#[test]
fn other_jobs_claims_prevent_duplicate_work_and_preflight_rejects_wrong_recipe() {
    let (mut teams, definition, mut damage, mut stock, services) = fixture(500);
    let target = damaged_component(&definition, &mut damage);
    teams.claim_external(0, "foreign".into());
    let before = remaining(&stock);
    advance(
        &mut teams,
        &definition,
        &mut damage,
        &mut stock,
        &services,
        1.0,
        1.0,
        &BTreeSet::from([target.clone()]),
    );
    assert_eq!(remaining(&stock), before);
    assert_eq!(reserved(&stock), 0);
    assert!(needs_work(&definition, &damage, &target));
    let mut teams = RepairTeams::new(1);
    teams.claim_external(0, "foreign".into());
    advance(
        &mut teams,
        &definition,
        &mut damage,
        &mut stock,
        &services,
        0.1,
        0.0,
        &BTreeSet::new(),
    );
    let mut invalid = teams.save_state();
    invalid
        .external_physical_work
        .values_mut()
        .next()
        .unwrap()
        .work_secs = Some(9999.0);
    assert!(teams
        .validate_external_physical_targets(
            &invalid,
            &BTreeMap::from([("foreign".into(), definition)]),
            &BTreeSet::from(["foreign".into()])
        )
        .is_err());
}

#[test]
fn deleted_target_frontier_is_continuable_only_for_claim_release() {
    let (mut teams, definition, mut damage, mut stock, services) = fixture(500);
    damaged_component(&definition, &mut damage);
    teams.claim_external(0, "foreign".into());
    advance(
        &mut teams,
        &definition,
        &mut damage,
        &mut stock,
        &services,
        0.1,
        0.0,
        &BTreeSet::new(),
    );
    let saved = teams.save_state();
    let stage = saved.external_physical_work.values().next().unwrap();
    assert!(!stage.requirements.is_empty());
    assert!(stage.reserved);
    teams
        .validate_external_physical_targets(&saved, &BTreeMap::new(), &BTreeSet::new())
        .unwrap();
    assert!(
        teams
            .validate_external_physical_targets(
                &saved,
                &BTreeMap::new(),
                &BTreeSet::from(["foreign".into()])
            )
            .is_err(),
        "a present nonphysical/malformed target must not borrow vanished-target cleanup semantics"
    );
    let mut resumed = RepairTeams::new(1);
    resumed
        .restore_state(&serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap())
        .unwrap();
    let before = remaining(&stock);
    resumed.cleanup_external_physical(|_| false, |claim| stock.release_repair_claim(claim));
    assert_eq!(reserved(&stock), 0);
    assert_eq!(remaining(&stock), before);
    assert!(resumed.save_state().external_physical_work.is_empty());
}

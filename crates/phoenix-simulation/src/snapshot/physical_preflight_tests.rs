//! Physical restore preflight must inspect rebuilt target shapes before any
//! live roster, layer membership, existing entity or clock is changed.
use super::*;
use crate::ship::compartment_damage::ShipDamage;
use crate::world::{
    server::{WorldLayerMap, WorldRuntime},
    spawn_origin::SpawnOrigin,
};
use phoenix_model::damage::HitSample;

const ATTACKER: &str = "physical-preflight-attacker";
const TARGET: &str = "physical-preflight-target";
const LAYER: &str = "physical-preflight-layer";
const BANK: &str = "fore";

fn base_world() -> World {
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(23));
    world.insert_resource(Time::<Fixed>::default());
    world.insert_resource(WorldLayerMap(std::collections::HashMap::from([(
        LAYER.into(),
        WorldRuntime {
            is_active: true,
            activation_order: 1,
            ..Default::default()
        },
    )])));
    // Capture's optional groups deliberately preserve registration boundaries.
    // Register every member rather than allowing a sparse fixture to omit the
    // actual weapon, repair or entity state the rejection must preserve.
    world.register_component::<ShipPhysics>();
    world.register_component::<EntitySystemHull>();
    world.register_component::<ShipRedAlert>();
    world.register_component::<crate::console::command::server::ShipStationStances>();
    world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    world.register_component::<ActiveBeam>();
    world.register_component::<PhaserCooldown>();
    world.register_component::<TorpedoSystemResource>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<ShipRepairTeams>();
    world.register_component::<RepairRequestQueue>();
    world.register_component::<RepairHumanAlerted>();
    world.register_component::<crate::server_app::ShipSystemBlackboards>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<crate::ship::shields::ShipShields>();
    world
}

fn spawn(world: &mut World, uuid: &str, in_layer: bool) -> Entity {
    let origin = SpawnOrigin {
        template_path: "assets/entities/alliance_cruiser.toml".into(),
        name: uuid.into(),
        position: if in_layer { [60.0, 0.0, 0.0] } else { [0.0; 3] },
        layer_path: in_layer.then(|| LAYER.into()),
        ..Default::default()
    };
    let row = EntityState {
        uuid: uuid.into(),
        spawn: Some(origin.clone()),
        ..Default::default()
    };
    super::spawn_from_origin(world, &row, &origin)
        .expect("actual shipped spawn recipe must rebuild")
}

fn saved_attack() -> PhoenixSnapshot {
    let mut world = base_world();
    let attacker = spawn(&mut world, ATTACKER, false);
    let target = spawn(&mut world, TARGET, true);
    let deck = world
        .get::<ShipDamage>(target)
        .unwrap()
        .definition
        .interior
        .decks[0]
        .id
        .clone();
    let mut beam = world.get_mut::<ActiveBeam>(attacker).unwrap();
    beam.start(BANK, TARGET, 4.0, 2.0);
    let slot = beam.bank_slot_mut(BANK).unwrap();
    slot.impact_sample_target = Some(TARGET.into());
    slot.impact_sample = Some(HitSample {
        deck,
        profile_fraction: 0.25,
    });
    drop(beam);
    let payload = capture(&world);
    assert_eq!(payload.entities.len(), 2);
    let attacker = payload
        .entities
        .iter()
        .find(|row| row.uuid == ATTACKER)
        .unwrap();
    assert_eq!(
        attacker.weapons.as_ref().unwrap().beam_impact_samples.len(),
        1
    );
    assert!(payload
        .entities
        .iter()
        .find(|row| row.uuid == TARGET)
        .unwrap()
        .physical_damage
        .is_some());
    serde_json::from_slice(&serde_json::to_vec(&payload).unwrap()).unwrap()
}

#[test]
fn existing_attacker_missing_target_invalid_deck_refuses_before_all_live_writes() {
    let mut payload = saved_attack();
    payload.tick = 999;
    let row = payload
        .entities
        .iter_mut()
        .find(|row| row.uuid == ATTACKER)
        .unwrap();
    row.red_alert = Some(true);
    row.weapons
        .as_mut()
        .unwrap()
        .beam_impact_samples
        .get_mut(BANK)
        .unwrap()
        .1
        .deck = "absent-deck".into();
    let mut world = base_world();
    let attacker = spawn(&mut world, ATTACKER, false);
    // The original live-only preflight cannot resolve the missing target shape.
    // The rejection must therefore come from the complete staged roster check.
    super::preflight::validate_entity_resources(&world, attacker, row).unwrap();
    let before = serde_json::to_value(capture(&world)).unwrap();
    let entity_count = world.entities().len();
    let layer_entities = world.resource::<WorldLayerMap>().0[LAYER]
        .spawned_entities
        .clone();
    let report = restore(&mut world, &payload);
    assert!(report.gaps.iter().any(|gap| matches!(gap,
        RestoreGap::ResourceStateInvalid { uuid, reason } if uuid == ATTACKER && reason.contains("absent target deck")
    )), "{report:?}");
    assert_eq!(report.entities_spawned, 0);
    assert_eq!(report.entities_restored, 0);
    assert_eq!(
        world.entities().len(),
        entity_count,
        "a refused target must never enter the live World"
    );
    assert_eq!(
        world.resource::<WorldLayerMap>().0[LAYER].spawned_entities,
        layer_entities
    );
    assert_eq!(
        serde_json::to_value(capture(&world)).unwrap(),
        before,
        "roster, existing entity and run clock remain unchanged"
    );
}

#[test]
fn valid_retained_deck_can_restore_the_missing_physical_target() {
    let payload = saved_attack();
    let mut world = base_world();
    spawn(&mut world, ATTACKER, false);
    let report = restore(&mut world, &payload);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(
        report.entities_spawned, 1,
        "positive control must actually materialize the missing target"
    );
    assert_eq!(report.entities_restored, 2);
    let restored = capture(&world);
    assert_eq!(restored.entities.len(), 2);
    let target = restored
        .entities
        .iter()
        .find(|row| row.uuid == TARGET)
        .unwrap();
    assert_eq!(
        target.physical_damage,
        payload
            .entities
            .iter()
            .find(|row| row.uuid == TARGET)
            .unwrap()
            .physical_damage
    );
    assert_eq!(
        world.resource::<WorldLayerMap>().0[LAYER]
            .spawned_entities
            .len(),
        1
    );
    let attacker = restored
        .entities
        .iter()
        .find(|row| row.uuid == ATTACKER)
        .unwrap();
    assert_eq!(
        attacker.weapons.as_ref().unwrap().beam_impact_samples,
        payload
            .entities
            .iter()
            .find(|row| row.uuid == ATTACKER)
            .unwrap()
            .weapons
            .as_ref()
            .unwrap()
            .beam_impact_samples
    );
}

fn external_stage_world() -> (
    World,
    Entity,
    Entity,
    phoenix_model::damage::RepairWorkTarget,
) {
    use phoenix_model::damage::{ComponentCondition, RepairWorkTarget};
    let mut world = base_world();
    let source = spawn(&mut world, ATTACKER, false);
    let target = spawn(&mut world, TARGET, true);
    let mut physical = world.get::<ShipDamage>(target).unwrap().clone();
    let (system, component) = physical
        .definition
        .components
        .iter()
        .flat_map(|(system, rows)| {
            rows.iter()
                .filter(|(_, row)| row.field_replaceable)
                .map(move |(id, _)| (system.clone(), id.clone()))
        })
        .next()
        .unwrap();
    let state = physical
        .state
        .components
        .get_mut(&system)
        .unwrap()
        .get_mut(&component)
        .unwrap();
    state.condition = ComponentCondition::Destroyed;
    state.last_source = Some("snapshot-external-test".into());
    let work_target = RepairWorkTarget::Component {
        system: system.clone(),
        component,
    };
    world.entity_mut(target).insert(physical);
    let mut teams = world.get::<ShipRepairTeams>(source).unwrap().clone();
    assert!(teams.0.claim_external(0, TARGET.into()));
    world.entity_mut(source).insert(teams);
    advance_external_stage(&mut world, source, target, 0.1);
    let saved = world.get::<ShipRepairTeams>(source).unwrap().0.save_state();
    let work = saved.external_physical_work.values().next().unwrap();
    assert!(work.reserved && work.progress > 0.0);
    assert_eq!(work.target.as_ref(), Some(&work_target));
    // A local order may wait for this exact task without claiming or preempting
    // the foreign crew's already active stage.
    let mut local = world.get_mut::<ShipRepairTeams>(target).unwrap();
    assert!(local
        .0
        .dispatch_physical(0, system, Some(work_target.clone()), "Local visit".into()));
    let local_state = local.0.save_state();
    assert!(local_state
        .physical_work
        .values()
        .all(|work| work.target.is_none() && work.requested_target.as_ref() == Some(&work_target)));
    (world, source, target, work_target)
}

fn advance_external_stage(world: &mut World, source: Entity, target: Entity, dt: f32) {
    use crate::infrastructure::InfrastructureCondition;
    use crate::jobs::repair::RepairStockAction;
    let mut teams = world.get::<ShipRepairTeams>(source).unwrap().clone();
    let mut inventory = world
        .get::<InfrastructureCondition>(source)
        .unwrap()
        .clone();
    let services = world
        .get::<crate::resources::server::ResourceServices>(source)
        .unwrap()
        .clone();
    let mut damage = world.get::<ShipDamage>(target).unwrap().clone();
    teams.0.tick_external_physical(
        dt,
        TARGET,
        &damage.definition,
        &mut damage.state,
        &std::collections::BTreeSet::new(),
        |action, claim, recipe| {
            if action == RepairStockAction::Release {
                inventory.0.release_repair_claim(claim);
                true
            } else {
                inventory
                    .0
                    .repair_parts(action, claim, recipe.unwrap(), &services.0)
                    .is_ok()
            }
        },
        |_, _| 0.0,
    ); // Actual selected remedy's authored manual contribution.
    world.entity_mut(source).insert((teams, inventory));
    world.entity_mut(target).insert(damage);
}

fn stock_level(world: &World, entity: Entity) -> i64 {
    world
        .get::<crate::infrastructure::InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacities()
        .iter()
        .map(|capacity| capacity.stock.level)
        .sum()
}

#[test]
fn duplicate_cross_actor_work_or_wrong_source_reservation_refuses_without_writes() {
    for wrong_stock in [false, true] {
        let (source_world, source, _, _) = external_stage_world();
        let mut payload: PhoenixSnapshot =
            serde_json::from_slice(&serde_json::to_vec(&capture(&source_world)).unwrap()).unwrap();
        payload.tick = 999;
        if wrong_stock {
            let source = payload
                .entities
                .iter_mut()
                .find(|row| row.uuid == ATTACKER)
                .unwrap();
            let inventory = source.infrastructure.as_mut().unwrap();
            // Corrupt the serialized candidate through its public save shape,
            // without opening a mutable runtime inventory bypass for tests.
            let mut value = serde_json::to_value(&*inventory).unwrap();
            let capacity = value["capacities"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|capacity| {
                    capacity["stock"]["reserved"]
                        .as_object()
                        .unwrap()
                        .keys()
                        .any(|claim| claim.starts_with("external-repair/"))
                })
                .unwrap();
            let amount = capacity["stock"]["reserved"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap();
            *amount = serde_json::json!(amount.as_i64().unwrap() + 1);
            *inventory = serde_json::from_value(value).unwrap();
            inventory.validate_stocks().unwrap();
        } else {
            let target = payload
                .entities
                .iter_mut()
                .find(|row| row.uuid == TARGET)
                .unwrap();
            let work = target
                .repair
                .as_mut()
                .unwrap()
                .jobs
                .as_mut()
                .unwrap()
                .physical_work
                .values_mut()
                .next()
                .unwrap();
            work.target = work.requested_target.take();
        }
        let mut world = base_world();
        spawn(&mut world, ATTACKER, false);
        let before = serde_json::to_value(capture(&world)).unwrap();
        let count = world.entities().len();
        let layer = world.resource::<WorldLayerMap>().0[LAYER]
            .spawned_entities
            .clone();
        let report = restore(&mut world, &payload);
        assert!(report.gaps.iter().any(|gap| matches!(gap, RestoreGap::RepairJobsInvalid { reason, .. }
            if reason.contains(if wrong_stock { "source inventory claims disagree" } else { "same physical task" })
        )), "{report:?}");
        assert_eq!(report.entities_spawned, 0);
        assert_eq!(world.entities().len(), count);
        assert_eq!(
            world.resource::<WorldLayerMap>().0[LAYER].spawned_entities,
            layer
        );
        assert_eq!(serde_json::to_value(capture(&world)).unwrap(), before);
        assert!(
            stock_level(&source_world, source) > 0,
            "candidate source uses actual finite stock"
        );
    }
}

#[test]
fn external_stage_and_pending_local_order_resume_with_exact_source_parts_once() {
    use phoenix_model::damage::{ComponentCondition, RepairWorkTarget};
    let (mut live, source, target, work_target) = external_stage_world();
    let payload: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&capture(&live)).unwrap()).unwrap();
    let saved = live.get::<ShipRepairTeams>(source).unwrap().0.save_state();
    let stage = saved.external_physical_work.values().next().unwrap();
    let required: i64 = stage.requirements.iter().map(|part| part.amount).sum();
    assert!(required > 0);
    let recipe = &live
        .get::<ShipDamage>(target)
        .unwrap()
        .definition
        .config
        .remedies[stage.recipe.as_ref().unwrap()];
    let steps = (recipe.work_secs / recipe.manual_work_fraction / 0.1).ceil() as usize + 2;
    let initial = stock_level(&live, source);
    let mut resumed = base_world();
    let resumed_source = spawn(&mut resumed, ATTACKER, false);
    let report = restore(&mut resumed, &payload);
    assert!(report.is_complete(), "{:?}", report.gaps);
    assert_eq!(report.entities_spawned, 1);
    let resumed_target = resumed
        .query::<(Entity, &EntityUuid)>()
        .iter(&resumed)
        .find(|(_, uuid)| uuid.0 == TARGET)
        .unwrap()
        .0;
    for _ in 0..steps {
        assert_eq!(
            serde_json::to_value(capture(&live)).unwrap(),
            serde_json::to_value(capture(&resumed)).unwrap()
        );
        advance_external_stage(&mut live, source, target, 0.1);
        advance_external_stage(&mut resumed, resumed_source, resumed_target, 0.1);
    }
    assert_eq!(
        initial - stock_level(&live, source),
        required,
        "one completed stage consumes exactly its source requirements"
    );
    assert_eq!(
        stock_level(&live, source),
        stock_level(&resumed, resumed_source)
    );
    let RepairWorkTarget::Component { system, component } = work_target else {
        unreachable!()
    };
    assert_eq!(
        live.get::<ShipDamage>(target).unwrap().state.components[&system][&component].condition,
        ComponentCondition::Healthy
    );
    let local = live.get::<ShipRepairTeams>(target).unwrap().0.save_state();
    assert!(local
        .physical_work
        .values()
        .all(|work| work.target.is_none() && work.requested_target.is_some() && !work.reserved));
}

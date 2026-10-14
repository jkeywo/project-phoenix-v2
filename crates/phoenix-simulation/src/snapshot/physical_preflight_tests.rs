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

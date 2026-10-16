use super::*;
use crate::resources::recovery::{RecoveryState, RecoveryToolConfig};
use crate::resources::recovery_server::{CompiledShipRecovery, ShipRecovery};

fn fixture() -> (World, Entity) {
    let mut content = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let ship = content.ship_config.as_mut().unwrap();
    let tool = if let Some(profile) = &content.recovery {
        profile.system.clone()
    } else {
        let system = ship
            .systems
            .iter_mut()
            .find(|s| s.kind == crate::ship::system_registry::FABRICATION_KIND)
            .unwrap();
        system.kind = crate::resources::recovery::RECOVERY_KIND.into();
        content.production = None;
        system.id.clone()
    };
    let logistics = crate::resources::logistics_profile::compile_entity_logistics(&content)
        .unwrap()
        .unwrap();
    let inventory = crate::infrastructure::InfrastructureCondition(
        crate::infrastructure::InfrastructureState::from_config(
            content.infrastructure.as_ref().unwrap(),
        ),
    );
    let torpedoes = content.torpedoes.as_ref().unwrap();
    let magazine = TorpedoSystemResource(crate::weapons::torpedo::TorpedoSystem::from_configs(
        &torpedoes.tubes,
        torpedoes.to_runtime(),
    ));
    let mut world = World::new();
    world.register_component::<ShipPhysics>();
    world.register_component::<EntitySystemHull>();
    world.register_component::<ShipRedAlert>();
    world.register_component::<crate::console::command::server::ShipStationStances>();
    world.register_component::<crate::ship_plugin::ShipSystemControlSources>();
    // Capture the actual finite owners required by the managed endpoint profile.
    world.register_component::<ActiveBeam>();
    world.register_component::<PhaserCooldown>();
    world.register_component::<EntityShipArcHull>();
    world.register_component::<ShipRepairTeams>();
    world.register_component::<RepairRequestQueue>();
    world.register_component::<RepairHumanAlerted>();
    world.register_component::<crate::server_app::ShipSystemBlackboards>();
    world.register_component::<crate::ai::server::ObjectiveCursors>();
    world.register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    world.register_component::<crate::ship::shields::ShipShields>();
    world.insert_resource(SimTick(2));
    let profile = RecoveryToolConfig {
        system: tool,
        range: 20.0,
        units_per_sec: 0.25,
        resources: vec!["alloy".into()],
    };
    let mut state = RecoveryState::new(&profile).unwrap();
    state
        .select(&profile, Some("source"), Some("alloy"), Some("hold-alloy"))
        .unwrap();
    state.set_enabled(true);
    state.advance(&profile, 1, 1.0, 2, |_, _| Ok(1.0)).unwrap();
    state.set_enabled(false);
    state
        .advance(&profile, 2, 1.0, 2, |_, _| panic!("disabled"))
        .unwrap();
    let entity = world
        .spawn((
            EntityUuid("tool-ship".into()),
            CompiledShipRecovery(profile),
            ShipRecovery(state),
            crate::resources::logistics_server::ShipLogistics(
                crate::resources::logistics::LogisticsState::new(&logistics),
            ),
            crate::resources::logistics_server::CompiledShipLogistics(logistics),
            inventory,
            magazine,
        ))
        .id();
    // A Recovery tool also installs the idle equipment-recovery owner, even
    // when this focused test exercises only fractional resource processing.
    content.recovery = Some(world.get::<CompiledShipRecovery>(entity).unwrap().0.clone());
    crate::resources::equipment_recovery_server::install(
        &content,
        &mut world.commands().entity(entity),
    );
    world.flush();
    (world, entity)
}

#[test]
fn recovery_capture_json_and_restore_keep_disabled_fraction_and_receipt() {
    let (world, _) = fixture();
    let saved: PhoenixSnapshot =
        serde_json::from_str(&serde_json::to_string(&capture(&world)).unwrap()).unwrap();
    let state = saved.entities[0].recovery.as_ref().unwrap();
    assert_eq!(state.fraction, 0.25);
    assert!(!state.enabled);
    assert!(state.last_outcome.as_ref().unwrap().settled);
    let (mut fresh, entity) = fixture();
    fresh.get_mut::<ShipRecovery>(entity).unwrap().0.fraction = 0.0;
    fresh.resource_mut::<SimTick>().0 = 5;
    let report = restore(&mut fresh, &saved);
    assert!(report.gaps.is_empty(), "{:?}", report.gaps);
    assert_eq!(capture(&fresh), saved);
}

#[test]
fn invalid_recovery_continuation_refuses_before_any_live_write() {
    let (world, _) = fixture();
    let saved = capture(&world);
    for mode in 0..7 {
        let mut bad = saved.clone();
        match mode {
            0 => bad.entities[0].recovery.as_mut().unwrap().target = Some("tool-ship".into()),
            1 => bad.entities[0].recovery.as_mut().unwrap().last_tick = 3,
            2 => {
                bad.entities[0]
                    .recovery
                    .as_mut()
                    .unwrap()
                    .last_outcome
                    .as_mut()
                    .unwrap()
                    .settled = false
            }
            3 => bad.entities[0].recovery = None,
            4 => {
                bad.entities[0].recovery.as_mut().unwrap().destination_store =
                    Some("absent-store".into())
            }
            5 => {
                bad.entities[0].recovery.as_mut().unwrap().destination_store =
                    Some("repair-machinery".into())
            }
            _ => {
                bad.entities[0].recovery.as_mut().unwrap().destination_store =
                    Some("hold-machinery-parts".into())
            }
        }
        let (mut fresh, _) = fixture();
        fresh.resource_mut::<SimTick>().0 = 5;
        let before = capture(&fresh);
        let report = restore(&mut fresh, &bad);
        assert!(!report.gaps.is_empty(), "case {mode} accepted");
        assert_eq!(capture(&fresh), before, "case {mode} wrote live state");
    }
}

#[test]
fn capture_and_fold_require_the_same_tick_receipt_barrier() {
    let (mut world, entity) = fixture();
    let mut profile = world.get::<CompiledShipRecovery>(entity).unwrap().0.clone();
    profile.units_per_sec = 2.0;
    world.get_mut::<CompiledShipRecovery>(entity).unwrap().0 = profile.clone();
    let mut state = world.get_mut::<ShipRecovery>(entity).unwrap();
    state.0.set_enabled(true);
    assert_eq!(
        state
            .0
            .advance(&profile, 3, 1.0, 1, |_, _| Ok(1.0))
            .unwrap(),
        1
    );
    world.resource_mut::<SimTick>().0 = 3;
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| capture(&world))).is_err());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::sim_digest::world_digest(&world)
    }))
    .is_err());
}

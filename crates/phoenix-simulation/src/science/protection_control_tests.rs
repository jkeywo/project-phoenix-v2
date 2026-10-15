use super::*;
use crate::core::messages::AdmittedCommand;
use bevy::ecs::system::RunSystemOnce;
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn apply(world: &mut World, entity: Entity, target: &str, payload: Control) {
    world.get_mut::<AdmittedCommands>(entity).unwrap().0 = vec![AdmittedCommand {
        target: id(target),
        payload,
        response_token: None,
        feedback_correlation: None,
    }];
    world.run_system_once(handle_commands).unwrap();
    world.get_mut::<AdmittedCommands>(entity).unwrap().0.clear();
}
#[test]
fn actual_owned_admitted_controls_preserve_order_and_refuse_foreign_duplicate_and_overbound_priorities(
) {
    let (mut world, entity) = super::super::tests::fixture();
    let owner = id("signal-protection");
    world
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
    apply(
        &mut world,
        entity,
        &owner.0,
        Control::SetSignalProtectionPriorities {
            systems: vec![id("tactical-radar"), id("comms"), id("sensors")],
        },
    );
    let before = world.get::<ShipSignalProtection>(entity).unwrap().clone();
    assert_eq!(
        before.priorities(&world.get::<ShipConfigComponent>(entity).unwrap().0, &owner),
        &[id("tactical-radar"), id("comms"), id("sensors")]
    );
    for systems in [
        vec![id("foreign")],
        vec![id("comms"), id("comms")],
        vec![id("sensors"); 65],
    ] {
        apply(
            &mut world,
            entity,
            &owner.0,
            Control::SetSignalProtectionPriorities { systems },
        );
        assert_eq!(world.get::<ShipSignalProtection>(entity), Some(&before));
    }
    apply(
        &mut world,
        entity,
        "comms",
        Control::SetSignalProtectionEnabled { enabled: false },
    );
    assert_eq!(world.get::<ShipSignalProtection>(entity), Some(&before));
    apply(
        &mut world,
        entity,
        &owner.0,
        Control::SetSignalProtectionEnabled { enabled: false },
    );
    assert!(!project(&world, entity, &owner).unwrap().enabled);
    let view = project(&world, entity, &owner).unwrap();
    assert!(view.controllable);
    assert!(!view.active && view.protected_systems.is_empty());
    assert!(view
        .systems
        .iter()
        .any(|choice| choice.system == id("comms")
            && choice.cost == 2
            && choice.home_station == Some(crate::core::messages::StationId("comms".into()))));
    world
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Offline,
        );
    apply(
        &mut world,
        entity,
        &owner.0,
        Control::SetSignalProtectionEnabled { enabled: true },
    );
    assert!(!project(&world, entity, &owner).unwrap().enabled);
    assert!(!project(&world, entity, &owner).unwrap().controllable);
}
#[test]
fn ordinary_same_system_tree_emits_normally_admitted_enable_then_empty_priority_stop_without_exposure_input(
) {
    let (world, entity) = super::super::tests::fixture();
    let mut app = App::new();
    *app.world_mut() = world;
    crate::ai::host::register_ai_host_env(&mut app);
    let world = app.world_mut();
    let owner = id("signal-protection");
    world
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
    apply(
        world,
        entity,
        &owner.0,
        Control::SetSignalProtectionEnabled { enabled: false },
    );
    let before = world.get::<ShipSignalProtection>(entity).unwrap().clone();
    world.run_system_once(operate_ai).unwrap();
    assert!(
        world.get::<AdmittedCommands>(entity).unwrap().0.is_empty(),
        "same tree cannot actuate Human equipment"
    );
    world
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    world.run_system_once(operate_ai).unwrap();
    let admitted = &world.get::<AdmittedCommands>(entity).unwrap().0;
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0].target, owner);
    assert_eq!(
        admitted[0].payload,
        Control::SetSignalProtectionEnabled { enabled: true }
    );
    assert_eq!(
        admitted[0].response_token.as_deref(),
        Some("ai:protection-source")
    );
    world.run_system_once(handle_commands).unwrap();
    world.get_mut::<AdmittedCommands>(entity).unwrap().0.clear();
    world.run_system_once(super::super::tick).unwrap();
    let enabled = project(world, entity, &owner).unwrap();
    assert!(enabled.enabled && enabled.active && !enabled.controllable);
    assert_eq!(enabled.protected_systems, vec![id("comms"), id("sensors")]);
    assert_eq!(
        enabled.priorities,
        before.priorities(&world.get::<ShipConfigComponent>(entity).unwrap().0, &owner)
    );
    world
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
    apply(
        world,
        entity,
        &owner.0,
        Control::SetSignalProtectionPriorities { systems: vec![] },
    );
    world
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    world.run_system_once(operate_ai).unwrap();
    assert_eq!(world.get::<AdmittedCommands>(entity).unwrap().0.len(), 1);
    assert_eq!(
        world.get::<AdmittedCommands>(entity).unwrap().0[0].payload,
        Control::SetSignalProtectionEnabled { enabled: false }
    );
    world.run_system_once(handle_commands).unwrap();
    assert!(!project(world, entity, &owner).unwrap().enabled);
}

#[test]
fn missing_actual_source_never_offers_control_or_changes_retained_intent() {
    let (mut world, entity) = super::super::tests::fixture();
    let owner = id("signal-protection");
    world.get_mut::<ShipSystemControlSources>(entity).unwrap().0 =
        crate::ship::control_source::ControlSourceResolver::new();
    let before = world.get::<ShipSignalProtection>(entity).unwrap().clone();
    assert!(!project(&world, entity, &owner).unwrap().controllable);
    apply(
        &mut world,
        entity,
        &owner.0,
        Control::SetSignalProtectionEnabled { enabled: false },
    );
    apply(
        &mut world,
        entity,
        &owner.0,
        Control::SetSignalProtectionPriorities { systems: vec![] },
    );
    assert_eq!(world.get::<ShipSignalProtection>(entity), Some(&before));
}

#[test]
fn priority_choices_use_the_actual_own_equipment_display_label() {
    // Projection-only legacy metadata fixture: the actual physical Alliance
    // fixture has ShipDamage and intentionally does not install this wrapper.
    // Keep one hull owner here; no duplicate physical HP pool or work is added.
    let config = super::super::tests::config().ship_config.unwrap();
    let state = ShipSignalProtection::from_ship(&config).unwrap();
    let binding =
        protection_composition(&config.system(&id("signal-protection")).unwrap().components)
            .unwrap();
    let hull = crate::ship::damage::SystemHull::from_config_with_display_names(
        binding
            .config
            .systems
            .keys()
            .map(|id| {
                (
                    id.clone(),
                    format!("observed-own-label/{}", id.0),
                    100.0,
                    crate::ship::damage::ConsoleTierConfig::default(),
                )
            })
            .collect(),
    );
    let mut world = World::new();
    let entity = world
        .spawn((
            ShipConfigComponent(config),
            state,
            crate::entities::spawner::EntitySystemHull(hull),
        ))
        .id();
    let view = project(&world, entity, &id("signal-protection")).unwrap();
    let hull = world
        .get::<crate::entities::spawner::EntitySystemHull>(entity)
        .unwrap();
    assert!(!view.systems.is_empty());
    for choice in view.systems {
        assert_eq!(
            choice.display_name,
            hull.0.get(&choice.system).unwrap().display_name
        );
    }
}

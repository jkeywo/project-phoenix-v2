use super::*;
use crate::core::messages::ViewscreenBlackboard;
use crate::server_app::ShipSystemBlackboards;
use crate::ship_plugin::ShipConfigComponent;
use bevy::ecs::system::RunSystemOnce;

fn fixture() -> (World, Entity, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap()
    .ship_config
    .unwrap();
    let mut sources = ShipSystemControlSources::default();
    for system in &config.systems {
        sources.0.set(
            system.id.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
    }
    let mut boards = ShipSystemBlackboards::default();
    boards.0.insert(
        crate::ship::system_registry::viewscreen_system_id(),
        SystemBlackboard::Viewscreen(ViewscreenBlackboard {
            combat_lock: Some("target".into()),
            ..Default::default()
        }),
    );
    let mut world = World::new();
    world.insert_resource(Time::<()>::default());
    world.init_resource::<NpcFrequencyMatchStates>();
    let ship = world
        .spawn((
            ShipConfigComponent(config),
            sources,
            boards,
            crate::ship::state::ShipPhaserFrequency(0.5),
            crate::ai::server::AiHighFidelity,
            crate::entities::spawner::EntityUuid("observer".into()),
        ))
        .id();
    let target = world
        .spawn((
            crate::entities::spawner::EntityUuid("target".into()),
            crate::ship::shields::ShipShields(
                crate::weapons::shield::ShieldSystem::default(),
                0.93,
            ),
        ))
        .id();
    (world, ship, target)
}
fn acquire(world: &mut World, ship: Entity) {
    crate::science::shield_enquiry::fixture::acquired(
        &mut world.get_mut::<ShipSystemBlackboards>(ship).unwrap(),
        "target",
        0.73,
    );
}
fn run(world: &mut World, count: usize) {
    for _ in 0..count {
        world
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(1));
        world
            .run_system_once(tick_npc_auto_match_frequency)
            .unwrap();
    }
}
fn frequency(world: &World, ship: Entity) -> f32 {
    world
        .get::<crate::ship::state::ShipPhaserFrequency>(ship)
        .unwrap()
        .0
}

#[test]
fn npc_matches_only_acquired_frequency_and_cannot_read_rotated_target_truth() {
    let (mut world, ship, target) = fixture();
    run(&mut world, 5);
    assert_eq!(
        frequency(&world, ship),
        0.5,
        "a physical target and lock do not grant a finding"
    );
    acquire(&mut world, ship);
    run(&mut world, 5);
    assert_eq!(frequency(&world, ship), 0.73);
    world
        .get_mut::<crate::ship::shields::ShipShields>(target)
        .unwrap()
        .1 = 0.14;
    world
        .get_mut::<crate::ship::state::ShipPhaserFrequency>(ship)
        .unwrap()
        .0 = 0.5;
    world.resource_mut::<NpcFrequencyMatchStates>().0.clear();
    run(&mut world, 5);
    assert_eq!(
        frequency(&world, ship),
        0.73,
        "held sampled configuration remains historical"
    );
}

#[test]
fn npc_rechecks_exact_weapon_feed_conceal_and_current_observation_before_matching() {
    for variant in 0..3 {
        let (mut world, ship, _) = fixture();
        acquire(&mut world, ship);
        run(&mut world, 1);
        assert!(world
            .resource::<NpcFrequencyMatchStates>()
            .0
            .contains_key(&ship));
        match variant {
            0 => {
                let source = crate::science::scan_blackboard_key_for(
                    &crate::ship::system_registry::sensors_system_id(),
                );
                let mut config = world.get_mut::<ShipConfigComponent>(ship).unwrap();
                let consoles: Vec<_> = config
                    .0
                    .systems
                    .iter()
                    .filter(|system| system.kind == crate::ship::system_registry::PHASER_BANK_KIND)
                    .filter_map(|system| system.console.clone())
                    .collect();
                for console in &mut config.0.consoles {
                    if consoles.contains(&console.id) {
                        console.feeds.retain(|feed| feed.source != source);
                    }
                }
            }
            1 => {
                world.init_resource::<crate::world::server::WorldContentRuntime>();
                crate::gm::operations::contact::set(
                    &mut world
                        .resource_mut::<crate::world::server::WorldContentRuntime>()
                        .contact_overrides,
                    "observer",
                    "target",
                    crate::gm::operations::contact::ContactMode::Conceal,
                );
            }
            _ => {
                world
                    .entity_mut(ship)
                    .insert(crate::science::broad::BroadTracks(Default::default()));
            }
        }
        run(&mut world, 5);
        assert_eq!(frequency(&world, ship), 0.5, "variant {variant}");
        assert!(!world
            .resource::<NpcFrequencyMatchStates>()
            .0
            .contains_key(&ship));
    }
}

#[test]
fn npc_uses_effective_per_bank_intent_and_never_substitutes_primary_for_hold_or_unknown() {
    let (mut world, ship, _) = fixture();
    acquire(&mut world, ship);
    let banks: Vec<_> = world
        .get::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .systems
        .iter()
        .filter(|system| system.kind == crate::ship::system_registry::PHASER_BANK_KIND)
        .map(|system| system.id.clone())
        .collect();
    assert!(banks.len() > 1);
    let mut targets = super::super::assignments::ShipWeaponTargets::default();
    for bank in &banks {
        targets.0.set_individual(bank.clone(), None);
    }
    world.entity_mut(ship).insert(targets);
    run(&mut world, 5);
    assert_eq!(
        frequency(&world, ship),
        0.5,
        "Hold cannot borrow primary evidence"
    );
    world
        .get_mut::<super::super::assignments::ShipWeaponTargets>(ship)
        .unwrap()
        .0
        .set_individual(banks[0].clone(), Some("other-contact".into()));
    run(&mut world, 5);
    assert_eq!(
        frequency(&world, ship),
        0.5,
        "uninvestigated override cannot borrow primary evidence"
    );
    crate::science::shield_enquiry::fixture::acquired(
        &mut world.get_mut::<ShipSystemBlackboards>(ship).unwrap(),
        "other-contact",
        0.21,
    );
    run(&mut world, 5);
    assert_eq!(frequency(&world, ship), 0.21);
    world
        .get_mut::<super::super::assignments::ShipWeaponTargets>(ship)
        .unwrap()
        .0
        .follow_primary(&banks[1]);
    world
        .get_mut::<crate::ship::state::ShipPhaserFrequency>(ship)
        .unwrap()
        .0 = 0.5;
    run(&mut world, 5);
    assert_eq!(
        frequency(&world, ship),
        0.5,
        "conflicting effective subjects withhold shared-actuator advice"
    );
}

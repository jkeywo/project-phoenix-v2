use super::*;
use crate::ship::compartment_damage::ShipDamage;

fn fixture() -> (ShipDamage, crate::ship::config::ShipConfig) {
    let entity = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    (
        ShipDamage::from_config(&entity).unwrap().unwrap(),
        entity.ship_config.unwrap(),
    )
}

#[test]
fn gm_scopes_resolve_shared_real_rooms_and_project_existing_arms() {
    let (damage, config) = fixture();
    let (system, room) = damage.definition.interior.placements.iter().next().unwrap();
    let first = GmDirectEffectScope::System(system.clone());
    let rooms = physical_scope_rooms(&first, &damage, Some(&config)).unwrap();
    assert_eq!(rooms, [room.clone()]);
    let other = damage
        .definition
        .interior
        .placements
        .iter()
        .find(|(id, candidate)| *id != system && *candidate == room)
        .unwrap()
        .0;
    assert_eq!(
        physical_scope_rooms(
            &GmDirectEffectScope::System(other.clone()),
            &damage,
            Some(&config)
        )
        .unwrap(),
        rooms
    );
    let mut pending = PendingGmDirectEffects::default();
    pending.push(PendingGmDirectEffect {
        tick: 0,
        order: GmActionOrder::default(),
        target: "ship".into(),
        scope: first,
        kind: GmDirectEffectKind::Damage,
        amount_milli_hp: 5_000,
    });
    let totals = physical_scope_totals(&damage, &rooms);
    assert_eq!(
        pending
            .project_physical_totals(
                "ship",
                &rooms,
                &damage,
                Some(&config),
                GmDirectEffectKind::Damage,
                totals
            )
            .0,
        totals.0 - 5.0
    );
    assert!(physical_scope_rooms(
        &GmDirectEffectScope::System(crate::core::messages::SystemId("missing".into())),
        &damage,
        Some(&config)
    )
    .is_err());
}

#[test]
fn gm_structural_zero_is_recoverable_and_healing_keeps_component_faults() {
    let (damage, config) = fixture();
    let mut app = App::new();
    app.init_resource::<PendingGmDirectEffects>()
        .insert_sim_rng(crate::sim_rng::SimRng::new(
            44,
            crate::sim_rng::SeedSource::Cli,
        ))
        .init_resource::<Messages<crate::ai::server::AiEntityDestroyed>>()
        .add_systems(Update, apply_gm_direct_effects);
    let total = damage.structural_totals().1;
    let entity = app
        .world_mut()
        .spawn((
            crate::entities::spawner::EntityUuid("ship".into()),
            damage,
            crate::ship::components::ShipConfigComponent(config),
        ))
        .id();
    let effect = |kind, amount| PendingGmDirectEffect {
        tick: 0,
        order: GmActionOrder::default(),
        target: "ship".into(),
        scope: GmDirectEffectScope::Entity,
        kind,
        amount_milli_hp: hp_to_milli(amount as f32).saturating_add(1),
    };
    app.world_mut()
        .resource_mut::<PendingGmDirectEffects>()
        .push(effect(GmDirectEffectKind::Damage, total));
    app.update();
    let damage = app
        .world()
        .get::<ShipDamage>(entity)
        .expect("zero HP is retained");
    assert_eq!(damage.structural_totals().0, 0.0);
    assert!(!damage.catastrophic());
    assert!(app
        .world()
        .resource::<Messages<crate::ai::server::AiEntityDestroyed>>()
        .is_empty());
    let faults = damage.state.components.clone();
    app.world_mut()
        .resource_mut::<PendingGmDirectEffects>()
        .push(effect(GmDirectEffectKind::Heal, total));
    app.update();
    let damage = app.world().get::<ShipDamage>(entity).unwrap();
    assert_eq!(damage.structural_totals().0, total);
    assert_eq!(damage.state.components, faults);
    assert!(damage
        .state
        .compartments
        .values()
        .all(|room| !room.disabled));
    assert!(app.world().get::<EntitySystemHull>(entity).is_none());
}

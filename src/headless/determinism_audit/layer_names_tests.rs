use super::*;
#[test]
fn layer_migration_preserves_unknown_symbols_and_row_multiplicity() {
    assert_eq!(
        migrated_symbol("project_phoenix::future::NewDebt"),
        "project_phoenix::future::NewDebt"
    );
    let row = Ambiguity {
        systems: [
            "project_phoenix::ai::server::register_ai_tokens_on_spawn".into(),
            "project_phoenix::future::NewDebt".into(),
        ],
        access: vec!["project_phoenix::world_id::WorldIdMint".into()],
    };
    let migrated = parse_census(&serde_json::to_string(&vec![row.clone(), row]).unwrap()).unwrap();
    assert_eq!(migrated.len(), 2);
    assert_eq!(migrated[0], migrated[1]);
    assert_eq!(
        migrated[0].systems[0],
        "phoenix_simulation::ai::server::register_ai_tokens_on_spawn"
    );
    assert_eq!(migrated[0].systems[1], "project_phoenix::future::NewDebt");
    assert_eq!(
        migrated[0].access,
        ["phoenix_sim_contracts::world_id::WorldIdMint"]
    );
    assert_eq!(uncovered(&migrated, &migrated[..1]).len(), 1);
}
#[test]
fn layer_migration_keeps_generic_instantiations_distinct() {
    let a = migrated_symbol("project_phoenix::core::broadcast::broadcaster::BroadcastRegistry<project_phoenix::core::broadcast::sim::Sim>");
    let b = migrated_symbol("project_phoenix::core::broadcast::broadcaster::BroadcastRegistry<project_phoenix::future::Other>");
    assert_ne!(a, b);
    assert!(a.starts_with("phoenix_simulation::"));
}

#[test]
fn simulation_split_maps_reviewed_symbols_without_remapping_unknown_siblings() {
    assert_eq!(
        migrated_symbol("phoenix_simulation::world_id::WorldIdMint"),
        std::any::type_name::<crate::world_id::WorldIdMint>()
    );
    assert_eq!(
        migrated_symbol("phoenix_simulation::ship::power::ShipPowerSystem"),
        std::any::type_name::<crate::ship::power::ShipPowerSystem>()
    );
    assert_eq!(
        migrated_symbol("phoenix_simulation::future::NewDebt"),
        "phoenix_simulation::future::NewDebt"
    );
}

use super::*;

#[test]
fn missing_snapshot_resolves_once_even_before_bootstrap() {
    let mut run = snapshot::run_for(
        snapshot::PhoenixSnapshot::default(),
        0,
        42,
        "assets/worlds/duel.toml",
        vellum_save::Versions::new(1, "test", 0),
    );
    run.snapshot = None;
    let mut world = World::new();
    stage(&mut world, run);
    assert!(save_slots_lifecycle::startup_restore_pending(&world));
    assert_eq!(
        advance(&mut world),
        Some(RestoreOutcome::Failed(RestoreFailure::NoSnapshot))
    );
    assert!(!is_pending(&world));
    assert!(!save_slots_lifecycle::startup_restore_pending(&world));
    assert_eq!(advance(&mut world), None);
}

use super::*;
use crate::core::messages::{HelmBlackboard, SystemId, TacticalRadarBlackboard};
use bevy::ecs::system::RunSystemOnce;

#[test]
fn actual_helm_estimates_only_explicitly_permitted_tracks_and_rebuilds_after_feed_loss() {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let mut app = App::new();
    let entity = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "approach-observer".into(),
        None,
    );
    app.world_mut().flush();
    let record = serde_json::from_value(serde_json::json!({
        "contact_id":"opaque-torpedo", "observed_tick":2, "position":[100,20],
        "velocity":[-10,0], "position_error":1, "selectable":false,
        "stage":"classified", "identity":null, "identity_tick":null,
        "classification":"torpedo", "classification_tick":2,
        "assessment":null, "assessment_tick":null, "predicted":false,
        "samples":[{"origin":{"source":"public-origin", "system":"tactical-radar", "contact":"opaque-torpedo"},
            "sample_id":"sample-2", "observed_tick":2, "observer_position":[0,0],
            "velocity_error":0.5,"drift_error_per_sec":0.1,"received_tick":null}]
    })).unwrap();
    let owner = SystemId("helm-radar".into());
    let mut boards = app
        .world_mut()
        .get_mut::<ShipSystemBlackboards>(entity)
        .unwrap();
    boards.0.insert(
        owner.clone(),
        SystemBlackboard::Helm(HelmBlackboard {
            tracks: Some(Vec::new()),
            ..Default::default()
        }),
    );
    boards.0.insert(
        SystemId("tactical-radar".into()),
        SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
            tracks: Some(vec![record]),
            ..Default::default()
        }),
    );
    drop(boards);
    app.world_mut().run_system_once(publish).unwrap();
    let result = |app: &App| {
        let SystemBlackboard::Helm(board) =
            &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0[&owner]
        else {
            panic!("actual Helm publication");
        };
        board.approaches.clone()
    };
    assert_eq!(result(&app).len(), 1);
    assert_eq!(result(&app)[0].seconds, 10.0);
    let before = *app.world().get::<ShipPhysics>(entity).unwrap();
    let mut config = app
        .world_mut()
        .get_mut::<ShipConfigComponent>(entity)
        .unwrap();
    let console = config.0.system(&owner).unwrap().console.clone().unwrap();
    config
        .0
        .consoles
        .iter_mut()
        .find(|row| row.id == console)
        .unwrap()
        .feeds
        .retain(|feed| feed.source.0 != "tactical-radar");
    drop(config);
    app.world_mut().run_system_once(publish).unwrap();
    assert!(
        result(&app).is_empty(),
        "co-hosted/available foreign board grants no implicit feed"
    );
    assert_eq!(
        *app.world().get::<ShipPhysics>(entity).unwrap(),
        before,
        "an evidence estimate never issues avoidance or changes motion"
    );
}

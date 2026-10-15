use super::*;

#[test]
fn los_measured_ray_miss_spends_work_without_damaging_the_private_named_actor() {
    use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTrackState, BroadTracks};
    let mut app = los_test_app();
    let shooter = spawn_los_ship(&mut app, "measured-shooter", 0.0, 0.0, None, 200.0, true);
    let target = spawn_los_ship(&mut app, "private-target", 0.0, -30.0, None, 500.0, false);
    app.update();
    app.update();
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profiles = BroadProfile::from_config(&authored).unwrap().unwrap();
    let instrument = SystemId("tactical-radar".into());
    let mut state = BroadTrackState::new(&profiles.0[&instrument].profile);
    state
        .advance(
            &profiles.0[&instrument].profile,
            0,
            0.5,
            1.0,
            &[BroadMeasurement {
                source: "private-target".into(),
                selectable: true,
                observation: None,
                position: [20.0, -30.0],
                support: 1.0,
                identity: None,
                classification: Some("ship".into()),
                assessment: None,
            }],
        )
        .unwrap();
    let records = state
        .projection(&profiles.0[&instrument].profile, 0, 0.5)
        .unwrap();
    let contact = records[0].contact_id.clone();
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    boards.0.insert(
        instrument.clone(),
        SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
            tracks: Some(records),
            ..Default::default()
        }),
    );
    let mut targets = crate::console::weapons::assignments::ShipWeaponTargets::default();
    targets
        .0
        .set_individual(SystemId("phaser-fore".into()), Some(contact));
    app.world_mut()
        .entity_mut(shooter)
        .insert((
            crate::ship_plugin::ShipConfigComponent(authored.ship_config.unwrap()),
            boards,
            targets,
            BroadTracks(std::collections::BTreeMap::from([(instrument, state)])),
            profiles,
            PhaserCombatConfigResource(crate::entities::config::PhaserCombatConfig {
                banks: vec![crate::entities::config::PhaserBankConfig {
                    beam_range: 100.0,
                    beam_damage_per_sec: 100.0,
                    beam_duration_secs: 10.0,
                    ..wide_bank("fore", 0.0)
                }],
            }),
        ))
        .remove::<crate::ship::power::ShipPowerSystem>();
    app.world_mut()
        .get_mut::<ActiveBeam>(shooter)
        .unwrap()
        .start("fore", "private-target", 10.0, 0.0);
    let before = hull_hp(&app, target);
    app.update();
    let context = app
        .world()
        .resource::<crate::console::weapons::shared::BeamContext>();
    let ray = context
        .0
        .iter()
        .find(|row| row.shooter_entity == shooter)
        .expect("actual measured cycle must execute");
    assert!(
        ray.zero_damage,
        "the displaced measured ray missed the real collider"
    );
    assert!(
        ray.damage_to_apply > 0,
        "paid work still happened despite the miss"
    );
    assert_eq!(hull_hp(&app, target), before);
}

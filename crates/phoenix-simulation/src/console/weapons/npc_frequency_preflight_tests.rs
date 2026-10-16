//! Owner-level advisory fixtures start after acquisition. These compare the
//! complete actual producer against its frozen original, not scanner payment.
use super::*;

fn measured(world: &mut World, ship: Entity) -> (String, String) {
    use crate::core::messages::{SensorRadarBlackboard, TacticalRadarBlackboard};
    use crate::science::broad::{BroadMeasurement, BroadProfile, BroadTracks};
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let profiles = BroadProfile::from_config(&authored).unwrap().unwrap();
    let mut tracks = BroadTracks::new(&profiles);
    let mut handles = Vec::new();
    for (instrument, sensor) in [
        (SystemId("sensor-radar".into()), true),
        (SystemId("tactical-radar".into()), false),
    ] {
        let state = tracks.0.get_mut(&instrument).unwrap();
        state
            .advance(
                &profiles.0[&instrument].profile,
                0,
                0.5,
                1.0,
                &[BroadMeasurement {
                    elevation: Some(phoenix_model::sensing_tracks::ObservedElevation {
                        height: 7.0,
                        error: 2.0,
                        velocity: None,
                    }),
                    source: "target".into(),
                    selectable: true,
                    observation: None,
                    position: [4.0, 0.0],
                    support: 1.0,
                    identity: None,
                    classification: None,
                    assessment: None,
                }],
            )
            .unwrap();
        let records = state
            .projection(&profiles.0[&instrument].profile, 0, 0.5)
            .unwrap();
        handles.push(records[0].contact_id.clone());
        world
            .get_mut::<ShipSystemBlackboards>(ship)
            .unwrap()
            .0
            .insert(
                instrument,
                if sensor {
                    SystemBlackboard::SensorRadar(SensorRadarBlackboard {
                        tracks: Some(records),
                        ..Default::default()
                    })
                } else {
                    SystemBlackboard::TacticalRadar(TacticalRadarBlackboard {
                        tracks: Some(records),
                        ..Default::default()
                    })
                },
            );
    }
    let mut targets = super::super::super::assignments::ShipWeaponTargets::default();
    for system in &world.get::<ShipConfigComponent>(ship).unwrap().0.systems {
        if system.kind == crate::ship::system_registry::PHASER_BANK_KIND {
            targets
                .0
                .set_individual(system.id.clone(), Some(handles[1].clone()));
        }
    }
    world.entity_mut(ship).insert((tracks, targets));
    world
        .get_mut::<ShipSystemBlackboards>(ship)
        .unwrap()
        .0
        .remove(&crate::science::scan_blackboard_key_for(&SystemId(
            "sensors".into(),
        )));
    crate::science::shield_enquiry::fixture::acquired(
        &mut world.get_mut::<ShipSystemBlackboards>(ship).unwrap(),
        &handles[0],
        0.73,
    );
    assert_ne!(handles[0], handles[1]);
    (handles.remove(0), handles.remove(0))
}

fn step(world: &mut World, original: bool) {
    world
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs(1));
    if original {
        world
            .run_system_once(original_tick_npc_auto_match_frequency)
            .unwrap();
    } else {
        world
            .run_system_once(tick_npc_auto_match_frequency)
            .unwrap();
    }
}

#[test]
fn npc_frequency_preflight_matches_frozen_full_producer_each_tick() {
    for variant in 0..18 {
        let mut outputs = Vec::new();
        for original in [false, true] {
            let (mut world, ship, _) = fixture();
            acquire(&mut world, ship);
            if variant >= 15 {
                measured(&mut world, ship);
            }
            step(&mut world, original);
            assert!(world
                .resource::<NpcFrequencyMatchStates>()
                .0
                .contains_key(&ship));
            let source = crate::science::scan_blackboard_key_for(&SystemId("sensors".into()));
            match variant {
                0..=4 | 10..=12 => {
                    let mut boards = world.get_mut::<ShipSystemBlackboards>(ship).unwrap();
                    let Some(SystemBlackboard::Scan(scan)) = boards.0.get_mut(&source) else {
                        panic!("Scan");
                    };
                    match variant {
                        0 => scan.history.clear(),
                        1 => {
                            scan.history[0].aim =
                                phoenix_model::focused_enquiry::FocusedAim::General
                        }
                        2 => scan.history[0].shield_finding = None,
                        3 => scan.capable = false,
                        4 => scan.owner = Some(SystemId("wrong-owner".into())),
                        10 => {
                            let mut newer = scan.history[0].clone();
                            newer.taken_at_tick += 1;
                            newer.shield_finding = None;
                            scan.history.push(newer);
                        }
                        11 => scan.history[0].shield_finding.as_mut().unwrap().frequency = -1.0,
                        _ => scan.history[0].subject_uuid = "unrelated-public".into(),
                    }
                }
                5 | 16 => {
                    let mut config = world.get_mut::<ShipConfigComponent>(ship).unwrap();
                    for console in &mut config.0.consoles {
                        console.feeds.retain(|feed| {
                            if variant == 5 {
                                feed.source != source
                            } else {
                                feed.source.0 != "sensor-radar"
                            }
                        });
                    }
                }
                6 => {
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
                7 => {
                    world
                        .entity_mut(ship)
                        .insert(crate::science::broad::BroadTracks(Default::default()));
                }
                8 => {
                    world
                        .entity_mut(ship)
                        .remove::<crate::ai::server::AiHighFidelity>();
                }
                9 => {
                    let ids: Vec<_> = world
                        .get::<ShipConfigComponent>(ship)
                        .unwrap()
                        .0
                        .systems
                        .iter()
                        .map(|s| s.id.clone())
                        .collect();
                    for id in ids {
                        world
                            .get_mut::<ShipSystemControlSources>(ship)
                            .unwrap()
                            .0
                            .set(id, crate::ship::control_source::ControlSource::Human);
                    }
                }
                14 => {
                    let mut boards = world.get_mut::<ShipSystemBlackboards>(ship).unwrap();
                    let Some(SystemBlackboard::Scan(scan)) = boards.0.get_mut(&source) else {
                        panic!("Scan");
                    };
                    let mut conflict = scan.history[0].clone();
                    conflict.shield_finding = phoenix_model::shield_enquiry::ShieldFinding::acquire(
                        0.2,
                        Default::default(),
                        0.01,
                    );
                    scan.history.push(conflict);
                }
                17 => {
                    for board in world
                        .get_mut::<ShipSystemBlackboards>(ship)
                        .unwrap()
                        .0
                        .values_mut()
                    {
                        match board {
                            SystemBlackboard::SensorRadar(board) => {
                                for record in board.tracks.as_mut().unwrap() {
                                    record.predicted = true;
                                }
                            }
                            SystemBlackboard::TacticalRadar(board) => {
                                for record in board.tracks.as_mut().unwrap() {
                                    record.selectable = false;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            let held = world.get::<ShipSystemBlackboards>(ship).unwrap().0.clone();
            let mut series = Vec::new();
            for _ in 0..5 {
                step(&mut world, original);
                series.push((
                    frequency(&world, ship),
                    world.resource::<NpcFrequencyMatchStates>().0.clone(),
                    world
                        .entity(ship)
                        .get_ref::<crate::ship::state::ShipPhaserFrequency>()
                        .unwrap()
                        .last_changed(),
                    world
                        .get_resource_ref::<NpcFrequencyMatchStates>()
                        .unwrap()
                        .last_changed(),
                ));
                assert_eq!(world.get::<ShipSystemBlackboards>(ship).unwrap().0, held);
            }
            if matches!(variant, 13 | 15) {
                assert_eq!(frequency(&world, ship), 0.73);
            } else {
                assert_eq!(frequency(&world, ship), 0.5, "variant {variant}");
                assert!(!world
                    .resource::<NpcFrequencyMatchStates>()
                    .0
                    .contains_key(&ship));
            }
            outputs.push(series);
        }
        assert_eq!(
            outputs[0], outputs[1],
            "full actual producer variant {variant}"
        );
    }
}

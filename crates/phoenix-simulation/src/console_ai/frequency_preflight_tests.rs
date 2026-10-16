//! Advisory-owner differential proof; acquisition/payment is covered by the
//! existing focused instrument tests, not manufactured by this fixture.
use super::*;

type HintMessage = (
    Entity,
    ControlSource,
    crate::core::messages::CoordinationAddress,
    CoordinationPayload,
    crate::core::messages::CoordinationPresentation,
    String,
    crate::core::messages::SystemId,
);

fn messages(app: &App) -> Vec<HintMessage> {
    app.world()
        .resource::<CoordBox>()
        .0
        .iter()
        .map(|m| {
            (
                m.source_entity,
                m.sender_origin,
                m.address.clone(),
                m.payload.clone(),
                m.presentation.clone(),
                m.sender_label.clone(),
                m.sender_system.clone(),
            )
        })
        .collect()
}

#[test]
fn frequency_hint_preflight_matches_frozen_full_producer_state_and_messages() {
    for variant in 0..13 {
        let mut outputs = Vec::new();
        for original in [false, true] {
            let mut app = freq_hint_test_app_with_producer(original);
            let source = app.world().resource::<SourceShip>().0;
            tick_with_dt(&mut app, 0.1);
            assert!(app
                .world()
                .get::<ShipFrequencyHintState>(source)
                .unwrap()
                .0
                .current_target
                .is_some());
            let scan_key = crate::science::scan_blackboard_key_for(
                &crate::ship::system_registry::sensors_system_id(),
            );
            match variant {
                0..=4 | 9..=11 => {
                    let mut boards = app
                        .world_mut()
                        .get_mut::<crate::server_app::ShipSystemBlackboards>(source)
                        .unwrap();
                    let Some(crate::core::messages::SystemBlackboard::Scan(scan)) =
                        boards.0.get_mut(&scan_key)
                    else {
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
                        4 => scan.owner = Some(crate::core::messages::SystemId("foreign".into())),
                        9 => {
                            let mut newer = scan.history[0].clone();
                            newer.taken_at_tick += 1;
                            newer.shield_finding = None;
                            scan.history.push(newer);
                        }
                        10 => scan.history[0].shield_finding.as_mut().unwrap().frequency = -1.0,
                        _ => {
                            let mut conflicting = scan.history[0].clone();
                            conflicting.shield_finding =
                                phoenix_model::shield_enquiry::ShieldFinding::acquire(
                                    0.2,
                                    Default::default(),
                                    0.01,
                                );
                            scan.history.push(conflicting);
                        }
                    }
                }
                5 => {
                    for console in &mut app
                        .world_mut()
                        .get_mut::<ShipConfigComponent>(source)
                        .unwrap()
                        .0
                        .consoles
                    {
                        console.feeds.clear();
                    }
                }
                6 => {
                    app.init_resource::<crate::world::server::WorldContentRuntime>();
                    app.world_mut()
                        .entity_mut(source)
                        .insert(crate::entities::spawner::EntityUuid("observer".into()));
                    crate::gm::operations::contact::set(
                        &mut app
                            .world_mut()
                            .resource_mut::<crate::world::server::WorldContentRuntime>()
                            .contact_overrides,
                        "observer",
                        "target-1",
                        crate::gm::operations::contact::ContactMode::Conceal,
                    );
                }
                7 => {
                    app.world_mut()
                        .get_mut::<TacticalRadarSelection>(source)
                        .unwrap()
                        .0 = None;
                }
                8 => {
                    app.world_mut()
                        .get_mut::<ShipSystemControlSources>(source)
                        .unwrap()
                        .0
                        .set(
                            crate::ship::system_registry::sensors_system_id(),
                            ControlSource::Human,
                        );
                }
                _ => {}
            }
            let scan_before = app
                .world()
                .get::<crate::server_app::ShipSystemBlackboards>(source)
                .unwrap()
                .0[&scan_key]
                .clone();
            let mut series = Vec::new();
            let hz = crate::entities::config::GlobalConfig::default().ai_tick_hz;
            let period = 1.0 / hz;
            for _ in 0..(5.0 * hz).ceil() as usize {
                app.world_mut()
                    .resource_mut::<Time>()
                    .advance_by(std::time::Duration::from_secs_f32(period));
                app.update();
                series.push((
                    app.world()
                        .get::<ShipFrequencyHintState>(source)
                        .unwrap()
                        .0
                        .clone(),
                    messages(&app),
                    app.world()
                        .entity(source)
                        .get_ref::<ShipFrequencyHintState>()
                        .unwrap()
                        .last_changed(),
                ));
                assert_eq!(
                    app.world()
                        .get::<crate::server_app::ShipSystemBlackboards>(source)
                        .unwrap()
                        .0[&scan_key],
                    scan_before
                );
            }
            if matches!(variant, 8 | 12) {
                assert_eq!(messages(&app).len(), 1);
                assert!(
                    app.world()
                        .get::<ShipFrequencyHintState>(source)
                        .unwrap()
                        .0
                        .hint_sent
                );
            } else {
                assert!(messages(&app).is_empty(), "variant {variant}");
                assert_eq!(
                    app.world().get::<ShipFrequencyHintState>(source).unwrap().0,
                    Default::default()
                );
            }
            outputs.push(series);
        }
        assert_eq!(
            outputs[0], outputs[1],
            "full actual hint producer variant {variant}"
        );
    }
}

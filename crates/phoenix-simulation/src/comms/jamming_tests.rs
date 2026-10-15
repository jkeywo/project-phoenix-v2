use super::*;
use crate::core::messages::AdmittedCommand;
use bevy::ecs::system::RunSystemOnce;
const OWNER: &str = "comms-jamming";
fn id(value: &str) -> SystemId {
    SystemId(value.into())
}
fn energy(world: &World, entity: Entity) -> f32 {
    world
        .get::<ShipPowerSystem>(entity)
        .unwrap()
        .0
        .network()
        .consumers[&id(OWNER)]
        .energy
}
fn factor(world: &mut World, point: Vec3) -> f32 {
    world
        .run_system_once(
            move |paths: crate::regions::signal_path::RegionSignalQuery| {
                paths.comms_receiver_factor(point)
            },
        )
        .unwrap()
}
fn fixture() -> (World, Entity, Entity, String) {
    fixture_with_faction(None)
}
fn fixture_with_faction(faction: Option<&str>) -> (World, Entity, Entity, String) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/jamming_cruiser.toml",
    )
    .unwrap();
    let target_config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    let target_config = faction.map_or_else(
        || target_config.clone(),
        |faction| {
            crate::entities::loader::apply_overrides(
                &target_config,
                &toml::Value::Table(toml::toml! { faction = faction }),
            )
            .unwrap()
        },
    );
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(0));
    world.insert_resource(Time::<Fixed>::from_hz(60.0));
    world.insert_resource(crate::world::server::WorldContentRuntime::default());
    world.init_resource::<super::super::server::CommsRuntime>();
    world.init_resource::<super::super::server::CommsInboxRes>();
    world.init_resource::<crate::server_app::SimOutbox>();
    world.init_resource::<crate::lobby::server::ShipClientConfigResource>();
    world.init_resource::<crate::lobby::server::WorldResource>();
    let source = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "jam-source".into(),
        None,
    );
    let target = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &target_config,
        Vec3::new(0.0, 0.0, -20.0),
        "actual-receiver".into(),
        None,
    );
    world.flush();
    // Real paid Broad producer plus its ordinary Tactical publication. No typed
    // record, current-route map or receiver pose is inserted as an observation.
    for now in 0..=360 {
        world.resource_mut::<crate::sim_tick::SimTick>().0 = now;
        world
            .get_mut::<ShipPowerSystem>(source)
            .unwrap()
            .0
            .work_continuous(&id("tactical-radar"), 1.0 / 60.0, 1.0)
            .unwrap();
        world.run_system_once(crate::science::broad::tick).unwrap();
    }
    world
        .run_system_once(crate::console::weapons::blackboard::publish_tactical_radar_blackboard)
        .unwrap();
    let tracks = world
        .get::<crate::science::broad::BroadTracks>(source)
        .unwrap();
    let contact = tracks.0[&id("tactical-radar")]
        .current
        .get("actual-receiver")
        .unwrap()
        .clone();
    (world, source, target, contact)
}
fn command(world: &mut World, source: Entity, payload: Control) {
    world.get_mut::<AdmittedCommands>(source).unwrap().0 = vec![AdmittedCommand {
        target: id(OWNER),
        payload,
        response_token: None,
        feedback_correlation: None,
    }];
    world.run_system_once(handle_commands).unwrap();
    world.get_mut::<AdmittedCommands>(source).unwrap().0.clear();
}
fn prepare(world: &mut World, source: Entity, contact: &str) {
    command(
        world,
        source,
        Control::SetJammingTarget {
            target_uuid: Some(contact.into()),
        },
    );
    command(world, source, Control::SetJammingEnabled { enabled: true });
}
#[test]
fn actual_paid_mounted_emission_has_self_and_receiver_exposure_once_without_replay_credit() {
    let (mut world, source, _, contact) = fixture();
    prepare(&mut world, source, &contact);
    let reserve = energy(&world, source);
    assert_eq!(
        factor(&mut world, Vec3::ZERO),
        1.0,
        "unprocessed equipment cannot create free bootstrap influence"
    );
    world.run_system_once(tick).unwrap();
    assert!(energy(&world, source) < reserve);
    let paid = energy(&world, source);
    assert_eq!(
        factor(&mut world, Vec3::ZERO),
        0.2,
        "full self exposure4 gives one endpoint factor1/5"
    );
    assert!((factor(&mut world, Vec3::new(0.0, 0.0, -100.0)) - 1.0 / 3.0).abs() < 1e-6);
    assert_eq!(
        factor(&mut world, Vec3::new(0.0, 0.0, 100.0)),
        1.0,
        "outside actual mount coverage"
    );
    world.run_system_once(tick).unwrap();
    assert_eq!(
        energy(&world, source),
        paid,
        "same paid boundary does not run twice"
    );
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    assert_eq!(
        factor(&mut world, Vec3::ZERO),
        1.0,
        "prior paid interval cannot be reused"
    );
    command(
        &mut world,
        source,
        Control::SetJammingEnabled { enabled: false },
    );
    world.run_system_once(tick).unwrap();
    assert_eq!(energy(&world, source), paid);
    assert_eq!(
        factor(&mut world, Vec3::ZERO),
        1.0,
        "explicit stop recovers the physical link"
    );
}
#[test]
fn retained_target_cannot_borrow_current_hidden_pose_offline_work_or_stale_evidence() {
    for fault in 0..3 {
        let (mut world, source, target, contact) = fixture();
        prepare(&mut world, source, &contact);
        match fault {
            0 => world
                .get_mut::<ShipSystemControlSources>(source)
                .unwrap()
                .0
                .set(
                    id(OWNER),
                    crate::ship::control_source::ControlSource::Offline,
                ),
            1 => world.resource_mut::<crate::sim_tick::SimTick>().0 += 121,
            _ => {
                crate::gm::operations::contact::set(
                    &mut world
                        .resource_mut::<crate::world::server::WorldContentRuntime>()
                        .contact_overrides,
                    "jam-source",
                    "actual-receiver",
                    crate::gm::operations::contact::ContactMode::Conceal,
                );
            }
        }
        world.get_mut::<ShipPhysics>(target).unwrap().z = -1.0;
        let before = energy(&world, source);
        world.run_system_once(tick).unwrap();
        assert_eq!(energy(&world, source), before, "fault{fault}");
        assert_eq!(
            factor(&mut world, Vec3::ZERO),
            1.0,
            "no paid emission in unavailable interval"
        );
        let state = world.get::<ShipJamming>(source).unwrap();
        let config = world.get::<ShipConfigComponent>(source).unwrap();
        assert_eq!(state.target(&config.0, &id(OWNER)), Some(contact.as_str()));
    }
}

#[test]
fn vacant_ordinary_comms_tree_reads_only_its_bound_jam_request_then_admits_owned_work() {
    use crate::core::messages::*;
    for bound in [false, true] {
        let (mut world, source, _, contact) = fixture();
        world
            .get_mut::<crate::server_app::ShipSystemBlackboards>(source)
            .unwrap()
            .0
            .insert(
                crate::ship::system_registry::viewscreen_system_id(),
                SystemBlackboard::Viewscreen(ViewscreenBlackboard {
                    scored_objectives: vec![ScoredObjective {
                        id: "interference-task".into(),
                        score: 50.0,
                        directive: AiDirective::Jam {
                            target: contact.clone(),
                        },
                        source: ObjectiveSource::Mission,
                        relevance: vec![SystemAffinity::Comms],
                        snapshot: ObjectiveSnapshot {
                            id: "interference-task".into(),
                            text: "jamming.mission.support".into(),
                            text_params: Default::default(),
                            mandatory: false,
                            status: ObjectiveStatus::Active,
                            unassigned: false,
                            progress: None,
                            targets: vec![contact.clone()],
                            source: ObjectiveSource::Mission,
                        },
                    }],
                    ..Default::default()
                }),
            );
        if !bound {
            world
                .get_mut::<ShipConfigComponent>(source)
                .unwrap()
                .0
                .consoles
                .iter_mut()
                .find(|console| console.id.0 == "comms")
                .unwrap()
                .feeds
                .retain(|feed| feed.source != crate::ship::system_registry::viewscreen_system_id());
        }
        let mut app = App::new();
        std::mem::swap(app.world_mut(), &mut world);
        app.world_mut()
            .init_resource::<bevy::ecs::schedule::Schedules>();
        app.configure_sets(
            FixedUpdate,
            (
                crate::sim_sets::SimSet::Input,
                crate::sim_sets::SimSet::Physics,
            )
                .chain(),
        );
        crate::ai::host::register_ai_host_env(&mut app);
        crate::science::reading::register(&mut app);
        register(&mut app);
        let initial = energy(app.world(), source);
        for now in 360..=391 {
            app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = now;
            app.world_mut().run_schedule(FixedUpdate);
            if now < 390 {
                assert_eq!(
                    energy(app.world(), source),
                    initial,
                    "reading cannot emit work"
                );
                assert!(!app.world().get::<ShipJamming>(source).unwrap().enabled(
                    &app.world().get::<ShipConfigComponent>(source).unwrap().0,
                    &id(OWNER)
                ));
            }
        }
        let commands = &app.world().get::<AdmittedCommands>(source).unwrap().0;
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command.payload, Control::SetJammingTarget { .. }))
                .count(),
            usize::from(bound)
        );
        if bound {
            assert!(commands.iter().all(|command| command.target == id(OWNER)
                && command.response_token.as_deref() == Some("ai:jam-source")));
            assert!(energy(app.world(), source) < initial);
            assert!(app
                .world()
                .get::<JammingEmissions>(source)
                .unwrap()
                .entries
                .contains_key(&id(OWNER)));
        } else {
            assert_eq!(
                energy(app.world(), source),
                initial,
                "another Console's current primary cannot substitute for an unbound request"
            );
        }
    }
}

#[test]
fn stationary_or_missing_physical_prerequisites_settle_empty_instead_of_jamming_every_link_forever()
{
    for missing_power in [false, true] {
        let (mut world, source, _, contact) = fixture();
        prepare(&mut world, source, &contact);
        world
            .entity_mut(source)
            .remove::<crate::server_app::Ship>()
            .remove::<ShipPhysics>();
        if missing_power {
            world.entity_mut(source).remove::<ShipPowerSystem>();
        }
        world.run_system_once(tick).unwrap();
        assert!(world
            .get::<JammingEmissions>(source)
            .unwrap()
            .processed_tick
            .is_some());
        assert_eq!(
            factor(&mut world, Vec3::ZERO),
            if missing_power { 1.0 } else { 0.2 }
        );
        if missing_power {
            assert!(world
                .get::<JammingEmissions>(source)
                .unwrap()
                .entries
                .is_empty());
        }
    }
}

#[test]
fn shared_actual_signal_gate_scales_each_endpoint_once_and_recovers_after_stop() {
    let (mut world, source, _, contact) = fixture();
    prepare(&mut world, source, &contact);
    world.run_system_once(tick).unwrap();
    let reaches = |world: &mut World, range: f32| {
        world
            .run_system_once(
                move |paths: crate::regions::signal_path::RegionSignalQuery| {
                    super::super::server::signal_comms_in_range(
                        &paths,
                        Vec3::ZERO,
                        Vec3::new(0.0, 0.0, -100.0),
                        range,
                        400.0,
                    )
                },
            )
            .unwrap()
    };
    assert!(
        reaches(&mut world, 501.0),
        "one factor per endpoint leaves both physical reaches above100"
    );
    assert!(
        !reaches(&mut world, 499.0),
        "jammer's own receiver is also exposed"
    );
    command(
        &mut world,
        source,
        Control::SetJammingEnabled { enabled: false },
    );
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    world.run_system_once(tick).unwrap();
    assert!(
        reaches(&mut world, 101.0),
        "same signal owner recovers after an actual stop boundary"
    );
}

#[test]
fn actual_late_spawn_never_creates_free_global_denial_or_erases_current_paid_peers() {
    let (mut world, source, _, contact) = fixture();
    prepare(&mut world, source, &contact);
    world.run_system_once(tick).unwrap();
    assert_eq!(factor(&mut world, Vec3::ZERO), 0.2);
    // Same accepted spawn path used by a World action after the Input boundary.
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/jamming_cruiser.toml",
    )
    .unwrap();
    let late = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::new(10000.0, 0.0, 0.0),
        "late-disabled-jammer".into(),
        None,
    );
    world.flush();
    let before = energy(&world, late);
    assert_eq!(
        factor(&mut world, Vec3::ZERO),
        0.2,
        "late unpaid gear cannot erase actual paid peer influence"
    );
    let notice = |world: &mut World| {
        world
            .run_system_once(|paths: crate::regions::signal_path::RegionSignalQuery| {
                paths.comms_receiver_interference(Vec3::ZERO)
            })
            .unwrap()
    };
    assert_eq!(
        notice(&mut world),
        Some(true),
        "actual current effect is still locally knowable"
    );
    world.despawn(source);
    assert_eq!(
        factor(&mut world, Vec3::ZERO),
        1.0,
        "only late disabled equipment remains, so no physical effect"
    );
    assert_eq!(
        notice(&mut world),
        None,
        "unprocessed receiver knowledge stays unknown"
    );
    assert_eq!(
        energy(&world, late),
        before,
        "receiver query never buys work"
    );
    world.run_system_once(tick).unwrap();
    assert_eq!(factor(&mut world, Vec3::ZERO), 1.0);
    assert_eq!(notice(&mut world), Some(false));
    assert_eq!(
        energy(&world, late),
        before,
        "disabled actual owner still cannot emit"
    );
}

#[test]
fn actual_radio_loses_friendly_and_other_faction_links_without_changing_local_emission_then_recovers(
) {
    use crate::world_id::InstallWorldIdMint;
    use phoenix_model::comms_channel::CommsAudience;
    let mut local = Vec::new();
    for faction in [
        "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa",
        "cccccccc-3333-4333-8333-cccccccccccc",
    ] {
        for enabled in [false, true] {
            let (mut world, source, _, contact) = fixture_with_faction(Some(faction));
            world.insert_world_id_mint(crate::world_id::WorldIdMint::default());
            // A distinct ordinary sender is on the authored jammer range edge:
            // zero exposure here isolates the receiving radio's degradation.
            let config = crate::entities::include_resolve::load_entity_config(
                "assets/entities/alliance_cruiser.toml",
            )
            .unwrap();
            let sender = crate::entities::spawner::spawn_entity(
                &mut world.commands(),
                &config,
                Vec3::new(0.0, 0.0, -200.0),
                "actual-sender".into(),
                None,
            );
            world.flush();
            if enabled {
                prepare(&mut world, source, &contact);
            }
            world.run_system_once(tick).unwrap();
            assert_eq!(factor(&mut world, Vec3::new(0.0, 0.0, -200.0)), 1.0);
            assert_eq!(
                factor(&mut world, Vec3::new(0.0, 0.0, -20.0)) < 1.0,
                enabled
            );
            let send = |world: &mut World| {
                world.get_mut::<AdmittedCommands>(sender).unwrap().0 = vec![AdmittedCommand {
                    target: id("comms"),
                    payload: Control::SendCommsText {
                        channel: "public".into(),
                        audience: CommsAudience::All,
                        body: "Local emission only".into(),
                    },
                    response_token: None,
                    feedback_correlation: None,
                }];
                world
                    .run_system_once(super::super::radio::tick_radio)
                    .unwrap();
                world.get_mut::<AdmittedCommands>(sender).unwrap().0.clear();
            };
            let messages = |world: &World, recipient: &str| {
                world
                    .resource::<super::super::server::CommsInboxRes>()
                    .0
                    .messages()
                    .into_iter()
                    .filter(|message| message.is_for_ship(Some(recipient)))
                    .collect::<Vec<_>>()
            };
            send(&mut world);
            let sent = messages(&world, "actual-sender");
            assert_eq!(sent.len(), 1);
            assert_eq!(
                messages(&world, "actual-receiver").len(),
                usize::from(!enabled),
                "actual receiver faction {faction} has no immunity"
            );
            local.push((
                sent,
                world
                    .get::<ShipPowerSystem>(sender)
                    .unwrap()
                    .0
                    .network()
                    .consumers[&id("comms")]
                    .energy,
            ));
            command(
                &mut world,
                source,
                Control::SetJammingEnabled { enabled: false },
            );
            world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
            world.run_system_once(tick).unwrap();
            send(&mut world);
            assert_eq!(
                messages(&world, "actual-receiver").len(),
                if enabled { 1 } else { 2 },
                "another admitted transmission uses the actual recovered link"
            );
        }
    }
    assert!(local.windows(2).all(|pair|pair[0]==pair[1]),"sender's original fact and finite radio debit never reveal hidden reception or affiliation");
}

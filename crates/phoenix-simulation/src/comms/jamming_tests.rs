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
                paths.comms_receiver_factor(point, None)
            },
        )
        .unwrap()
}
fn fixture() -> (World, Entity, Entity, String) {
    fixture_with_faction(None)
}
fn fixture_with_faction(faction: Option<&str>) -> (World, Entity, Entity, String) {
    fixture_for_effect(
        faction,
        phoenix_sim_gameplay::electronic::jamming::JammingEffect::Comms,
        false,
    )
}
pub(crate) fn fixture_for_effect(
    faction: Option<&str>,
    effect: phoenix_sim_gameplay::electronic::jamming::JammingEffect,
    protected: bool,
) -> (World, Entity, Entity, String) {
    let source =
        crate::entities::include_resolve::resolve_from_disk("assets/entities/jamming_cruiser.toml")
            .unwrap()
            .toml;
    let mut raw: toml::Value = toml::from_str(&source).unwrap();
    let system = raw["system"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|system| system["id"].as_str() == Some(OWNER))
        .unwrap();
    let component = system["component"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|component| component["kind"].as_str() == Some("jamming_emitter"))
        .unwrap();
    // The shipped profile omits the default Comms effect. TOML's IndexMut
    // requires an existing key; insert this actual authored test variation.
    component
        .get_mut("config")
        .and_then(toml::Value::as_table_mut)
        .expect("actual jamming component has a configuration table")
        .insert(
            "effect".into(),
            match effect {
                phoenix_sim_gameplay::electronic::jamming::JammingEffect::Comms => "comms",
                phoenix_sim_gameplay::electronic::jamming::JammingEffect::Sensors => "sensors",
            }
            .into(),
        );
    if effect == phoenix_sim_gameplay::electronic::jamming::JammingEffect::Sensors {
        // Explicit accepted finite source receiver: self-exposure must not make
        // this intended sustained producer lose its own20-unit measured aim.
        // The affected target retains its real50-unit Tactical instrument.
        raw["weapons_console"]["radar"]["range"] = 200.0.into();
    }
    let config =
        crate::entities::config::EntityConfig::from_toml(&toml::to_string(&raw).unwrap()).unwrap();
    assert_eq!(
        phoenix_sim_gameplay::ship::composition::jamming_composition(
            &config
                .ship_config
                .as_ref()
                .unwrap()
                .system(&id(OWNER))
                .unwrap()
                .components
        )
        .unwrap()
        .config
        .effect,
        effect,
        "fixture must select actual accepted effect hardware"
    );
    let target_config = crate::entities::include_resolve::load_entity_config(if protected {
        "assets/entities/protection_cruiser.toml"
    } else {
        "assets/entities/alliance_cruiser.toml"
    })
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
pub(crate) fn prepare(world: &mut World, source: Entity, contact: &str) {
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
                        super::super::server::SignalEndpoint {
                            point: Vec3::ZERO,
                            range,
                            receiver: None,
                        },
                        super::super::server::SignalEndpoint {
                            point: Vec3::new(0.0, 0.0, -100.0),
                            range: 400.0,
                            receiver: None,
                        },
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
                paths.comms_receiver_interference(Vec3::ZERO, None)
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

fn owned_factor(
    world: &mut World,
    point: Vec3,
    observer: &str,
    system: &str,
    sensors: bool,
) -> f32 {
    let observer = observer.to_owned();
    let system = id(system);
    world
        .run_system_once(
            move |paths: crate::regions::signal_path::RegionSignalQuery| {
                if sensors {
                    paths.sensors_receiver_factor(point, &observer, &system)
                } else {
                    paths.comms_receiver_factor(point, Some((&observer, &system)))
                }
            },
        )
        .unwrap()
}
#[test]
fn actual_paid_protection_reduces_only_exact_current_receiver_exposure_without_coordinate_ownership(
) {
    let (mut world, source, receiver, contact) = fixture_for_effect(
        None,
        phoenix_sim_gameplay::electronic::jamming::JammingEffect::Comms,
        true,
    );
    prepare(&mut world, source, &contact);
    world.run_system_once(tick).unwrap();
    let point = Vec3::new(0.0, 0.0, -20.0);
    let before = owned_factor(&mut world, point, "actual-receiver", "comms", false);
    assert!(before > 0.0 && before < 1.0);
    let link = |world: &mut World| {
        world
            .run_system_once(|paths: crate::regions::signal_path::RegionSignalQuery| {
                super::super::server::signal_comms_in_range(
                    &paths,
                    super::super::server::SignalEndpoint {
                        point: Vec3::ZERO,
                        range: 100.0,
                        receiver: Some(("jam-source", &id("comms"))),
                    },
                    super::super::server::SignalEndpoint {
                        point: Vec3::new(0.0, 0.0, -20.0),
                        range: 50.0,
                        receiver: Some(("actual-receiver", &id("comms"))),
                    },
                )
            })
            .unwrap()
    };
    assert!(
        !link(&mut world),
        "unprotected actual receiving endpoint denies link"
    );
    let reserve = world
        .get::<ShipPowerSystem>(receiver)
        .unwrap()
        .0
        .network()
        .consumers[&id("signal-protection")]
        .energy;
    world
        .run_system_once(crate::science::protection::tick)
        .unwrap();
    assert!(
        world
            .get::<ShipPowerSystem>(receiver)
            .unwrap()
            .0
            .network()
            .consumers[&id("signal-protection")]
            .energy
            < reserve
    );
    assert_eq!(
        owned_factor(&mut world, point, "actual-receiver", "comms", false),
        1.0
    );
    assert!(
        link(&mut world),
        "paid exact receiver protection restores the same link"
    );
    assert_eq!(
        owned_factor(
            &mut world,
            point,
            "actual-receiver",
            "tactical-radar",
            false
        ),
        before,
        "unallocated actual System cannot borrow protection"
    );
    assert_eq!(
        owned_factor(&mut world, point, "another-actor", "comms", false),
        before,
        "coincident coordinates do not imply same equipment owner"
    );
    assert_eq!(
        factor(&mut world, point),
        before,
        "range-only endpoint has no owned protection"
    );
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    world.run_system_once(tick).unwrap();
    assert_eq!(
        owned_factor(&mut world, point, "actual-receiver", "comms", false),
        before,
        "prior protection work cannot be reused"
    );
    world
        .run_system_once(crate::science::protection::tick)
        .unwrap();
    assert_eq!(
        owned_factor(&mut world, point, "actual-receiver", "comms", false),
        1.0
    );
}
fn fund_radar(world: &mut World, receiver: Entity) {
    let grant = world
        .get_mut::<ShipPowerSystem>(receiver)
        .unwrap()
        .0
        .work_continuous(&id("tactical-radar"), 1.0 / 60.0, 1.0)
        .unwrap();
    assert!(
        grant.granted > 0.0,
        "the actual receiver radar remains paid"
    );
}
#[test]
fn sensors_effect_actual_broad_acquisition_loss_and_priority_recovery_preserve_held_original_sample(
) {
    use phoenix_sim_gameplay::ship::composition::CommandChannelValue;
    let (mut world, source, receiver, contact) = fixture_for_effect(
        None,
        phoenix_sim_gameplay::electronic::jamming::JammingEffect::Sensors,
        true,
    );
    prepare(&mut world, source, &contact);
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    let baseline = world.resource::<crate::sim_tick::SimTick>().0;
    fund_radar(&mut world, receiver);
    world.run_system_once(crate::science::broad::tick).unwrap();
    assert!(world
        .get::<crate::science::broad::BroadTracks>(receiver)
        .unwrap()
        .0[&id("tactical-radar")]
        .current
        .contains_key("jam-source"));
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    world.run_system_once(tick).unwrap();
    assert_eq!(
        owned_factor(
            &mut world,
            Vec3::new(0.0, 0.0, -20.0),
            "actual-receiver",
            "comms",
            false
        ),
        1.0,
        "Sensors effect is not a Comms effect"
    );
    fund_radar(&mut world, receiver);
    world.run_system_once(crate::science::broad::tick).unwrap();
    let now = world.resource::<crate::sim_tick::SimTick>().0;
    let profile = &world
        .get::<crate::science::broad::BroadProfile>(receiver)
        .unwrap()
        .0[&id("tactical-radar")]
        .profile;
    let recovery_ticks = (profile.sample_interval_secs
        / world.resource::<Time<Fixed>>().timestep().as_secs_f32())
    .ceil() as u64
        + 1;
    let state = &world
        .get::<crate::science::broad::BroadTracks>(receiver)
        .unwrap()
        .0[&id("tactical-radar")];
    assert!(
        !state.current.contains_key("jam-source"),
        "paid jammer makes20 outside actual50 scaled reach"
    );
    let held = state.projection(profile, now, 1.0 / 60.0).unwrap();
    assert!(
        held.iter()
            .any(|record| record.predicted && record.observed_tick == baseline),
        "equipment denial retains old dated evidence"
    );
    world
        .get_mut::<crate::science::protection::ShipSignalProtection>(receiver)
        .unwrap()
        .0
        .get_mut(&id("signal-protection"))
        .unwrap()
        .set(
            "priorities",
            CommandChannelValue::SystemPriority(vec![
                id("tactical-radar"),
                id("comms"),
                id("sensors"),
            ]),
        )
        .unwrap();
    world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
    world
        .run_system_once(crate::science::protection::tick)
        .unwrap();
    world.run_system_once(tick).unwrap();
    assert!(
        world
            .get::<JammingEmissions>(source)
            .unwrap()
            .entries
            .contains_key(&id(OWNER)),
        "recovery must not be transmitter loss"
    );
    fund_radar(&mut world, receiver);
    world.run_system_once(crate::science::broad::tick).unwrap();
    assert_eq!(
        owned_factor(
            &mut world,
            Vec3::new(0.0, 0.0, -20.0),
            "actual-receiver",
            "tactical-radar",
            true,
        ),
        1.0,
        "current paid allocated protection restores the actual receiver factor"
    );
    // Denial removes current acquisition immediately; a recovered receiver
    // must still wait for its actual authored sampling cadence. Continue real
    // funded work, without forcing a sample clock, refreshing the held record
    // or letting transmitter failure stand in for successful protection.
    for _ in 0..recovery_ticks {
        if world
            .get::<crate::science::broad::BroadTracks>(receiver)
            .unwrap()
            .0[&id("tactical-radar")]
            .current
            .contains_key("jam-source")
        {
            break;
        }
        world.resource_mut::<crate::sim_tick::SimTick>().0 += 1;
        let mut power = world.query::<&mut ShipPowerSystem>();
        for mut power in power.iter_mut(&mut world) {
            power.0.begin_work_interval();
        }
        let before = energy(&world, source);
        let protection_before = world
            .get::<ShipPowerSystem>(receiver)
            .unwrap()
            .0
            .network()
            .consumers[&id("signal-protection")]
            .energy;
        world
            .run_system_once(crate::science::protection::tick)
            .unwrap();
        world.run_system_once(tick).unwrap();
        assert!(
            energy(&world, source) < before,
            "the jammer stays genuinely paid"
        );
        assert!(
            world
                .get::<ShipPowerSystem>(receiver)
                .unwrap()
                .0
                .network()
                .consumers[&id("signal-protection")]
                .energy
                < protection_before,
            "each recovery interval uses actual finite protection work"
        );
        assert!(world
            .get::<JammingEmissions>(source)
            .unwrap()
            .entries
            .contains_key(&id(OWNER)));
        assert_eq!(
            owned_factor(
                &mut world,
                Vec3::new(0.0, 0.0, -20.0),
                "actual-receiver",
                "tactical-radar",
                true
            ),
            1.0
        );
        fund_radar(&mut world, receiver);
        world.run_system_once(crate::science::broad::tick).unwrap();
    }
    assert!(
        world
            .get::<crate::science::broad::BroadTracks>(receiver)
            .unwrap()
            .0[&id("tactical-radar")]
            .current
            .contains_key("jam-source"),
        "own paid priority allocation restores actual acquisition"
    );
    let state = &world
        .get::<crate::science::broad::BroadTracks>(receiver)
        .unwrap()
        .0[&id("tactical-radar")];
    let track = &state.tracks[&state.current["jam-source"]];
    assert!(
        track.observed_tick > baseline,
        "recovery requires a genuine new instrument sample"
    );
}

#[test]
fn actual_focused_fidelity_uses_the_same_receiver_factor_once_and_paid_protection_restores_precision(
) {
    use phoenix_sim_gameplay::ship::composition::CommandChannelValue;
    let mut bands = Vec::new();
    for protect in [false, true] {
        let (mut world, source, receiver, contact) = fixture_for_effect(
            None,
            phoenix_sim_gameplay::electronic::jamming::JammingEffect::Sensors,
            true,
        );
        prepare(&mut world, source, &contact);
        world
            .get_mut::<crate::science::protection::ShipSignalProtection>(receiver)
            .unwrap()
            .0
            .get_mut(&id("signal-protection"))
            .unwrap()
            .set(
                "priorities",
                CommandChannelValue::SystemPriority(if protect {
                    vec![id("sensors")]
                } else {
                    vec![]
                }),
            )
            .unwrap();
        let first = world.resource::<crate::sim_tick::SimTick>().0 + 1;
        for now in first..first + 125 {
            world.resource_mut::<crate::sim_tick::SimTick>().0 = now;
            for (actor, owner) in [(source, "tactical-radar"), (receiver, "sensor-radar")] {
                world
                    .get_mut::<ShipPowerSystem>(actor)
                    .unwrap()
                    .0
                    .work_continuous(&id(owner), 1.0 / 60.0, 1.0)
                    .unwrap();
            }
            world.run_system_once(crate::science::broad::tick).unwrap();
            world
                .run_system_once(
                    crate::console::weapons::blackboard::publish_tactical_radar_blackboard,
                )
                .unwrap();
            world
                .run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard)
                .unwrap();
            if now == first {
                let alias = world
                    .get::<crate::science::broad::BroadTracks>(receiver)
                    .unwrap()
                    .0[&id("sensor-radar")]
                    .contact_for_source("jam-source")
                    .unwrap()
                    .to_owned();
                world
                    .get_mut::<AdmittedCommands>(receiver)
                    .unwrap()
                    .0
                    .push(AdmittedCommand {
                        target: id("sensors"),
                        payload: Control::ScanTarget {
                            uuid: alias,
                            aim: Default::default(),
                            intrusive: false,
                        },
                        response_token: None,
                        feedback_correlation: None,
                    });
            }
            world
                .run_system_once(crate::science::protection::tick)
                .unwrap();
            world.run_system_once(tick).unwrap();
            assert!(world
                .get::<JammingEmissions>(source)
                .unwrap()
                .entries
                .contains_key(&id(OWNER)));
            world.run_system_once(crate::science::tick_scans).unwrap();
            world
                .get_mut::<AdmittedCommands>(receiver)
                .unwrap()
                .0
                .clear();
            let record = &world
                .get::<crate::science::ShipScanRecord>(receiver)
                .unwrap()
                .0[&id("sensors")];
            if let Some(reading) = &record.last {
                bands.push(reading.band.clone());
                break;
            }
        }
        assert_eq!(
            bands.len(),
            if protect { 2 } else { 1 },
            "actual finite focused work must complete"
        );
    }
    assert_eq!(bands,vec!["coarse","detailed"],"equipment loss applies once on existing fidelity ladder; funded exact protection restores precision");
}

#[test]
fn missing_actual_source_withholds_jammer_controls_and_refuses_retained_intent_changes() {
    let (mut world, source, _, contact) = fixture();
    prepare(&mut world, source, &contact);
    world.entity_mut(source).insert(ShipSystemControlSources(
        crate::ship::control_source::ControlSourceResolver::new(),
    ));
    let before = world.get::<ShipJamming>(source).unwrap().clone();
    assert!(!project(&world, source, &id(OWNER)).unwrap().controllable);
    command(
        &mut world,
        source,
        Control::SetJammingEnabled { enabled: false },
    );
    command(
        &mut world,
        source,
        Control::SetJammingTarget { target_uuid: None },
    );
    assert_eq!(world.get::<ShipJamming>(source).unwrap(), &before);
}

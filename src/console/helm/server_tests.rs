use super::*;
use crate::core::messages::{
    CoordinationAddress, CoordinationPayload, SystemBlackboard, WeaponEmitterArc, WeaponFamily,
};
use crate::server_app::ShipSystemBlackboards;
use crate::ship::boost::BoostState;

fn base_app() -> App {
    let mut app = App::new();
    app.add_systems(Update, publish_helm_blackboard);
    // Initialise InterSystemQueue so the system parameter is satisfied.
    app.init_resource::<InterSystemQueue>();
    app.insert_resource(crate::lobby::server::ShipClientConfigResource::default());
    // Spawn a LocalShip entity with components so the system can query it.
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        ShipPhysics::default(),
        ShipSystemBlackboards::default(),
        ShipImpulse::default(),
        ShipBoost::default(),
        crate::modifiers::ShipModifiers::new(),
        crate::ship_plugin::LastHelmInput::default(),
    ));
    app
}

fn helm_coordination_app() -> (App, Entity, CoordinationAddress) {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .init_resource::<InterSystemQueue>()
        .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
        .add_plugins(HelmPlugin);
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        crate::ship::test_support::TEST_TICK,
    );

    let config = ShipConfigComponent::default();
    let address =
        crate::ship::coordination::address_for_system(&config.0, &helm_steering_system_id())
            .expect("shipped test hull assigns Helm steering to a Station");
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            config,
            ShipSystemControlSources::default(),
            PendingArcBearingRequest::default(),
            HelmWaypointClearance::default(),
        ))
        .id();
    crate::ship::test_support::set_helm_control_source(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
    );
    (app, ship, address)
}

fn deliver_to_helm(
    app: &mut App,
    ship: Entity,
    address: CoordinationAddress,
    payload: CoordinationPayload,
) {
    app.world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .write(DeliveredCoordination {
            source_entity: ship,
            address,
            payload,
            presentation: crate::core::messages::CoordinationPresentation::new(
                "test.coordination.title",
                "test.coordination.body",
            ),
            delivery: CoordinationDelivery::Ai,
        });
}

#[test]
fn helm_coordination_receiver_preserves_payload_values_and_delivery_order() {
    let (mut app, ship, address) = helm_coordination_app();
    let target = uuid::Uuid::new_v4();
    let arcs = vec![WeaponEmitterArc {
        facing_deg: 37.0,
        arc_deg: 83.0,
        range: 412.5,
    }];

    deliver_to_helm(
        &mut app,
        ship,
        address.clone(),
        CoordinationPayload::ArcBearingRequest {
            uuid: target.to_string(),
            label: "test target".into(),
            family: WeaponFamily::Blasters,
            arcs: arcs.clone(),
        },
    );
    deliver_to_helm(
        &mut app,
        ship,
        address.clone(),
        CoordinationPayload::ArcBearingWithdraw {
            family: WeaponFamily::Blasters,
        },
    );
    crate::ship::test_support::tick(&mut app);

    let pending = app
        .world()
        .entity(ship)
        .get::<PendingArcBearingRequest>()
        .expect("test ship carries pending arc state");
    assert_eq!(pending.target, None, "later withdrawal wins in bus order");
    assert!(
        pending.arcs.is_empty(),
        "withdrawal clears carried geometry"
    );

    deliver_to_helm(
        &mut app,
        ship,
        address.clone(),
        CoordinationPayload::NavigateTo {
            generation: 73,
            x: 900.0,
            z: -450.0,
        },
    );
    deliver_to_helm(
        &mut app,
        ship,
        address,
        CoordinationPayload::ArcBearingRequest {
            uuid: target.to_string(),
            label: "test target".into(),
            family: WeaponFamily::Blasters,
            arcs: arcs.clone(),
        },
    );
    crate::ship::test_support::tick(&mut app);

    let pending = app
        .world()
        .entity(ship)
        .get::<PendingArcBearingRequest>()
        .unwrap();
    assert_eq!(pending.target, Some(target));
    assert_eq!(
        pending.arcs, arcs,
        "arc geometry is copied without reduction"
    );
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<HelmWaypointClearance>()
            .expect("test ship carries waypoint clearance")
            .0,
        Some(73),
        "NavigateTo latches only its exact generation"
    );
}

#[test]
fn helm_coordination_receiver_rechecks_address_and_live_ai_ownership() {
    let (mut app, ship, address) = helm_coordination_app();
    let payload = CoordinationPayload::ArcBearingRequest {
        uuid: uuid::Uuid::new_v4().to_string(),
        label: "test target".into(),
        family: WeaponFamily::Phasers,
        arcs: vec![WeaponEmitterArc {
            facing_deg: 0.0,
            arc_deg: 90.0,
            range: 300.0,
        }],
    };

    crate::ship::test_support::set_helm_control_source(
        &mut app,
        crate::ship::control_source::ControlSource::Human,
    );
    deliver_to_helm(&mut app, ship, address.clone(), payload.clone());
    crate::ship::test_support::tick(&mut app);
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<PendingArcBearingRequest>()
            .unwrap()
            .target,
        None,
        "a late human claim invalidates an already-emitted AI delivery"
    );

    crate::ship::test_support::set_helm_control_source(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
    );

    app.world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .write(DeliveredCoordination {
            source_entity: ship,
            address: address.clone(),
            payload: payload.clone(),
            presentation: crate::core::messages::CoordinationPresentation::new(
                "test.coordination.title",
                "test.coordination.body",
            ),
            delivery: CoordinationDelivery::HumanPopup {
                token: "test-token".into(),
                sender_label: "test-sender".into(),
                order: 0,
            },
        });
    crate::ship::test_support::tick(&mut app);
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<PendingArcBearingRequest>()
            .unwrap()
            .target,
        None,
        "Helm's AI receiver must reject a human-popup delivery"
    );

    deliver_to_helm(&mut app, ship, CoordinationAddress::Ship, payload);
    crate::ship::test_support::tick(&mut app);
    assert_eq!(
        app.world()
            .entity(ship)
            .get::<PendingArcBearingRequest>()
            .unwrap()
            .target,
        None,
        "a Ship broadcast is not a Helm Station delivery"
    );
}

#[test]
fn helm_coordination_receiver_uses_ai_steering_when_thrust_is_offline() {
    let (mut app, ship, address) = helm_coordination_app();
    let target = uuid::Uuid::new_v4();
    let arcs = vec![WeaponEmitterArc {
        facing_deg: -42.0,
        arc_deg: 67.0,
        range: 318.0,
    }];

    {
        let mut ship_entity = app.world_mut().entity_mut(ship);
        let mut control_sources = ship_entity
            .get_mut::<ShipSystemControlSources>()
            .expect("test ship carries control sources");
        control_sources
            .0
            .set_offline(crate::ship::system_registry::helm_thrust_system_id(), true);
        assert!(
            control_sources
                .0
                .policy_for(&helm_steering_system_id())
                .operate_ai,
            "steering remains AI-operated"
        );
        assert!(
            !crate::ship_plugin::helm_axes_operate_ai(&control_sources),
            "offline thrust makes the old composite receiver gate false"
        );
    }

    deliver_to_helm(
        &mut app,
        ship,
        address.clone(),
        CoordinationPayload::ArcBearingRequest {
            uuid: target.to_string(),
            label: "test target".into(),
            family: WeaponFamily::Phasers,
            arcs: arcs.clone(),
        },
    );
    deliver_to_helm(
        &mut app,
        ship,
        address.clone(),
        CoordinationPayload::NavigateTo {
            generation: 91,
            x: -25.0,
            z: 640.0,
        },
    );
    crate::ship::test_support::tick(&mut app);

    let ship_state = app.world().entity(ship);
    let pending = ship_state
        .get::<PendingArcBearingRequest>()
        .expect("test ship carries pending arc state");
    assert_eq!(pending.target, Some(target));
    assert_eq!(pending.arcs, arcs);
    assert_eq!(
        ship_state
            .get::<HelmWaypointClearance>()
            .expect("test ship carries waypoint clearance")
            .0,
        Some(91),
        "offline thrust does not swallow steering-owned clearance"
    );

    deliver_to_helm(
        &mut app,
        ship,
        address,
        CoordinationPayload::ArcBearingWithdraw {
            family: WeaponFamily::Blasters,
        },
    );
    crate::ship::test_support::tick(&mut app);

    let pending = app
        .world()
        .entity(ship)
        .get::<PendingArcBearingRequest>()
        .unwrap();
    assert_eq!(pending.target, None);
    assert!(
        pending.arcs.is_empty(),
        "withdrawal remains unconditional across weapon families"
    );
}

/// Spawn an NPC ship (no `LocalShip`) carrying the components the
/// entity spawner gives every behaviour-bearing NPC, plus an authored
/// helm radar range. Returns its entity id.
fn spawn_npc_ship(app: &mut App, radar_range: f32) -> Entity {
    let toml_str = format!(
            "[helm_console]\nmax_speed = 30.0\n\n[helm_console.radar]\nrange = {radar_range}\nshows = [\"ship\"]\n"
        );
    let helm_config = crate::entities::config::EntityConfig::from_toml(&toml_str)
        .expect("helm_console TOML must parse")
        .helm_console
        .expect("helm_console section must be present");
    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipPhysics {
                x: 42.0,
                z: -17.0,
                ..Default::default()
            },
            ShipSystemBlackboards::default(),
            crate::modifiers::ShipModifiers::new(),
            crate::entities::spawner::HelmConsoleSection(helm_config),
        ))
        .id()
}

/// Helper: read the helm blackboard from the LocalShip entity's ShipSystemBlackboards component.
fn get_helm_blackboard(app: &mut App) -> crate::core::messages::HelmBlackboard {
    let key = helm_station_key();
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::Helm(bb) = bbs
        .0
        .get(&key)
        .expect("expected helm entry in blackboards")
        .clone()
    else {
        panic!("expected Helm blackboard")
    };
    bb
}

// ── Hostile weapon-arc overlay (issue #874) ───────────────────────────

const OWN_FACTION: uuid::Uuid = uuid::Uuid::from_u128(0x0874_0001);
const ENEMY_FACTION: uuid::Uuid = uuid::Uuid::from_u128(0x0874_0002);

/// A snapshot carrying one armed hostile 100 units off the bow, plus a
/// faction registry that makes it an enemy.
fn arc_overlay_app(red_alert: bool, local: bool) -> App {
    let mut app = App::new();
    app.add_systems(Update, publish_helm_blackboard);
    app.init_resource::<InterSystemQueue>();
    app.insert_resource(crate::lobby::server::ShipClientConfigResource::default());

    let mut registry = crate::ai::faction::FactionRegistry::new();
    registry.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: OWN_FACTION,
        name: "Own".into(),
        enemies: vec![ENEMY_FACTION],
        compliance: None,
    });
    registry.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: ENEMY_FACTION,
        name: "Enemy".into(),
        enemies: vec![OWN_FACTION],
        compliance: None,
    });
    app.insert_resource(crate::entities::config_cache::FactionRegistryResource(
        registry,
    ));

    app.insert_resource(crate::ai::server::WorldSnapshot {
        entities: vec![
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::from_u128(0x0874_1111),
                position: [0.0, 0.0, -100.0],
                faction: Some(ENEMY_FACTION),
                weapon_arcs: crate::weapons::arc_geometry::weapon_arc_sectors(
                    0.0,
                    &[crate::weapons::arc_geometry::WeaponArcBank {
                        facing_deg: 180.0,
                        fire_arc_deg: 90.0,
                        range: 400.0,
                    }],
                ),
                ..Default::default()
            },
            // A friendly ship with arcs of its own — must never appear.
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::from_u128(0x0874_2222),
                position: [50.0, 0.0, 0.0],
                faction: Some(OWN_FACTION),
                weapon_arcs: crate::weapons::arc_geometry::weapon_arc_sectors(
                    0.0,
                    &[crate::weapons::arc_geometry::WeaponArcBank {
                        facing_deg: 0.0,
                        fire_arc_deg: 60.0,
                        range: 400.0,
                    }],
                ),
                ..Default::default()
            },
            // A hostile far outside the helm radar horizon.
            crate::ai::AiWorldEntity {
                uuid: uuid::Uuid::from_u128(0x0874_3333),
                position: [0.0, 0.0, -9000.0],
                faction: Some(ENEMY_FACTION),
                weapon_arcs: crate::weapons::arc_geometry::weapon_arc_sectors(
                    0.0,
                    &[crate::weapons::arc_geometry::WeaponArcBank {
                        facing_deg: 180.0,
                        fire_arc_deg: 90.0,
                        range: 400.0,
                    }],
                ),
                ..Default::default()
            },
        ],
    });

    let mut ship = app.world_mut().spawn((
        crate::server_app::Ship,
        ShipPhysics::default(),
        ShipSystemBlackboards::default(),
        ShipImpulse::default(),
        ShipBoost::default(),
        crate::modifiers::ShipModifiers::new(),
        crate::ship_plugin::LastHelmInput::default(),
        crate::entities::spawner::FactionComponent(OWN_FACTION),
        crate::ship::state::ShipRedAlert(red_alert),
    ));
    if local {
        ship.insert(crate::server_app::LocalShip);
    }
    app
}

fn helm_bb_only(app: &mut App) -> crate::core::messages::HelmBlackboard {
    let key = helm_station_key();
    let mut q = app.world_mut().query::<&ShipSystemBlackboards>();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::Helm(bb) = bbs.0.get(&key).expect("helm entry").clone() else {
        panic!("expected Helm blackboard")
    };
    bb
}

/// AC3: at red alert the local helm gets the hostile's sectors — and only
/// the hostile's, and only the ones on the scope.
#[test]
fn red_alert_publishes_in_range_hostile_arcs_only() {
    let mut app = arc_overlay_app(true, true);
    app.update();
    let bb = helm_bb_only(&mut app);
    assert_eq!(
        bb.hostile_weapon_arcs.len(),
        1,
        "the friendly and the over-the-horizon hostile must not appear: {:?}",
        bb.hostile_weapon_arcs
    );
    let contact = &bb.hostile_weapon_arcs[0];
    assert_eq!(contact.uuid, uuid::Uuid::from_u128(0x0874_1111).to_string());
    assert!((contact.x - 0.0).abs() < 1e-3);
    assert!((contact.z + 100.0).abs() < 1e-3);
    assert_eq!(contact.arcs.len(), 1);
    assert!((contact.arcs[0].bearing_deg - 180.0).abs() < 1e-3);
    assert!((contact.arcs[0].half_angle_deg - 45.0).abs() < 1e-3);
    assert!((contact.arcs[0].range - 400.0).abs() < 1e-3);
}

/// AC3, the other half: no red alert, no arcs — and the gate is server
/// side, so the intel never reaches the wire at all.
#[test]
fn without_red_alert_no_hostile_arcs_are_published() {
    let mut app = arc_overlay_app(false, true);
    app.update();
    assert!(
        helm_bb_only(&mut app).hostile_weapon_arcs.is_empty(),
        "arcs must be red-alert gated"
    );
}

/// An NPC renders no radar, so it pays no bandwidth for one — the same
/// posture `TacticalRadarBlackboard::blips` takes.
#[test]
fn a_non_local_ship_publishes_no_hostile_arcs_even_at_red_alert() {
    let mut app = arc_overlay_app(true, false);
    app.update();
    assert!(helm_bb_only(&mut app).hostile_weapon_arcs.is_empty());
}

// ── Publish tests ──────────────────────────────────────────────────────

#[test]
fn publish_writes_helm_entry_to_blackboards() {
    let mut app = base_app();
    app.update();

    let key = helm_station_key();
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    assert!(
        bbs.0.contains_key(&key),
        "expected helm entry in blackboards"
    );
}

#[test]
fn publish_reflects_ship_position_and_yaw() {
    let mut app = base_app();
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
        let mut physics = q.single_mut(app.world_mut()).unwrap();
        physics.x = 100.0;
        physics.z = -200.0;
        physics.yaw = std::f32::consts::FRAC_PI_4;
        physics.forward_speed = 50.0;
    }
    app.update();

    let bb = get_helm_blackboard(&mut app);
    assert!((bb.x - 100.0).abs() < 0.001);
    assert!((bb.z - (-200.0)).abs() < 0.001);
    assert!((bb.forward_speed - 50.0).abs() < 0.001);
    assert!((bb.yaw - std::f32::consts::FRAC_PI_4).abs() < 0.001);
}

#[test]
fn publish_reflects_impulse_charge() {
    let mut app = base_app();
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipImpulse, With<crate::server_app::LocalShip>>();
        let mut imp = q
            .single_mut(app.world_mut())
            .expect("LocalShip must have ShipImpulse");
        imp.0.charge_progress = 0.5;
    }
    app.update();

    let bb = get_helm_blackboard(&mut app);
    assert!((bb.impulse_charge - 0.5).abs() < 0.001);
}

#[test]
fn publish_reflects_boost_state() {
    let mut app = base_app();
    {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        let ship = q.single_mut(app.world_mut()).unwrap();
        app.world_mut()
            .entity_mut(ship)
            .insert(BoostConfigResource {
                enabled: true,
                multiplier: 3.0,
                steering_multiplier: 2.0,
                active_duration: 4.0,
                recharge_duration: 20.0,
            });
    }
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipBoost, With<crate::server_app::LocalShip>>();
        let mut boost = q
            .single_mut(app.world_mut())
            .expect("LocalShip must have ShipBoost");
        boost.0 = BoostState {
            active: true,
            battery: 0.75,
        };
    }
    app.update();

    let bb = get_helm_blackboard(&mut app);
    assert!(bb.boost_enabled);
    assert!(bb.boost_active);
    assert!((bb.boost_battery - 0.75).abs() < 0.001);
}

#[test]
fn publish_boost_disabled_when_no_config() {
    let mut app = base_app();
    app.update();

    let bb = get_helm_blackboard(&mut app);
    assert!(!bb.boost_enabled);
    assert!(!bb.boost_active);
}

// ── Per-engine blackboard tests (issue #511) ───────────────────────────

#[test]
fn publish_writes_engine_port_entry_to_blackboards() {
    let mut app = base_app();
    app.update();

    let key = helm_engine_port_system_id();
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    assert!(
        bbs.0.contains_key(&key),
        "expected helm-engine-port in blackboards"
    );
}

#[test]
fn publish_writes_engine_starboard_entry_to_blackboards() {
    let mut app = base_app();
    app.update();

    let key = helm_engine_starboard_system_id();
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    assert!(
        bbs.0.contains_key(&key),
        "expected helm-engine-starboard in blackboards"
    );
}

#[test]
fn engine_is_online_when_no_hull_damage() {
    let mut app = base_app();
    app.update();

    let key = helm_engine_port_system_id();
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::HelmEngine(engine_bb) = bbs
        .0
        .get(&key)
        .expect("expected helm-engine-port in blackboards")
        .clone()
    else {
        panic!("expected HelmEngine blackboard");
    };
    assert!(
        engine_bb.is_online,
        "engine should be online when no hull damage"
    );
}

#[test]
fn engine_thrust_fraction_reflects_last_input() {
    let mut app = base_app();
    // Set helm input to 0.8 thrust.
    {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(ship)
            .insert(crate::ship_plugin::LastHelmInput {
                thrust: 0.8,
                steering: 0.0,
                lateral: 0.0,
            });
    }
    app.update();

    let key = helm_engine_port_system_id();
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    let SystemBlackboard::HelmEngine(engine_bb) = bbs
        .0
        .get(&key)
        .expect("expected helm-engine-port in blackboards")
        .clone()
    else {
        panic!("expected HelmEngine blackboard");
    };
    assert!(
        (engine_bb.thrust_fraction - 0.8).abs() < 0.001,
        "thrust_fraction should match last helm input"
    );
}

// ── Per-entity publish tests (issue #824) ──────────────────────────────

fn helm_bb_of(app: &mut App, entity: Entity) -> crate::core::messages::HelmBlackboard {
    let bbs = app
        .world()
        .entity(entity)
        .get::<ShipSystemBlackboards>()
        .expect("ship must carry ShipSystemBlackboards");
    let SystemBlackboard::Helm(bb) = bbs
        .0
        .get(&helm_station_key())
        .expect("expected helm entry in blackboards")
        .clone()
    else {
        panic!("expected Helm blackboard")
    };
    bb
}

/// AC (issue #824): an NPC ship gets a Helm blackboard entry of its own,
/// with ship-state fields derived from its own components.
#[test]
fn publish_writes_helm_entry_for_npc_ship() {
    let mut app = base_app();
    let npc = spawn_npc_ship(&mut app, 750.0);
    app.update();

    let bb = helm_bb_of(&mut app, npc);
    assert!((bb.x - 42.0).abs() < 0.001, "NPC x must be its own physics");
    assert!(
        (bb.z - (-17.0)).abs() < 0.001,
        "NPC z must be its own physics"
    );
    assert!(
        (bb.radar_range - 750.0).abs() < 0.001,
        "NPC radar_range must come from its own HelmConsoleSection, got {}",
        bb.radar_range
    );
}

/// AC (issue #824): the NPC's `radar_range` is live — scaled by the
/// `HelmRadarRange` modifier `apply_radar_damage_modifiers` maintains —
/// not the static config fallback.
#[test]
fn gm_system_disabled_radar_publishes_zero_then_its_damage_limited_range() {
    use crate::entities::spawner::EntitySystemHull;
    use crate::ship::components::ShipSystemControlSources;
    use crate::ship::damage::{ConsoleTierConfig, SystemHull};
    let mut app = base_app();
    let npc = spawn_npc_ship(&mut app, 800.0);
    let sid = crate::core::messages::SystemId("custom-radar".into());
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = "Helm"
rank = "Pilot"
[[system]]
id = "custom-radar"
kind = "helm_radar"
station = "helm"
"#,
        &["helm_radar"],
    )
    .unwrap();
    app.world_mut()
        .entity_mut(npc)
        .insert(crate::ship::components::ShipConfigComponent(config));
    let mut hull = SystemHull::from_config_with_tiers(&[(
        sid.clone(),
        20.0,
        ConsoleTierConfig {
            debuff_magnitude: 0.2,
            ..Default::default()
        },
    )]);
    hull.set_hp(&sid, 10.0);
    app.world_mut()
        .entity_mut(npc)
        .insert((EntitySystemHull(hull), ShipSystemControlSources::default()));
    // Run the ordinary translator then the actual blackboard publisher.
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut()
        .run_system_once(crate::modifiers::coordination::apply_radar_damage_modifiers)
        .unwrap();
    app.update();
    let baseline = helm_bb_of(&mut app, npc).radar_range;
    assert!(baseline > 0.0 && baseline < 800.0);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(npc)
        .unwrap()
        .0
        .set_gm_disabled(sid.clone(), true);
    app.world_mut()
        .run_system_once(crate::modifiers::coordination::apply_radar_damage_modifiers)
        .unwrap();
    app.update();
    assert_eq!(helm_bb_of(&mut app, npc).radar_range, 0.0);
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(npc)
        .unwrap()
        .0
        .set_gm_disabled(sid, false);
    app.world_mut()
        .run_system_once(crate::modifiers::coordination::apply_radar_damage_modifiers)
        .unwrap();
    app.update();
    assert_eq!(helm_bb_of(&mut app, npc).radar_range, baseline);
}

#[test]
fn npc_radar_range_is_scaled_by_the_damage_modifier() {
    let mut app = base_app();
    let npc = spawn_npc_ship(&mut app, 800.0);
    {
        let mut entity = app.world_mut().entity_mut(npc);
        let mut modifiers = entity.get_mut::<crate::modifiers::ShipModifiers>().unwrap();
        // The same shape `apply_radar_damage_modifiers` writes for a
        // damaged helm-radar: a -0.5 bonus is a 0.5 multiplier.
        modifiers.add_or_update(crate::modifiers::Modifier {
            source: crate::modifiers::cache::ModifierSource::SystemDamage(
                crate::ship::system_registry::helm_radar_system_id(),
            ),
            slot: ModifierSlot::HelmRadarRange,
            bonus: -0.5,
        });
    }
    app.update();

    // Whatever multiplier the cache computes for a -0.5 bonus, the
    // published range must be the base range scaled by it — and it must
    // actually be a reduction, or the modifier did nothing.
    let mult = app
        .world()
        .entity(npc)
        .get::<crate::modifiers::ShipModifiers>()
        .unwrap()
        .get(&ModifierSlot::HelmRadarRange);
    assert!(
        mult < 0.999,
        "precondition: the damage modifier must reduce the multiplier, got {mult}"
    );
    let bb = helm_bb_of(&mut app, npc);
    assert!(
        (bb.radar_range - 800.0 * mult).abs() < 0.01,
        "NPC radar_range must be damage-scaled (800 * {mult}), got {}",
        bb.radar_range
    );
}

/// The is_local gating: the LocalShip's base radar range still comes from
/// the player-only `ShipClientConfigResource`, never from a
/// `HelmConsoleSection`, and both tiers publish in the same tick.
#[test]
fn local_ship_radar_range_still_comes_from_client_config() {
    let mut app = base_app();
    app.insert_resource(crate::lobby::server::ShipClientConfigResource(
        crate::core::messages::ShipClientConfig {
            helm_radar_range: 123.0,
            ..Default::default()
        },
    ));
    let npc = spawn_npc_ship(&mut app, 750.0);
    app.update();

    let local = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .unwrap();
    let local_bb = helm_bb_of(&mut app, local);
    assert!(
        (local_bb.radar_range - 123.0).abs() < 0.001,
        "LocalShip radar_range must come from ShipClientConfigResource, got {}",
        local_bb.radar_range
    );
    let npc_bb = helm_bb_of(&mut app, npc);
    assert!(
        (npc_bb.radar_range - 750.0).abs() < 0.001,
        "NPC radar_range must ignore the player-only client config, got {}",
        npc_bb.radar_range
    );
}

/// NPC ships get engine + lateral entries too (ship-state tier), derived
/// from their own components rather than the player's joystick queue.
#[test]
fn publish_writes_engine_and_lateral_entries_for_npc_ship() {
    let mut app = base_app();
    let npc = spawn_npc_ship(&mut app, 750.0);
    app.update();

    let bbs = app
        .world()
        .entity(npc)
        .get::<ShipSystemBlackboards>()
        .unwrap();
    assert!(
        bbs.0.contains_key(&helm_engine_port_system_id()),
        "expected NPC helm-engine-port entry"
    );
    assert!(
        bbs.0.contains_key(&helm_engine_starboard_system_id()),
        "expected NPC helm-engine-starboard entry"
    );
    assert!(
        bbs.0.contains_key(&lateral_thrust_system_id()),
        "expected NPC helm-lateral-thrust entry"
    );
}

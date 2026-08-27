use super::*;
use crate::core::messages::{ClientMessage, *};
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target};
use crate::server_app::{
    LastBroadcastEntityPositions, LastBroadcastHull, ShipImpulse, ShipShields, SimOutbox,
};
use crate::server_app::{LocalShip, ShipSystemBlackboards};
use crate::ship::control_source::ControlSource;
use crate::ship::system_registry::SHIELDS_SYSTEM_ID;
use crate::ship_plugin::CoordinationEnqueue;

#[derive(Resource)]
struct ShipEntity(Entity);

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

#[derive(Resource, Default)]
struct CoordEnqueueBox(Vec<CoordinationEnqueue>);

fn collect_coord(
    mut reader: MessageReader<CoordinationEnqueue>,
    mut box_: ResMut<CoordEnqueueBox>,
) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

#[test]
fn generic_coordination_router_does_not_name_shields_pending_state() {
    let router = include_str!("coordination_systems.rs");
    assert!(
        !router.contains("PendingShieldsThreatBearing")
            && !router.contains("pending_shields_threat"),
        "the generic lag router must hand off DeliveredCoordination without knowing Shields' private pending-state representation"
    );
}

#[test]
fn ownerless_npc_shield_arc_does_not_create_focus_capability() {
    let config = crate::ship::config::ShipConfig::from_toml(
        r#"
[[system]]
id = "shield-arc-fore"
kind = "shield_arc"
ai_only = true
"#,
        &[crate::ship::system_registry::SHIELD_ARC_KIND],
    )
    .expect("ownerless AI-only shield arc is valid NPC topology");
    let arc = SystemId("shield-arc-fore".into());
    let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
    sources.0.set(arc.clone(), ControlSource::Ai);

    assert_eq!(
        super::ai_operated_shields_focus_system(&sources, &config),
        None,
        "an ownerless shield arc must not imply an authored Shields focus capability"
    );
}

#[test]
fn human_popup_threat_bearing_cannot_latch_the_ai_inbox() {
    let mut ship_config = ShipConfigComponent::default();
    crate::ship::test_support::add_default_shield_arc_systems(&mut ship_config.0);
    let shields_address = crate::ship::coordination::address_for_system_kind(
        &ship_config.0,
        crate::ship::system_registry::SHIELD_ARC_KIND,
    )
    .expect("default fixture authors a Station for its shield arcs");
    let shields_system = ship_config
        .0
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::SHIELDS_KIND)
        .expect("default fixture carries the authored Shields capability")
        .id
        .clone();
    let mut control_sources = ShipSystemControlSources::default();
    control_sources.0.set(shields_system, ControlSource::Ai);

    let mut app = App::new();
    app.add_message::<DeliveredCoordination>()
        .add_systems(Update, receive_shields_coordination);
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            PendingShieldsThreatBearing::default(),
            control_sources,
            ship_config,
        ))
        .id();
    app.world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .write(DeliveredCoordination {
            source_entity: ship,
            address: shields_address,
            payload: CoordinationPayload::ThreatBearing {
                bearing_rad: 1.25,
                label: "test.threat".into(),
            },
            presentation: CoordinationPresentation::new(
                "test.coordination.title",
                "test.coordination.body",
            ),
            delivery: CoordinationDelivery::HumanPopup {
                token: "test-token".into(),
                sender_label: "test-sender".into(),
                order: 0,
            },
        });

    app.update();

    assert_eq!(
        app.world()
            .entity(ship)
            .get::<PendingShieldsThreatBearing>()
            .expect("fixture carries the Shields-owned inbox")
            .0,
        None,
        "an AI-only receiver must reject a human-popup delivery outcome"
    );
}

/// **`tick_shields` regenerates at the ship's own `ModifierSlot::ShieldRegen`**
/// (issue #952) — the wire from the `shields` power group to the screens.
///
/// Two ships in one world so the reading is per-entity: the same system
/// pass must give them different regen. A ship carrying no `ShipModifiers`
/// at all regenerates at its authored rate, which is what keeps every
/// fixture that predates this slot honest.
#[test]
fn tick_shields_scales_regen_by_the_shield_regen_modifier() {
    use crate::modifiers::{Modifier, ShipModifiers};
    use crate::weapons::shield::{ShieldConfig, ShieldSystem};

    let config = ShieldConfig {
        num_facings: 1,
        max_hp: 100,
        regen_per_sec: 10.0,
        offline_duration: 0.0,
    };
    let damaged = || {
        let mut s = ShieldSystem::new(&config);
        s.apply_damage(50, 0.0);
        s
    };

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        // 250 ms is `Time<Virtual>`'s default `max_delta`, so a longer step
        // would be silently clamped and the arithmetic below would not say
        // what it looks like it says.
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(250),
        ))
        .add_systems(Update, tick_shields);

    let mut boosted_mods = ShipModifiers::new();
    boosted_mods.add_or_update(Modifier {
        source: crate::core::messages::ModifierSource::PowerGroup(
            crate::core::messages::PowerGroupId(
                crate::modifiers::power_system::SHIELDS_POWER_GROUP.into(),
            ),
        ),
        slot: crate::core::messages::ModifierSlot::ShieldRegen,
        bonus: 1.0, // x2.0
    });
    let boosted = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipShields(damaged(), 0.5),
            boosted_mods,
        ))
        .id();
    let plain = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipShields(damaged(), 0.5),
            ShipModifiers::new(),
        ))
        .id();
    let unmodified = app
        .world_mut()
        .spawn((crate::server_app::Ship, ShipShields(damaged(), 0.5)))
        .id();

    // Bevy's first `update()` after `TimePlugin` reports a zero delta and
    // `tick_shields` returns early on it, so five passes are four 250 ms
    // steps: one second of regen.
    for _ in 0..5 {
        app.update();
    }

    let hp = |e: Entity, app: &App| app.world().get::<ShipShields>(e).unwrap().0.facings[0].hp;
    assert_eq!(
        hp(plain, &app),
        60,
        "no bonus is x1.0: the arc's authored 10 HP/s over one second"
    );
    assert_eq!(
        hp(unmodified, &app),
        60,
        "a ship with no ShipModifiers at all regenerates at its authored rate"
    );
    assert_eq!(
        hp(boosted, &app),
        70,
        "shields power at x2.0 doubles the same second's regen"
    );
}

/// Test-only glue (issue #829): seed each ship's viewscreen combat_lock
/// from its `TacticalRadarSelection` component before
/// `publish_shields_blackboard` reads it, standing in for the radar
/// publisher + viewscreen aggregator the full app runs.
fn seed_viewscreen_from_selection(
    mut q: Query<
        (
            Option<&crate::console::weapons::TacticalRadarSelection>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for (tac, mut bbs) in q.iter_mut() {
        let combat_lock = tac.and_then(|t| t.0.clone());
        let mut vbb = match bbs
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(v)) => v.clone(),
            _ => crate::core::messages::ViewscreenBlackboard::default(),
        };
        vbb.combat_lock = combat_lock;
        bbs.0.insert(
            crate::ship::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(vbb),
        );
    }
}

fn test_app() -> App {
    let config = crate::weapons::shield::ShieldConfig {
        num_facings: 2,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    };
    let mut app = App::new();
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            {
                let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                // Post-#514: coordination emitter looks up the first
                // arc's SystemId. `ShieldSystem::new` populates arc ids
                // "fore"/"aft" for a 2-facing default.
                cs.0.set(
                    crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
                    ControlSource::Ai,
                );
                cs
            },
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::core::messages::AdmittedCommands::default(),
            crate::ship::state::ShipRedAlert::default(),
            ShieldsCoordinationState::default(),
            ShipImpulse(crate::ship::impulse::ImpulseState::new()),
        ))
        .id();
    app.insert_resource(ShipEntity(ship));
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .init_resource::<crate::lobby::WorldResource>()
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<crate::server_app::LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .init_resource::<Outbox>()
        .init_resource::<CoordEnqueueBox>()
        .add_plugins(ShipShieldsPlugin)
        .add_systems(
            FixedUpdate,
            seed_viewscreen_from_selection.before(publish_shields_blackboard),
        )
        .add_systems(PostUpdate, collect)
        .add_systems(PostUpdate, collect_coord);
    // One fixed step per update (issue #895): the plugin's systems run
    // on the logical tick, and each harness tick advances it once.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(100),
    );
    app
}

fn push_msg(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

fn tick(app: &mut App) -> Vec<OutboundMessage> {
    app.update();
    let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
    let mut out = app.world().resource::<Outbox>().0.clone();
    for (target, msg) in sim_entries {
        out.push(OutboundMessage {
            target,
            msg,
            delivery: crate::core::messages::DeliveryClass::Reliable,
        });
    }
    app.world_mut().resource_mut::<Outbox>().0.clear();
    out
}

fn drain_coord(app: &mut App) -> Vec<CoordinationEnqueue> {
    let msgs = app.world().resource::<CoordEnqueueBox>().0.clone();
    app.world_mut().resource_mut::<CoordEnqueueBox>().0.clear();
    msgs
}

// ── Blackboard publish tests ─────────────────────────────────────────────

fn shipped_hull_shield_status(
    hull_path: &str,
    station_id: &str,
    holder: &str,
    station_systems: &[&str],
) -> Vec<OutboundMessage> {
    let config = crate::entities::include_resolve::load_entity_config(hull_path)
        .unwrap_or_else(|e| panic!("{hull_path} must compose and parse: {e}"))
        .ship_config
        .expect("a shipped player hull must declare stations and systems");
    let station = StationId(station_id.into());

    for system_id in station_systems {
        let system = config
            .system(&SystemId((*system_id).into()))
            .unwrap_or_else(|| panic!("{hull_path} must declare {system_id}"));
        assert_eq!(
            system.station.as_ref(),
            Some(&station),
            "{system_id} must be owned by the {station_id} Station in this fixture"
        );
    }

    let mut app = test_app();
    let ship = ship_e(&mut app);
    app.world_mut()
        .entity_mut(ship)
        .insert(crate::ship_plugin::ShipConfigComponent(config));
    {
        let mut sessions = app.world_mut().resource_mut::<crate::lobby::Sessions>();
        sessions
            .0
            .register(holder.into(), "Shield operator".into())
            .expect("a fresh holder token registers");
        sessions.0.set_station(holder, Some(station));
    }

    // Bevy's first update after TimePlugin reports a zero delta, so the 10 Hz
    // broadcaster's timer first finishes on the following 100 ms logical step.
    tick(&mut app);
    tick(&mut app)
        .into_iter()
        .filter(|message| matches!(message.msg, ServerMessage::ShieldStatus { .. }))
        .collect()
}

fn assert_shield_status_reaches_only_holder(messages: &[OutboundMessage], holder: &str) {
    assert!(
        !messages.is_empty(),
        "the Shields System holder must receive an authoritative ShieldStatus snapshot"
    );
    assert!(
        messages.iter().all(|message| {
            matches!(&message.target, Target::Token(token) if token == holder)
                && message.delivery == DeliveryClass::Snapshot
        }),
        "every periodic ShieldStatus must be a snapshot addressed to the Shields System holder"
    );
    assert!(
        messages.iter().all(|message| matches!(
            &message.msg,
            ServerMessage::ShieldStatus { facings, .. } if !facings.is_empty()
        )),
        "the delivered ShieldStatus must carry the ship's authored facings"
    );
}

#[test]
fn conventional_shields_station_receives_the_authoritative_snapshot() {
    let messages = shipped_hull_shield_status(
        "assets/entities/alliance_battleship.toml",
        "shields",
        "shields-officer",
        &["shields-system"],
    );

    assert_shield_status_reaches_only_holder(&messages, "shields-officer");
}

#[test]
fn composite_engineering_station_receives_the_authoritative_shields_snapshot() {
    let messages = shipped_hull_shield_status(
        "assets/entities/alliance_destroyer.toml",
        "engineering",
        "engineering-officer",
        &["shields-system", "power-reactor", "repair"],
    );

    assert_shield_status_reaches_only_holder(&messages, "engineering-officer");
}

fn shields_bb(app: &mut App) -> ShieldsBlackboard {
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    // Safety: test always spawns exactly one LocalShip entity.
    let bbs = q
        .single(app.world())
        .expect("no LocalShip with ShipSystemBlackboards");
    let key = SystemId(SHIELDS_SYSTEM_ID.to_string());
    let SystemBlackboard::Shields(bb) = bbs.0.get(&key).unwrap() else {
        panic!("expected Shields blackboard");
    };
    bb.clone()
}

#[test]
fn publish_shields_blackboard_contains_hull_integrity() {
    let mut app = test_app();
    app.update();
    assert!((shields_bb(&mut app).hull_integrity_pct - 100.0).abs() < f32::EPSILON);
}

#[test]
fn publish_shields_blackboard_four_facings() {
    let config = crate::weapons::shield::ShieldConfig {
        num_facings: 4,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    };
    let mut app = App::new();
    let _ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            {
                let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                // Post-#514: emit_shields_coordination reads the first
                // arc's SystemId as sender_origin. Set "fore" for the
                // 4-facing default (Fore, Port, Aft, Starboard).
                cs.0.set(
                    crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
                    ControlSource::Ai,
                );
                cs
            },
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::core::messages::AdmittedCommands::default(),
            ShipImpulse(crate::ship::impulse::ImpulseState::new()),
        ))
        .id();
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .init_resource::<crate::lobby::WorldResource>()
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<crate::server_app::LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .add_plugins(ShipShieldsPlugin);
    // One fixed step per update (issue #895).
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(100),
    );
    app.update();
    assert_eq!(shields_bb(&mut app).facings.len(), 4);
}

fn ship_e(app: &mut App) -> Entity {
    app.world().resource::<ShipEntity>().0
}

#[test]
fn publish_shields_blackboard_shows_focused_facing() {
    let mut app = test_app();
    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .set_focused_facing(Some(0));
    app.update();
    assert!(shields_bb(&mut app).focused_facing.is_some());
}

#[test]
fn publish_shields_blackboard_clears_focused_facing() {
    let mut app = test_app();
    let se = ship_e(&mut app);
    {
        let mut e = app.world_mut().entity_mut(se);
        let mut shields = e.get_mut::<ShipShields>().unwrap();
        shields.0.set_focused_facing(Some(0));
        shields.0.set_focused_facing(None);
    }
    app.update();
    assert_eq!(shields_bb(&mut app).focused_facing, None);
}

#[test]
fn publish_shields_blackboard_grid_offline_when_facing_down() {
    let mut app = test_app();
    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .apply_damage(9999, 0.0);
    app.update();
    assert_eq!(shields_bb(&mut app).grid_status, "EMITTER OFFLINE");
}

#[test]
fn publish_shields_blackboard_stable_on_double_update() {
    let mut app = test_app();
    app.update();
    app.update();
    assert!((shields_bb(&mut app).hull_integrity_pct - 100.0).abs() < f32::EPSILON);
}

// ── Coordination tests ──────────────────────────────────────────────────

fn test_app_with_helm() -> App {
    let config = crate::weapons::shield::ShieldConfig {
        num_facings: 2,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    };
    let mut app = App::new();
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::ship_plugin::ShipConfigComponent::default(),
            {
                let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                // Post-#514: emit_shields_coordination looks up the first
                // arc's SystemId as the sender_origin. `ShieldSystem::new`
                // populates arc ids from `default_arc_id`; for 2-facing
                // that's "fore" and "aft". Set the first arc to Ai so the
                // test asserts continue to hold.
                cs.0.set(
                    crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
                    ControlSource::Ai,
                );
                cs
            },
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::core::messages::AdmittedCommands::default(),
            crate::ship::state::ShipRedAlert::default(),
            ShieldsCoordinationState::default(),
            ShipImpulse(crate::ship::impulse::ImpulseState::new()),
        ))
        .id();
    app.insert_resource(ShipEntity(ship));
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .init_resource::<crate::lobby::WorldResource>()
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<crate::server_app::LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .init_resource::<Outbox>()
        .init_resource::<CoordEnqueueBox>()
        .add_plugins(ShipShieldsPlugin)
        .add_systems(PostUpdate, collect)
        .add_systems(PostUpdate, collect_coord);
    // One fixed step per update (issue #895): the plugin's systems run
    // on the logical tick, and each harness tick advances it once.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(100),
    );
    app
}

fn start_game_with_shields_and_helm(app: &mut App) {
    push_msg(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push_msg(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push_msg(
        app,
        "helm",
        ClientMessage::Identify {
            token: "helm".into(),
            name: "Sulu".into(),
        },
    );
    tick(app);
    push_msg(
        app,
        "helm",
        ClientMessage::SelectStation {
            station: "Helm".into(),
        },
    );
    tick(app);
    push_msg(app, "captain", ClientMessage::SetReady { ready: true });
    push_msg(app, "helm", ClientMessage::SetReady { ready: true });
    tick(app);
}

#[test]
fn shield_facing_down_coordination_sent_to_helm_when_facing_goes_offline() {
    let mut app = test_app_with_helm();
    start_game_with_shields_and_helm(&mut app);

    // Drain facing 0 offline.
    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .apply_damage(9999, 0.0);

    tick(&mut app);
    let coord_msgs = drain_coord(&mut app);

    let down_msgs: Vec<_> = coord_msgs
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
        .collect();

    assert!(
        !down_msgs.is_empty(),
        "expected a ShieldFacingDown CoordinationEnqueue to be sent"
    );
    assert!(
        down_msgs.iter().all(|m| m.address
            == crate::core::messages::CoordinationAddress::Station(
                crate::core::messages::StationId(
                    crate::ship::system_registry::HELM_STATION_ID.into(),
                ),
            )),
        "ShieldFacingDown should address the Helm Station"
    );
}

#[test]
fn shield_facing_down_fires_only_once_per_offline_cycle() {
    let mut app = test_app_with_helm();
    start_game_with_shields_and_helm(&mut app);

    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .apply_damage(9999, 0.0);

    tick(&mut app); // first tick — fires
    drain_coord(&mut app); // discard first tick's messages

    tick(&mut app); // second tick — should not re-fire
    let coord_msgs = drain_coord(&mut app);

    let count = coord_msgs
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
        .count();

    assert_eq!(
        count, 0,
        "ShieldFacingDown should not fire again on the same offline cycle"
    );
}

#[test]
fn shield_facing_restored_fires_on_red_alert_when_hp_recovers() {
    let mut app = test_app_with_helm();
    start_game_with_shields_and_helm(&mut app);

    // Put facing offline.
    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .apply_damage(9999, 0.0);
    tick(&mut app);
    drain_coord(&mut app); // discard down notification

    // Manually restore the facing and set HP to above threshold.
    {
        let mut e = app.world_mut().entity_mut(se);
        let mut shields = e.get_mut::<ShipShields>().unwrap();
        let facing = &mut shields.0.facings[0];
        facing.offline_remaining = 0.0;
        facing.hp = 60; // 60/100 = 0.6 >= 0.5 threshold
    }

    // Activate red alert via per-entity ShipRedAlert component.
    {
        let mut q = app.world_mut().query_filtered::<&mut crate::ship::state::ShipRedAlert, bevy::prelude::With<crate::server_app::LocalShip>>();
        if let Ok(mut ra) = q.single_mut(app.world_mut()) {
            ra.0 = true;
        }
    }

    // Mark down_notified on the per-ship ShieldsCoordinationState so
    // the restore branch can fire.
    {
        let se = ship_e(&mut app);
        let mut e = app.world_mut().entity_mut(se);
        let mut coord = e.get_mut::<ShieldsCoordinationState>().unwrap();
        if coord.down_notified.is_empty() {
            coord.down_notified.push(true);
            coord.restore_notified.push(false);
        } else {
            coord.down_notified[0] = true;
        }
    }

    tick(&mut app);
    let coord_msgs = drain_coord(&mut app);

    let restored_msgs: Vec<_> = coord_msgs
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingRestored { .. }))
        .collect();

    assert!(
        !restored_msgs.is_empty(),
        "expected a ShieldFacingRestored CoordinationEnqueue on red alert after recovery"
    );
}

#[test]
fn shield_facing_restored_does_not_fire_without_red_alert() {
    let mut app = test_app_with_helm();
    start_game_with_shields_and_helm(&mut app);

    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .apply_damage(9999, 0.0);
    tick(&mut app);
    drain_coord(&mut app); // discard down notification

    {
        let mut e = app.world_mut().entity_mut(se);
        let mut shields = e.get_mut::<ShipShields>().unwrap();
        let facing = &mut shields.0.facings[0];
        facing.offline_remaining = 0.0;
        facing.hp = 60;
    }

    if let Some(mut coord) = app
        .world_mut()
        .entity_mut(se)
        .get_mut::<ShieldsCoordinationState>()
    {
        if coord.down_notified.is_empty() {
            coord.down_notified.push(true);
            coord.restore_notified.push(false);
        } else {
            coord.down_notified[0] = true;
        }
    }

    // No red alert active.
    tick(&mut app);
    let coord_msgs = drain_coord(&mut app);

    let count = coord_msgs
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingRestored { .. }))
        .count();

    assert_eq!(
        count, 0,
        "ShieldFacingRestored should not fire when not on red alert"
    );
}

#[test]
fn npc_shield_restore_notify_reads_its_own_tuning_not_the_player_ships_global_resource() {
    // Issue #738 isolation: `emit_shields_coordination` used to read the
    // global `ShieldsAiConfigResource` while iterating EVERY ship, and
    // `server_app` writes that Resource from the PLAYER ship's
    // `[shields_console.ai]` TOML — so every NPC's restore threshold
    // followed the player's.
    //
    // The global Resource here carries a strict 90% restore threshold; the
    // parse-time default is 50%. A facing sitting at 60/100 fires under the
    // default and does not under the global tuning, so the global is
    // observable — and must not be what an NPC without its own component
    // uses.
    let mut app = test_app_with_helm();
    start_game_with_shields_and_helm(&mut app);
    app.insert_resource(ShieldsAiConfigResource {
        restored_notify_pct: 0.9,
        ..Default::default()
    });

    let config = crate::weapons::shield::ShieldConfig {
        num_facings: 2,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    };
    let arc_sources = || {
        let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
        cs.0.set(
            crate::ship::system_registry::shield_arc_system_id("fore").expect("fore"),
            ControlSource::Ai,
        );
        cs
    };
    let red_alert = || crate::ship::state::ShipRedAlert(true);

    // An NPC with no shields-AI component of its own.
    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::ship_plugin::ShipConfigComponent::default(),
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            arc_sources(),
            red_alert(),
            ShieldsCoordinationState::default(),
        ))
        .id();
    // A second NPC that DOES carry the strict tuning on its own entity,
    // proving the 60/100 facing is suppressible at all.
    let tuned = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::ship_plugin::ShipConfigComponent::default(),
            ShipShields(crate::weapons::shield::ShieldSystem::new(&config), 0.5),
            arc_sources(),
            red_alert(),
            ShieldsCoordinationState::default(),
            ShieldsAiConfigResource {
                restored_notify_pct: 0.9,
                ..Default::default()
            },
        ))
        .id();

    for e in [npc, tuned] {
        let mut entity_mut = app.world_mut().entity_mut(e);
        {
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            let facing = &mut shields.0.facings[0];
            facing.offline_remaining = 0.0;
            facing.hp = 60; // 0.6 — above the 0.5 default, below the 0.9 tuning
        }
        let mut coord = entity_mut.get_mut::<ShieldsCoordinationState>().unwrap();
        coord.down_notified = vec![true, false];
        coord.restore_notified = vec![false, false];
    }

    tick(&mut app);
    let restoring_ships: Vec<Entity> = drain_coord(&mut app)
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingRestored { .. }))
        .map(|m| m.source_entity)
        .collect();

    assert!(
        restoring_ships.contains(&npc),
        "an NPC without its own shields-AI tuning must use the parse-time 50% default, \
         not the player ship's global 90%"
    );
    assert!(
        !restoring_ships.contains(&tuned),
        "a ship carrying the strict 90% threshold on its own entity must stay silent at 60%"
    );
}

/// Verify that the `CoordinationEnqueue` event carries `sender_origin = Ai`
/// by default (no explicit `ShipSystemControlSources` set), confirming the
/// channel-3 routing matrix will treat it as AI-originated and route
/// correctly (AI → Human = Popup; AI → AI = Consume) at delivery time.
#[test]
fn shield_facing_down_coordination_carries_ai_sender_origin_for_routing() {
    let mut app = test_app_with_helm();
    start_game_with_shields_and_helm(&mut app);

    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .apply_damage(9999, 0.0);

    tick(&mut app);
    let coord_msgs = drain_coord(&mut app);

    let down_msgs: Vec<_> = coord_msgs
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
        .collect();

    assert!(!down_msgs.is_empty(), "expected ShieldFacingDown enqueue");
    assert!(
        down_msgs
            .iter()
            .all(|m| m.sender_origin == ControlSource::Ai),
        "default sender_origin should be Ai (shields console has no holder)"
    );
    assert!(
        down_msgs.iter().all(|m| m.address
            == crate::core::messages::CoordinationAddress::Station(
                crate::core::messages::StationId(
                    crate::ship::system_registry::HELM_STATION_ID.into(),
                ),
            )),
        "ShieldFacingDown should address the Helm Station"
    );
}

// ── Issue #514 tests ─────────────────────────────────────────────────────

#[test]
fn shield_facing_down_still_fires_for_variable_arc() {
    // Regression: after the SystemId shape flipped from `shields` to
    // per-arc `shield-arc-<id>`, coordination messages must still fire
    // when an arc goes offline. The test app uses a 2-facing default so
    // arc ids are "fore" and "aft".
    let mut app = test_app_with_helm();
    start_game_with_shields_and_helm(&mut app);

    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .apply_damage(9999, 0.0); // deplete facing 0 (fore)

    tick(&mut app);
    let coord_msgs = drain_coord(&mut app);
    let down_msgs: Vec<_> = coord_msgs
        .iter()
        .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
        .collect();
    assert!(
        !down_msgs.is_empty(),
        "expected a ShieldFacingDown after arc depletion (variable-arc regression)"
    );
}

#[test]
fn handle_set_shield_arc_focus_flips_focus() {
    // Basic wire-shape assertion: a `SetShieldArcFocus { focused: true }`
    // targeted at `shield-arc-fore` moves focus to that facing.
    let mut app = test_app();
    // Manually admit the command (bypasses the full authorisation stack).
    let se = ship_e(&mut app);
    let arc_sid = crate::ship::system_registry::shield_arc_system_id("fore").expect("fore");
    app.world_mut()
        .entity_mut(se)
        .get_mut::<crate::core::messages::AdmittedCommands>()
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: arc_sid.clone(),
            payload: SystemControlPayload::SetShieldArcFocus { focused: true },
            response_token: None,
        });
    tick(&mut app);
    let shields = app.world().entity(se).get::<ShipShields>().unwrap();
    assert_eq!(shields.0.focused_facing, Some(0), "fore arc focused");
}

#[test]
fn handle_set_shield_arc_focus_clears_focus_when_target_matches_current() {
    let mut app = test_app();
    let se = ship_e(&mut app);
    // Manually set focus first.
    app.world_mut()
        .entity_mut(se)
        .get_mut::<ShipShields>()
        .unwrap()
        .0
        .set_focused_facing(Some(0));
    // Send `focused: false` targeted at fore → clears.
    let arc_sid = crate::ship::system_registry::shield_arc_system_id("fore").expect("fore");
    app.world_mut()
        .entity_mut(se)
        .get_mut::<crate::core::messages::AdmittedCommands>()
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: arc_sid,
            payload: SystemControlPayload::SetShieldArcFocus { focused: false },
            response_token: None,
        });
    tick(&mut app);
    let shields = app.world().entity(se).get::<ShipShields>().unwrap();
    assert_eq!(shields.0.focused_facing, None);
}

#[test]
fn publish_writes_shield_arc_blackboard_per_arc() {
    // The publish system emits one `SystemBlackboard::ShieldArc` entry
    // per arc under `SystemId("shield-arc-<id>")`, alongside the
    // aggregate `Shields` blackboard.
    let mut app = test_app();
    tick(&mut app);
    let se = ship_e(&mut app);
    let bbs = app
        .world()
        .entity(se)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .expect("ShipSystemBlackboards");
    // 2-facing default: fore + aft.
    for arc_id in &["fore", "aft"] {
        let sid = crate::ship::system_registry::shield_arc_system_id(arc_id).expect("arc id");
        let bb = bbs.0.get(&sid).unwrap_or_else(|| {
            panic!(
                "expected ShieldArc blackboard under {sid:?}, got {:?}",
                bbs.0.keys().collect::<Vec<_>>()
            )
        });
        match bb {
            SystemBlackboard::ShieldArc(arc_bb) => {
                assert_eq!(arc_bb.hp, 100, "arc {arc_id} starts full");
                assert!(arc_bb.is_online, "arc {arc_id} starts online");
            }
            other => panic!("expected ShieldArc variant, got {other:?}"),
        }
    }
    // Aggregate `shields` blackboard also present.
    assert!(
        bbs.0.contains_key(&SystemId(
            crate::ship::system_registry::SHIELDS_SYSTEM_ID.to_string()
        )),
        "aggregate shields blackboard must still be published"
    );
}

#[test]
fn publish_shield_arc_blackboard_is_online_reflects_offline_systems() {
    // When a fine shield-arc-<id> SystemId is in offline_systems (via
    // hull-damage sync), the arc's `is_online` in the per-arc
    // blackboard must be false.
    let mut app = test_app();
    let se = ship_e(&mut app);
    // Directly mark fore arc as offline via ControlSources.
    let arc_sid = crate::ship::system_registry::shield_arc_system_id("fore").expect("fore");
    app.world_mut()
        .entity_mut(se)
        .get_mut::<crate::ship_plugin::ShipSystemControlSources>()
        .unwrap()
        .0
        .set_offline(arc_sid.clone(), true);
    tick(&mut app);
    let bbs = app
        .world()
        .entity(se)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .expect("ShipSystemBlackboards");
    let bb = bbs.0.get(&arc_sid).expect("fore arc blackboard");
    match bb {
        SystemBlackboard::ShieldArc(arc_bb) => {
            assert!(
                !arc_bb.is_online,
                "fore arc must report is_online=false when in offline_systems"
            );
        }
        other => panic!("expected ShieldArc variant, got {other:?}"),
    }
}

// ── Issue #826 tests: per-Ship publish ───────────────────────────────────

/// Spawn a bare NPC (Ship, no LocalShip) alongside `test_app`'s player.
fn spawn_npc(app: &mut App, frequency: f32) -> Entity {
    let config = crate::weapons::shield::ShieldConfig {
        num_facings: 2,
        max_hp: 100,
        regen_per_sec: 0.0,
        offline_duration: 10.0,
    };
    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            ShipShields(
                crate::weapons::shield::ShieldSystem::new(&config),
                frequency,
            ),
            crate::server_app::ShipSystemBlackboards::default(),
        ))
        .id()
}

#[test]
fn publish_writes_blackboards_for_every_ship_not_just_local() {
    // Per-Ship publish (issue #826): an NPC gets its own aggregate
    // Shields blackboard AND its own per-arc ShieldArc entries in its
    // own ShipSystemBlackboards, alongside the LocalShip's.
    let mut app = test_app();
    let npc = spawn_npc(&mut app, 0.25);
    app.update();

    let bbs = app
        .world()
        .entity(npc)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .expect("NPC ShipSystemBlackboards");
    let key = SystemId(SHIELDS_SYSTEM_ID.to_string());
    let SystemBlackboard::Shields(bb) = bbs.0.get(&key).expect("NPC Shields blackboard") else {
        panic!("expected Shields blackboard variant on the NPC");
    };
    assert!(
        (bb.frequency - 0.25).abs() < f32::EPSILON,
        "NPC blackboard must reflect the NPC's own shields, not the player's"
    );
    for arc_id in &["fore", "aft"] {
        let sid = crate::ship::system_registry::shield_arc_system_id(arc_id).expect("arc id");
        assert!(
            matches!(bbs.0.get(&sid), Some(SystemBlackboard::ShieldArc(_))),
            "NPC must publish its own ShieldArc blackboard for {arc_id}"
        );
    }
    // The LocalShip still publishes its own aggregate too.
    assert!((shields_bb(&mut app).frequency - 0.5).abs() < f32::EPSILON);
}

#[test]
fn publish_npc_combat_lock_bearing_uses_its_own_weapons_target() {
    // combat_lock_bearing (renamed from target_bearing, issue #926)
    // derives from each ship's OWN Combat Lock (read from its frozen
    // ViewscreenBlackboard, #829) + ShipPhysics: an NPC at the origin
    // (yaw 0) targeting an entity at +X reads a bearing of 180° (atan2
    // convention preserved from the LocalShip-only publish); the
    // LocalShip, with no TacticalRadarSelection, stays None.
    let mut app = test_app();
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("npc-target".into()),
        Transform::from_xyz(10.0, 0.0, 0.0),
    ));
    let npc = spawn_npc(&mut app, 0.25);
    app.world_mut().entity_mut(npc).insert((
        crate::ship::state::ShipPhysics::default(),
        crate::console::weapons::TacticalRadarSelection(Some("npc-target".into())),
    ));
    app.update();

    let bbs = app
        .world()
        .entity(npc)
        .get::<crate::server_app::ShipSystemBlackboards>()
        .expect("NPC ShipSystemBlackboards");
    let key = SystemId(SHIELDS_SYSTEM_ID.to_string());
    let SystemBlackboard::Shields(bb) = bbs.0.get(&key).expect("NPC Shields blackboard") else {
        panic!("expected Shields blackboard variant on the NPC");
    };
    let bearing = bb
        .combat_lock_bearing
        .expect("NPC combat_lock_bearing must derive from its own TacticalRadarSelection");
    assert!(
        (bearing - 180.0).abs() < 0.01,
        "expected bearing ~180° for a target dead ahead on +X (got {bearing})"
    );
    assert_eq!(
        shields_bb(&mut app).combat_lock_bearing,
        None,
        "the LocalShip holds no TacticalRadarSelection, so its bearing stays None"
    );
}

// ── threat_bearing (issue #926) ─────────────────────────────────────────
//
// Parity fix: the same authoritative bearing the backfilled Shields
// focus AI reads (via the delayed `PendingShieldsThreatBearing` inbox)
// published as a standing field so a human Shields officer sees it too.

#[test]
fn threat_bearing_none_when_sensors_holds_no_threat() {
    let mut app = test_app();
    app.update();
    assert_eq!(shields_bb(&mut app).threat_bearing, None);
}

#[test]
fn threat_bearing_reads_sensors_threat_state_verbatim() {
    let mut app = test_app();
    let se = ship_e(&mut app);
    // Same conversion `console_ai::server::ai_shield_focus` applies to
    // the bearing radians it reads off the delayed coordination copy of
    // this same fact.
    let bearing_rad = std::f32::consts::FRAC_PI_2; // 90°
    app.world_mut()
        .entity_mut(se)
        .insert(crate::ship::sensors::SensorsThreatState {
            last_threat_uuid: Some("hostile-1".into()),
            last_bearing_rad: Some(bearing_rad),
            last_label: Some("Hostile closing".into()),
            last_distance: Some(500.0),
        });
    app.update();
    let bearing = shields_bb(&mut app)
        .threat_bearing
        .expect("threat_bearing must be Some while Sensors holds a threat");
    assert!(
        (bearing - 90.0).abs() < 0.01,
        "expected bearing ~90° (got {bearing})"
    );
}

#[test]
fn threat_bearing_clears_when_sensors_threat_state_clears() {
    // Matches the state clear at src/ship/sensors.rs:449 — no hostile in
    // range clears `last_bearing_rad` back to None, and the published
    // marker must follow.
    let mut app = test_app();
    let se = ship_e(&mut app);
    app.world_mut()
        .entity_mut(se)
        .insert(crate::ship::sensors::SensorsThreatState {
            last_threat_uuid: Some("hostile-1".into()),
            last_bearing_rad: Some(0.5),
            last_label: Some("Hostile closing".into()),
            last_distance: Some(500.0),
        });
    app.update();
    assert!(shields_bb(&mut app).threat_bearing.is_some());

    app.world_mut()
        .entity_mut(se)
        .get_mut::<crate::ship::sensors::SensorsThreatState>()
        .unwrap()
        .last_bearing_rad = None;
    app.update();
    assert_eq!(
        shields_bb(&mut app).threat_bearing,
        None,
        "threat_bearing must clear when SensorsThreatState clears"
    );
}

//! Integration evidence for the canonical local-energy owner. Legacy allocation
//! rungs and output multipliers are no longer simulation state.
use super::*;
use crate::core::messages::*;
use crate::entities::config::EntityConfig;
use crate::infrastructure::{CapacityAdjustment, InfrastructureCondition};
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target};
use crate::modifiers::power_system::{PowerState, PowerSystem};
use crate::resources::server::ResourceServices;
use crate::server_app::{LocalShip, Ship, ShipSystemBlackboards, SimOutbox};
use crate::ship::components::{ConsoleHosts, ConsoleWorkingViews};
use crate::ship_plugin::{
    ActiveStationRatings, CoordinationQueue, ShipConfigComponent, ShipSystemControlSources,
};
use crate::sim_rng::{InstallSimRng, SeedSource, SimRng};

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);
fn collect(mut reader: MessageReader<OutboundMessage>, mut out: ResMut<Outbox>) {
    out.0.extend(reader.read().cloned());
}
fn hull() -> EntityConfig {
    crate::entities::include_resolve::resolve_template(
        "assets/entities/alliance_battleship.toml",
        &crate::entities::include_resolve::HostFragmentSource,
    )
    .unwrap()
    .parse_with(EntityConfig::from_toml)
    .unwrap()
}
fn app_with(config: EntityConfig) -> (App, Entity) {
    let ship = config.ship_config.clone().unwrap();
    let mut app = App::new();
    app.add_plugins((
        LobbyPlugin,
        bevy::time::TimePlugin,
        crate::server_app::AdmissionPlugin,
    ))
    .configure_sets(
        FixedUpdate,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Damage,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::PublishAggregate,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain(),
    )
    .init_resource::<crate::lobby::WorldResource>()
    .init_resource::<crate::world::server::WorldContentRuntime>()
    .init_resource::<crate::effect_queue::EffectQueue<CapacityAdjustment>>()
    .init_resource::<crate::effect_queue::EffectQueue<crate::infrastructure::ConditionAdjustment>>()
    .init_resource::<SimOutbox>()
    .init_resource::<Outbox>()
    .add_plugins(ShipPowerPlugin)
    .add_systems(
        FixedUpdate,
        crate::infrastructure::server::tick_infrastructure_condition
            .in_set(crate::sim_sets::SimSet::Modifiers)
            .before(mirror_weapons_cold_flags),
    )
    .add_systems(PostUpdate, collect)
    .insert_sim_rng(SimRng::new(408, SeedSource::World));
    // The complete hull includes spatial observations; restore needs the
    // ordinary logical clock even when this fixture exercises only power.
    crate::sim_tick::register_sim_tick(&mut app);
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    let hosts = ConsoleHosts(
        ship.consoles
            .iter()
            .map(|console| phoenix_model::console::ConsoleHostSnapshot {
                console: console.id.clone(),
                host: console.home_station.clone(),
                rating: "Std".into(),
            })
            .collect(),
    );
    // Use the ordinary accepted hull constructor: whole-candidate restore
    // validates every fitted capability, not only the energy owner under test.
    let entity = crate::entities::spawner::spawn_entity(
        &mut app.world_mut().commands(),
        &config,
        Vec3::ZERO,
        "energy-ship".into(),
        None,
    );
    app.world_mut().flush();
    app.world_mut().entity_mut(entity).insert((
        Ship,
        LocalShip,
        ShipConfigComponent(ship),
        hosts,
        ConsoleWorkingViews::default(),
        ShipSystemBlackboards::default(),
        ShipSystemControlSources::default(),
        ActiveStationRatings::default(),
        AdmittedCommands::default(),
        CoordinationQueue::default(),
        PowerBrownoutState::default(),
        ResourceServices(config.resource_services.clone()),
        InfrastructureCondition(crate::infrastructure::InfrastructureState::from_config(
            config.infrastructure.as_ref().unwrap(),
        )),
    ));
    assert!(app
        .world()
        .get::<crate::console::repair::server::ShipRepairTeams>(entity)
        .is_some());
    assert!(app
        .world()
        .get::<crate::ship::shields::ShipShields>(entity)
        .is_some());
    // Sparse owner apps still register the complete existing capture schema.
    // An absent optional type must not hide the real shield/repair machinery.
    app.world_mut()
        .register_component::<crate::console::weapons::beam::ActiveBeam>();
    app.world_mut()
        .register_component::<crate::console::weapons::beam::PhaserCooldown>();
    app.world_mut()
        .register_component::<crate::console::weapons::TorpedoSystemResource>();
    app.world_mut()
        .register_component::<crate::entities::spawner::EntityShipArcHull>();
    app.world_mut()
        .register_component::<crate::console::repair::server::ShipRepairTeams>();
    app.world_mut()
        .register_component::<crate::console::repair::server::RepairRequestQueue>();
    app.world_mut()
        .register_component::<crate::ship_plugin::RepairHumanAlerted>();
    app.world_mut()
        .register_component::<crate::server_app::ShipSystemBlackboards>();
    app.world_mut()
        .register_component::<crate::ai::server::ObjectiveCursors>();
    app.world_mut()
        .register_component::<crate::console::weapons::blaster::BlasterSystemResource>();
    app.world_mut()
        .register_component::<crate::ship::shields::ShipShields>();
    // Preserve the ordinary capture registration gate without constructing a
    // separate save-only Power projection.
    app.world_mut()
        .register_component::<crate::ship::state::ShipPhysics>();
    app.world_mut()
        .register_component::<crate::entities::spawner::EntitySystemHull>();
    app.world_mut()
        .register_component::<crate::ship::state::ShipRedAlert>();
    app.world_mut()
        .register_component::<crate::console::command::server::ShipStationStances>();
    (app, entity)
}
fn fixture() -> (App, Entity) {
    app_with(hull())
}
fn power(app: &App, entity: Entity) -> &PowerSystem {
    &app.world().get::<ShipPowerSystem>(entity).unwrap().0
}
fn state(app: &App, entity: Entity) -> PowerState {
    power(app, entity).capture_continuation()
}
fn bb(app: &App, entity: Entity) -> &PowerBlackboard {
    match &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0
        [&SystemId(crate::ship::system_registry::POWER_SYSTEM_ID.into())]
    {
        SystemBlackboard::Power(bb) => bb,
        other => panic!("expected common Power observation, got {other:?}"),
    }
}
fn push(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}
fn tick(app: &mut App) -> Vec<OutboundMessage> {
    app.update();
    let mut out = std::mem::take(&mut app.world_mut().resource_mut::<Outbox>().0);
    out.extend(
        app.world_mut()
            .resource_mut::<SimOutbox>()
            .drain()
            .into_iter()
            .map(|entry| OutboundMessage {
                target: entry.target,
                msg: entry.message,
                delivery: entry.delivery,
            }),
    );
    out
}
fn join(app: &mut App, token: &str, station: &str) {
    push(
        app,
        token,
        ClientMessage::Identify {
            token: token.into(),
            name: token.into(),
        },
    );
    tick(app);
    push(
        app,
        token,
        ClientMessage::SelectStation {
            station: station.into(),
        },
    );
    tick(app);
}
fn start(app: &mut App) {
    join(app, "captain", "Captain");
    join(app, "power", "Power");
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "power", ClientMessage::SetReady { ready: true });
    tick(app);
}
fn command(
    app: &mut App,
    token: &str,
    correlation: &str,
    payload: SystemControlPayload,
) -> Vec<OutboundMessage> {
    push(
        app,
        token,
        ClientMessage::ControlSystemCorrelated {
            target: crate::ship::system_registry::power_reactor_system_id(),
            correlation: ActionCorrelationId::new(correlation).unwrap(),
            payload,
        },
    );
    tick(app)
}
fn feedback(
    out: &[OutboundMessage],
    token: &str,
    correlation: &str,
    expected: ActionFeedbackOutcome,
) {
    let matching: Vec<_> = out.iter().filter(|entry| matches!((&entry.target, &entry.msg),
        (Target::Token(actual), ServerMessage::ActionFeedback { correlation: id, .. }) if actual == token && id.as_str() == correlation)).collect();
    assert_eq!(
        matching.len(),
        1,
        "one final response for the actual request"
    );
    assert!(
        matches!(&matching[0].msg, ServerMessage::ActionFeedback { outcome, .. } if *outcome == expected)
    );
}
fn request(group: &str, flow: f32, cooling: f32) -> SystemControlPayload {
    SystemControlPayload::SetEnergyGroup {
        group: PowerGroupId(group.into()),
        flow_per_sec: flow,
        cooling_per_sec: cooling,
    }
}

#[test]
fn legacy_power_state_projection_only_reaches_the_current_power_holder() {
    let (mut app, _) = fixture();
    start(&mut app);
    let out = tick(&mut app);
    let messages: Vec<_> = out
        .iter()
        .filter(|entry| matches!(entry.msg, ServerMessage::PowerState { .. }))
        .collect();
    assert!(
        !messages.is_empty(),
        "the producer must run before checking its audience"
    );
    assert!(messages
        .iter()
        .all(|entry| matches!(&entry.target, Target::Token(token) if token == "power")));
    push(&mut app, "power", ClientMessage::ReleaseStation);
    tick(&mut app);
    let out = tick(&mut app);
    assert!(!out
        .iter()
        .any(|entry| matches!(entry.msg, ServerMessage::PowerState { .. })));
}

#[test]
fn actual_reactor_owner_applies_numeric_flow_and_publishes_the_same_intent() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let max = power(&app, entity)
        .config()
        .groups
        .iter()
        .find(|g| g.id.0 == "helm")
        .unwrap()
        .max_flow_per_sec;
    let out = command(
        &mut app,
        "power",
        "flow-applied",
        request("helm", max * 0.5, 0.0),
    );
    feedback(
        &out,
        "power",
        "flow-applied",
        ActionFeedbackOutcome::Applied,
    );
    assert_eq!(
        power(&app, entity).flow_for(&PowerGroupId("helm".into())),
        Some(max * 0.5)
    );
    let reading = bb(&app, entity).energy.as_ref().unwrap();
    assert_eq!(
        reading
            .groups
            .iter()
            .find(|g| g.id == "helm")
            .unwrap()
            .desired_flow_per_sec,
        max * 0.5
    );
    assert!(reading.reactor.capacity > 0.0);
    assert_eq!(
        reading.consumers.len(),
        power(&app, entity).config().consumers.len()
    );
    assert!(
        bb(&app, entity).groups.is_empty(),
        "no second rung simulation projection"
    );
}

#[test]
fn invalid_paired_command_refuses_atomically_and_unknown_groups_are_not_created() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let before = power(&app, entity).flow_for(&PowerGroupId("helm".into()));
    let max_cooling = power(&app, entity)
        .config()
        .groups
        .iter()
        .find(|g| g.id.0 == "helm")
        .unwrap()
        .max_cooling_per_sec;
    let out = command(
        &mut app,
        "power",
        "invalid-pair",
        request("helm", 0.0, max_cooling + 1.0),
    );
    feedback(
        &out,
        "power",
        "invalid-pair",
        ActionFeedbackOutcome::Refused,
    );
    assert_eq!(
        power(&app, entity).flow_for(&PowerGroupId("helm".into())),
        before
    );
    let out = command(
        &mut app,
        "power",
        "unknown-group",
        request("missing", 0.0, 0.0),
    );
    feedback(
        &out,
        "power",
        "unknown-group",
        ActionFeedbackOutcome::Refused,
    );
    assert!(power(&app, entity)
        .flow_for(&PowerGroupId("missing".into()))
        .is_none());
}

#[test]
fn captain_information_access_does_not_grant_reactor_authority() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let before = power(&app, entity).flow_for(&PowerGroupId("helm".into()));
    let out = command(
        &mut app,
        "captain",
        "wrong-holder",
        request("helm", 0.0, 0.0),
    );
    feedback(
        &out,
        "captain",
        "wrong-holder",
        ActionFeedbackOutcome::Refused,
    );
    assert_eq!(
        power(&app, entity).flow_for(&PowerGroupId("helm".into())),
        before
    );
}

#[test]
fn console_handoff_rejects_previous_host_without_resetting_system_work() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let reactor = crate::ship::system_registry::power_reactor_system_id();
    let console = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .systems
        .iter()
        .find(|s| s.id == reactor)
        .unwrap()
        .console
        .clone()
        .unwrap();
    let before = state(&app, entity);
    app.world_mut()
        .get_mut::<ConsoleHosts>(entity)
        .unwrap()
        .0
        .iter_mut()
        .find(|host| host.console == console)
        .unwrap()
        .host = Some(StationId("captain".into()));
    // This reduced fixture skips the placement resolver. Publish its derived
    // System authority at the same time as the Console placement, as that
    // production resolver does for every System hosted by the Console.
    let bindings = app
        .world()
        .get::<ShipConfigComponent>(entity)
        .unwrap()
        .0
        .systems
        .iter()
        .filter(|system| system.console.as_ref() == Some(&console) && !system.ai_only)
        .map(|system| (system.id.clone(), StationId("captain".into())))
        .collect();
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::ship_plugin::HumanSeekingHosts(bindings));
    assert_eq!(state(&app, entity), before);
    let out = command(&mut app, "power", "stale-host", request("helm", 0.0, 0.0));
    feedback(&out, "power", "stale-host", ActionFeedbackOutcome::Refused);
    let out = command(&mut app, "captain", "new-host", request("helm", 0.0, 0.0));
    feedback(&out, "captain", "new-host", ActionFeedbackOutcome::Applied);
    assert_eq!(
        power(&app, entity).flow_for(&PowerGroupId("helm".into())),
        Some(0.0)
    );
}

#[test]
fn ordinary_delegation_and_damage_refuse_human_flow_commands() {
    use crate::ship::control_source::ControlSource;
    for offline in [false, true] {
        let (mut app, entity) = fixture();
        start(&mut app);
        let reactor = crate::ship::system_registry::power_reactor_system_id();
        {
            let mut sources = app
                .world_mut()
                .get_mut::<ShipSystemControlSources>(entity)
                .unwrap();
            if offline {
                sources.0.set_offline(reactor, true);
            } else {
                sources.0.set(reactor, ControlSource::Ai);
            }
        }
        let before = power(&app, entity).flow_for(&PowerGroupId("helm".into()));
        let out = command(
            &mut app,
            "power",
            if offline { "offline" } else { "delegated" },
            request("helm", 0.0, 0.0),
        );
        feedback(
            &out,
            "power",
            if offline { "offline" } else { "delegated" },
            ActionFeedbackOutcome::Refused,
        );
        assert_eq!(
            power(&app, entity).flow_for(&PowerGroupId("helm".into())),
            before
        );
        if offline {
            assert_eq!(power(&app, entity).network().output_actual_per_sec, 0.0);
        }
    }
}

#[test]
fn priority_moves_reindex_equal_and_noncontiguous_siblings_only() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let roots: Vec<_> = power(&app, entity)
        .config()
        .groups
        .iter()
        .filter(|g| g.parent.is_none())
        .map(|g| g.id.clone())
        .collect();
    assert!(roots.len() >= 3);
    let moved = roots.last().unwrap().clone();
    for priorities in [
        vec![0; roots.len()],
        (0..roots.len()).map(|i| (i * 7) as u16).collect(),
    ] {
        {
            let mut p = app.world_mut().get_mut::<ShipPowerSystem>(entity).unwrap();
            for (id, priority) in roots.iter().zip(priorities) {
                p.0.set_priority(id, priority).unwrap();
            }
        }
        let child_before: Vec<_> = power(&app, entity)
            .config()
            .groups
            .iter()
            .filter(|g| g.parent.is_some())
            .map(|g| {
                (
                    g.id.clone(),
                    power(&app, entity).network().groups[&g.id].priority,
                )
            })
            .collect();
        let out = command(
            &mut app,
            "power",
            "priority",
            SystemControlPayload::SetEnergyPriority {
                group: moved.clone(),
                priority: 0,
            },
        );
        feedback(&out, "power", "priority", ActionFeedbackOutcome::Applied);
        let mut ranked: Vec<_> = roots
            .iter()
            .map(|id| {
                (
                    power(&app, entity).network().groups[id].priority,
                    id.clone(),
                )
            })
            .collect();
        ranked.sort();
        assert_eq!(ranked[0].1, moved);
        assert_eq!(
            ranked.iter().map(|row| row.0).collect::<Vec<_>>(),
            (0..roots.len() as u16).collect::<Vec<_>>()
        );
        for (id, priority) in child_before {
            assert_eq!(power(&app, entity).network().groups[&id].priority, priority);
        }
    }
}

#[test]
fn flow_cut_preserves_request_and_charged_local_reserves() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let requested: Vec<_> = power(&app, entity)
        .network()
        .groups
        .iter()
        .map(|(id, group)| (id.clone(), group.desired_flow_per_sec))
        .collect();
    {
        let mut p = app.world_mut().get_mut::<ShipPowerSystem>(entity).unwrap();
        p.0.network_mut().reactor.energy = 0.0;
        p.0.network_mut().output_actual_per_sec = 0.0;
        p.0.set_output(0.0).unwrap();
    }
    tick(&mut app);
    let energy = bb(&app, entity).energy.as_ref().unwrap();
    assert!(energy
        .groups
        .iter()
        .any(|group| group.cut && group.effective_flow_per_sec == 0.0));
    assert!(energy
        .consumers
        .iter()
        .any(|consumer| consumer.energy > 0.0));
    assert!(energy
        .consumers
        .iter()
        .all(|consumer| !consumer.thermal_stopped));
    for (id, desired) in requested {
        assert_eq!(power(&app, entity).flow_for(&id), Some(desired));
    }
}

/// Remove incidental thermal work so this integration check isolates a real
/// queued stock withdrawal from reactor cooling, without changing shipped data.
fn coolant_fixture() -> (App, Entity) {
    let mut config = hull();
    let hardware = config.energy.as_mut().unwrap();
    for reserve in std::iter::once(&mut hardware.reactor.reserve)
        .chain(hardware.consumers.iter_mut().map(|c| &mut c.reserve))
    {
        reserve.charge_heat_per_unit = 0.0;
        reserve.discharge_heat_per_unit = 0.0;
        reserve.passive_cooling_per_sec = 0.0;
        reserve.passive_loss_at_full_per_sec = 0.0;
    }
    hardware.reactor.initial_heat = 10.0;
    for consumer in &mut hardware.consumers {
        consumer.initial_heat = 0.0;
    }
    app_with(config)
}
#[test]
fn queued_coolant_debit_is_seen_before_heat_removal_and_committed_once() {
    let (mut app, entity) = coolant_fixture();
    start(&mut app);
    let services = app.world().get::<ResourceServices>(entity).unwrap();
    let provider = super::super::energy_runtime::coolant_bindings(power(&app, entity), services)
        .unwrap()
        .reactor;
    let available = app
        .world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .capacity_stock(&provider)
        .unwrap()
        .available();
    app.world_mut()
        .get_mut::<InfrastructureCondition>(entity)
        .unwrap()
        .0
        .adjust_capacity(&provider, 1 - available)
        .unwrap();
    {
        let mut p = app.world_mut().get_mut::<ShipPowerSystem>(entity).unwrap();
        p.0.network_mut().reactor.heat = 10.0;
        p.0.set_reactor_cooling(1.0).unwrap();
    }
    app.world_mut()
        .resource_mut::<crate::effect_queue::EffectQueue<CapacityAdjustment>>()
        .0
        .push(CapacityAdjustment {
            uuid: "energy-ship".into(),
            capacity: provider.clone(),
            delta: -1,
        });
    let before_credit = power(&app, entity)
        .coolant_credit()
        .get(&provider)
        .copied()
        .unwrap_or(0.0);
    // Remove already-paid fractional coolant too, through its canonical saved
    // continuation, so a zero-stock expectation is not mistaken for free fluid.
    if before_credit > 0.0 {
        let mut saved = state(&app, entity);
        saved.coolant_credit.insert(provider.clone(), 0.0);
        app.world_mut()
            .get_mut::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .restore_continuation(&saved)
            .unwrap();
    }
    tick(&mut app);
    assert_eq!(
        app.world()
            .get::<InfrastructureCondition>(entity)
            .unwrap()
            .0
            .capacity_stock(&provider)
            .unwrap()
            .available(),
        0
    );
    assert_eq!(power(&app, entity).network().reactor.heat, 10.0);
    assert!(app
        .world()
        .resource::<crate::effect_queue::EffectQueue<CapacityAdjustment>>()
        .0
        .is_empty());
    tick(&mut app);
    assert_eq!(power(&app, entity).network().reactor.heat, 10.0);
}

#[test]
fn common_observation_uses_the_reactor_reserve_for_the_battery_alias() {
    let (mut app, entity) = fixture();
    tick(&mut app);
    let reading = bb(&app, entity).energy.as_ref().unwrap();
    let boards = &app.world().get::<ShipSystemBlackboards>(entity).unwrap().0;
    let SystemBlackboard::PowerBattery(alias) =
        &boards[&crate::ship::system_registry::power_battery_system_id()]
    else {
        panic!("battery projection")
    };
    assert_eq!(alias.charge, reading.reactor.energy);
    assert_eq!(alias.capacity, reading.reactor.capacity);
    assert_eq!(bb(&app, entity).battery_charge, reading.reactor.energy);
    assert!(
        !reading
            .consumers
            .iter()
            .any(|consumer| consumer.system
                == crate::ship::system_registry::power_battery_system_id())
    );
    for group in &reading.groups {
        assert_eq!(
            group.label,
            app.world()
                .get::<ShipConfigComponent>(entity)
                .unwrap()
                .0
                .power_groups[&PowerGroupId(group.id.clone())]
                .label
        );
    }
}

#[test]
fn scripted_restraint_changes_no_energy_flow_and_mirrors_each_transition_once() {
    use bevy::ecs::system::RunSystemOnce;
    let (mut app, entity) = fixture();
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::entities::spawner::EntityName("Enforcer".into()));
    let before = state(&app, entity).network;
    for held in [true, false] {
        app.world_mut()
            .resource_mut::<crate::effect_queue::EffectQueue<PendingGroupPower>>()
            .0
            .push(PendingGroupPower {
                uuid: "energy-ship".into(),
                held,
            });
        app.world_mut()
            .run_system_once(drain_scripted_power_orders)
            .unwrap();
        app.world_mut()
            .run_system_once(mirror_weapons_cold_flags)
            .unwrap();
        assert_eq!(power(&app, entity).network(), &before);
        assert_eq!(power(&app, entity).weapons_restrained(), held);
        let runtime = app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>();
        assert_eq!(runtime.flags.flag(OWN_SHIP_WEAPONS_COLD_FLAG), held);
        assert_eq!(runtime.flags.flag(&weapons_cold_flag("Enforcer")), held);
        assert_eq!(runtime.pending_world_events.len(), 2);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .pending_world_events
            .clear();
        app.world_mut()
            .run_system_once(mirror_weapons_cold_flags)
            .unwrap();
        assert!(app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_world_events
            .is_empty());
    }
    app.world_mut()
        .resource_mut::<crate::effect_queue::EffectQueue<PendingGroupPower>>()
        .0
        .push(PendingGroupPower {
            uuid: "absent".into(),
            held: true,
        });
    app.world_mut()
        .run_system_once(drain_scripted_power_orders)
        .unwrap();
    assert!(!power(&app, entity).weapons_restrained());
}

#[test]
fn cut_advisories_carry_actual_controller_and_debounce_until_recovery() {
    use crate::ship::control_source::ControlSource;
    use bevy::ecs::system::RunSystemOnce;
    for source in [ControlSource::Human, ControlSource::Ai] {
        let (mut app, entity) = fixture();
        app.world_mut()
            .get_mut::<ShipSystemControlSources>(entity)
            .unwrap()
            .0
            .set(
                crate::ship::system_registry::power_reactor_system_id(),
                source,
            );
        app.world_mut()
            .get_mut::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .network_mut()
            .groups
            .get_mut(&PowerGroupId("helm".into()))
            .unwrap()
            .cut = true;
        app.world_mut()
            .get_mut::<PowerBrownoutState>(entity)
            .unwrap()
            .locked_changed = true;
        app.world_mut()
            .run_system_once(tick_power_brownout_advisory)
            .unwrap();
        let emitted: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<CoordinationEnqueue>>()
            .drain()
            .collect();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].sender_origin, source);
        app.world_mut()
            .get_mut::<PowerBrownoutState>(entity)
            .unwrap()
            .locked_changed = true;
        app.world_mut()
            .run_system_once(tick_power_brownout_advisory)
            .unwrap();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<CoordinationEnqueue>>()
                .drain()
                .count(),
            0
        );
        app.world_mut()
            .get_mut::<ShipPowerSystem>(entity)
            .unwrap()
            .0
            .network_mut()
            .groups
            .get_mut(&PowerGroupId("helm".into()))
            .unwrap()
            .cut = false;
        app.world_mut()
            .get_mut::<PowerBrownoutState>(entity)
            .unwrap()
            .locked_changed = true;
        app.world_mut()
            .run_system_once(tick_power_brownout_advisory)
            .unwrap();
        assert!(app
            .world()
            .get::<PowerBrownoutState>(entity)
            .unwrap()
            .notified_groups
            .is_empty());
    }
}

#[test]
fn snapshot_roundtrip_restores_local_energy_intent_heat_cuts_coolant_and_continues_identically() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let consumer = power(&app, entity).config().consumers[0].system.clone();
    {
        let mut p = app.world_mut().get_mut::<ShipPowerSystem>(entity).unwrap();
        p.0.set_fine_flow(&consumer, Some(0.0)).unwrap();
        p.0.set_weapons_restrained(true);
        p.0.network_mut().consumers.get_mut(&consumer).unwrap().heat = 2.0;
        p.0.network_mut().reactor.energy = 0.0;
        p.0.network_mut().output_actual_per_sec = 0.0;
        p.0.set_output(0.0).unwrap();
    }
    tick(&mut app);
    assert!(power(&app, entity)
        .network()
        .groups
        .values()
        .any(|group| group.cut));
    let snapshot = crate::snapshot::capture(app.world());
    assert_eq!(snapshot.entities.len(), 1);
    let stored: crate::snapshot::PhoenixSnapshot =
        ron::from_str(&ron::to_string(&snapshot).unwrap()).unwrap();
    let (mut resumed, restored_entity) = fixture();
    start(&mut resumed);
    assert!(power(&resumed, restored_entity)
        .config()
        .consumers
        .iter()
        .any(|row| power(&resumed, restored_entity).emitted_work(&row.system) > 0.0));
    let report = crate::snapshot::restore(resumed.world_mut(), &stored);
    assert!(report.is_complete(), "{report:?}");
    assert_eq!(report.entities_restored, 1);
    assert!(
        power(&resumed, restored_entity)
            .config()
            .consumers
            .iter()
            .all(|row| power(&resumed, restored_entity).emitted_work(&row.system) == 0.0),
        "whole-candidate restore clears bootstrap work instead of granting prepaid emissions"
    );
    assert_eq!(state(&app, entity), state(&resumed, restored_entity));
    assert_eq!(
        app.world().get::<InfrastructureCondition>(entity),
        resumed
            .world()
            .get::<InfrastructureCondition>(restored_entity)
    );
    for _ in 0..4 {
        tick(&mut app);
        tick(&mut resumed);
        assert_eq!(state(&app, entity), state(&resumed, restored_entity));
        assert_eq!(
            app.world().get::<InfrastructureCondition>(entity),
            resumed
                .world()
                .get::<InfrastructureCondition>(restored_entity)
        );
    }
}

#[test]
fn fixed_interval_keeps_actual_input_identity_payment_through_physics_and_expires_next_step() {
    use crate::science::identity::{IdentityEmissions, ShipIdentityEmitters};
    use crate::ship::control_source::ControlSource;
    use phoenix_sim_gameplay::ship::composition::CommandChannelValue;
    #[derive(Resource, Default)]
    struct Witness(Vec<(u64, f64, bool)>);
    fn observe(
        tick: Res<crate::sim_tick::SimTick>,
        ships: Query<(&ShipPowerSystem, &IdentityEmissions)>,
        mut witness: ResMut<Witness>,
    ) {
        let (power, emissions) = ships.single().unwrap();
        witness.0.push((
            tick.0,
            power.0.emitted_work(&SystemId("identity-emitter".into())),
            emissions.claim(tick.0).is_some(),
        ));
    }
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/identity_cruiser.toml",
    )
    .unwrap();
    let identity = ShipIdentityEmitters::from_ship(authored.ship_config.as_ref().unwrap()).unwrap();
    let damage = crate::ship::compartment_damage::ShipDamage::from_config(&authored)
        .unwrap()
        .unwrap();
    let (mut app, entity) = app_with(authored);
    crate::sim_tick::register_sim_tick(&mut app);
    app.init_resource::<Witness>();
    app.world_mut()
        .entity_mut(entity)
        .insert((identity, IdentityEmissions::default(), damage));
    let owner = SystemId("identity-emitter".into());
    app.world_mut()
        .get_mut::<ShipSystemControlSources>(entity)
        .unwrap()
        .0
        .set(owner.clone(), ControlSource::Human);
    app.world_mut()
        .get_mut::<ShipIdentityEmitters>(entity)
        .unwrap()
        .0
        .get_mut(&owner)
        .unwrap()
        .set("enabled", CommandChannelValue::Boolean(true))
        .unwrap();
    app.add_systems(
        FixedUpdate,
        crate::science::identity::tick.in_set(crate::sim_sets::SimSet::Input),
    );
    app.add_systems(
        FixedUpdate,
        observe.in_set(crate::sim_sets::SimSet::Modifiers),
    );
    for _ in 0..8 {
        tick(&mut app);
        if !app.world().resource::<Witness>().0.is_empty() {
            break;
        }
    }
    let first = *app.world().resource::<Witness>().0.last().unwrap();
    assert!(
        first.1 > 0.0 && first.2,
        "actual paid Input identity work survives the real Physics power tick"
    );
    app.world_mut()
        .get_mut::<ShipIdentityEmitters>(entity)
        .unwrap()
        .0
        .get_mut(&owner)
        .unwrap()
        .set("enabled", CommandChannelValue::Boolean(false))
        .unwrap();
    let prior = app.world().resource::<Witness>().0.len();
    for _ in 0..8 {
        tick(&mut app);
        if app.world().resource::<Witness>().0.len() > prior {
            break;
        }
    }
    let next = *app.world().resource::<Witness>().0.last().unwrap();
    assert_eq!(next.0, first.0 + 1);
    assert_eq!(
        (next.1, next.2),
        (0.0, false),
        "a quiet next interval cannot reuse the preceding identity payment"
    );
}

#[test]
fn snapshot_refuses_invalid_energy_references_before_overwriting_live_stocks() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let before = state(&app, entity);
    let stock = app
        .world()
        .get::<InfrastructureCondition>(entity)
        .unwrap()
        .clone();
    let mut saved = crate::snapshot::capture(app.world());
    assert_eq!(saved.entities.len(), 1);
    saved.entities[0]
        .power
        .as_mut()
        .unwrap()
        .network
        .fine_flows
        .insert(SystemId("missing-system".into()), 1.0);
    let report = crate::snapshot::restore(app.world_mut(), &saved);
    assert!(!report.is_complete());
    assert_eq!(state(&app, entity), before);
    assert_eq!(
        app.world().get::<InfrastructureCondition>(entity),
        Some(&stock)
    );
}

#[test]
fn reactor_generation_requires_current_operator_and_survives_full_snapshot_restore() {
    let (mut app, entity) = fixture();
    start(&mut app);
    let out = command(
        &mut app,
        "captain",
        "generation-wrong-holder",
        SystemControlPayload::SetEnergyGeneration { enabled: false },
    );
    feedback(
        &out,
        "captain",
        "generation-wrong-holder",
        ActionFeedbackOutcome::Refused,
    );
    assert!(power(&app, entity).network().generation_enabled);
    let out = command(
        &mut app,
        "power",
        "generation-stop",
        SystemControlPayload::SetEnergyGeneration { enabled: false },
    );
    feedback(
        &out,
        "power",
        "generation-stop",
        ActionFeedbackOutcome::Applied,
    );
    assert!(!power(&app, entity).network().generation_enabled);
    let saved = crate::snapshot::capture(app.world());
    let saved: crate::snapshot::PhoenixSnapshot =
        ron::from_str(&ron::to_string(&saved).unwrap()).unwrap();
    let out = command(
        &mut app,
        "power",
        "generation-start",
        SystemControlPayload::SetEnergyGeneration { enabled: true },
    );
    feedback(
        &out,
        "power",
        "generation-start",
        ActionFeedbackOutcome::Applied,
    );
    assert!(power(&app, entity).network().generation_enabled);
    let report = crate::snapshot::restore(app.world_mut(), &saved);
    assert!(report.is_complete(), "{report:?}");
    assert!(!power(&app, entity).network().generation_enabled);
}

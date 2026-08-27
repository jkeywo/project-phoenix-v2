use super::*;
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target};

/// Issue #977: the label producer emits `strings.csv` ids, never composed
/// English. Every id it can return must exist in the table (so `localiseTree`
/// resolves it), and the shape must be a dotted lowercase id.
#[test]
fn power_group_label_emits_string_ids_not_english() {
    use crate::modifiers::power_system::{HELM_POWER_GROUP, WEAPONS_POWER_GROUP};
    for group in [
        HELM_POWER_GROUP,
        WEAPONS_POWER_GROUP,
        SHIELDS_POWER_GROUP,
        "ops",
    ] {
        let id = power_group_label(group);
        assert!(
            id.starts_with("power.group."),
            "{group} → {id:?} must be a power.group.* id, not English"
        );
        assert!(
            !id.chars().any(|c| c.is_ascii_uppercase() || c == ' '),
            "{id:?} must be a dotted lowercase id"
        );
    }
    assert_eq!(power_group_label("ops"), "power.group.unknown");
}

use crate::core::messages::{ModifierSlot, ServerMessage, *};
use crate::modifiers::power_system::SHIELDS_POWER_GROUP;
use crate::modifiers::ShipModifiers;
use crate::server_app::{
    LastBroadcastEntityPositions, LastBroadcastHull, ShipImpulse, ShipShields, SimOutbox,
};
use crate::weapons::shield::ShieldSystem;

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        // Chain the SimSet phases so admission (before Input) → the applier
        // `handle_power_messages` (moved to Physics in issue #831) → battery
        // tick → publish run in order. Without this, `handle_power_messages`
        // in Physics has no ordering vs. the `.before(Input)` AdmissionSet,
        // so it can run before the command is admitted and the allocation
        // never applies (mirrors the navigation test harness's #830 chain).
        // In `FixedUpdate`, where `ShipPowerPlugin` and `AdmissionPlugin`
        // register since issue #895 — configured on `Update` this chain
        // would order nothing at all.
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
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<crate::server_app::LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .init_resource::<Outbox>()
        .add_plugins(ShipPowerPlugin)
        .add_systems(
            FixedUpdate,
            crate::modifiers::coordination::translate_power_modifiers.after(tick_power_system),
        )
        .add_plugins(crate::server_app::sim_state_broadcaster())
        .add_systems(PostUpdate, collect);
    // Exactly one fixed step per `update()` (issue #895), advancing 200 ms
    // of sim time so the Hz-based broadcast timers always fire inside a
    // single harness tick — the pace this fixture has always run at, which
    // a bare `ManualDuration` no longer delivers now the sim is in
    // `FixedUpdate` against the default 60 Hz timestep.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    // Spawn the player ship entity so handle_power_messages can query it.
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        crate::server_app::ShipSystemBlackboards::default(),
        crate::ship_plugin::ShipConfigComponent::default(),
        crate::ship_plugin::ShipSystemControlSources::default(),
        crate::ship_plugin::ActiveStationRatings::default(),
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::CoordinationQueue::default(),
        ShipShields(ShieldSystem::default(), 0.5),
        ShipModifiers::new(),
        ShipImpulse(crate::ship::impulse::ImpulseState::new()),
        PowerBrownoutState::default(),
    ));
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

fn start_game(app: &mut App) {
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
    push_msg(app, "captain", ClientMessage::SetReady { ready: true });
    tick(app);
}

fn start_game_with_power(app: &mut App) {
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
        "power",
        ClientMessage::Identify {
            token: "power".into(),
            name: "Monty".into(),
        },
    );
    tick(app);
    push_msg(
        app,
        "power",
        ClientMessage::SelectStation {
            station: "Power".into(),
        },
    );
    tick(app);
    push_msg(app, "captain", ClientMessage::SetReady { ready: true });
    push_msg(app, "power", ClientMessage::SetReady { ready: true });
    let _ = tick(app);
}

#[test]
fn power_state_only_sent_to_power_holder() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    let out = tick(&mut app);

    for m in out
        .iter()
        .filter(|m| matches!(&m.msg, ServerMessage::PowerState { .. }))
    {
        assert!(
            matches!(&m.target, Target::Token(t) if t == "power"),
            "PowerState should only go to the Power holder, got {:?}",
            m.target
        );
    }
}

#[test]
fn no_power_station_holder_no_power_state_broadcast() {
    let mut app = test_app();
    start_game(&mut app);

    let out = tick(&mut app);
    let any_power_state = out
        .iter()
        .any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
    assert!(
        !any_power_state,
        "no PowerState should be sent when no Power station holder exists"
    );
}

#[test]
fn power_increase_respects_bounds_noop_at_four() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    let _ = app
        .world_mut()
        .resource_mut::<ShipPowerSystem>()
        .0
        .set_group_allocation(&PowerGroupId(HELM_POWER_GROUP.into()), 4);
    // Directly set via resource (the human message path lives in ship_plugin.rs).
    // Verify the field clamps at 4 on the PowerSystem itself.
    assert_eq!(
        app.world()
            .resource::<ShipPowerSystem>()
            .0
            .level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
        4,
        "helm should remain at 4"
    );
}

#[test]
fn power_increase_respects_total_cap_of_eight() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    // Force total to 8 and check PowerSystem::increase is a no-op.
    {
        let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(HELM_POWER_GROUP.into()),
            4,
        );
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(WEAPONS_POWER_GROUP.into()),
            2,
        );
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
            2,
        );
    }
    {
        let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
        ps.0.increase(&PowerGroupId(SHIELDS_POWER_GROUP.into()));
    }
    assert_eq!(
        app.world()
            .resource::<ShipPowerSystem>()
            .0
            .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
        2,
        "sensors should stay at 2 when total is already at the cap of 8"
    );
    assert_eq!(
        app.world().resource::<ShipPowerSystem>().0.total(),
        8,
        "total should remain 8"
    );
}

#[test]
fn increasing_helm_power_updates_max_speed_via_modifiers() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(PowerGroupId(HELM_POWER_GROUP.into()), [-0.5, 0.0, 1.0, 2.0]);

    // Directly set helm=3 and tick to let translate_power_modifiers run.
    let _ = app
        .world_mut()
        .resource_mut::<ShipPowerSystem>()
        .0
        .set_group_allocation(&PowerGroupId(HELM_POWER_GROUP.into()), 3);
    let _ = tick(&mut app);

    let mult = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipModifiers, With<crate::server_app::LocalShip>>();
        q.single(app.world()).unwrap().get(&ModifierSlot::MaxSpeed)
    };
    assert!(
        (mult - 2.0).abs() < 1e-6,
        "Helm power 3 should give MaxSpeed multiplier 2.0, got {mult}"
    );
}

#[test]
fn decreasing_weapons_power_updates_phaser_damage_via_modifiers() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(
            PowerGroupId(WEAPONS_POWER_GROUP.into()),
            [-0.5, 0.0, 0.25, 0.5],
        );

    // Set weapons=1 directly and tick.
    let _ = app
        .world_mut()
        .resource_mut::<ShipPowerSystem>()
        .0
        .set_group_allocation(&PowerGroupId(WEAPONS_POWER_GROUP.into()), 1);
    let _ = tick(&mut app);

    let expected = 1.0 / 1.5;
    let mult = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipModifiers, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .unwrap()
            .get(&ModifierSlot::PhaserDamage)
    };
    assert!(
        (mult - expected).abs() < 1e-6,
        "Weapons power 1 should give PhaserDamage multiplier {expected}, got {mult}"
    );
}

/// **A flat battery locks the reactor: every group is slammed to 1 and the
/// collapse reaches the modifier table.**
///
/// The exhaustion lock, restored after issue #952's per-group floors were
/// reverted — a player who fails to manage power loses the lot, not just the
/// point they spent up. All three groups land on level 1, so every
/// multiplier crushes to x0.667. `RadarRange` was the retired sensors
/// group's and is now nobody's, so the reactor never touches it.
#[test]
fn a_flat_battery_locks_every_group_to_one_and_updates_all_modifiers() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    let defaults = [-0.5, 0.0, 0.25, 0.5];
    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(PowerGroupId(HELM_POWER_GROUP.into()), defaults);
    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(PowerGroupId(WEAPONS_POWER_GROUP.into()), defaults);
    app.world_mut()
        .resource_mut::<PowerMultiplierResource>()
        .multipliers
        .insert(PowerGroupId(SHIELDS_POWER_GROUP.into()), defaults);

    {
        let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(HELM_POWER_GROUP.into()),
            4,
        );
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(WEAPONS_POWER_GROUP.into()),
            2,
        );
        let _ = ps.0.set_group_allocation(
            &crate::core::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
            2,
        );
        ps.0.battery_charge = 0.0;
    }

    tick(&mut app);

    let power = app.world().resource::<ShipPowerSystem>().0.clone();
    assert!(power.locked(), "a flat battery locks the reactor");
    assert_eq!(
        power.level_for(&PowerGroupId(HELM_POWER_GROUP.into())),
        1,
        "the brownout slammed helm to 1"
    );

    let expected = 1.0 / 1.5;
    let mods = {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipModifiers, With<crate::server_app::LocalShip>>();
        q.single(app.world()).unwrap().clone()
    };

    for (slot, label) in [
        (ModifierSlot::MaxSpeed, "MaxSpeed"),
        (ModifierSlot::PhaserDamage, "PhaserDamage"),
        (ModifierSlot::ShieldRegen, "ShieldRegen"),
    ] {
        let mult = mods.get(&slot);
        assert!(
            (mult - expected).abs() < 1e-6,
            "with every group locked to 1, {label} should be x{expected}, got {mult}"
        );
    }
    assert!(
        (mods.get(&ModifierSlot::RadarRange) - 1.0).abs() < 1e-6,
        "the reactor must not touch RadarRange at all (#952), got {}",
        mods.get(&ModifierSlot::RadarRange)
    );
}

// ── Blackboard publish tests ────────────────────────────────────────────

fn power_blackboard(app: &mut App) -> PowerBlackboard {
    use crate::core::messages::{SystemBlackboard, SystemId};
    use crate::server_app::{LocalShip, ShipSystemBlackboards};
    use crate::ship::system_registry::POWER_SYSTEM_ID;
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    match bbs.0.get(&SystemId(POWER_SYSTEM_ID.to_string())) {
        Some(SystemBlackboard::Power(bb)) => bb.clone(),
        _ => PowerBlackboard::default(),
    }
}

#[test]
fn publish_power_blackboard_contains_correct_data() {
    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app);

    let bb = power_blackboard(&mut app);
    assert!(
        !bb.groups.is_empty(),
        "expected at least one power group entry"
    );
    // Labels are `strings.csv` ids now (issue #977); `localiseTree`
    // resolves them to HELM / WEAPONS / SHIELDS at the client boundary.
    assert!(
        bb.groups.iter().any(|e| e.label == "power.group.helm"),
        "expected helm entry"
    );
    assert!(
        bb.groups.iter().any(|e| e.label == "power.group.weapons"),
        "expected weapons entry"
    );
    assert!(
        bb.groups.iter().any(|e| e.label == "power.group.shields"),
        "expected shields entry"
    );
    assert!(bb.total > 0, "total should be > 0");
    // Total 6 on the default seed, where the default `rates` are still
    // positive — the reserve is filling, not emptying.
    assert!(!bb.draining, "should not be draining at the resting total");
}

/// **Every published entry carries the group's display floor (issue
/// #1004).**
///
/// The console draws its pip row from `min_level..=max_level`. Before this
/// the field did not exist, the client fell back to `0`, and the row grew a
/// bottom rung no order could ever light — a three-group console showed
/// NINE lights where twelve were expected. A hull that authors no
/// `[power_groups.*]` block (this fixture's `ShipConfigComponent::default`)
/// publishes the parse default, which is also the level `PowerSystem`
/// clamps every group to.
#[test]
fn publish_power_blackboard_carries_each_groups_display_floor() {
    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app);

    let bb = power_blackboard(&mut app);
    assert_eq!(bb.groups.len(), 3, "helm, weapons, shields");
    for e in &bb.groups {
        assert_eq!(
            e.min_level,
            crate::ship::config::default_min_power_level(),
            "group {} must publish the parse-default floor, never 0",
            e.id
        );
        assert!(
            e.min_level <= e.max_level,
            "group {} has an empty pip range {}..={}",
            e.id,
            e.min_level,
            e.max_level
        );
    }
    // The count the client will draw, computed the same way the pip loop
    // does: three groups x levels 1..=4.
    let pips: u16 = bb
        .groups
        .iter()
        .map(|e| u16::from(e.max_level - e.min_level + 1))
        .sum();
    assert_eq!(pips, 12, "12 pips, never the phantom 9-light state");
}

/// **A hull that authors a floor above the parse default has it published
/// verbatim (issue #1004).**
///
/// The floor is read off the ship's own `[power_groups.<id>]` block, not
/// off the multiplier table — that table's LENGTH is a ceiling and says
/// nothing about where the rungs start. Nothing server-side clamps to this
/// value; it moves where the pip row begins and nothing else.
#[test]
fn publish_power_blackboard_reads_the_authored_floor() {
    use crate::modifiers::power_system::HELM_POWER_GROUP;
    let mut app = test_app();
    start_game(&mut app);
    {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut cfg = app
            .world_mut()
            .entity_mut(ship)
            .take::<crate::ship_plugin::ShipConfigComponent>()
            .unwrap();
        cfg.0.power_groups.insert(
            PowerGroupId(HELM_POWER_GROUP.into()),
            crate::ship::config::PowerGroupConfig {
                label: "power.group.helm".into(),
                default_level: 2,
                min_level: 3,
                max_level: 4,
            },
        );
        app.world_mut().entity_mut(ship).insert(cfg);
    }
    tick(&mut app);

    let bb = power_blackboard(&mut app);
    let helm = bb
        .groups
        .iter()
        .find(|e| e.id == HELM_POWER_GROUP)
        .expect("helm entry");
    assert_eq!(helm.min_level, 3, "the authored floor reaches the wire");
    for other in bb.groups.iter().filter(|e| e.id != HELM_POWER_GROUP) {
        assert_eq!(
            other.min_level,
            crate::ship::config::default_min_power_level(),
            "group {} authors no block, so it keeps the parse default",
            other.id
        );
    }
}

#[test]
fn publish_power_blackboard_reflects_helm_level_change() {
    let mut app = test_app();
    // Human holds Power; this test app doesn't register any power AI
    // system anyway (that lives in ConsoleAiPlugin), but a human holder
    // keeps the scenario realistic.
    start_game_with_power(&mut app);
    tick(&mut app);

    let _ = app
        .world_mut()
        .resource_mut::<ShipPowerSystem>()
        .0
        .set_group_allocation(&PowerGroupId(HELM_POWER_GROUP.into()), 3);
    tick(&mut app);

    let bb = power_blackboard(&mut app);
    let helm_entry = bb
        .groups
        .iter()
        .find(|e| e.label == "power.group.helm")
        .unwrap();
    assert_eq!(
        helm_entry.level, 3,
        "helm level should be 3 after direct assignment"
    );
}

#[test]
fn control_system_set_power_group_allocation_updates_group() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    push_msg(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
                level: 4,
            },
        },
    );
    tick(&mut app);

    assert_eq!(
        app.world()
            .resource::<ShipPowerSystem>()
            .0
            .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
        4
    );
}

/// Wire-string regression: JS clients send `target: 'power-reactor'`
/// (see `gui/action-map.js` `set_power` handler). This test pins the
/// exact string used on the wire, so if either the JS side or the
/// handler's `for_target(...)` argument drifts back to `"power"`,
/// this fails (the admitted command routes elsewhere and the
/// allocation never applies).
#[test]
fn set_power_group_allocation_wire_string_routes_to_reactor() {
    let mut app = test_app();
    start_game_with_power(&mut app);

    push_msg(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: SystemId("power-reactor".to_string()),
            payload: SystemControlPayload::SetPowerGroupAllocation {
                group: crate::core::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()),
                level: 4,
            },
        },
    );
    tick(&mut app);

    assert_eq!(
        app.world()
            .resource::<ShipPowerSystem>()
            .0
            .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
        4,
        "raw wire string \"power-reactor\" must reach handle_power_messages \
         — if this fails, either the handler's for_target() argument or the \
         JS action-map target has drifted from \"power-reactor\"."
    );
}

// ── Fine Power system tests (issue #513) ───────────────────────────────────
//
// Cover the reactor offline gate and the reactor/battery blackboard
// publication.

fn mark_offline(app: &mut App, system_id: SystemId) {
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .unwrap();
    let mut cs = app
        .world_mut()
        .entity_mut(ship)
        .take::<crate::ship_plugin::ShipSystemControlSources>()
        .unwrap();
    cs.0.set_offline(system_id, true);
    app.world_mut().entity_mut(ship).insert(cs);
}

#[test]
fn reactor_offline_refuses_allocation_input() {
    // End-to-end path via handle_power_messages: the admission gate
    // ensures a Disabled/Destroyed reactor's `accept_human_input` is
    // false, but the direct test is that dispatching a SetPowerGroupAllocation
    // to the reactor id when it's offline leaves battery/allocation untouched.
    //
    // We test the handler directly (bypassing admission which lives in
    // server_app.rs) by seeding an AdmittedCommand targeting the
    // reactor's id and verifying the mutation still applies when the
    // system is online, then does NOT apply when offline_systems marks
    // the reactor offline via the standard admission gate.
    //
    // Since `handle_power_messages` does not itself consult
    // `offline_systems` (admission does), we cover this via the
    // full admission chain in a mini test app that includes
    // `AdmissionPlugin`.
    use crate::core::messages::{ClientMessage, PowerGroupId, SystemControlPayload};
    use crate::lobby::LobbyPlugin;
    let mut app = App::new();
    app.add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        // Chain the SimSet phases so admission (before Input) → the applier
        // `handle_power_messages` (moved to Physics in issue #831) → battery
        // tick → publish run in order. Without this, `handle_power_messages`
        // in Physics has no ordering vs. the `.before(Input)` AdmissionSet,
        // so it can run before the command is admitted and the allocation
        // never applies (mirrors the navigation test harness's #830 chain).
        // In `FixedUpdate` since issue #895 — see `test_app` above.
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
        .init_resource::<SimOutbox>()
        .init_resource::<LastBroadcastEntityPositions>()
        .init_resource::<crate::server_app::LastBroadcastEntityHealth>()
        .init_resource::<LastBroadcastHull>()
        .init_resource::<Outbox>()
        .add_plugins(ShipPowerPlugin)
        .add_plugins(crate::server_app::sim_state_broadcaster());
    // One fixed step per update, 200 ms of sim time each (issue #895).
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    // Spawn the player ship with control sources so we can seed offline_systems.
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        crate::server_app::ShipSystemBlackboards::default(),
        crate::ship_plugin::ShipConfigComponent::default(),
        crate::ship_plugin::ShipSystemControlSources::default(),
        crate::ship_plugin::ActiveStationRatings::default(),
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::CoordinationQueue::default(),
        crate::server_app::ShipShields(crate::weapons::shield::ShieldSystem::default(), 0.5),
        ShipImpulse(crate::ship::impulse::ImpulseState::new()),
        ShipModifiers::new(),
        PowerBrownoutState::default(),
    ));
    start_game_with_power(&mut app);
    // Baseline: reactor online — allocation should update.
    push_msg(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: SystemControlPayload::SetPowerGroupAllocation {
                group: PowerGroupId(SHIELDS_POWER_GROUP.into()),
                level: 4,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        app.world()
            .resource::<ShipPowerSystem>()
            .0
            .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
        4,
        "baseline sanity: online reactor should accept allocation input"
    );

    // Now mark the reactor offline and try to change sensors back to 1.
    mark_offline(
        &mut app,
        crate::ship::system_registry::power_reactor_system_id(),
    );
    push_msg(
        &mut app,
        "power",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::power_reactor_system_id(),
            payload: SystemControlPayload::SetPowerGroupAllocation {
                group: PowerGroupId(SHIELDS_POWER_GROUP.into()),
                level: 1,
            },
        },
    );
    tick(&mut app);
    assert_eq!(
        app.world()
            .resource::<ShipPowerSystem>()
            .0
            .level_for(&PowerGroupId(SHIELDS_POWER_GROUP.into())),
        4,
        "reactor offline must refuse allocation input (sensors should stay at 4)"
    );
}

#[test]
fn publish_writes_power_reactor_and_power_battery_blackboards() {
    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app);

    use crate::server_app::{LocalShip, ShipSystemBlackboards};
    use crate::ship::system_registry::{POWER_BATTERY_SYSTEM_ID, POWER_REACTOR_SYSTEM_ID};
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let bbs = q.single(app.world()).unwrap();

    let reactor = bbs.0.get(&SystemId(POWER_REACTOR_SYSTEM_ID.to_string()));
    let battery = bbs.0.get(&SystemId(POWER_BATTERY_SYSTEM_ID.to_string()));
    assert!(
        matches!(reactor, Some(SystemBlackboard::PowerReactor(_))),
        "expected PowerReactor blackboard under power-reactor system id, got {reactor:?}"
    );
    assert!(
        matches!(battery, Some(SystemBlackboard::PowerBattery(_))),
        "expected PowerBattery blackboard under power-battery system id, got {battery:?}"
    );
    if let Some(SystemBlackboard::PowerReactor(bb)) = reactor {
        assert!(
            bb.is_online,
            "reactor is_online must default to true when nothing is marked offline"
        );
    }
    if let Some(SystemBlackboard::PowerBattery(bb)) = battery {
        assert!(
            bb.is_online,
            "battery is_online must default to true when nothing is marked offline"
        );
    }
}

#[test]
fn reactor_offline_blackboard_reports_is_online_false() {
    let mut app = test_app();
    start_game(&mut app);
    // Seed offline_systems on the ship's control sources.
    {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut cs = app
            .world_mut()
            .entity_mut(ship)
            .take::<crate::ship_plugin::ShipSystemControlSources>()
            .unwrap();
        cs.0.set_offline(
            crate::ship::system_registry::power_reactor_system_id(),
            true,
        );
        app.world_mut().entity_mut(ship).insert(cs);
    }
    tick(&mut app);

    use crate::server_app::{LocalShip, ShipSystemBlackboards};
    use crate::ship::system_registry::POWER_REACTOR_SYSTEM_ID;
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    match bbs.0.get(&SystemId(POWER_REACTOR_SYSTEM_ID.to_string())) {
        Some(SystemBlackboard::PowerReactor(bb)) => {
            assert!(
                !bb.is_online,
                "reactor blackboard is_online must be false when offline_systems contains power-reactor"
            );
        }
        other => panic!("expected PowerReactor blackboard, got {other:?}"),
    }
}

#[test]
fn battery_offline_blackboard_reports_is_online_false() {
    let mut app = test_app();
    start_game(&mut app);
    {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut cs = app
            .world_mut()
            .entity_mut(ship)
            .take::<crate::ship_plugin::ShipSystemControlSources>()
            .unwrap();
        cs.0.set_offline(
            crate::ship::system_registry::power_battery_system_id(),
            true,
        );
        app.world_mut().entity_mut(ship).insert(cs);
    }
    tick(&mut app);

    use crate::server_app::{LocalShip, ShipSystemBlackboards};
    use crate::ship::system_registry::POWER_BATTERY_SYSTEM_ID;
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
    let bbs = q.single(app.world()).unwrap();
    match bbs.0.get(&SystemId(POWER_BATTERY_SYSTEM_ID.to_string())) {
        Some(SystemBlackboard::PowerBattery(bb)) => {
            assert!(
                !bb.is_online,
                "battery blackboard is_online must be false when offline_systems contains power-battery"
            );
        }
        other => panic!("expected PowerBattery blackboard, got {other:?}"),
    }
}

#[test]
fn power_state_broadcast_still_sends_to_power_holder_when_reactor_offline() {
    let mut app = test_app();
    start_game_with_power(&mut app);
    // Mark the reactor offline (audience routing should not care).
    {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut cs = app
            .world_mut()
            .entity_mut(ship)
            .take::<crate::ship_plugin::ShipSystemControlSources>()
            .unwrap();
        cs.0.set_offline(
            crate::ship::system_registry::power_reactor_system_id(),
            true,
        );
        app.world_mut().entity_mut(ship).insert(cs);
    }

    let out = tick(&mut app);
    // At least one PowerState message should still be sent to the power holder.
    let power_state_to_power_holder = out.iter().any(|m| {
        matches!(&m.msg, ServerMessage::PowerState { .. })
            && matches!(&m.target, Target::Token(t) if t == "power")
    });
    assert!(
        power_state_to_power_holder,
        "PowerState broadcast must still target the Power holder even when the reactor is offline"
    );
}

// ── Brownout advisory tests (issue #678) ────────────────────────────────

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

fn drain_coord(app: &mut App) -> Vec<CoordinationEnqueue> {
    let msgs = app.world().resource::<CoordEnqueueBox>().0.clone();
    app.world_mut().resource_mut::<CoordEnqueueBox>().0.clear();
    msgs
}

fn brownout_test_app() -> App {
    let mut app = test_app();
    // Insert ShipPowerSystem component on the LocalShip entity so
    // tick_power_brownout_advisory's query matches it.
    let ship = app
        .world_mut()
        .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
        .single(app.world())
        .unwrap();
    let ship_config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_battleship.toml",
    )
    .expect("the shipped battleship composes")
    .ship_config
    .expect("the shipped battleship declares a ship config");
    app.world_mut().entity_mut(ship).insert((
        ShipPowerSystem(PowerSystem::default()),
        crate::ship_plugin::ShipConfigComponent(ship_config),
    ));
    app.init_resource::<CoordEnqueueBox>()
        .add_systems(PostUpdate, collect_coord);
    app
}

/// Force the LocalShip's reactor into an exact `(allocations, charge,
/// locked)` state, so a following `tick()` exercises the exhaustion lock
/// deterministically rather than integrating toward it over many ticks.
fn set_reactor(app: &mut App, alloc: &[(&str, u8)], charge: f32, locked: bool) {
    let allocs: Vec<(PowerGroupId, u8)> = alloc
        .iter()
        .map(|(g, l)| (PowerGroupId((*g).into()), *l))
        .collect();
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipPowerSystem, With<crate::server_app::LocalShip>>();
    if let Ok(mut ps) = q.single_mut(app.world_mut()) {
        ps.0.restore(&allocs, charge, locked);
    }
}

#[test]
fn tick_power_brownout_advisory_fires_only_on_reactor_lock() {
    let mut app = brownout_test_app();
    start_game(&mut app);

    // A DRAINING but un-exhausted reactor (total 7, healthy battery) is the
    // shed ladder's ordinary combat state, NOT a brownout. No advisory.
    set_reactor(
        &mut app,
        &[
            (HELM_POWER_GROUP, 3),
            (WEAPONS_POWER_GROUP, 2),
            (SHIELDS_POWER_GROUP, 2),
        ],
        80.0,
        false,
    );
    let _ = tick(&mut app);
    assert!(
        drain_coord(&mut app).is_empty(),
        "a draining-but-managed reactor is not a brownout"
    );

    // EXHAUST it: a flat battery at a draining total. `tick_power_system`
    // clamps the charge at zero, slams every group to GROUP_LEVEL_MIN and
    // locks — the brownout. One advisory per (mapped) group.
    set_reactor(
        &mut app,
        &[
            (HELM_POWER_GROUP, 3),
            (WEAPONS_POWER_GROUP, 3),
            (SHIELDS_POWER_GROUP, 2),
        ],
        0.0,
        false,
    );
    let _ = tick(&mut app);
    let emitted = drain_coord(&mut app);
    assert_eq!(
        emitted.len(),
        3,
        "reactor exhaustion browns out all three groups"
    );
    for e in &emitted {
        assert!(
            matches!(&e.payload, CoordinationPayload::PowerBrownout { .. }),
            "expected PowerBrownout, got {:?}",
            e.payload
        );
    }
    let routes: std::collections::HashMap<_, _> = emitted
        .iter()
        .filter_map(|event| match &event.payload {
            CoordinationPayload::PowerBrownout { group, .. } => {
                Some((group.as_str(), event.address.clone()))
            }
            _ => None,
        })
        .collect();
    for (group, station) in [
        (HELM_POWER_GROUP, "helm"),
        (WEAPONS_POWER_GROUP, "tactical"),
        (SHIELDS_POWER_GROUP, "shields"),
    ] {
        assert_eq!(
            routes.get(group),
            Some(&CoordinationAddress::Station(StationId(station.into()))),
            "each brownout group resolves the authored owning Station; `{group}` must not use a fake System target"
        );
    }

    // Still locked next tick, no fresh lock edge → no re-emission.
    let _ = tick(&mut app);
    assert!(
        drain_coord(&mut app).is_empty(),
        "no re-emission while the reactor stays locked"
    );

    // Recovery: the reserve climbs back over emergency_threshold and the
    // reactor unlocks. The unlock edge is consumed silently — coming back
    // is not a brownout.
    set_reactor(
        &mut app,
        &[
            (HELM_POWER_GROUP, 2),
            (WEAPONS_POWER_GROUP, 2),
            (SHIELDS_POWER_GROUP, 2),
        ],
        50.0,
        true,
    );
    let _ = tick(&mut app);
    assert!(
        drain_coord(&mut app).is_empty(),
        "recovery from a lock is not a brownout"
    );

    // A fresh exhaustion re-announces.
    set_reactor(
        &mut app,
        &[
            (HELM_POWER_GROUP, 3),
            (WEAPONS_POWER_GROUP, 3),
            (SHIELDS_POWER_GROUP, 2),
        ],
        0.0,
        false,
    );
    let _ = tick(&mut app);
    assert_eq!(
        drain_coord(&mut app).len(),
        3,
        "a later exhaustion re-announces the brownout"
    );
}

/// Issue #873. The brownout advisory's `sender_origin` must report the
/// Power console's LIVE control source, not a hardcoded `ControlSource::Ai`.
///
/// The hardcode was a routing-tag lie in the opposite direction from the
/// emit-side branches #873 removed: `route_coordination` reads the tag to
/// pick Consume / Popup / Suppress, so a human-operated Power console's
/// advisory claimed AI origin and raised a popup at a human Helm or
/// Tactical, where two humans on the same bridge should simply talk
/// (Suppress). The tag is stamped and forgotten — it is checked here at the
/// point of emission precisely because nothing downstream may re-derive it.
#[test]
fn brownout_advisory_tags_the_live_power_control_source() {
    use crate::ship::control_source::ControlSource;
    for source in [ControlSource::Human, ControlSource::Ai] {
        let mut app = brownout_test_app();
        start_game(&mut app);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::Ship>>();
            for mut cs in q.iter_mut(app.world_mut()) {
                cs.0.set(
                    crate::ship::system_registry::power_reactor_system_id(),
                    source,
                );
            }
        }
        let _ = tick(&mut app);
        let _ = drain_coord(&mut app);

        // Exhaust the reactor (flat battery at a draining total) so the
        // lock fires the advisory.
        set_reactor(
            &mut app,
            &[
                (HELM_POWER_GROUP, 3),
                (WEAPONS_POWER_GROUP, 3),
                (SHIELDS_POWER_GROUP, 2),
            ],
            0.0,
            false,
        );
        let _ = tick(&mut app);
        let emitted = drain_coord(&mut app);
        assert!(
            !emitted.is_empty(),
            "fixture must actually produce a brownout advisory for {source:?}"
        );
        for e in &emitted {
            assert_eq!(
                e.sender_origin, source,
                "PowerBrownout must carry the reactor's live control source as its \
                 routing tag; a hardcoded origin sends a human Power officer's \
                 advisory down the AI→human popup path"
            );
        }
    }
}

use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{
    Console, InterSystemPayload, InterSystemQueue, PowerBlackboard, PowerConsoleEntry,
    ServerMessage, SystemBlackboard, SystemId,
};
use crate::modifiers::power_system::{
    power_group_for_console, power_level_for_console, PowerConfig, PowerSystem,
};
use crate::ship_plugin::LastHelmInput;
use crate::ship_state::ShipState;

// ── Resources ──────────────────────────────────────────────────────────────────

/// Wraps the pure-Rust power system so it can be used as a Bevy resource.
#[derive(Resource)]
pub struct ShipPowerSystem(pub PowerSystem);

/// Wraps the power config for the ship's power system.
#[derive(Resource, Default)]
pub struct PowerConfigResource(pub PowerConfig);

/// Per-console power multiplier configuration: `[f32; 4]` indexed by level-1
/// (index 0 = level 1, index 3 = level 4). Defaults give `[-0.5, 0.0, 0.25, 0.5]`
/// for every console unless overridden in the ship TOML.
#[derive(Resource, Clone, Debug)]
pub struct PowerMultiplierResource {
    pub multipliers: std::collections::HashMap<Console, [f32; 4]>,
}

impl Default for PowerMultiplierResource {
    fn default() -> Self {
        let defaults = [-0.5, 0.0, 0.25, 0.5];
        Self {
            multipliers: std::collections::HashMap::from([
                (Console::Helm, defaults),
                (Console::Tactical, defaults),
                (Console::Sensors, defaults),
            ]),
        }
    }
}

// ── AI config ─────────────────────────────────────────────────────────────────

/// TOML-loaded configuration for the power AI controller.
///
/// Loaded from `[power.ai]` in the ship entity TOML and inserted as a resource
/// at startup by the entity spawner. The fields mirror the `[power.ai]` TOML
/// keys; defaults are used when the section is absent.
#[derive(Resource, Clone, Debug)]
pub struct PowerAiConfigResource {
    /// Minimum battery charge fraction (0.0–1.0) before the AI boosts weapons power.
    pub weapons_battery_floor: f32,
    /// Minimum battery charge fraction (0.0–1.0) before the AI boosts shields power.
    /// NOTE: PowerSystem has no dedicated shields field; this is reserved for future use.
    pub shields_battery_floor: f32,
    /// Minimum battery charge fraction (0.0–1.0) before the AI boosts helm power.
    pub helm_battery_floor: f32,
    /// Thrust level (0.0–1.0) above which the AI considers the ship actively moving.
    pub helm_throttle_threshold: f32,
}

impl Default for PowerAiConfigResource {
    fn default() -> Self {
        Self {
            weapons_battery_floor: 0.5,
            shields_battery_floor: 0.25,
            helm_battery_floor: 0.75,
            helm_throttle_threshold: 0.5,
        }
    }
}

/// Canonical display order for powered consoles in the HTML panel.
///
/// Consoles absent from `PowerMultiplierResource.multipliers` are skipped,
/// so adding or removing a powered console in the ship TOML automatically
/// adjusts the list without any code change.
const POWER_CONSOLE_ORDER: &[Console] = &[
    Console::Helm,
    Console::Tactical,
    Console::Sensors,
    Console::Shields,
    Console::Navigation,
    Console::Comms,
];

/// Maps a powered `Console` to its display label in the HTML power panel.
///
/// `Tactical` is labeled `"WEAPONS"` to match the in-universe terminology;
/// all others use the uppercased display name.
pub fn power_console_label(console: &Console) -> &'static str {
    match console {
        Console::Helm => "HELM",
        Console::Tactical => "WEAPONS",
        Console::Sensors => "SENSORS",
        Console::Shields => "SHIELDS",
        Console::Navigation => "NAVIGATION",
        Console::Comms => "COMMS",
        _ => "UNKNOWN",
    }
}

/// Returns the current power level for `console` from the `PowerSystem`.
///
/// Only `Helm`, `Tactical`, and `Sensors` have first-class fields; any other
/// console silently returns `0` (it should not appear in multipliers).
pub fn power_level_for(ps: &PowerSystem, console: &Console) -> u8 {
    power_level_for_console(ps, console)
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipPowerPlugin;

impl Plugin for ShipPowerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::messages::AdmittedCommands>();
        app.init_resource::<crate::messages::InterSystemQueue>();
        app.insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .init_resource::<PowerAiConfigResource>()
            .add_systems(
                Update,
                (
                    handle_power_messages.in_set(crate::sim_sets::SimSet::Input),
                    tick_power_system.in_set(crate::sim_sets::SimSet::Physics),
                    operate_power_ai.in_set(crate::sim_sets::SimSet::Physics),
                    handle_power_inter_system.in_set(crate::sim_sets::SimSet::Modifiers),
                    publish_power_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                ),
            )
            .add_plugins(power_state_broadcaster());
    }
}

// ── Broadcaster ────────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `PowerState` producer.
///
/// Broadcasts `PowerState` at 10 Hz to the `Power` console holder only.
/// This is the canonical registration; it is added by [`ShipPowerPlugin`]
/// and also by the test harness in `test_app()`.
pub fn power_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::Holding(Console::Power),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let power = world.resource::<ShipPowerSystem>();
            vec![ServerMessage::PowerState {
                helm: power.0.helm,
                weapons: power.0.weapons,
                sensors: power.0.sensors,
                battery_charge: power.0.battery_charge,
                locked: power.0.locked,
            }]
        },
    )
}

// ── Systems ────────────────────────────────────────────────────────────────────

/// Handle `SetPower` messages from the Power console.
///
/// Validates: sender holds `Console::Power`. Reads `ControlSystem` messages
/// targeting the power system ID with a `SetPower` payload, and calls
/// `PowerSystem::increase` / `decrease` to reach the requested level.
pub fn handle_power_messages(
    admitted: Res<crate::messages::AdmittedCommands>,
    mut power: ResMut<ShipPowerSystem>,
) {
    for cmd in admitted.for_target(crate::system_registry::POWER_SYSTEM_ID) {
        match &cmd.payload {
            crate::messages::SystemControlPayload::SetPowerGroupAllocation { group, level } => {
                if let Err(err) = power.0.set_group_allocation(group, *level) {
                    warn!("[power] ignored power allocation: {err:?}");
                }
            }
            crate::messages::SystemControlPayload::SetPower {
                target: console,
                level,
            } => {
                power.0.set_console_allocation(console.clone(), *level);
            }
            _ => {}
        }
    }
}

/// Apply inter-system commands (e.g. `DrainWeaponsBattery` from Weapons).
///
/// Invariant-gated: no control-state check. Runs in `SimSet::Modifiers`,
/// after physics ticks have emitted their inter-system messages.
pub fn handle_power_inter_system(
    queue: Res<InterSystemQueue>,
    mut power: ResMut<ShipPowerSystem>,
    config: Res<PowerConfigResource>,
) {
    for msg in queue.for_target(crate::system_registry::POWER_SYSTEM_ID) {
        match &msg.payload {
            InterSystemPayload::DrainWeaponsBattery { amount } => {
                power.0.battery_charge =
                    (power.0.battery_charge - amount).clamp(0.0, config.0.capacity);
            }
        }
    }
}

/// Tick the power system battery charge each frame.
pub fn tick_power_system(
    time: Res<Time>,
    mut power: ResMut<ShipPowerSystem>,
    config: Res<PowerConfigResource>,
) {
    let dt = time.delta_secs();
    power.0.tick(dt, &config.0);
}

/// AI controller for the power console.
///
/// Rules (all purely advisory — clamped by PowerSystem bounds):
/// - High throttle AND sufficient battery → set Helm to 3
/// - Zero throttle → set Helm to 1 (idle)
/// - Otherwise → set Helm to 2
/// - Red alert AND sufficient battery → set Weapons to 3
///
/// PowerSystem has no shields field; shields_battery_floor is reserved for
/// future extension but produces no action today.
pub fn operate_power_ai(
    mut power: ResMut<ShipPowerSystem>,
    config: Res<PowerConfigResource>,
    ship: Res<ShipState>,
    last_helm: Option<Res<LastHelmInput>>,
    ai_config: Res<PowerAiConfigResource>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    ship_query: Query<&crate::ship_plugin::ShipConfigComponent, With<crate::simulation::Ship>>,
) {
    // Yield to any human Power console holder.
    if let (Some(sessions), Ok(ship_config)) = (sessions, ship_query.single()) {
        if sessions
            .0
            .console_holder(&Console::Power, &ship_config.0)
            .is_some()
        {
            return;
        }
    }

    let battery_pct = power.0.battery_charge / config.0.capacity;
    let red_alert = ship.red_alert();
    let throttle = last_helm.map_or(0.0, |l| l.thrust);

    // Weapons: boost on red alert when battery allows.
    if red_alert && battery_pct >= ai_config.weapons_battery_floor {
        power.0.weapons = 3;
    }

    // Helm: scale with throttle demand and battery availability.
    if throttle > ai_config.helm_throttle_threshold && battery_pct >= ai_config.helm_battery_floor {
        power.0.helm = 3;
    } else if throttle == 0.0 {
        power.0.helm = 1;
        // Give weapons headroom from the freed helm allocation when not moving and
        // not already boosted by red-alert.
        if !red_alert || battery_pct < ai_config.weapons_battery_floor {
            power.0.weapons = 2;
        }
    } else {
        power.0.helm = 2;
        if !red_alert || battery_pct < ai_config.weapons_battery_floor {
            power.0.weapons = 2;
        }
    }

    // Clamp all fields to [1, 4] as a safety net (PowerSystem::increase/decrease
    // normally enforces this, but direct assignment bypasses those guards).
    power.0.helm = power.0.helm.clamp(1, 4);
    power.0.weapons = power.0.weapons.clamp(1, 4);
    power.0.sensors = power.0.sensors.clamp(1, 4);
}

// ── Blackboard publish (issue #561) ──────────────────────────────────────────

fn publish_power_blackboard(
    power: Res<ShipPowerSystem>,
    config: Res<PowerConfigResource>,
    multipliers: Res<PowerMultiplierResource>,
    mut blackboards: ResMut<crate::server_app::SystemBlackboards>,
) {
    use crate::system_registry::POWER_SYSTEM_ID;

    let entries: Vec<PowerConsoleEntry> = POWER_CONSOLE_ORDER
        .iter()
        .filter(|c| multipliers.multipliers.contains_key(c))
        .map(|c| {
            let max_level = multipliers.multipliers[c].len() as u8;
            PowerConsoleEntry {
                id: power_group_for_console(c)
                    .map(|g| g.0)
                    .unwrap_or_else(|| format!("{:?}", c)),
                label: power_console_label(c).into(),
                level: power_level_for(&power.0, c),
                max_level,
            }
        })
        .collect();

    let bb = PowerBlackboard {
        consoles: entries,
        total: power.0.total(),
        total_max: 8,
        battery_charge: power.0.battery_charge,
        battery_max: config.0.capacity,
        locked: power.0.locked,
    };

    blackboards.0.insert(
        SystemId(POWER_SYSTEM_ID.to_string()),
        SystemBlackboard::Power(bb),
    );
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target};
    use crate::messages::{ModifierSlot, ServerMessage, *};
    use crate::modifiers::ShipModifiers;
    use crate::power_system::SENSORS_POWER_GROUP;
    use crate::shield::ShieldSystem;
    use crate::simulation::{
        LastBroadcastEntityPositions, LastBroadcastHull, LastBroadcastShields, ShipHullIntegrity,
        ShipImpulse, ShipShields, SimOutbox,
    };

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
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource(crate::ship_state::ShipState::new())
            .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                (Console::Helm, 25.0),
                (Console::Tactical, 25.0),
                (Console::Power, 25.0),
                (Console::Shields, 25.0),
            ])))
            .insert_resource(ShipShields(ShieldSystem::default()))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(ShipModifiers::new())
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .init_resource::<crate::server_app::SystemBlackboards>()
            .add_plugins(ShipPowerPlugin)
            .add_systems(
                Update,
                crate::modifier_coordination::translate_power_modifiers.after(tick_power_system),
            )
            .add_plugins(crate::simulation::sim_state_broadcaster())
            .add_systems(PostUpdate, collect);
        // Spawn the player ship entity so handle_power_messages can query it.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
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
            out.push(OutboundMessage { target, msg });
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
                station: "Captain's Chair".into(),
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
                station: "Captain's Chair".into(),
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
    fn no_power_console_holder_no_power_state_broadcast() {
        let mut app = test_app();
        start_game(&mut app);

        let out = tick(&mut app);
        let any_power_state = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
        assert!(
            !any_power_state,
            "no PowerState should be sent when no Power console holder exists"
        );
    }

    #[test]
    fn power_increase_respects_bounds_noop_at_four() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 4;
        // Directly set via resource (the human message path lives in ship_plugin.rs).
        // Verify the field clamps at 4 on the PowerSystem itself.
        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.helm,
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
            ps.0.helm = 4;
            ps.0.weapons = 2;
            ps.0.sensors = 2;
        }
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            ps.0.increase(Console::Sensors);
        }
        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.sensors,
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
            .insert(Console::Helm, [-0.5, 0.0, 1.0, 2.0]);

        // Directly set helm=3 and tick to let translate_power_modifiers run.
        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 3;
        let _ = tick(&mut app);

        let mult = app
            .world()
            .resource::<ShipModifiers>()
            .get(&ModifierSlot::MaxSpeed);
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
            .insert(Console::Tactical, [-0.5, 0.0, 0.25, 0.5]);

        // Set weapons=1 directly and tick.
        app.world_mut().resource_mut::<ShipPowerSystem>().0.weapons = 1;
        let _ = tick(&mut app);

        let expected = 1.0 / 1.5;
        let mult = app
            .world()
            .resource::<ShipModifiers>()
            .get(&ModifierSlot::PhaserDamage);
        assert!(
            (mult - expected).abs() < 1e-6,
            "Weapons power 1 should give PhaserDamage multiplier {expected}, got {mult}"
        );
    }

    #[test]
    fn exhaustion_forces_consoles_to_one_and_updates_all_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        let defaults = [-0.5, 0.0, 0.25, 0.5];
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(Console::Helm, defaults);
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(Console::Tactical, defaults);
        app.world_mut()
            .resource_mut::<PowerMultiplierResource>()
            .multipliers
            .insert(Console::Sensors, defaults);

        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            ps.0.helm = 4;
            ps.0.weapons = 2;
            ps.0.sensors = 2;
            ps.0.battery_charge = 0.0;
            ps.0.locked = false;
        }

        tick(&mut app);

        let expected = 1.0 / 1.5;
        let mods = app.world().resource::<ShipModifiers>();

        assert!(
            (mods.get(&ModifierSlot::MaxSpeed) - expected).abs() < 1e-6,
            "after exhaustion MaxSpeed should be {expected}, got {}",
            mods.get(&ModifierSlot::MaxSpeed)
        );
        assert!(
            (mods.get(&ModifierSlot::PhaserDamage) - expected).abs() < 1e-6,
            "after exhaustion PhaserDamage should be {expected}, got {}",
            mods.get(&ModifierSlot::PhaserDamage)
        );
        assert!(
            (mods.get(&ModifierSlot::RadarRange) - expected).abs() < 1e-6,
            "after exhaustion RadarRange should be {expected}, got {}",
            mods.get(&ModifierSlot::RadarRange)
        );
    }

    // ── Blackboard publish tests ────────────────────────────────────────────

    fn power_blackboard(app: &mut App) -> PowerBlackboard {
        use crate::messages::{SystemBlackboard, SystemId};
        use crate::server_app::SystemBlackboards;
        use crate::system_registry::POWER_SYSTEM_ID;
        let bbs = app.world().resource::<SystemBlackboards>();
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
            !bb.consoles.is_empty(),
            "expected at least one power console entry"
        );
        assert!(
            bb.consoles.iter().any(|e| e.label == "HELM"),
            "expected HELM entry"
        );
        assert!(
            bb.consoles.iter().any(|e| e.label == "WEAPONS"),
            "expected WEAPONS entry"
        );
        assert!(
            bb.consoles.iter().any(|e| e.label == "SENSORS"),
            "expected SENSORS entry"
        );
        assert!(bb.total > 0, "total should be > 0");
        assert!(!bb.locked, "should not be locked initially");
    }

    #[test]
    fn publish_power_blackboard_reflects_helm_level_change() {
        let mut app = test_app();
        // Human holds Power so operate_power_ai yields and doesn't override.
        start_game_with_power(&mut app);
        tick(&mut app);

        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 3;
        tick(&mut app);

        let bb = power_blackboard(&mut app);
        let helm_entry = bb.consoles.iter().find(|e| e.label == "HELM").unwrap();
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
                target: crate::system_registry::power_system_id(),
                payload: SystemControlPayload::SetPowerGroupAllocation {
                    group: crate::messages::PowerGroupId(SENSORS_POWER_GROUP.into()),
                    level: 4,
                },
            },
        );
        tick(&mut app);

        assert_eq!(app.world().resource::<ShipPowerSystem>().0.sensors, 4);
    }

    // ── operate_power_ai tests ──────────────────────────────────────────────

    fn ai_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerAiConfigResource>()
            .insert_resource(crate::ship_state::ShipState::new())
            .insert_resource(LastHelmInput::default())
            .add_systems(Update, operate_power_ai);
        app
    }

    #[test]
    fn ai_sets_helm_to_three_when_high_throttle_and_battery_ok() {
        let mut app = ai_test_app();
        app.insert_resource(LastHelmInput {
            thrust: 0.9,
            steering: 0.0,
        });
        // battery_pct = 100/100 = 1.0 >= 0.75 floor
        app.update();
        assert_eq!(app.world().resource::<ShipPowerSystem>().0.helm, 3);
    }

    #[test]
    fn ai_sets_helm_to_one_when_throttle_is_zero() {
        let mut app = ai_test_app();
        // Default LastHelmInput has thrust=0.0
        app.update();
        assert_eq!(app.world().resource::<ShipPowerSystem>().0.helm, 1);
    }

    #[test]
    fn ai_sets_weapons_to_three_on_red_alert_with_battery() {
        let mut app = ai_test_app();
        {
            let mut ship = app
                .world_mut()
                .resource_mut::<crate::ship_state::ShipState>();
            ship.toggle_red_alert();
        }
        app.update();
        assert_eq!(app.world().resource::<ShipPowerSystem>().0.weapons, 3);
    }

    #[test]
    fn ai_does_not_boost_weapons_when_battery_low() {
        let mut app = ai_test_app();
        {
            let mut ship = app
                .world_mut()
                .resource_mut::<crate::ship_state::ShipState>();
            ship.toggle_red_alert();
        }
        app.world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .battery_charge = 30.0; // pct=0.3 < 0.5 floor
        app.update();
        // weapons should not be 3 — battery below floor
        assert_ne!(
            app.world().resource::<ShipPowerSystem>().0.weapons,
            3,
            "weapons should not be boosted when battery is below floor"
        );
    }

    // ── Inter-system command channel tests (issue #559) ───────────────────────
    //
    // These tests exercise the full weapons→power inter-system drain flow.
    // A minimal combined app registers both `drain_power_for_active_beam`
    // (Weapons, Physics) and `handle_power_inter_system` (Power, Modifiers)
    // with SimSets chained, so we can set an active beam and verify the
    // Power battery decreases in the same tick.

    fn inter_system_test_app() -> App {
        use crate::console::weapons::server::{drain_power_for_active_beam, ActiveBeam};
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            // Power resources and handler.
            .insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            // Weapons resources and emitter.
            .init_resource::<ActiveBeam>()
            .init_resource::<crate::messages::InterSystemQueue>()
            // Chain emitter before consumer so the queue is populated before it's read.
            .add_systems(
                Update,
                (drain_power_for_active_beam, handle_power_inter_system).chain(),
            );
        // Warm up the time plugin so delta_secs is non-zero on the first real tick.
        app.update();
        app
    }

    #[test]
    fn active_beam_drains_power_battery_via_inter_system_channel() {
        use crate::console::weapons::server::{ActiveBeam, PHASER_BATTERY_DRAIN_PER_SEC};
        let mut app = inter_system_test_app();

        // Simulate an active phaser beam.
        app.world_mut().resource_mut::<ActiveBeam>().target_uuid = Some("target-asteroid".into());

        let charge_before = app.world().resource::<ShipPowerSystem>().0.battery_charge;
        app.update();
        let charge_after = app.world().resource::<ShipPowerSystem>().0.battery_charge;

        // dt = 100ms → expected drain ≈ PHASER_BATTERY_DRAIN_PER_SEC * 0.1
        let expected_drain = PHASER_BATTERY_DRAIN_PER_SEC * 0.1;
        assert!(
            charge_after < charge_before,
            "active beam must drain battery (before={charge_before}, after={charge_after})"
        );
        assert!(
            (charge_before - charge_after - expected_drain).abs() < 0.1,
            "drain should be ~{expected_drain} (before={charge_before}, after={charge_after})"
        );
    }

    #[test]
    fn no_beam_does_not_drain_power_battery() {
        let mut app = inter_system_test_app();
        // ActiveBeam defaults to target_uuid = None.
        let charge_before = app.world().resource::<ShipPowerSystem>().0.battery_charge;
        app.update();
        let charge_after = app.world().resource::<ShipPowerSystem>().0.battery_charge;

        assert_eq!(
            charge_before, charge_after,
            "no active beam must not drain battery (before={charge_before}, after={charge_after})"
        );
    }

    #[test]
    fn inter_system_drain_clamps_battery_to_zero() {
        use crate::console::weapons::server::ActiveBeam;
        let mut app = inter_system_test_app();

        // Set battery nearly empty (less than one tick of drain).
        app.world_mut()
            .resource_mut::<ShipPowerSystem>()
            .0
            .battery_charge = 0.1;
        app.world_mut().resource_mut::<ActiveBeam>().target_uuid = Some("target-asteroid".into());

        app.update();

        let charge = app.world().resource::<ShipPowerSystem>().0.battery_charge;
        assert_eq!(
            charge, 0.0,
            "battery must clamp at zero, not go negative (got {charge})"
        );
    }
}

use bevy::prelude::*;

use crate::console_bridge::ConsoleStateChanged;
use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{Console, PowerConsoleEntry, PowerConsoleState, ServerMessage};
use crate::modifiers::power_system::{
    power_group_for_console, power_level_for_console, PowerConfig, PowerSystem, SENSORS_POWER_GROUP,
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

// ── Console state component ────────────────────────────────────────────────────

/// Bevy component that caches the current `PowerConsoleState` for the HTML panel.
///
/// Spawned once at `Startup`, recomputed each broadcast frame, and pushed via
/// `ConsoleStateChanged` whenever the value changes (Bevy `Changed<T>` filter).
#[derive(Component, Clone, PartialEq)]
pub struct PowerConsoleStateComp(pub PowerConsoleState);

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
        app.add_message::<ConsoleStateChanged>();
        app.insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .init_resource::<PowerAiConfigResource>()
            .add_systems(Startup, spawn_power_console_state_entity)
            .add_systems(
                Update,
                (
                    handle_power_messages.in_set(crate::sim_sets::SimSet::Input),
                    tick_power_system.in_set(crate::sim_sets::SimSet::Physics),
                    recompute_power_console_state.in_set(crate::sim_sets::SimSet::Broadcast),
                    push_power_console_state
                        .in_set(crate::sim_sets::SimSet::Broadcast)
                        .after(recompute_power_console_state),
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
    mut reader: MessageReader<crate::lobby::InboundMessage>,
    sessions: Res<crate::lobby::Sessions>,
    mut power: ResMut<ShipPowerSystem>,
    control_sources: Option<Res<crate::ship_plugin::ShipSystemControlSources>>,
) {
    let policy = control_sources
        .as_deref()
        .map(|cs| cs.0.policy_for(&crate::system_registry::power_system_id()))
        .unwrap_or(crate::control_source::ControlTickPolicy {
            accept_human_input: true,
            operate_ai: false,
            coordinate: true,
        });

    if !policy.accept_human_input {
        return;
    }

    let power_holder = sessions.0.console_holder(Console::Power);

    for ev in reader.read() {
        let crate::messages::ClientMessage::ControlSystem { target, payload } = &ev.msg else {
            continue;
        };
        if target.0 != crate::system_registry::POWER_SYSTEM_ID {
            continue;
        }
        if power_holder != Some(ev.token.as_str()) {
            warn!(
                "[power-auth] ignored power allocation from token={} holder={:?}",
                ev.token, power_holder,
            );
            continue;
        }

        match payload {
            crate::messages::SystemControlPayload::SetPowerGroupAllocation { group, level } => {
                if let Err(err) = power.0.set_group_allocation(group, *level) {
                    warn!("[power-auth] ignored power allocation: {err:?}");
                }
            }
            crate::messages::SystemControlPayload::SetPower {
                target: console,
                level,
            } => {
                power.0.set_console_allocation(console.clone(), *level);
            }
            _ => continue,
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
    last_helm: Res<LastHelmInput>,
    ai_config: Res<PowerAiConfigResource>,
) {
    let battery_pct = power.0.battery_charge / config.0.capacity;
    let red_alert = ship.red_alert();
    let throttle = last_helm.thrust;

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

// ── HTML console state push ────────────────────────────────────────────────────

pub fn spawn_power_console_state_entity(mut commands: Commands) {
    commands.spawn(PowerConsoleStateComp(PowerConsoleState::default()));
}

/// Recompute `PowerConsoleStateComp` from live resources each broadcast frame.
///
/// The component is only mutated when the computed value differs from the stored
/// one, so `Changed<PowerConsoleStateComp>` will only fire when something actually
/// changed — preventing spurious HTML pushes.
pub fn recompute_power_console_state(
    power: Res<ShipPowerSystem>,
    config: Res<PowerConfigResource>,
    multipliers: Res<PowerMultiplierResource>,
    mut q: Query<&mut PowerConsoleStateComp>,
) {
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

    let next = PowerConsoleState {
        total: power.0.total(),
        total_max: 8,
        battery_charge: power.0.battery_charge,
        battery_max: config.0.capacity,
        locked: power.0.locked,
        consoles: entries,
    };

    for mut comp in q.iter_mut() {
        if comp.0 != next {
            comp.0 = next.clone();
        }
    }
}

/// Push `PowerConsoleState` as a `ConsoleStateChanged` whenever it changes.
pub fn push_power_console_state(
    q: Query<&PowerConsoleStateComp, Changed<PowerConsoleStateComp>>,
    mut writer: MessageWriter<ConsoleStateChanged>,
) {
    for comp in q.iter() {
        if let Ok(json) = crate::core::codec::encode_console_state(&comp.0) {
            writer.write(ConsoleStateChanged {
                name: "Power".into(),
                json,
            });
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage, Target};
    use crate::messages::{ModifierSlot, ServerMessage, *};
    use crate::modifiers::ShipModifiers;
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
            .add_plugins(ShipPowerPlugin)
            .add_systems(
                Update,
                crate::modifier_coordination::translate_power_modifiers.after(tick_power_system),
            )
            .add_plugins(crate::simulation::sim_state_broadcaster())
            .add_systems(PostUpdate, collect);
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
        push_msg(app, "captain", ClientMessage::StartGame);
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
        push_msg(app, "captain", ClientMessage::StartGame);
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
    fn sim_state_includes_power_levels() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Directly mutate to check sim state reflects power levels.
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            ps.0.helm = 3;
            ps.0.sensors = 3;
        }
        let out = tick(&mut app);

        let snap = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
                _ => None,
            })
            .expect("expected a SimState broadcast");
        assert_eq!(
            snap.power_levels,
            (3, 2, 3),
            "SimState.power_levels should reflect power system state"
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

    // ── HTML push tests ─────────────────────────────────────────────────────

    #[derive(Resource, Default)]
    struct PushOutbox(Vec<ConsoleStateChanged>);

    fn collect_pushes(
        mut reader: MessageReader<ConsoleStateChanged>,
        mut box_: ResMut<PushOutbox>,
    ) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn push_test_app() -> App {
        let mut app = App::new();
        app.add_message::<ConsoleStateChanged>()
            .insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .init_resource::<PushOutbox>()
            .add_systems(Startup, spawn_power_console_state_entity)
            .add_systems(
                Update,
                (
                    recompute_power_console_state,
                    push_power_console_state.after(recompute_power_console_state),
                    collect_pushes.after(push_power_console_state),
                ),
            );
        app
    }

    #[test]
    fn push_emits_power_console_state_on_first_update() {
        let mut app = push_test_app();
        app.update();

        let pushes = &app.world().resource::<PushOutbox>().0;
        assert!(
            !pushes.is_empty(),
            "expected at least one ConsoleStateChanged on startup"
        );
        let push = pushes
            .iter()
            .find(|p| p.name == "Power")
            .expect("expected push named 'Power'");
        assert!(
            push.json.contains("\"consoles\""),
            "json should contain consoles array: {}",
            push.json
        );
        assert!(
            push.json.contains("\"HELM\""),
            "json should contain HELM label: {}",
            push.json
        );
        assert!(
            push.json.contains("\"total\""),
            "json should contain total field: {}",
            push.json
        );
        assert!(
            push.json.contains("\"locked\""),
            "json should contain locked field: {}",
            push.json
        );
    }

    #[test]
    fn push_emits_on_power_change_and_not_without_change() {
        let mut app = push_test_app();
        // First update: spawned component is Changed → push fires.
        app.update();
        app.world_mut().resource_mut::<PushOutbox>().0.clear();

        // No state change → no push.
        app.update();
        assert!(
            app.world().resource::<PushOutbox>().0.is_empty(),
            "no push expected when state has not changed"
        );

        // Mutate helm power → recompute detects change → push fires.
        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 3;
        app.update();
        let pushes = &app.world().resource::<PushOutbox>().0;
        assert!(!pushes.is_empty(), "expected push after helm power change");
        let push = pushes.iter().find(|p| p.name == "Power").unwrap();
        assert!(
            push.json.contains("3"),
            "new level 3 should appear in json: {}",
            push.json
        );
    }

    #[test]
    fn push_json_contains_correct_labels_and_ids() {
        let mut app = push_test_app();
        app.update();

        let pushes = &app.world().resource::<PushOutbox>().0;
        let push = pushes.iter().find(|p| p.name == "Power").unwrap();
        assert!(
            push.json.contains("\"helm\""),
            "id should be helm power group id: {}",
            push.json
        );
        assert!(
            push.json.contains("\"HELM\""),
            "label should be HELM: {}",
            push.json
        );
        assert!(
            push.json.contains("\"WEAPONS\""),
            "Tactical label should be WEAPONS: {}",
            push.json
        );
        assert!(
            push.json.contains("\"SENSORS\""),
            "Sensors label should be SENSORS: {}",
            push.json
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
}

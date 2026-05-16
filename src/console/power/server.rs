use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, ServerMessage};
use crate::power_system::{PowerConfig, PowerSystem};

// ── Resources ──────────────────────────────────────────────────────────────────

/// Wraps the pure-Rust power system so it can be used as a Bevy resource.
#[derive(Resource)]
pub struct ShipPowerSystem(pub PowerSystem);

/// Wraps the power config for the ship's power system.
#[derive(Resource)]
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

impl Default for PowerConfigResource {
    fn default() -> Self {
        Self(PowerConfig::default())
    }
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct PowerPlugin;

impl Plugin for PowerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShipPowerSystem(PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .add_systems(Update, (
                handle_power_messages.in_set(crate::sim_sets::SimSet::Input),
                tick_power_system.in_set(crate::sim_sets::SimSet::Physics),
            ))
            .add_plugins(power_state_broadcaster());
    }
}

// ── Broadcaster ────────────────────────────────────────────────────────────────

/// Returns a [`SimBroadcaster`] pre-configured with the `PowerState` producer.
///
/// Broadcasts `PowerState` at 10 Hz to the `Power` console holder only.
/// This is the canonical registration; it is added by [`PowerPlugin`]
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

/// Handle `IncreasePower` and `DecreasePower` messages from the Power console.
///
/// Validates: game is in-progress, sender holds `Console::Power`.
/// Forwards to `PowerSystem::increase` / `decrease` which enforce bounds and lock.
/// Modifier sync is handled separately by `translate_power_modifiers` in the
/// coordination plugin (runs after this system each frame).
pub fn handle_power_messages(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut power: ResMut<ShipPowerSystem>,
) {
    for ev in reader.read() {
        match &ev.msg {
            ClientMessage::IncreasePower { console } => {
                if sessions.0.console_holder(Console::Power) == Some(ev.token.as_str()) {
                    power.0.increase(console.clone());
                }
            }
            ClientMessage::DecreasePower { console } => {
                if sessions.0.console_holder(Console::Power) == Some(ev.token.as_str()) {
                    power.0.decrease(console.clone());
                }
            }
            _ => {}
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

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::HullIntegrity;
    use crate::lobby::{LobbyPlugin, OutboundMessage, Target};
    use crate::messages::{ModifierSlot, ServerMessage, *};
    use crate::modifiers::ShipModifiers;
    use crate::simulation::{ShipHullIntegrity, ShipImpulse, ShipShields, SimOutbox};
    use crate::shield::ShieldSystem;

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
            .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
            .insert_resource(ShipShields(ShieldSystem::default()))
            .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
            .insert_resource(ShipModifiers::new())
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .add_plugins(PowerPlugin)
            .add_systems(Update, crate::modifier_coordination::translate_power_modifiers
                .after(handle_power_messages)
                .after(tick_power_system))
            .add_plugins(crate::simulation::sim_state_broadcaster())
            .add_systems(PostUpdate, collect);
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage { token: token.into(), msg });
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
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn start_game_with_power(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "power", ClientMessage::Identify { token: "power".into(), name: "Monty".into() });
        tick(app);
        push(app, "power", ClientMessage::SelectStation { station: "Power".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        let _ = tick(app);
    }

    #[test]
    fn non_power_sender_increase_power_is_ignored() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 1;

        push(&mut app, "captain", ClientMessage::IncreasePower { console: Console::Helm });
        let _ = tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.helm,
            1,
            "non-Power sender should not be able to increase power"
        );
    }

    #[test]
    fn non_power_sender_decrease_power_is_ignored() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        push(&mut app, "captain", ClientMessage::DecreasePower { console: Console::Sensors });
        let _ = tick(&mut app);

        assert_eq!(
            app.world().resource::<ShipPowerSystem>().0.sensors,
            2,
            "non-Power sender should not be able to decrease power"
        );
    }

    #[test]
    fn power_sender_increase_reflected_in_next_power_state() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Helm });
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out.iter().find_map(|m| match &m.msg {
            ServerMessage::PowerState { helm, .. } => Some(*helm),
            _ => None,
        }).expect("expected a PowerState message for power holder");
        assert_eq!(power_state, 3, "PowerState should show helm=3 after increase");
    }

    #[test]
    fn power_sender_decrease_reflected_in_next_power_state() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        push(&mut app, "power", ClientMessage::DecreasePower { console: Console::Tactical });
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out.iter().find_map(|m| match &m.msg {
            ServerMessage::PowerState { weapons, .. } => Some(*weapons),
            _ => None,
        }).expect("expected a PowerState message");
        assert_eq!(power_state, 1, "PowerState should show weapons=1 after decrease");
    }

    #[test]
    fn power_state_only_sent_to_power_holder() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        let out = tick(&mut app);

        for m in out.iter().filter(|m| matches!(&m.msg, ServerMessage::PowerState { .. })) {
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
        let any_power_state = out.iter().any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
        assert!(!any_power_state, "no PowerState should be sent when no Power console holder exists");
    }

    #[test]
    fn sim_state_includes_power_levels() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Helm });
        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Sensors });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");
        assert_eq!(snap.power_levels, (3, 2, 3), "SimState.power_levels should reflect power system state");
    }

    #[test]
    fn power_increase_respects_bounds_noop_at_four() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 4;

        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Helm });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let power_state = out.iter().find_map(|m| match &m.msg {
            ServerMessage::PowerState { helm, .. } => Some(*helm),
            _ => None,
        }).expect("expected a PowerState message");
        assert_eq!(power_state, 4, "helm should stay at 4 (max bound enforced by PowerSystem)");
    }

    #[test]
    fn increasing_helm_power_updates_max_speed_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(
            Console::Helm, [-0.5, 0.0, 1.0, 2.0],
        );

        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Helm });
        let _ = tick(&mut app);

        let mult = app.world().resource::<ShipModifiers>().get(&ModifierSlot::MaxSpeed);
        assert!((mult - 2.0).abs() < 1e-6,
            "Helm power 3 should give MaxSpeed multiplier 2.0, got {mult}");
    }

    #[test]
    fn decreasing_weapons_power_updates_phaser_damage_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(
            Console::Tactical, [-0.5, 0.0, 0.25, 0.5],
        );

        push(&mut app, "power", ClientMessage::DecreasePower { console: Console::Tactical });
        let _ = tick(&mut app);

        let expected = 1.0 / 1.5;
        let mult = app.world().resource::<ShipModifiers>().get(&ModifierSlot::PhaserDamage);
        assert!((mult - expected).abs() < 1e-6,
            "Weapons power 1 should give PhaserDamage multiplier {expected}, got {mult}");
    }

    #[test]
    fn exhaustion_forces_consoles_to_one_and_updates_all_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        let defaults = [-0.5, 0.0, 0.25, 0.5];
        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(Console::Helm, defaults);
        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(Console::Tactical, defaults);
        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(Console::Sensors, defaults);

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

        assert!((mods.get(&ModifierSlot::MaxSpeed) - expected).abs() < 1e-6,
            "after exhaustion MaxSpeed should be {expected}, got {}", mods.get(&ModifierSlot::MaxSpeed));
        assert!((mods.get(&ModifierSlot::PhaserDamage) - expected).abs() < 1e-6,
            "after exhaustion PhaserDamage should be {expected}, got {}", mods.get(&ModifierSlot::PhaserDamage));
        assert!((mods.get(&ModifierSlot::RadarRange) - expected).abs() < 1e-6,
            "after exhaustion RadarRange should be {expected}, got {}", mods.get(&ModifierSlot::RadarRange));
    }

    #[test]
    fn power_increase_respects_total_cap_of_eight() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 4;

        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Sensors });
        let _ = tick(&mut app);

        let out = tick(&mut app);
        let power_state = out.iter().find_map(|m| match &m.msg {
            ServerMessage::PowerState { sensors, .. } => Some(*sensors),
            _ => None,
        }).expect("expected a PowerState message");
        assert_eq!(power_state, 2, "sensors should stay at 2 when total is already at the cap of 8");
        assert_eq!(app.world().resource::<ShipPowerSystem>().0.total(), 8,
            "total should remain 8");
    }
}

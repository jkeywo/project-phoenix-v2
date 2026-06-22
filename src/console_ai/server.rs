//! Bevy orchestrator for server-side console AI.
//!
//! Manages complexity-preset state used by Tactical AI (torpedo auto-fire,
//! frequency match) and viewscreen. Science/Sensors AI is handled by
//! `ship::sensors`.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::lobby::OutboundMessage;
use crate::messages::{Console, ServerMessage};

// ── Constants ──────────────────────────────────────────────────────────────

// AI rule keys, matching the `[preset.ai]` table keys in
// `assets/complexity/*.toml`.
pub const AI_RULE_TORPEDO_AUTO_FIRE: &str = "torpedo_auto_fire";
pub const AI_RULE_FREQUENCY_MATCH: &str = "frequency_match";

// ── Resources ──────────────────────────────────────────────────────────────

/// Parsed complexity configs per console, sourced from the player ship's
/// `complexity_toml` references via the runtime config cache (the same TOMLs
/// the lobby preloads). Built once by [`build_complexity_rules`].
///
/// Together with [`ConsoleComplexityState`] (the live per-console preset
/// selections) this answers "which AI rules and delegation grants are active
/// right now" — see [`Self::active_preset`] and [`Self::ai_rule`].
#[derive(Resource, Default)]
pub struct ComplexityRules {
    pub per_console: HashMap<Console, crate::complexity::ComplexityConfig>,
    /// True once the player-ship config and all referenced complexity TOMLs
    /// have been loaded and the map built.
    pub loaded: bool,
}

impl ComplexityRules {
    /// The currently-active preset for `console`, given the live preset
    /// selections in `ConsoleComplexityState`. `None` when the console has
    /// no selection yet, no complexity TOML, or the selected name is not a
    /// preset in its config.
    pub fn active_preset<'a>(
        &'a self,
        console: &Console,
        state: &ConsoleComplexityState,
    ) -> Option<&'a crate::complexity::ComplexityPreset> {
        let name = state.presets.get(console)?;
        self.per_console.get(console)?.get_preset(name)
    }

    /// The named `[preset.ai]` rule on `console`'s active preset, if any.
    /// `Some` means the behaviour is enabled right now.
    pub fn ai_rule<'a>(
        &'a self,
        console: &Console,
        state: &ConsoleComplexityState,
        rule: &str,
    ) -> Option<&'a crate::complexity::AiBehaviorConfig> {
        self.active_preset(console, state)?.ai.get(rule)
    }

    /// Native/test helper: build rules by reading the player-ship template
    /// and its complexity TOMLs from the filesystem — the same mapping
    /// [`build_complexity_rules`] derives from the wasm config cache. Tests
    /// using this exercise the shipped asset files, so asset/behaviour drift
    /// shows up as test failures.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_asset_files() -> Self {
        let mut per_console = HashMap::new();
        let ship = std::fs::read_to_string("assets/entities/player_ship.toml")
            .ok()
            .and_then(|s| crate::entity_config::EntityConfig::from_toml(&s).ok());
        if let Some(ship) = ship {
            for (console, path) in ship.complexity_toml_by_console() {
                if let Ok(toml_str) = std::fs::read_to_string(path) {
                    if let Ok(cfg) = crate::complexity::parse_complexity_config(&toml_str) {
                        per_console.insert(console, cfg);
                    }
                }
            }
        }
        Self {
            per_console,
            loaded: true,
        }
    }
}

/// Float param from an AI rule, with a fallback for omitted params.
pub fn ai_param_f32(rule: &crate::complexity::AiBehaviorConfig, key: &str, default: f32) -> f32 {
    rule.params
        .get(key)
        .and_then(|v| v.as_float())
        .map(|v| v as f32)
        .unwrap_or(default)
}

/// Build [`ComplexityRules`] from the runtime config cache once the player
/// ship template and every complexity TOML it references have been loaded.
/// Runs each tick until it succeeds, then becomes a no-op.
fn build_complexity_rules(mut rules: ResMut<ComplexityRules>) {
    if rules.loaded {
        return;
    }
    let cache = crate::config_cache::get_config_cache();
    let Some(ship_config) = cache.get("assets/entities/player_ship.toml") else {
        return;
    };
    let refs = ship_config.complexity_toml_by_console();
    let resources = crate::config_cache::get_complexity_resources();
    if refs.iter().any(|(_, path)| !resources.contains_key(*path)) {
        return;
    }
    rules.per_console = refs
        .into_iter()
        .filter_map(|(console, path)| resources.get(path).map(|cfg| (console, cfg.clone())))
        .collect();
    rules.loaded = true;
}

/// Server-authoritative per-console complexity preset.
///
/// Updated whenever a `ComplexityChanged` message is broadcast.
/// The AI orchestrator reads this to decide whether to run.
#[derive(Resource, Default, Clone)]
pub struct ConsoleComplexityState {
    pub presets: HashMap<Console, String>,
}

impl ConsoleComplexityState {
    /// Returns `true` when the given console is currently at "Low" complexity.
    pub fn is_low(&self, console: &Console) -> bool {
        self.presets
            .get(console)
            .map(|p| p == "Low")
            .unwrap_or(false)
    }

    /// Update the preset for a console.
    pub fn set(&mut self, console: Console, preset_name: String) {
        self.presets.insert(console, preset_name);
    }
}

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct ConsoleAiPlugin;

impl Plugin for ConsoleAiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConsoleComplexityState>()
            .init_resource::<ComplexityRules>()
            .add_systems(
                Update,
                (
                    build_complexity_rules.in_set(crate::sim_sets::SimSet::Input),
                    track_complexity_changes.in_set(crate::sim_sets::SimSet::Input),
                ),
            );
    }
}

// ── Systems ────────────────────────────────────────────────────────────────

/// Update `ConsoleComplexityState` whenever an outbound `ComplexityChanged`
/// message is observed.  We tap the outbound message stream so the AI state
/// stays consistent with what every client was told.
fn track_complexity_changes(
    mut outbound: MessageReader<OutboundMessage>,
    mut complexity: ResMut<ConsoleComplexityState>,
) {
    for msg in outbound.read() {
        if let ServerMessage::ComplexityChanged {
            console,
            preset_name,
        } = &msg.msg
        {
            complexity.set(console.clone(), preset_name.clone());
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::impulse::ImpulseState;
    use crate::lobby::{InboundMessage, WorldResource};
    use crate::lobby::{LobbyPlugin, OutboundMessage, Target};
    use crate::messages::*;
    use crate::repair_teams::RepairTeams;
    use crate::shield::ShieldSystem;
    use crate::ship_plugin::LastHelmInput;
    use crate::simulation::{
        ActiveBeam, CurrentPhaserMode, PhaserCooldown, PowerConfigResource,
        PowerMultiplierResource, ShipHullIntegrity, ShipImpulse, ShipPowerSystem, ShipRepairTeams,
        ShipShields, SimOutbox, TorpedoSystemResource, TrackedEntities, WeaponsTarget,
    };
    use crate::ship_state::ShipState;
    use crate::torpedo::{TorpedoConfig, TorpedoSystem};

    #[derive(Resource, Default)]
    struct Inbox(Vec<InboundMessage>);

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect_outbound(mut reader: MessageReader<OutboundMessage>, mut outbox: ResMut<Outbox>) {
        for m in reader.read() {
            outbox.0.push(m.clone());
        }
    }

    fn collect_inbound(mut reader: MessageReader<InboundMessage>, mut inbox: ResMut<Inbox>) {
        for m in reader.read() {
            inbox.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(ConsoleAiPlugin)
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
                (crate::messages::Console::Helm, 25.0),
                (crate::messages::Console::Tactical, 25.0),
                (crate::messages::Console::Power, 25.0),
                (crate::messages::Console::Shields, 25.0),
            ])))
            .insert_resource(ShipShields(ShieldSystem::default()))
            .insert_resource(ShipImpulse(ImpulseState::new()))
            .init_resource::<WorldResource>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .add_message::<crate::simulation::AsteroidDestroyedVfx>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<CurrentPhaserMode>()
            .init_resource::<crate::weapons_plugin::PhaserCombatConfigResource>()
            .insert_resource(ShipRepairTeams(RepairTeams::default()))
            .insert_resource(crate::modifiers::ShipModifiers::new())
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
                TorpedoConfig::default(),
            )))
            .insert_resource(ShipPowerSystem(crate::power_system::PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .init_resource::<TrackedEntities>()
            .init_resource::<LastHelmInput>()
            .init_resource::<SimOutbox>()
            .init_resource::<Inbox>()
            .init_resource::<Outbox>()
            .add_systems(PostUpdate, collect_inbound)
            .add_systems(PostUpdate, collect_outbound);
        app
    }

    fn push_outbound(app: &mut App, msg: ServerMessage) {
        app.world_mut()
            .resource_mut::<Messages<OutboundMessage>>()
            .write(OutboundMessage {
                target: Target::All,
                msg,
            });
    }

    fn push_inbound(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> (Vec<InboundMessage>, Vec<OutboundMessage>) {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let inbound = app.world().resource::<Inbox>().0.clone();
        let mut outbound = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            outbound.push(OutboundMessage { target, msg });
        }
        app.world_mut().resource_mut::<Inbox>().0.clear();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        (inbound, outbound)
    }

    #[test]
    fn complexity_state_updates_on_complexity_changed_message() {
        let mut app = test_app();
        app.update();

        push_outbound(
            &mut app,
            ServerMessage::ComplexityChanged {
                console: Console::Tactical,
                preset_name: "Low".into(),
            },
        );
        app.update();

        let state = app.world().resource::<ConsoleComplexityState>();
        assert!(
            state.is_low(&Console::Tactical),
            "complexity state should update to Low on ComplexityChanged"
        );
    }

    #[test]
    fn complexity_state_updates_back_to_full() {
        let mut app = test_app();
        app.update();

        push_outbound(
            &mut app,
            ServerMessage::ComplexityChanged {
                console: Console::Tactical,
                preset_name: "Low".into(),
            },
        );
        app.update();
        push_outbound(
            &mut app,
            ServerMessage::ComplexityChanged {
                console: Console::Tactical,
                preset_name: "Std".into(),
            },
        );
        app.update();

        let state = app.world().resource::<ConsoleComplexityState>();
        assert!(
            !state.is_low(&Console::Tactical),
            "complexity state should be Full after switching back"
        );
    }
}

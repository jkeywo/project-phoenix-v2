//! Bevy orchestrator for server-side console AI.
//!
//! This plugin runs per-tick AI decision functions from `console_ai` and
//! synthesises the same `InboundMessage` types that a human player would
//! produce. AI only runs on **occupied** consoles whose *active complexity
//! preset* carries the matching `[preset.ai]` rule in that console's
//! complexity TOML (see [`ComplexityRules`]).
//!
//! When the holder switches preset, `ConsoleComplexityState` is updated and
//! any AI rules absent from the new preset immediately stop generating
//! actions.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::console_ai::{
    tick_auto_match_frequency, tick_frequency_hint, FrequencyHintInput, FrequencyHintState,
    FrequencyMatchInput, FrequencyMatchState,
};
use crate::lobby::{InboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, ServerMessage};
use crate::ship_state::ShipState;
use crate::simulation::SimOutbox;
use crate::simulation::WeaponsTarget;

// ── Constants ──────────────────────────────────────────────────────────────

// AI rule keys, matching the `[preset.ai]` table keys in
// `assets/complexity/*.toml`.
pub const AI_RULE_TORPEDO_AUTO_FIRE: &str = "torpedo_auto_fire";
pub const AI_RULE_FREQUENCY_MATCH: &str = "frequency_match";
pub const AI_RULE_AUTO_HINT: &str = "auto_hint";

// These constants are TOML-param fallbacks used by `ai_param_f32` when an
// `[preset.ai]` rule block omits the corresponding key. The canonical values
// live in the complexity TOML files (e.g. `assets/complexity/sensors.toml`);
// these exist only as compile-time safety nets for future presets that forget
// to specify a param. They must stay in sync with the shipped TOML values.
const DEFAULT_AUTO_HINT_DELAY_SECS: f32 = 3.0;
const DEFAULT_AUTO_MATCH_DELAY_SECS: f32 = 3.0;

// ── Resources ──────────────────────────────────────────────────────────────

/// Wraps `FrequencyHintState` as a Bevy resource so it persists between frames.
#[derive(Resource, Default)]
pub struct FrequencyHintTimer(pub FrequencyHintState);

/// Wraps `FrequencyMatchState` as a Bevy resource so it persists between frames.
#[derive(Resource, Default)]
pub struct FrequencyMatchTimer(pub FrequencyMatchState);

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
fn ai_param_f32(rule: &crate::complexity::AiBehaviorConfig, key: &str, default: f32) -> f32 {
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
    // Wait until every referenced complexity TOML has been fetched and
    // parsed; a TOML that fails to parse never arrives, which (deliberately)
    // keeps the AI off rather than running with partial rules.
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
            .init_resource::<FrequencyHintTimer>()
            .init_resource::<FrequencyMatchTimer>()
            .add_systems(
                Update,
                (
                    build_complexity_rules.in_set(crate::sim_sets::SimSet::Input),
                    track_complexity_changes.in_set(crate::sim_sets::SimSet::Input),
                    run_science_hint_ai
                        .in_set(crate::sim_sets::SimSet::Input)
                        .after(track_complexity_changes),
                    run_auto_match_ai
                        .in_set(crate::sim_sets::SimSet::Input)
                        .after(track_complexity_changes),
                ),
            );
    }
}

// ── Systems ────────────────────────────────────────────────────────────────

/// Update `ConsoleComplexityState` whenever an outbound `ComplexityChanged`
/// message is observed.  We tap the outbound message stream so the AI state
/// stays consistent with what every client was told.
fn track_complexity_changes(
    mut outbound: MessageReader<crate::lobby::OutboundMessage>,
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


/// Run the Sensors frequency-hint AI.
///
/// Conditions to run:
/// 1. Game is InProgress
/// 2. Sensors' active preset has the `auto_hint` `[preset.ai]` rule (the
///    readout is hidden, so the AI provides the hint)
/// 3. Tactical is **not** auto-matching (its active preset lacks
///    `frequency_match` — the player is controlling frequency and needs
///    the hint)
/// 4. Tactical console is occupied (someone to send the hint to)
/// 5. A target is currently locked on Tactical
///
/// After `auto_hint_delay_secs` (an `auto_hint` rule param) of continuous
/// lock on the same target, sends a `FrequencyHint` outbound message
/// addressed to the Tactical holder.
///
/// The timer resets when:
/// - The locked target changes
/// - The enabling preset conditions stop holding (checked each tick).
fn run_science_hint_ai(
    sessions: Res<Sessions>,
    complexity: Res<ConsoleComplexityState>,
    rules: Res<ComplexityRules>,
    ship: Res<ShipState>,
    weapons_target: Res<WeaponsTarget>,
    time: Res<Time>,
    mut hint_timer: ResMut<FrequencyHintTimer>,
    mut outbox: ResMut<SimOutbox>,
) {
    // Hint is only relevant when Sensors' preset enables it and Tactical is
    // not auto-matching the frequency itself.
    let hint_rule = rules.ai_rule(&Console::Sensors, &complexity, AI_RULE_AUTO_HINT);
    let tactical_auto_matches = rules
        .ai_rule(&Console::Tactical, &complexity, AI_RULE_FREQUENCY_MATCH)
        .is_some();
    let Some(hint_rule) = hint_rule.filter(|_| !tactical_auto_matches) else {
        // Reset timer when conditions aren't met so it doesn't carry over.
        hint_timer.0 = FrequencyHintState::default();
        return;
    };

    // Need a Tactical holder to send the hint to.
    let Some(tactical_token) = sessions.0.console_holder(Console::Tactical) else {
        hint_timer.0 = FrequencyHintState::default();
        return;
    };

    let input = FrequencyHintInput {
        locked_target: weapons_target.0.clone(),
        correct_frequency: ship.phaser_frequency,
        dt: time.delta_secs(),
        delay_secs: ai_param_f32(
            hint_rule,
            "auto_hint_delay_secs",
            DEFAULT_AUTO_HINT_DELAY_SECS,
        ),
    };

    use crate::console_ai::FrequencyHintOutput;
    use crate::lobby::Target;

    if let FrequencyHintOutput::Hint { frequency } = tick_frequency_hint(&mut hint_timer.0, &input)
    {
        outbox.0.push((
            Target::Token(tactical_token.to_string()),
            ServerMessage::FrequencyHint { frequency },
        ));
    }
}

/// Run the auto-match frequency AI when Tactical's preset enables it and
/// Sensors is assisted or unmanned.
///
/// Conditions to run:
/// 1. Game is InProgress
/// 2. Tactical's active preset has the `frequency_match` `[preset.ai]` rule
///    (phaser-frequency control is delegated to AI)
/// 3. Sensors' active preset has the `auto_hint` rule (assisted) OR Sensors
///    is unmanned (no holder)
/// 4. Tactical console is occupied (someone to receive the synthesised message)
/// 5. A target is currently locked on Tactical
///
/// After `auto_match_delay_secs` (a `frequency_match` rule param) of
/// continuous lock on the same target, synthesises `SetPhaserFrequency` as
/// an `InboundMessage` from the Tactical holder token — the same path a
/// human player would use.
///
/// The frequency persists at its last set value when the trigger ends.
/// There is no auto-revert.
///
/// The pending countdown is cancelled when:
/// - The enabling presets stop holding (trigger_active becomes false)
/// - The locked target changes (handled inside `tick_auto_match_frequency`)
fn run_auto_match_ai(
    sessions: Res<Sessions>,
    complexity: Res<ConsoleComplexityState>,
    rules: Res<ComplexityRules>,
    ship: Res<ShipState>,
    weapons_target: Res<WeaponsTarget>,
    time: Res<Time>,
    mut match_timer: ResMut<FrequencyMatchTimer>,
    mut writer: MessageWriter<InboundMessage>,
) {
    // Tactical's active preset must enable frequency matching.
    let Some(match_rule) = rules.ai_rule(&Console::Tactical, &complexity, AI_RULE_FREQUENCY_MATCH)
    else {
        match_timer.0 = FrequencyMatchState::default();
        return;
    };

    // Trigger: Sensors is assisted (auto_hint preset active) OR unmanned.
    let sensors_assisted = rules
        .ai_rule(&Console::Sensors, &complexity, AI_RULE_AUTO_HINT)
        .is_some();
    let sensors_unmanned = sessions.0.console_holder(Console::Sensors).is_none();
    let trigger_active = sensors_assisted || sensors_unmanned;

    // Need a Tactical holder to synthesise the message on behalf of.
    let Some(tactical_token) = sessions.0.console_holder(Console::Tactical) else {
        match_timer.0 = FrequencyMatchState::default();
        return;
    };

    let input = FrequencyMatchInput {
        locked_target: weapons_target.0.clone(),
        target_frequency: ship.phaser_frequency,
        dt: time.delta_secs(),
        delay_secs: ai_param_f32(
            match_rule,
            "auto_match_delay_secs",
            DEFAULT_AUTO_MATCH_DELAY_SECS,
        ),
        trigger_active,
    };

    use crate::console_ai::FrequencyMatchOutput;

    if let FrequencyMatchOutput::Match { frequency } =
        tick_auto_match_frequency(&mut match_timer.0, &input)
    {
        writer.write(InboundMessage {
            token: tactical_token.to_string(),
            msg: ClientMessage::SetPhaserFrequency { frequency },
        });
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

// (tube_id_to_msg removed: TorpedoTubeId and messages::TorpedoTube are both
// String aliases now, so no conversion is needed.)

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
        ShipShields, TorpedoSystemResource, TrackedEntities, WeaponsTarget,
    };
    use crate::torpedo::{TorpedoConfig, TorpedoSystem};

    #[derive(Resource, Default)]
    struct Inbox(Vec<InboundMessage>);

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    /// Return ComplexityRules populated from shipped asset files on native,
    /// or an empty default on WASM (tests do not run on WASM).
    fn test_complexity_rules() -> ComplexityRules {
        #[cfg(not(target_arch = "wasm32"))]
        {
            ComplexityRules::from_asset_files()
        }
        #[cfg(target_arch = "wasm32")]
        {
            ComplexityRules::default()
        }
    }

    /// Override a float param on an AI rule across every preset that carries
    /// it, in the test app's `ComplexityRules`. Lets tests tune delays the
    /// way a designer would edit the complexity TOML.
    fn set_ai_param(app: &mut App, console: Console, rule: &str, key: &str, value: f64) {
        let mut rules = app.world_mut().resource_mut::<ComplexityRules>();
        let cfg = rules
            .per_console
            .get_mut(&console)
            .unwrap_or_else(|| panic!("no complexity rules for {console:?}"));
        let mut found = false;
        for preset in &mut cfg.presets {
            if let Some(r) = preset.ai.get_mut(rule) {
                r.params.insert(key.to_string(), toml::Value::Float(value));
                found = true;
            }
        }
        assert!(found, "no preset of {console:?} has ai rule '{rule}'");
    }

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
            // Rules from the shipped asset files, so these tests exercise the
            // real complexity TOMLs (native builds have no wasm config cache).
            .insert_resource(test_complexity_rules())
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
            // Collect inbound messages AFTER the AI plugin generates them
            // (PostUpdate runs after Update where the AI runs).
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

    fn setup_occupied_low_complexity_tactical(app: &mut App) {
        // Register and assign the Tactical console holder
        push_inbound(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        app.update();
        push_inbound(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        app.update();
        // Switch to InProgress phase manually
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        // Set Tactical to Low complexity
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        // Set a locked target
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());
        app.world_mut()
            .resource_mut::<TorpedoSystemResource>()
            .0
            .tube_mut("fore_port")
            .expect("default torpedo tube should exist")
            .load_state = crate::torpedo::TubeLoadState::Loaded;
        // Add the target entity to the world at a position in ForePort arc
        {
            let mut world_res = app.world_mut().resource_mut::<WorldResource>();
            world_res
                .0
                .entities
                .push(EntitySnapshot::asteroid("target-uuid", 0.0, -30.0, 2.0));
        }
        // Also spawn the live ECS entity — run_tactical_ai uses live Transforms
        // (not the WorldResource snapshot) since the fix in e147fe2.
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("target-uuid".into()),
            crate::entity_spawner::EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
                30.0,
            )])),
            bevy::prelude::Transform::from_xyz(0.0, 0.0, -30.0),
        ));
    }

    // ── Tests ────────────────────────────────────────────────────────────

    #[test]
    fn ai_does_not_fire_when_no_console_holder() {
        let mut app = test_app();
        // No one is holding Tactical
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(
            fired.is_empty(),
            "AI must not fire when console is unoccupied"
        );
    }

    #[test]
    fn ai_does_not_fire_at_full_complexity() {
        let mut app = test_app();
        push_inbound(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        app.update();
        push_inbound(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        app.update();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        // Tactical is Full (default / unset → not Low)
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());
        {
            let mut world_res = app.world_mut().resource_mut::<WorldResource>();
            world_res
                .0
                .entities
                .push(EntitySnapshot::asteroid("target-uuid", 0.0, -30.0, 2.0));
        }

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(fired.is_empty(), "AI must not fire at Full complexity");
    }

    #[test]
    fn ai_does_not_fire_in_lobby_phase() {
        let mut app = test_app();
        push_inbound(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        app.update();
        push_inbound(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        app.update();
        // Leave phase as Lobby (default)
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(fired.is_empty(), "AI must not fire during Lobby phase");
    }

    #[test]
    fn ai_fires_with_correct_target_uuid() {
        let mut app = test_app();
        setup_occupied_low_complexity_tactical(&mut app);

        let (inbound, _) = tick(&mut app);
        for msg in &inbound {
            if let ClientMessage::FireTorpedo { target_uuid, .. } = &msg.msg {
                assert_eq!(
                    target_uuid.as_deref(),
                    Some("target-uuid"),
                    "AI fire must reference the locked target"
                );
            }
        }
    }

    #[test]
    fn ai_does_not_fire_without_locked_target() {
        let mut app = test_app();
        push_inbound(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        app.update();
        push_inbound(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        app.update();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        // No target locked
        app.world_mut().resource_mut::<WeaponsTarget>().0 = None;

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(fired.is_empty(), "AI must not fire without a locked target");
    }

    #[test]
    fn complexity_state_updates_on_complexity_changed_message() {
        let mut app = test_app();
        app.update(); // initial tick

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

        // First set to Low
        push_outbound(
            &mut app,
            ServerMessage::ComplexityChanged {
                console: Console::Tactical,
                preset_name: "Low".into(),
            },
        );
        app.update();
        // Then switch back to Full
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

    // ── Science-hint AI tests ──────────────────────────────────────────────

    /// Set up conditions for the Science-hint AI:
    /// - Tactical Full, Science Low, Tactical occupied, target locked.
    fn setup_science_hint_conditions(app: &mut App) {
        // Register a Tactical holder.
        push_inbound(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        app.update();
        push_inbound(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        app.update();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        // Tactical is Full (default), Science is Low.
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Low".into());
        // Tactical complexity left at default (not Low) → Full by omission.
        // Lock a target.
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("hint-target".into());
    }

    #[test]
    fn hint_not_emitted_under_delay() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        // Use a very long delay so a single tick won't fire.
        set_ai_param(
            &mut app,
            Console::Sensors,
            AI_RULE_AUTO_HINT,
            "auto_hint_delay_secs",
            9999.0,
        );

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(
            hints.is_empty(),
            "hint must not emit when delay has not elapsed"
        );
    }

    #[test]
    fn hint_emitted_when_delay_reached() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        // Use zero delay so any elapsed time triggers.
        set_ai_param(
            &mut app,
            Console::Sensors,
            AI_RULE_AUTO_HINT,
            "auto_hint_delay_secs",
            0.0,
        );

        // Inject elapsed time directly into the hint timer.
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("hint-target".into()),
            elapsed_secs: 5.0,
            hint_sent: false,
        };

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(!hints.is_empty(), "hint must emit when delay has elapsed");
    }

    #[test]
    fn hint_not_emitted_when_tactical_is_low() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        // Both Tactical and Science Low → hint should NOT emit (Tactical player
        // doesn't need the hint, auto-fire handles frequency).
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        set_ai_param(
            &mut app,
            Console::Sensors,
            AI_RULE_AUTO_HINT,
            "auto_hint_delay_secs",
            0.0,
        );
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("hint-target".into()),
            elapsed_secs: 5.0,
            hint_sent: false,
        };

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(
            hints.is_empty(),
            "hint must not emit when Tactical is also Low"
        );
    }

    #[test]
    fn hint_not_emitted_when_science_is_full() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        // Science Full → player sees the readout, no hint needed.
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Std".into());
        set_ai_param(
            &mut app,
            Console::Sensors,
            AI_RULE_AUTO_HINT,
            "auto_hint_delay_secs",
            0.0,
        );
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("hint-target".into()),
            elapsed_secs: 5.0,
            hint_sent: false,
        };

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(hints.is_empty(), "hint must not emit when Science is Full");
    }

    #[test]
    fn hint_not_emitted_without_locked_target() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        app.world_mut().resource_mut::<WeaponsTarget>().0 = None;
        set_ai_param(
            &mut app,
            Console::Sensors,
            AI_RULE_AUTO_HINT,
            "auto_hint_delay_secs",
            0.0,
        );

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(
            hints.is_empty(),
            "hint must not emit when no target is locked"
        );
    }

    #[test]
    fn hint_not_emitted_without_tactical_holder() {
        let mut app = test_app();
        // Science Low, no Tactical holder.
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("hint-target".into());
        set_ai_param(
            &mut app,
            Console::Sensors,
            AI_RULE_AUTO_HINT,
            "auto_hint_delay_secs",
            0.0,
        );
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("hint-target".into()),
            elapsed_secs: 5.0,
            hint_sent: false,
        };

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(
            hints.is_empty(),
            "hint must not emit without a Tactical holder"
        );
    }

    #[test]
    fn target_change_resets_hint_timer_in_plugin() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        set_ai_param(
            &mut app,
            Console::Sensors,
            AI_RULE_AUTO_HINT,
            "auto_hint_delay_secs",
            0.0,
        );
        // Fake nearly-elapsed timer for old target.
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("old-target".into()),
            elapsed_secs: 2.9,
            hint_sent: false,
        };

        // Change the locked target.
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("new-target".into());

        // Tick — the target change should reset the timer; no hint yet.
        // (elapsed = 0.0 + dt, which is tiny, so delay=0.0 means it WILL fire
        // immediately with the new target because tick_frequency_hint resets to
        // elapsed=0 then adds dt. Let's use a longer delay to confirm reset.)
        set_ai_param(
            &mut app,
            Console::Sensors,
            AI_RULE_AUTO_HINT,
            "auto_hint_delay_secs",
            100.0,
        );
        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound
            .iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(
            hints.is_empty(),
            "after target change, timer should reset and hint should not fire"
        );
        // Confirm the timer is now tracking the new target.
        let state = app.world().resource::<FrequencyHintTimer>();
        assert_eq!(state.0.current_target.as_deref(), Some("new-target"));
    }

    // ── Auto-match frequency AI plugin tests ─────────────────────────────

    /// Set up conditions for the auto-match AI:
    /// both Tactical and Science Low, Tactical occupied, target locked.
    fn setup_auto_match_conditions(app: &mut App) {
        push_inbound(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        app.update();
        push_inbound(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        app.update();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("match-target".into());
        // Set a known phaser frequency to match against.
        app.world_mut().resource_mut::<ShipState>().phaser_frequency = 0.65;
    }

    #[test]
    fn auto_match_not_emitted_under_delay() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        // Very long delay — single tick won't fire.
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            9999.0,
        );

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            matched.is_empty(),
            "auto-match must not fire before delay elapses"
        );
    }

    #[test]
    fn auto_match_emitted_when_delay_reached() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        // Zero delay — triggers immediately once any elapsed time is added.
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            0.0,
        );
        // Pre-seed elapsed time so a single tick fires.
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            !matched.is_empty(),
            "auto-match must fire when delay has elapsed"
        );
    }

    #[test]
    fn auto_match_emits_correct_frequency() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            0.0,
        );
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let freq_msg = inbound
            .iter()
            .find(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }));
        if let Some(msg) = freq_msg {
            if let ClientMessage::SetPhaserFrequency { frequency } = &msg.msg {
                assert!(
                    (*frequency - 0.65).abs() < 1e-5,
                    "auto-match must set frequency to ship.phaser_frequency (0.65), got {}",
                    frequency
                );
            }
        } else {
            panic!("expected SetPhaserFrequency message");
        }
    }

    #[test]
    fn auto_match_not_emitted_when_tactical_is_full() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        // Override Tactical to Full
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            0.0,
        );
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            matched.is_empty(),
            "auto-match must not fire when Tactical is Full"
        );
    }

    #[test]
    fn auto_match_fires_when_science_unmanned() {
        let mut app = test_app();
        // Set up Tactical Low but Science has no holder (unmanned).
        push_inbound(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        app.update();
        push_inbound(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        app.update();
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        // Science NOT set to Low AND no holder → unmanned
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("match-target".into());
        app.world_mut().resource_mut::<ShipState>().phaser_frequency = 0.4;
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            0.0,
        );
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            !matched.is_empty(),
            "auto-match must fire when Science is unmanned"
        );
    }

    #[test]
    fn auto_match_not_emitted_without_locked_target() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        app.world_mut().resource_mut::<WeaponsTarget>().0 = None;
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            0.0,
        );

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            matched.is_empty(),
            "auto-match must not fire without a locked target"
        );
    }

    #[test]
    fn auto_match_not_emitted_without_tactical_holder() {
        let mut app = test_app();
        // No Tactical holder.
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("match-target".into());
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            0.0,
        );
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            matched.is_empty(),
            "auto-match must not fire without a Tactical holder"
        );
    }

    #[test]
    fn auto_match_timer_resets_when_tactical_flips_to_full() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            100.0,
        );
        // Nearly at delay
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 99.0,
            match_sent: false,
        };
        // Flip Tactical to Full mid-countdown
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            matched.is_empty(),
            "pending match must be cancelled when Tactical flips to Full"
        );
        // Timer must be reset
        let state = app.world().resource::<FrequencyMatchTimer>();
        assert!(
            state.0.current_target.is_none(),
            "timer state must reset when Tactical goes Full"
        );
    }

    #[test]
    fn auto_match_no_auto_revert_after_trigger_ends() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        set_ai_param(
            &mut app,
            Console::Tactical,
            AI_RULE_FREQUENCY_MATCH,
            "auto_match_delay_secs",
            0.0,
        );
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        // First tick — match fires
        let (inbound1, _) = tick(&mut app);
        let matched1: Vec<_> = inbound1
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            !matched1.is_empty(),
            "match should fire on first qualifying tick"
        );

        // Now flip both consoles to Full — trigger ends
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());
        app.world_mut()
            .resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Std".into());

        // Second tick — no revert message
        let (inbound2, _) = tick(&mut app);
        let matched2: Vec<_> = inbound2
            .iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(
            matched2.is_empty(),
            "frequency must persist — no auto-revert when trigger ends"
        );
    }

}

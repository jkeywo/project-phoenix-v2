/// Bevy orchestrator for server-side console AI.
///
/// This plugin runs per-tick AI decision functions from `console_ai` and
/// synthesises the same `InboundMessage` types that a human player would
/// produce. AI only runs on **occupied** consoles whose complexity preset is
/// currently "Low".
///
/// If the holder switches from "Low" to "Std" (or back), the complexity
/// state is updated and AI immediately stops generating actions.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::console_ai::{
    auto_fire_torpedo, tick_auto_match_frequency, tick_frequency_hint,
    tick_power_movement_rule, tick_power_red_alert_rule,
    EngageState, FrequencyHintInput, FrequencyHintState, FrequencyMatchInput,
    FrequencyMatchState, PowerEngageOutput, PowerMovementInput,
    PowerRedAlertInput, TorpedoAiInput, TubeSummary,
};
use crate::lobby::{CurrentPhase, InboundMessage, OutboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, GamePhase, ServerMessage, TorpedoTube as MsgTorpedoTube};
use crate::ship_state::ShipState;
use crate::simulation::{LastHelmInput, ShipPowerSystem, TorpedoSystemResource, WeaponsTarget};
use crate::torpedo::TorpedoTubeId;

// ── Constants ──────────────────────────────────────────────────────────────

const LOW_PRESET: &str = "Low";

/// Default delay (seconds) before the frequency hint fires when Science is Low.
const DEFAULT_AUTO_HINT_DELAY_SECS: f32 = 3.0;

/// Default delay (seconds) before the auto-match fires when both consoles are Low.
const DEFAULT_AUTO_MATCH_DELAY_SECS: f32 = 3.0;

// Power AI tuning defaults (used when TOML loading fails).
const DEFAULT_THRUST_THRESHOLD: f32 = 0.7;
const DEFAULT_ENGAGE_DELAY_SECS: f32 = 3.0;
const DEFAULT_BATTERY_ENGAGE_MIN_PCT_MOVEMENT: f32 = 50.0;
const DEFAULT_BATTERY_ENGAGE_MIN_PCT_RED_ALERT: f32 = 10.0;
const DEFAULT_BATTERY_RECHARGE_PCT: f32 = 100.0;

// ── Resources ──────────────────────────────────────────────────────────────

/// Wraps `FrequencyHintState` as a Bevy resource so it persists between frames.
#[derive(Resource, Default)]
pub struct FrequencyHintTimer(pub FrequencyHintState);

/// Configurable delay (in seconds) before the auto-hint fires.
/// Loaded from `assets/complexity/science.toml` `[preset.ai] auto_hint`.
#[derive(Resource)]
pub struct AutoHintDelaySecs(pub f32);

impl Default for AutoHintDelaySecs {
    fn default() -> Self {
        Self(load_auto_hint_delay_secs())
    }
}

/// Read `auto_hint_delay_secs` from the embedded Science complexity TOML.
/// Falls back to `DEFAULT_AUTO_HINT_DELAY_SECS` on any parse failure.
fn load_auto_hint_delay_secs() -> f32 {
    let toml_str = include_str!("../assets/complexity/science.toml");
    if let Ok(config) = crate::complexity::parse_complexity_config(toml_str) {
        if let Some(low) = config.get_preset("Low") {
            if let Some(ai_cfg) = low.ai.get("auto_hint") {
                if let Some(v) = ai_cfg.params.get("auto_hint_delay_secs") {
                    if let Some(f) = v.as_float() {
                        return f as f32;
                    }
                }
            }
        }
    }
    DEFAULT_AUTO_HINT_DELAY_SECS
}

/// Wraps `FrequencyMatchState` as a Bevy resource so it persists between frames.
#[derive(Resource, Default)]
pub struct FrequencyMatchTimer(pub FrequencyMatchState);

/// Configurable delay (in seconds) before the auto-match fires.
/// Loaded from `assets/complexity/tactical.toml` `[preset.ai] frequency_match`.
#[derive(Resource)]
pub struct AutoMatchDelaySecs(pub f32);

impl Default for AutoMatchDelaySecs {
    fn default() -> Self {
        Self(load_auto_match_delay_secs())
    }
}

/// Read `auto_match_delay_secs` from the embedded Tactical complexity TOML.
/// Falls back to `DEFAULT_AUTO_MATCH_DELAY_SECS` on any parse failure.
fn load_auto_match_delay_secs() -> f32 {
    let toml_str = include_str!("../assets/complexity/tactical.toml");
    if let Ok(config) = crate::complexity::parse_complexity_config(toml_str) {
        if let Some(low) = config.get_preset("Low") {
            if let Some(ai_cfg) = low.ai.get("frequency_match") {
                if let Some(v) = ai_cfg.params.get("auto_match_delay_secs") {
                    if let Some(f) = v.as_float() {
                        return f as f32;
                    }
                }
            }
        }
    }
    DEFAULT_AUTO_MATCH_DELAY_SECS
}

/// Tuning parameters for the Power Low AI, loaded from `assets/complexity/power.toml`.
#[derive(Resource, Clone)]
pub struct PowerAiConfig {
    pub thrust_threshold: f32,
    pub movement_engage_delay_secs: f32,
    pub battery_engage_min_pct_movement: f32,
    pub battery_engage_min_pct_red_alert: f32,
    pub red_alert_engage_delay_secs: f32,
    pub battery_recharge_pct: f32,
}

impl Default for PowerAiConfig {
    fn default() -> Self {
        Self::load()
    }
}

impl PowerAiConfig {
    /// Load tuning params from the embedded power TOML. Falls back to
    /// compiled-in defaults on any parse failure.
    pub fn load() -> Self {
        let toml_str = include_str!("../assets/complexity/power.toml");
        let mut cfg = Self {
            thrust_threshold: DEFAULT_THRUST_THRESHOLD,
            movement_engage_delay_secs: DEFAULT_ENGAGE_DELAY_SECS,
            battery_engage_min_pct_movement: DEFAULT_BATTERY_ENGAGE_MIN_PCT_MOVEMENT,
            battery_engage_min_pct_red_alert: DEFAULT_BATTERY_ENGAGE_MIN_PCT_RED_ALERT,
            red_alert_engage_delay_secs: DEFAULT_ENGAGE_DELAY_SECS,
            battery_recharge_pct: DEFAULT_BATTERY_RECHARGE_PCT,
        };
        if let Ok(config) = crate::complexity::parse_complexity_config(toml_str) {
            if let Some(low) = config.get_preset("Low") {
                if let Some(ai_cfg) = low.ai.get("movement_rule") {
                    if let Some(v) = ai_cfg.params.get("thrust_threshold").and_then(|v| v.as_float()) {
                        cfg.thrust_threshold = v as f32;
                    }
                    if let Some(v) = ai_cfg.params.get("engage_delay_secs").and_then(|v| v.as_float()) {
                        cfg.movement_engage_delay_secs = v as f32;
                    }
                    if let Some(v) = ai_cfg.params.get("battery_engage_min_pct").and_then(|v| v.as_float()) {
                        cfg.battery_engage_min_pct_movement = v as f32;
                    }
                    if let Some(v) = ai_cfg.params.get("battery_recharge_pct").and_then(|v| v.as_float()) {
                        cfg.battery_recharge_pct = v as f32;
                    }
                }
                if let Some(ai_cfg) = low.ai.get("red_alert_rule") {
                    if let Some(v) = ai_cfg.params.get("engage_delay_secs").and_then(|v| v.as_float()) {
                        cfg.red_alert_engage_delay_secs = v as f32;
                    }
                    if let Some(v) = ai_cfg.params.get("battery_engage_min_pct").and_then(|v| v.as_float()) {
                        cfg.battery_engage_min_pct_red_alert = v as f32;
                    }
                    if let Some(v) = ai_cfg.params.get("battery_recharge_pct").and_then(|v| v.as_float()) {
                        cfg.battery_recharge_pct = v as f32;
                    }
                }
            }
        }
        cfg
    }
}

/// Persistent engage-state for the Power movement rule (Helm +1).
#[derive(Resource, Default)]
pub struct PowerMovementEngageState(pub EngageState);

/// Persistent engage-state for the Power red-alert rule (Weapons +1).
#[derive(Resource, Default)]
pub struct PowerRedAlertEngageState(pub EngageState);

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
        self.presets.get(console).map(|p| p == LOW_PRESET).unwrap_or(false)
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
            .init_resource::<FrequencyHintTimer>()
            .init_resource::<AutoHintDelaySecs>()
            .init_resource::<FrequencyMatchTimer>()
            .init_resource::<AutoMatchDelaySecs>()
            .init_resource::<PowerAiConfig>()
            .init_resource::<PowerMovementEngageState>()
            .init_resource::<PowerRedAlertEngageState>()
            .add_systems(Update, (
                track_complexity_changes,
                run_tactical_ai.after(track_complexity_changes),
                run_science_hint_ai.after(track_complexity_changes),
                run_auto_match_ai.after(track_complexity_changes),
                run_power_ai.after(track_complexity_changes),
            ));
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
        if let ServerMessage::ComplexityChanged { console, preset_name } = &msg.msg {
            complexity.set(console.clone(), preset_name.clone());
        }
    }
}

/// Run the torpedo auto-fire AI for the Tactical console when:
/// 1. Game is in-progress
/// 2. The Tactical console is occupied (has a connected holder)
/// 3. Tactical complexity is "Low"
///
/// On each qualifying tick, calls `auto_fire_torpedo` and synthesises a
/// `FireTorpedo` `InboundMessage` for every tube that should fire.  Those
/// messages are processed by `handle_fire_torpedo` in the same frame via the
/// normal message pipeline.
fn run_tactical_ai(
    phase: Res<CurrentPhase>,
    sessions: Res<Sessions>,
    complexity: Res<ConsoleComplexityState>,
    ship: Res<ShipState>,
    torpedo_sys: Res<TorpedoSystemResource>,
    weapons_target: Res<WeaponsTarget>,
    world: Res<crate::lobby::WorldResource>,
    mut writer: MessageWriter<InboundMessage>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    // AI only runs on occupied consoles.
    let Some(holder_token) = sessions.0.console_holder(Console::Tactical) else {
        return;
    };

    // AI only runs at Low complexity.
    if !complexity.is_low(&Console::Tactical) {
        return;
    }

    // No target locked → nothing to do.
    let Some(target_uuid) = &weapons_target.0 else {
        return;
    };

    // Look up the target's world position. If the entity is gone (destroyed
    // since the lock was set), skip this tick silently.
    let Some(target_entity) = world.0.entities.iter().find(|e| &e.uuid == target_uuid) else {
        return;
    };
    let (tx, tz) = (target_entity.x(), target_entity.z());

    // Compute bearing from ship to target (same convention as torpedo.is_in_arc).
    let dx = tx - ship.x;
    let dz = tz - ship.z;
    // atan2(dx, -dz) gives world-bearing in yaw convention (0 = forward = -Z)
    let world_bearing = dx.atan2(-dz);
    let bearing = world_bearing - ship.yaw;

    // Asteroids have no shields → target_shields = 0 always satisfies ≤ 0.
    let target_shields = 0i32;

    let ts = &torpedo_sys.0;

    let tubes = [
        TubeSummary {
            id: TorpedoTubeId::ForePort,
            loaded: ts.fore_port.is_loaded(),
            in_arc: ts.fore_port.is_in_arc(bearing),
        },
        TubeSummary {
            id: TorpedoTubeId::ForeStarboard,
            loaded: ts.fore_starboard.is_loaded(),
            in_arc: ts.fore_starboard.is_in_arc(bearing),
        },
        TubeSummary {
            id: TorpedoTubeId::Aft,
            loaded: ts.aft.is_loaded(),
            in_arc: ts.aft.is_in_arc(bearing),
        },
    ];

    let input = TorpedoAiInput {
        target_locked: true, // we already checked above
        target_shields,
        tubes,
        magazine: ts.torpedoes_remaining,
    };

    let tubes_to_fire = auto_fire_torpedo(&input);
    for tube_id in tubes_to_fire {
        let tube = tube_id_to_msg(tube_id);
        writer.write(InboundMessage {
            token: holder_token.to_string(),
            msg: ClientMessage::FireTorpedo {
                tube,
                target_uuid: Some(target_uuid.clone()),
            },
        });
    }
}

/// Run the Science-Low frequency-hint AI.
///
/// Conditions to run:
/// 1. Game is InProgress
/// 2. Tactical is **Full** (player needs the hint — they are controlling frequency)
/// 3. Science is **Low** (the readout is hidden, so the AI provides the hint)
/// 4. Tactical console is occupied (someone to send the hint to)
/// 5. A target is currently locked on Tactical
///
/// After `auto_hint_delay_secs` of continuous lock on the same target, sends a
/// `FrequencyHint` outbound message addressed to the Tactical holder.
///
/// The timer resets when:
/// - The locked target changes
/// - Science complexity changes (back to Full) — handled by clearing the timer
///   via the `ConsoleComplexityState` check each tick.
fn run_science_hint_ai(
    phase: Res<CurrentPhase>,
    sessions: Res<Sessions>,
    complexity: Res<ConsoleComplexityState>,
    ship: Res<ShipState>,
    weapons_target: Res<WeaponsTarget>,
    time: Res<Time>,
    delay: Res<AutoHintDelaySecs>,
    mut hint_timer: ResMut<FrequencyHintTimer>,
    mut writer: MessageWriter<OutboundMessage>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    // Hint is only relevant when Tactical is Full (player manages frequency)
    // and Sensors is Low (readout is hidden).
    if !complexity.is_low(&Console::Sensors) || complexity.is_low(&Console::Tactical) {
        // Reset timer when conditions aren't met so it doesn't carry over.
        hint_timer.0 = FrequencyHintState::default();
        return;
    }

    // Need a Tactical holder to send the hint to.
    let Some(tactical_token) = sessions.0.console_holder(Console::Tactical) else {
        hint_timer.0 = FrequencyHintState::default();
        return;
    };

    let input = FrequencyHintInput {
        locked_target: weapons_target.0.clone(),
        correct_frequency: ship.phaser_frequency,
        dt: time.delta_secs(),
        delay_secs: delay.0,
    };

    use crate::console_ai::FrequencyHintOutput;
    use crate::lobby::Target;

    if let FrequencyHintOutput::Hint { frequency } = tick_frequency_hint(&mut hint_timer.0, &input) {
        writer.write(OutboundMessage {
            target: Target::Token(tactical_token.to_string()),
            msg: ServerMessage::FrequencyHint { frequency },
        });
    }
}

/// Run the auto-match frequency AI when both Tactical and Science are Low
/// (or Science is unmanned).
///
/// Conditions to run:
/// 1. Game is InProgress
/// 2. Tactical is **Low** (phaser-frequency control is delegated to AI)
/// 3. Science is **Low** OR Science is unmanned (no holder)
/// 4. Tactical console is occupied (someone to receive the synthesised message)
/// 5. A target is currently locked on Tactical
///
/// After `auto_match_delay_secs` of continuous lock on the same target,
/// synthesises `SetPhaserFrequency` as an `InboundMessage` from the Tactical
/// holder token — the same path a human player would use.
///
/// The frequency persists at its last set value when the trigger ends.
/// There is no auto-revert.
///
/// The pending countdown is cancelled when:
/// - Either console flips to Full (trigger_active becomes false)
/// - The locked target changes (handled inside `tick_auto_match_frequency`)
fn run_auto_match_ai(
    phase: Res<CurrentPhase>,
    sessions: Res<Sessions>,
    complexity: Res<ConsoleComplexityState>,
    ship: Res<ShipState>,
    weapons_target: Res<WeaponsTarget>,
    time: Res<Time>,
    delay: Res<AutoMatchDelaySecs>,
    mut match_timer: ResMut<FrequencyMatchTimer>,
    mut writer: MessageWriter<InboundMessage>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    // Tactical must be Low for the AI to act.
    if !complexity.is_low(&Console::Tactical) {
        match_timer.0 = FrequencyMatchState::default();
        return;
    }

    // Trigger: Sensors is Low OR Sensors is unmanned (no holder).
    let sensors_is_low = complexity.is_low(&Console::Sensors);
    let sensors_unmanned = sessions.0.console_holder(Console::Sensors).is_none();
    let trigger_active = sensors_is_low || sensors_unmanned;

    // Need a Tactical holder to synthesise the message on behalf of.
    let Some(tactical_token) = sessions.0.console_holder(Console::Tactical) else {
        match_timer.0 = FrequencyMatchState::default();
        return;
    };

    let input = FrequencyMatchInput {
        locked_target: weapons_target.0.clone(),
        target_frequency: ship.phaser_frequency,
        dt: time.delta_secs(),
        delay_secs: delay.0,
        trigger_active,
    };

    use crate::console_ai::FrequencyMatchOutput;

    if let FrequencyMatchOutput::Match { frequency } = tick_auto_match_frequency(&mut match_timer.0, &input) {
        writer.write(InboundMessage {
            token: tactical_token.to_string(),
            msg: ClientMessage::SetPhaserFrequency { frequency },
        });
    }
}

/// Run the Power-Low AI: two independent overflow rules.
///
/// Conditions to run:
/// 1. Game is InProgress
/// 2. Power console is occupied
/// 3. Power complexity is "Low"
///
/// **Movement rule**: sustained thrust ≥ threshold AND battery ≥ min% for
/// `engage_delay_secs` → synthesise `IncreasePower { Helm }`.  Immediate
/// `DecreasePower { Helm }` when battery drops.
///
/// **Red Alert rule**: symmetric — sustained red alert AND battery ≥ min% →
/// `IncreasePower { Tactical }`.  Immediate `DecreasePower { Tactical }` on
/// battery drop.
///
/// Both rules stack independently (both can fire → 8 total).
/// Switching Power to Full (power_is_low = false) cancels pending engages.
fn run_power_ai(
    phase: Res<CurrentPhase>,
    sessions: Res<Sessions>,
    complexity: Res<ConsoleComplexityState>,
    power_sys: Res<ShipPowerSystem>,
    helm_input: Res<LastHelmInput>,
    ship: Res<ShipState>,
    time: Res<Time>,
    config: Res<PowerAiConfig>,
    mut movement_state: ResMut<PowerMovementEngageState>,
    mut red_alert_state: ResMut<PowerRedAlertEngageState>,
    mut writer: MessageWriter<InboundMessage>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    // AI only runs on occupied consoles.
    let Some(holder_token) = sessions.0.console_holder(Console::Power) else {
        return;
    };

    let power_is_low = complexity.is_low(&Console::Power);
    let battery_pct = power_sys.0.battery_charge; // range 0–100
    let dt = time.delta_secs();

    // ── Movement rule ─────────────────────────────────────────────────────

    let prev_movement = movement_state.0.clone();
    let movement_out = tick_power_movement_rule(
        &mut movement_state.0,
        &PowerMovementInput {
            thrust: helm_input.thrust,
            thrust_threshold: config.thrust_threshold,
            engage_delay_secs: config.movement_engage_delay_secs,
            battery_engage_min_pct: config.battery_engage_min_pct_movement,
            battery_recharge_pct: config.battery_recharge_pct,
            battery_pct,
            dt,
            power_is_low,
        },
    );

    // Disengagement must also be synthesised when power_is_low goes false
    // while a rule was Engaged (the state machine resets to Idle but doesn't
    // return Disengage).
    let movement_was_engaged = matches!(prev_movement, EngageState::Engaged);
    let movement_disengaged_implicitly = movement_was_engaged && !power_is_low;

    match movement_out {
        PowerEngageOutput::Engage => {
            writer.write(InboundMessage {
                token: holder_token.to_string(),
                msg: ClientMessage::IncreasePower { console: Console::Helm },
            });
        }
        PowerEngageOutput::Disengage => {
            writer.write(InboundMessage {
                token: holder_token.to_string(),
                msg: ClientMessage::DecreasePower { console: Console::Helm },
            });
        }
        PowerEngageOutput::NoChange => {
            if movement_disengaged_implicitly {
                writer.write(InboundMessage {
                    token: holder_token.to_string(),
                    msg: ClientMessage::DecreasePower { console: Console::Helm },
                });
            }
        }
    }

    // ── Red Alert rule ────────────────────────────────────────────────────

    let prev_red_alert = red_alert_state.0.clone();
    let red_alert_out = tick_power_red_alert_rule(
        &mut red_alert_state.0,
        &PowerRedAlertInput {
            red_alert: ship.red_alert(),
            engage_delay_secs: config.red_alert_engage_delay_secs,
            battery_engage_min_pct: config.battery_engage_min_pct_red_alert,
            battery_recharge_pct: config.battery_recharge_pct,
            battery_pct,
            dt,
            power_is_low,
        },
    );

    let red_alert_was_engaged = matches!(prev_red_alert, EngageState::Engaged);
    let red_alert_disengaged_implicitly = red_alert_was_engaged && !power_is_low;

    match red_alert_out {
        PowerEngageOutput::Engage => {
            writer.write(InboundMessage {
                token: holder_token.to_string(),
                msg: ClientMessage::IncreasePower { console: Console::Tactical },
            });
        }
        PowerEngageOutput::Disengage => {
            writer.write(InboundMessage {
                token: holder_token.to_string(),
                msg: ClientMessage::DecreasePower { console: Console::Tactical },
            });
        }
        PowerEngageOutput::NoChange => {
            if red_alert_disengaged_implicitly {
                writer.write(InboundMessage {
                    token: holder_token.to_string(),
                    msg: ClientMessage::DecreasePower { console: Console::Tactical },
                });
            }
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Convert a `torpedo::TorpedoTubeId` to a `messages::TorpedoTube`.
fn tube_id_to_msg(id: TorpedoTubeId) -> MsgTorpedoTube {
    match id {
        TorpedoTubeId::ForePort => MsgTorpedoTube::ForePort,
        TorpedoTubeId::ForeStarboard => MsgTorpedoTube::ForeStarboard,
        TorpedoTubeId::Aft => MsgTorpedoTube::Aft,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{LobbyPlugin, OutboundMessage, Target};
    use crate::messages::*;
    use crate::simulation::{
        ShipHullIntegrity, ShipImpulse, ShipRepairTeams, ShipShields,
        TorpedoSystemResource, WeaponsTarget,
        BreakdownQueueResource, RepairIconState, ShipPowerSystem,
        PowerConfigResource, PowerMultiplierResource, TrackedEntities,
        ActiveBeam, PhaserCooldown, CurrentPhaserMode, LastHelmInput,
    };
    use crate::damage::HullIntegrity;
    use crate::shield::ShieldSystem;
    use crate::impulse::ImpulseState;
    use crate::repair_teams::RepairTeams;
    use crate::torpedo::{TorpedoConfig, TorpedoSystem};
    use crate::lobby::{InboundMessage, WorldResource};

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
            .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
            .insert_resource(ShipShields(ShieldSystem::default()))
            .insert_resource(ShipImpulse(ImpulseState::new()))
            .init_resource::<WorldResource>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .add_message::<crate::simulation::AsteroidDestroyedVfx>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<CurrentPhaserMode>()
            .insert_resource(ShipRepairTeams(RepairTeams::new()))
            .init_resource::<BreakdownQueueResource>()
            .insert_resource(crate::modifiers::ShipModifiers::new())
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())))
            .init_resource::<RepairIconState>()
            .insert_resource(ShipPowerSystem(crate::power_system::PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .init_resource::<TrackedEntities>()
            .init_resource::<LastHelmInput>()
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
            .write(OutboundMessage { target: Target::All, msg });
    }

    fn push_inbound(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage { token: token.into(), msg });
    }

    fn tick(app: &mut App) -> (Vec<InboundMessage>, Vec<OutboundMessage>) {
        app.update();
        let inbound = app.world().resource::<Inbox>().0.clone();
        let outbound = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Inbox>().0.clear();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        (inbound, outbound)
    }

    fn setup_occupied_low_complexity_tactical(app: &mut App) {
        // Register and assign the Tactical console holder
        push_inbound(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        app.update();
        push_inbound(app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        app.update();
        // Switch to InProgress phase manually
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        // Set Tactical to Low complexity
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        // Set a locked target
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());
        // Add the target entity to the world at a position in ForePort arc
        let mut world_res = app.world_mut().resource_mut::<WorldResource>();
        world_res.0.entities.push(EntitySnapshot::asteroid("target-uuid", 0.0, -30.0, 2.0));
    }

    // ── Tests ────────────────────────────────────────────────────────────

    #[test]
    fn ai_does_not_fire_when_no_console_holder() {
        let mut app = test_app();
        // No one is holding Tactical
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(fired.is_empty(), "AI must not fire when console is unoccupied");
    }

    #[test]
    fn ai_does_not_fire_at_full_complexity() {
        let mut app = test_app();
        push_inbound(&mut app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        app.update();
        push_inbound(&mut app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        app.update();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        // Tactical is Full (default / unset → not Low)
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());
        let mut world_res = app.world_mut().resource_mut::<WorldResource>();
        world_res.0.entities.push(EntitySnapshot::asteroid("target-uuid", 0.0, -30.0, 2.0));
        drop(world_res);

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(fired.is_empty(), "AI must not fire at Full complexity");
    }

    #[test]
    fn ai_does_not_fire_in_lobby_phase() {
        let mut app = test_app();
        push_inbound(&mut app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        app.update();
        push_inbound(&mut app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        app.update();
        // Leave phase as Lobby (default)
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(fired.is_empty(), "AI must not fire during Lobby phase");
    }

    #[test]
    fn ai_fires_torpedo_when_conditions_met_with_target_in_arc() {
        let mut app = test_app();
        setup_occupied_low_complexity_tactical(&mut app);
        // Target at (0, -30) → bearing 0 from ship at origin yaw=0 → in ForePort arc

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(!fired.is_empty(), "AI should fire when all conditions are met");
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
        push_inbound(&mut app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        app.update();
        push_inbound(&mut app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        app.update();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        // No target locked
        app.world_mut().resource_mut::<WeaponsTarget>().0 = None;

        let (inbound, _) = tick(&mut app);
        let fired: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(fired.is_empty(), "AI must not fire without a locked target");
    }

    #[test]
    fn complexity_state_updates_on_complexity_changed_message() {
        let mut app = test_app();
        app.update(); // initial tick

        push_outbound(&mut app, ServerMessage::ComplexityChanged {
            console: Console::Tactical,
            preset_name: "Low".into(),
        });
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
        push_outbound(&mut app, ServerMessage::ComplexityChanged {
            console: Console::Tactical,
            preset_name: "Low".into(),
        });
        app.update();
        // Then switch back to Full
        push_outbound(&mut app, ServerMessage::ComplexityChanged {
            console: Console::Tactical,
            preset_name: "Std".into(),
        });
        app.update();

        let state = app.world().resource::<ConsoleComplexityState>();
        assert!(
            !state.is_low(&Console::Tactical),
            "complexity state should be Full after switching back"
        );
    }

    #[test]
    fn ai_stops_firing_when_preset_switches_to_full() {
        let mut app = test_app();
        setup_occupied_low_complexity_tactical(&mut app);

        // First tick — AI should fire
        let (inbound1, _) = tick(&mut app);
        let fired1: Vec<_> = inbound1.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(!fired1.is_empty(), "AI should fire at Low complexity");

        // Switch to Full
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());

        // Next tick — AI should not fire
        let (inbound2, _) = tick(&mut app);
        let fired2: Vec<_> = inbound2.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::FireTorpedo { .. }))
            .collect();
        assert!(fired2.is_empty(), "AI must not fire after switching to Full");
    }

    // ── Science-hint AI tests ──────────────────────────────────────────────

    /// Set up conditions for the Science-hint AI:
    /// - Tactical Full, Science Low, Tactical occupied, target locked.
    fn setup_science_hint_conditions(app: &mut App) {
        // Register a Tactical holder.
        push_inbound(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        app.update();
        push_inbound(app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        app.update();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        // Tactical is Full (default), Science is Low.
        app.world_mut().resource_mut::<ConsoleComplexityState>()
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
        app.insert_resource(AutoHintDelaySecs(9999.0));

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound.iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(hints.is_empty(), "hint must not emit when delay has not elapsed");
    }

    #[test]
    fn hint_emitted_when_delay_reached() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        // Use zero delay so any elapsed time triggers.
        app.insert_resource(AutoHintDelaySecs(0.0));

        // Inject elapsed time directly into the hint timer.
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("hint-target".into()),
            elapsed_secs: 5.0,
            hint_sent: false,
        };

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound.iter()
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
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.insert_resource(AutoHintDelaySecs(0.0));
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("hint-target".into()),
            elapsed_secs: 5.0,
            hint_sent: false,
        };

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound.iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(hints.is_empty(), "hint must not emit when Tactical is also Low");
    }

    #[test]
    fn hint_not_emitted_when_science_is_full() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        // Science Full → player sees the readout, no hint needed.
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Std".into());
        app.insert_resource(AutoHintDelaySecs(0.0));
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("hint-target".into()),
            elapsed_secs: 5.0,
            hint_sent: false,
        };

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound.iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(hints.is_empty(), "hint must not emit when Science is Full");
    }

    #[test]
    fn hint_not_emitted_without_locked_target() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        app.world_mut().resource_mut::<WeaponsTarget>().0 = None;
        app.insert_resource(AutoHintDelaySecs(0.0));

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound.iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(hints.is_empty(), "hint must not emit when no target is locked");
    }

    #[test]
    fn hint_not_emitted_without_tactical_holder() {
        let mut app = test_app();
        // Science Low, no Tactical holder.
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("hint-target".into());
        app.insert_resource(AutoHintDelaySecs(0.0));
        app.world_mut().resource_mut::<FrequencyHintTimer>().0 = FrequencyHintState {
            current_target: Some("hint-target".into()),
            elapsed_secs: 5.0,
            hint_sent: false,
        };

        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound.iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(hints.is_empty(), "hint must not emit without a Tactical holder");
    }

    #[test]
    fn target_change_resets_hint_timer_in_plugin() {
        let mut app = test_app();
        setup_science_hint_conditions(&mut app);
        app.insert_resource(AutoHintDelaySecs(0.0));
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
        app.insert_resource(AutoHintDelaySecs(100.0));
        let (_, outbound) = tick(&mut app);
        let hints: Vec<_> = outbound.iter()
            .filter(|m| matches!(&m.msg, ServerMessage::FrequencyHint { .. }))
            .collect();
        assert!(hints.is_empty(), "after target change, timer should reset and hint should not fire");
        // Confirm the timer is now tracking the new target.
        let state = app.world().resource::<FrequencyHintTimer>();
        assert_eq!(state.0.current_target.as_deref(), Some("new-target"));
    }

    // ── Auto-match frequency AI plugin tests ─────────────────────────────

    /// Set up conditions for the auto-match AI:
    /// both Tactical and Science Low, Tactical occupied, target locked.
    fn setup_auto_match_conditions(app: &mut App) {
        push_inbound(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        app.update();
        push_inbound(app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        app.update();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.world_mut().resource_mut::<ConsoleComplexityState>()
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
        app.insert_resource(AutoMatchDelaySecs(9999.0));

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(matched.is_empty(), "auto-match must not fire before delay elapses");
    }

    #[test]
    fn auto_match_emitted_when_delay_reached() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        // Zero delay — triggers immediately once any elapsed time is added.
        app.insert_resource(AutoMatchDelaySecs(0.0));
        // Pre-seed elapsed time so a single tick fires.
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(!matched.is_empty(), "auto-match must fire when delay has elapsed");
    }

    #[test]
    fn auto_match_emits_correct_frequency() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        app.insert_resource(AutoMatchDelaySecs(0.0));
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let freq_msg = inbound.iter()
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
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());
        app.insert_resource(AutoMatchDelaySecs(0.0));
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(matched.is_empty(), "auto-match must not fire when Tactical is Full");
    }

    #[test]
    fn auto_match_fires_when_science_unmanned() {
        let mut app = test_app();
        // Set up Tactical Low but Science has no holder (unmanned).
        push_inbound(&mut app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        app.update();
        push_inbound(&mut app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        app.update();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        // Science NOT set to Low AND no holder → unmanned
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("match-target".into());
        app.world_mut().resource_mut::<ShipState>().phaser_frequency = 0.4;
        app.insert_resource(AutoMatchDelaySecs(0.0));
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(!matched.is_empty(), "auto-match must fire when Science is unmanned");
    }

    #[test]
    fn auto_match_not_emitted_without_locked_target() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        app.world_mut().resource_mut::<WeaponsTarget>().0 = None;
        app.insert_resource(AutoMatchDelaySecs(0.0));

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(matched.is_empty(), "auto-match must not fire without a locked target");
    }

    #[test]
    fn auto_match_not_emitted_without_tactical_holder() {
        let mut app = test_app();
        // No Tactical holder.
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Low".into());
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("match-target".into());
        app.insert_resource(AutoMatchDelaySecs(0.0));
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(matched.is_empty(), "auto-match must not fire without a Tactical holder");
    }

    #[test]
    fn auto_match_timer_resets_when_tactical_flips_to_full() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        app.insert_resource(AutoMatchDelaySecs(100.0));
        // Nearly at delay
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 99.0,
            match_sent: false,
        };
        // Flip Tactical to Full mid-countdown
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());

        let (inbound, _) = tick(&mut app);
        let matched: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(matched.is_empty(), "pending match must be cancelled when Tactical flips to Full");
        // Timer must be reset
        let state = app.world().resource::<FrequencyMatchTimer>();
        assert!(state.0.current_target.is_none(), "timer state must reset when Tactical goes Full");
    }

    #[test]
    fn auto_match_no_auto_revert_after_trigger_ends() {
        let mut app = test_app();
        setup_auto_match_conditions(&mut app);
        app.insert_resource(AutoMatchDelaySecs(0.0));
        app.world_mut().resource_mut::<FrequencyMatchTimer>().0 = FrequencyMatchState {
            current_target: Some("match-target".into()),
            elapsed_secs: 5.0,
            match_sent: false,
        };

        // First tick — match fires
        let (inbound1, _) = tick(&mut app);
        let matched1: Vec<_> = inbound1.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(!matched1.is_empty(), "match should fire on first qualifying tick");

        // Now flip both consoles to Full — trigger ends
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Tactical, "Std".into());
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Sensors, "Std".into());

        // Second tick — no revert message
        let (inbound2, _) = tick(&mut app);
        let matched2: Vec<_> = inbound2.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::SetPhaserFrequency { .. }))
            .collect();
        assert!(matched2.is_empty(), "frequency must persist — no auto-revert when trigger ends");
    }

    // ── Power AI plugin tests ─────────────────────────────────────────────

    /// Set up the Power console occupied and at Low complexity with full battery.
    fn setup_power_low(app: &mut App) {
        push_inbound(app, "power", ClientMessage::Identify { token: "power".into(), name: "Alice".into() });
        app.update();
        push_inbound(app, "power", ClientMessage::SelectStation { station: "Power".into() });
        app.update();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Power, "Low".into());
        // Full battery
        app.world_mut().resource_mut::<ShipPowerSystem>().0.battery_charge = 100.0;
    }

    #[test]
    fn power_ai_does_not_run_when_console_unoccupied() {
        let mut app = test_app();
        app.world_mut().resource_mut::<CurrentPhase>().0 = GamePhase::InProgress;
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Power, "Low".into());
        // Thrust above threshold
        app.world_mut().resource_mut::<LastHelmInput>().thrust = 0.9;
        app.world_mut().resource_mut::<ShipPowerSystem>().0.battery_charge = 100.0;

        let (inbound, _) = tick(&mut app);
        let power_msgs: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::IncreasePower { .. }))
            .collect();
        assert!(power_msgs.is_empty(), "AI must not run when Power console is unoccupied");
    }

    #[test]
    fn power_ai_does_not_run_at_full_complexity() {
        let mut app = test_app();
        setup_power_low(&mut app);
        // Override to Full
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Power, "Std".into());
        app.world_mut().resource_mut::<LastHelmInput>().thrust = 0.9;
        // Pre-seed the movement state as if counting was underway
        app.world_mut().resource_mut::<PowerMovementEngageState>().0 =
            EngageState::Counting { elapsed_secs: 2.9 };

        let (inbound, _) = tick(&mut app);
        let power_msgs: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::IncreasePower { .. } | ClientMessage::DecreasePower { .. }))
            .collect();
        assert!(power_msgs.is_empty(), "AI must not generate power messages at Full complexity");
        // State should reset to Idle
        assert_eq!(
            app.world().resource::<PowerMovementEngageState>().0,
            EngageState::Idle,
            "engage state should reset to Idle when complexity is Full"
        );
    }

    #[test]
    fn power_ai_engages_helm_after_sustained_thrust() {
        let mut app = test_app();
        setup_power_low(&mut app);

        // Use zero delay so the rule engages in a single tick with any elapsed time.
        app.insert_resource(PowerAiConfig {
            thrust_threshold: 0.7,
            movement_engage_delay_secs: 0.0,
            battery_engage_min_pct_movement: 50.0,
            battery_engage_min_pct_red_alert: 10.0,
            red_alert_engage_delay_secs: 0.0,
            battery_recharge_pct: 100.0,
        });
        // Pre-seed elapsed time past the (zero) delay.
        app.world_mut().resource_mut::<PowerMovementEngageState>().0 =
            EngageState::Counting { elapsed_secs: 1.0 };
        // Thrust above threshold
        app.world_mut().resource_mut::<LastHelmInput>().thrust = 0.9;

        let (inbound, _) = tick(&mut app);
        let increase_helm: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::IncreasePower { console: Console::Helm }))
            .collect();
        assert!(!increase_helm.is_empty(), "AI should increase Helm power after sustained thrust");
    }

    #[test]
    fn power_ai_disengages_helm_when_battery_drops() {
        let mut app = test_app();
        setup_power_low(&mut app);
        // Pre-set movement state to Engaged
        app.world_mut().resource_mut::<PowerMovementEngageState>().0 = EngageState::Engaged;
        // Battery drops below minimum (50%)
        app.world_mut().resource_mut::<ShipPowerSystem>().0.battery_charge = 30.0;
        app.world_mut().resource_mut::<LastHelmInput>().thrust = 0.9;

        let (inbound, _) = tick(&mut app);
        let decrease_helm: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::DecreasePower { console: Console::Helm }))
            .collect();
        assert!(!decrease_helm.is_empty(), "AI should decrease Helm power when battery drops");
    }

    #[test]
    fn power_ai_engages_weapons_after_sustained_red_alert() {
        let mut app = test_app();
        setup_power_low(&mut app);
        app.insert_resource(PowerAiConfig {
            thrust_threshold: 0.7,
            movement_engage_delay_secs: 9999.0,
            battery_engage_min_pct_movement: 50.0,
            battery_engage_min_pct_red_alert: 10.0,
            red_alert_engage_delay_secs: 0.0,
            battery_recharge_pct: 100.0,
        });
        // Pre-seed elapsed time past the (zero) delay.
        app.world_mut().resource_mut::<PowerRedAlertEngageState>().0 =
            EngageState::Counting { elapsed_secs: 1.0 };
        // Set red alert
        app.world_mut().resource_mut::<ShipState>().toggle_red_alert();

        let (inbound, _) = tick(&mut app);
        let increase_weapons: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::IncreasePower { console: Console::Tactical }))
            .collect();
        assert!(!increase_weapons.is_empty(), "AI should increase Weapons power under red alert");
    }

    #[test]
    fn power_ai_disengages_weapons_when_battery_drops() {
        let mut app = test_app();
        setup_power_low(&mut app);
        app.world_mut().resource_mut::<PowerRedAlertEngageState>().0 = EngageState::Engaged;
        // Battery drops below min for red alert (10%)
        app.world_mut().resource_mut::<ShipPowerSystem>().0.battery_charge = 5.0;
        app.world_mut().resource_mut::<ShipState>().toggle_red_alert();

        let (inbound, _) = tick(&mut app);
        let decrease_weapons: Vec<_> = inbound.iter()
            .filter(|m| matches!(&m.msg, ClientMessage::DecreasePower { console: Console::Tactical }))
            .collect();
        assert!(!decrease_weapons.is_empty(), "AI should decrease Weapons power when battery drops");
    }

    #[test]
    fn power_ai_both_rules_can_engage_simultaneously() {
        let mut app = test_app();
        setup_power_low(&mut app);
        app.insert_resource(PowerAiConfig {
            thrust_threshold: 0.7,
            movement_engage_delay_secs: 0.0,
            battery_engage_min_pct_movement: 50.0,
            battery_engage_min_pct_red_alert: 10.0,
            red_alert_engage_delay_secs: 0.0,
            battery_recharge_pct: 100.0,
        });
        // Both at Counting with elapsed past delay
        app.world_mut().resource_mut::<PowerMovementEngageState>().0 =
            EngageState::Counting { elapsed_secs: 1.0 };
        app.world_mut().resource_mut::<PowerRedAlertEngageState>().0 =
            EngageState::Counting { elapsed_secs: 1.0 };
        app.world_mut().resource_mut::<LastHelmInput>().thrust = 0.9;
        app.world_mut().resource_mut::<ShipState>().toggle_red_alert();

        let (inbound, _) = tick(&mut app);
        let increase_helm = inbound.iter()
            .any(|m| matches!(&m.msg, ClientMessage::IncreasePower { console: Console::Helm }));
        let increase_weapons = inbound.iter()
            .any(|m| matches!(&m.msg, ClientMessage::IncreasePower { console: Console::Tactical }));
        assert!(increase_helm, "movement rule should engage Helm");
        assert!(increase_weapons, "red alert rule should engage Weapons");
    }

    #[test]
    fn switching_power_to_full_cancels_pending_engage_and_disengages_if_engaged() {
        let mut app = test_app();
        setup_power_low(&mut app);
        // One rule counting, one engaged
        app.world_mut().resource_mut::<PowerMovementEngageState>().0 =
            EngageState::Counting { elapsed_secs: 2.9 };
        app.world_mut().resource_mut::<PowerRedAlertEngageState>().0 = EngageState::Engaged;
        // Switch to Full
        app.world_mut().resource_mut::<ConsoleComplexityState>()
            .set(Console::Power, "Std".into());
        app.world_mut().resource_mut::<ShipState>().toggle_red_alert();

        let (inbound, _) = tick(&mut app);

        // Movement rule: counting → should reset (no increase)
        let increase_helm = inbound.iter()
            .any(|m| matches!(&m.msg, ClientMessage::IncreasePower { console: Console::Helm }));
        assert!(!increase_helm, "pending movement rule must not fire when switching to Full");

        // Red alert rule was Engaged → implicit disengage expected
        let decrease_weapons = inbound.iter()
            .any(|m| matches!(&m.msg, ClientMessage::DecreasePower { console: Console::Tactical }));
        assert!(decrease_weapons, "engaged red alert rule must disengage when switching to Full");

        // Both states should be Idle now
        assert_eq!(app.world().resource::<PowerMovementEngageState>().0, EngageState::Idle);
        assert_eq!(app.world().resource::<PowerRedAlertEngageState>().0, EngageState::Idle);
    }
}

/// Bevy orchestrator for server-side console AI.
///
/// This plugin runs per-tick AI decision functions from `console_ai` and
/// synthesises the same `InboundMessage` types that a human player would
/// produce. AI only runs on **occupied** consoles whose complexity preset is
/// currently "Low".
///
/// If the holder switches from "Low" to "Full" (or back), the complexity
/// state is updated and AI immediately stops generating actions.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::console_ai::{auto_fire_torpedo, tick_frequency_hint, FrequencyHintInput, FrequencyHintState, TorpedoAiInput, TubeSummary};
use crate::lobby::{CurrentPhase, InboundMessage, OutboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, GamePhase, ServerMessage, TorpedoTube as MsgTorpedoTube};
use crate::ship_state::ShipState;
use crate::simulation::{TorpedoSystemResource, WeaponsTarget};
use crate::torpedo::TorpedoTubeId;

// ── Constants ──────────────────────────────────────────────────────────────

const LOW_PRESET: &str = "Low";

/// Default delay (seconds) before the frequency hint fires when Science is Low.
const DEFAULT_AUTO_HINT_DELAY_SECS: f32 = 3.0;

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
            .add_systems(Update, (
                track_complexity_changes,
                run_tactical_ai.after(track_complexity_changes),
                run_science_hint_ai.after(track_complexity_changes),
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
    // and Science is Low (readout is hidden).
    if !complexity.is_low(&Console::Science) || complexity.is_low(&Console::Tactical) {
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
        ActiveBeam, PhaserCooldown, CurrentPhaserMode,
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
            .set(Console::Tactical, "Full".into());
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
            preset_name: "Full".into(),
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
            .set(Console::Tactical, "Full".into());

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
            .set(Console::Science, "Low".into());
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
            .set(Console::Science, "Full".into());
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
            .set(Console::Science, "Low".into());
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
}

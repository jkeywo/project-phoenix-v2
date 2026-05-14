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

use crate::console_ai::{auto_fire_torpedo, TorpedoAiInput, TubeSummary};
use crate::lobby::{CurrentPhase, InboundMessage, OutboundMessage, Sessions};
use crate::messages::{ClientMessage, Console, GamePhase, ServerMessage, TorpedoTube as MsgTorpedoTube};
use crate::ship_state::ShipState;
use crate::simulation::{TorpedoSystemResource, WeaponsTarget};
use crate::torpedo::TorpedoTubeId;

// ── Constants ──────────────────────────────────────────────────────────────

const LOW_PRESET: &str = "Low";

// ── Resources ──────────────────────────────────────────────────────────────

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
            .add_systems(Update, (
                track_complexity_changes,
                run_tactical_ai.after(track_complexity_changes),
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
}

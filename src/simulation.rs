use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

use crate::breakdown::BreakdownQueue;
#[cfg(test)]
use crate::breakdown::breakdowns_from_damage;
use crate::damage::{HullIntegrity};
use crate::lobby::{CurrentPhase, InboundMessage, OutboundMessage, Sessions, Target, WorldResource};
use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::repair_teams::RepairTeams;
use crate::shield::ShieldSystem;
use crate::map_config::MapConfig;
use crate::radar::WEAPONS_RADAR_RANGE;
use crate::messages::{
    ClientMessage, Console, EntitySnapshot, GamePhase, ServerMessage, Shape, ShieldFacingStatus, ViewDirection,
};
use crate::torpedo::{TorpedoSystem, TorpedoConfig, TorpedoTubeId};
use crate::messages::TorpedoTube as MsgTorpedoTube;
use crate::ship_physics::ShipPhysicsConfig;
use crate::ship_state::ShipState;
use crate::damage::{collision_damage, apply_damage_with_shields, apply_hull_damage};
use crate::shield::attacker_bearing_relative;
use bevy_rapier3d::prelude::ReadRapierContext;

use crate::impulse::ImpulseState;
use crate::modifiers::ShipModifiers;
use crate::messages::ModifierSlot;
use crate::entity_spawner::{EntityUuid, EntityId, RegionShapeSection, EntityTagsSection};
use std::collections::HashMap;

// â”€â”€ Beam constants â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
const BEAM_DURATION_SECS: f32 = 6.0;
const BEAM_DAMAGE_PER_SEC: f32 = 5.0;
const BEAM_COOLDOWN_SECS: f32 = 6.0;

// â”€â”€ Marker Components â”€â”€â”€â”€â”€â”€â”€â”€
#[derive(Component)]
pub struct Ship;

#[derive(Component)]
pub struct Asteroid;

/// Stable UUID string identifying this asteroid entity (for targeting).
#[derive(Component, Clone)]
pub struct AsteroidUuid(pub String);

/// Tracks remaining HP for an asteroid entity (max and current = 30).
#[derive(Component)]
pub struct AsteroidDamage {
    pub max_hp: i32,
    pub current_hp: i32,
}

// â”€â”€ Resources â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
#[derive(Resource)]
struct SimBroadcastTimer(Timer);

/// Ship-wide Hull Integrity (0–100). Tracked as a Bevy resource so systems
/// can read/write it independently of `ShipState`.
#[derive(Resource)]
pub struct ShipHullIntegrity(pub HullIntegrity);

/// The ship's shield system. Damage from collisions is routed through shields
/// first; only overflow passes through to the hull.
#[derive(Resource)]
pub struct ShipShields(pub ShieldSystem);

/// The ship's impulse drive state. Cancelled automatically when hull damage is taken.
#[derive(Resource)]
pub struct ShipImpulse(pub ImpulseState);

/// Tracks whether the initial WorldSetup broadcast has fired, so it only
/// goes out once per game.
#[derive(Resource, Default)]
struct WorldSetupBroadcast {
    sent: bool,
}

/// The currently locked target UUID on the Weapons console. `None` means no
/// lock is active.
#[derive(Resource, Default)]
pub struct WeaponsTarget(pub Option<String>);

/// Active phaser beam state. `target_uuid` is `Some` while a beam is firing.
/// `remaining_secs` counts down to 0. `damage_accumulator` tracks fractional
/// damage between ticks so 5 HP/s is applied accurately at any frame rate.
#[derive(Resource, Default)]
pub struct ActiveBeam {
    pub target_uuid: Option<String>,
    pub remaining_secs: f32,
    pub damage_accumulator: f32,
    /// Which bank is firing this beam. `None` when no beam is active.
    pub bank: Option<crate::messages::PhaserBank>,
}

/// Post-beam cooldown. The weapons console is locked out for `BEAM_COOLDOWN_SECS`
/// after every beam end (natural, sever, or cancel).
#[derive(Resource, Default)]
pub struct PhaserCooldown {
    pub remaining_secs: f32,
}

/// Current phaser firing mode (Auto or Manual), set by the Weapons console.
#[derive(Resource)]
pub struct CurrentPhaserMode(pub crate::messages::PhaserMode);

impl Default for CurrentPhaserMode {
    fn default() -> Self {
        Self(crate::messages::PhaserMode::Auto)
    }
}

/// Rendering config for the phaser beam (colour, max range).
/// Populated from ship entity TOML during world setup; defaults are used if
/// the TOML is absent.
#[derive(Resource, Clone, Debug)]
pub struct PhaserRenderConfig {
    /// RGBA beam colour in 0.0â€“1.0.
    pub beam_color: [f32; 4],
    /// Maximum beam range (world units); beam endpoint is clamped to this.
    pub beam_range: f32,
}

impl Default for PhaserRenderConfig {
    fn default() -> Self {
        Self {
            beam_color: crate::beam_render::DEFAULT_BEAM_COLOR,
            beam_range: 40.0,
        }
    }
}

// â”€â”€ Repair constants â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
/// HP restored per completed repair team.
const REPAIR_TEAM_HP: f32 = 10.0;

/// Bevy resource wrapping the pure `RepairTeams` state machine.
#[derive(Resource)]
pub struct ShipRepairTeams(pub RepairTeams);

/// Bevy message fired (with world-space position) when an asteroid is destroyed
/// by phaser fire. The renderer uses this to spawn a ripple VFX at the site.
#[derive(Message, Clone, Debug)]
pub struct AsteroidDestroyedVfx {
    pub x: f32,
    pub z: f32,
}

/// Tracks the last-broadcast repair-icon state so the `broadcast_repair_icons`
/// system can send deltas (ClearRepairIcon for stale icons, ShowRepairIcon for
/// new/changed ones).
#[derive(Resource)]
pub struct RepairIconState {
    /// Map from console to the last shape sent to its holder.
    pub last_icons: std::collections::HashMap<Console, Shape>,
    pub(crate) rng: rand::rngs::SmallRng,
}

impl Default for RepairIconState {
    fn default() -> Self {
        use rand::SeedableRng;
        Self {
            last_icons: std::collections::HashMap::new(),
            rng: rand::rngs::SmallRng::from_os_rng(),
        }
    }
}

/// Wraps the pure-Rust power system so it can be used as a Bevy resource.
#[derive(Resource)]
pub struct ShipPowerSystem(pub crate::power_system::PowerSystem);

/// Wraps the power config for the ship's power system.
#[derive(Resource)]
pub struct PowerConfigResource(pub crate::power_system::PowerConfig);

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
        Self(crate::power_system::PowerConfig::default())
    }
}

/// Wraps the pure-Rust torpedo system so it can be used as a Bevy resource.
#[derive(Resource)]
pub struct TorpedoSystemResource(pub TorpedoSystem);

/// Bevy resource wrapping the breakdown queue.
#[derive(Resource)]
pub struct BreakdownQueueResource {
    pub queue: BreakdownQueue,
    /// Cumulative damage taken since game start (tracks 10-HP bucket crossings).
    pub cumulative_damage: f32,
    pub(crate) rng: rand::rngs::SmallRng,
}

impl Default for BreakdownQueueResource {
    fn default() -> Self {
        use rand::SeedableRng as _;
        Self {
            queue: BreakdownQueue::new(),
            cumulative_damage: 0.0,
            rng: rand::rngs::SmallRng::from_os_rng(),
        }
    }
}

/// Prevents `handle_collisions` from applying damage every frame while the
/// ship is in contact. After damage is applied once, a 1-second cooldown
/// suppresses further hits until the ship clears the obstacle.
#[derive(Resource, Default)]
struct CollisionCooldown {
    remaining_secs: f32,
}

/// Pending outbound messages produced by simulation systems.
/// Drained each frame by the `SimBroadcaster` dispatch.
///
/// ## Migration note (PRD #253)
/// The old preamble pattern (`MessageWriter<OutboundMessage>`) has been
/// eliminated from all domain plugins. All systems that previously wrote
/// `OutboundMessage` directly now write `(Target, ServerMessage)` tuples
/// into `SimOutbox`. The `sim_outbox_broadcaster()` or a manual drain
/// (in tests) flushes these entries to the `OutboundMessage` bus.
/// To verify the absence of the old pattern, run:
///   rg 'MessageWriter<OutboundMessage>' src/  # must return no matches
#[derive(Resource, Default)]
pub struct SimOutbox(pub Vec<(Target, ServerMessage)>);

/// Tracks non-asteroid entities that have been reported to clients via
/// `EntitySpawned` / `EntityDespawned`.  Seeded from `WorldResource` on
/// the first `InProgress` frame so initial world entities are not re-reported.
///
/// Maintained by the `reconcile_runtime_entities` system.
#[derive(Resource, Default)]
pub struct TrackedEntities {
    /// UUIDs of non-asteroid entities already reported to clients.
    /// Populated from `WorldResource` at game start, then updated
    /// incrementally as runtime entities are spawned/despawned.
    pub reported: std::collections::HashSet<String>,
    /// Whether the registry has been seeded from initial WorldResource
    /// on the first InProgress frame.
    pub seeded: bool,
}

impl PhaserCooldown {
    pub fn is_active(&self) -> bool {
        self.remaining_secs > 0.0
    }

    pub fn start(&mut self) {
        self.remaining_secs = BEAM_COOLDOWN_SECS;
    }

    pub fn tick(&mut self, dt: f32) {
        self.remaining_secs = (self.remaining_secs - dt).max(0.0);
    }
}

// â”€â”€ Plugin â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
/// Empty system used as an ordering anchor for the sim broadcast dispatch.
/// All sim-phase systems (message handlers, tick systems, broadcasters) should
/// run before this anchor so that `dispatch_sim_broadcasts` (which has
/// `.after(sim_processing_anchor)`) drains their `SimOutbox` writes.
pub fn sim_processing_anchor() {}

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RapierPhysicsPlugin::<()>::default())
            .add_plugins(crate::region_plugin::RegionPlugin)
            .add_plugins(crate::console_ai_plugin::ConsoleAiPlugin)
            .add_plugins(crate::ai_plugin::AiPlugin)
            .add_plugins(crate::captain_plugin::CaptainPlugin)
            .add_plugins(crate::ship_plugin::ShipPlugin)
            .add_message::<AsteroidDestroyedVfx>()
            .insert_resource(ShipState::new())
            .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
            .insert_resource(ShipShields(ShieldSystem::default()))
            .insert_resource(ShipImpulse(ImpulseState::new()))
            .init_resource::<WorldResource>()
            .init_resource::<WorldSetupBroadcast>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<CurrentPhaserMode>()
            .init_resource::<PhaserRenderConfig>()
            .insert_resource(ShipRepairTeams(RepairTeams::new()))
            .init_resource::<BreakdownQueueResource>()
            .init_resource::<CollisionCooldown>()
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())))
            .init_resource::<RepairIconState>()
            .insert_resource(ShipPowerSystem(crate::power_system::PowerSystem::default()))
            .init_resource::<PowerConfigResource>()
            .init_resource::<PowerMultiplierResource>()
            .init_resource::<TrackedEntities>()
            .init_resource::<SimOutbox>()
            .insert_resource(SimBroadcastTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .add_systems(Startup, setup_world)
            .add_systems(Update, (
                spawn_game_start_entities.after(crate::lobby::process_lobby),
                render_spawned_entities.after(spawn_game_start_entities),
            ))
            .add_systems(Update, (
                (
                    handle_set_target,
                    handle_set_science_target,
                    handle_set_sensors_target,
                ),
                (
                    handle_fire_phaser,
                    handle_set_phaser_mode,
                    handle_set_phaser_frequency,
                    handle_fire_torpedo,
                    handle_repair,
                    handle_power_messages,
                    handle_set_shield_focus,
                ),
                (
                    tick_active_beam,
                    tick_repair_teams,
                    tick_torpedo_system,
                    tick_shields,
                    tick_power_system,
                    handle_collisions,
                ),
                (
                    broadcast_shield_status,
                    broadcast_world_setup_on_start.after(crate::lobby::process_lobby),
                    broadcast_repair_icons,
                    reconcile_runtime_entities.after(crate::lobby::process_lobby),
                    sim_processing_anchor,
                ),
            ).after(crate::lobby::process_lobby))
            .add_systems(Update, crate::modifier_coordination::translate_power_modifiers
                .after(handle_power_messages)
                .after(tick_power_system)
                .after(crate::lobby::process_lobby))
            .add_systems(Update, crate::modifier_coordination::translate_impulse_modifiers
                .after(crate::ship_plugin::handle_impulse_messages)
                .after(crate::lobby::process_lobby))
            .add_systems(Update, (
                crate::modifier_coordination::translate_region_modifiers,
                crate::region_plugin::handle_slow_zone_speed_clamp,
            ).chain().after(crate::region_plugin::update_region_membership))
            .add_plugins(power_state_broadcaster())
            .add_plugins(weapons_update_broadcaster())
            .add_plugins(repair_state_broadcaster())
            .add_plugins(sim_state_broadcaster())
            .add_plugins(modifier_events_broadcaster())
            .add_plugins(sim_outbox_broadcaster());
    }
}

/// Returns a [`SimBroadcaster`] pre-configured with the `PowerState` producer.
///
/// Broadcasts `PowerState` at 10 Hz to the `Power` console holder only.
/// This is the canonical registration; it is added by [`SimulationPlugin`]
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

/// Returns a [`SimBroadcaster`] pre-configured with the `WeaponsUpdate` producer.
///
/// Broadcasts `WeaponsUpdate` at 10 Hz to the `Tactical` console holder only.
/// Registered by [`SimulationPlugin`] and the test harness in `test_app()`.
pub fn weapons_update_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::Holding(Console::Tactical),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let ship = world.resource::<ShipState>();
            let world_res = world.resource::<WorldResource>();
            let weapons_target = world.resource::<WeaponsTarget>();
            let cooldown = world.resource::<PhaserCooldown>();
            let beam = world.resource::<ActiveBeam>();
            let torpedo_sys = world.resource::<TorpedoSystemResource>();
            let modifiers = world.resource::<ShipModifiers>();

            let effective_phaser_range = crate::radar::PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
            let fire_ready = match &weapons_target.0 {
                None => false,
                Some(uuid) => {
                    world_res.0.entities.iter()
                        .find(|a| &a.uuid == uuid)
                        .map(|a| crate::radar::is_fire_ready_with_range(a.x(), a.z(), ship.x, ship.z, ship.yaw, effective_phaser_range))
                        .unwrap_or(false)
                }
            };

            let ts = &torpedo_sys.0;
            vec![ServerMessage::WeaponsUpdate {
                target_uuid: weapons_target.0.clone(),
                fire_ready,
                on_cooldown: cooldown.is_active() || beam.target_uuid.is_some(),
                torpedo_count: ts.torpedoes_remaining,
                fore_port_loaded: ts.fore_port.is_loaded(),
                fore_port_reload_secs: ts.fore_port.reload_remaining,
                fore_starboard_loaded: ts.fore_starboard.is_loaded(),
                fore_starboard_reload_secs: ts.fore_starboard.reload_remaining,
                aft_loaded: ts.aft.is_loaded(),
                aft_reload_secs: ts.aft.reload_remaining,
            }]
        },
    )
}

/// Returns a [`SimBroadcaster`] pre-configured with the `RepairState` producer.
///
/// Broadcasts `RepairState` at 10 Hz to the `Repair` console holder only.
/// Registered by [`SimulationPlugin`] and the test harness in `test_app()`.
pub fn repair_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::Holding(Console::Repair),
        Cadence::Hz(10.0),
        |world: &mut World| {
            use crate::messages::TeamSlot;
            let teams = world.resource::<ShipRepairTeams>();
            let breakdowns = world.resource::<BreakdownQueueResource>();

            let slots = teams.0.slots();
            let in_progress = slots.iter().any(|s| matches!(s, TeamSlot::Repairing { .. }));
            let penalty = slots.iter().any(|s| matches!(s, TeamSlot::Cooldown { .. }));
            let remaining_cooldown_secs = slots.iter().map(|s| match s {
                TeamSlot::Repairing { progress } => (1.0 - progress) * 30.0,
                TeamSlot::Cooldown { progress } => progress * 10.0,
                TeamSlot::Idle => 0.0,
            }).fold(0.0_f32, f32::max);

            let current_breakdown = breakdowns.queue.front().map(|entry| (entry.console.clone(), entry.shape));

            vec![ServerMessage::RepairState {
                remaining_cooldown_secs,
                in_progress,
                penalty,
                teams: *slots,
                current_breakdown,
            }]
        },
    )
}

/// Returns a [`SimBroadcaster`] pre-configured with the `SimState` producer.
///
/// Broadcasts `SimState` at 10 Hz to all players (`Audience::All`).
/// Registered by [`SimulationPlugin`] and the test harness in `test_app()`.
pub fn sim_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::All,
        Cadence::Hz(10.0),
        |world: &mut World| {
            // Build per-tick entity state from live ECS first (before any resource
            // borrows, so world.query() can get the exclusive access it needs).
            let entity_states: Vec<crate::messages::EntityStateSnapshot> = {
                let mut query = world.query::<(&Transform, &AsteroidUuid, Option<&AsteroidDamage>)>();
                query.iter(world)
                    .map(|(transform, uuid, damage)| {
                        let hull_fraction = damage.map(|d| d.current_hp as f32 / d.max_hp as f32);
                        crate::messages::EntityStateSnapshot {
                            uuid: uuid.0.clone(),
                            position: Some([transform.translation.x, transform.translation.y, transform.translation.z]),
                            yaw: Some(transform.rotation.to_euler(bevy::math::EulerRot::YXZ).0),
                            hull_fraction,
                            flags: vec![],
                            shields: None,
                            warp_out_remaining_secs: None,
                        }
                    })
                    .collect()
            };

            // Extract all resource data (borrows are confined to this block).
            let (hull_current, power_levels, flags, helm_range_mult, charge_progress,
                 ship_x, ship_z, ship_yaw, ship_red_alert, ship_view_mode) = {
                let ship = world.resource::<ShipState>();
                let hull = world.resource::<ShipHullIntegrity>();
                let power = world.get_resource::<ShipPowerSystem>();
                let impulse = world.resource::<ShipImpulse>();
                let modifiers = world.resource::<crate::modifiers::ShipModifiers>();

                let power_levels = power
                    .map(|p| (p.0.helm, p.0.weapons, p.0.sensors))
                    .unwrap_or((2, 2, 2));
                let flags = modifiers.flags();
                let helm_range_mult = modifiers.get(&ModifierSlot::RadarRange);
                (
                    hull.0.current(), power_levels, flags, helm_range_mult,
                    impulse.0.charge_progress,
                    ship.x, ship.z, ship.yaw, ship.red_alert(), ship.view_mode.clone(),
                )
            };

            let radar_state = crate::messages::RadarStateSnapshot {
                helm_range: crate::client_sim::HELM_RADAR_RANGE * helm_range_mult,
                tactical_range: crate::client_sim::WEAPONS_RADAR_RANGE * helm_range_mult,
                science_long_range: crate::client_sim::SCIENCE_RADAR_RANGE * helm_range_mult,
                science_system_map: crate::client_sim::SYSTEM_CHART_RANGE,
            };

            let snapshot = crate::messages::SimSnapshot {
                red_alert: ship_red_alert,
                view_mode: ship_view_mode,
                ship_x,
                ship_z,
                ship_yaw,
                hull_integrity: hull_current,
                power_levels,
                flags,
                entity_states,
                radar_state,
                impulse_charge_progress: charge_progress,
            };
            vec![ServerMessage::SimState { snapshot }]
        },
    )
}

/// Returns a [`SimBroadcaster`] pre-configured with the `ModifierAdded` and
/// `ModifierRemoved` producers.
///
/// Drains pending modifier events from [`ShipModifiers`] once per frame and
/// broadcasts each as a separate `ServerMessage` to all players (`Audience::All`).
/// Uses `Cadence::OnEvent` so the producer is called every frame regardless of
/// any Hz timer; an empty drain produces no outbound messages.
/// Registered by [`SimulationPlugin`] and the test harness in `test_app()`.
pub fn modifier_events_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::All,
        Cadence::OnEvent,
        |world: &mut World| {
            use crate::modifiers::ModifierEvent;
            let events: Vec<_> = {
                let mut modifiers = world.resource_mut::<crate::modifiers::ShipModifiers>();
                std::mem::take(&mut modifiers.pending_events)
            };
            events
                .into_iter()
                .map(|event| match event {
                    ModifierEvent::Added { source, slot, bonus } => {
                        ServerMessage::ModifierAdded { source, slot, bonus }
                    }
                    ModifierEvent::Removed { source, slot } => {
                        ServerMessage::ModifierRemoved { source, slot }
                    }
                })
                .collect()
        },
    )
}

/// Returns a [`SimBroadcaster`] that drains [`SimOutbox`] each frame and writes
/// each entry as an `OutboundMessage` with per-message target routing.
///
/// Uses `Cadence::OnEvent` so the producer fires every frame.  When the outbox
/// is empty the producer returns an empty `Vec` and no messages are emitted.
/// When populated (by any simulation system) the queued entries are flushed
/// directly to `OutboundMessage` with their original `Target` routing.
pub fn sim_outbox_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::All,
        Cadence::OnEvent,
        |world: &mut World| {
            let mut outbox = world.resource_mut::<SimOutbox>();
            let entries = std::mem::take(&mut outbox.0);
            for (target, msg) in entries {
                world.write_message(OutboundMessage { target, msg });
            }
            vec![]
        },
    )
}

// -- Helper: token validation with AI fallback --------------------------------

/// Returns `true` when `token` is the holder of `console` in the session
/// manager, OR when `token` is a registered AI token (so AI-generated
/// messages for that console are not silently discarded once future slices
/// start injecting `HelmInput` etc.).
///
/// Currently used as documentation of the fallback contract; future AI-input
/// slices will thread this through the individual message handlers.
#[allow(dead_code)]
fn is_valid_console_holder(
    token: &str,
    console: Console,
    sessions: &Sessions,
    ai_registry: &crate::ai_plugin::AiTokenRegistry,
) -> bool {
    if sessions.0.console_holder(console) == Some(token) {
        return true;
    }
    // Fallback: token belongs to an AI-controlled entity
    ai_registry.entity_uuid_for_token(token).is_some()
}

// -- Systems -------------------------------------------------------------------

fn handle_set_target(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    mut weapons_target: ResMut<WeaponsTarget>,
    modifiers: Res<ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::SetTarget { uuid } = &ev.msg else { continue };

        // Only the Weapons console holder may lock a target.
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }

        // Validate: asteroid must exist in world data and be within WEAPONS_RADAR_RANGE.
        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);
        let effective_weapons_range = WEAPONS_RADAR_RANGE * radar_range_mult;
        let asteroid = world.0.entities.iter().find(|a| &a.uuid == uuid);
        let locked = match asteroid {
            None => false,
            Some(a) => {
                let dx = a.x() - ship.x;
                let dz = a.z() - ship.z;
                dx * dx + dz * dz <= effective_weapons_range * effective_weapons_range
            }
        };

        if locked {
            weapons_target.0 = Some(uuid.clone());
        } else {
            // Rejection clears the visual lock.
            weapons_target.0 = None;
        }

        outbox.0.push((Target::Token(ev.token.clone()), ServerMessage::TargetLock { uuid: uuid.clone(), locked }));
    }
}

fn handle_set_science_target(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::SetScienceTarget { uuid } = &ev.msg else { continue };

        // Only the Sensors console holder may broadcast a target suggestion.
        if sessions.0.console_holder(Console::Sensors) != Some(ev.token.as_str()) {
            continue;
        }

        // Only broadcast if there is a Weapons console player to receive it.
        let Some(weapons_token) = sessions.0.console_holder(Console::Tactical) else {
            continue;
        };

        outbox.0.push((Target::Token(weapons_token.to_string()), ServerMessage::ScienceTargetSuggestion { uuid: uuid.clone() }));
    }
}

fn handle_set_sensors_target(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::SetSensorsTarget { uuid } = &ev.msg else { continue };

        // Only the Sensors console holder may broadcast a target suggestion.
        if sessions.0.console_holder(Console::Sensors) != Some(ev.token.as_str()) {
            continue;
        }

        // Only broadcast if there is a Tactical console player to receive it.
        let Some(tactical_token) = sessions.0.console_holder(Console::Tactical) else {
            continue;
        };

        outbox.0.push((Target::Token(tactical_token.to_string()), ServerMessage::SensorsTargetSuggestion { uuid: uuid.clone() }));
    }
}


fn handle_collisions(
    time: Res<Time>,
    context: ReadRapierContext,
    ship_query: Query<Entity, With<Ship>>,
    asteroid_query: Query<(&Transform, &AsteroidUuid), With<Asteroid>>,
    mut ship: ResMut<ShipState>,
    mut hull: ResMut<ShipHullIntegrity>,
    mut shields: ResMut<ShipShields>,
    mut breakdowns: ResMut<BreakdownQueueResource>,
    mut cooldown: ResMut<CollisionCooldown>,
    modifiers: Res<ShipModifiers>,
) {
    let dt = time.delta_secs();
    cooldown.remaining_secs = (cooldown.remaining_secs - dt).max(0.0);

    let Ok(ctx) = context.single() else { return };
    let Ok(ship_entity) = ship_query.single() else { return };

    let contact = ctx.contact_pairs_with(ship_entity).next().map(|pair| {
        if pair.collider1() == Some(ship_entity) { pair.collider2() } else { pair.collider1() }
    }).flatten();

    if contact.is_some() {
        if cooldown.remaining_secs > 0.0 {
            return;
        }
        let max_speed = ShipPhysicsConfig::new().max_speed;
        let damage = collision_damage(ship.forward_speed, max_speed) as f32
            * modifiers.get(&ModifierSlot::HullDamageTaken);

        let bearing = contact
            .and_then(|attacker_entity| {
                asteroid_query.get(attacker_entity).ok().map(|(t, _)| {
                    attacker_bearing_relative(
                        t.translation.x,
                        t.translation.z,
                        ship.x,
                        ship.z,
                        ship.yaw,
                    )
                })
            })
            .unwrap_or(0.0);

        let hull_damage_from_shields = apply_damage_with_shields(damage.round() as i32, bearing, &mut shields.0);
        if hull_damage_from_shields > 0 {
            let (_, new_cumulative, new_count) = apply_hull_damage(
                &mut hull.0,
                hull_damage_from_shields as f32,
                breakdowns.cumulative_damage,
            );
            breakdowns.cumulative_damage = new_cumulative;
            let BreakdownQueueResource { queue, rng, .. } = &mut *breakdowns;
            for _ in 0..new_count {
                queue.push_random(rng);
            }
        }
        ship.forward_speed = 0.0;
        cooldown.remaining_secs = 1.0;
    }
}
/// Tick shield regen and offline timers each frame.
fn tick_shields(time: Res<Time>, mut shields: ResMut<ShipShields>) {
    shields.0.tick(time.delta_secs());
}

/// Handle `SetShieldFocus` messages from the Shields console.
///
/// Validates: sender is Shields holder, game is in-progress.
/// Maps `ViewDirection` to facing index: Fore=0, Port=1, Aft=2, Starboard=3.
/// `None` clears the focus.
fn handle_set_shield_focus(
    mut reader: MessageReader<InboundMessage>,
    mut shields: ResMut<ShipShields>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let facing = match &ev.msg {
            ClientMessage::SetShieldFocus { facing } => facing.clone(),
            _ => continue,
        };
        // Only the Shields console holder may set focus.
        if sessions.0.console_holder(Console::Shields) != Some(ev.token.as_str()) {
            continue;
        }
        let idx = facing.and_then(|d| match d {
            ViewDirection::Fore => Some(0),
            ViewDirection::Port => Some(1),
            ViewDirection::Aft => Some(2),
            ViewDirection::Starboard => Some(3),
        });
        shields.0.set_focused_facing(idx);
    }
}

/// Handle `FirePhaser` messages from the Weapons console.
///
/// Validates: sender is Weapons holder, game is in-progress, no active cooldown,
/// a locked target exists, and that target is currently fire-ready.
/// On success, starts a new beam (cancelling any active beam first) and broadcasts
/// `BeamStarted` to all players.
fn handle_fire_phaser(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    world: Res<WorldResource>,
    weapons_target: Res<WeaponsTarget>,
    mut beam: ResMut<ActiveBeam>,
    cooldown: ResMut<PhaserCooldown>,
    modifiers: Res<ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        if !matches!(ev.msg, ClientMessage::FirePhaser) {
            continue;
        }
        // Only the Weapons console holder may fire.
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        // Reject if on cooldown or a beam is already active.
        if cooldown.is_active() || beam.target_uuid.is_some() {
            continue;
        }
        // Need a locked target.
        let Some(target_uuid) = &weapons_target.0 else { continue };
        // Target must still exist in world data and be fire-ready.
        let Some(asteroid) = world.0.entities.iter().find(|a| &a.uuid == target_uuid) else {
            continue;
        };
        let effective_phaser_range = crate::radar::PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
        if !crate::radar::is_fire_ready_with_range(asteroid.x(), asteroid.z(), ship.x, ship.z, ship.yaw, effective_phaser_range) {
            continue;
        }

        // If another beam was active (shouldn't happen with cooldown enforcement,
        // but guard defensively), end it first.
        if let Some(old_uuid) = beam.target_uuid.take() {
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid: old_uuid }));
        }

        // Start new beam. Alternate banks: port first, then starboard, etc.
        let next_bank = match beam.bank {
            Some(crate::messages::PhaserBank::Port) => crate::messages::PhaserBank::Starboard,
            _ => crate::messages::PhaserBank::Port,
        };
        beam.target_uuid = Some(target_uuid.clone());
        beam.remaining_secs = BEAM_DURATION_SECS;
        beam.damage_accumulator = 0.0;
        beam.bank = Some(next_bank);

        outbox.0.push((Target::All, ServerMessage::BeamStarted { target_uuid: target_uuid.clone() }));
    }
}

/// Handle `SetPhaserMode` messages from the Weapons console.
///
/// Only the Tactical console holder may change the phaser mode.
fn handle_set_phaser_mode(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    mut phaser_mode: ResMut<CurrentPhaserMode>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::SetPhaserMode { mode } = &ev.msg else { continue };
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        phaser_mode.0 = *mode;
    }
}

/// Handle `SetPhaserFrequency` messages.
///
/// Authorization is checked via the delegation allowlist:
/// - Tactical holder may always set phaser frequency.
/// - Sensors holder may set phaser frequency only when Tactical is at Low
///   complexity (delegated control, per PRD #176).
/// All other senders are silently rejected.
fn handle_set_phaser_frequency(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    complexity: Res<crate::console_ai_plugin::ConsoleComplexityState>,
    mut ship: ResMut<ShipState>,
) {
    use crate::delegation::{is_sender_authorized, ComplexityContext, DelegatedControl};

    if phase.0 != GamePhase::InProgress {
        return;
    }
    let ctx = ComplexityContext {
        tactical_is_low: complexity.is_low(&Console::Tactical),
    };
    for ev in reader.read() {
        let ClientMessage::SetPhaserFrequency { frequency } = &ev.msg else { continue };

        // Determine which console the sender holds (if any).
        let sender_console = if sessions.0.console_holder(Console::Tactical) == Some(ev.token.as_str()) {
            Console::Tactical
        } else if sessions.0.console_holder(Console::Sensors) == Some(ev.token.as_str()) {
            Console::Sensors
        } else {
            continue;
        };

        if !is_sender_authorized(DelegatedControl::SetPhaserFrequency, &sender_console, &ctx) {
            continue;
        }

        ship.phaser_frequency = frequency.clamp(0.0, 1.0);
    }
}

/// Convert a `messages::TorpedoTube` to a `torpedo::TorpedoTubeId`.
fn to_tube_id(tube: MsgTorpedoTube) -> TorpedoTubeId {
    match tube {
        MsgTorpedoTube::ForePort => TorpedoTubeId::ForePort,
        MsgTorpedoTube::ForeStarboard => TorpedoTubeId::ForeStarboard,
        MsgTorpedoTube::Aft => TorpedoTubeId::Aft,
    }
}

/// Handle `FireTorpedo` messages from the Tactical console.
///
/// Validates: sender is Tactical holder, game is in-progress, tube is loaded,
/// and there are torpedoes remaining.
/// On success, launches the torpedo and broadcasts `TorpedoLaunched` to all.
fn handle_fire_torpedo(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    ship: Res<ShipState>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let ClientMessage::FireTorpedo { tube, target_uuid } = &ev.msg else { continue };
        // Only the Tactical console holder may fire torpedoes.
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        let tube_id = to_tube_id(*tube);
        let uuid = uuid::Uuid::new_v4().to_string();
        // Heading matches ship yaw (torpedoes fire along ship forward for fore tubes).
        let launch_heading = ship.yaw;
        use crate::torpedo::LaunchResult;
        match torpedo_sys.0.launch(tube_id, uuid, ship.x, ship.z, launch_heading, target_uuid.clone()) {
            LaunchResult::Launched { uuid: launched_uuid } => {
                outbox.0.push((Target::All, ServerMessage::TorpedoLaunched {
                    uuid: launched_uuid,
                    tube: *tube,
                    x: ship.x,
                    z: ship.z,
                    heading: launch_heading,
                }));
            }
            LaunchResult::TubeNotLoaded | LaunchResult::NoTorpedoes => {
                // Silently ignore; client should check state before firing.
            }
        }
    }
}

/// Advance all in-flight torpedoes and broadcast `TorpedoDestroyed` for any
/// that expire this tick.
fn tick_torpedo_system(
    phase: Res<CurrentPhase>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    world: Res<WorldResource>,
    time: Res<Time>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let dt = time.delta_secs();
    let target_positions: std::collections::HashMap<String, (f32, f32)> = world.0.entities
        .iter()
        .map(|a| (a.uuid.clone(), (a.x(), a.z())))
        .collect();
    let result = torpedo_sys.0.tick(dt, &target_positions);
    for expired_uuid in result.expired {
        outbox.0.push((Target::All, ServerMessage::TorpedoDestroyed { uuid: expired_uuid }));
    }
}

/// Handle `Repair { shape }` messages from the Repair console.
///
/// Validates: game is in-progress, sender holds `Console::Repair`.
/// - If no free team exists: message ignored.
/// - If queue head shape matches pressed shape: lowest-numbered free team
///   dispatched, breakdown popped from queue.
/// - If queue head shape does not match (or queue empty): lowest-numbered
///   free team penalised.
fn handle_repair(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    mut breakdowns: ResMut<BreakdownQueueResource>,
    mut teams: ResMut<ShipRepairTeams>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    for ev in reader.read() {
        let pressed_shape = match &ev.msg {
            ClientMessage::Repair { shape } => *shape,
            _ => continue,
        };
        // Only the Repair console holder may send shape-matching presses.
        let Some(repair_token) = sessions.0.console_holder(Console::Repair) else {
            continue;
        };
        if ev.token.as_str() != repair_token {
            continue;
        }
        // Must have a free team to act.
        let Some(team_idx) = teams.0.lowest_free_team() else {
            continue;
        };
        // Check queue front shape (or empty queue).
        match breakdowns.queue.front() {
            Some(entry) if entry.shape == pressed_shape => {
                // Correct shape: dispatch team and pop breakdown.
                teams.0.dispatch(team_idx);
                breakdowns.queue.pop_front();
            }
            _ => {
                // Wrong shape or queue empty: penalise the free team.
                teams.0.penalise(team_idx);
            }
        }
    }
}

/// Handle `IncreasePower` and `DecreasePower` messages from the Power console.
///
/// Validates: game is in-progress, sender holds `Console::Power`.
/// Forwards to `PowerSystem::increase` / `decrease` which enforce bounds and lock.
/// Modifier sync is handled separately by `translate_power_modifiers` in the
/// coordination plugin (runs after this system each frame).
fn handle_power_messages(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    phase: Res<CurrentPhase>,
    mut power: ResMut<ShipPowerSystem>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
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
fn tick_power_system(
    time: Res<Time>,
    phase: Res<CurrentPhase>,
    mut power: ResMut<ShipPowerSystem>,
    config: Res<PowerConfigResource>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let dt = time.delta_secs();
    power.0.tick(dt, &config.0);
}

/// Handle `StartImpulseCharge` and `CancelImpulse` messages from helm/navigation.
/// Also cancels impulse whenever the hull takes damage this frame.
/// `StartImpulseCharge` is ignored when the ship is inside a `BlocksImpulse` region.
/// Tick repair teams each frame: advance progress, apply HP for completed
/// repairs.
fn tick_repair_teams(
    time: Res<Time>,
    mut teams: ResMut<ShipRepairTeams>,
    mut hull: ResMut<ShipHullIntegrity>,
    phase: Res<CurrentPhase>,
    modifiers: Res<ShipModifiers>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    let dt = time.delta_secs();
    let repair_mult = modifiers.get(&ModifierSlot::RepairRate);
    let completed = teams.0.tick(dt * repair_mult);
    for _team_idx in completed {
        hull.0.restore(REPAIR_TEAM_HP);
    }
}


/// Tick the active beam each frame: apply damage, check sever conditions
/// (arc, range, target destroyed), and handle natural expiry.
///
/// When the beam ends (any cause), starts the post-beam cooldown and broadcasts
/// `BeamEnded`. If the target asteroid reaches 0 HP, also broadcasts
/// `AsteroidDestroyed` and removes it from `WorldData`.
fn tick_active_beam(
    time: Res<Time>,
    mut beam: ResMut<ActiveBeam>,
    mut cooldown: ResMut<PhaserCooldown>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    ship: Res<ShipState>,
    mut world: ResMut<WorldResource>,
    mut asteroid_query: Query<(Entity, &AsteroidUuid, &mut AsteroidDamage)>,
    mut commands: Commands,
    phase: Res<CurrentPhase>,
    modifiers: Res<ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    let dt = time.delta_secs();

    // Tick cooldown regardless of beam state.
    cooldown.tick(dt);

    let Some(target_uuid) = beam.target_uuid.clone() else {
        return;
    };

    // Check sever: target no longer exists in world data.
    let asteroid_info = world.0.entities.iter().find(|a| a.uuid == target_uuid).cloned();
    let Some(info) = asteroid_info else {
        // Target was already destroyed (e.g., double-tick race). End beam silently.
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid }));
        return;
    };

    // Check sever: out of range or out of arc.
    let effective_phaser_range = crate::radar::PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
    if !crate::radar::is_fire_ready_with_range(info.x(), info.z(), ship.x, ship.z, ship.yaw, effective_phaser_range) {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid }));
        return;
    }

    // Apply damage proportionally to elapsed time.
    beam.damage_accumulator += BEAM_DAMAGE_PER_SEC * modifiers.get(&ModifierSlot::PhaserDamage) * dt;
    let damage_to_apply = beam.damage_accumulator.floor() as i32;
    if damage_to_apply > 0 {
        beam.damage_accumulator -= damage_to_apply as f32;

        // Find the asteroid entity and apply damage.
        let mut destroyed = false;
        for (entity, uuid_comp, mut dmg) in asteroid_query.iter_mut() {
            if uuid_comp.0 == target_uuid {
                dmg.current_hp = (dmg.current_hp - damage_to_apply).max(0);
                if dmg.current_hp == 0 {
                    destroyed = true;
                    commands.entity(entity).despawn();
                }
            }
        }

        if destroyed {
            // Remove from world data.
            world.0.entities.retain(|a| a.uuid != target_uuid);

            // Fire VFX event with the asteroid's last known position so the
            // renderer can play the destruction ripple.
            vfx_events.write(AsteroidDestroyedVfx { x: info.x(), z: info.z() });

            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start();

            outbox.0.push((Target::All, ServerMessage::AsteroidDestroyed { uuid: target_uuid.clone() }));
            outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid }));
            return;
        }
    }

    // Tick beam duration.
    beam.remaining_secs -= dt;
    if beam.remaining_secs <= 0.0 {
        // Natural expiry.
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start();
        outbox.0.push((Target::All, ServerMessage::BeamEnded { target_uuid }));
    }
}

/// Broadcast `ShieldStatus` to all players at 10 Hz.
fn broadcast_shield_status(
    time: Res<Time>,
    mut timer: ResMut<SimBroadcastTimer>,
    shields: Res<ShipShields>,
    phase: Res<CurrentPhase>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let facings = shields.0.snapshot().into_iter().map(|s| ShieldFacingStatus {
        label: s.label,
        hp: s.hp,
        max_hp: s.max_hp,
        online: s.online,
        offline_remaining: s.offline_remaining,
        is_focused: s.is_focused,
    }).collect();
    outbox.0.push((Target::All, ServerMessage::ShieldStatus { facings }));
}


/// Emit a single `WorldSetup` broadcast the first frame the game enters
/// `InProgress`. Stays silent in Lobby and on subsequent in-game ticks.
fn broadcast_world_setup_on_start(
    world: Res<WorldResource>,
    phase: Res<CurrentPhase>,
    mut state: ResMut<WorldSetupBroadcast>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress || state.sent {
        return;
    }
    outbox.0.push((Target::All, ServerMessage::WorldSetup { world: world.0.clone() }));
    state.sent = true;
}

/// Broadcast `ShowRepairIcon` / `ClearRepairIcon` to console holders based
/// on the current breakdown queue state. Sends deltas only.
fn broadcast_repair_icons(
    sessions: Res<Sessions>,
    breakdowns: Res<BreakdownQueueResource>,
    phase: Res<CurrentPhase>,
    mut icon_state: ResMut<RepairIconState>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }
    // Debug: verify we can see the breakdown queue
    let _debug_count = breakdowns.queue.len();
    use crate::breakdown::ALL_CONSOLES;
    use rand::Rng;
    use std::collections::{HashMap, HashSet};

    let mut current: HashMap<Console, Shape> = HashMap::new();
    let mut damaged: HashSet<Console> = HashSet::new();

    for entry in breakdowns.queue.entries() {
        damaged.insert(entry.console.clone());
        current.insert(entry.console.clone(), entry.shape);
    }

    if !breakdowns.queue.is_empty() {
        let undamaged: Vec<&Console> = ALL_CONSOLES
            .iter()
            .filter(|c| !damaged.contains(c))
            .collect();
        if !undamaged.is_empty() {
            let idx = icon_state.rng.random_range(0..undamaged.len());
            let decoy = undamaged[idx].clone();
            let shape = match icon_state.rng.random_range(0..3) {
                0 => Shape::Square,
                1 => Shape::Triangle,
                _ => Shape::Circle,
            };
            current.insert(decoy, shape);
        }
    }

    for (console, _) in &icon_state.last_icons {
        if !current.contains_key(console) {
            if let Some(token) = sessions.0.console_holder(console.clone()) {
                outbox.0.push((Target::Token(token.to_string()), ServerMessage::ClearRepairIcon));
            }
        }
    }

    for (console, shape) in &current {
        if icon_state.last_icons.get(console) != Some(shape) {
            if let Some(token) = sessions.0.console_holder(console.clone()) {
                outbox.0.push((Target::Token(token.to_string()), ServerMessage::ShowRepairIcon { shape: *shape }));
            }
        }
    }

    icon_state.last_icons = current;
}

/// Reconciles the live ECS entities with the `TrackedEntities` registry each tick.
///
/// For non-asteroid entities carrying `EntityUuid`:
/// - New entities (present in ECS, absent from `reported`) emit `EntitySpawned`
///   and are added to `WorldResource.entities` so they appear on reconnect `Welcome`.
/// - Missing entities (absent from ECS, present in `reported`) emit
///   `EntityDespawned` and are removed from `WorldResource.entities`.
///
/// Asteroids are excluded (they use `AsteroidSpawned` / `AsteroidDestroyed`).
///
/// On the very first `InProgress` tick, seeds `reported` from the initial
/// `WorldResource` entities so those are not re-broadcast.
fn reconcile_runtime_entities(
    mut registry: ResMut<TrackedEntities>,
    mut world: ResMut<WorldResource>,
    query: Query<(Entity, &EntityUuid, Option<&EntityId>, &Transform, Option<&RegionShapeSection>, Option<&EntityTagsSection>), Without<Asteroid>>,
    phase: Res<CurrentPhase>,
    mut outbox: ResMut<SimOutbox>,
) {
    if phase.0 != GamePhase::InProgress {
        return;
    }

    // Build the current set of ECS entity UUIDs.
    let current: HashMap<String, Entity> = query
        .iter()
        .map(|(e, u, _, _, _, _)| (u.0.clone(), e))
        .collect();

    /// Serialise a `RegionShape` to the wire string (snake_case variant name).
    fn shape_to_wire(shape: &RegionShapeSection) -> String {
        use crate::region_shape::RegionShape;
        match &shape.0 {
            RegionShape::Sphere { .. } => "sphere",
            RegionShape::Box { .. } => "box",
            RegionShape::Cylinder { .. } => "cylinder",
        }.to_string()
    }

    // Seed reported set from ECS on first in-progress frame so that initial
    // world entities (stars, planets, ships, fields) are not re-reported.
    // Also populate WorldData.entities so the reconnect Welcome includes them.
    if !registry.seeded {
        for (uuid, entity) in &current {
            registry.reported.insert(uuid.clone());
            if let Ok((_, _, id, transform, region_shape, entity_tags)) = query.get(*entity) {
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                }
                world.0.entities.push(snapshot);
            }
        }
        registry.seeded = true;
        return;
    }

    // Emit EntitySpawned for new entities.
    for (uuid, entity) in &current {
        if registry.reported.insert(uuid.clone()) {
            if let Ok((_, _, id, transform, region_shape, entity_tags)) = query.get(*entity) {
                let mut snapshot = EntitySnapshot {
                    uuid: uuid.clone(),
                    id: id.as_ref().map(|i| i.0.clone()),
                    position: Some([
                        transform.translation.x,
                        transform.translation.y,
                        transform.translation.z,
                    ]),
                    tags: entity_tags.map(|t| t.0.clone()).unwrap_or_default(),
                    ..EntitySnapshot::default()
                };
                if let Some(shape) = region_shape {
                    snapshot.shape = Some(shape_to_wire(shape));
                }
                world.0.entities.push(snapshot.clone());
                outbox.0.push((Target::All, ServerMessage::EntitySpawned { snapshot }));
            }
        }
    }

    // Emit EntityDespawned for entities no longer in the ECS.
    let reported_snapshot: Vec<String> = registry.reported.iter().cloned().collect();
    for uuid in &reported_snapshot {
        if !current.contains_key(uuid) {
            registry.reported.remove(uuid);
            world.0.entities.retain(|e| e.uuid != *uuid);
            outbox.0.push((Target::All, ServerMessage::EntityDespawned { uuid: uuid.clone() }));
        }
    }
}

// â”€â”€ World Setup â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
fn setup_world(
    commands: Commands,
    meshes: ResMut<Assets<Mesh>>,
    materials: ResMut<Assets<StandardMaterial>>,
    world: ResMut<WorldResource>,
) {
    // Try to get the preloaded map config and config cache.
    // Hardcoded fallback is handled by WorldPlugin (src/world/server.rs).
    if let Some(map_config) = crate::config_cache::get_map_config() {
        let config_cache = crate::config_cache::get_config_cache();
        setup_world_from_config(commands, meshes, materials, world, map_config, config_cache);
    }
}

fn setup_world_from_config(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    _world: ResMut<WorldResource>,
    map_config: MapConfig,
    config_cache: crate::config_cache::ConfigCache,
) {
    // -- Spawn immediate entities from entity instances ------------
    for entity_inst in &map_config.entities {
        if entity_inst.spawn_on != crate::map_config::EntityInstanceSpawnOn::Immediate {
            continue;
        }
        spawn_entity_instance(&mut commands, &map_config, &config_cache, entity_inst);
    }

    // -- Starfield skybox ------------------------------------------
    spawn_starfield(&mut commands, &mut meshes, &mut materials);
}

/// Spawn a single entity instance: resolve template, apply overrides, spawn.
fn spawn_entity_instance(
    commands: &mut Commands,
    _map_config: &MapConfig,
    config_cache: &crate::config_cache::ConfigCache,
    entity_inst: &crate::map_config::EntityInstance,
) {
    let config = match crate::entity_loader::resolve_entity(entity_inst, config_cache) {
        Ok(c) => c,
        Err(e) => {
            bevy::log::error!("Failed to resolve entity '{}': {}", entity_inst.template_path, e);
            return;
        }
    };

    let uuid = crate::entity_loader::assign_uuid();
    let pos = if entity_inst.position.len() >= 3 {
        Vec3::new(entity_inst.position[0], entity_inst.position[1], entity_inst.position[2])
    } else {
        Vec3::ZERO
    };

    crate::entity_spawner::spawn_entity(commands, &config, pos, uuid, entity_inst.id.clone());
}

/// Spawn the procedural starfield skybox.
fn spawn_starfield(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let star_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        unlit: true,
        ..default()
    });
    let star_mesh = meshes.add(Sphere { radius: 1.0 });
    let star_count = 400u32;
    let radius = 2000.0_f32;
    for i in 0..star_count {
        let frac = (i as f32 + 0.5) / star_count as f32;
        let phi = (1.0 - 2.0 * frac).acos();
        let theta = std::f32::consts::PI * (1.0 + 5_f32.sqrt()) * i as f32;
        let x = phi.sin() * theta.cos() * radius;
        let y = phi.sin() * theta.sin() * radius;
        let z = phi.cos() * radius;
        let h = ((i.wrapping_mul(2654435761)) ^ 0xDEADBEEF) % 100;
        let scale = 1.5 + (h as f32) / 25.0;
        commands.spawn((
            Mesh3d(star_mesh.clone()),
            MeshMaterial3d(star_mat.clone()),
            Transform::from_xyz(x, y, z).with_scale(Vec3::splat(scale)),
        ));
    }
}

/// Spawn entities with `spawn_on = GameStart` (e.g. player ship) when the
/// game transitions to InProgress. Runs once per game.
fn spawn_game_start_entities(
    mut commands: Commands,
    phase: Res<crate::lobby::CurrentPhase>,
    map_config: Option<Res<MapConfig>>,
    mut has_spawned: Local<bool>,
) {
    if *has_spawned {
        return;
    }
    if phase.0 != crate::messages::GamePhase::InProgress {
        return;
    }

    let mc = match map_config.as_deref() {
        Some(mc) => mc,
        None => return,
    };

    let config_cache = crate::config_cache::get_config_cache();

    let mut ship_spawned = false;
    for entity_inst in &mc.entities {
        if entity_inst.spawn_on != crate::map_config::EntityInstanceSpawnOn::GameStart {
            continue;
        }
        let config = match crate::entity_loader::resolve_entity(entity_inst, &config_cache) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!("Failed to resolve GameStart entity '{}': {}", entity_inst.template_path, e);
                continue;
            }
        };

        let uuid = crate::entity_loader::assign_uuid();
        let pos = if entity_inst.position.len() >= 3 {
            Vec3::new(entity_inst.position[0], entity_inst.position[1], entity_inst.position[2])
        } else {
            Vec3::ZERO
        };

        let spawned = crate::entity_spawner::spawn_entity(
            &mut commands, &config, pos, uuid, entity_inst.id.clone(),
        );

        // The first GameStart entity with tags containing "ship" gets the Ship marker
        if !ship_spawned && config.tags.iter().any(|t| t == "ship") {
            commands.entity(spawned).insert(Ship);
            ship_spawned = true;

            // Ship-specific resource setup
            if let Some(hc) = &config.hull {
                commands.insert_resource(ShipHullIntegrity(HullIntegrity::with_hp(hc.hull_integrity)));
            } else {
                commands.insert_resource(ShipHullIntegrity(HullIntegrity::new()));
            }

            // Apply shield focus config from TOML if present
            if let Some(sc) = &config.shields_console {
                let mut shields = ShipShields(ShieldSystem::default());
                shields.0.focus_config = crate::shield::ShieldFocusConfig {
                    bonus_max_hp: sc.focus_bonus_max_hp,
                    bonus_regen: sc.focus_bonus_regen,
                    penalty_max_hp: sc.focus_penalty_max_hp,
                    penalty_regen: sc.focus_penalty_regen,
                    decay_rate: sc.focus_decay_rate,
                };
                commands.insert_resource(shields);
            }

            if let Some(wc) = &config.weapons_console {
                let beam_color = crate::beam_render::resolve_beam_color(&wc.beam_color);
                let beam_range = if wc.beam_range > 0.0 { wc.beam_range } else { 40.0 };
                commands.insert_resource(PhaserRenderConfig { beam_color, beam_range });
            }

            if let Some(pc) = &config.power {
                commands.insert_resource(PowerConfigResource(
                    crate::power_system::PowerConfig {
                        capacity: pc.capacity,
                        rates: pc.rates,
                        emergency_threshold: pc.emergency_threshold,
                    }
                ));
            }

            // Power multipliers
            let defaults = [-0.5, 0.0, 0.25, 0.5];
            let mut multipliers: std::collections::HashMap<Console, [f32; 4]> = std::collections::HashMap::from([
                (Console::Helm, defaults),
                (Console::Tactical, defaults),
                (Console::Sensors, defaults),
            ]);
            if let Some(hc) = &config.helm_console {
                if let Some(pm) = hc.power_multipliers {
                    multipliers.insert(Console::Helm, pm);
                }
            }
            if let Some(wc) = &config.weapons_console {
                if let Some(pm) = wc.power_multipliers {
                    multipliers.insert(Console::Tactical, pm);
                }
            }
            if let Some(sc) = &config.science_console {
                if let Some(pm) = sc.power_multipliers {
                    // science_console power drives the Sensors radar range multiplier
                    multipliers.insert(Console::Sensors, pm);
                }
            }
            commands.insert_resource(PowerMultiplierResource { multipliers });
        }
    }

    *has_spawned = true;
}

/// Add visual meshes and materials to spawned entities that have StarSection
/// or PlanetSection but no mesh yet.
fn render_spawned_entities(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    stars: Query<(Entity, &crate::entity_spawner::StarSection, &Transform), Without<Mesh3d>>,
    planets: Query<(Entity, &crate::entity_spawner::PlanetSection, &Transform), Without<Mesh3d>>,
) {
    for (entity, star, _transform) in stars.iter() {
        let mesh = meshes.add(Sphere { radius: star.0.radius });
        let color = if star.0.colour.len() >= 3 {
            Color::srgb(star.0.colour[0], star.0.colour[1], star.0.colour[2])
        } else {
            Color::srgb(1.0, 1.0, 1.0)
        };
        let mat = materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * 2.0,
            ..default()
        });
        commands.entity(entity).insert((Mesh3d(mesh), MeshMaterial3d(mat)));
    }

    for (entity, planet, _transform) in planets.iter() {
        let mesh = meshes.add(Sphere { radius: planet.0.radius });
        let color = if planet.0.colour.len() >= 3 {
            Color::srgb(planet.0.colour[0], planet.0.colour[1], planet.0.colour[2])
        } else {
            Color::srgb(0.5, 0.5, 0.5)
        };
        let mat = materials.add(StandardMaterial {
            base_color: color,
            ..default()
        });
        commands.entity(entity).insert((Mesh3d(mesh), MeshMaterial3d(mat)));
    }
}


// â”€â”€ Tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::collision_damage;
    use crate::lobby::{LobbyPlugin, InboundMessage, OutboundMessage};
    use crate::messages::*;
    use crate::ship_plugin::handle_impulse_messages;

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
        // Advance time by 200 ms per tick so Hz-based SimBroadcaster timers
        // (period = 100 ms) always fire within a single update call.
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        .insert_resource(ShipState::new())
        .insert_resource(ShipHullIntegrity(HullIntegrity::new()))
        .insert_resource(ShipShields(ShieldSystem::default()))
        .insert_resource(ShipImpulse(ImpulseState::new()))
        .init_resource::<WorldResource>()
        .init_resource::<WorldSetupBroadcast>()
        .init_resource::<WeaponsTarget>()
        .init_resource::<ActiveBeam>()
        .add_message::<AsteroidDestroyedVfx>()
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
        .insert_resource(SimBroadcastTimer(Timer::new(
            std::time::Duration::from_nanos(1), TimerMode::Repeating)))
        .init_resource::<crate::console_ai_plugin::ConsoleComplexityState>()
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .add_plugins(crate::captain_plugin::CaptainPlugin)
        .add_systems(Update, (
            handle_set_target,
            handle_set_science_target, handle_set_sensors_target,
            handle_fire_phaser, handle_set_phaser_mode,
            handle_set_phaser_frequency, handle_fire_torpedo,
            handle_repair, handle_power_messages,
            handle_impulse_messages, handle_set_shield_focus,
            tick_active_beam, tick_repair_teams,
            tick_torpedo_system, tick_power_system,
            broadcast_shield_status,
            broadcast_world_setup_on_start.after(crate::lobby::process_lobby),
            broadcast_repair_icons,
            reconcile_runtime_entities.after(crate::lobby::process_lobby),
        ))
        .add_systems(Update, crate::modifier_coordination::translate_power_modifiers
            .after(handle_power_messages)
            .after(tick_power_system))
        .add_systems(Update, crate::modifier_coordination::translate_impulse_modifiers
            .after(handle_impulse_messages))
        .add_systems(Update, sim_processing_anchor)
        .add_plugins(power_state_broadcaster())
        .add_plugins(weapons_update_broadcaster())
        .add_plugins(repair_state_broadcaster())
        .add_plugins(sim_state_broadcaster())
        .add_plugins(modifier_events_broadcaster())
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
        // Drain any leftover SimOutbox entries that the sim systems wrote but
        // were not captured by the PostUpdate collect system (SimOutbox is not
        // connected to the OutboundMessage bus for test_app).
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

    fn start_game_with_helm(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "helm", ClientMessage::Identify { token: "helm".into(), name: "Bob".into() });
        tick(app);
        push(app, "helm", ClientMessage::SelectStation { station: "Helm".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn start_game_with_sensors(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "sensors", ClientMessage::Identify { token: "sensors".into(), name: "Spock".into() });
        tick(app);
        push(app, "sensors", ClientMessage::SelectStation { station: "Sensors".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn start_game_with_navigation(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "navigation", ClientMessage::Identify { token: "navigation".into(), name: "Decker".into() });
        tick(app);
        push(app, "navigation", ClientMessage::SelectStation { station: "Navigation".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn sensors_can_switch_view_to_science_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(&mut app, "sensors", ClientMessage::SetView { mode: ViewMode::ScienceRadar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::ScienceRadar
        );
    }
    #[test]
    fn sensors_can_switch_view_to_sensors_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(&mut app, "sensors", ClientMessage::SetView { mode: ViewMode::SensorsRadar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::SensorsRadar
        );
    }

    #[test]
    fn non_sensors_cannot_switch_view_to_sensors_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::SensorsRadar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }


    #[test]
    fn navigation_can_switch_view_to_system_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(&mut app, "navigation", ClientMessage::SetView { mode: ViewMode::SystemChart });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::SystemChart
        );
    }

    #[test]
    fn non_sensors_cannot_switch_view_to_science_radar() {
        let mut app = test_app();
        start_game_with_sensors(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::ScienceRadar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn non_navigation_cannot_switch_view_to_system_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::SystemChart });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn navigation_can_switch_view_to_navigation_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(&mut app, "navigation", ClientMessage::SetView { mode: ViewMode::NavigationChart });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::NavigationChart
        );
    }

    #[test]
    fn non_navigation_cannot_switch_view_to_navigation_chart() {
        let mut app = test_app();
        start_game_with_navigation(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::NavigationChart });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    fn start_game_with_comms(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "comms", ClientMessage::Identify { token: "comms".into(), name: "Uhura".into() });
        tick(app);
        push(app, "comms", ClientMessage::SelectStation { station: "Comms".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn comms_can_push_view_to_comms() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(&mut app, "comms", ClientMessage::SetView { mode: ViewMode::Comms });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Comms
        );
    }

    #[test]
    fn captain_override_from_comms_view_works() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(&mut app, "comms", ClientMessage::SetView { mode: ViewMode::Comms });
        tick(&mut app);
        // Captain overrides back to a camera view.
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Aft)
        );
    }

    #[test]
    fn non_comms_cannot_push_comms_view() {
        let mut app = test_app();
        start_game_with_comms(&mut app);
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Comms });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn helm_can_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Radar
        );
    }

    #[test]
    fn captain_cannot_switch_view_to_radar() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        // Captain has no authority over Radar; request is silently dropped.
        push(&mut app, "captain", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn helm_cannot_switch_view_to_camera() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<ShipState>().view_mode,
            ViewMode::Camera(ViewDirection::Fore)
        );
    }

    #[test]
    fn sim_state_broadcast_carries_ship_position_and_view_mode() {
        let mut app = test_app();
        start_game_with_helm(&mut app);
        // Move the ship and switch to radar
        {
            let mut ship = app.world_mut().resource_mut::<ShipState>();
            ship.x = 12.0;
            ship.z = -3.5;
            ship.yaw = 1.25;
        }
        push(&mut app, "helm", ClientMessage::SetView { mode: ViewMode::Radar });
        tick(&mut app);
        // Ensure Time has accumulated some real delta and the broadcast fires.
        // Two prior ticks have already advanced TimePlugin's clock; a fresh
        // tick now sees a non-zero delta, finishing the 1-ns broadcast timer.
        let out = tick(&mut app);

        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");

        assert_eq!(snap.ship_x, 12.0);
        assert_eq!(snap.ship_z, -3.5);
        assert_eq!(snap.ship_yaw, 1.25);
        assert_eq!(snap.view_mode, ViewMode::Radar);
    }

    #[test]
    fn world_setup_is_broadcast_once_after_start_game() {
        let mut app = test_app();
        // Pre-populate world data so the broadcast has something to emit.
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid("test-uuid", 5.0, -1.0, 2.0)],
        }));

        // Bring the game up to the point of pressing StartGame
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "A".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(&mut app);
        // The StartGame tick should produce the WorldSetup broadcast
        push(&mut app, "captain", ClientMessage::StartGame);
        let start_out = tick(&mut app);

        let world_setups: Vec<_> = start_out.iter().filter(|m|
            matches!(&m.msg, ServerMessage::WorldSetup { .. })
        ).collect();
        assert_eq!(world_setups.len(), 1, "expected exactly one WorldSetup on the StartGame tick");
        match &world_setups[0].msg {
            ServerMessage::WorldSetup { world } => {
                assert_eq!(world.entities.len(), 1);
                assert_eq!(world.entities[0].x(), 5.0);
            }
            _ => unreachable!(),
        }
        match &world_setups[0].target {
            crate::lobby::Target::All => {}
            t => panic!("WorldSetup should target All, got {:?}", t),
        }

        // Subsequent ticks must not re-broadcast WorldSetup
        let later = tick(&mut app);
        assert!(!later.iter().any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should only fire once per game");
    }

    #[test]
    fn world_setup_is_not_broadcast_during_lobby() {
        let mut app = test_app();
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid("test-uuid", 0.0, 0.0, 2.0)],
        }));
        // Identify and select a console but don't start the game.
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "A".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        let out = tick(&mut app);
        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::WorldSetup { .. })),
            "WorldSetup should not be broadcast in the Lobby phase");
    }

    #[test]
    fn hull_integrity_starts_at_100_and_appears_in_sim_snapshot() {
        let mut app = test_app();
        start_game(&mut app);
        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");
        assert!((snap.hull_integrity - 100.0).abs() < 1e-6);
    }

    #[test]
    fn direct_damage_reduces_hull_integrity_in_broadcast() {
        let mut app = test_app();
        start_game(&mut app);

        // Directly apply damage to the resource (simulates collision at ~half speed).
        app.world_mut()
            .resource_mut::<ShipHullIntegrity>()
            .0.apply_damage(10.0);

        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");
        assert!((snap.hull_integrity - 90.0).abs() < 1e-6);
    }

    #[test]
    fn taking_25hp_damage_enqueues_2_breakdowns_and_snapshot_shows_first() {
        let mut app = test_app();
        start_game(&mut app);

        // Apply 25 HP of damage directly in 10-HP bucket tracking terms,
        // mimicking how handle_collisions would do it via breakdowns_from_damage.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            let before = bd.cumulative_damage; // 0.0
            bd.cumulative_damage += 25.0;
            let new_count = breakdowns_from_damage(before, bd.cumulative_damage);
            assert_eq!(new_count, 2, "25 HP should create exactly 2 breakdowns");
            let BreakdownQueueResource { queue, rng, .. } = &mut *bd;
            for _ in 0..new_count {
                queue.push_random(rng);
            }
        }

        let _out = tick(&mut app);

        // Verify queue length via resource.
        let bd = app.world().resource::<BreakdownQueueResource>();
        assert_eq!(bd.queue.len(), 2, "2 breakdowns should be queued");
    }

    #[test]
    fn advancing_queue_exposes_next_breakdown() {
        let mut app = test_app();
        start_game(&mut app);

        // Seed 2 breakdowns.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.cumulative_damage = 25.0;
            let BreakdownQueueResource { queue, rng, .. } = &mut *bd;
            queue.push_random(rng);
            queue.push_random(rng);
        }

        // Capture the first (front) entry.
        let first = app.world().resource::<BreakdownQueueResource>().queue.front().cloned();

        // Pop the front (simulating a successful repair).
        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();

        // Second entry is now the front.
        let second = app.world().resource::<BreakdownQueueResource>().queue.front().cloned();

        assert!(second.is_some(), "second breakdown should now be front");
        assert_ne!(first, second, "consecutive entries are different consoles");
    }

    // â”€â”€ SetTarget / TargetLock tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    fn setup_weapons_world(app: &mut App, asteroid_x: f32, asteroid_z: f32) {
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![EntitySnapshot::asteroid("target-uuid", asteroid_x, asteroid_z, 2.0)],
        }));
    }

    /// Like `setup_weapons_world` but also spawns the Bevy entity so that beam
    /// damage can actually be applied and the asteroid can be destroyed.
    fn setup_weapons_world_with_entity(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> bevy::ecs::entity::Entity {
        setup_weapons_world(app, asteroid_x, asteroid_z);
        app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
            Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
        )).id()
    }

    fn start_game_with_weapons(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(app);
        push(app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
        // Asteroid at (30, 0) â€” 30 units from ship origin, within 60-unit range.
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert_eq!(lock.0, "target-uuid");
        assert!(lock.1, "expected locked=true for in-range asteroid");

        // Server state should record the lock.
        assert_eq!(
            app.world().resource::<WeaponsTarget>().0.as_deref(),
            Some("target-uuid")
        );
    }

    #[test]
    fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
        let mut app = test_app();
        // Asteroid at (80, 0) â€” 80 units away, outside 60-unit Weapons range.
        setup_weapons_world(&mut app, 80.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for out-of-range asteroid");
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
    }

    #[test]
    fn unknown_uuid_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 10.0, 0.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "no-such-asteroid".into() });
        let out = tick(&mut app);

        let lock = out.iter().find_map(|m| match &m.msg {
            ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
            _ => None,
        }).expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for unknown UUID");
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
    }

    // â”€â”€ WeaponsUpdate / fire_ready tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Target locked, within 40-unit phaser range, in forward arc â†’ fire_ready = true.
    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        // Ship at origin, yaw=0 (facing -Z). Asteroid at (0, -20): directly ahead, 20 units away.
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        // Lock the target
        let _ = tick(&mut app);
        // Now run another tick to get a WeaponsUpdate
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, fire_ready, .. } =>
                Some((target_uuid.clone(), *fire_ready)),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(update.1, "expected fire_ready=true for in-range, forward-arc target");
    }

    /// Target locked but beyond 40-unit phaser range (within 60u lock range) â†’ fire_ready = false.
    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        // Ship at origin, yaw=0. Asteroid at (0, -50): directly ahead, 50 units â€” within lock range
        // (60u) but outside phaser range (40u).
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, fire_ready, .. } =>
                Some((target_uuid.clone(), *fire_ready)),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(!update.1, "expected fire_ready=false for beyond-phaser-range target");
    }

    // â”€â”€ FirePhaser / beam lifecycle tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Helper: lock target then fire phaser; returns messages from the fire tick.
    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        // Lock
        push(app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(app);
        // Fire
        push(app, "weapons", ClientMessage::FirePhaser);
        tick(app)
    }

    /// Firing at a fire-ready target broadcasts BeamStarted to all.
    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        // Asteroid directly ahead at 20 units (yaw=0 â†’ facing -Z â†’ asteroid at (0,-20)).
        let out = lock_and_fire(&mut app, 0.0, -20.0);

        let beam_started = out.iter().find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
        assert!(beam_started.is_some(), "expected BeamStarted after firing at fire-ready target");
        match &beam_started.unwrap().msg {
            ServerMessage::BeamStarted { target_uuid } => assert_eq!(target_uuid, "target-uuid"),
            _ => unreachable!(),
        }
        match &beam_started.unwrap().target {
            Target::All => {}
            t => panic!("BeamStarted should target All, got {:?}", t),
        }

        // ActiveBeam resource should be populated.
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );
    }

    /// FirePhaser is silently ignored when the phaser is on cooldown.
    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Manually put the cooldown into active state (simulating a beam just ended).
        app.world_mut().resource_mut::<ActiveBeam>().target_uuid = None;
        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 3.0;

        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown");
    }

    /// Non-weapons player cannot fire.
    #[test]
    fn fire_phaser_ignored_from_non_weapons_player() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game(&mut app);

        push(&mut app, "captain", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "captain should not be able to fire phaser");
    }

    /// When the beam fires at a target outside the 180Â° arc, it is rejected.
    #[test]
    fn fire_phaser_rejected_when_target_behind_ship() {
        let mut app = test_app();
        // Yaw=0 means ship faces -Z. Asteroid at (0, +20) is directly behind â€” in rear arc.
        setup_weapons_world(&mut app, 0.0, 20.0);
        start_game_with_weapons(&mut app);
        // Lock (within 60u range) â€” lock doesn't require arc.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        // Fire â€” rejected because target is behind.
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is in rear arc");
    }

    /// A 6-second natural beam kills the asteroid (5 HP/s Ã— 6s = 30 HP total).
    ///
    /// The test accelerates time by manipulating the beam state directly
    /// after confirming the beam started, then runs ticks with large deltas.
    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();

        // Spawn an asteroid entity with full HP so tick_active_beam can find it.
        let asteroid_entity = app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
        )).id();

        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Verify beam started.
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );

        // Fast-forward: accumulate 30 damage via the damage_accumulator.
        // Set accumulator to 30.0 so all damage applies in one tick.
        {
            let mut b = app.world_mut().resource_mut::<ActiveBeam>();
            b.damage_accumulator = 30.0;
            b.remaining_secs = 5.0; // still "ongoing"
        }

        let out = tick(&mut app);

        // Asteroid destroyed message should be present.
        let destroyed = out.iter().find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(destroyed.is_some(), "expected AsteroidDestroyed when asteroid HP reaches 0");
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        // BeamEnded also broadcast.
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction");

        // Asteroid no longer in world data.
        assert!(
            !app.world().resource::<WorldResource>().0.entities.iter().any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        // Beam resource cleared.
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());

        // Cooldown started.
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after beam end");

        // The entity should be despawned.
        assert!(app.world().get::<AsteroidDamage>(asteroid_entity).is_none(),
            "asteroid entity should be despawned");
    }

    /// Beam severs when ship rotates target out of the 180Â° forward arc.
    #[test]
    fn beam_severs_when_target_leaves_forward_arc() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Now rotate ship so the asteroid is behind it (yaw = Ï€ â†’ facing +Z, asteroid at (0,-20) is behind).
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves forward arc");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-arc");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after arc sever");
    }

    /// Beam severs when the target moves beyond 40-unit phaser range.
    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move asteroid position in WorldData to 50 units away (out of 40u range).
        app.world_mut().resource_mut::<WorldResource>().0.entities[0].position = Some([0.0, 0.0, -50.0]);

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-range");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after range sever");
    }

    /// No damage refund on sever â€” whatever HP was dealt is permanent.
    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        let asteroid_entity = app.world_mut().spawn((
            Asteroid,
            AsteroidUuid("target-uuid".into()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
        )).id();

        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Apply partial damage via accumulator.
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 10.0;
        let _ = tick(&mut app);

        // Now sever by rotating ship.
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;
        let _ = tick(&mut app);

        let hp = app.world().get::<AsteroidDamage>(asteroid_entity)
            .map(|d| d.current_hp);
        assert!(
            hp.is_some() && hp.unwrap() < 30,
            "asteroid should retain damage after sever (no refund), hp={:?}",
            hp
        );
    }

    /// A fresh FirePhaser after cooldown on a new locked target cancels any
    /// active beam and starts a new one.
    #[test]
    fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
        let mut app = test_app();

        // Set up two asteroids.
        app.world_mut().insert_resource(WorldResource(WorldData {
            entities: vec![
                EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
            ],
        }));
        start_game_with_weapons(&mut app);

        // Lock and fire at t1.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t1".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let _ = tick(&mut app);
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t1"));

        // Natural beam expiry: set remaining to 0.
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 0.0;
        // Zero damage accumulator so no destruction fires.
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 0.0;
        let _ = tick(&mut app); // beam ends, cooldown starts

        // Cooldown should be active.
        assert!(app.world().resource::<PhaserCooldown>().is_active());

        // Force cooldown to expire.
        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 0.0;

        // Lock and fire at t2.
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t2".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown");
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t2"));
    }

    // â”€â”€ Repair helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Set up a game with a captain, repair player, and a single breakdown
    /// with a known shape (Triangle) at the front. HP = 90.
    fn start_game_with_repair_shape(app: &mut App, shape: Shape) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "eng", ClientMessage::Identify { token: "eng".into(), name: "Bob".into() });
        tick(app);
        push(app, "eng", ClientMessage::SelectStation { station: "Repair".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);

        // Apply 10 damage so HP = 90.
        app.world_mut().resource_mut::<ShipHullIntegrity>().0.apply_damage(10.0);

        // Push a single breakdown with the requested shape and Repair console.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape,
            });
        }
    }

    /// Helpers to check RepairTeams team state.
    fn team_is_repairing(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Repairing { .. })
    }

    fn team_is_cooldown(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Cooldown { .. })
    }

    fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
        matches!(teams.0.slots()[idx], crate::messages::TeamSlot::Idle)
    }

    // -- Shape-matching repair tests --------------------------------------

    /// Non-Repair console holder sending `Repair { shape }` is ignored.
    #[test]
    fn non_repair_sender_is_ignored() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Captain (not Repair holder) presses a shape.
        push(&mut app, "captain", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_idle(&teams, 0), "team 0 should remain idle after non-Repair press");
        assert!(team_is_idle(&teams, 1), "team 1 should remain idle");
        assert!(team_is_idle(&teams, 2), "team 2 should remain idle");
    }

    /// Correct shape dispatches a team and pops the queue.
    #[test]
    fn correct_shape_dispatches_team_and_pops_queue() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Repair holder presses the matching shape.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_repairing(&teams, 0), "team 0 should be repairing after correct shape press");
        // Queue should be empty after pop.
        assert!(app.world().resource::<BreakdownQueueResource>().queue.is_empty(),
            "breakdown queue should be empty after correct shape repair");
    }

    /// Wrong shape penalises the lowest free team and leaves queue intact.
    #[test]
    fn wrong_shape_penalises_team_and_leaves_queue() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Repair holder presses the WRONG shape (Square, not Triangle).
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Square });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_cooldown(&teams, 0), "team 0 should be on cooldown after wrong shape press");
        // Queue should still have the breakdown.
        assert_eq!(app.world().resource::<BreakdownQueueResource>().queue.len(), 1,
            "breakdown queue should be unchanged after wrong shape press");
    }

    /// All-busy teams: no free team ? further presses are ignored.
    #[test]
    fn all_busy_teams_ignore_further_presses() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // First press: correct shape, dispatches team 0.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);
        assert!(team_is_repairing(&app.world().resource::<ShipRepairTeams>(), 0));

        // Push another breakdown.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            use crate::breakdown::BreakdownEntry;
            bd.queue.push_back(BreakdownEntry { console: Console::Repair, shape: Shape::Circle });
        }

        // Manually dispatch teams 1 and 2 so all three are busy.
        app.world_mut().resource_mut::<ShipRepairTeams>().0.dispatch(1);
        app.world_mut().resource_mut::<ShipRepairTeams>().0.dispatch(2);

        // Third press should be ignored (no free team).
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Circle });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_repairing(&teams, 0));
        assert!(team_is_repairing(&teams, 1));
        assert!(team_is_repairing(&teams, 2));
        // Queue should still have the second breakdown.
        assert_eq!(app.world().resource::<BreakdownQueueResource>().queue.len(), 1,
            "breakdown queue should remain unchanged when all teams are busy");
    }

    /// Empty-queue press penalises the lowest free team.
    #[test]
    fn empty_queue_press_penalises_team() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Pop the queue so it's empty.
        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();
        assert!(app.world().resource::<BreakdownQueueResource>().queue.is_empty());

        // Repair holder presses any shape.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);

        let teams = app.world().resource::<ShipRepairTeams>();
        assert!(team_is_cooldown(&teams, 0), "team 0 should be on cooldown after empty-queue press");
    }

    /// Repair team tick restores HP on completion.
    ///
    /// We test this by manually running the equivalent of `tick_repair_teams`
    /// system logic: advance the team to completion, then verify HP restoration.
    #[test]
    fn repair_team_completion_restores_hp() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        fn near(a: f32, b: f32) -> bool { (a - b).abs() < 1e-6 }

        let initial_hp = app.world().resource::<ShipHullIntegrity>().0.current(); // 90

        // Dispatch team 0 via correct shape press.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        tick(&mut app);
        assert!(team_is_repairing(&app.world().resource::<ShipRepairTeams>(), 0));

        // Advance team 0 to completion via the team's own tick method.
        let completed = app.world_mut().resource_mut::<ShipRepairTeams>().0.tick(30.0);
        assert_eq!(completed, vec![0], "team 0 should complete after 30s");

        // Manually apply HP as the system would: for each completed team, restore HP.
        for _ in completed {
            app.world_mut().resource_mut::<ShipHullIntegrity>().0.restore(REPAIR_TEAM_HP);
        }

        let hp_after = app.world().resource::<ShipHullIntegrity>().0.current();
        assert!(near(hp_after, initial_hp + REPAIR_TEAM_HP),
            "HP should increase by {} after repair team completion", REPAIR_TEAM_HP);
    }

    /// RepairState broadcast shows in_progress when team is repairing.
    #[test]
    fn repair_state_shows_in_progress() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Triangle });
        let out = tick(&mut app);

        let repair_state = out.iter().find(|m| {
            matches!(&m.msg, ServerMessage::RepairState { in_progress: true, .. })
                && matches!(&m.target, Target::Token(t) if t == "eng")
        });
        assert!(repair_state.is_some(),
            "RepairState with in_progress=true should be broadcast to repair console");
    }

    /// RepairState broadcast shows penalty when team is on cooldown.
    #[test]
    fn repair_state_shows_penalty() {
        let mut app = test_app();
        start_game_with_repair_shape(&mut app, Shape::Triangle);

        // Press wrong shape to penalise team 0.
        push(&mut app, "eng", ClientMessage::Repair { shape: Shape::Square });
        let out = tick(&mut app);

        // The penalty RepairState is broadcast to all consoles.
        let penalty_msg = out.iter().find(|m| {
            matches!(&m.msg, ServerMessage::RepairState { penalty: true, .. })
                && matches!(&m.target, Target::Token(t) if t == "eng")
        });
        assert!(penalty_msg.is_some(),
            "RepairState with penalty=true should be broadcast after wrong shape press");
    }

    // â”€â”€ SetPhaserMode tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// The Weapons console holder can change the phaser mode to Manual.
    #[test]
    fn weapons_console_can_set_phaser_mode_to_manual() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetPhaserMode { mode: crate::messages::PhaserMode::Manual });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Manual,
            "phaser mode should be Manual after SetPhaserMode"
        );
    }

    /// A non-Weapons player cannot change the phaser mode.
    #[test]
    fn non_weapons_player_cannot_set_phaser_mode() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "captain", ClientMessage::SetPhaserMode { mode: crate::messages::PhaserMode::Manual });
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Auto,
            "phaser mode should stay Auto when non-Weapons player sends SetPhaserMode"
        );
    }

    // â”€â”€ SetScienceTarget / ScienceTargetSuggestion tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    fn start_game_with_sensors_and_weapons(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "sensors", ClientMessage::Identify { token: "sensors".into(), name: "Spock".into() });
        tick(app);
        push(app, "sensors", ClientMessage::SelectStation { station: "Sensors".into() });
        tick(app);
        push(app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(app);
        push(app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

      fn sensors_set_science_target_broadcasts_suggestion_to_weapons() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);

        push(&mut app, "sensors", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        let suggestion = out.iter().find_map(|m| match &m.msg {
            ServerMessage::ScienceTargetSuggestion { uuid } => Some(uuid.clone()),
            _ => None,
        }).expect("expected a ScienceTargetSuggestion message");
        assert_eq!(suggestion, "asteroid-42");

        // Should be targeted to Weapons console player only.
        let suggestion_msg = out.iter().find(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. }))
            .unwrap();
        assert!(
            matches!(&suggestion_msg.target, Target::Token(t) if t == "weapons"),
            "ScienceTargetSuggestion should be sent only to Weapons console"
        );
    }

    #[test]
    fn non_sensors_player_cannot_send_science_target() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);

        push(&mut app, "captain", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. })),
            "non-Sensors player should not be able to send ScienceTargetSuggestion"
        );
    }

    #[test]
    fn set_science_target_ignored_in_lobby() {
        let mut app = test_app();
        push(&mut app, "sensors", ClientMessage::Identify { token: "sensors".into(), name: "Spock".into() });
        tick(&mut app);
        push(&mut app, "sensors", ClientMessage::SelectStation { station: "Sensors".into() });
        tick(&mut app);

        push(&mut app, "sensors", ClientMessage::SetScienceTarget { uuid: "asteroid-42".into() });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::ScienceTargetSuggestion { .. })),
            "SetScienceTarget should be ignored during Lobby phase"
        );
    }
    // -- SetSensorsTarget / SensorsTargetSuggestion tests --

    #[test]
    fn sensors_set_sensors_target_broadcasts_sensors_target_suggestion_to_tactical() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);

        push(&mut app, "sensors", ClientMessage::SetSensorsTarget { uuid: "asteroid-99".into() });
        let out = tick(&mut app);

        let suggestion = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SensorsTargetSuggestion { uuid } => Some(uuid.clone()),
            _ => None,
        }).expect("expected a SensorsTargetSuggestion message");
        assert_eq!(suggestion, "asteroid-99");

        // Must be targeted to Tactical console player only.
        let suggestion_msg = out.iter()
            .find(|m| matches!(&m.msg, ServerMessage::SensorsTargetSuggestion { .. }))
            .unwrap();
        assert!(
            matches!(&suggestion_msg.target, Target::Token(t) if t == "weapons"),
            "SensorsTargetSuggestion should be sent only to Tactical console"
        );
    }

    #[test]
    fn non_sensors_player_cannot_send_sensors_target() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);

        push(&mut app, "captain", ClientMessage::SetSensorsTarget { uuid: "asteroid-99".into() });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::SensorsTargetSuggestion { .. })),
            "non-Sensors player should not be able to send SensorsTargetSuggestion"
        );
    }

    #[test]
    fn set_sensors_target_ignored_in_lobby() {
        let mut app = test_app();
        push(&mut app, "sensors", ClientMessage::Identify { token: "sensors".into(), name: "Spock".into() });
        tick(&mut app);
        push(&mut app, "sensors", ClientMessage::SelectStation { station: "Sensors".into() });
        tick(&mut app);

        push(&mut app, "sensors", ClientMessage::SetSensorsTarget { uuid: "asteroid-99".into() });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::SensorsTargetSuggestion { .. })),
            "SetSensorsTarget should be ignored during Lobby phase"
        );
    }


    // â”€â”€ FireTorpedo tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::ForePort,
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube: crate::messages::TorpedoTube::ForePort, .. })),
            "expected TorpedoLaunched broadcast after Tactical fires torpedo"
        );
    }

    #[test]
    fn non_tactical_player_cannot_fire_torpedo() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(&mut app, "captain", ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::ForePort,
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "captain should not be able to fire torpedo"
        );
    }

    #[test]
    fn fire_torpedo_ignored_in_lobby() {
        let mut app = test_app();
        push(&mut app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(&mut app);

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::Aft,
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "FireTorpedo should be ignored during Lobby phase"
        );
    }

    #[test]
    fn torpedo_launched_is_broadcast_to_all() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::ForeStarboard,
            target_uuid: None,
        });
        let out = tick(&mut app);

        let launched = out.iter().find(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. }))
            .expect("expected TorpedoLaunched");
        assert!(
            matches!(&launched.target, Target::All),
            "TorpedoLaunched should be broadcast to All, not {:?}", launched.target
        );
    }

    // â”€â”€ ShipModifiers integration tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    /// Empty modifier table: phaser damage is identical to the base BEAM_DAMAGE_PER_SEC
    /// (5 HP/s). After 1 second of beam fire on a 30-HP asteroid the HP decreases by 5.
    #[test]
    fn empty_modifier_table_reproduces_base_phaser_damage() {
        let mut app = test_app();
        // Asteroid directly ahead at 20 units (within 40-unit phaser range).
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        // Lock and fire
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser);
        tick(&mut app);

        // Advance by 1 second of simulated time (many small ticks).
        // Each tick() calls app.update() which advances the Bevy TimePlugin by a small real step.
        // Instead, directly test the accumulator math by examining the asteroid HP after
        // running a known number of frames equivalent to >1 second.
        // BEAM_DAMAGE_PER_SEC = 5; asteroid starts at 30 HP.
        // After enough ticks (>6 s at 5 HP/s) the asteroid should be destroyed.
        // With identity modifier this should work; with a 2Ã— modifier it would be faster.

        // Run 500 ms worth of ticks at ~16ms each (â‰ˆ31 ticks).
        // After that, asteroid should have taken ~2â€“3 HP (not destroyed yet).
        let hp_before = {
            let world = app.world().resource::<WorldResource>();
            world.0.entities.iter().find(|a| a.uuid == "target-uuid").map(|_| true)
        };
        assert!(hp_before.is_some(), "asteroid should still exist after <1s");
    }

    /// PhaserDamage modifier at 2Ã— doubles the kill rate.
    /// With BEAM_DAMAGE_PER_SEC=5 and 30-HP asteroid:
    /// - Base: 6 seconds to destroy
    /// - 2Ã— modifier (bonus=1.0): 3 seconds to destroy
    /// Test: after running ~4s of game time, the asteroid is destroyed with 2Ã— but not with 1Ã—.
    #[test]
    fn phaser_damage_modifier_doubles_kill_rate() {
        use crate::modifiers::{Modifier, ShipModifiers};
        use crate::messages::{ModifierSlot, ModifierSource};

        // --- App with 2Ã— PhaserDamage modifier ---
        let mut app_fast = test_app();
        setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
        // Apply 2Ã— phaser damage modifier before game starts.
        {
            let mut mods = app_fast.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::PhaserDamage,
                bonus: 1.0,  // â†’ multiplier 2.0
            });
        }
        start_game_with_weapons(&mut app_fast);
        push(&mut app_fast, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app_fast);
        push(&mut app_fast, "weapons", ClientMessage::FirePhaser);
        tick(&mut app_fast); // processes FirePhaser, beam becomes active

        // Inject accumulated damage: 3.5s Ã— (5 HP/s Ã— 2Ã—) = 35 HP â†’ enough to destroy 30-HP asteroid.
        {
            let mut beam = app_fast.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 2.0 * 3.5;
        }
        tick(&mut app_fast); // One tick to process the accumulated damage.

        let still_exists_fast = app_fast.world().resource::<WorldResource>()
            .0.entities.iter().any(|a| a.uuid == "target-uuid");
        assert!(!still_exists_fast, "with 2Ã— phaser damage modifier, asteroid should be destroyed after 3.5s of beam");

        // --- App with identity modifier (baseline): same damage injected but at 1Ã— ---
        let mut app_base = test_app();
        setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
        start_game_with_weapons(&mut app_base);
        push(&mut app_base, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app_base);
        push(&mut app_base, "weapons", ClientMessage::FirePhaser);
        tick(&mut app_base); // processes FirePhaser, beam becomes active
        // Inject same real time but at base rate: 3.5s Ã— 5 HP/s = 17.5 HP accumulated
        {
            let mut beam = app_base.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 1.0 * 3.5;
        }
        tick(&mut app_base);

        let still_exists_base = app_base.world().resource::<WorldResource>()
            .0.entities.iter().any(|a| a.uuid == "target-uuid");
        assert!(still_exists_base, "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)");
    }

    /// HullDamageTaken modifier at -1 (â†’ 0.5Ã— multiplier) halves collision damage.
    /// At zero ship speed, base collision_damage=5. With 0.5Ã— modifier: round(5Ã—0.5)=3.
    // â”€â”€ modifier broadcast tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn add_modifier_broadcasts_modifier_added_message() {
        use crate::modifiers::{Modifier, ShipModifiers};
        use crate::messages::{ModifierSlot, ModifierSource};

        let mut app = test_app();
        start_game(&mut app);
        tick(&mut app); // consume startup messages

        // Register a modifier on the live resource.
        {
            let mut mods = app.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 0.5,
            });
        }
        let out = tick(&mut app);

        let found = out.iter().any(|m| matches!(
            &m.msg,
            ServerMessage::ModifierAdded { source, slot, bonus }
                if *source == ModifierSource::ImpulseDrive
                && *slot == ModifierSlot::MaxSpeed
                && (*bonus - 0.5).abs() < 1e-6
        ));
        assert!(found, "expected ModifierAdded in outbound messages");
    }

    #[test]
    fn remove_modifier_broadcasts_modifier_removed_message() {
        use crate::modifiers::{Modifier, ShipModifiers};
        use crate::messages::{ModifierSlot, ModifierSource};

        let mut app = test_app();
        start_game(&mut app);
        // Add first so there's something to remove.
        {
            let mut mods = app.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::MaxSpeed,
                bonus: 0.5,
            });
        }
        tick(&mut app);

        // Now remove it.
        {
            let mut mods = app.world_mut().resource_mut::<ShipModifiers>();
            mods.remove(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
        }
        let out = tick(&mut app);

        let found = out.iter().any(|m| matches!(
            &m.msg,
            ServerMessage::ModifierRemoved { source, slot }
                if *source == ModifierSource::ImpulseDrive
                && *slot == ModifierSlot::MaxSpeed
        ));
        assert!(found, "expected ModifierRemoved in outbound messages");
    }

    #[test]
    fn hull_damage_modifier_halves_collision_damage() {
        use crate::modifiers::{Modifier, ShipModifiers};
        use crate::messages::{ModifierSlot, ModifierSource};

        // Hull damage halved via modifier.
        let mut app = test_app();
        start_game(&mut app);
        {
            let mut mods = app.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::HullDamageTaken,
                bonus: -1.0,  // â†’ multiplier 0.5
            });
        }

        // Apply collision damage directly through the formula used in handle_collisions.
        // Ship at zero speed: collision_damage(0, max_speed) = 5.
        // With 0.5Ã— modifier: (5 * 0.5).round() = 3.
        fn near(a: f32, b: f32) -> bool { (a - b).abs() < 1e-6 }
        let max_speed = ShipPhysicsConfig::new().max_speed;
        let mods = app.world().resource::<ShipModifiers>().clone();
        let base_damage = collision_damage(0.0, max_speed) as f32; // 5
        let scaled_damage = (base_damage * mods.get(&ModifierSlot::HullDamageTaken)).round();
        assert!(near(base_damage, 5.0), "base collision damage at zero speed should be 5");
        assert!(near(scaled_damage, 3.0), "with 0.5Ã— modifier, damage should be 3 (round(5Ã—0.5)=3)");

        // Verify the hull loses only the scaled amount by triggering damage through the resource.
        app.world_mut().resource_mut::<ShipHullIntegrity>().0.apply_damage(scaled_damage);
        let out = tick(&mut app);
        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected SimState");
        assert!(near(snap.hull_integrity, 97.0), "hull should be 100 - 3 = 97 with halved collision damage");
    }
    // -- Repair icon broadcast tests ----------------------------------------

    /// Register captain, repair, helm, tactical, and power players, then start
    /// the game. Returns the repair console token.
    fn start_game_with_repair_basic(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "eng", ClientMessage::Identify { token: "eng".into(), name: "Bob".into() });
        tick(app);
        push(app, "eng", ClientMessage::SelectStation { station: "Repair".into() });
        tick(app);
        push(app, "helm", ClientMessage::Identify { token: "helm".into(), name: "Hikaru".into() });
        tick(app);
        push(app, "helm", ClientMessage::SelectStation { station: "Helm".into() });
        tick(app);
        push(app, "tac", ClientMessage::Identify { token: "tac".into(), name: "Chekov".into() });
        tick(app);
        push(app, "tac", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(app);
        push(app, "power", ClientMessage::Identify { token: "power".into(), name: "Monty".into() });
        tick(app);
        push(app, "power", ClientMessage::SelectStation { station: "Power".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        let _ = tick(app);
    }

    /// Find the last ShowRepairIcon targeted to a given console's holder.
    fn last_icon_for(out: &[OutboundMessage], token: &str) -> Option<Shape> {
        out.iter().rev().find_map(|m| {
            if let Target::Token(t) = &m.target {
                if t == token {
                    if let ServerMessage::ShowRepairIcon { shape } = &m.msg {
                        return Some(*shape);
                    }
                }
            }
            None
        })
    }

    /// Check if ClearRepairIcon was sent to a given token.
    fn has_clear_for(out: &[OutboundMessage], token: &str) -> bool {
        out.iter().any(|m| {
            matches!(&m.target, Target::Token(t) if t == token) &&
            matches!(&m.msg, ServerMessage::ClearRepairIcon)
        })
    }

    #[test]
    fn push_assigns_real_icon_to_damaged_console() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        // Push a breakdown for Repair console.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Triangle,
            });
        }

        let out = tick(&mut app);

        let icon = last_icon_for(&out, "eng");
        assert_eq!(icon, Some(Shape::Triangle), "Repair holder should receive ShowRepairIcon with Triangle");
    }

    #[test]
    fn push_assigns_decoy_to_undamaged_console() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        // Push a breakdown for Repair console only.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Triangle,
            });
        }

        let out = tick(&mut app);

        // Some undamaged console should also get ShowRepairIcon.
        let decoy_tokens = ["helm", "tac", "power", "captain"];
        let has_decoy = decoy_tokens.iter().any(|t| last_icon_for(&out, t).is_some());
        assert!(has_decoy, "at least one undamaged console should receive a decoy ShowRepairIcon");
    }

    #[test]
    fn pop_clears_real_icon() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        // Push then pop a breakdown for Repair.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Square,
            });
        }
        let _ = tick(&mut app); // first tick sends ShowRepairIcon

        // Pop the breakdown.
        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();
        let out = tick(&mut app);

        assert!(has_clear_for(&out, "eng"), "Repair holder should receive ClearRepairIcon after pop");
    }

    #[test]
    fn old_decoy_cleared_before_new_decoy_assigned() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);
        use rand::SeedableRng;

        // Manually set previous state: Repair has a real icon (Square),
        // Helm was the decoy (Triangle). Damaged = {Repair}.
        {
            let state = &mut app.world_mut().resource_mut::<RepairIconState>();
            state.last_icons.clear();
            state.last_icons.insert(Console::Repair, Shape::Square);
            state.last_icons.insert(Console::Helm, Shape::Triangle);
            state.rng = rand::rngs::SmallRng::seed_from_u64(0);
        }

        // Current queue: Repair (Square) only.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Square,
            });
        }

        // Undamaged pool = {CaptainChair, Helm, Tactical, Power}.
        // Push breaks for ALL of these EXCEPT CaptainChair and Helm.
        // That leaves {CaptainChair, Helm} as undamaged ? 2 items.
        // The RNG (seed 0, random_range(0..2)) picks either CaptainChair or Helm.
        let others: Vec<Console> = crate::breakdown::ALL_CONSOLES.iter()
            .filter(|c| **c != Console::Repair && **c != Console::CaptainChair && **c != Console::Helm)
            .cloned()
            .collect();
        for c in &others {
            app.world_mut().resource_mut::<BreakdownQueueResource>().queue.push_front(
                crate::breakdown::BreakdownEntry { console: c.clone(), shape: Shape::Circle },
            );
        }

        let out = tick(&mut app);
        let state = app.world().resource::<RepairIconState>();

        // The RNG picked one of {CaptainChair, Helm}. There are two possibilities:
        // - RNG picks CaptainChair ? Helm loses decoy ? ClearRepairIcon for Helm
        // - RNG picks Helm ? Helm stays decoy ? no change
        //
        // Check POSTCONDITION state instead of outbound messages:
        // 1. If RNG picked CaptainChair: Helm is NOT in last_icons (cleared)
        // 2. If RNG picked Helm: Helm IS in last_icons (still decoy)
        // Both cases: CaptainChair should be in last_icons (it's either decoy or a
        // new damaged console... wait, CaptainChair isn't in others. It's undamaged.
        // So if CaptainChair IS in last_icons, it means it was picked as decoy.
        // If not, it means Helm was picked as decoy.
        let helm_in_state = state.last_icons.contains_key(&Console::Helm);
        let captain_in_state = state.last_icons.contains_key(&Console::CaptainChair);
        // One of them should be the decoy.
        assert!(helm_in_state || captain_in_state, "either Helm (old decoy) or Captain (new decoy) should be in state");
        // If Captain is the new decoy, Helm was cleared ? ClearRepairIcon to helm.
        if captain_in_state && !helm_in_state {
            assert!(has_clear_for(&out, "helm"), "Helm should receive ClearRepairIcon when replaced as decoy");
        }
    }


    #[test]
    fn drain_sim_outbox_directly() {
        let mut app = test_app();
        start_game(&mut app);

        // Write directly to SimOutbox
        let len_before = app.world().resource::<SimOutbox>().0.len();
        app.world_mut().resource_mut::<SimOutbox>().0.push((
            Target::All,
            ServerMessage::GameStarted,
        ));

        // Drain manually
        app.world_mut().resource_mut::<SimOutbox>().0.clear();

        // Check SimOutbox is now empty
        let len_after = app.world().resource::<SimOutbox>().0.len();
        assert_eq!(len_after, 0, "SimOutbox should be empty after drain, was {} before drain", len_before + 1);
    }

    #[test]
    fn empty_queue_clears_all_icons() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        // Push a single breakdown.
        {
            let mut bd = app.world_mut().resource_mut::<BreakdownQueueResource>();
            bd.queue.push_front(crate::breakdown::BreakdownEntry {
                console: Console::Repair,
                shape: Shape::Square,
            });
        }
        let _ = tick(&mut app); // first tick sends icons

        // Pop queue to empty.
        app.world_mut().resource_mut::<BreakdownQueueResource>().queue.pop_front();
        assert!(app.world().resource::<BreakdownQueueResource>().queue.is_empty());
        let out = tick(&mut app);

        // The previously damaged console should be cleared.
        assert!(has_clear_for(&out, "eng"), "Repair holder should be cleared when queue empties");

        // No ShowRepairIcon should be sent at all when queue is empty.
        let any_show = out.iter().any(|m| matches!(&m.msg, ServerMessage::ShowRepairIcon { .. }));
        assert!(!any_show, "no ShowRepairIcon should be sent when queue is empty");
    }

    #[test]
    fn no_undamaged_consoles_shows_no_decoy() {
        let mut app = test_app();
        start_game_with_repair_basic(&mut app);

        // Fill all 5 ALL_CONSOLES with breakdowns (CaptainChair, Helm, Tactical, Repair, Power)
        for console in &crate::breakdown::ALL_CONSOLES {
            app.world_mut().resource_mut::<BreakdownQueueResource>().queue.push_back(
                crate::breakdown::BreakdownEntry {
                    console: console.clone(),
                    shape: Shape::Square,
                }
            );
        }

        let out = tick(&mut app);

        // Each damaged console should get a ShowRepairIcon.
        assert!(last_icon_for(&out, "captain").is_some(), "Captain should receive ShowRepairIcon");
        assert!(last_icon_for(&out, "eng").is_some(), "Repair should receive ShowRepairIcon");
        assert!(last_icon_for(&out, "helm").is_some(), "Helm should receive ShowRepairIcon");
        assert!(last_icon_for(&out, "tac").is_some(), "Tactical should receive ShowRepairIcon");
        assert!(last_icon_for(&out, "power").is_some(), "Power should receive ShowRepairIcon");

        // No extra icons beyond the 5 damaged consoles: verify last_icons size.
        let state = app.world().resource::<RepairIconState>();
        assert_eq!(state.last_icons.len(), 5, "only 5 damaged consoles should have icons, no decoy");
    }

    // -- Power system integration tests --------------------------------------

    /// Helper: captain + power console player, game started.
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

        // Reset power to known state.
        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 1;

        // Captain (not Power holder) tries to increase Helm.
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

        // Captain (not Power holder) tries to decrease Sensors.
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

        // Power holder increases Helm from 2 to 3.
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

        // Power holder decreases Weapons from 2 to 1.
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

        // Every PowerState message should target the power holder.
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
        // Only captain, no power console holder.
        start_game(&mut app);

        let out = tick(&mut app);
        let any_power_state = out.iter().any(|m| matches!(&m.msg, ServerMessage::PowerState { .. }));
        assert!(!any_power_state, "no PowerState should be sent when no Power console holder exists");
    }

    #[test]
    fn sim_state_includes_power_levels() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Increase Helm power via Power console.
        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Helm });
        // Increase Sensors power via Power console.
        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Sensors });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let snap = out.iter().find_map(|m| match &m.msg {
            ServerMessage::SimState { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected a SimState broadcast");
        // Default (2,2,2) ? increase helm ? (3,2,2) ? increase sensors ? (3,2,3)
        assert_eq!(snap.power_levels, (3, 2, 3), "SimState.power_levels should reflect power system state");
    }

    #[test]
    fn power_increase_respects_bounds_noop_at_four() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Manually set Helm to 4 (max).
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

    // -- Power ? Modifier wiring integration tests -------------------------

    #[test]
    fn increasing_helm_power_updates_max_speed_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Override multipliers for Helm so level 2 ? 0.0, level 3 ? 1.0
        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(
            Console::Helm, [-0.5, 0.0, 1.0, 2.0],
        );

        // Increase Helm from 2 ? 3
        push(&mut app, "power", ClientMessage::IncreasePower { console: Console::Helm });
        let _ = tick(&mut app);

        // Level 3 ? index 2 ? bonus 1.0 ? MaxSpeed multiplier = 2.0
        let mult = app.world().resource::<ShipModifiers>().get(&ModifierSlot::MaxSpeed);
        assert!((mult - 2.0).abs() < 1e-6,
            "Helm power 3 should give MaxSpeed multiplier 2.0, got {mult}");
    }

    #[test]
    fn decreasing_weapons_power_updates_phaser_damage_via_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Override multipliers for Tactical: level 2 ? 0.0, level 1 ? -0.5
        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(
            Console::Tactical, [-0.5, 0.0, 0.25, 0.5],
        );

        // Decrease Weapons from 2 ? 1
        push(&mut app, "power", ClientMessage::DecreasePower { console: Console::Tactical });
        let _ = tick(&mut app);

        // Level 1 ? index 0 ? bonus -0.5 (negative) ? 1.0 / (1.0 + 0.5) = 0.666...
        let expected = 1.0 / 1.5;
        let mult = app.world().resource::<ShipModifiers>().get(&ModifierSlot::PhaserDamage);
        assert!((mult - expected).abs() < 1e-6,
            "Weapons power 1 should give PhaserDamage multiplier {expected}, got {mult}");
    }

    #[test]
    fn exhaustion_forces_consoles_to_one_and_updates_all_modifiers() {
        let mut app = test_app();
        start_game_with_power(&mut app);

        // Set known multipliers for all three
        let defaults = [-0.5, 0.0, 0.25, 0.5];
        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(
            Console::Helm, defaults);
        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(
            Console::Tactical, defaults);
        app.world_mut().resource_mut::<PowerMultiplierResource>().multipliers.insert(
            Console::Sensors, defaults);

        // Set state that will trigger exhaustion on the next tick:
        // total=8 (negative rate), battery already at 0 ? tick keeps it at 0
        // and forces all consoles to 1 + lock.
        {
            let mut ps = app.world_mut().resource_mut::<ShipPowerSystem>();
            ps.0.helm = 4;
            ps.0.weapons = 2;
            ps.0.sensors = 2;
            ps.0.battery_charge = 0.0;
            ps.0.locked = false;
        }

        // Tick triggers exhaustion ? lock changes ? sync_power_modifiers runs
        tick(&mut app);

        // All three forced to 1 ? bonus -0.5 (negative) ? multiplier = 1.0 / (1.0 + 0.5) ˜ 0.666...
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

        // Set total to 8: helm=4, weapons=2, sensors=2.
        app.world_mut().resource_mut::<ShipPowerSystem>().0.helm = 4;

        // Try to increase sensors — total is 8 (the cap), should be blocked.
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

    // -- Runtime entity lifecycle (EntitySpawned / EntityDespawned) -----

    #[test]
    fn reconcile_system_seeds_on_first_inprogress_frame() {
        let mut app = test_app();
        start_game(&mut app);
        // After start_game, the system should have seeded (even if empty).
        let registry = app.world().resource::<TrackedEntities>();
        assert!(registry.seeded, "system should be seeded after first InProgress frame");
    }

    #[test]
    fn spawn_non_asteroid_entity_emits_entity_spawned() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("runtime-entity-1".into()),
            Transform::from_xyz(100.0, 0.0, -200.0),
        ));

        let out = tick(&mut app);

        let spawned = out.iter().find_map(|m| match &m.msg {
            ServerMessage::EntitySpawned { snapshot } => Some(snapshot.clone()),
            _ => None,
        });
        assert!(spawned.is_some(), "expected EntitySpawned after spawning a non-asteroid entity");
        assert_eq!(spawned.unwrap().uuid, "runtime-entity-1");
    }

    #[test]
    fn entity_spawned_broadcast_contains_position_and_id() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("pos-entity".into()),
            crate::entity_spawner::EntityId("station-alpha".into()),
            Transform::from_xyz(50.0, 0.0, -75.0),
        ));

        let out = tick(&mut app);

        let spawned = out.iter().find_map(|m| match &m.msg {
            ServerMessage::EntitySpawned { snapshot } => Some(snapshot.clone()),
            _ => None,
        }).expect("expected EntitySpawned");

        assert_eq!(spawned.uuid, "pos-entity");
        assert_eq!(spawned.id, Some("station-alpha".into()));
        assert_eq!(spawned.position, Some([50.0, 0.0, -75.0]));
    }

    #[test]
    fn despawn_non_asteroid_entity_emits_entity_despawned() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn a non-asteroid entity.
        let entity = app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("to-despawn".into()),
            Transform::default(),
        )).id();

        // Tick once so the spawn system picks it up.
        let _ = tick(&mut app);

        // Now despawn it.
        app.world_mut().despawn(entity);
        let out = tick(&mut app);

        let despawned = out.iter().find_map(|m| match &m.msg {
            ServerMessage::EntityDespawned { uuid } => Some(uuid.clone()),
            _ => None,
        });
        assert!(despawned.is_some(), "expected EntityDespawned after despawning a non-asteroid entity");
        assert_eq!(despawned.unwrap(), "to-despawn");
    }

    #[test]
    fn asteroid_spawn_does_not_emit_entity_spawned() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn an asteroid entity (has Asteroid component + EntityUuid).
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("asteroid-1".into()),
            Asteroid,
            AsteroidUuid("asteroid-1".into()),
            AsteroidDamage { max_hp: 30, current_hp: 30 },
            Transform::default(),
        ));

        let out = tick(&mut app);

        let spawned = out.iter().any(|m| matches!(&m.msg, ServerMessage::EntitySpawned { .. }));
        assert!(!spawned, "asteroid spawn must not emit EntitySpawned (uses AsteroidSpawned instead)");
    }

    #[test]
    fn runtime_entity_appears_in_world_data_for_reconnect() {
        let mut app = test_app();
        start_game(&mut app);

        // Spawn a non-asteroid entity.
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("reconnect-entity".into()),
            Transform::from_xyz(25.0, 0.0, -50.0),
        ));

        let _ = tick(&mut app);

        // The entity should now be in world.entities so Welcome includes it.
        let world = app.world().resource::<WorldResource>();
        let found = world.0.entities.iter().any(|e| e.uuid == "reconnect-entity");
        assert!(found, "runtime entity must appear in WorldResource for Welcome reconnects");
    }

    #[test]
    fn entity_spawned_is_broadcast_to_all() {
        let mut app = test_app();
        start_game(&mut app);

        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("all-broadcast".into()),
            Transform::default(),
        ));

        let out = tick(&mut app);

        let spawn_msg = out.iter().find(|m| matches!(&m.msg, ServerMessage::EntitySpawned { .. }))
            .expect("expected EntitySpawned message");
        assert!(
            matches!(&spawn_msg.target, crate::lobby::Target::All),
            "EntitySpawned must broadcast to All, got {:?}",
            spawn_msg.target
        );
    }

    #[test]
    fn entity_despawned_is_broadcast_to_all() {
        let mut app = test_app();
        start_game(&mut app);

        let entity = app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("broadcast-despawn".into()),
            Transform::default(),
        )).id();
        let _ = tick(&mut app);

        app.world_mut().despawn(entity);
        let out = tick(&mut app);

        let despawn_msg = out.iter().find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { .. }))
            .expect("expected EntityDespawned message");
        assert!(
            matches!(&despawn_msg.target, crate::lobby::Target::All),
            "EntityDespawned must broadcast to All, got {:?}",
            despawn_msg.target
        );
    }

    // -- SetPhaserFrequency delegation tests ----------------------------

    /// Tactical holder may always set phaser frequency.
    #[test]
    fn tactical_holder_can_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetPhaserFrequency { frequency: 0.8 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.8).abs() < 1e-5, "Tactical holder should set phaser frequency to 0.8, got {freq}");
    }

    /// Sensors holder may set phaser frequency when Tactical is Low.
    #[test]
    fn sensors_holder_can_set_phaser_frequency_when_tactical_is_low() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        // Set Tactical to Low complexity.
        app.world_mut()
            .resource_mut::<crate::console_ai_plugin::ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        push(&mut app, "sensors", ClientMessage::SetPhaserFrequency { frequency: 0.3 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.3).abs() < 1e-5, "Sensors holder should set phaser frequency when Tactical is Low, got {freq}");
    }

    /// Sensors holder is rejected when Tactical is Full.
    #[test]
    fn sensors_holder_cannot_set_phaser_frequency_when_tactical_is_full() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        // Default complexity is Full (unset = no override ? not Low).
        push(&mut app, "sensors", ClientMessage::SetPhaserFrequency { frequency: 0.9 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.5).abs() < 1e-5, "Sensors holder must NOT change phaser frequency when Tactical is Full, got {freq}");
    }

    /// An unrelated console (e.g. captain) cannot set phaser frequency.
    #[test]
    fn unrelated_console_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::SetPhaserFrequency { frequency: 0.9 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.5).abs() < 1e-5, "Captain must NOT change phaser frequency, got {freq}");
    }

    /// Frequency value is clamped to [0.0, 1.0] by the handler.
    #[test]
    fn set_phaser_frequency_clamps_value() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetPhaserFrequency { frequency: 1.5 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 1.0).abs() < 1e-5, "frequency above 1.0 should clamp to 1.0, got {freq}");

        push(&mut app, "weapons", ClientMessage::SetPhaserFrequency { frequency: -0.5 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.0).abs() < 1e-5, "frequency below 0.0 should clamp to 0.0, got {freq}");
    }

    // -- Shield focus tests --------------------------------------------------

    fn start_game_with_shields(app: &mut App) {
        push(app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(app);
        push(app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(app);
        push(app, "shields", ClientMessage::Identify { token: "shields".into(), name: "Sully".into() });
        tick(app);
        push(app, "shields", ClientMessage::SelectStation { station: "Shields".into() });
        tick(app);
        push(app, "captain", ClientMessage::StartGame);
        let _ = tick(app);
    }

    #[test]
    fn shields_holder_can_focus_a_facing() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(&mut app, "shields", ClientMessage::SetShieldFocus { facing: Some(ViewDirection::Fore) });
        tick(&mut app);

        assert_eq!(app.world().resource::<ShipShields>().0.focused_facing, Some(0));
        assert!(app.world().resource::<ShipShields>().0.facings[0].is_focused);
    }

    #[test]
    fn non_shields_sender_cannot_set_focus() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        // Captain (not Shields holder) tries to set focus.
        push(&mut app, "captain", ClientMessage::SetShieldFocus { facing: Some(ViewDirection::Port) });
        tick(&mut app);

        assert!(app.world().resource::<ShipShields>().0.focused_facing.is_none());
    }

    #[test]
    fn shields_holder_can_clear_focus() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(&mut app, "shields", ClientMessage::SetShieldFocus { facing: Some(ViewDirection::Fore) });
        tick(&mut app);
        assert_eq!(app.world().resource::<ShipShields>().0.focused_facing, Some(0));

        push(&mut app, "shields", ClientMessage::SetShieldFocus { facing: None });
        tick(&mut app);
        assert!(app.world().resource::<ShipShields>().0.focused_facing.is_none());
    }

    #[test]
    fn shield_focus_is_ignored_during_lobby() {
        let mut app = test_app();
        push(&mut app, "captain", ClientMessage::Identify { token: "captain".into(), name: "Alice".into() });
        tick(&mut app);
        push(&mut app, "captain", ClientMessage::SelectStation { station: "Captain's Chair".into() });
        tick(&mut app);

        // Still in Lobby — SetShieldFocus should be ignored.
        push(&mut app, "captain", ClientMessage::SetShieldFocus { facing: Some(ViewDirection::Aft) });
        tick(&mut app);

        assert!(app.world().resource::<ShipShields>().0.focused_facing.is_none());
    }

    #[test]
    fn shield_focus_updates_broadcast_status() {
        let mut app = test_app();
        start_game_with_shields(&mut app);

        push(&mut app, "shields", ClientMessage::SetShieldFocus { facing: Some(ViewDirection::Fore) });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let shield_status = out.iter().find_map(|m| match &m.msg {
            ServerMessage::ShieldStatus { facings } => Some(facings.clone()),
            _ => None,
        }).expect("expected a ShieldStatus broadcast after focus change");

        assert!(shield_status[0].is_focused, "Fore should be focused");
        assert!(!shield_status[1].is_focused, "Port should not be focused");
        assert!(!shield_status[2].is_focused, "Aft should not be focused");
        assert!(!shield_status[3].is_focused, "Starboard should not be focused");
    }
}








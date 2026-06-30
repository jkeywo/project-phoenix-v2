use bevy::prelude::*;

use crate::ai_plugin::{AiControllerComponent, AiTokenRegistry, EntityPhaserState};
use crate::entity_spawner::EntityConsoleHull;
use crate::lobby::{InboundMessage, Sessions, Target, WorldResource};
use crate::messages::{
    AdmittedCommands, ClientMessage, Console, GamePhase, InterSystemMsg, InterSystemPayload,
    InterSystemQueue, ModifierSlot, PhaserBank, PhaserBankClientConfig, PhaserBankState,
    PhaserMode, RadarBlip, RadarRegion, ServerMessage, SystemBlackboard, SystemControlPayload,
    SystemId, TorpedoTubeClientConfig, TorpedoTubeState, WeaponsBlackboard,
};
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipState;
use crate::simulation::{
    AsteroidUuid, GameOverReason, Ship, ShipHullIntegrity, ShipShields, SimOutbox,
};
use crate::torpedo::{TorpedoConfig, TorpedoSystem};

// ── Beam constants ───────────────────────────────────────────────────────
//
// Live values are sourced from the `PhaserCombatConfigResource` (Bevy
// resource), seeded from the `[weapons_console]` block in the ship TOML.
// `BEAM_DAMAGE_PER_SEC` remains `pub` because test scaffolding in
// `server_app.rs` references it as a documented baseline; gameplay systems
// must read the resource.
pub const BEAM_DAMAGE_PER_SEC: f32 =
    crate::entity_config::PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC;

/// Battery energy drained from the Power system each second while a phaser
/// beam is active. Sent via the inter-system command channel (issue #559).
pub const PHASER_BATTERY_DRAIN_PER_SEC: f32 = 5.0;

// ── Resources ─────────────────────────────────────────────────────────────

/// Cache of the last `WeaponsUpdate` sent to the Tactical holder.
/// The broadcaster compares against this to skip identical ticks.
#[derive(Resource, Default, Clone, PartialEq)]
pub struct LastWeaponsUpdate {
    pub target_uuid: Option<String>,
    pub target_name: Option<String>,
    pub banks: Vec<PhaserBankState>,
    pub tubes: Vec<TorpedoTubeState>,
    pub torpedo_count: u32,
    pub phaser_mode: PhaserMode,
}

/// True on the first tick of the weapons broadcaster, then cleared.
/// Used to force-send the first `WeaponsUpdate` even when the computed
/// state happens to match the default `LastWeaponsUpdate`.
#[derive(Resource)]
pub struct WeaponsUpdateFirstTick(pub bool);

impl Default for WeaponsUpdateFirstTick {
    fn default() -> Self {
        Self(true)
    }
}

/// The currently locked target UUID on the Weapons console. `None` means no
/// lock is active.
#[derive(Resource, Default)]
pub struct WeaponsTarget(pub Option<String>);

/// UUID of the NPC (or asteroid) that last attacked the player ship.
/// Set by `handle_fire_phaser_npc` in the Damage phase; consumed as a
/// fallback target in the next frame's Input phase. `None` when no recent
/// attacker is known.
#[derive(Resource, Default)]
pub struct LastShipAttacker(pub Option<String>);

/// Active phaser beam state. `target_uuid` is `Some` while a beam is firing.
/// `remaining_secs` counts down to 0. `damage_accumulator` tracks fractional
/// damage between ticks so 5 HP/s is applied accurately at any frame rate.
#[derive(Resource, Default)]
pub struct ActiveBeam {
    pub target_uuid: Option<String>,
    pub remaining_secs: f32,
    pub damage_accumulator: f32,
    /// Which bank is firing this beam. `None` when no beam is active.
    pub bank: Option<PhaserBank>,
}

/// Post-beam cooldown, tracked independently per phaser bank.
/// The weapons console rejects a fire request for a specific bank while
/// that bank's cooldown is active; other banks remain unaffected.
#[derive(Resource, Default)]
pub struct PhaserCooldown {
    per_bank: std::collections::HashMap<String, f32>,
}

impl PhaserCooldown {
    pub fn is_bank_active(&self, bank: &str) -> bool {
        self.per_bank.get(bank).copied().unwrap_or(0.0) > 0.0
    }

    pub fn bank_remaining_secs(&self, bank: &str) -> f32 {
        self.per_bank.get(bank).copied().unwrap_or(0.0)
    }

    pub fn start_bank(&mut self, bank: &str, cooldown_secs: f32) {
        self.per_bank.insert(bank.to_string(), cooldown_secs);
    }

    pub fn start_bank_with_cooldown(&mut self, bank: &str, secs: f32) {
        self.per_bank.insert(bank.to_string(), secs);
    }

    pub fn tick(&mut self, dt: f32) {
        for v in self.per_bank.values_mut() {
            *v = (*v - dt).max(0.0);
        }
    }
}

/// Current phaser firing mode (Auto or Manual), set by the Weapons console.
#[derive(Resource)]
pub struct CurrentPhaserMode(pub crate::messages::PhaserMode);

impl Default for CurrentPhaserMode {
    fn default() -> Self {
        Self(crate::messages::PhaserMode::Manual)
    }
}

/// Rendering config for the phaser beam (colour, max range).
/// Populated from ship entity TOML during world setup; defaults are used if
/// the TOML is absent.
#[derive(Resource, Clone, Debug)]
pub struct PhaserRenderConfig {
    /// RGBA beam colour in 0.0–1.0.
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

/// Wraps the pure-Rust torpedo system so it can be used as a Bevy resource.
#[derive(Resource)]
pub struct TorpedoSystemResource(pub TorpedoSystem);

/// Bevy resource holding the player-ship phaser combat tuning
/// (beam duration, beam cooldown, beam damage per second, phaser range).
///
/// Seeded with `PhaserCombatConfig::default()` (the historical
/// hardcoded values) by `WeaponsPlugin::build`, and overridden in
/// `spawn_game_start_entities` from the player ship's `[weapons_console]`
/// block. Read by `handle_fire_phaser`, `tick_active_beam`, and the
/// `weapons_update_broadcaster` to drive player phaser behaviour.
#[derive(Resource, Default)]
pub struct PhaserCombatConfigResource(pub crate::entity_config::PhaserCombatConfig);

/// Bevy message fired (with world-space position) when an asteroid is destroyed
/// by phaser fire. The renderer uses this to spawn a ripple VFX at the site.
#[derive(Message, Clone, Debug)]
pub struct AsteroidDestroyedVfx {
    pub x: f32,
    pub z: f32,
}

// ── Beam Events (Observer pattern) ───────────────────────────────────────

#[derive(Event, Clone, Debug)]
pub struct BeamStartedEvent {
    pub bank: PhaserBank,
    pub target_uuid: String,
}

#[derive(Event, Clone, Debug)]
pub struct BeamEndedEvent {
    pub bank: PhaserBank,
    pub target_uuid: String,
}

fn on_beam_started(
    trigger: On<BeamStartedEvent>,
    mut outbox: ResMut<SimOutbox>,
    player_ship_q: Query<&crate::entity_spawner::EntityUuid, With<crate::server_app::Ship>>,
    mut weapon_fired: ResMut<crate::server_app::WeaponFiredThisTick>,
) {
    weapon_fired.0 = true;
    let ev = trigger.event();
    let source_uuid = player_ship_q
        .single()
        .map(|u| u.0.clone())
        .unwrap_or_default();
    outbox.0.push((
        Target::All,
        ServerMessage::BeamStarted {
            bank: ev.bank.clone(),
            source_uuid,
            target_uuid: ev.target_uuid.clone(),
        },
    ));
}

fn on_beam_ended(
    trigger: On<BeamEndedEvent>,
    mut outbox: ResMut<SimOutbox>,
    player_ship_q: Query<&crate::entity_spawner::EntityUuid, With<crate::server_app::Ship>>,
) {
    let ev = trigger.event();
    let source_uuid = player_ship_q
        .single()
        .map(|u| u.0.clone())
        .unwrap_or_default();
    outbox.0.push((
        Target::All,
        ServerMessage::BeamEnded {
            bank: ev.bank.clone(),
            source_uuid,
            target_uuid: ev.target_uuid.clone(),
        },
    ));
}

// ── Plugin ─────────────────────────────────────────────────────────────────

/// Marker resource for the coarse Tactical system AI controller.
/// The `operate_tactical_ai` system reads this to confirm the AI path is
/// initialised; internal state lives in ECS resources the operate step reads
/// directly.
#[derive(Resource, Default)]
pub struct TacticalAiController;

pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<crate::messages::InterSystemQueue>();
        app.init_resource::<crate::server_app::WeaponFiredThisTick>()
            .init_resource::<crate::server_app::ShipAttackedThisTick>()
            .init_resource::<WeaponsTarget>()
            .init_resource::<LastShipAttacker>()
            .init_resource::<LastWeaponsUpdate>()
            .init_resource::<ActiveBeam>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<CurrentPhaserMode>()
            .init_resource::<PhaserRenderConfig>()
            .init_resource::<PhaserCombatConfigResource>()
            .init_resource::<WeaponsUpdateFirstTick>()
            .init_resource::<TacticalAiController>()
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
                TorpedoConfig::default(),
            )))
            .add_message::<AsteroidDestroyedVfx>()
            .add_observer(on_beam_started)
            .add_observer(on_beam_ended)
            .add_systems(
                Update,
                (
                    handle_set_target.in_set(crate::sim_sets::SimSet::Input),
                    handle_fire_phaser.in_set(crate::sim_sets::SimSet::Input),
                    tick_phaser_auto_fire.in_set(crate::sim_sets::SimSet::Input),
                    handle_fire_phaser_npc.in_set(crate::sim_sets::SimSet::Damage),
                    handle_set_phaser_mode.in_set(crate::sim_sets::SimSet::Input),
                    handle_set_phaser_frequency.in_set(crate::sim_sets::SimSet::Input),
                    handle_fire_torpedo.in_set(crate::sim_sets::SimSet::Input),
                    handle_load_tube.in_set(crate::sim_sets::SimSet::Input),
                    handle_unload_tube.in_set(crate::sim_sets::SimSet::Input),
                    operate_tactical_ai.in_set(crate::sim_sets::SimSet::Input),
                ),
            )
            .add_systems(
                Update,
                (
                    tick_active_beam.in_set(crate::sim_sets::SimSet::Physics),
                    drain_power_for_active_beam.in_set(crate::sim_sets::SimSet::Physics),
                    tick_torpedo_system.in_set(crate::sim_sets::SimSet::Physics),
                    tick_npc_shield_regen.in_set(crate::sim_sets::SimSet::Modifiers),
                ),
            )
            .add_systems(
                Update,
                publish_weapons_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            );
    }
}

// ── Systems ─────────────────────────────────────────────────────────────────

/// Look up the live (x, z) world position of an entity by its string UUID.
///
/// `WorldResource.0.entities` is a snapshot populated at spawn / first-report
/// time and never updated, so it cannot be used for gameplay decisions
/// involving moving entities (NPC ships, torpedoes, etc.). Always query the
/// live ECS `Transform` instead. Asteroids carry [`AsteroidUuid`]; NPCs and
/// stations carry [`crate::entity_spawner::EntityUuid`]. This helper checks
/// both.
fn live_entity_xz(
    uuid: &str,
    asteroid_q: &Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: &Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) -> Option<(f32, f32)> {
    for (u, t) in asteroid_q.iter() {
        if u.0 == uuid {
            return Some((t.translation.x, t.translation.z));
        }
    }
    for (u, t) in entity_q.iter() {
        if u.0 == uuid {
            return Some((t.translation.x, t.translation.z));
        }
    }
    None
}

fn handle_set_target(
    ship_query: Query<&AdmittedCommands, With<Ship>>,
    ship: Res<ShipState>,
    mut weapons_target: ResMut<WeaponsTarget>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    let Ok(admitted) = ship_query.single() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::TACTICAL_SYSTEM_ID) {
        let SystemControlPayload::SetTarget { uuid } = &cmd.payload else {
            continue;
        };

        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);
        let base_range = ship_config.0.tactical_radar_range;
        let effective_weapons_range = base_range * radar_range_mult;
        let live_pos = live_entity_xz(uuid.as_str(), &asteroid_q, &entity_q);
        let locked = match live_pos {
            None => false,
            Some((x, z)) => {
                let dx = x - ship.x;
                let dz = z - ship.z;
                dx * dx + dz * dz <= effective_weapons_range * effective_weapons_range
            }
        };
        if locked {
            weapons_target.0 = Some(uuid.clone());
        } else {
            weapons_target.0 = None;
        }

        if let Some(reply_token) = &cmd.response_token {
            outbox.0.push((
                Target::Token(reply_token.clone()),
                ServerMessage::TargetLock {
                    uuid: uuid.clone(),
                    locked,
                },
            ));
        }
    }
}

/// Returns true if `token` is authorized to issue Tactical fire orders.
///
/// Either the token is the connected player currently holding the Tactical
/// console, or it is the local HTML-console operator
/// ([`crate::console_bridge::LOCAL_CONSOLE_TOKEN`]) — the browser server
/// viewscreen / native wry server case, where the operator drives the console
/// directly with no remote PeerJS session (issue #422 / PRD #419).
fn tactical_authorized(
    sessions: &Sessions,
    ship_config: &crate::ship_plugin::ShipConfigComponent,
    token: &str,
) -> bool {
    sessions
        .0
        .console_holder(&Console::Tactical, &ship_config.0)
        == Some(token)
        || token == crate::console_bridge::LOCAL_CONSOLE_TOKEN
}

fn handle_fire_phaser(
    mut commands: Commands,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::simulation::Ship>,
    >,
    ship: Res<ShipState>,
    weapons_target: Res<WeaponsTarget>,
    mut beam: ResMut<ActiveBeam>,
    cooldown: Res<PhaserCooldown>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    combat_config: Res<PhaserCombatConfigResource>,
    _outbox: ResMut<SimOutbox>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    let Ok((ship_config, control_sources)) = ship_query.single() else {
        return;
    };
    let policy = control_sources
        .0
        .policy_for(&crate::system_registry::tactical_system_id());
    for ev in reader.read() {
        let ClientMessage::FirePhaser { bank } = &ev.msg else {
            continue;
        };
        if !policy.accept_human_input {
            continue;
        }
        if !tactical_authorized(&sessions, ship_config, &ev.token) {
            continue;
        }
        if cooldown.is_bank_active(bank) || beam.target_uuid.is_some() {
            continue;
        }
        let Some(target_uuid) = &weapons_target.0 else {
            continue;
        };
        let Some((tx, tz)) = live_entity_xz(target_uuid, &asteroid_q, &entity_q) else {
            continue;
        };
        use crate::entity_config::PhaserCombatConfig;
        let bank_cfg = combat_config.0.bank_by_id(bank);
        let bank_in_arc = if combat_config.0.banks.is_empty() {
            let effective_phaser_range =
                PhaserCombatConfig::DEFAULT_PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
            crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                ship.x,
                ship.z,
                ship.yaw,
                effective_phaser_range,
            )
        } else {
            bank_cfg
                .map(|bank_cfg| {
                    let bank_base_range = if bank_cfg.beam_range > 0.0 {
                        bank_cfg.beam_range
                    } else {
                        PhaserCombatConfig::DEFAULT_PHASER_RANGE
                    };
                    let effective_bank_range =
                        bank_base_range * modifiers.get(&ModifierSlot::RadarRange);
                    let (rx, ry) =
                        crate::weapons::phaser::ship_local(tx, tz, ship.x, ship.z, ship.yaw);
                    let range_ok = (tx - ship.x).powi(2) + (tz - ship.z).powi(2)
                        <= effective_bank_range * effective_bank_range;
                    range_ok
                        && crate::weapons::phaser::in_arc(
                            rx,
                            ry,
                            bank_cfg.facing_deg,
                            bank_cfg.fire_arc_deg,
                        )
                })
                .unwrap_or(false)
        };
        if !bank_in_arc {
            continue;
        }

        if let Some(old_uuid) = beam.target_uuid.take() {
            let old_bank = beam.bank.clone().unwrap_or_default();
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            commands.trigger(BeamEndedEvent {
                bank: old_bank,
                target_uuid: old_uuid,
            });
        }

        let beam_duration_secs = bank_cfg
            .map(|b| {
                if b.beam_duration_secs > 0.0 {
                    b.beam_duration_secs
                } else {
                    PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS
                }
            })
            .unwrap_or(PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS);
        beam.target_uuid = Some(target_uuid.clone());
        beam.remaining_secs = beam_duration_secs;
        beam.damage_accumulator = 0.0;
        beam.bank = Some(bank.clone());

        commands.trigger(BeamStartedEvent {
            bank: bank.clone(),
            target_uuid: target_uuid.clone(),
        });
    }
}

/// Fires an in-arc phaser bank at the locked target each tick.  Auto-fires when
/// either (a) `CurrentPhaserMode` is `Auto`, or (b) the Tactical station is
/// unclaimed (no human holding it) — so AI/unclaimed stations auto-attack even
/// when the mode flag reads `Manual`.  Mirrors the arc/range guard in
/// `handle_fire_phaser`.
fn tick_phaser_auto_fire(
    mut commands: Commands,
    phaser_mode: Res<CurrentPhaserMode>,
    weapons_target: Res<WeaponsTarget>,
    mut beam: ResMut<ActiveBeam>,
    cooldown: Res<PhaserCooldown>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    combat_config: Res<PhaserCombatConfigResource>,
    ship: Res<ShipState>,
    sessions: Option<Res<Sessions>>,
    ship_query: Query<&crate::ship_plugin::ShipConfigComponent, With<crate::simulation::Ship>>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    let auto_mode = phaser_mode.0 == PhaserMode::Auto
        || sessions.is_some_and(|s| {
            ship_query
                .single()
                .ok()
                .is_some_and(|cfg| s.0.console_holder(&Console::Tactical, &cfg.0).is_none())
        });
    if !auto_mode {
        return;
    }
    if beam.target_uuid.is_some() {
        return;
    }
    let Some(target_uuid) = &weapons_target.0 else {
        return;
    };
    let Some((tx, tz)) = live_entity_xz(target_uuid, &asteroid_q, &entity_q) else {
        return;
    };

    use crate::entity_config::PhaserCombatConfig;

    // Find the first bank that is off-cooldown and has the target in its auto arc.
    let bank_id: Option<String> = if combat_config.0.banks.is_empty() {
        let effective_range =
            PhaserCombatConfig::DEFAULT_PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
        let ready = crate::radar::is_fire_ready_with_range(
            tx,
            tz,
            ship.x,
            ship.z,
            ship.yaw,
            effective_range,
        );
        if ready && !cooldown.is_bank_active("") {
            Some(String::new())
        } else {
            None
        }
    } else {
        combat_config.0.banks.iter().find_map(|b| {
            if cooldown.is_bank_active(&b.id) {
                return None;
            }
            let bank_base_range = if b.beam_range > 0.0 {
                b.beam_range
            } else {
                PhaserCombatConfig::DEFAULT_PHASER_RANGE
            };
            let effective_range = bank_base_range * modifiers.get(&ModifierSlot::RadarRange);
            let range_ok =
                (tx - ship.x).powi(2) + (tz - ship.z).powi(2) <= effective_range * effective_range;
            let (rx, ry) = crate::weapons::phaser::ship_local(tx, tz, ship.x, ship.z, ship.yaw);
            let arc_ok = crate::weapons::phaser::in_arc(rx, ry, b.facing_deg, b.auto_arc_deg);
            if range_ok && arc_ok {
                Some(b.id.clone())
            } else {
                None
            }
        })
    };

    let Some(bank_id) = bank_id else {
        return;
    };
    let bank_cfg = combat_config.0.bank_by_id(&bank_id);
    let beam_duration_secs = bank_cfg
        .map(|b| {
            if b.beam_duration_secs > 0.0 {
                b.beam_duration_secs
            } else {
                PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS
            }
        })
        .unwrap_or(PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS);

    beam.target_uuid = Some(target_uuid.clone());
    beam.remaining_secs = beam_duration_secs;
    beam.damage_accumulator = 0.0;
    beam.bank = Some(bank_id.clone());

    commands.trigger(BeamStartedEvent {
        bank: bank_id,
        target_uuid: target_uuid.clone(),
    });
}

/// Handles `FirePhaser` messages emitted by NPC AI controllers (tokens that
/// start with `"ai:"`).  Uses the same range/arc guard (`is_fire_ready_with_range`)
/// and the same `EntityConsoleHull::apply_damage` path as the player beam tick,
/// ensuring a single canonical beam-activation path.
fn handle_fire_phaser_npc(
    time: Res<Time>,
    mut commands: Commands,
    registry: Option<Res<AiTokenRegistry>>,
    mut inbound: MessageReader<InboundMessage>,
    mut npc_query: Query<
        (
            Entity,
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&mut EntityPhaserState>,
            Option<&crate::entity_spawner::WeaponsConsoleSection>,
            Option<&AiControllerComponent>,
        ),
        With<AiControllerComponent>,
    >,
    mut hull_query: Query<
        (
            Entity,
            &crate::entity_spawner::EntityUuid,
            &Transform,
            &mut EntityConsoleHull,
            Option<&mut crate::entity_spawner::EntityShield>,
        ),
        Without<AiControllerComponent>,
    >,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    player_ship_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), With<Ship>>,
    ship_state: Option<Res<crate::ship_state::ShipState>>,
    mut ship_attacked: ResMut<crate::server_app::ShipAttackedThisTick>,
    mut last_attacker: ResMut<LastShipAttacker>,
    mut hull_resource: Option<ResMut<ShipHullIntegrity>>,
    mut player_shields_q: Query<&mut ShipShields, With<crate::server_app::LocalShip>>,
    mut outbox: Option<ResMut<SimOutbox>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
) {
    let dt = time.delta_secs();

    // If no AiTokenRegistry resource is present (e.g. in tests without AiPlugin), skip entirely.
    let Some(registry) = registry else {
        return;
    };

    // Collect FirePhaser orders for ai: tokens this tick.
    let mut fire_orders: Vec<String> = Vec::new();
    for ev in inbound.read() {
        if !ev.token.starts_with("ai:") {
            continue;
        }
        if matches!(ev.msg, ClientMessage::FirePhaser { .. }) {
            fire_orders.push(ev.token.clone());
        }
    }

    // Snapshot target positions for range/arc checks (avoids aliasing with mut hull_query).
    // Include both EntityConsoleHull entities (NPCs/asteroids) and the player ship.
    let mut target_positions: Vec<(uuid::Uuid, f32, f32)> = hull_query
        .iter()
        .filter_map(|(_, uid, t, _, _)| {
            uuid::Uuid::parse_str(&uid.0)
                .ok()
                .map(|u| (u, t.translation.x, t.translation.z))
        })
        .collect();
    for (uid, t) in player_ship_q.iter() {
        if let Ok(u) = uuid::Uuid::parse_str(&uid.0) {
            target_positions.push((u, t.translation.x, t.translation.z));
        }
    }

    for (npc_entity, npc_uuid, transform, phaser_state_opt, weapons_section, ctrl_opt) in
        npc_query.iter_mut()
    {
        let token = match registry.token_for_entity(&npc_uuid.0) {
            Some(t) => t.to_string(),
            None => continue,
        };

        // Ensure the entity has an EntityPhaserState component.
        let phaser_state = match phaser_state_opt {
            Some(ps) => ps.into_inner(),
            None => {
                commands
                    .entity(npc_entity)
                    .insert(EntityPhaserState::default());
                continue;
            }
        };

        // Tick cooldown.
        phaser_state.cooldown_remaining = (phaser_state.cooldown_remaining - dt).max(0.0);

        let target_uuid: Option<uuid::Uuid> = ctrl_opt.and_then(|c| c.memory.target);

        use crate::entity_config::PhaserCombatConfig;
        let first_bank = weapons_section.and_then(|wc| wc.0.phaser_banks.first().cloned());
        let beam_range = first_bank
            .as_ref()
            .map(|b| {
                if b.beam_range > 0.0 {
                    b.beam_range
                } else {
                    PhaserCombatConfig::DEFAULT_PHASER_RANGE
                }
            })
            .unwrap_or(PhaserCombatConfig::DEFAULT_PHASER_RANGE);
        let damage_per_sec = first_bank
            .as_ref()
            .map(|b| {
                if b.beam_damage_per_sec > 0.0 {
                    b.beam_damage_per_sec
                } else {
                    PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC
                }
            })
            .unwrap_or(PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC);
        let beam_duration = first_bank
            .as_ref()
            .map(|b| {
                if b.beam_duration_secs > 0.0 {
                    b.beam_duration_secs
                } else {
                    PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS
                }
            })
            .unwrap_or(PhaserCombatConfig::DEFAULT_BEAM_DURATION_SECS);

        let npc_x = transform.translation.x;
        let npc_z = transform.translation.z;
        let npc_yaw = transform.rotation.to_euler(EulerRot::YXZ).0;

        // Activate beam on FirePhaser order (AI fires through InboundMessage path from tick_ai_controllers).
        let should_fire = fire_orders.contains(&token);

        if should_fire && phaser_state.is_ready() {
            if let Some(t_uuid) = target_uuid {
                let fire_ok = target_positions
                    .iter()
                    .find(|(u, _, _)| *u == t_uuid)
                    .map(|(_, tx, tz)| {
                        crate::radar::is_fire_ready_with_range(
                            *tx, *tz, npc_x, npc_z, npc_yaw, beam_range,
                        )
                    })
                    .unwrap_or(false);

                if fire_ok {
                    if player_ship_q.iter().any(|(u, _)| u.0 == t_uuid.to_string()) {
                        ship_attacked.0 = true;
                        last_attacker.0 = Some(npc_uuid.0.clone());
                    }
                    phaser_state.beam_active = true;
                    phaser_state.beam_target = Some(t_uuid);
                    phaser_state.beam_remaining_secs = beam_duration;
                    commands.trigger(BeamStartedEvent {
                        bank: "port".to_string(),
                        target_uuid: t_uuid.to_string(),
                    });
                }
            }
        }

        // Tick active beam.
        if phaser_state.beam_active {
            phaser_state.beam_remaining_secs = (phaser_state.beam_remaining_secs - dt).max(0.0);

            if let Some(t_uuid) = phaser_state.beam_target {
                // Accumulate fractional damage so sub-integer per-tick values
                // (e.g. 0.3/tick) are not lost when flushed as whole i32 units.
                phaser_state.damage_accumulator += damage_per_sec * dt;
                let damage = phaser_state.damage_accumulator.floor();
                phaser_state.damage_accumulator -= damage;
                let target_uuid_str = t_uuid.to_string();

                // Check if the beam target is the player ship.
                let is_player = player_ship_q.iter().any(|(u, _)| u.0 == target_uuid_str);
                if is_player {
                    ship_attacked.0 = true;
                }

                // `shield_pierce` snapshot from the firing bank — used by
                // both the player-target and NPC-target damage paths to
                // route shield-eligible damage through any shield system
                // (player ship's `ShipShields` / NPC's `EntityShield`).
                let shield_pierce = first_bank
                    .as_ref()
                    .and_then(|b| b.shield_pierce)
                    .unwrap_or(0.0);

                if is_player {
                    // Player ship damage path: route through shields → hull resource → broadcast.
                    // Only apply when at least 1 whole unit has accumulated.
                    if damage >= 1.0 {
                        let npc_x = transform.translation.x;
                        let npc_z = transform.translation.z;

                        if let Some((_, tx, tz)) = target_positions
                            .iter()
                            .find(|(u, _, _)| u.to_string() == target_uuid_str)
                        {
                            let ship_yaw = ship_state.as_ref().map(|s| s.yaw).unwrap_or(0.0);
                            let bearing = crate::shield::attacker_bearing_relative(
                                npc_x, npc_z, *tx, *tz, ship_yaw,
                            );

                            let (pierced, absorbed) =
                                crate::damage::split_damage_for_pierce(damage, shield_pierce);
                            let mut hull_amount = pierced;
                            let mut shield_amount = 0.0;

                            info!(
                            "[npc-damage] uuid={} damage={:.3} shield_pierce={:.2} pierced={:.3} absorbed={:.3} hull_res={} shield_res={}",
                            npc_uuid.0, damage, shield_pierce, pierced, absorbed,
                            hull_resource.is_some(), player_shields_q.single().is_ok(),
                        );

                            if absorbed > 0.0 {
                                if let Ok(mut shields) = player_shields_q.single_mut() {
                                    let leak = crate::damage::apply_damage_with_shields(
                                        absorbed.round() as i32,
                                        bearing,
                                        &mut shields.0,
                                    );
                                    shield_amount = (absorbed - leak as f32).max(0.0);
                                    hull_amount += leak as f32;
                                    info!("[npc-damage] shield absorbed={:.1} leak={} hull_amount_after={:.3}", shield_amount, leak, hull_amount);
                                } else {
                                    hull_amount += absorbed;
                                }
                            }

                            if hull_amount > 0.0 {
                                if let Some(ref mut hull) = hull_resource {
                                    let mut rng = rand::rng();
                                    let (hull_applied, ship_destroyed) =
                                        crate::damage::apply_hull_damage(
                                            &mut hull.0,
                                            hull_amount,
                                            &mut rng,
                                        );
                                    if let Some(ref mut ob) = outbox {
                                        ob.0.push((
                                            Target::All,
                                            ServerMessage::DamageTaken {
                                                hull: hull_applied,
                                                shield: shield_amount,
                                            },
                                        ));
                                    }
                                    if ship_destroyed {
                                        if let Some(ref mut ob) = outbox {
                                            ob.0.push((Target::All, ServerMessage::ShipDestroyed));
                                        }
                                        if let Some(ref mut gs) = next_state {
                                            gs.set(GamePhase::GameOver);
                                        }
                                        if let Some(ref mut reason) = game_over_reason {
                                            if reason.0.is_none() {
                                                reason.0 =
                                                    Some("Ship destroyed by NPC fire".into());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } // end if damage >= 1.0

                    // Deactivate beam if elapsed.
                    if phaser_state.beam_remaining_secs <= 0.0 {
                        let ended_uuid = t_uuid.to_string();
                        phaser_state.beam_active = false;
                        phaser_state.beam_target = None;
                        phaser_state.beam_remaining_secs = 0.0;
                        phaser_state.cooldown_remaining = beam_duration;
                        commands.trigger(BeamEndedEvent {
                            bank: "port".to_string(),
                            target_uuid: ended_uuid,
                        });
                    }
                } else {
                    // NPC/asteroid target: existing EntityConsoleHull component path.
                    let mut target_destroyed = false;
                    if damage >= 1.0 {
                        for (tgt_entity, tgt_uid, _tgt_transform, mut tgt_hull, mut tgt_shield) in
                            hull_query.iter_mut()
                        {
                            if tgt_uid.0 != target_uuid_str {
                                continue;
                            }
                            let mut rng = rand::rng();
                            // Route through any `EntityShield` component using
                            // the bank's `shield_pierce` snapshot. Asteroids
                            // / shieldless stations skip this and hit hull
                            // directly. (#471)
                            let damage_to_hull = if let Some(ref mut shield) = tgt_shield {
                                if shield.broken {
                                    damage
                                } else {
                                    let (pierced, absorbed) =
                                        crate::damage::split_damage_for_pierce(
                                            damage,
                                            shield_pierce,
                                        );
                                    let leak = shield.apply_damage(absorbed);
                                    pierced + leak
                                }
                            } else {
                                damage
                            };
                            if damage_to_hull > 0.0 {
                                tgt_hull.0.apply_damage(damage_to_hull, &mut rng);
                            }
                            if tgt_hull.0.is_destroyed() {
                                target_destroyed = true;
                                commands.entity(tgt_entity).try_despawn();
                                destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                                    entity_uuid: tgt_uid.0.clone(),
                                });
                            }
                            break;
                        }
                    } // end if damage >= 1.0
                    if target_destroyed || phaser_state.beam_remaining_secs <= 0.0 {
                        let ended_uuid = t_uuid.to_string();
                        phaser_state.beam_active = false;
                        phaser_state.beam_target = None;
                        phaser_state.beam_remaining_secs = 0.0;
                        phaser_state.cooldown_remaining = beam_duration;
                        commands.trigger(BeamEndedEvent {
                            bank: "port".to_string(),
                            target_uuid: ended_uuid,
                        });
                    }
                }
            } else {
                phaser_state.beam_active = false;
                phaser_state.beam_remaining_secs = 0.0;
            }
        }
    }
}

fn handle_set_phaser_mode(
    ship_query: Query<&AdmittedCommands, With<Ship>>,
    mut phaser_mode: ResMut<CurrentPhaserMode>,
) {
    let Ok(admitted) = ship_query.single() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::TACTICAL_SYSTEM_ID) {
        if let SystemControlPayload::SetPhaserMode { mode } = &cmd.payload {
            phaser_mode.0 = *mode;
        }
    }
}

fn handle_set_phaser_frequency(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::simulation::Ship>,
    >,
    mut ship: ResMut<ShipState>,
) {
    let Ok((ship_config, control_sources)) = ship_query.single() else {
        return;
    };
    let tactical_policy = control_sources
        .0
        .policy_for(&crate::system_registry::tactical_system_id());
    for ev in reader.read() {
        let ClientMessage::SetPhaserFrequency { frequency } = &ev.msg else {
            continue;
        };
        // Only the Tactical holder may set phaser frequency (delegation removed in B4).
        if !tactical_policy.accept_human_input {
            continue;
        }
        if sessions
            .0
            .console_holder(&Console::Tactical, &ship_config.0)
            != Some(ev.token.as_str())
        {
            continue;
        }
        ship.phaser_frequency = frequency.clamp(0.0, 1.0);
    }
}

fn handle_load_tube(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::simulation::Ship>,
    >,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
) {
    let Ok((ship_config, control_sources)) = ship_query.single() else {
        return;
    };
    let policy = control_sources
        .0
        .policy_for(&crate::system_registry::tactical_system_id());
    for ev in reader.read() {
        let ClientMessage::LoadTube { tube } = &ev.msg else {
            continue;
        };
        if !policy.accept_human_input {
            continue;
        }
        if !tactical_authorized(&sessions, ship_config, &ev.token) {
            continue;
        }
        torpedo_sys.0.start_load(tube.as_str());
    }
}

fn handle_unload_tube(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::simulation::Ship>,
    >,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
) {
    let Ok((ship_config, control_sources)) = ship_query.single() else {
        return;
    };
    let policy = control_sources
        .0
        .policy_for(&crate::system_registry::tactical_system_id());
    for ev in reader.read() {
        let ClientMessage::UnloadTube { tube } = &ev.msg else {
            continue;
        };
        if !policy.accept_human_input {
            continue;
        }
        if !tactical_authorized(&sessions, ship_config, &ev.token) {
            continue;
        }
        torpedo_sys.0.start_unload(tube.as_str());
    }
}

fn handle_fire_torpedo(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
        ),
        With<crate::simulation::Ship>,
    >,
    ship: Res<ShipState>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
    player_ship_q: Query<&crate::entity_spawner::EntityUuid, With<crate::server_app::Ship>>,
    weapons_target: Res<WeaponsTarget>,
    mut weapon_fired: ResMut<crate::server_app::WeaponFiredThisTick>,
) {
    let Ok((ship_config, control_sources)) = ship_query.single() else {
        return;
    };
    let policy = control_sources
        .0
        .policy_for(&crate::system_registry::tactical_system_id());
    for ev in reader.read() {
        let ClientMessage::FireTorpedo { tube, target_uuid } = &ev.msg else {
            continue;
        };
        if !policy.accept_human_input {
            continue;
        }
        if !tactical_authorized(&sessions, ship_config, &ev.token) {
            continue;
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let tube_facing_rad = torpedo_sys
            .0
            .tube(tube.as_str())
            .map(|t| t.facing_deg.to_radians())
            .unwrap_or(0.0);
        let launch_heading = ship.yaw + tube_facing_rad;
        let source_uuid = player_ship_q.single().map(|u| u.0.clone()).ok();
        // Use the server-side locked target as the authoritative homing UUID.
        // Fall back to whatever the client sent in case there's no server lock.
        let homing_uuid = weapons_target.0.clone().or_else(|| target_uuid.clone());
        use crate::torpedo::LaunchResult;
        match torpedo_sys.0.launch(
            tube.as_str(),
            uuid,
            ship.x,
            ship.z,
            launch_heading,
            homing_uuid,
            source_uuid,
        ) {
            LaunchResult::Launched {
                uuid: launched_uuid,
            } => {
                weapon_fired.0 = true;
                outbox.0.push((
                    Target::All,
                    ServerMessage::TorpedoLaunched {
                        uuid: launched_uuid,
                        tube: tube.clone(),
                        x: ship.x,
                        z: ship.z,
                        heading: launch_heading,
                    },
                ));
            }
            LaunchResult::TubeNotLoaded | LaunchResult::NoTorpedoes | LaunchResult::UnknownTube => {
            }
        }
    }
}

/// Drains the Power battery via the inter-system command channel while a
/// phaser beam is active. Runs alongside `tick_active_beam` in `SimSet::Physics`;
/// the Power system consumes the drain in `SimSet::Modifiers`.
pub fn drain_power_for_active_beam(
    beam: Res<ActiveBeam>,
    time: Res<Time>,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    if beam.target_uuid.is_some() {
        inter_system.0.push(InterSystemMsg {
            target: crate::system_registry::power_system_id(),
            payload: InterSystemPayload::DrainWeaponsBattery {
                amount: PHASER_BATTERY_DRAIN_PER_SEC * time.delta_secs(),
            },
        });
    }
}

fn tick_torpedo_system(
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    mut world: ResMut<WorldResource>,
    time: Res<Time>,
    mut outbox: ResMut<SimOutbox>,
    mut hull_query: Query<(
        Entity,
        Option<&AsteroidUuid>,
        Option<&crate::entity_spawner::EntityUuid>,
        &mut EntityConsoleHull,
        Option<&mut crate::entity_spawner::EntityShield>,
    )>,
    mut commands: Commands,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    // Virtual entities (asteroid-field anchors, region trigger volumes) are
    // organisational/effect-only. They carry an `EntityUuid` and a non-zero
    // `radius` in the world snapshot (from `outer_radius` or region shape),
    // so without this filter `find_detonation_hits` treats them as giant
    // hittable targets — and a torpedo fired anywhere inside a 350 m
    // asteroid-field annulus detonates on the field anchor on its first
    // physics tick. (Regression that made torpedoes invisible from the
    // viewscreen because the sphere lifetime was a single frame.)
    virtual_entity_q: Query<
        &crate::entity_spawner::EntityUuid,
        Or<(
            With<crate::entity_spawner::AsteroidFieldSection>,
            With<crate::entity_spawner::RegionShapeSection>,
        )>,
    >,
    mut weapons_target: ResMut<WeaponsTarget>,
) {
    let dt = time.delta_secs();

    // UUIDs of virtual (non-hittable) entities — anchors / regions. Used to
    // exclude them from the detonation target list below.
    let virtual_uuids: std::collections::HashSet<String> =
        virtual_entity_q.iter().map(|u| u.0.clone()).collect();
    // World snapshot also carries virtual entities — recognise them by the
    // shape field (`Some("torus" | "sphere" | "box")` marks a region or
    // asteroid-field anchor). The live ECS filter above is the source of
    // truth when the entity is present; this catches snapshot-only entries.
    let virtual_snapshot_uuids: std::collections::HashSet<String> = world
        .0
        .entities
        .iter()
        .filter(|e| e.shape.is_some())
        .map(|e| e.uuid.clone())
        .collect();

    // Build target positions from *live* ECS transforms, falling back to the
    // (stale) WorldResource snapshot for entities not currently in the ECS.
    let target_positions: std::collections::HashMap<String, (f32, f32)> = {
        let mut map: std::collections::HashMap<String, (f32, f32)> =
            std::collections::HashMap::new();
        for (u, t) in asteroid_q.iter() {
            map.insert(u.0.clone(), (t.translation.x, t.translation.z));
        }
        for (u, t) in entity_q.iter() {
            map.insert(u.0.clone(), (t.translation.x, t.translation.z));
        }
        // Fill remaining entries from WorldResource snapshot for completeness.
        for e in world.0.entities.iter() {
            map.entry(e.uuid.clone()).or_insert_with(|| (e.x(), e.z()));
        }
        map
    };
    let result = torpedo_sys.0.tick(dt, &target_positions);
    for expired_uuid in result.expired {
        outbox.0.push((
            Target::All,
            ServerMessage::TorpedoDestroyed { uuid: expired_uuid },
        ));
    }

    // Proximity detonation. Use live positions for the target list.
    let targets: Vec<(String, f32, f32, f32)> = {
        let mut map: std::collections::HashMap<String, (f32, f32, f32)> =
            std::collections::HashMap::new();
        for (u, t) in asteroid_q.iter() {
            // Look up radius from WorldResource (AsteroidUuid has no radius field).
            let radius = world
                .0
                .entities
                .iter()
                .find(|e| e.uuid == u.0)
                .map(|e| e.radius_or_zero())
                .unwrap_or(0.0);
            map.insert(u.0.clone(), (t.translation.x, t.translation.z, radius));
        }
        for (u, t) in entity_q.iter() {
            if virtual_uuids.contains(&u.0) || virtual_snapshot_uuids.contains(&u.0) {
                continue;
            }
            let radius = world
                .0
                .entities
                .iter()
                .find(|e| e.uuid == u.0)
                .map(|e| e.radius_or_zero())
                .unwrap_or(0.0);
            map.insert(u.0.clone(), (t.translation.x, t.translation.z, radius));
        }
        // Fill remaining from WorldResource snapshot (entities not currently in ECS).
        for e in world.0.entities.iter() {
            if virtual_uuids.contains(&e.uuid) || virtual_snapshot_uuids.contains(&e.uuid) {
                continue;
            }
            map.entry(e.uuid.clone())
                .or_insert_with(|| (e.x(), e.z(), e.radius_or_zero()));
        }
        map.into_iter()
            .map(|(uuid, (x, z, r))| (uuid, x, z, r))
            .collect()
    };
    let hits = torpedo_sys.0.find_detonation_hits(&targets);
    for (torpedo_uuid, target_uuid) in hits {
        // `handle_collision_full` returns the structured detonation
        // (`damage_hull` always pierces; `damage_shields` is the
        // shield-eligible portion to be split via `shield_pierce`).
        let Some(detonation) = torpedo_sys.0.handle_collision_full(&torpedo_uuid) else {
            continue;
        };
        outbox.0.push((
            Target::All,
            ServerMessage::TorpedoDestroyed { uuid: torpedo_uuid },
        ));

        let mut asteroid_destroyed = false;
        let mut npc_destroyed = false;
        let mut hit_x = 0.0_f32;
        let mut hit_z = 0.0_f32;

        for (entity, asteroid_uuid, entity_uuid, mut hull_comp, mut shield_comp) in
            hull_query.iter_mut()
        {
            let uuid_matches = asteroid_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str())
                || entity_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str());
            if !uuid_matches {
                continue;
            }
            let is_asteroid = asteroid_uuid.is_some();
            let mut rng = rand::rng();

            // Route shield-eligible damage through any `EntityShield`
            // component, with overflow leaking to hull. Hull damage
            // (always-pierces) goes straight to hull. Asteroids carry no
            // shield so the shielded path is a no-op for them. (#471)
            let mut hull_damage = detonation.damage_hull as f32;
            let shield_eligible = detonation.damage_shields as f32;
            if shield_eligible > 0.0 {
                if let Some(ref mut shield) = shield_comp {
                    if shield.broken {
                        hull_damage += shield_eligible;
                    } else {
                        let (pierced, absorbed) = crate::damage::split_damage_for_pierce(
                            shield_eligible,
                            detonation.shield_pierce,
                        );
                        let leak = shield.apply_damage(absorbed);
                        hull_damage += pierced + leak;
                    }
                } else {
                    hull_damage += shield_eligible;
                }
            }
            if hull_damage > 0.0 {
                hull_comp.0.apply_damage(hull_damage, &mut rng);
            }

            if hull_comp.0.is_destroyed() {
                commands.entity(entity).try_despawn();
                if is_asteroid {
                    asteroid_destroyed = true;
                } else {
                    npc_destroyed = true;
                }
                // Use live position from whichever query matches (asteroid or NPC).
                if is_asteroid {
                    if let Some((_, t)) = asteroid_q.iter().find(|(u, _)| u.0 == target_uuid) {
                        hit_x = t.translation.x;
                        hit_z = t.translation.z;
                    }
                } else if let Some((_, t)) = entity_q.iter().find(|(u, _)| u.0 == target_uuid) {
                    hit_x = t.translation.x;
                    hit_z = t.translation.z;
                }
            }
        }

        if asteroid_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            vfx_events.write(AsteroidDestroyedVfx { x: hit_x, z: hit_z });
            outbox.0.push((
                Target::All,
                ServerMessage::AsteroidDestroyed {
                    uuid: target_uuid.clone(),
                },
            ));
            if weapons_target.0.as_deref() == Some(target_uuid.as_str()) {
                weapons_target.0 = None;
            }
        } else if npc_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                entity_uuid: target_uuid.clone(),
            });
            outbox.0.push((
                Target::All,
                ServerMessage::EntityDespawned {
                    uuid: target_uuid.clone(),
                },
            ));
            if weapons_target.0.as_deref() == Some(target_uuid.as_str()) {
                weapons_target.0 = None;
            }
        }
    }
}

/// Tick NPC shield regen each frame (#471).
///
/// For every entity carrying an `EntityShield` that is not broken and is
/// below `max_hp`, advance `current_hp` by `regen_per_sec * dt`, clamped
/// to `max_hp`. Broken shields do not regen — once down they stay down
/// for the rest of the engagement (no offline timer / recovery model).
fn tick_npc_shield_regen(
    time: Res<Time>,
    mut shields: Query<&mut crate::entity_spawner::EntityShield>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for mut shield in shields.iter_mut() {
        shield.tick_regen(dt);
    }
}

/// Active beam tick handler for weapons plugin integration tests
/// to reference when building their test app.
fn tick_active_beam(
    time: Res<Time>,
    mut beam: ResMut<ActiveBeam>,
    mut cooldown: ResMut<PhaserCooldown>,
    ship: Res<ShipState>,
    mut world: ResMut<WorldResource>,
    mut hull_query: Query<(
        Entity,
        Option<&AsteroidUuid>,
        Option<&crate::entity_spawner::EntityUuid>,
        &mut EntityConsoleHull,
        Option<&mut crate::entity_spawner::EntityShield>,
    )>,
    mut commands: Commands,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    combat_config: Res<PhaserCombatConfigResource>,
    mut outbox: ResMut<SimOutbox>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    player_ship_q: Query<&crate::entity_spawner::EntityUuid, With<crate::server_app::Ship>>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut weapons_target: ResMut<WeaponsTarget>,
) {
    let dt = time.delta_secs();
    cooldown.tick(dt);

    let Some(target_uuid) = beam.target_uuid.clone() else {
        return;
    };
    let active_bank = beam.bank.clone().unwrap_or_default();

    use crate::entity_config::PhaserCombatConfig;
    let active_bank_cfg = combat_config.0.bank_by_id(&active_bank);
    let active_bank_cooldown_secs_early = active_bank_cfg
        .map(|b| {
            if b.cooldown_secs > 0.0 {
                b.cooldown_secs
            } else {
                PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS
            }
        })
        .unwrap_or(PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS);

    // Use live ECS position for arc/range check — WorldResource snapshot is stale.
    let live_pos = live_entity_xz(&target_uuid, &asteroid_q, &entity_q);
    let Some((tx, tz)) = live_pos else {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start_bank(&active_bank, active_bank_cooldown_secs_early);
        commands.trigger(BeamEndedEvent {
            bank: active_bank.clone(),
            target_uuid,
        });
        return;
    };
    let bank_in_arc = if combat_config.0.banks.is_empty() {
        let effective_phaser_range =
            PhaserCombatConfig::DEFAULT_PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
        crate::radar::is_fire_ready_with_range(
            tx,
            tz,
            ship.x,
            ship.z,
            ship.yaw,
            effective_phaser_range,
        )
    } else {
        active_bank_cfg
            .map(|bank_cfg| {
                let bank_base_range = if bank_cfg.beam_range > 0.0 {
                    bank_cfg.beam_range
                } else {
                    PhaserCombatConfig::DEFAULT_PHASER_RANGE
                };
                let effective_bank_range =
                    bank_base_range * modifiers.get(&ModifierSlot::RadarRange);
                let (rx, ry) = crate::weapons::phaser::ship_local(tx, tz, ship.x, ship.z, ship.yaw);
                let range_ok = (tx - ship.x).powi(2) + (tz - ship.z).powi(2)
                    <= effective_bank_range * effective_bank_range;
                range_ok
                    && crate::weapons::phaser::in_arc(
                        rx,
                        ry,
                        bank_cfg.facing_deg,
                        bank_cfg.fire_arc_deg,
                    )
            })
            .unwrap_or(false)
    };
    let active_bank_cooldown_secs = active_bank_cooldown_secs_early;
    if !bank_in_arc {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start_bank(&active_bank, active_bank_cooldown_secs);
        commands.trigger(BeamEndedEvent {
            bank: active_bank.clone(),
            target_uuid,
        });
        return;
    }

    let active_bank_damage_per_sec = active_bank_cfg
        .map(|b| {
            if b.beam_damage_per_sec > 0.0 {
                b.beam_damage_per_sec
            } else {
                PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC
            }
        })
        .unwrap_or(PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC);
    beam.damage_accumulator +=
        active_bank_damage_per_sec * modifiers.get(&ModifierSlot::PhaserDamage) * dt;
    let damage_to_apply = beam.damage_accumulator.floor() as i32;
    if damage_to_apply > 0 {
        beam.damage_accumulator -= damage_to_apply as f32;

        let mut asteroid_destroyed = false;
        let mut npc_destroyed = false;

        // Per-bank `shield_pierce` snapshot for routing damage through any
        // `EntityShield` component on the target. NPCs/stations with a
        // shield split incoming damage via `split_damage_for_pierce`:
        // the pierced fraction lands on hull directly, the absorbed
        // fraction hits the shield (with overflow leaking to hull).
        // Asteroids carry no `EntityShield` so the routing is a no-op
        // for them. (#471)
        let bank_pierce = active_bank_cfg.and_then(|b| b.shield_pierce).unwrap_or(0.0);

        for (entity, asteroid_uuid, entity_uuid, mut hull_comp, mut shield_comp) in
            hull_query.iter_mut()
        {
            // Match by whichever UUID component is present
            let uuid_matches = asteroid_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str())
                || entity_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str());
            if !uuid_matches {
                continue;
            }

            let is_asteroid = asteroid_uuid.is_some();
            let mut rng = rand::rng();

            // Route through shield if present and not broken; otherwise
            // hit hull directly.
            let damage_to_hull: f32 = if let Some(ref mut shield) = shield_comp {
                if shield.broken {
                    damage_to_apply as f32
                } else {
                    let (pierced, absorbed) =
                        crate::damage::split_damage_for_pierce(damage_to_apply as f32, bank_pierce);
                    let leak = shield.apply_damage(absorbed);
                    pierced + leak
                }
            } else {
                damage_to_apply as f32
            };

            if damage_to_hull > 0.0 {
                hull_comp.0.apply_damage(damage_to_hull, &mut rng);
            }

            // Tag the NPC with AttackerThisTick so its AI's `on_attacked`
            // transition fires. Skipped if there's no player-ship entity
            // (e.g. test apps that don't spawn one) or if its UUID is malformed.
            if !is_asteroid {
                if let Ok(player_uuid) = player_ship_q.single() {
                    if let Ok(parsed) = uuid::Uuid::parse_str(&player_uuid.0) {
                        commands
                            .entity(entity)
                            .insert(crate::ai_plugin::AttackerThisTick(parsed));
                    }
                }
            }

            if hull_comp.0.is_destroyed() {
                commands.entity(entity).try_despawn();
                if is_asteroid {
                    asteroid_destroyed = true;
                } else {
                    npc_destroyed = true;
                }
            }
        }

        if asteroid_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            vfx_events.write(AsteroidDestroyedVfx { x: tx, z: tz });

            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start_bank(&active_bank, active_bank_cooldown_secs);
            weapons_target.0 = None;

            outbox.0.push((
                Target::All,
                ServerMessage::AsteroidDestroyed {
                    uuid: target_uuid.clone(),
                },
            ));
            commands.trigger(BeamEndedEvent {
                bank: active_bank.clone(),
                target_uuid,
            });
            return;
        }

        if npc_destroyed {
            // Non-asteroid entity destroyed
            world.0.entities.retain(|a| a.uuid != target_uuid);
            destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                entity_uuid: target_uuid.clone(),
            });
            outbox.0.push((
                Target::All,
                ServerMessage::EntityDespawned {
                    uuid: target_uuid.clone(),
                },
            ));

            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start_bank(&active_bank, active_bank_cooldown_secs);
            weapons_target.0 = None;

            commands.trigger(BeamEndedEvent {
                bank: active_bank.clone(),
                target_uuid,
            });
            return;
        }
    }

    beam.remaining_secs -= dt;
    if beam.remaining_secs <= 0.0 {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start_bank(&active_bank, active_bank_cooldown_secs);
        commands.trigger(BeamEndedEvent {
            bank: active_bank.clone(),
            target_uuid,
        });
    }
}

// ── Tactical AI controller ────────────────────────────────────────────────
//
// Runs only when the Tactical system's ControlSource is Ai.  Sub-regions
// are separated by comment banners — each banner marks a future split point
// when the coarse Tactical system is decomposed into fine-grained systems.

fn operate_tactical_ai(
    ship_query: Query<
        (
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship_plugin::ActiveStationRatings,
        ),
        (
            With<crate::simulation::Ship>,
            Without<crate::ai::server::AiControllerComponent>,
        ),
    >,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    mut weapons_target: ResMut<WeaponsTarget>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
    player_ship_q: Query<&crate::entity_spawner::EntityUuid, With<crate::server_app::Ship>>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), With<crate::simulation::Asteroid>>,
    blackboards: Option<Res<crate::server_app::SystemBlackboards>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    npc_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::EntityName>,
        ),
        Without<crate::simulation::Asteroid>,
    >,
    last_attacker: Res<LastShipAttacker>,
) {
    let Ok((ship_config, _control_sources, active_ratings)) = ship_query.single() else {
        return;
    };

    // Always set weapons_target from Destroy objectives regardless of control
    // source. This lets both human and AI Tactical operators benefit from
    // mission objective auto-targeting.
    // When no Destroy objective is available (or its target entity can't be
    // resolved), fall back to the last NPC that attacked the player ship.
    let objective_target = match top_destroy_objective_target(blackboards.as_deref()) {
        Some(target_name) if target_name.is_empty() => None,
        Some(target_name) => resolve_objective_target_uuid(target_name, runtime.as_deref(), &npc_q),
        None => None,
    };
    if let Some(uuid) = objective_target.or_else(|| last_attacker.0.clone()) {
        weapons_target.0 = Some(uuid);
    }

    // ── TORPEDO AUTO-FIRE (future: split to torpedo_tube system) ─────────────
    //
    // When the station is claimed, gate on whether the active rating's
    // ai_tuning has the torpedo_auto_fire rule. Unclaimed → unconditional.
    let tactical_station = crate::messages::StationId("tactical".into());
    let auto_fire_enabled = match sessions
        .0
        .console_holder(&Console::Tactical, &ship_config.0)
    {
        Some(_) => active_ratings.0.get(&tactical_station).is_some_and(|r| {
            ship_config.0.has_ai_rule(
                &tactical_station,
                r,
                crate::console_ai_plugin::AI_RULE_TORPEDO_AUTO_FIRE,
            )
        }),
        None => true,
    };

    if auto_fire_enabled {
        if let Some(target_uuid) = &weapons_target.0 {
            // Look up live world position — WorldResource snapshot is stale for
            // moving targets.
            let target_xz = asteroid_q
                .iter()
                .find_map(|(u, t)| {
                    (u.0 == *target_uuid).then_some((t.translation.x, t.translation.z))
                })
                .or_else(|| {
                    npc_q.iter().find_map(|(u, t, _)| {
                        (u.0 == *target_uuid).then_some((t.translation.x, t.translation.z))
                    })
                });

            if let Some((tx, tz)) = target_xz {
                let dx = tx - ship.x;
                let dz = tz - ship.z;
                let world_bearing = dx.atan2(-dz);
                let bearing = world_bearing - ship.yaw;

                let ts = &torpedo_sys.0;
                let tubes: Vec<crate::console_ai::TubeSummary> = ts
                    .tubes
                    .iter()
                    .map(|tube| crate::console_ai::TubeSummary {
                        id: tube.id.clone(),
                        loaded: tube.is_loaded(),
                        in_arc: tube.is_in_arc(bearing),
                    })
                    .collect();

                let input = crate::console_ai::TorpedoAiInput {
                    target_locked: true,
                    target_shields: 0,
                    tubes,
                    magazine: ts.torpedoes_remaining,
                };

                let tubes_to_fire = crate::console_ai::auto_fire_torpedo(&input);
                let source_uuid = player_ship_q.single().map(|u| u.0.clone()).ok();

                for tube_id in tubes_to_fire {
                    let torpedo_uuid = uuid::Uuid::new_v4().to_string();
                    let tube_facing_rad = torpedo_sys
                        .0
                        .tube(tube_id.as_str())
                        .map(|t| t.facing_deg.to_radians())
                        .unwrap_or(0.0);
                    let launch_heading = ship.yaw + tube_facing_rad;
                    use crate::torpedo::LaunchResult;
                    match torpedo_sys.0.launch(
                        tube_id.as_str(),
                        torpedo_uuid,
                        ship.x,
                        ship.z,
                        launch_heading,
                        Some(target_uuid.clone()),
                        source_uuid.clone(),
                    ) {
                        LaunchResult::Launched {
                            uuid: launched_uuid,
                        } => {
                            outbox.0.push((
                                Target::All,
                                ServerMessage::TorpedoLaunched {
                                    uuid: launched_uuid,
                                    tube: tube_id,
                                    x: ship.x,
                                    z: ship.z,
                                    heading: launch_heading,
                                },
                            ));
                        }
                        LaunchResult::TubeNotLoaded
                        | LaunchResult::NoTorpedoes
                        | LaunchResult::UnknownTube => {}
                    }
                }
            }
        }
    }

    // ── PHASER AUTO-FIRE (future: split to phaser_bank system) ───────────────
    //
    // tick_phaser_auto_fire handles auto-mode phasers for both human and AI
    // (phaser mode is a ship-level setting, not control-source specific).
    // No additional AI logic needed at the coarse system level.

    // ── FREQUENCY COORDINATION (future: split to channel-3 coordination) ─────
    //
    // Science AI emits FrequencyHint when its preset grants auto_hint.
    // The Tactical AI has no corresponding action at the coarse level.
}

fn top_destroy_objective_target(
    blackboards: Option<&crate::server_app::SystemBlackboards>,
) -> Option<&str> {
    let bb = blackboards?
        .0
        .get(&crate::system_registry::viewscreen_system_id())?;
    let crate::messages::SystemBlackboard::Viewscreen(viewscreen) = bb else {
        return None;
    };
    viewscreen.scored_objectives.iter().find_map(|objective| {
        if objective.score <= 0.0
            || !objective
                .relevance
                .contains(&crate::messages::SystemAffinity::Weapons)
        {
            return None;
        }
        match &objective.directive {
            crate::messages::AiDirective::Destroy { target } => Some(target.as_str()),
            _ => None,
        }
    })
}

fn resolve_objective_target_uuid(
    target_name: &str,
    runtime: Option<&crate::world::server::WorldContentRuntime>,
    npc_q: &Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::EntityName>,
        ),
        Without<crate::simulation::Asteroid>,
    >,
) -> Option<String> {
    runtime
        .and_then(|rt| rt.name_to_uuid.get(target_name).cloned())
        .or_else(|| {
            npc_q.iter().find_map(|(uuid, _, name)| {
                (uuid.0 == target_name || name.is_some_and(|n| n.0 == target_name))
                    .then(|| uuid.0.clone())
            })
        })
}

// ── Broadcaster ───────────────────────────────────────────────────────────

pub fn weapons_update_broadcaster() -> crate::core::broadcast::SimBroadcaster {
    crate::core::broadcast::SimBroadcaster::new().register(
        crate::core::broadcast::Audience::Holding(Console::Tactical),
        crate::core::broadcast::Cadence::Hz(10.0),
        |world: &mut World| {
            // Extract all resource values as owned copies/clones so we can
            // release the immutable borrows before calling world.query_filtered.
            let (ship_x, ship_z, ship_yaw) = {
                let s = world.resource::<ShipState>();
                (s.x, s.z, s.yaw)
            };
            let target_uuid: Option<String> = world.resource::<WeaponsTarget>().0.clone();
            let (beam_active, active_beam_bank) = {
                let b = world.resource::<ActiveBeam>();
                (b.target_uuid.is_some(), b.bank.clone())
            };
            let bank_cooldowns: std::collections::HashMap<String, f32> = {
                let cd = world.resource::<PhaserCooldown>();
                cd.per_bank.clone()
            };
            let tubes: Vec<TorpedoTubeState> = {
                let ts = &world.resource::<TorpedoSystemResource>().0;
                ts.tubes
                    .iter()
                    .map(|t| {
                        let remaining = match &t.load_state {
                            crate::torpedo::TubeLoadState::Loading { remaining, .. }
                            | crate::torpedo::TubeLoadState::Unloading { remaining, .. } => {
                                *remaining
                            }
                            _ => 0.0,
                        };
                        TorpedoTubeState {
                            id: t.id.clone(),
                            loaded: t.is_loaded(),
                            reload_secs: remaining,
                            state: t.load_state.label().to_string(),
                            progress: t.load_state.progress(),
                            load_time: t.load_time,
                        }
                    })
                    .collect()
            };
            let torpedo_count = world
                .resource::<TorpedoSystemResource>()
                .0
                .torpedoes_remaining;
            let radar_range_mult = world
                .resource::<crate::modifiers::ShipModifiers>()
                .get(&ModifierSlot::RadarRange);
            let phaser_mode = world.resource::<CurrentPhaserMode>().0;
            let banks_config = {
                let cc = world.resource::<PhaserCombatConfigResource>();
                cc.0.banks.clone()
            };

            // Query live ECS Transform for the target — WorldResource is a
            // stale spawn-time snapshot and doesn't contain NPC ships that
            // spawn after the scene loads.
            let target_live_pos: Option<(f32, f32)> = match &target_uuid {
                None => None,
                Some(uuid) => {
                    let uuid = uuid.clone();
                    let mut pos = None;
                    let mut entity_qs = world.query_filtered::<
                        (&crate::entity_spawner::EntityUuid, &Transform),
                        Without<AsteroidUuid>,
                    >();
                    for (u, t) in entity_qs.iter(world) {
                        if u.0 == uuid {
                            pos = Some((t.translation.x, t.translation.z));
                            break;
                        }
                    }
                    if pos.is_none() {
                        let mut asteroid_qs = world.query_filtered::<
                            (&AsteroidUuid, &Transform),
                            Without<crate::entity_spawner::EntityUuid>,
                        >();
                        for (u, t) in asteroid_qs.iter(world) {
                            if u.0 == uuid {
                                pos = Some((t.translation.x, t.translation.z));
                                break;
                            }
                        }
                    }
                    pos
                }
            };

            // Look up the display name for the locked target.
            let target_name: Option<String> = match &target_uuid {
                None => None,
                Some(uuid) => {
                    let uuid = uuid.clone();
                    let mut name = None;
                    let mut name_qs = world.query::<(
                        &crate::entity_spawner::EntityUuid,
                        &crate::entities::spawner::EntityName,
                    )>();
                    for (u, n) in name_qs.iter(world) {
                        if u.0 == uuid {
                            name = Some(n.0.clone());
                            break;
                        }
                    }
                    name
                }
            };

            let banks: Vec<PhaserBankState> = if banks_config.is_empty() {
                let effective_phaser_range =
                    crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                        * radar_range_mult;
                let fire_ready = match target_live_pos {
                    None => false,
                    Some((tx, tz)) => crate::radar::is_fire_ready_with_range(
                        tx,
                        tz,
                        ship_x,
                        ship_z,
                        ship_yaw,
                        effective_phaser_range,
                    ),
                };
                let cd = bank_cooldowns.get("").copied().unwrap_or(0.0);
                vec![PhaserBankState {
                    id: String::new(),
                    fire_ready,
                    on_cooldown: beam_active || cd > 0.0,
                    cooldown_remaining: cd,
                }]
            } else {
                banks_config
                    .iter()
                    .map(|b| {
                        let bank_ready = match target_live_pos {
                            None => false,
                            Some((tx, tz)) => {
                                let bank_base_range = if b.beam_range > 0.0 {
                                    b.beam_range
                                } else {
                                    crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                                };
                                let effective_bank_range = bank_base_range * radar_range_mult;
                                let (rx, ry) = crate::weapons::phaser::ship_local(
                                    tx, tz, ship_x, ship_z, ship_yaw,
                                );
                                let range_ok = (tx - ship_x).powi(2) + (tz - ship_z).powi(2)
                                    <= effective_bank_range * effective_bank_range;
                                range_ok
                                    && crate::weapons::phaser::in_arc(
                                        rx,
                                        ry,
                                        b.facing_deg,
                                        b.fire_arc_deg,
                                    )
                            }
                        };
                        let cd = bank_cooldowns.get(b.id.as_str()).copied().unwrap_or(0.0);
                        let beam_on_this_bank =
                            beam_active && active_beam_bank.as_deref() == Some(b.id.as_str());
                        PhaserBankState {
                            id: b.id.clone(),
                            fire_ready: bank_ready,
                            on_cooldown: beam_on_this_bank || cd > 0.0,
                            cooldown_remaining: cd,
                        }
                    })
                    .collect()
            };

            let current = LastWeaponsUpdate {
                target_uuid: target_uuid.clone(),
                target_name: target_name.clone(),
                banks: banks.clone(),
                tubes: tubes.clone(),
                torpedo_count,
                phaser_mode,
            };
            let is_first_tick = world.resource::<WeaponsUpdateFirstTick>().0;
            if !is_first_tick {
                let last = world.resource::<LastWeaponsUpdate>();
                if *last == current {
                    return vec![];
                }
            }
            if is_first_tick {
                *world.resource_mut::<WeaponsUpdateFirstTick>() = WeaponsUpdateFirstTick(false);
            }
            *world.resource_mut::<LastWeaponsUpdate>() = current;

            vec![ServerMessage::WeaponsUpdate {
                target_uuid,
                target_name,
                banks,
                tubes,
                torpedo_count,
                phaser_mode,
            }]
        },
    )
}

// ── Blackboard publish (issue #560) ─────────────────────────────────────────

/// Publish the Weapons system's blackboard from current sim state.
/// Runs in `SimSet::Publish` (phase 1a). Dirty-tracking and broadcast are
/// handled globally by `broadcast_blackboard_updates` in `SimSet::Broadcast`.
fn publish_weapons_blackboard(
    weapons_target: Res<WeaponsTarget>,
    beam: Res<ActiveBeam>,
    cooldown: Res<PhaserCooldown>,
    combat_config: Res<PhaserCombatConfigResource>,
    phaser_mode: Res<CurrentPhaserMode>,
    torpedo_sys: Res<TorpedoSystemResource>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    ship: Res<ShipState>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    world_res: Res<WorldResource>,
    entity_name_q: Query<(
        &crate::entity_spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut blackboards: ResMut<crate::server_app::SystemBlackboards>,
) {
    use crate::system_registry::TACTICAL_SYSTEM_ID;

    let target_uuid = weapons_target.0.clone();
    let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);
    let beam_active = beam.target_uuid.is_some();
    let active_beam_bank = beam.bank.clone();

    let target_live_pos: Option<(f32, f32)> = target_uuid
        .as_deref()
        .and_then(|uuid| live_entity_xz(uuid, &asteroid_q, &entity_q));

    let target_name: Option<String> = target_uuid.as_deref().and_then(|uuid| {
        entity_name_q
            .iter()
            .find_map(|(u, n)| (u.0 == uuid).then(|| n.0.clone()))
    });

    let banks: Vec<PhaserBankState> = if combat_config.0.banks.is_empty() {
        let effective_range =
            crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE * radar_range_mult;
        let fire_ready = match target_live_pos {
            None => false,
            Some((tx, tz)) => crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                ship.x,
                ship.z,
                ship.yaw,
                effective_range,
            ),
        };
        let cd = cooldown.bank_remaining_secs("");
        vec![PhaserBankState {
            id: String::new(),
            fire_ready,
            on_cooldown: beam_active || cd > 0.0,
            cooldown_remaining: cd,
        }]
    } else {
        combat_config
            .0
            .banks
            .iter()
            .map(|b| {
                let bank_ready = match target_live_pos {
                    None => false,
                    Some((tx, tz)) => {
                        let bank_base_range = if b.beam_range > 0.0 {
                            b.beam_range
                        } else {
                            crate::entity_config::PhaserCombatConfig::DEFAULT_PHASER_RANGE
                        };
                        let effective_bank_range = bank_base_range * radar_range_mult;
                        let (rx, ry) =
                            crate::weapons::phaser::ship_local(tx, tz, ship.x, ship.z, ship.yaw);
                        let range_ok = (tx - ship.x).powi(2) + (tz - ship.z).powi(2)
                            <= effective_bank_range * effective_bank_range;
                        range_ok
                            && crate::weapons::phaser::in_arc(rx, ry, b.facing_deg, b.fire_arc_deg)
                    }
                };
                let cd = cooldown.bank_remaining_secs(b.id.as_str());
                let beam_on_this_bank =
                    beam_active && active_beam_bank.as_deref() == Some(b.id.as_str());
                PhaserBankState {
                    id: b.id.clone(),
                    fire_ready: bank_ready,
                    on_cooldown: beam_on_this_bank || cd > 0.0,
                    cooldown_remaining: cd,
                }
            })
            .collect()
    };

    let tubes: Vec<TorpedoTubeState> = torpedo_sys
        .0
        .tubes
        .iter()
        .map(|t| {
            let remaining = match &t.load_state {
                crate::torpedo::TubeLoadState::Loading { remaining, .. }
                | crate::torpedo::TubeLoadState::Unloading { remaining, .. } => *remaining,
                _ => 0.0,
            };
            TorpedoTubeState {
                id: t.id.clone(),
                loaded: t.is_loaded(),
                reload_secs: remaining,
                state: t.load_state.label().to_string(),
                progress: t.load_state.progress(),
                load_time: t.load_time,
            }
        })
        .collect();

    // ── Radar blips ──────────────────────────────────────────────────────────
    let effective_tactical_range = ship_config.0.tactical_radar_range * radar_range_mult;
    let shows: Vec<crate::entity_tags::EntityTag> = ship_config
        .0
        .tactical_radar_shows
        .iter()
        .filter_map(|s| crate::entity_tags::EntityTag::from_str(s))
        .collect();
    let selects: Vec<crate::entity_tags::EntityTag> = ship_config
        .0
        .tactical_radar_selects
        .iter()
        .filter_map(|s| crate::entity_tags::EntityTag::from_str(s))
        .collect();

    let entity_meta: std::collections::HashMap<&str, &crate::messages::EntitySnapshot> = world_res
        .0
        .entities
        .iter()
        .map(|e| (e.uuid.as_str(), e))
        .collect();

    let mut blips: Vec<RadarBlip> = Vec::new();
    if !shows.is_empty() && effective_tactical_range > 0.0 {
        for (uuid_comp, transform) in asteroid_q.iter() {
            let meta = entity_meta.get(uuid_comp.0.as_str()).copied();
            if let Some(b) = project_blip(
                &uuid_comp.0,
                transform.translation.x,
                transform.translation.z,
                ship.x,
                ship.z,
                ship.yaw,
                effective_tactical_range,
                meta,
                &shows,
                &selects,
            ) {
                blips.push(b);
            }
        }
        for (uuid_comp, transform) in entity_q.iter() {
            let meta = entity_meta.get(uuid_comp.0.as_str()).copied();
            if let Some(b) = project_blip(
                &uuid_comp.0,
                transform.translation.x,
                transform.translation.z,
                ship.x,
                ship.z,
                ship.yaw,
                effective_tactical_range,
                meta,
                &shows,
                &selects,
            ) {
                blips.push(b);
            }
        }
    }

    // ── Region overlays ──────────────────────────────────────────────────────
    let regions: Vec<RadarRegion> = world_res
        .0
        .entities
        .iter()
        .filter_map(|e| {
            let shape = e.shape.as_deref()?;
            Some(RadarRegion {
                uuid: e.uuid.clone(),
                x: e.x(),
                z: e.z(),
                shape: shape.to_string(),
                radius: e.radius,
                inner_radius: e.inner_radius,
                outer_radius: e.radius,
                half_extents: e.half_extents.map(|h| [h[0], h[2]]),
                yaw: e.yaw,
                color: e.colour.unwrap_or([0.6, 0.4, 1.0]),
                name: e.name.clone(),
            })
        })
        .collect();

    let phaser_arcs: Vec<PhaserBankClientConfig> = ship_config.0.phaser_banks.clone();
    let torpedo_arcs: Vec<TorpedoTubeClientConfig> = ship_config.0.torpedo_tubes.clone();

    let bb = WeaponsBlackboard {
        target_uuid,
        target_name,
        banks,
        tubes,
        torpedo_count: torpedo_sys.0.torpedoes_remaining,
        phaser_mode: phaser_mode.0,
        phaser_arcs,
        torpedo_arcs,
        blips,
        regions,
    };

    blackboards.0.insert(
        SystemId(TACTICAL_SYSTEM_ID.to_string()),
        SystemBlackboard::Weapons(bb),
    );
}

/// Project a world-space entity to a [`RadarBlip`] for the HTML Tactical radar.
///
/// Returns `None` when:
/// - `shows` is empty (radar configured to show nothing)
/// - the entity's tags don't overlap `shows` (OR-logic tag filter)
/// - the entity is farther than `effective_range` from the ship (range cull)
///
/// Positions are normalised to `[-1.0, 1.0]` where ±1.0 = `effective_range`.
/// The projection is ship-centred and ship-aligned (forward = +radar_y = up).
///
/// `meta` supplies the full [`EntitySnapshot`] for richer blip data
/// (icon name, colour tint, objective flag, display name). Pass `None`
/// for dynamically-spawned entities not yet in `WorldResource`.
fn project_blip(
    uuid: &str,
    wx: f32,
    wz: f32,
    ship_x: f32,
    ship_z: f32,
    ship_yaw: f32,
    effective_range: f32,
    meta: Option<&crate::messages::EntitySnapshot>,
    shows: &[crate::entity_tags::EntityTag],
    selects: &[crate::entity_tags::EntityTag],
) -> Option<RadarBlip> {
    let raw_tags: &[String] = meta.map(|e| e.tags.as_slice()).unwrap_or(&[]);
    let radius: f32 = meta.and_then(|e| e.radius).unwrap_or(0.0);

    let entity_tags = crate::entity_tags::parse_tags(raw_tags);
    if !crate::entity_tags::matches_any(&entity_tags, shows) {
        return None;
    }
    let dx = wx - ship_x;
    let dz = wz - ship_z;
    if dx * dx + dz * dz > effective_range * effective_range {
        return None;
    }
    let cos_y = ship_yaw.cos();
    let sin_y = ship_yaw.sin();
    // Ship-aligned projection: forward = -Z at yaw=0, right = +X.
    // radar_x = dot((dx,dz), right)   = dx*cos(yaw) + dz*sin(yaw)
    // radar_y = dot((dx,dz), forward) = dx*sin(yaw) - dz*cos(yaw)
    let radar_x = (dx * cos_y + dz * sin_y) / effective_range;
    let radar_y = (dx * sin_y - dz * cos_y) / effective_range;
    let scaled_radius = radius / effective_range;
    let kind = entity_tags
        .iter()
        .find_map(|t| match t {
            crate::entity_tags::EntityTag::Asteroid => Some("asteroid"),
            crate::entity_tags::EntityTag::Ship => Some("ship"),
            crate::entity_tags::EntityTag::Station => Some("station"),
            _ => None,
        })
        .unwrap_or("unknown")
        .to_string();

    // Resolve icon name: prefer explicit `radar_icon` from snapshot, else
    // derive from tags the same way `kind` does but with finer granularity
    // (planet, star, torpedo).
    let icon = meta
        .and_then(|e| e.radar_icon.as_deref())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            entity_tags
                .iter()
                .find_map(|t| match t {
                    crate::entity_tags::EntityTag::Asteroid => Some("asteroid"),
                    crate::entity_tags::EntityTag::Ship => Some("ship"),
                    crate::entity_tags::EntityTag::Station => Some("station"),
                    _ => None,
                })
                .unwrap_or("unknown")
                .to_string()
        });

    // Colour: from snapshot or per-icon default (matches JS KIND_COLOR).
    let color: [f32; 3] = meta
        .and_then(|e| e.colour)
        .unwrap_or_else(|| blip_default_color(&icon));

    let objective_target = meta.map(|e| e.objective_target).unwrap_or(false);
    let name = meta.and_then(|e| e.name.clone());

    // Resolve target info for selectability.
    let target_tags_raw: &[String] = meta.map(|e| e.target_tags.as_slice()).unwrap_or(&[]);
    let target_tags = crate::entity_tags::parse_tags(target_tags_raw);
    let selectable = crate::entity_tags::matches_any(&target_tags, selects);
    let threat_level = meta
        .and_then(|e| e.threat_level.as_deref())
        .map(|s| s.to_string());
    let description = meta
        .and_then(|e| e.target_description.as_deref())
        .or(name.as_deref())
        .map(|s| s.to_string());

    Some(RadarBlip {
        uuid: uuid.to_string(),
        radar_x,
        radar_y,
        scaled_radius,
        kind,
        icon,
        color,
        objective_target,
        name,
        selectable,
        threat_level,
        description,
        target_tags: target_tags_raw.to_vec(),
    })
}

/// Default RGB colour tint for a blip when the entity snapshot carries no
/// explicit colour.  Mirrors the `KIND_COLOR` palette in `radar-widget.js`.
fn blip_default_color(icon: &str) -> [f32; 3] {
    match icon {
        "asteroid" => [0.478, 0.753, 1.0], // #7ac0ff
        "ship" => [1.0, 0.502, 0.376],     // #ff8060
        "station" => [1.0, 0.878, 0.376],  // #ffe060
        "torpedo" => [1.0, 0.376, 1.0],    // #ff60ff
        "planet" => [0.376, 1.0, 0.753],   // #60ffc0
        "star" => [1.0, 0.980, 0.753],     // #fffac0
        "player" => [0.0, 1.0, 0.2],       // green — player ship
        "battleship" => [0.9, 0.2, 0.05],  // dark red — large enemy
        "cruiser" => [0.8, 0.3, 0.1],      // orange-red — medium enemy
        "destroyer" => [1.0, 0.2, 0.2],    // bright red — small enemy
        _ => [0.659, 0.690, 0.753],        // #a8b0c0 unknown
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::ConsoleHull;
    use crate::lobby::{LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::modifiers::ShipModifiers;
    use crate::simulation::{ShipHullIntegrity, ShipImpulse, SimOutbox};

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    /// Build a minimal `ShipConfigComponent` with a tactical station that has an
    /// "Assisted" rating containing `torpedo_auto_fire` in its ai_tuning table.
    fn test_ship_config() -> crate::ship_plugin::ShipConfigComponent {
        const TOML: &str = r#"
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons."
rank = "Ltn."
short_code = "TAC"
console = "tactical"

[[station.rating]]
name = "Std"
automated_systems = []

[[station.rating]]
name = "Assisted"
automated_systems = []

[station.rating.ai_tuning]
torpedo_auto_fire = {}

[[system]]
id = "tactical"
kind = "tactical"
station = "tactical"
"#;
        crate::ship_plugin::ShipConfigComponent(
            crate::ship::config::parse_and_validate(TOML, &["tactical"])
                .expect("test ship config must be valid"),
        )
    }

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.configure_sets(
            Update,
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
        .add_plugins(LobbyPlugin)
        .add_plugins(bevy::time::TimePlugin)
        .add_plugins(crate::server_app::AdmissionPlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(200),
        ))
        .insert_resource(ShipState::new())
        .insert_resource(ShipHullIntegrity(ConsoleHull::from_config(&[
            (Console::Helm, 25.0),
            (Console::Tactical, 25.0),
            (Console::Power, 25.0),
            (Console::Shields, 25.0),
        ])))
        .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
        .init_resource::<WorldResource>()
        .init_resource::<WeaponsTarget>()
        .init_resource::<ActiveBeam>()
        .add_message::<AsteroidDestroyedVfx>()
        .add_message::<crate::ai_plugin::AiEntityDestroyed>()
        .init_resource::<PhaserCooldown>()
        .init_resource::<CurrentPhaserMode>()
        .insert_resource(ShipModifiers::new())
        .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
            TorpedoConfig::default(),
        )))
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
        .init_resource::<crate::server_app::SystemBlackboards>()
        .init_resource::<crate::world::server::WorldContentRuntime>()
        .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
        .add_plugins(WeaponsPlugin)
        // Override with two banks so per-bank arc checks work.
        // Uses wide (270°) arcs so existing tests that fire "port" at a
        // target ahead still pass. Tighter arcs are tested in dedicated
        // per-bank arc severance tests.
        .insert_resource(PhaserCombatConfigResource(
            crate::entity_config::PhaserCombatConfig {
                banks: vec![
                    crate::entity_config::PhaserBankConfig {
                        id: "port".into(),
                        facing_deg: -90.0,
                        fire_arc_deg: 270.0,
                        auto_arc_deg: 240.0,
                        beam_range: 0.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 6.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    },
                    crate::entity_config::PhaserBankConfig {
                        id: "starboard".into(),
                        facing_deg: 90.0,
                        fire_arc_deg: 270.0,
                        auto_arc_deg: 240.0,
                        beam_range: 0.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 6.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    },
                ],
            },
        ))
        .add_systems(Update, (tick_active_beam, tick_torpedo_system))
        .add_plugins(weapons_update_broadcaster())
        .add_systems(PostUpdate, collect);
        // Spawn the Ship entity with config/control-source components so all
        // weapons systems that use `Query<..., With<Ship>>.single()` have a
        // valid entity to operate on, matching what `spawn_game_start_entities`
        // would do in a full server build.
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            test_ship_config(),
            ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::CoordinationQueue::default(),
        ));
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
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

    fn load_tube_now(app: &mut App, tube: &str) {
        app.world_mut()
            .resource_mut::<TorpedoSystemResource>()
            .0
            .tube_mut(tube)
            .expect("test tube should exist")
            .load_state = crate::torpedo::TubeLoadState::Loaded;
    }

    fn start_game(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn setup_weapons_world(
        app: &mut App,
        asteroid_x: f32,
        asteroid_z: f32,
    ) -> bevy::ecs::entity::Entity {
        let uuid = "target-uuid".to_string();
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![crate::messages::EntitySnapshot::asteroid(
                    &uuid, asteroid_x, asteroid_z, 2.0,
                )],
                ..Default::default()
            }));
        // handle_set_target and tick_active_beam use live ECS Transforms
        // (live_entity_xz), so every WorldResource entry must also have a
        // matching ECS entity with the components all queries expect.
        app.world_mut()
            .spawn((
                crate::simulation::Asteroid,
                crate::simulation::AsteroidUuid(uuid),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    crate::messages::Console::CaptainChair,
                    30.0,
                )])),
                Transform::from_xyz(asteroid_x, 0.0, asteroid_z),
            ))
            .id()
    }

    fn setup_weapons_world_with_entity(
        app: &mut App,
        asteroid_x: f32,
        asteroid_z: f32,
    ) -> bevy::ecs::entity::Entity {
        setup_weapons_world(app, asteroid_x, asteroid_z)
    }

    fn start_game_with_weapons(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "weapons", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world_with_entity(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        push(
            app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let _ = tick(app);
        push(
            app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(app)
    }

    // ── SetTarget / TargetLock tests ───────────────────────────────────────

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 30.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert_eq!(lock.0, "target-uuid");
        assert!(lock.1, "expected locked=true for in-range asteroid");

        assert_eq!(
            app.world().resource::<WeaponsTarget>().0.as_deref(),
            Some("target-uuid")
        );
    }

    #[test]
    fn asteroid_outside_weapons_range_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 400.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for out-of-range asteroid");
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
    }

    #[test]
    fn unknown_uuid_replies_with_target_lock_rejected() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 10.0, 0.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "no-such-asteroid".into(),
                },
            },
        );
        let out = tick(&mut app);

        let lock = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::TargetLock { uuid, locked } => Some((uuid.clone(), *locked)),
                _ => None,
            })
            .expect("expected a TargetLock response");
        assert!(!lock.1, "expected locked=false for unknown UUID");
        assert!(app.world().resource::<WeaponsTarget>().0.is_none());
    }

    // ── WeaponsUpdate / fire_ready tests ───────────────────────────────────

    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        // Target changes → WeaponsUpdate fires this tick.
        let out = tick(&mut app);

        let update = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::WeaponsUpdate {
                    target_uuid, banks, ..
                } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
                _ => None,
            })
            .expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(
            update.1,
            "expected fire_ready=true for in-range, forward-arc target"
        );
    }

    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        // Target changes → WeaponsUpdate fires this tick.
        let out = tick(&mut app);

        let update = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::WeaponsUpdate {
                    target_uuid, banks, ..
                } => Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
                _ => None,
            })
            .expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(
            !update.1,
            "expected fire_ready=false for beyond-phaser-range target"
        );
    }

    // ── FirePhaser / beam lifecycle tests ──────────────────────────────────

    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        let out = lock_and_fire(&mut app, 0.0, -20.0);

        let beam_started = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
        assert!(
            beam_started.is_some(),
            "expected BeamStarted after firing at fire-ready target"
        );
        match &beam_started.unwrap().msg {
            ServerMessage::BeamStarted { target_uuid, .. } => {
                assert_eq!(target_uuid, "target-uuid")
            }
            _ => unreachable!(),
        }
        match &beam_started.unwrap().target {
            Target::All => {}
            t => panic!("BeamStarted should target All, got {:?}", t),
        }

        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );
    }

    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        app.world_mut().resource_mut::<ActiveBeam>().target_uuid = None;
        app.world_mut()
            .resource_mut::<PhaserCooldown>()
            .start_bank_with_cooldown("port", 3.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown"
        );
    }

    #[test]
    fn fire_phaser_ignored_from_non_weapons_player() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game(&mut app);

        push(
            &mut app,
            "captain",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "captain should not be able to fire phaser"
        );
    }

    #[test]
    fn fire_phaser_rejected_when_target_outside_bank_arc() {
        let mut app = test_app();
        // Target at starboard beam (20, 0), bearing +90°, which is outside the
        // port bank's 270° arc centered at -90° (covers -135° to 45°).
        setup_weapons_world(&mut app, 20.0, 0.0);
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is outside bank's fire arc"
        );
    }

    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();
        // setup_weapons_world (called by lock_and_fire) now spawns the
        // asteroid ECS entity. Fetch its handle after setup.
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        let asteroid_entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("setup_weapons_world should have spawned the target asteroid")
        };

        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("target-uuid")
        );

        {
            let mut b = app.world_mut().resource_mut::<ActiveBeam>();
            b.damage_accumulator = 30.0;
            b.remaining_secs = 5.0;
        }

        let out = tick(&mut app);

        let destroyed = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(
            destroyed.is_some(),
            "expected AsteroidDestroyed when asteroid HP reaches 0"
        );
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction"
        );

        assert!(
            !app.world()
                .resource::<WorldResource>()
                .0
                .entities
                .iter()
                .any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());

        assert!(
            app.world()
                .resource::<PhaserCooldown>()
                .is_bank_active("port"),
            "cooldown should start after beam end"
        );

        assert!(
            app.world()
                .get::<EntityConsoleHull>(asteroid_entity)
                .is_none(),
            "asteroid entity should be despawned"
        );
    }

    #[test]
    fn beam_severs_when_target_leaves_bank_arc() {
        let mut app = test_app();
        // Target at port beam (-20, 0), bearing -90° — inside port bank's
        // 270° arc centered at -90° (covers -135° to 45°).
        let _ = lock_and_fire(&mut app, -20.0, 0.0);

        // Rotate 180° so the target moves to starboard beam (bearing +90°),
        // which is outside the port bank's arc.
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves bank fire arc"
        );
        assert!(
            app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-arc"
        );
        assert!(
            app.world()
                .resource::<PhaserCooldown>()
                .is_bank_active("port"),
            "cooldown should start after arc sever"
        );
    }

    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move the live ECS Transform out of range. tick_active_beam reads the
        // live position, not the WorldResource snapshot.
        let entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("target entity should exist")
        };
        app.world_mut()
            .entity_mut(entity)
            .insert(Transform::from_xyz(0.0, 0.0, -50.0));

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range"
        );
        assert!(
            app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-range"
        );
        assert!(
            app.world()
                .resource::<PhaserCooldown>()
                .is_bank_active("port"),
            "cooldown should start after range sever"
        );
    }

    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        let asteroid_entity = app
            .world_mut()
            .spawn((
                crate::simulation::Asteroid,
                crate::simulation::AsteroidUuid("target-uuid".into()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    crate::messages::Console::CaptainChair,
                    30.0,
                )])),
            ))
            .id();
        // Target at port beam (-20, 0) so the port bank's arc check passes.
        let _ = lock_and_fire(&mut app, -20.0, 0.0);

        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 10.0;
        let _ = tick(&mut app);

        // Rotate 180° — target moves to starboard beam, outside port bank's arc.
        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;
        let _ = tick(&mut app);

        let hp = app
            .world()
            .get::<EntityConsoleHull>(asteroid_entity)
            .map(|h| h.0.total_current());
        assert!(
            hp.is_some() && hp.unwrap() < 30.0,
            "asteroid should retain damage after sever (no refund), hp={:?}",
            hp
        );
    }

    #[test]
    fn retarget_after_cooldown_cancels_prior_beam_and_starts_new() {
        let mut app = test_app();
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![
                    crate::messages::EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                    crate::messages::EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
                ],
                ..Default::default()
            }));
        // Spawn matching ECS entities so live_entity_xz can find them.
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("t1".into()),
            EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("t2".into()),
            EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
                30.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -15.0),
        ));
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: "t1".into() },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let _ = tick(&mut app);
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("t1")
        );

        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 0.0;
        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 0.0;
        let _ = tick(&mut app);

        assert!(app
            .world()
            .resource::<PhaserCooldown>()
            .is_bank_active("port"));

        app.world_mut()
            .resource_mut::<PhaserCooldown>()
            .start_bank_with_cooldown("port", 0.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget { uuid: "t2".into() },
            },
        );
        let _ = tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown"
        );
        assert_eq!(
            app.world().resource::<ActiveBeam>().target_uuid.as_deref(),
            Some("t2")
        );
    }

    // ── SetPhaserMode tests ────────────────────────────────────────────────

    #[test]
    fn weapons_console_can_set_phaser_mode_to_manual() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Manual,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Manual,
            "phaser mode should be Manual after SetPhaserMode"
        );
    }

    #[test]
    fn non_weapons_player_cannot_set_phaser_mode() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        // Establish a known mode (Auto) via the authorised player first.
        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Auto,
                },
            },
        );
        tick(&mut app);
        // Non-weapons player attempts to switch back to Manual — must be ignored.
        push(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetPhaserMode {
                    mode: crate::messages::PhaserMode::Manual,
                },
            },
        );
        tick(&mut app);
        assert_eq!(
            app.world().resource::<CurrentPhaserMode>().0,
            crate::messages::PhaserMode::Auto,
            "phaser mode should stay Auto when non-Weapons player sends SetPhaserMode"
        );
    }

    // ── FireTorpedo tests ──────────────────────────────────────────────────

    #[test]
    fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")),
            "expected TorpedoLaunched broadcast after Tactical fires torpedo"
        );
    }

    #[test]
    fn local_console_token_can_fire_torpedo() {
        // issue #422: actions from the local HTML console (browser server
        // viewscreen / native wry server) arrive under LOCAL_CONSOLE_TOKEN with
        // no remote PeerJS session, so console_holder(Tactical) is None.
        // `tactical_authorized` must treat that token as an authorized local
        // operator so a button press actually launches end-to-end — the
        // decode→map→InboundMessage→fire hop the wasm bridge cannot unit-test.
        let mut app = test_app();
        // No player holds Tactical here — authorization comes purely from the
        // local-console bypass.
        load_tube_now(&mut app, "fore_port");
        push(
            &mut app,
            crate::console_bridge::LOCAL_CONSOLE_TOKEN,
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")),
            "local console token should be authorized to fire torpedoes end-to-end (issue #422)"
        );
    }

    #[test]
    fn torpedo_system_resource_reflects_player_ship_toml_torpedoes_block() {
        // End-to-end TOML-driven wiring check: build the runtime
        // TorpedoSystem the same way `spawn_game_start_entities` does
        // (parse player_ship.toml → TorpedoesConfig::to_runtime → TorpedoSystem)
        // and assert the magazine size matches the TOML.
        let toml_str = include_str!("../../../assets/entities/player_ship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("player_ship.toml must parse");
        let tc = config
            .torpedoes
            .expect("player_ship must declare [torpedoes]");
        let runtime = tc.to_runtime();
        let sys = crate::torpedo::TorpedoSystem::new(runtime.clone());
        // Magazine size matches TOML — changing `count = 10` to `count = 99`
        // in player_ship.toml would fail this assertion.
        assert_eq!(sys.torpedoes_remaining, tc.count);
        assert_eq!(sys.config.damage_hull, tc.damage_hull);
        assert_eq!(sys.config.load_time, tc.load_time);
        assert!((sys.config.turn_rate - tc.turn_rate_deg_per_sec.to_radians()).abs() < 1e-5);
    }

    #[test]
    fn phaser_combat_config_resource_reflects_player_ship_toml_weapons_console() {
        // End-to-end TOML-driven wiring check: build the runtime
        // PhaserCombatConfig the same way `spawn_game_start_entities` does
        // (parse player_ship.toml → PhaserCombatConfig::from_weapons_console
        // → PhaserCombatConfigResource) and assert the resulting per-bank
        // values are exactly what the TOML says.
        let toml_str = include_str!("../../../assets/entities/player_ship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("player_ship.toml must parse");
        let wc = config
            .weapons_console
            .expect("player_ship must declare [weapons_console]");
        let combat = crate::entity_config::PhaserCombatConfig::from_weapons_console(&wc);

        // player_ship.toml has two banks (fore, aft) with matching combat values.
        assert_eq!(combat.banks.len(), 2, "must have fore and aft banks");
        let fore = &combat.banks[0];
        assert_eq!(fore.id, "fore");
        assert_eq!(fore.cooldown_secs, 6.0, "cooldown_secs from TOML bank");
        assert_eq!(
            fore.beam_duration_secs, 6.0,
            "beam_duration_secs from TOML bank"
        );
        assert_eq!(
            fore.beam_damage_per_sec, 5.0,
            "beam_damage_per_sec from TOML bank"
        );
        assert_eq!(fore.beam_range, 50.0, "beam_range from TOML bank");

        // And starting the cooldown produces exactly that value, so it flows
        // through to live `PhaserCooldown.bank_remaining_secs`.
        let mut cd = PhaserCooldown::default();
        cd.start_bank("test", fore.cooldown_secs);
        assert_eq!(
            cd.bank_remaining_secs("test"),
            6.0,
            "PhaserCooldown::start_bank must use the TOML-sourced cooldown"
        );
    }

    #[test]
    fn non_tactical_player_cannot_fire_torpedo() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "captain",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            !out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "captain should not be able to fire torpedo"
        );
    }

    #[test]
    fn fire_torpedo_during_lobby_fires_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level.
        // In test configurations without SimSet, the system processes messages during Lobby.
        let mut app = test_app();
        load_tube_now(&mut app, "aft");
        push(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "aft".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "FireTorpedo should fire during Lobby when no SimSet gate is configured"
        );
    }

    #[test]
    fn torpedo_launched_is_broadcast_to_all() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        load_tube_now(&mut app, "fore_starboard");

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_starboard".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        let launched = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. }))
            .expect("expected TorpedoLaunched");
        assert!(
            matches!(&launched.target, Target::All),
            "TorpedoLaunched should be broadcast to All, not {:?}",
            launched.target
        );
    }

    #[test]
    fn torpedo_does_not_detonate_on_asteroid_field_anchor_entity() {
        // Regression for "torpedoes don't appear when you hit fire": the
        // default scenario seats the player ship at (280, 0, 0), 280 m from
        // an `asteroid_field_main` anchor entity at the origin. That anchor
        // entity carries an `[asteroid_field]` section with
        // `outer_radius = 350`, and `EntitySnapshot.radius` is populated from
        // that outer radius. `find_detonation_hits` treats every entity in
        // the world with a non-zero radius as a hittable target, so the
        // torpedo detonated on the field anchor on its first physics tick —
        // before the firing crew ever saw a sphere on the viewscreen.
        //
        // Asteroid-field anchors are virtual organisational entities and
        // must never act as torpedo detonation targets.
        use crate::entity_config::AsteroidFieldConfig;
        use crate::entity_spawner::{AsteroidFieldSection, EntityUuid};

        let mut app = test_app();
        start_game_with_weapons(&mut app);

        let field_uuid = "field-uuid".to_string();
        // Mirror the production code path: the WorldResource snapshot for the
        // field anchor reports radius = outer_radius.
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![crate::messages::EntitySnapshot {
                    uuid: field_uuid.clone(),
                    position: Some([0.0, 0.0, 0.0]),
                    radius: Some(350.0),
                    inner_radius: Some(300.0),
                    shape: Some("torus".into()),
                    tags: vec!["asteroid_field".into()],
                    ..Default::default()
                }],
                ..Default::default()
            }));
        // Real ECS-side anchor entity so the live-position path also sees it.
        app.world_mut().spawn((
            EntityUuid(field_uuid.clone()),
            AsteroidFieldSection(AsteroidFieldConfig {
                inner_radius: 300.0,
                outer_radius: 350.0,
                density: 0.005,
                spawn_distance: 250.0,
                despawn_distance: 300.0,
                asteroid_type_paths: vec![],
                cosmetic_type_paths: vec![],
                shape: None,
                anchor: None,
                anchor_offset: [0.0, 0.0, 0.0],
                shield_pierce: 0.0,
                tags: vec![],
                grid: None,
                random_rotation: None,
            }),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));

        // Move the ship inside the field-anchor's "radius" (300 < 350).
        app.world_mut().resource_mut::<ShipState>().x = 280.0;
        load_tube_now(&mut app, "fore_port");

        push(
            &mut app,
            "weapons",
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        // First tick processes the FireTorpedo; second tick is where
        // `tick_torpedo_system` evaluates detonations against the live
        // target list (including the field anchor at the origin).
        tick(&mut app);
        tick(&mut app);

        let in_flight_len = app
            .world()
            .resource::<TorpedoSystemResource>()
            .0
            .in_flight
            .len();
        assert_eq!(
            in_flight_len, 1,
            "torpedo should still be in flight after ticking — the asteroid \
             field anchor entity must not be treated as a detonation target"
        );
    }

    // ── ShipModifiers integration tests ────────────────────────────────────

    #[test]
    fn empty_modifier_table_reproduces_base_phaser_damage() {
        let mut app = test_app();
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        let hp_before = {
            let world = app.world().resource::<WorldResource>();
            world
                .0
                .entities
                .iter()
                .find(|a| a.uuid == "target-uuid")
                .map(|_| true)
        };
        assert!(hp_before.is_some(), "asteroid should still exist after <1s");
    }

    #[test]
    fn phaser_damage_modifier_doubles_kill_rate() {
        use crate::messages::{ModifierSlot, ModifierSource};
        use crate::modifiers::{Modifier, ShipModifiers};

        let mut app_fast = test_app();
        setup_weapons_world_with_entity(&mut app_fast, 0.0, -20.0);
        {
            let mut mods = app_fast.world_mut().resource_mut::<ShipModifiers>();
            mods.add_or_update(Modifier {
                source: ModifierSource::ImpulseDrive,
                slot: ModifierSlot::PhaserDamage,
                bonus: 1.0,
            });
        }
        start_game_with_weapons(&mut app_fast);
        push(
            &mut app_fast,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app_fast);
        push(
            &mut app_fast,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app_fast);

        {
            let mut beam = app_fast.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 2.0 * 3.5;
        }
        tick(&mut app_fast);

        let still_exists_fast = app_fast
            .world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid");
        assert!(
            !still_exists_fast,
            "with 2× phaser damage modifier, asteroid should be destroyed after 3.5s of beam"
        );

        let mut app_base = test_app();
        setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
        start_game_with_weapons(&mut app_base);
        push(
            &mut app_base,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "target-uuid".into(),
                },
            },
        );
        tick(&mut app_base);
        push(
            &mut app_base,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app_base);
        {
            let mut beam = app_base.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 1.0 * 3.5;
        }
        tick(&mut app_base);

        let still_exists_base = app_base
            .world()
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|a| a.uuid == "target-uuid");
        assert!(still_exists_base, "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)");
    }

    // ── SetPhaserFrequency delegation tests ────────────────────────────────

    fn start_game_with_sensors_and_weapons(app: &mut App) {
        push(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain's Chair".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::Identify {
                token: "sensors".into(),
                name: "Spock".into(),
            },
        );
        tick(app);
        push(
            app,
            "sensors",
            ClientMessage::SelectStation {
                station: "Sensors".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(app);
        push(
            app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(app);
        push(app, "captain", ClientMessage::SetReady { ready: true });
        push(app, "sensors", ClientMessage::SetReady { ready: true });
        push(app, "weapons", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    #[test]
    fn tactical_holder_can_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SetPhaserFrequency { frequency: 0.8 },
        );
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!(
            (freq - 0.8).abs() < 1e-5,
            "Tactical holder should set phaser frequency to 0.8, got {freq}"
        );
    }

    #[test]
    fn sensors_holder_cannot_set_phaser_frequency() {
        // Delegation removed in B4 — only Tactical holder may set phaser frequency.
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        push(
            &mut app,
            "sensors",
            ClientMessage::SetPhaserFrequency { frequency: 0.9 },
        );
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "Sensors holder must NOT change phaser frequency, got {freq}"
        );
    }

    #[test]
    fn unrelated_console_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game(&mut app);
        push(
            &mut app,
            "captain",
            ClientMessage::SetPhaserFrequency { frequency: 0.9 },
        );
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!(
            (freq - 0.5).abs() < 1e-5,
            "Captain must NOT change phaser frequency, got {freq}"
        );
    }

    #[test]
    fn set_phaser_frequency_clamps_value() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SetPhaserFrequency { frequency: 1.5 },
        );
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!(
            (freq - 1.0).abs() < 1e-5,
            "frequency above 1.0 should clamp to 1.0, got {freq}"
        );

        push(
            &mut app,
            "weapons",
            ClientMessage::SetPhaserFrequency { frequency: -0.5 },
        );
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!(
            (freq - 0.0).abs() < 1e-5,
            "frequency below 0.0 should clamp to 0.0, got {freq}"
        );
    }

    // ── NPC / station phaser damage (issue #311) ──────────────────────────

    fn setup_npc_world(app: &mut App, npc_x: f32, npc_z: f32) {
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![crate::messages::EntitySnapshot {
                    uuid: "npc-1".into(),
                    position: Some([npc_x, 0.0, npc_z]),
                    tags: vec!["ship".into()],
                    ..Default::default()
                }],
                ..Default::default()
            }));
    }

    fn spawn_npc_entity(
        app: &mut App,
        npc_x: f32,
        npc_z: f32,
        max_hp: f32,
    ) -> bevy::ecs::entity::Entity {
        app.world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("npc-1".into()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    crate::messages::Console::CaptainChair,
                    max_hp,
                )])),
                Transform::from_xyz(npc_x, 0.0, npc_z),
            ))
            .id()
    }

    // ── Cycle 1: phaser beam reduces NPC hull ─────────────────────────────

    #[test]
    fn phaser_beam_damages_npc_entity_hull() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Accumulate damage but don't destroy
        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 10.0;
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 5.0;
        tick(&mut app);

        let hp = app
            .world()
            .get::<EntityConsoleHull>(npc_entity)
            .expect("NPC entity should still exist")
            .0
            .total_current();
        assert!(
            hp < 30.0,
            "NPC hull should be reduced after phaser hit, got {hp}"
        );
    }

    // ── Cycle 2: NPC at 0 HP is despawned and EntityDespawned broadcast ──

    #[test]
    fn phaser_beam_destroys_npc_entity_when_hull_reaches_zero() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Force lethal damage
        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 30.0;
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 5.0;
        let out = tick(&mut app);

        // ECS entity despawned
        assert!(
            app.world().get::<EntityConsoleHull>(npc_entity).is_none(),
            "NPC entity should be despawned after hull reaches 0"
        );

        // EntityDespawned wire message broadcast to all
        let despawned_msg = out
            .iter()
            .find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { uuid } if uuid == "npc-1"));
        assert!(
            despawned_msg.is_some(),
            "expected EntityDespawned {{ uuid: npc-1 }} broadcast"
        );

        // BeamEnded sent
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after NPC destruction"
        );

        // Beam cleared, cooldown started
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());
        assert!(app
            .world()
            .resource::<PhaserCooldown>()
            .is_bank_active("port"));
    }

    // ── NPC shields integration (#471) ────────────────────────────────────

    /// Spawn a shielded NPC: same as `spawn_npc_entity` but also attaches an
    /// `EntityShield` so the damage routing path is exercised end-to-end.
    fn spawn_shielded_npc_entity(
        app: &mut App,
        npc_x: f32,
        npc_z: f32,
        hull_max: f32,
        shield_max: f32,
        regen_per_sec: f32,
    ) -> bevy::ecs::entity::Entity {
        app.world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("npc-1".into()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    crate::messages::Console::CaptainChair,
                    hull_max,
                )])),
                crate::entity_spawner::EntityShield {
                    current_hp: shield_max,
                    max_hp: shield_max,
                    regen_per_sec,
                    broken: false,
                },
                Transform::from_xyz(npc_x, 0.0, npc_z),
            ))
            .id()
    }

    #[test]
    fn phaser_beam_damages_shielded_npc_routes_through_shield_first() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 0.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Apply 5 units of damage. With pierce=0 (default in test config),
        // the entire amount lands on the shield, hull is unchanged.
        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 5.0;
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 5.0;
        tick(&mut app);

        let shield = app
            .world()
            .get::<crate::entity_spawner::EntityShield>(npc_entity)
            .expect("NPC must still have shield component");
        assert!(
            shield.current_hp < 20.0,
            "shield must absorb damage, got {}",
            shield.current_hp
        );
        assert!(!shield.broken, "shield must still be intact");

        let hull_hp = app
            .world()
            .get::<EntityConsoleHull>(npc_entity)
            .expect("hull must still exist")
            .0
            .total_current();
        assert_eq!(hull_hp, 30.0, "hull must be untouched while shield holds");
    }

    #[test]
    fn phaser_beam_breaks_shield_then_leaks_to_hull() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 10.0, 0.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        // Apply 15 units of damage. With shield=10, shield depletes
        // and 5 units leak to hull.
        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 15.0;
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 5.0;
        tick(&mut app);

        let shield = app
            .world()
            .get::<crate::entity_spawner::EntityShield>(npc_entity)
            .expect("shield component must persist after break");
        assert_eq!(shield.current_hp, 0.0);
        assert!(shield.broken, "shield must latch broken once depleted");

        let hull_hp = app
            .world()
            .get::<EntityConsoleHull>(npc_entity)
            .expect("hull must exist")
            .0
            .total_current();
        assert!(
            hull_hp < 30.0 && hull_hp > 20.0,
            "hull must take only the leak (~5 units), got {hull_hp}"
        );
    }

    #[test]
    fn phaser_beam_post_break_skips_shield_routing_entirely() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        // Spawn with already-broken shield.
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("npc-1".into()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    crate::messages::Console::CaptainChair,
                    30.0,
                )])),
                crate::entity_spawner::EntityShield {
                    current_hp: 0.0,
                    max_hp: 20.0,
                    regen_per_sec: 0.0,
                    broken: true,
                },
                Transform::from_xyz(0.0, 0.0, -20.0),
            ))
            .id();

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 5.0;
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 5.0;
        tick(&mut app);

        let hull_hp = app
            .world()
            .get::<EntityConsoleHull>(npc_entity)
            .expect("hull must exist")
            .0
            .total_current();
        // Hull must take damage (broken shield does not absorb).
        // We don't pin the exact amount because the beam tick may
        // accumulate additional damage during the same frame; we just
        // verify the broken shield path didn't absorb any of it.
        assert!(
            hull_hp < 30.0,
            "broken shield must let damage through to hull, got {hull_hp}"
        );
        let shield = app
            .world()
            .get::<crate::entity_spawner::EntityShield>(npc_entity)
            .expect("shield component must persist");
        assert_eq!(
            shield.current_hp, 0.0,
            "broken shield current_hp must remain 0, got {}",
            shield.current_hp
        );
        assert!(shield.broken, "shield must remain broken");
    }

    #[test]
    fn shield_regen_advances_npc_shield_below_max() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 5.0);

        // Damage the shield to 10 HP.
        if let Some(mut shield) = app
            .world_mut()
            .get_mut::<crate::entity_spawner::EntityShield>(npc_entity)
        {
            shield.current_hp = 10.0;
        }

        // Advance time. The Bevy `Time` resource advances on each `app.update()`
        // call; we tick a few frames and expect regen to push hp upward.
        for _ in 0..3 {
            tick(&mut app);
        }

        let shield = app
            .world()
            .get::<crate::entity_spawner::EntityShield>(npc_entity)
            .expect("shield must persist");
        // We don't assert exact values (frame timing varies in tests) but we
        // verify regen is making forward progress and not stuck at 10.
        assert!(
            shield.current_hp > 10.0,
            "shield must regen between ticks, got {}",
            shield.current_hp
        );
        assert!(
            shield.current_hp <= 20.0,
            "shield must clamp to max_hp, got {}",
            shield.current_hp
        );
        assert!(!shield.broken);
    }

    // ── Cycle 3: AiEntityDestroyed message written on NPC destruction ─────

    #[test]
    fn phaser_beam_emits_ai_entity_destroyed_on_npc_kill() {
        #[derive(Resource, Default)]
        struct DestroyedBox(Vec<crate::ai_plugin::AiEntityDestroyed>);

        let mut app = test_app();
        app.init_resource::<DestroyedBox>();
        app.add_systems(
            bevy::app::Update,
            |mut r: bevy::ecs::prelude::MessageReader<crate::ai_plugin::AiEntityDestroyed>,
             mut b: bevy::ecs::prelude::ResMut<DestroyedBox>| {
                for ev in r.read() {
                    b.0.push(ev.clone());
                }
            },
        );

        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);
        spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(
            &mut app,
            "weapons",
            ClientMessage::ControlSystem {
                target: crate::system_registry::tactical_system_id(),
                payload: SystemControlPayload::SetTarget {
                    uuid: "npc-1".into(),
                },
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        tick(&mut app);

        app.world_mut()
            .resource_mut::<ActiveBeam>()
            .damage_accumulator = 30.0;
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 5.0;
        tick(&mut app);
        tick(&mut app); // second tick allows PostUpdate-equivalent collector to drain the message

        let destroyed_events = app.world().resource::<DestroyedBox>();
        assert!(
            destroyed_events.0.iter().any(|e| e.entity_uuid == "npc-1"),
            "AiEntityDestroyed must be emitted with entity_uuid 'npc-1' so on_destroyed triggers fire"
        );
    }

    // ── NPC as shooter: handle_fire_phaser_npc ────────────────────────────

    /// Set up `AiTokenRegistry`, an NPC entity with `AiControllerComponent` +
    /// `EntityPhaserState`, and a target entity.
    fn setup_npc_shooter(
        app: &mut App,
        npc_uuid: &str,
        target_uuid: &str,
        target_x: f32,
        target_z: f32,
    ) -> (bevy::ecs::entity::Entity, bevy::ecs::entity::Entity) {
        use crate::ai::AiMemory;
        use crate::ai_plugin::{AiControllerComponent, EntityPhaserState};
        use crate::entity_spawner::{EntityConsoleHull, EntityUuid};

        // Register the AI token.
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register(npc_uuid);
        }

        // Build AiMemory with the target pre-selected.
        let target_as_uuid = uuid::Uuid::parse_str(target_uuid).ok();
        let memory = AiMemory {
            target: target_as_uuid,
            ..Default::default()
        };

        // Spawn NPC entity facing toward negative-Z (yaw = 0 → forward = -Z).
        let npc_entity = app
            .world_mut()
            .spawn((
                EntityUuid(npc_uuid.to_string()),
                AiControllerComponent {
                    memory,
                    entity_uuid: npc_uuid.to_string(),
                    forward_speed: 0.0,
                    last_helm_intent: None,
                },
                EntityPhaserState::default(),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        // Spawn target entity.
        let target_entity = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid.to_string()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    crate::messages::Console::CaptainChair,
                    50.0,
                )])),
                Transform::from_xyz(target_x, 0.0, target_z),
            ))
            .id();

        (npc_entity, target_entity)
    }

    #[test]
    fn npc_fire_phaser_activates_entity_phaser_state() {
        // NPC entity at origin, target directly ahead (negative-Z), within beam range.
        // Sending a FirePhaser InboundMessage for the NPC's ai: token should set
        // `EntityPhaserState::beam_active = true` after one update.
        use crate::ai_plugin::{AiTokenRegistry, EntityPhaserState};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000001";
        let target_uuid = "00000000-0000-0000-0000-000000000002";

        let (npc_entity, _target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid, 0.0, -20.0);

        // Send FirePhaser as the NPC's synthetic token.
        let ai_token = format!("ai:{}", npc_uuid);
        push(
            &mut app,
            &ai_token,
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        app.update();

        let phaser_state = app
            .world()
            .get::<EntityPhaserState>(npc_entity)
            .expect("NPC entity must have EntityPhaserState");
        assert!(
            phaser_state.beam_active,
            "EntityPhaserState::beam_active should be true after NPC fires phaser via ai: token"
        );
    }

    #[test]
    fn npc_beam_tick_applies_damage_to_target_hull() {
        // With an active NPC beam, each tick of handle_fire_phaser_npc reduces
        // the target's EntityConsoleHull.
        use crate::ai_plugin::{AiTokenRegistry, EntityPhaserState};
        use crate::entity_spawner::EntityConsoleHull;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000003";
        let target_uuid_str = "00000000-0000-0000-0000-000000000004";

        let (npc_entity, target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        // Activate the beam directly (no FirePhaser required for this test).
        let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();
        {
            let mut ps = app
                .world_mut()
                .get_mut::<EntityPhaserState>(npc_entity)
                .unwrap();
            ps.beam_active = true;
            ps.beam_target = Some(target_uuid_parsed);
            ps.beam_remaining_secs = 10.0;
        }

        let hp_before = app
            .world()
            .get::<EntityConsoleHull>(target_entity)
            .unwrap()
            .0
            .total_current();

        // Run several ticks so damage accumulates.
        for _ in 0..10 {
            app.update();
        }

        let hp_after = app
            .world()
            .get::<EntityConsoleHull>(target_entity)
            .unwrap()
            .0
            .total_current();
        assert!(
            hp_after < hp_before,
            "target hull must decrease as NPC beam ticks (before={hp_before}, after={hp_after})"
        );
    }

    #[test]
    fn npc_beam_tick_applies_damage_to_player_ship_through_shields() {
        // When the beam target is the player ship (has Ship marker), damage
        // must route through shields → hull resource, not EntityConsoleHull.
        use crate::ai_plugin::{AiTokenRegistry, EntityPhaserState};
        use crate::entity_spawner::EntityUuid;
        use crate::server_app::{LocalShip, Ship, ShipAttackedThisTick};
        use crate::shield::ShieldConfig;
        use crate::simulation::{GameOverReason, ShipShields};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<GameOverReason>();

        // Insert shields so the shield-routing path is exercised.
        let shield_config = ShieldConfig {
            max_hp: 100,
            regen_per_sec: 0.0,
            num_facings: 4,
            ..Default::default()
        };
        app.insert_resource(ShipShields(crate::shield::ShieldSystem::new(
            &shield_config,
        )));

        let npc_uuid = "00000000-0000-0000-0000-000000000010";
        let player_uuid = "00000000-0000-0000-0000-000000000011";
        let player_uuid_parsed = uuid::Uuid::parse_str(player_uuid).unwrap();

        // Spawn the player ship entity with Ship marker, EntityUuid, EntityConsoleHull.
        let _player_entity = app
            .world_mut()
            .spawn((
                EntityUuid(player_uuid.to_string()),
                Ship,
                LocalShip,
                crate::entity_spawner::EntityConsoleHull(crate::damage::ConsoleHull::from_config(
                    &[
                        (Console::Helm, 25.0),
                        (Console::Tactical, 25.0),
                        (Console::Power, 25.0),
                        (Console::Shields, 25.0),
                    ],
                )),
                Transform::from_xyz(0.0, 0.0, -10.0),
            ))
            .id();

        // Spawn NPC entity (same pattern as setup_npc_shooter).
        let npc_entity = {
            use crate::ai::AiMemory;
            let memory = AiMemory {
                target: Some(player_uuid_parsed),
                ..Default::default()
            };

            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register(npc_uuid);

            app.world_mut()
                .spawn((
                    EntityUuid(npc_uuid.to_string()),
                    crate::ai_plugin::AiControllerComponent {
                        memory,
                        entity_uuid: npc_uuid.to_string(),
                        forward_speed: 0.0,
                        last_helm_intent: None,
                    },
                    EntityPhaserState::default(),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                ))
                .id()
        };

        let hull_before = app
            .world()
            .resource::<ShipHullIntegrity>()
            .0
            .total_current();
        let shields_hp_before: Vec<i32> = app
            .world()
            .resource::<ShipShields>()
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .collect();
        let shields_sum_before: i32 = shields_hp_before.iter().sum();

        // Activate the beam directly targeting the player ship.
        {
            let mut ps = app
                .world_mut()
                .get_mut::<EntityPhaserState>(npc_entity)
                .unwrap();
            ps.beam_active = true;
            ps.beam_target = Some(player_uuid_parsed);
            ps.beam_remaining_secs = 10.0;
        }

        for _ in 0..10 {
            app.update();
        }

        let hull_after = app
            .world()
            .resource::<ShipHullIntegrity>()
            .0
            .total_current();
        let shields_sum_after: i32 = app
            .world()
            .resource::<ShipShields>()
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum();

        let hull_lost = hull_before - hull_after;
        let shields_lost = shields_sum_before - shields_sum_after;

        assert!(
            hull_lost > 0.0 || shields_lost > 0,
            "NPC beam must damage player ship: hull {hull_before}->{hull_after} ({hull_lost}), shields {shields_sum_before}->{shields_sum_after} ({shields_lost})"
        );
        assert!(
            app.world().resource::<ShipAttackedThisTick>().0,
            "NPC beam targeting the player ship must mark the ship as attacked for Captain AI"
        );
    }

    #[test]
    fn npc_beam_cooldown_starts_after_beam_expires() {
        // When an NPC's beam_remaining_secs reaches zero, cooldown_remaining must
        // be set to a positive value and beam_active must become false.
        use crate::ai_plugin::{AiTokenRegistry, EntityPhaserState};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000005";
        let target_uuid_str = "00000000-0000-0000-0000-000000000006";

        let (npc_entity, _target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();
        {
            let mut ps = app
                .world_mut()
                .get_mut::<EntityPhaserState>(npc_entity)
                .unwrap();
            ps.beam_active = true;
            ps.beam_target = Some(target_uuid_parsed);
            ps.beam_remaining_secs = 0.001; // expires on first tick
        }

        app.update(); // beam expires
        app.update(); // cooldown ticked

        let ps = app.world().get::<EntityPhaserState>(npc_entity).unwrap();
        assert!(
            !ps.beam_active,
            "beam_active must be false after beam expires"
        );
        assert!(
            ps.cooldown_remaining > 0.0,
            "cooldown_remaining must be positive after beam ends, got {}",
            ps.cooldown_remaining
        );
    }

    // ── End-to-end: tick_ai_controllers → InboundMessage → handle_fire_phaser_npc ──

    /// Build an app that includes BOTH `WeaponsPlugin` AND `AiPlugin` together
    /// with all their required resources, so the full routing path can be tested:
    /// `tick_ai_controllers` emits a `FirePhaser` `InboundMessage` which
    /// `handle_fire_phaser_npc` picks up and converts into `EntityPhaserState::beam_active`.
    fn combined_test_app() -> App {
        use crate::ai_plugin::AiPlugin;
        use crate::config_cache::FactionRegistryResource;

        let mut app = test_app();
        app.add_plugins(AiPlugin)
            .insert_resource(FactionRegistryResource(
                crate::config_cache::get_faction_registry(),
            ));
        app
    }

    #[test]
    fn tick_ai_controllers_fire_phaser_routes_through_handle_fire_phaser_npc() {
        // Full end-to-end test: an NPC with a Destroy doctrine and a pre-selected
        // target directly in its forward arc causes `tick_ai_controllers` to write
        // a `FirePhaser` `InboundMessage`, which `handle_fire_phaser_npc` picks up
        // and sets `EntityPhaserState::beam_active`.
        use crate::ai_plugin::{AiControllerComponent, EntityPhaserState};
        use crate::damage::ConsoleHull;
        use crate::entity_config::{BehaviourConfig, DoctrineObjective};
        use crate::entity_spawner::{EntityConsoleHull, EntityUuid, WeaponsConsoleSection};
        use crate::messages::{Console, GamePhase};
        use bevy::prelude::State;

        let mut app = combined_test_app();

        // Put the simulation in InProgress so tick_ai_controllers runs.
        app.world_mut()
            .insert_resource(State::new(GamePhase::InProgress));

        let beam_range = 50.0_f32;
        let npc_uuid_str = "ee000000-0000-0000-0000-000000000010";
        let target_uuid_str = "ee000000-0000-0000-0000-000000000011";
        let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();

        // Doctrine: single Destroy objective at high priority — always scores > 0.
        let behaviour = BehaviourConfig {
            doctrine: vec![DoctrineObjective {
                id: "destroy-hostiles".into(),
                text: "Destroy target".into(),
                directive_kind: Some("Destroy".into()),
                base_priority: 35.0,
                target_speed: 0.9,
                maintain_range: 25.0,
                ..Default::default()
            }],
            ..Default::default()
        };

        // Spawn NPC at origin, facing -Z (yaw = 0 → forward = -Z).
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::BehaviourSection(behaviour),
                EntityUuid(npc_uuid_str.to_string()),
                EntityPhaserState::default(),
                WeaponsConsoleSection(crate::entity_config::WeaponsConsoleConfig {
                    torpedo_arc_color: vec![],
                    power_multipliers: None,
                    phaser_banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 360.0,
                        auto_arc_deg: 360.0,
                        beam_range,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: Some(0.0),
                        marker: None,
                    }],
                    radar: None,
                }),
                EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)])),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        // Spawn target directly ahead (-Z), well within beam range.
        let _target = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid_str.to_string()),
                EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 200.0)])),
                Transform::from_xyz(0.0, 0.0, -10.0),
            ))
            .id();

        // Tick 1: `attach_controllers_on_spawn` runs → AiControllerComponent attached
        //         and token registered in AiTokenRegistry.
        app.update();

        // Set memory.target so `operate_weapons` selects it (target is in WorldView).
        {
            let mut ctrl = app
                .world_mut()
                .get_mut::<AiControllerComponent>(npc_entity)
                .unwrap();
            ctrl.memory.target = Some(target_uuid_parsed);
        }

        // Tick 2: `tick_ai_controllers` emits FirePhaser InboundMessage.
        // Tick 3: `handle_fire_phaser_npc` reads the message (messages are
        //         available to readers on the tick after they are written).
        app.update();
        app.update();

        let ps = app
            .world()
            .get::<EntityPhaserState>(npc_entity)
            .expect("NPC must still have EntityPhaserState");
        assert!(
            ps.beam_active,
            "beam_active must be true after tick_ai_controllers → InboundMessage → handle_fire_phaser_npc routing"
        );
    }

    // ── Radar blip tests ─────────────────────────────────────────────────────

    fn tactical_blips(app: &mut App) -> Vec<RadarBlip> {
        use crate::messages::{SystemBlackboard, SystemId};
        use crate::server_app::SystemBlackboards;
        use crate::system_registry::TACTICAL_SYSTEM_ID;
        let bbs = app.world().resource::<SystemBlackboards>();
        match bbs.0.get(&SystemId(TACTICAL_SYSTEM_ID.to_string())) {
            Some(SystemBlackboard::Weapons(bb)) => bb.blips.clone(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn radar_blip_appears_for_asteroid_within_tactical_range() {
        let mut app = test_app();
        // Configure tactical radar to show asteroids with range 300.
        {
            let mut cfg = app
                .world_mut()
                .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
            cfg.0.tactical_radar_shows = vec!["asteroid".into()];
            cfg.0.tactical_radar_range = 300.0;
        }
        // Asteroid 50 units ahead (z=-50, within 300 range).
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game(&mut app);
        tick(&mut app); // first InProgress tick → publish runs

        let blips = tactical_blips(&mut app);

        assert_eq!(blips.len(), 1, "expected one blip for in-range asteroid");
        assert_eq!(blips[0].uuid, "target-uuid");
        assert_eq!(blips[0].kind, "asteroid");
        // Forward (z=-50) at yaw=0 maps to radar_y > 0 (forward = up).
        assert!(
            blips[0].radar_y > 0.0,
            "asteroid ahead should have positive radar_y"
        );
        assert!(
            (blips[0].radar_x).abs() < 1e-4,
            "asteroid directly ahead has radar_x ≈ 0"
        );
    }

    #[test]
    fn asteroid_beyond_tactical_range_not_in_blips() {
        let mut app = test_app();
        {
            let mut cfg = app
                .world_mut()
                .resource_mut::<crate::lobby::server::ShipClientConfigResource>();
            cfg.0.tactical_radar_shows = vec!["asteroid".into()];
            cfg.0.tactical_radar_range = 100.0;
        }
        // Asteroid 200 units ahead — beyond the 100-unit radar range.
        setup_weapons_world(&mut app, 0.0, -200.0);
        start_game(&mut app);
        tick(&mut app);

        let blips = tactical_blips(&mut app);
        assert!(
            blips.is_empty(),
            "asteroid beyond tactical range must not appear in blips"
        );
    }

    // ── TacticalAiController tests ─────────────────────────────────────────

    fn set_tactical_control_source(
        app: &mut App,
        source: crate::ship::control_source::ControlSource,
    ) {
        let world = app.world_mut();
        let mut q =
            world.query_filtered::<&mut ShipSystemControlSources, With<crate::simulation::Ship>>();
        for mut cs in q.iter_mut(world) {
            cs.0.set(crate::system_registry::tactical_system_id(), source);
        }
    }

    fn spawn_asteroid_target(app: &mut App, uuid: &str, x: f32, z: f32) {
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            AsteroidUuid(uuid.into()),
            crate::entity_spawner::EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                crate::messages::Console::CaptainChair,
                30.0,
            )])),
            Transform::from_xyz(x, 0.0, z),
        ));
    }

    fn spawn_entity_target(app: &mut App, uuid: &str, x: f32, z: f32) {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid(uuid.into()),
            Transform::from_xyz(x, 0.0, z),
        ));
    }

    fn insert_destroy_objective_blackboard(app: &mut App, target: &str, score: f32) {
        use crate::messages::{
            AiDirective, ObjectiveSnapshot, ObjectiveSource, ObjectiveStatus, ScoredObjective,
            SystemAffinity, SystemBlackboard, ViewscreenBlackboard,
        };
        use crate::server_app::SystemBlackboards;

        let mut bbs = SystemBlackboards::default();
        let mut viewscreen = ViewscreenBlackboard::default();
        viewscreen.scored_objectives = vec![ScoredObjective {
            id: format!("obj-destroy-{target}"),
            score,
            directive: AiDirective::Destroy {
                target: target.into(),
            },
            source: ObjectiveSource::Mission,
            relevance: vec![
                SystemAffinity::Helm,
                SystemAffinity::Weapons,
                SystemAffinity::Captain,
            ],
            snapshot: ObjectiveSnapshot {
                id: format!("obj-destroy-{target}"),
                text: format!("Destroy {target}"),
                mandatory: true,
                status: ObjectiveStatus::Active,
                targets: vec![target.into()],
                source: ObjectiveSource::Mission,
            },
        }];
        bbs.0.insert(
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(viewscreen),
        );
        app.insert_resource(bbs);
    }

    #[test]
    fn tactical_ai_selects_named_destroy_objective_target() {
        let mut app = test_app();
        let target_uuid = uuid::Uuid::new_v4().to_string();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        spawn_entity_target(&mut app, &target_uuid, 0.0, -30.0);
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .name_to_uuid
            .insert("wave_1".into(), target_uuid.clone());
        insert_destroy_objective_blackboard(&mut app, "wave_1", 80.0);

        tick(&mut app);

        assert_eq!(
            app.world().resource::<WeaponsTarget>().0.as_deref(),
            Some(target_uuid.as_str()),
            "Tactical AI must lock the live entity named by the Destroy objective"
        );
    }

    #[test]
    fn tactical_ai_ignores_missing_destroy_objective_target() {
        let mut app = test_app();
        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        insert_destroy_objective_blackboard(&mut app, "wave_404", 80.0);

        tick(&mut app);

        assert!(
            app.world().resource::<WeaponsTarget>().0.is_none(),
            "Tactical AI must not lock an arbitrary target when the objective target is missing"
        );
    }

    #[test]
    fn ai_fires_torpedo_when_ai_controls_unclaimed_station() {
        // Unclaimed station + Ai ControlSource → operate_tactical_ai fires unconditionally.
        let mut app = test_app();

        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());
        load_tube_now(&mut app, "fore_port");
        // Asteroid at (0, -30) → bearing 0 from ship at origin yaw=0 → in ForePort arc.
        spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "AI should fire TorpedoLaunched when controlling an unclaimed Tactical station"
        );
    }

    fn set_tactical_station_rating(app: &mut App, rating: &str) {
        let rating = rating.to_string();
        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&mut crate::ship_plugin::ActiveStationRatings, With<crate::simulation::Ship>>();
        for mut ratings in q.iter_mut(world) {
            ratings.0.insert(
                crate::messages::StationId("tactical".into()),
                rating.clone(),
            );
        }
    }

    #[test]
    fn ai_stops_firing_when_rating_switches_to_std() {
        // Occupied station: AI fires when rating is Assisted (has torpedo_auto_fire
        // in ai_tuning), stops when rating is Std (no ai_tuning).
        let mut app = test_app();

        // Assign a human holder so the ai_tuning gate is active.
        push(
            &mut app,
            "weapons",
            ClientMessage::Identify {
                token: "weapons".into(),
                name: "Bob".into(),
            },
        );
        tick(&mut app);
        push(
            &mut app,
            "weapons",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        tick(&mut app);

        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        // Set rating to Assisted (has torpedo_auto_fire in ai_tuning).
        set_tactical_station_rating(&mut app, "Assisted");
        app.world_mut().resource_mut::<WeaponsTarget>().0 = Some("target-uuid".into());
        load_tube_now(&mut app, "fore_port");
        spawn_asteroid_target(&mut app, "target-uuid", 0.0, -30.0);

        // First tick — AI should fire with Assisted rating.
        let out1 = tick(&mut app);
        assert!(
            out1.iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "AI should fire TorpedoLaunched when rating is Assisted"
        );

        // Reload the tube (launch consumed it) so the only gate is the rating.
        load_tube_now(&mut app, "fore_port");

        // Switch to Std rating (no torpedo_auto_fire in ai_tuning).
        set_tactical_station_rating(&mut app, "Std");

        // Second tick — AI must not fire.
        let out2 = tick(&mut app);
        assert!(
            !out2
                .iter()
                .any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "AI must not fire TorpedoLaunched when rating is Std"
        );
    }
}

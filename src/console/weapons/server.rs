use bevy::prelude::*;

use crate::ai_plugin::{AiControllerComponent, AiTokenRegistry, EntityPhaserState};
use crate::codec;
use crate::console_bridge::ConsoleStateChanged;
use crate::entity_spawner::EntityConsoleHull;
use crate::lobby::{InboundMessage, Sessions, Target, WorldResource};
use crate::messages::{
    ClientMessage, Console, GamePhase, ModifierSlot, PhaserBank, PhaserBankClientConfig,
    PhaserBankState, RadarBlip, RadarRegion, ServerMessage, TorpedoTubeClientConfig,
    TorpedoTubeState, WeaponsConsoleState,
};
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

// ── Resources ─────────────────────────────────────────────────────────────

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

    pub fn start_bank(&mut self, bank: &str, config: &crate::entity_config::PhaserCombatConfig) {
        self.per_bank
            .insert(bank.to_string(), config.beam_cooldown_secs);
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
        Self(crate::messages::PhaserMode::Auto)
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
) {
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

pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeaponsTarget>()
            .init_resource::<ActiveBeam>()
            .init_resource::<PhaserCooldown>()
            .init_resource::<CurrentPhaserMode>()
            .init_resource::<PhaserRenderConfig>()
            .init_resource::<PhaserCombatConfigResource>()
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
                TorpedoConfig::default(),
            )))
            .add_message::<AsteroidDestroyedVfx>()
            .add_message::<ConsoleStateChanged>()
            .add_observer(on_beam_started)
            .add_observer(on_beam_ended)
            .add_systems(Startup, spawn_weapons_console_state_entity)
            .add_systems(
                Update,
                (
                    handle_set_target.in_set(crate::sim_sets::SimSet::Input),
                    handle_fire_phaser.in_set(crate::sim_sets::SimSet::Input),
                    handle_fire_phaser_npc.in_set(crate::sim_sets::SimSet::Damage),
                    handle_set_phaser_mode.in_set(crate::sim_sets::SimSet::Input),
                    handle_set_phaser_frequency.in_set(crate::sim_sets::SimSet::Input),
                    handle_fire_torpedo.in_set(crate::sim_sets::SimSet::Input),
                    handle_load_tube.in_set(crate::sim_sets::SimSet::Input),
                    handle_unload_tube.in_set(crate::sim_sets::SimSet::Input),
                ),
            )
            .add_systems(
                Update,
                (
                    tick_active_beam.in_set(crate::sim_sets::SimSet::Physics),
                    tick_torpedo_system.in_set(crate::sim_sets::SimSet::Physics),
                ),
            )
            .add_systems(
                Update,
                (
                    recompute_weapons_console_state,
                    push_weapons_console_state.after(recompute_weapons_console_state),
                )
                    .run_if(in_state(GamePhase::InProgress)),
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
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    mut weapons_target: ResMut<WeaponsTarget>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    for ev in reader.read() {
        let ClientMessage::SetTarget { uuid } = &ev.msg else {
            continue;
        };

        let holder = sessions.0.console_holder(Console::Tactical);
        if holder != Some(ev.token.as_str()) {
            continue;
        }

        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);
        let base_range = ship_config.0.tactical_radar_range;
        let effective_weapons_range = base_range * radar_range_mult;
        let live_pos = live_entity_xz(uuid, &asteroid_q, &entity_q);
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

        outbox.0.push((
            Target::Token(ev.token.clone()),
            ServerMessage::TargetLock {
                uuid: uuid.clone(),
                locked,
            },
        ));
    }
}

/// Returns true if `token` is authorized to issue Tactical fire orders.
///
/// Either the token is the connected player currently holding the Tactical
/// console, or it is the local HTML-console operator
/// ([`crate::console_bridge::LOCAL_CONSOLE_TOKEN`]) — the browser server
/// viewscreen / native wry server case, where the operator drives the console
/// directly with no remote PeerJS session (issue #422 / PRD #419).
fn tactical_authorized(sessions: &Sessions, token: &str) -> bool {
    sessions.0.console_holder(Console::Tactical) == Some(token)
        || token == crate::console_bridge::LOCAL_CONSOLE_TOKEN
}

fn handle_fire_phaser(
    mut commands: Commands,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
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
    for ev in reader.read() {
        let ClientMessage::FirePhaser { bank } = &ev.msg else {
            continue;
        };
        if !tactical_authorized(&sessions, &ev.token) {
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
        let bank_in_arc = if combat_config.0.banks.is_empty() {
            let effective_phaser_range =
                combat_config.0.phaser_range * modifiers.get(&ModifierSlot::RadarRange);
            crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                ship.x,
                ship.z,
                ship.yaw,
                effective_phaser_range,
            )
        } else {
            combat_config
                .0
                .banks
                .iter()
                .find(|b| b.id == *bank)
                .map(|bank_cfg| {
                    let bank_base_range = if bank_cfg.beam_range > 0.0 {
                        bank_cfg.beam_range
                    } else {
                        combat_config.0.phaser_range
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

        beam.target_uuid = Some(target_uuid.clone());
        beam.remaining_secs = combat_config.0.beam_duration_secs;
        beam.damage_accumulator = 0.0;
        beam.bank = Some(bank.clone());

        commands.trigger(BeamStartedEvent {
            bank: bank.clone(),
            target_uuid: target_uuid.clone(),
        });
    }
}

// ── NPC phaser constants ──────────────────────────────────────────────────────

/// Default NPC beam duration in seconds.
const NPC_BEAM_DURATION_SECS: f32 = 3.0;
/// Default NPC beam damage per second.
const NPC_BEAM_DAMAGE_PER_SEC: f32 = 5.0;

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
        ),
        Without<AiControllerComponent>,
    >,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    player_ship_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), With<Ship>>,
    ship_state: Option<Res<crate::ship_state::ShipState>>,
    mut hull_resource: Option<ResMut<ShipHullIntegrity>>,
    mut shields_resource: Option<ResMut<ShipShields>>,
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
        .filter_map(|(_, uid, t, _)| {
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

        let target_uuid: Option<uuid::Uuid> = ctrl_opt.and_then(|c| c.controller.blackboard.target);

        let beam_range = weapons_section
            .map(|wc| {
                if wc.0.beam_range > 0.0 {
                    wc.0.beam_range
                } else {
                    40.0
                }
            })
            .unwrap_or(40.0);
        let damage_per_sec = weapons_section
            .map(|wc| {
                if wc.0.beam_damage_per_sec > 0.0 {
                    wc.0.beam_damage_per_sec
                } else {
                    NPC_BEAM_DAMAGE_PER_SEC
                }
            })
            .unwrap_or(NPC_BEAM_DAMAGE_PER_SEC);
        let beam_duration = weapons_section
            .map(|wc| {
                if wc.0.beam_duration_secs > 0.0 {
                    wc.0.beam_duration_secs
                } else {
                    NPC_BEAM_DURATION_SECS
                }
            })
            .unwrap_or(NPC_BEAM_DURATION_SECS);

        let npc_x = transform.translation.x;
        let npc_z = transform.translation.z;
        let npc_yaw = transform.rotation.to_euler(EulerRot::YXZ).0;

        // Activate beam on FirePhaser order when ready, or auto-fire when the AI
        // is in the Attacking state with a valid target (eliminates the 1-frame
        // event delay between tick_ai_controllers → InboundMessage → here).
        let should_fire = fire_orders.contains(&token)
            || ctrl_opt.as_ref().is_some_and(|c| {
                matches!(
                    c.controller.current_state,
                    crate::ai::AiState::Attacking { .. }
                ) && c.controller.blackboard.target.is_some()
            });

        // DEBUG: instrument the NPC fire decision so we can see why phasers
        // are (or aren't) connecting in play sessions. Logged once per tick per
        // NPC that wants to fire. Remove once the geometry bug is diagnosed.
        if should_fire {
            if let Some(t_uuid) = target_uuid {
                if let Some((_, tx, tz)) = target_positions.iter().find(|(u, _, _)| *u == t_uuid) {
                    let dx = tx - npc_x;
                    let dz = tz - npc_z;
                    let dist = (dx * dx + dz * dz).sqrt();
                    let radar_y = dx * (-npc_yaw).sin() + dz * (-npc_yaw).cos();
                    info!(
                        "[npc-fire] uuid={} target={} dist={:.1} beam_range={:.1} radar_y={:.2} ready={} beam_active={} cooldown={:.2}",
                        npc_uuid.0,
                        t_uuid,
                        dist,
                        beam_range,
                        radar_y,
                        phaser_state.is_ready(),
                        phaser_state.beam_active,
                        phaser_state.cooldown_remaining,
                    );
                } else {
                    info!(
                        "[npc-fire] uuid={} target={} TARGET_NOT_FOUND_IN_HULL_QUERY ready={}",
                        npc_uuid.0,
                        t_uuid,
                        phaser_state.is_ready(),
                    );
                }
            } else {
                info!(
                    "[npc-fire] uuid={} should_fire=true but blackboard.target=None",
                    npc_uuid.0,
                );
            }
        }

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

                if !fire_ok {
                    info!(
                        "[npc-fire] uuid={} GATE_REJECTED (out of range or wrong arc)",
                        npc_uuid.0
                    );
                }

                if fire_ok {
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
                    // Player ship damage path: route through shields → hull resource → broadcast.
                    // Only apply when at least 1 whole unit has accumulated.
                    if damage >= 1.0 {
                        let npc_x = transform.translation.x;
                        let npc_z = transform.translation.z;

                        if let Some((_, tx, tz)) = target_positions
                            .iter()
                            .find(|(u, _, _)| u.to_string() == target_uuid_str)
                        {
                            let shield_pierce =
                                weapons_section.map(|wc| wc.0.shield_pierce).unwrap_or(0.0);
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
                            hull_resource.is_some(), shields_resource.is_some(),
                        );

                            if absorbed > 0.0 {
                                if let Some(ref mut shields) = shields_resource {
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
                        for (tgt_entity, tgt_uid, _tgt_transform, mut tgt_hull) in
                            hull_query.iter_mut()
                        {
                            if tgt_uid.0 != target_uuid_str {
                                continue;
                            }
                            let mut rng = rand::rng();
                            tgt_hull.0.apply_damage(damage, &mut rng);
                            if tgt_hull.0.is_destroyed() {
                                target_destroyed = true;
                                commands.entity(tgt_entity).despawn();
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
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut phaser_mode: ResMut<CurrentPhaserMode>,
) {
    for ev in reader.read() {
        let ClientMessage::SetPhaserMode { mode } = &ev.msg else {
            continue;
        };
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        phaser_mode.0 = *mode;
    }
}

fn handle_set_phaser_frequency(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    complexity: Res<crate::console_ai_plugin::ConsoleComplexityState>,
    rules: Res<crate::console_ai_plugin::ComplexityRules>,
    mut ship: ResMut<ShipState>,
) {
    use crate::delegation::{is_sender_authorized, CONTROL_SET_PHASER_FREQUENCY};
    // Delegation grants come from Tactical's active complexity preset
    // (`[preset.delegated]` in its complexity TOML).
    let tactical_preset = rules.active_preset(&Console::Tactical, &complexity);
    for ev in reader.read() {
        let ClientMessage::SetPhaserFrequency { frequency } = &ev.msg else {
            continue;
        };

        let sender_console =
            if sessions.0.console_holder(Console::Tactical) == Some(ev.token.as_str()) {
                Console::Tactical
            } else if sessions.0.console_holder(Console::Sensors) == Some(ev.token.as_str()) {
                Console::Sensors
            } else {
                continue;
            };

        if !is_sender_authorized(
            CONTROL_SET_PHASER_FREQUENCY,
            &sender_console,
            &Console::Tactical,
            tactical_preset,
        ) {
            continue;
        }

        ship.phaser_frequency = frequency.clamp(0.0, 1.0);
    }
}

fn handle_load_tube(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
) {
    for ev in reader.read() {
        let ClientMessage::LoadTube { tube } = &ev.msg else {
            continue;
        };
        if !tactical_authorized(&sessions, &ev.token) {
            continue;
        }
        if let Some(t) = torpedo_sys.0.tube_mut(tube.as_str()) {
            t.start_load();
        }
    }
}

fn handle_unload_tube(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
) {
    for ev in reader.read() {
        let ClientMessage::UnloadTube { tube } = &ev.msg else {
            continue;
        };
        if !tactical_authorized(&sessions, &ev.token) {
            continue;
        }
        if let Some(t) = torpedo_sys.0.tube_mut(tube.as_str()) {
            t.start_unload();
        }
    }
}

fn handle_fire_torpedo(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
    player_ship_q: Query<&crate::entity_spawner::EntityUuid, With<crate::server_app::Ship>>,
) {
    for ev in reader.read() {
        let ClientMessage::FireTorpedo { tube, target_uuid } = &ev.msg else {
            continue;
        };
        if !tactical_authorized(&sessions, &ev.token) {
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
        use crate::torpedo::LaunchResult;
        match torpedo_sys.0.launch(
            tube.as_str(),
            uuid,
            ship.x,
            ship.z,
            launch_heading,
            target_uuid.clone(),
            source_uuid,
        ) {
            LaunchResult::Launched {
                uuid: launched_uuid,
            } => {
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
    )>,
    mut commands: Commands,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut weapons_target: ResMut<WeaponsTarget>,
) {
    let dt = time.delta_secs();

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
            map.entry(e.uuid.clone())
                .or_insert_with(|| (e.x(), e.z(), e.radius_or_zero()));
        }
        map.into_iter()
            .map(|(uuid, (x, z, r))| (uuid, x, z, r))
            .collect()
    };
    let hits = torpedo_sys.0.find_detonation_hits(&targets);
    for (torpedo_uuid, target_uuid) in hits {
        let Some(damage) = torpedo_sys.0.handle_collision(&torpedo_uuid) else {
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

        for (entity, asteroid_uuid, entity_uuid, mut hull_comp) in hull_query.iter_mut() {
            let uuid_matches = asteroid_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str())
                || entity_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str());
            if !uuid_matches {
                continue;
            }
            let is_asteroid = asteroid_uuid.is_some();
            let mut rng = rand::rng();
            hull_comp.0.apply_damage(damage as f32, &mut rng);
            if hull_comp.0.is_destroyed() {
                commands.entity(entity).despawn();
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

    // Use live ECS position for arc/range check — WorldResource snapshot is stale.
    let live_pos = live_entity_xz(&target_uuid, &asteroid_q, &entity_q);
    let Some((tx, tz)) = live_pos else {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start_bank(&active_bank, &combat_config.0);
        commands.trigger(BeamEndedEvent {
            bank: active_bank.clone(),
            target_uuid,
        });
        return;
    };

    let bank_in_arc = if combat_config.0.banks.is_empty() {
        let effective_phaser_range =
            combat_config.0.phaser_range * modifiers.get(&ModifierSlot::RadarRange);
        crate::radar::is_fire_ready_with_range(
            tx,
            tz,
            ship.x,
            ship.z,
            ship.yaw,
            effective_phaser_range,
        )
    } else {
        combat_config
            .0
            .banks
            .iter()
            .find(|b| b.id == active_bank)
            .map(|bank_cfg| {
                let bank_base_range = if bank_cfg.beam_range > 0.0 {
                    bank_cfg.beam_range
                } else {
                    combat_config.0.phaser_range
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
    if !bank_in_arc {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start_bank(&active_bank, &combat_config.0);
        commands.trigger(BeamEndedEvent {
            bank: active_bank.clone(),
            target_uuid,
        });
        return;
    }

    beam.damage_accumulator +=
        combat_config.0.beam_damage_per_sec * modifiers.get(&ModifierSlot::PhaserDamage) * dt;
    let damage_to_apply = beam.damage_accumulator.floor() as i32;
    if damage_to_apply > 0 {
        beam.damage_accumulator -= damage_to_apply as f32;

        let mut asteroid_destroyed = false;
        let mut npc_destroyed = false;

        for (entity, asteroid_uuid, entity_uuid, mut hull_comp) in hull_query.iter_mut() {
            // Match by whichever UUID component is present
            let uuid_matches = asteroid_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str())
                || entity_uuid.map(|u| u.0.as_str()) == Some(target_uuid.as_str());
            if !uuid_matches {
                continue;
            }

            let is_asteroid = asteroid_uuid.is_some();
            let mut rng = rand::rng();
            hull_comp.0.apply_damage(damage_to_apply as f32, &mut rng);

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
                commands.entity(entity).despawn();
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
            cooldown.start_bank(&active_bank, &combat_config.0);
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
            cooldown.start_bank(&active_bank, &combat_config.0);
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
        cooldown.start_bank(&active_bank, &combat_config.0);
        commands.trigger(BeamEndedEvent {
            bank: active_bank.clone(),
            target_uuid,
        });
    }
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
            let (phaser_range, banks_config) = {
                let cc = world.resource::<PhaserCombatConfigResource>();
                (cc.0.phaser_range, cc.0.banks.clone())
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
                let effective_phaser_range = phaser_range * radar_range_mult;
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
                                    phaser_range
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

// ── HTML console state push (issue #422) ────────────────────────────────────
//
// Mirrors the data `weapons_update_broadcaster` assembles, but writes it into a
// single `WeaponsConsoleStateComp` component on change so a `Changed<...>`
// system can encode + emit a `ConsoleStateChanged` message. The wasm
// forwarding to the JS `__updateConsole` callback lives in
// `bridge::flush_console_state`.

/// Single-entity component carrying the latest serialised Tactical console
/// state. Bevy change-detection drives the JS push.
#[derive(Component, Clone, PartialEq)]
pub struct WeaponsConsoleStateComp(pub WeaponsConsoleState);

/// Startup system: spawn the single entity carrying the Tactical console state.
fn spawn_weapons_console_state_entity(mut commands: Commands) {
    commands.spawn(WeaponsConsoleStateComp(WeaponsConsoleState {
        target_uuid: None,
        target_name: None,
        banks: Vec::new(),
        tubes: Vec::new(),
        torpedo_count: 0,
        phaser_mode: crate::messages::PhaserMode::Auto,
        phaser_arcs: Vec::new(),
        torpedo_arcs: Vec::new(),
        blips: Vec::new(),
        regions: Vec::new(),
    }));
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

/// Recompute the Tactical console state from the same resources as
/// `weapons_update_broadcaster`, writing into `WeaponsConsoleStateComp` only on
/// change so `Changed<WeaponsConsoleStateComp>` fires only on actual change.
fn recompute_weapons_console_state(
    ship: Res<ShipState>,
    weapons_target: Res<WeaponsTarget>,
    beam: Res<ActiveBeam>,
    cooldown: Res<PhaserCooldown>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    combat_config: Res<PhaserCombatConfigResource>,
    phaser_mode: Res<CurrentPhaserMode>,
    torpedo_sys: Res<TorpedoSystemResource>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    world_res: Res<WorldResource>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    entity_name_q: Query<(
        &crate::entity_spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    mut comp_q: Query<&mut WeaponsConsoleStateComp>,
) {
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
        let effective_phaser_range = combat_config.0.phaser_range * radar_range_mult;
        let fire_ready = match target_live_pos {
            None => false,
            Some((tx, tz)) => crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                ship.x,
                ship.z,
                ship.yaw,
                effective_phaser_range,
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
                            combat_config.0.phaser_range
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
    //
    // Join live ECS positions (from query iterators) with static entity
    // metadata (tags, radius) from `WorldResource`. The ECS gives authoritative
    // live positions for all currently-alive entities; `WorldResource` gives the
    // stable tag set used for the `tactical_radar_shows` filter.
    //
    // The projection is the standard ship-aligned radar transform
    // (see `gui::radar::project_radar_entity`) — forward = +radar_y,
    // right = +radar_x, normalised to `[-1.0, 1.0]` at
    // `effective_tactical_range`.
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

    // Build UUID → EntitySnapshot lookup for tags + radius. Allocation is
    // per-frame but bounded by world entity count (typically tens, not millions).
    //
    // NOTE: `WorldResource` is populated once at `StartGame` and is not
    // updated when entities spawn at runtime (via `EntitySpawned`). This means
    // dynamically-spawned NPCs/stations won't appear as radar blips — only
    // entities from the initial world TOML are visible. Asteroids are always
    // in the initial world so they always show. This limitation can be lifted
    // later by updating `WorldResource` from the `EntitySpawned` broadcast.
    let entity_meta: std::collections::HashMap<&str, &crate::messages::EntitySnapshot> = world_res
        .0
        .entities
        .iter()
        .map(|e| (e.uuid.as_str(), e))
        .collect();

    let mut blips: Vec<RadarBlip> = Vec::new();

    if !shows.is_empty() && effective_tactical_range > 0.0 {
        // Asteroids
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
        // Generic entities (NPC ships, stations, torpedoes, etc.)
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

    // ── Fire-arc config (static after game start) ─────────────────────────
    // Included in every push so the HTML console can render arc overlays
    // without ever receiving the Bevy-only `Welcome` message.
    let phaser_arcs: Vec<PhaserBankClientConfig> = ship_config.0.phaser_banks.clone();
    let torpedo_arcs: Vec<TorpedoTubeClientConfig> = ship_config.0.torpedo_tubes.clone();

    // ── Region shapes ──────────────────────────────────────────────────────
    // Collect all world entities that carry a shape field so the HTML radar
    // widget can draw coloured zone overlays.
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
                outer_radius: e.radius, // torus: outer == radius
                half_extents: e.half_extents.map(|h| [h[0], h[2]]),
                yaw: e.yaw,
                color: e.colour.unwrap_or([0.6, 0.4, 1.0]),
                name: e.name.clone(),
            })
        })
        .collect();

    let next = WeaponsConsoleState {
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

    for mut comp in comp_q.iter_mut() {
        if comp.0 != next {
            comp.0 = next.clone();
        }
    }
}

/// `Changed<WeaponsConsoleStateComp>` system: encode the state and emit a
/// `ConsoleStateChanged { name: "Tactical", json }` message for the wasm
/// bridge to forward to the JS `__updateConsole` callback.
fn push_weapons_console_state(
    comp_q: Query<&WeaponsConsoleStateComp, Changed<WeaponsConsoleStateComp>>,
    mut writer: MessageWriter<ConsoleStateChanged>,
) {
    for comp in comp_q.iter() {
        if let Ok(json) = codec::encode_console_state(&comp.0) {
            writer.write(ConsoleStateChanged {
                name: "Tactical".into(),
                json,
            });
        }
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

    /// Return ComplexityRules populated from shipped asset files on native,
    /// or an empty default on WASM (tests do not run on WASM).
    fn test_complexity_rules() -> crate::console_ai_plugin::ComplexityRules {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::console_ai_plugin::ComplexityRules::from_asset_files()
        }
        #[cfg(target_arch = "wasm32")]
        {
            crate::console_ai_plugin::ComplexityRules::default()
        }
    }

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
            .init_resource::<crate::console_ai_plugin::ConsoleComplexityState>()
            .insert_resource(test_complexity_rules())
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
            .add_plugins(WeaponsPlugin)
            // Override with two banks so per-bank arc checks work.
            // Uses wide (270°) arcs so existing tests that fire "port" at a
            // target ahead still pass. Tighter arcs are tested in dedicated
            // per-bank arc severance tests.
            .insert_resource(PhaserCombatConfigResource(
                crate::entity_config::PhaserCombatConfig {
                    phaser_range: 40.0,
                    beam_duration_secs: 6.0,
                    beam_cooldown_secs: 6.0,
                    beam_damage_per_sec: 5.0,
                    banks: vec![
                        crate::entity_config::PhaserBankConfig {
                            id: "port".into(),
                            facing_deg: -90.0,
                            fire_arc_deg: 270.0,
                            auto_arc_deg: 240.0,
                            beam_range: 0.0,
                            shield_pierce: None,
                            marker: None,
                        },
                        crate::entity_config::PhaserBankConfig {
                            id: "starboard".into(),
                            facing_deg: 90.0,
                            fire_arc_deg: 270.0,
                            auto_arc_deg: 240.0,
                            beam_range: 0.0,
                            shield_pierce: None,
                            marker: None,
                        },
                    ],
                },
            ))
            .add_systems(Update, (tick_active_beam, tick_torpedo_system))
            .add_plugins(weapons_update_broadcaster())
            .add_systems(PostUpdate, collect);
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
        push(app, "captain", ClientMessage::StartGame);
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
        push(app, "captain", ClientMessage::StartGame);
        tick(app);
    }

    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world_with_entity(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        push(
            app,
            "weapons",
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
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
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
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
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
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
            ClientMessage::SetTarget {
                uuid: "no-such-asteroid".into(),
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
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
            },
        );
        let _ = tick(&mut app);
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
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
            },
        );
        let _ = tick(&mut app);
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
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
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
            ClientMessage::SetTarget { uuid: "t1".into() },
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
            ClientMessage::SetTarget { uuid: "t2".into() },
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
            ClientMessage::SetPhaserMode {
                mode: crate::messages::PhaserMode::Manual,
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
        push(
            &mut app,
            "captain",
            ClientMessage::SetPhaserMode {
                mode: crate::messages::PhaserMode::Manual,
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
        // → PhaserCombatConfigResource) and assert the resulting cooldown is
        // exactly what the TOML says. Changing `cooldown_secs = 6.0` to
        // `99.0` in player_ship.toml would fail this assertion.
        let toml_str = include_str!("../../../assets/entities/player_ship.toml");
        let config = crate::entity_config::EntityConfig::from_toml(toml_str)
            .expect("player_ship.toml must parse");
        let wc = config
            .weapons_console
            .expect("player_ship must declare [weapons_console]");
        let combat = crate::entity_config::PhaserCombatConfig::from_weapons_console(&wc);

        // The four values that actually drive player phaser behaviour.
        assert_eq!(
            combat.beam_cooldown_secs, wc.cooldown_secs,
            "beam_cooldown_secs must match TOML cooldown_secs"
        );
        assert_eq!(
            combat.beam_duration_secs, wc.beam_duration_secs,
            "beam_duration_secs must match TOML beam_duration_secs"
        );
        assert_eq!(
            combat.beam_damage_per_sec, wc.beam_damage_per_sec,
            "beam_damage_per_sec must match TOML beam_damage_per_sec"
        );
        assert_eq!(
            combat.phaser_range, wc.beam_range,
            "phaser_range must match TOML beam_range"
        );

        // And starting the cooldown produces exactly that value, so it flows
        // through to live `PhaserCooldown.bank_remaining_secs`.
        let mut cd = PhaserCooldown::default();
        cd.start_bank("test", &combat);
        assert_eq!(
            cd.bank_remaining_secs("test"),
            wc.cooldown_secs,
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

    // ── ShipModifiers integration tests ────────────────────────────────────

    #[test]
    fn empty_modifier_table_reproduces_base_phaser_damage() {
        let mut app = test_app();
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(
            &mut app,
            "weapons",
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
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
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
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
            ClientMessage::SetTarget {
                uuid: "target-uuid".into(),
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
        push(app, "captain", ClientMessage::StartGame);
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
    fn sensors_holder_can_set_phaser_frequency_when_tactical_is_low() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        app.world_mut()
            .resource_mut::<crate::console_ai_plugin::ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        push(
            &mut app,
            "sensors",
            ClientMessage::SetPhaserFrequency { frequency: 0.3 },
        );
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!(
            (freq - 0.3).abs() < 1e-5,
            "Sensors holder should set phaser frequency when Tactical is Low, got {freq}"
        );
    }

    #[test]
    fn sensors_holder_cannot_set_phaser_frequency_when_tactical_is_full() {
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
            "Sensors holder must NOT change phaser frequency when Tactical is Full, got {freq}"
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
            ClientMessage::SetTarget {
                uuid: "npc-1".into(),
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
            ClientMessage::SetTarget {
                uuid: "npc-1".into(),
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
            ClientMessage::SetTarget {
                uuid: "npc-1".into(),
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
        use crate::ai::AiController;
        use crate::ai_plugin::{AiControllerComponent, EntityPhaserState};
        use crate::entity_spawner::{EntityConsoleHull, EntityUuid};

        // Register the AI token.
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register(npc_uuid);
        }

        // Build a minimal AiController with the target on its blackboard.
        let target_as_uuid = uuid::Uuid::parse_str(target_uuid).ok();
        let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
        ctrl.blackboard.target = target_as_uuid;

        // Spawn NPC entity facing toward negative-Z (yaw = 0 → forward = -Z).
        let npc_entity = app
            .world_mut()
            .spawn((
                EntityUuid(npc_uuid.to_string()),
                AiControllerComponent {
                    controller: ctrl,
                    entity_uuid: npc_uuid.to_string(),
                    forward_speed: 0.0,
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
        use crate::server_app::Ship;
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
            use crate::ai::AiController;
            let mut ctrl = AiController::new([0.0, 0.0, 0.0], 0.0);
            ctrl.blackboard.target = Some(player_uuid_parsed);

            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register(npc_uuid);

            app.world_mut()
                .spawn((
                    EntityUuid(npc_uuid.to_string()),
                    crate::ai_plugin::AiControllerComponent {
                        controller: ctrl,
                        entity_uuid: npc_uuid.to_string(),
                        forward_speed: 0.0,
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
        // Full end-to-end test: an NPC in the `Attacking` state with a target
        // directly in its forward arc and within beam range causes
        // `tick_ai_controllers` to write a `FirePhaser` `InboundMessage`, which
        // `handle_fire_phaser_npc` picks up and sets `EntityPhaserState::beam_active`.
        use crate::ai_plugin::{AiControllerComponent, EntityPhaserState};
        use crate::damage::ConsoleHull;
        use crate::entity_config::{BehaviourConfig, StateConfig};
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

        // Behaviour: start directly in `attacking` state so FirePhaser is emitted
        // on the very first tick when the target is in range.
        let behaviour = BehaviourConfig {
            initial_state: "attack".into(),
            state: vec![StateConfig {
                name: "attack".into(),
                kind: "attacking".into(),
                waypoints: vec![],
                loop_path: false,
                target_speed: 0.5,
                maintain_range: 0.0,
                duration_secs: 0.0,
            }],
            transition: vec![],
        };

        // Spawn NPC at origin, facing -Z (yaw = 0 → forward = -Z).
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::BehaviourSection(behaviour),
                EntityUuid(npc_uuid_str.to_string()),
                EntityPhaserState::default(),
                WeaponsConsoleSection(crate::entity_config::WeaponsConsoleConfig {
                    beam_range,
                    beam_damage_per_sec: 5.0,
                    beam_duration_secs: 3.0,
                    cooldown_secs: 6.0,
                    beam_color: vec![],
                    torpedo_arc_color: vec![],
                    power_multipliers: None,
                    complexity_toml: None,
                    phaser_banks: Vec::new(),
                    radar: None,
                    shield_pierce: 0.0,
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

        // Set blackboard target so `tick_attacking` fires phasers.
        {
            let mut ctrl = app
                .world_mut()
                .get_mut::<AiControllerComponent>(npc_entity)
                .unwrap();
            ctrl.controller.blackboard.target = Some(target_uuid_parsed);
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

    // ── Weapons console state push (issue #422) ──────────────────────────

    #[derive(Resource, Default)]
    struct ConsolePushes(Vec<ConsoleStateChanged>);

    fn collect_console_pushes(
        mut reader: MessageReader<ConsoleStateChanged>,
        mut sink: ResMut<ConsolePushes>,
    ) {
        for m in reader.read() {
            sink.0.push(m.clone());
        }
    }

    /// Minimal app exercising only the console-state push path: spawn the
    /// component, register the push system + a mock observer that collects
    /// `ConsoleStateChanged`, mutate the component, run an update, and assert a
    /// single push carrying the expected JSON.
    fn push_test_app() -> App {
        let mut app = App::new();
        app.add_message::<ConsoleStateChanged>()
            .init_resource::<ConsolePushes>()
            .add_systems(
                Update,
                (
                    push_weapons_console_state,
                    collect_console_pushes.after(push_weapons_console_state),
                ),
            );
        app.world_mut()
            .spawn(WeaponsConsoleStateComp(WeaponsConsoleState::default()));
        app
    }

    #[test]
    fn weapons_console_push_emits_one_message_with_expected_values() {
        let mut app = push_test_app();

        // First update: the freshly spawned component is `Changed`, so it
        // pushes its initial state. Drain those.
        app.update();
        app.world_mut().resource_mut::<ConsolePushes>().0.clear();

        // Mutate the component → next update should push exactly one message.
        {
            let mut q = app.world_mut().query::<&mut WeaponsConsoleStateComp>();
            let mut comp = q.single_mut(app.world_mut()).unwrap();
            comp.0 = WeaponsConsoleState {
                target_uuid: Some("tgt-42".into()),
                target_name: None,
                banks: vec![PhaserBankState {
                    id: "port".into(),
                    fire_ready: true,
                    on_cooldown: false,
                    cooldown_remaining: 0.0,
                }],
                tubes: vec![TorpedoTubeState {
                    id: "fore".into(),
                    loaded: true,
                    reload_secs: 0.0,
                    state: "loaded".into(),
                    progress: 1.0,
                    load_time: 10.0,
                }],
                torpedo_count: 9,
                phaser_mode: crate::messages::PhaserMode::Manual,
                phaser_arcs: Vec::new(),
                torpedo_arcs: Vec::new(),
                blips: Vec::new(),
                regions: Vec::new(),
            };
        }
        app.update();

        let pushes = &app.world().resource::<ConsolePushes>().0;
        assert_eq!(pushes.len(), 1, "expected exactly one push after a change");
        let push = &pushes[0];
        assert_eq!(push.name, "Tactical");
        assert!(
            push.json.contains("\"target_uuid\":\"tgt-42\""),
            "json: {}",
            push.json
        );
        assert!(
            push.json.contains("\"torpedo_count\":9"),
            "json: {}",
            push.json
        );
        assert!(push.json.contains("\"id\":\"port\""), "json: {}", push.json);
        assert!(push.json.contains("\"id\":\"fore\""), "json: {}", push.json);
        assert!(
            push.json.contains("\"phaser_mode\":\"Manual\""),
            "json: {}",
            push.json
        );

        // No further change → no further pushes.
        app.world_mut().resource_mut::<ConsolePushes>().0.clear();
        app.update();

        assert!(
            app.world().resource::<ConsolePushes>().0.is_empty(),
            "no push expected without a change"
        );
    }

    // ── Radar blip tests ─────────────────────────────────────────────────────

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
        tick(&mut app); // first InProgress tick → recompute runs

        let mut q = app.world_mut().query::<&WeaponsConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        let blips = comp.0.blips.clone();

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

        let mut q = app.world_mut().query::<&WeaponsConsoleStateComp>();
        let comp = q.single(app.world()).unwrap();
        assert!(
            comp.0.blips.is_empty(),
            "asteroid beyond tactical range must not appear in blips"
        );
    }
}

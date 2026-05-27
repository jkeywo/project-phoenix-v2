use bevy::prelude::*;

use crate::lobby::{InboundMessage, Target, Sessions, WorldResource};
use crate::messages::{
    ClientMessage, Console, ModifierSlot, PhaserBank, PhaserBankState, ServerMessage,
    TorpedoTubeState,
};
use crate::simulation::{AsteroidUuid, SimOutbox};
use crate::entity_spawner::EntityConsoleHull;
use crate::torpedo::{TorpedoSystem, TorpedoConfig};
use crate::ai_plugin::{AiTokenRegistry, AiControllerComponent, EntityPhaserState};
use crate::ship_state::ShipState;

// ── Beam constants ───────────────────────────────────────────────────────
//
// The legacy hardcoded values that used to drive the player phaser beam.
// As of slice 3 of the data-driven refactor these are sourced from the
// `PhaserCombatConfigResource` (Bevy resource), which is seeded from the
// `[weapons_console]` block in the ship TOML. `BEAM_DAMAGE_PER_SEC`
// remains `pub` because test scaffolding in `server_app.rs` references it
// as a documented baseline; gameplay systems must read the resource.
const _LEGACY_BEAM_DURATION_SECS: f32 = 6.0;
pub const BEAM_DAMAGE_PER_SEC: f32 = 5.0;
const _LEGACY_BEAM_COOLDOWN_SECS: f32 = 6.0;

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

/// Post-beam cooldown. The weapons console is locked out for
/// `PhaserCombatConfigResource.beam_cooldown_secs` after every beam end
/// (natural, sever, or cancel).
#[derive(Resource, Default)]
pub struct PhaserCooldown {
    pub remaining_secs: f32,
}

impl PhaserCooldown {
    pub fn is_active(&self) -> bool {
        self.remaining_secs > 0.0
    }

    /// Start the cooldown. Reads the per-ship cooldown from
    /// `PhaserCombatConfig`; callers without access to the resource
    /// (legacy tests) can use [`Self::start_with_cooldown`].
    pub fn start(&mut self, config: &crate::entity_config::PhaserCombatConfig) {
        self.remaining_secs = config.beam_cooldown_secs;
    }

    /// Start the cooldown with an explicit value. Convenience for unit
    /// tests that don't construct a `PhaserCombatConfig`.
    pub fn start_with_cooldown(&mut self, secs: f32) {
        self.remaining_secs = secs;
    }
}

impl PhaserCooldown {
    pub fn tick(&mut self, dt: f32) {
        self.remaining_secs = (self.remaining_secs - dt).max(0.0);
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
) {
    let ev = trigger.event();
    outbox.0.push((Target::All, ServerMessage::BeamStarted {
        bank: ev.bank.clone(),
        target_uuid: ev.target_uuid.clone(),
    }));
}

fn on_beam_ended(
    trigger: On<BeamEndedEvent>,
    mut outbox: ResMut<SimOutbox>,
) {
    let ev = trigger.event();
    outbox.0.push((Target::All, ServerMessage::BeamEnded {
        bank: ev.bank.clone(),
        target_uuid: ev.target_uuid.clone(),
    }));
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
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())))
            .add_message::<AsteroidDestroyedVfx>()
            .add_observer(on_beam_started)
            .add_observer(on_beam_ended)
            .add_systems(Update, (
                handle_set_target.in_set(crate::sim_sets::SimSet::Input),
                handle_fire_phaser.in_set(crate::sim_sets::SimSet::Input),
                handle_fire_phaser_npc.in_set(crate::sim_sets::SimSet::Damage),
                handle_set_phaser_mode.in_set(crate::sim_sets::SimSet::Input),
                handle_set_phaser_frequency.in_set(crate::sim_sets::SimSet::Input),
                handle_fire_torpedo.in_set(crate::sim_sets::SimSet::Input),
            ))
            .add_systems(Update, (
                tick_active_beam.in_set(crate::sim_sets::SimSet::Physics),
                tick_torpedo_system.in_set(crate::sim_sets::SimSet::Physics),
            ));
    }
}

// ── Systems ─────────────────────────────────────────────────────────────────

/// Look up the live (x, z) world position of an entity by its string UUID.
///
/// `WorldResource.0.entities` is a snapshot populated at spawn time and never
/// updated, so it cannot be used for gameplay decisions involving moving
/// entities (NPC ships, torpedoes, etc.). Always query the live ECS
/// `Transform` instead. Asteroids carry [`AsteroidUuid`]; NPCs and stations
/// carry [`crate::entity_spawner::EntityUuid`]. This helper checks both.
pub(crate) fn live_entity_xz(
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
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut weapons_target: ResMut<WeaponsTarget>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    mut outbox: ResMut<SimOutbox>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
) {
    for ev in reader.read() {
        let ClientMessage::SetTarget { uuid } = &ev.msg else { continue };

        let holder = sessions.0.console_holder(Console::Tactical);
        let holder_match = holder == Some(ev.token.as_str());
        crate::wasm_log!(
            "[radar-instr 7] handle_set_target: token={} uuid={} tactical_holder={:?} holder_match={}",
            ev.token, uuid, holder, holder_match
        );
        if !holder_match {
            continue;
        }

        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);
        let base_range = ship_config.0.tactical_radar_range;
        let effective_weapons_range = base_range * radar_range_mult;
        let pos = live_entity_xz(uuid, &asteroid_q, &entity_q);
        let locked = match pos {
            None => false,
            Some((x, z)) => {
                let dx = x - ship.x;
                let dz = z - ship.z;
                dx * dx + dz * dz <= effective_weapons_range * effective_weapons_range
            }
        };
        crate::wasm_log!(
            "[radar-instr 7] handle_set_target result: uuid={} entity_found={} base_range={} mult={} effective={} locked={}",
            uuid, pos.is_some(), base_range, radar_range_mult, effective_weapons_range, locked
        );

        if locked {
            weapons_target.0 = Some(uuid.clone());
        } else {
            weapons_target.0 = None;
        }

        outbox.0.push((Target::Token(ev.token.clone()), ServerMessage::TargetLock { uuid: uuid.clone(), locked }));
    }
}

fn handle_fire_phaser(
    mut commands: Commands,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    weapons_target: Res<WeaponsTarget>,
    mut beam: ResMut<ActiveBeam>,
    cooldown: Res<PhaserCooldown>,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    combat_config: Res<PhaserCombatConfigResource>,
    _outbox: ResMut<SimOutbox>,
) {
    for ev in reader.read() {
        let ClientMessage::FirePhaser { bank } = &ev.msg else {
            continue;
        };
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        if cooldown.is_active() || beam.target_uuid.is_some() {
            continue;
        }
        let Some(target_uuid) = &weapons_target.0 else { continue };
        let Some((tx, tz)) = live_entity_xz(target_uuid, &asteroid_q, &entity_q) else {
            continue;
        };
        let effective_phaser_range = combat_config.0.phaser_range * modifiers.get(&ModifierSlot::RadarRange);
        if !crate::radar::is_fire_ready_with_range(tx, tz, ship.x, ship.z, ship.yaw, effective_phaser_range) {
            continue;
        }

        if let Some(old_uuid) = beam.target_uuid.take() {
            let old_bank = beam.bank.clone().unwrap_or_default();
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            commands.trigger(BeamEndedEvent { bank: old_bank, target_uuid: old_uuid });
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
    mut npc_query: Query<(
        Entity,
        &crate::entity_spawner::EntityUuid,
        &Transform,
        Option<&mut EntityPhaserState>,
        Option<&crate::entity_spawner::WeaponsConsoleSection>,
        Option<&AiControllerComponent>,
    ), With<AiControllerComponent>>,
    mut hull_query: Query<
        (Entity, &crate::entity_spawner::EntityUuid, &Transform, &mut EntityConsoleHull),
        Without<AiControllerComponent>,
    >,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
) {
    let dt = time.delta_secs();

    // If no AiTokenRegistry resource is present (e.g. in tests without AiPlugin), skip entirely.
    let Some(registry) = registry else { return; };

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
    let target_positions: Vec<(uuid::Uuid, f32, f32)> = hull_query
        .iter()
        .filter_map(|(_, uid, t, _)| {
            uuid::Uuid::parse_str(&uid.0).ok()
                .map(|u| (u, t.translation.x, t.translation.z))
        })
        .collect();

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
                commands.entity(npc_entity).insert(EntityPhaserState::default());
                continue;
            }
        };

        // Tick cooldown.
        phaser_state.cooldown_remaining = (phaser_state.cooldown_remaining - dt).max(0.0);

        let target_uuid: Option<uuid::Uuid> = ctrl_opt.and_then(|c| c.controller.blackboard.target);

        let beam_range = weapons_section
            .map(|wc| if wc.0.beam_range > 0.0 { wc.0.beam_range } else { 40.0 })
            .unwrap_or(40.0);
        let damage_per_sec = weapons_section
            .map(|wc| if wc.0.beam_damage_per_sec > 0.0 { wc.0.beam_damage_per_sec } else { NPC_BEAM_DAMAGE_PER_SEC })
            .unwrap_or(NPC_BEAM_DAMAGE_PER_SEC);
        let beam_duration = weapons_section
            .map(|wc| if wc.0.beam_duration_secs > 0.0 { wc.0.beam_duration_secs } else { NPC_BEAM_DURATION_SECS })
            .unwrap_or(NPC_BEAM_DURATION_SECS);

        let npc_x = transform.translation.x;
        let npc_z = transform.translation.z;
        let npc_yaw = transform.rotation.to_euler(EulerRot::YXZ).0;

        // Activate beam on FirePhaser order when ready, or auto-fire when the AI
        // is in the Attacking state with a valid target (eliminates the 1-frame
        // event delay between tick_ai_controllers → InboundMessage → here).
        let should_fire = fire_orders.contains(&token)
            || ctrl_opt.as_ref().map_or(false, |c| {
                matches!(c.controller.current_state, crate::ai::AiState::Attacking { .. })
                    && c.controller.blackboard.target.is_some()
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
                let damage = damage_per_sec * dt;
                let mut target_destroyed = false;
                let target_uuid_str = t_uuid.to_string();
                for (tgt_entity, tgt_uid, _tgt_transform, mut tgt_hull) in hull_query.iter_mut() {
                    if tgt_uid.0 != target_uuid_str {
                        continue;
                    }
                    let mut rng = rand::rng();
                    tgt_hull.0.apply_damage(damage, &mut rng);
                    if tgt_hull.0.is_destroyed() {
                        target_destroyed = true;
                        commands.entity(tgt_entity).despawn();
                        destroyed_events
                            .write(crate::ai_plugin::AiEntityDestroyed { entity_uuid: tgt_uid.0.clone() });
                    }
                    break;
                }
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
        let ClientMessage::SetPhaserMode { mode } = &ev.msg else { continue };
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
    mut ship: ResMut<ShipState>,
) {
    use crate::delegation::{is_sender_authorized, ComplexityContext, DelegatedControl};
    let ctx = ComplexityContext {
        tactical_is_low: complexity.is_low(&Console::Tactical),
    };
    for ev in reader.read() {
        let ClientMessage::SetPhaserFrequency { frequency } = &ev.msg else { continue };

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

fn handle_fire_torpedo(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ship: Res<ShipState>,
    mut torpedo_sys: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
) {
    for ev in reader.read() {
        let ClientMessage::FireTorpedo { tube, target_uuid } = &ev.msg else { continue };
        if sessions.0.console_holder(Console::Tactical) != Some(ev.token.as_str()) {
            continue;
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let launch_heading = ship.yaw;
        use crate::torpedo::LaunchResult;
        match torpedo_sys.0.launch(tube.as_str(), uuid, ship.x, ship.z, launch_heading, target_uuid.clone()) {
            LaunchResult::Launched { uuid: launched_uuid } => {
                outbox.0.push((Target::All, ServerMessage::TorpedoLaunched {
                    uuid: launched_uuid,
                    tube: tube.clone(),
                    x: ship.x,
                    z: ship.z,
                    heading: launch_heading,
                }));
            }
            LaunchResult::TubeNotLoaded
            | LaunchResult::NoTorpedoes
            | LaunchResult::UnknownTube => {}
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
        &Transform,
        &mut EntityConsoleHull,
    )>,
    mut commands: Commands,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
) {
    let dt = time.delta_secs();

    // Build live (uuid → (x,z)) positions from the ECS so torpedoes guide
    // toward where the target actually is, not its spawn-time snapshot.
    let live_positions: std::collections::HashMap<String, (f32, f32)> = hull_query
        .iter()
        .filter_map(|(_, au, eu, t, _)| {
            au.map(|u| u.0.clone())
                .or_else(|| eu.map(|u| u.0.clone()))
                .map(|uuid| (uuid, (t.translation.x, t.translation.z)))
        })
        .collect();
    let result = torpedo_sys.0.tick(dt, &live_positions);
    for expired_uuid in result.expired {
        outbox.0.push((Target::All, ServerMessage::TorpedoDestroyed { uuid: expired_uuid }));
    }

    // Proximity detonation. Use live positions for hit-testing but pull
    // collision radius from the (immutable) world snapshot — radius is
    // spawn metadata, position is gameplay state. NPC shields are not yet
    // modelled server-side; when they are, the torpedo's `damage_shields`
    // should be applied here first (mirroring apply_damage_with_shields on
    // the ship).
    let radius_by_uuid: std::collections::HashMap<&str, f32> = world.0.entities
        .iter()
        .map(|e| (e.uuid.as_str(), e.radius_or_zero()))
        .collect();
    let targets: Vec<(String, f32, f32, f32)> = live_positions
        .iter()
        .map(|(uuid, (x, z))| {
            let r = radius_by_uuid.get(uuid.as_str()).copied().unwrap_or(0.0);
            (uuid.clone(), *x, *z, r)
        })
        .collect();
    let hits = torpedo_sys.0.find_detonation_hits(&targets);
    for (torpedo_uuid, target_uuid) in hits {
        let Some(damage) = torpedo_sys.0.handle_collision(&torpedo_uuid) else { continue };
        outbox.0.push((Target::All, ServerMessage::TorpedoDestroyed { uuid: torpedo_uuid }));

        let mut asteroid_destroyed = false;
        let mut npc_destroyed = false;
        let mut hit_x = 0.0_f32;
        let mut hit_z = 0.0_f32;

        for (entity, asteroid_uuid, entity_uuid, transform, mut hull_comp) in hull_query.iter_mut() {
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
                hit_x = transform.translation.x;
                hit_z = transform.translation.z;
            }
        }

        if asteroid_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            vfx_events.write(AsteroidDestroyedVfx { x: hit_x, z: hit_z });
            outbox.0.push((Target::All, ServerMessage::AsteroidDestroyed { uuid: target_uuid }));
        } else if npc_destroyed {
            world.0.entities.retain(|a| a.uuid != target_uuid);
            destroyed_events.write(crate::ai_plugin::AiEntityDestroyed { entity_uuid: target_uuid.clone() });
            outbox.0.push((Target::All, ServerMessage::EntityDespawned { uuid: target_uuid }));
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
    mut hull_query: Query<(Entity, Option<&AsteroidUuid>, Option<&crate::entity_spawner::EntityUuid>, &Transform, &mut EntityConsoleHull)>,
    mut commands: Commands,
    modifiers: Res<crate::modifiers::ShipModifiers>,
    combat_config: Res<PhaserCombatConfigResource>,
    mut outbox: ResMut<SimOutbox>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    // Player-ship UUID, used to tag NPCs we hit so their AI's `on_attacked`
    // transition fires. Empty in tests that don't spawn a player-ship entity;
    // the insertion is skipped in that case.
    player_ship_q: Query<&crate::entity_spawner::EntityUuid, With<crate::server_app::Ship>>,
) {

    let dt = time.delta_secs();
    cooldown.tick(dt);

    let Some(target_uuid) = beam.target_uuid.clone() else {
        return;
    };
    let active_bank = beam.bank.clone().unwrap_or_default();

    // Live position lookup from the ECS — the hull_query already has Transforms
    // for the entity, so use that. WorldResource.0.entities is a stale snapshot.
    let live_target_pos: Option<(f32, f32)> = hull_query.iter().find_map(|(_, au, eu, t, _)| {
        let matches = au.map(|u| u.0.as_str()) == Some(target_uuid.as_str())
            || eu.map(|u| u.0.as_str()) == Some(target_uuid.as_str());
        if matches { Some((t.translation.x, t.translation.z)) } else { None }
    });
    let Some((target_x, target_z)) = live_target_pos else {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start(&combat_config.0);
        commands.trigger(BeamEndedEvent { bank: active_bank.clone(), target_uuid });
        return;
    };

    let effective_phaser_range = combat_config.0.phaser_range * modifiers.get(&ModifierSlot::RadarRange);
    if !crate::radar::is_fire_ready_with_range(target_x, target_z, ship.x, ship.z, ship.yaw, effective_phaser_range) {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start(&combat_config.0);
        commands.trigger(BeamEndedEvent { bank: active_bank.clone(), target_uuid });
        return;
    }

    beam.damage_accumulator += combat_config.0.beam_damage_per_sec * modifiers.get(&ModifierSlot::PhaserDamage) * dt;
    let damage_to_apply = beam.damage_accumulator.floor() as i32;
    if damage_to_apply > 0 {
        beam.damage_accumulator -= damage_to_apply as f32;

        let mut asteroid_destroyed = false;
        let mut npc_destroyed = false;

        for (entity, asteroid_uuid, entity_uuid, _transform, mut hull_comp) in hull_query.iter_mut() {
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
            vfx_events.write(AsteroidDestroyedVfx { x: target_x, z: target_z });

            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start(&combat_config.0);

            outbox.0.push((Target::All, ServerMessage::AsteroidDestroyed { uuid: target_uuid.clone() }));
            commands.trigger(BeamEndedEvent { bank: active_bank.clone(), target_uuid });
            return;
        }

        if npc_destroyed {
            // Non-asteroid entity destroyed
            world.0.entities.retain(|a| a.uuid != target_uuid);
            destroyed_events.write(crate::ai_plugin::AiEntityDestroyed { entity_uuid: target_uuid.clone() });
            outbox.0.push((Target::All, ServerMessage::EntityDespawned { uuid: target_uuid.clone() }));

            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start(&combat_config.0);

            commands.trigger(BeamEndedEvent { bank: active_bank.clone(), target_uuid });
            return;
        }
    }

    beam.remaining_secs -= dt;
    if beam.remaining_secs <= 0.0 {
        beam.target_uuid = None;
        beam.remaining_secs = 0.0;
        beam.damage_accumulator = 0.0;
        cooldown.start(&combat_config.0);
        commands.trigger(BeamEndedEvent { bank: active_bank.clone(), target_uuid });
    }
}

// ── Broadcaster ───────────────────────────────────────────────────────────

pub fn weapons_update_broadcaster() -> crate::core::broadcast::SimBroadcaster {
    crate::core::broadcast::SimBroadcaster::new().register(
        crate::core::broadcast::Audience::Holding(Console::Tactical),
        crate::core::broadcast::Cadence::Hz(10.0),
        |world: &mut World| {
            // Look up the live target position from the ECS first (asteroid or
            // NPC/station), so fire_ready uses the same moving-target geometry
            // as handle_fire_phaser/tick_active_beam. WorldResource snapshot
            // would give the stale spawn position for NPCs.
            let target_uuid_opt: Option<String> = world.resource::<WeaponsTarget>().0.clone();
            let live_target_pos: Option<(f32, f32)> = if let Some(uuid) = target_uuid_opt.as_ref() {
                let mut found: Option<(f32, f32)> = None;
                {
                    let mut aq = world.query::<(&AsteroidUuid, &Transform)>();
                    for (u, t) in aq.iter(world) {
                        if &u.0 == uuid { found = Some((t.translation.x, t.translation.z)); break; }
                    }
                }
                if found.is_none() {
                    let mut eq = world.query::<(&crate::entity_spawner::EntityUuid, &Transform)>();
                    for (u, t) in eq.iter(world) {
                        if &u.0 == uuid { found = Some((t.translation.x, t.translation.z)); break; }
                    }
                }
                found
            } else {
                None
            };

            let ship = world.resource::<ShipState>();
            let weapons_target = world.resource::<WeaponsTarget>();
            let cooldown = world.resource::<PhaserCooldown>();
            let beam = world.resource::<ActiveBeam>();
            let torpedo_sys = world.resource::<TorpedoSystemResource>();
            let modifiers = world.resource::<crate::modifiers::ShipModifiers>();
            let combat_config = world.resource::<PhaserCombatConfigResource>();
            let phaser_mode = world.resource::<CurrentPhaserMode>().0;

            let effective_phaser_range = combat_config.0.phaser_range * modifiers.get(&ModifierSlot::RadarRange);
            let fire_ready = match (&weapons_target.0, live_target_pos) {
                (None, _) | (_, None) => false,
                (Some(_), Some((tx, tz))) => {
                    crate::radar::is_fire_ready_with_range(tx, tz, ship.x, ship.z, ship.yaw, effective_phaser_range)
                }
            };

            let ts = &torpedo_sys.0;
            let on_cooldown = cooldown.is_active() || beam.target_uuid.is_some();

            // Per-bank state. If `combat_config.0.banks` is empty (ship has
            // no per-bank TOML), fall back to a single anonymous bank using
            // the active-beam state.
            let banks: Vec<PhaserBankState> = if combat_config.0.banks.is_empty() {
                vec![PhaserBankState {
                    id: String::new(),
                    fire_ready,
                    on_cooldown,
                    cooldown_remaining: cooldown.remaining_secs,
                }]
            } else {
                combat_config.0.banks.iter().map(|b| PhaserBankState {
                    id: b.id.clone(),
                    fire_ready,
                    on_cooldown,
                    cooldown_remaining: cooldown.remaining_secs,
                }).collect()
            };

            let tubes: Vec<TorpedoTubeState> = ts.tubes.iter().map(|t| TorpedoTubeState {
                id: t.id.clone(),
                loaded: t.is_loaded(),
                reload_secs: t.reload_remaining,
            }).collect();

            crate::wasm_log!(
                "[radar-instr 9] weapons_update_broadcaster: target_uuid={:?} fire_ready={}",
                weapons_target.0, fire_ready
            );
            vec![ServerMessage::WeaponsUpdate {
                target_uuid: weapons_target.0.clone(),
                banks,
                tubes,
                torpedo_count: ts.torpedoes_remaining,
                phaser_mode,
            }]
        },
    )
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
            .insert_resource(TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())))
            .init_resource::<crate::console_ai_plugin::ConsoleComplexityState>()
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .insert_resource(crate::lobby::server::ShipClientConfigResource::default())
            .add_plugins(WeaponsPlugin)
            .add_systems(Update, (
                tick_active_beam,
                tick_torpedo_system,
            ))
            .add_plugins(weapons_update_broadcaster())
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

    fn setup_weapons_world(app: &mut App, asteroid_x: f32, asteroid_z: f32) {
        setup_weapons_world_with_entity(app, asteroid_x, asteroid_z);
    }

    fn setup_weapons_world_with_entity(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> bevy::ecs::entity::Entity {
        app.world_mut().insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot::asteroid("target-uuid", asteroid_x, asteroid_z, 2.0)],
            ..Default::default()
        }));
        app.world_mut().spawn((
            crate::simulation::Asteroid,
            crate::simulation::AsteroidUuid("target-uuid".into()),
            EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(crate::messages::Console::CaptainChair, 30.0)])),
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

    fn lock_and_fire(app: &mut App, asteroid_x: f32, asteroid_z: f32) -> Vec<OutboundMessage> {
        setup_weapons_world_with_entity(app, asteroid_x, asteroid_z);
        start_game_with_weapons(app);
        push(app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(app);
        push(app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        tick(app)
    }

    // ── SetTarget / TargetLock tests ───────────────────────────────────────

    #[test]
    fn valid_target_within_range_replies_with_target_lock_confirmed() {
        let mut app = test_app();
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

    // ── WeaponsUpdate / fire_ready tests ───────────────────────────────────

    #[test]
    fn weapons_update_fire_ready_true_when_target_in_range_and_arc() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, banks, .. } =>
                Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(update.1, "expected fire_ready=true for in-range, forward-arc target");
    }

    #[test]
    fn weapons_update_fire_ready_false_when_target_out_of_phaser_range() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -50.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        let out = tick(&mut app);

        let update = out.iter().find_map(|m| match &m.msg {
            ServerMessage::WeaponsUpdate { target_uuid, banks, .. } =>
                Some((target_uuid.clone(), banks.iter().any(|b| b.fire_ready))),
            _ => None,
        }).expect("expected a WeaponsUpdate message");
        assert_eq!(update.0.as_deref(), Some("target-uuid"));
        assert!(!update.1, "expected fire_ready=false for beyond-phaser-range target");
    }

    // ── FirePhaser / beam lifecycle tests ──────────────────────────────────

    #[test]
    fn fire_phaser_on_valid_target_broadcasts_beam_started() {
        let mut app = test_app();
        let out = lock_and_fire(&mut app, 0.0, -20.0);

        let beam_started = out.iter().find(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. }));
        assert!(beam_started.is_some(), "expected BeamStarted after firing at fire-ready target");
        match &beam_started.unwrap().msg {
            ServerMessage::BeamStarted { target_uuid, .. } => assert_eq!(target_uuid, "target-uuid"),
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
        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 3.0;

        push(&mut app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "BeamStarted should not fire during cooldown");
    }

    #[test]
    fn fire_phaser_ignored_from_non_weapons_player() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, -20.0);
        start_game(&mut app);

        push(&mut app, "captain", ClientMessage::FirePhaser { bank: "port".to_string() });
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "captain should not be able to fire phaser");
    }

    #[test]
    fn fire_phaser_rejected_when_target_behind_ship() {
        let mut app = test_app();
        setup_weapons_world(&mut app, 0.0, 20.0);
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        let out = tick(&mut app);

        assert!(!out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "FirePhaser should be rejected when target is in rear arc");
    }

    #[test]
    fn full_beam_duration_kills_asteroid() {
        let mut app = test_app();
        // setup_weapons_world (called by lock_and_fire) now spawns the
        // asteroid ECS entity. Fetch its handle after setup.
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        let asteroid_entity = {
            let mut q = app.world_mut().query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
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

        let destroyed = out.iter().find(|m| matches!(&m.msg, ServerMessage::AsteroidDestroyed { .. }));
        assert!(destroyed.is_some(), "expected AsteroidDestroyed when asteroid HP reaches 0");
        match &destroyed.unwrap().msg {
            ServerMessage::AsteroidDestroyed { uuid } => assert_eq!(uuid, "target-uuid"),
            _ => unreachable!(),
        }

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after asteroid destruction");

        assert!(
            !app.world().resource::<WorldResource>().0.entities.iter().any(|a| a.uuid == "target-uuid"),
            "destroyed asteroid should be removed from WorldData"
        );

        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());

        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after beam end");

        assert!(app.world().get::<EntityConsoleHull>(asteroid_entity).is_none(),
            "asteroid entity should be despawned");
    }

    #[test]
    fn beam_severs_when_target_leaves_forward_arc() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves forward arc");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-arc");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after arc sever");
    }

    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move the live ECS asteroid out of phaser range. The WorldResource
        // snapshot is no longer consulted by tick_active_beam.
        {
            let mut q = app.world_mut().query::<(&crate::simulation::AsteroidUuid, &mut Transform)>();
            for (u, mut t) in q.iter_mut(app.world_mut()) {
                if u.0 == "target-uuid" {
                    t.translation.z = -50.0;
                }
            }
        }

        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves phaser range");
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none(),
            "beam should be cleared after sever-by-range");
        assert!(app.world().resource::<PhaserCooldown>().is_active(),
            "cooldown should start after range sever");
    }

    #[test]
    fn no_damage_refund_on_sever() {
        let mut app = test_app();
        // setup_weapons_world (called by lock_and_fire) now spawns the asteroid
        // ECS entity itself; fetch its handle afterwards.
        let _ = lock_and_fire(&mut app, 0.0, -20.0);
        let asteroid_entity = {
            let mut q = app.world_mut().query::<(bevy::ecs::entity::Entity, &crate::simulation::AsteroidUuid)>();
            q.iter(app.world())
                .find(|(_, u)| u.0 == "target-uuid")
                .map(|(e, _)| e)
                .expect("setup_weapons_world should have spawned the target asteroid")
        };

        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 10.0;
        let _ = tick(&mut app);

        app.world_mut().resource_mut::<ShipState>().yaw = std::f32::consts::PI;
        let _ = tick(&mut app);

        let hp = app.world().get::<EntityConsoleHull>(asteroid_entity)
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
        app.world_mut().insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![
                crate::messages::EntitySnapshot::asteroid("t1", 0.0, -20.0, 2.0),
                crate::messages::EntitySnapshot::asteroid("t2", 0.0, -15.0, 2.0),
            ],
            ..Default::default()
        }));
        // Live ECS entities mirroring the snapshot so the lookups in
        // handle_set_target / tick_active_beam succeed.
        for (uuid, x, z) in &[("t1", 0.0f32, -20.0f32), ("t2", 0.0f32, -15.0f32)] {
            app.world_mut().spawn((
                crate::simulation::Asteroid,
                crate::simulation::AsteroidUuid((*uuid).into()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(crate::messages::Console::CaptainChair, 30.0)])),
                Transform::from_xyz(*x, 0.0, *z),
            ));
        }
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t1".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        let _ = tick(&mut app);
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t1"));

        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 0.0;
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 0.0;
        let _ = tick(&mut app);

        assert!(app.world().resource::<PhaserCooldown>().is_active());

        app.world_mut().resource_mut::<PhaserCooldown>().remaining_secs = 0.0;

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "t2".into() });
        let _ = tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        let out = tick(&mut app);

        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamStarted { .. })),
            "expected BeamStarted for new target after cooldown");
        assert_eq!(app.world().resource::<ActiveBeam>().target_uuid.as_deref(), Some("t2"));
    }

    // ── SetPhaserMode tests ────────────────────────────────────────────────

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

    // ── FireTorpedo tests ──────────────────────────────────────────────────

    #[test]
    fn tactical_player_can_fire_torpedo_broadcasts_torpedo_launched() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")),
            "expected TorpedoLaunched broadcast after Tactical fires torpedo"
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
        let tc = config.torpedoes.expect("player_ship must declare [torpedoes]");
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
        let wc = config.weapons_console
            .expect("player_ship must declare [weapons_console]");
        let combat = crate::entity_config::PhaserCombatConfig::from_weapons_console(&wc);

        // The four values that actually drive player phaser behaviour.
        assert_eq!(combat.beam_cooldown_secs, wc.cooldown_secs,
            "beam_cooldown_secs must match TOML cooldown_secs");
        assert_eq!(combat.beam_duration_secs, wc.beam_duration_secs,
            "beam_duration_secs must match TOML beam_duration_secs");
        assert_eq!(combat.beam_damage_per_sec, wc.beam_damage_per_sec,
            "beam_damage_per_sec must match TOML beam_damage_per_sec");
        assert_eq!(combat.phaser_range, wc.beam_range,
            "phaser_range must match TOML beam_range");

        // And starting the cooldown produces exactly that value, so it flows
        // through to live `PhaserCooldown.remaining_secs`.
        let mut cd = PhaserCooldown::default();
        cd.start(&combat);
        assert_eq!(cd.remaining_secs, wc.cooldown_secs,
            "PhaserCooldown::start must use the TOML-sourced cooldown");
    }

    #[test]
    fn non_tactical_player_cannot_fire_torpedo() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(&mut app, "captain", ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            !out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "captain should not be able to fire torpedo"
        );
    }

    #[test]
    fn fire_torpedo_during_lobby_fires_when_no_simset_gate() {
        // Note: The Lobby gate is now at the SimSet chain level.
        // In test configurations without SimSet, the system processes messages during Lobby.
        let mut app = test_app();
        push(&mut app, "weapons", ClientMessage::Identify { token: "weapons".into(), name: "Bob".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::SelectStation { station: "Tactical".into() });
        tick(&mut app);

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: "aft".to_string(),
            target_uuid: None,
        });
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { .. })),
            "FireTorpedo should fire during Lobby when no SimSet gate is configured"
        );
    }

    #[test]
    fn torpedo_launched_is_broadcast_to_all() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::FireTorpedo {
            tube: "fore_starboard".to_string(),
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

    // ── ShipModifiers integration tests ────────────────────────────────────

    #[test]
    fn empty_modifier_table_reproduces_base_phaser_damage() {
        let mut app = test_app();
        setup_weapons_world_with_entity(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        tick(&mut app);

        let hp_before = {
            let world = app.world().resource::<WorldResource>();
            world.0.entities.iter().find(|a| a.uuid == "target-uuid").map(|_| true)
        };
        assert!(hp_before.is_some(), "asteroid should still exist after <1s");
    }

    #[test]
    fn phaser_damage_modifier_doubles_kill_rate() {
        use crate::modifiers::{Modifier, ShipModifiers};
        use crate::messages::{ModifierSlot, ModifierSource};

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
        push(&mut app_fast, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app_fast);
        push(&mut app_fast, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        tick(&mut app_fast);

        {
            let mut beam = app_fast.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 2.0 * 3.5;
        }
        tick(&mut app_fast);

        let still_exists_fast = app_fast.world().resource::<WorldResource>()
            .0.entities.iter().any(|a| a.uuid == "target-uuid");
        assert!(!still_exists_fast, "with 2× phaser damage modifier, asteroid should be destroyed after 3.5s of beam");

        let mut app_base = test_app();
        setup_weapons_world_with_entity(&mut app_base, 0.0, -20.0);
        start_game_with_weapons(&mut app_base);
        push(&mut app_base, "weapons", ClientMessage::SetTarget { uuid: "target-uuid".into() });
        tick(&mut app_base);
        push(&mut app_base, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        tick(&mut app_base);
        {
            let mut beam = app_base.world_mut().resource_mut::<ActiveBeam>();
            beam.damage_accumulator = BEAM_DAMAGE_PER_SEC * 1.0 * 3.5;
        }
        tick(&mut app_base);

        let still_exists_base = app_base.world().resource::<WorldResource>()
            .0.entities.iter().any(|a| a.uuid == "target-uuid");
        assert!(still_exists_base, "with identity modifier, asteroid should survive 3.5s of beam (only 17.5/30 HP removed)");
    }

    // ── SetPhaserFrequency delegation tests ────────────────────────────────

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

    #[test]
    fn tactical_holder_can_set_phaser_frequency() {
        let mut app = test_app();
        start_game_with_weapons(&mut app);
        push(&mut app, "weapons", ClientMessage::SetPhaserFrequency { frequency: 0.8 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.8).abs() < 1e-5, "Tactical holder should set phaser frequency to 0.8, got {freq}");
    }

    #[test]
    fn sensors_holder_can_set_phaser_frequency_when_tactical_is_low() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        app.world_mut()
            .resource_mut::<crate::console_ai_plugin::ConsoleComplexityState>()
            .set(Console::Tactical, "Low".into());
        push(&mut app, "sensors", ClientMessage::SetPhaserFrequency { frequency: 0.3 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.3).abs() < 1e-5, "Sensors holder should set phaser frequency when Tactical is Low, got {freq}");
    }

    #[test]
    fn sensors_holder_cannot_set_phaser_frequency_when_tactical_is_full() {
        let mut app = test_app();
        start_game_with_sensors_and_weapons(&mut app);
        push(&mut app, "sensors", ClientMessage::SetPhaserFrequency { frequency: 0.9 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.5).abs() < 1e-5, "Sensors holder must NOT change phaser frequency when Tactical is Full, got {freq}");
    }

    #[test]
    fn unrelated_console_cannot_set_phaser_frequency() {
        let mut app = test_app();
        start_game(&mut app);
        push(&mut app, "captain", ClientMessage::SetPhaserFrequency { frequency: 0.9 });
        tick(&mut app);
        let freq = app.world().resource::<ShipState>().phaser_frequency;
        assert!((freq - 0.5).abs() < 1e-5, "Captain must NOT change phaser frequency, got {freq}");
    }

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

    // ── NPC / station phaser damage (issue #311) ──────────────────────────

    fn setup_npc_world(app: &mut App, npc_x: f32, npc_z: f32) {
        app.world_mut().insert_resource(WorldResource(crate::messages::WorldData {
            entities: vec![crate::messages::EntitySnapshot {
                uuid: "npc-1".into(),
                position: Some([npc_x, 0.0, npc_z]),
                tags: vec!["ship".into()],
                ..Default::default()
            }],
            ..Default::default()
        }));
    }

    fn spawn_npc_entity(app: &mut App, npc_x: f32, npc_z: f32, max_hp: f32) -> bevy::ecs::entity::Entity {
        app.world_mut().spawn((
            crate::entity_spawner::EntityUuid("npc-1".into()),
            EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(crate::messages::Console::CaptainChair, max_hp)])),
            Transform::from_xyz(npc_x, 0.0, npc_z),
        )).id()
    }

    // ── Cycle 1: phaser beam reduces NPC hull ─────────────────────────────

    #[test]
    fn phaser_beam_damages_npc_entity_hull() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "npc-1".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        tick(&mut app);

        // Accumulate damage but don't destroy
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 10.0;
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 5.0;
        tick(&mut app);

        let hp = app.world().get::<EntityConsoleHull>(npc_entity)
            .expect("NPC entity should still exist")
            .0.total_current();
        assert!(hp < 30.0, "NPC hull should be reduced after phaser hit, got {hp}");
    }

    // ── Cycle 2: NPC at 0 HP is despawned and EntityDespawned broadcast ──

    #[test]
    fn phaser_beam_destroys_npc_entity_when_hull_reaches_zero() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "npc-1".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        tick(&mut app);

        // Force lethal damage
        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 30.0;
        app.world_mut().resource_mut::<ActiveBeam>().remaining_secs = 5.0;
        let out = tick(&mut app);

        // ECS entity despawned
        assert!(
            app.world().get::<EntityConsoleHull>(npc_entity).is_none(),
            "NPC entity should be despawned after hull reaches 0"
        );

        // EntityDespawned wire message broadcast to all
        let despawned_msg = out.iter().find(|m| matches!(&m.msg, ServerMessage::EntityDespawned { uuid } if uuid == "npc-1"));
        assert!(despawned_msg.is_some(), "expected EntityDespawned {{ uuid: npc-1 }} broadcast");

        // BeamEnded sent
        assert!(out.iter().any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded after NPC destruction");

        // Beam cleared, cooldown started
        assert!(app.world().resource::<ActiveBeam>().target_uuid.is_none());
        assert!(app.world().resource::<PhaserCooldown>().is_active());
    }

    // ── Cycle 3: AiEntityDestroyed message written on NPC destruction ─────

    #[test]
    fn phaser_beam_emits_ai_entity_destroyed_on_npc_kill() {
        #[derive(Resource, Default)]
        struct DestroyedBox(Vec<crate::ai_plugin::AiEntityDestroyed>);

        let mut app = test_app();
        app.init_resource::<DestroyedBox>();
        app.add_systems(bevy::app::Update, |mut r: bevy::ecs::prelude::MessageReader<crate::ai_plugin::AiEntityDestroyed>, mut b: bevy::ecs::prelude::ResMut<DestroyedBox>| {
            for ev in r.read() { b.0.push(ev.clone()); }
        });

        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);
        spawn_npc_entity(&mut app, 0.0, -20.0, 30.0);

        push(&mut app, "weapons", ClientMessage::SetTarget { uuid: "npc-1".into() });
        tick(&mut app);
        push(&mut app, "weapons", ClientMessage::FirePhaser { bank: "port".to_string() });
        tick(&mut app);

        app.world_mut().resource_mut::<ActiveBeam>().damage_accumulator = 30.0;
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
        use crate::ai_plugin::{AiControllerComponent, EntityPhaserState};
        use crate::ai::AiController;
        use crate::entity_spawner::{EntityUuid, EntityConsoleHull};

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
        let npc_entity = app.world_mut().spawn((
            EntityUuid(npc_uuid.to_string()),
            AiControllerComponent {
                controller: ctrl,
                entity_uuid: npc_uuid.to_string(),
                forward_speed: 0.0,
            },
            EntityPhaserState::default(),
            Transform::from_xyz(0.0, 0.0, 0.0),
        )).id();

        // Spawn target entity.
        let target_entity = app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[
                (crate::messages::Console::CaptainChair, 50.0),
            ])),
            Transform::from_xyz(target_x, 0.0, target_z),
        )).id();

        (npc_entity, target_entity)
    }

    #[test]
    fn npc_fire_phaser_activates_entity_phaser_state() {
        // NPC entity at origin, target directly ahead (negative-Z), within beam range.
        // Sending a FirePhaser InboundMessage for the NPC's ai: token should set
        // `EntityPhaserState::beam_active = true` after one update.
        use crate::ai_plugin::{EntityPhaserState, AiTokenRegistry};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000001";
        let target_uuid = "00000000-0000-0000-0000-000000000002";

        let (npc_entity, _target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid, 0.0, -20.0);

        // Send FirePhaser as the NPC's synthetic token.
        let ai_token = format!("ai:{}", npc_uuid);
        push(&mut app, &ai_token, ClientMessage::FirePhaser { bank: "port".to_string() });
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
        use crate::ai_plugin::{EntityPhaserState, AiTokenRegistry};
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
            let mut ps = app.world_mut().get_mut::<EntityPhaserState>(npc_entity).unwrap();
            ps.beam_active = true;
            ps.beam_target = Some(target_uuid_parsed);
            ps.beam_remaining_secs = 10.0;
        }

        let hp_before = app.world().get::<EntityConsoleHull>(target_entity).unwrap().0.total_current();

        // Run several ticks so damage accumulates.
        for _ in 0..10 {
            app.update();
        }

        let hp_after = app.world().get::<EntityConsoleHull>(target_entity).unwrap().0.total_current();
        assert!(hp_after < hp_before, "target hull must decrease as NPC beam ticks (before={hp_before}, after={hp_after})");
    }

    #[test]
    fn npc_beam_cooldown_starts_after_beam_expires() {
        // When an NPC's beam_remaining_secs reaches zero, cooldown_remaining must
        // be set to a positive value and beam_active must become false.
        use crate::ai_plugin::{EntityPhaserState, AiTokenRegistry};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000005";
        let target_uuid_str = "00000000-0000-0000-0000-000000000006";

        let (npc_entity, _target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid_str).unwrap();
        {
            let mut ps = app.world_mut().get_mut::<EntityPhaserState>(npc_entity).unwrap();
            ps.beam_active = true;
            ps.beam_target = Some(target_uuid_parsed);
            ps.beam_remaining_secs = 0.001; // expires on first tick
        }

        app.update(); // beam expires
        app.update(); // cooldown ticked

        let ps = app.world().get::<EntityPhaserState>(npc_entity).unwrap();
        assert!(!ps.beam_active, "beam_active must be false after beam expires");
        assert!(ps.cooldown_remaining > 0.0, "cooldown_remaining must be positive after beam ends, got {}", ps.cooldown_remaining);
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
            .insert_resource(FactionRegistryResource(crate::config_cache::get_faction_registry()));
        app
    }

    #[test]
    fn tick_ai_controllers_fire_phaser_routes_through_handle_fire_phaser_npc() {
        // Full end-to-end test: an NPC in the `Attacking` state with a target
        // directly in its forward arc and within beam range causes
        // `tick_ai_controllers` to write a `FirePhaser` `InboundMessage`, which
        // `handle_fire_phaser_npc` picks up and sets `EntityPhaserState::beam_active`.
        use crate::ai_plugin::{AiControllerComponent, EntityPhaserState};
        use crate::entity_spawner::{EntityUuid, EntityConsoleHull, WeaponsConsoleSection};
        use crate::entity_config::{BehaviourConfig, StateConfig};
        use crate::damage::ConsoleHull;
        use crate::messages::{GamePhase, Console};
        use bevy::prelude::State;

        let mut app = combined_test_app();

        // Put the simulation in InProgress so tick_ai_controllers runs.
        app.world_mut().insert_resource(State::new(GamePhase::InProgress));

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
        let npc_entity = app.world_mut().spawn((
            crate::entity_spawner::BehaviourSection(behaviour),
            EntityUuid(npc_uuid_str.to_string()),
            EntityPhaserState::default(),
            WeaponsConsoleSection(crate::entity_config::WeaponsConsoleConfig {
                radar_range: 0.0,
                target_range: 0.0,
                fire_arc: 0.0,
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
            }),
            EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 100.0)])),
            Transform::from_xyz(0.0, 0.0, 0.0),
        )).id();

        // Spawn target directly ahead (-Z), well within beam range.
        let _target = app.world_mut().spawn((
            EntityUuid(target_uuid_str.to_string()),
            EntityConsoleHull(ConsoleHull::from_config(&[(Console::CaptainChair, 200.0)])),
            Transform::from_xyz(0.0, 0.0, -10.0),
        )).id();

        // Tick 1: `attach_controllers_on_spawn` runs → AiControllerComponent attached
        //         and token registered in AiTokenRegistry.
        app.update();

        // Set blackboard target so `tick_attacking` fires phasers.
        {
            let mut ctrl = app.world_mut().get_mut::<AiControllerComponent>(npc_entity).unwrap();
            ctrl.controller.blackboard.target = Some(target_uuid_parsed);
        }

        // Tick 2: `tick_ai_controllers` emits FirePhaser InboundMessage.
        // Tick 3: `handle_fire_phaser_npc` reads the message (messages are
        //         available to readers on the tick after they are written).
        app.update();
        app.update();

        let ps = app.world().get::<EntityPhaserState>(npc_entity)
            .expect("NPC must still have EntityPhaserState");
        assert!(
            ps.beam_active,
            "beam_active must be true after tick_ai_controllers → InboundMessage → handle_fire_phaser_npc routing"
        );
    }
}

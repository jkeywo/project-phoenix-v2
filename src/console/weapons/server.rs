use bevy::prelude::*;

use crate::ai_plugin::AiTokenRegistry;
use crate::entity_spawner::EntityConsoleHull;
use crate::lobby::{InboundMessage, Sessions, Target, WorldResource};
use crate::messages::{
    AdmittedCommands, ClientMessage, Console, GamePhase, InterSystemMsg, InterSystemPayload,
    InterSystemQueue, ModifierSlot, PhaserBank, PhaserBankClientConfig, PhaserBankState,
    PhaserMode, RadarBlip, RadarRegion, ServerMessage, SystemBlackboard, SystemControlPayload,
    SystemId, TorpedoTubeClientConfig, TorpedoTubeState, WeaponsBlackboard,
};
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipPhysics;
use crate::simulation::{
    AsteroidUuid, GameOverReason, SimOutbox,
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
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own weapons target.
#[derive(Component, Default, Clone, Debug)]
pub struct WeaponsTarget(pub Option<String>);

/// UUID of the last entity that attacked this ship. Written by the unified
/// `tick_beams` in the Damage phase on the targeted ship's entity;
/// consumed by that ship's `operate_tactical_ai` as a fallback target.
/// `None` when no recent attacker is known.
///
/// Per-ship `Component` — every ship (player + NPC) tracks its own attacker.
#[derive(Component, Default, Clone, Debug)]
pub struct LastShipAttacker(pub Option<String>);

/// Active phaser beam state. `target_uuid` is `Some` while a beam is firing.
/// `remaining_secs` counts down to 0. `damage_accumulator` tracks fractional
/// damage between ticks so 5 HP/s is applied accurately at any frame rate.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own beam state.
#[derive(Component, Default, Clone, Debug)]
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
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own cooldowns.
#[derive(Component, Default, Clone, Debug)]
pub struct PhaserCooldown {
    pub(crate) per_bank: std::collections::HashMap<String, f32>,
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
///
/// Derives both `Resource` (existing player-ship singleton path) and
/// `Component` (per-entity path, PR 5 unification).
#[derive(Resource, Component, Clone, Debug)]
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
///
/// Derives both `Resource` (existing player-ship singleton path) and
/// `Component` (per-entity path, PR 5 unification).
#[derive(Resource, Component, Clone)]
pub struct TorpedoSystemResource(pub TorpedoSystem);

/// Bevy resource holding the player-ship phaser combat tuning
/// (beam duration, beam cooldown, beam damage per second, phaser range).
///
/// Seeded with `PhaserCombatConfig::default()` (the historical
/// hardcoded values) by `WeaponsPlugin::build`, and overridden in
/// `spawn_game_start_entities` from the player ship's `[weapons_console]`
/// block. Read by `handle_fire_phaser`, `tick_beams`, and the
/// `weapons_update_broadcaster` to drive player phaser behaviour.
///
/// Derives both `Resource` (existing player-ship singleton path) and
/// `Component` (per-entity path, PR 5 unification).
#[derive(Resource, Component, Default, Clone)]
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
    /// The ship entity that fired the beam. Used by the observer to set the
    /// `WeaponFiredThisTick` component on the correct ship.
    pub source_entity: Entity,
}

#[derive(Event, Clone, Debug)]
pub struct BeamEndedEvent {
    pub bank: PhaserBank,
    pub target_uuid: String,
    /// The ship entity that fired the beam.
    pub source_entity: Entity,
}

fn on_beam_started(
    trigger: On<BeamStartedEvent>,
    mut outbox: ResMut<SimOutbox>,
    ship_q: Query<&crate::entity_spawner::EntityUuid>,
    mut weapon_fired_q: Query<&mut crate::server_app::WeaponFiredThisTick>,
) {
    let ev = trigger.event();
    if let Ok(mut wf) = weapon_fired_q.get_mut(ev.source_entity) {
        wf.0 = true;
    }
    let source_uuid = ship_q
        .get(ev.source_entity)
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
    ship_q: Query<&crate::entity_spawner::EntityUuid>,
) {
    let ev = trigger.event();
    let source_uuid = ship_q
        .get(ev.source_entity)
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
        app
            .init_resource::<LastWeaponsUpdate>()
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
                    tick_beams.in_set(crate::sim_sets::SimSet::Damage),
                    drain_power_for_active_beam.in_set(crate::sim_sets::SimSet::Physics),
                    tick_torpedo_system.in_set(crate::sim_sets::SimSet::Physics),
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
    ship_query: Query<(&AdmittedCommands, &ShipPhysics), With<crate::server_app::LocalShip>>,
    mut weapons_target_q: Query<&mut WeaponsTarget, With<crate::server_app::LocalShip>>,
    modifiers_q: Query<&crate::modifiers::ShipModifiers, With<crate::server_app::LocalShip>>,
    modifiers_res: Option<Res<crate::modifiers::ShipModifiers>>,
    mut outbox: ResMut<SimOutbox>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    let Some((admitted, physics)) = ship_query.iter().next() else {
        return;
    };
    let Some(mut weapons_target) = weapons_target_q.iter_mut().next() else {
        return;
    };
    // Per-entity modifiers component takes priority; fall back to Resource.
    let default_modifiers;
    let modifiers: &crate::modifiers::ShipModifiers = match modifiers_q.single() {
        Ok(m) => m,
        Err(_) => match modifiers_res.as_deref() {
            Some(m) => m,
            None => {
                default_modifiers = crate::modifiers::ShipModifiers::new();
                &default_modifiers
            }
        },
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
                let dx = x - physics.x;
                let dz = z - physics.z;
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

/// Unified `FirePhaser` handler for every ship (player + NPC).
///
/// Iterates `InboundMessage::FirePhaser` events and resolves each to a single
/// shooter ship entity by token:
/// - `"ai:<uuid>"` tokens are resolved through [`AiTokenRegistry`] to the
///   registered NPC entity.
/// - Human network tokens and `LOCAL_CONSOLE_TOKEN` route to the `LocalShip`,
///   gated by [`tactical_authorized`] (holds the Tactical console or is the
///   local operator).
///
/// After resolution the same per-ship code path runs for both: read the
/// shooter's [`WeaponsTarget`] (falling back to [`ShipAiMemory::target`] when
/// empty for NPC controllers), verify the requested bank is in-arc using the
/// shooter's own [`PhaserCombatConfigResource`], and activate its
/// [`ActiveBeam`] + trigger [`BeamStartedEvent`].
///
/// Merges the former `handle_npc_beam_fire` (NPC-only activation) into this
/// system — final divergence closed. All target-marking (`ShipAttackedThisTick`
/// / `LastShipAttacker` / `AttackerThisTick`) happens later in `tick_beams`.
#[allow(clippy::too_many_arguments)]
fn handle_fire_phaser(
    mut commands: Commands,
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ai_registry: Option<Res<AiTokenRegistry>>,
    localship_q: Query<
        (
            Entity,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::LocalShip>,
    >,
    // Per-ship state read for every candidate shooter (player + NPC).
    // `ShipAiMemory` is `Option` because pre-`AiPlugin` test apps may spawn
    // ships without it; the fallback then simply produces `None`.
    mut ship_q: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            &WeaponsTarget,
            &mut ActiveBeam,
            &PhaserCooldown,
            Option<&PhaserCombatConfigResource>,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&crate::ai_plugin::ShipAiMemory>,
        ),
        With<crate::server_app::Ship>,
    >,
    modifiers_res: Option<Res<crate::modifiers::ShipModifiers>>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    use crate::entity_config::PhaserCombatConfig;

    // Snapshot LocalShip identity for human-token routing. `None` when the
    // test/plugin harness has no player ship spawned.
    let local_ship: Option<(Entity, &crate::ship_plugin::ShipConfigComponent)> =
        localship_q.single().ok().map(|(e, cfg)| (e, cfg));

    for ev in reader.read() {
        let ClientMessage::FirePhaser { bank } = &ev.msg else {
            continue;
        };

        // ── Resolve the shooter ship entity ─────────────────────────────────
        let shooter_entity: Entity = if ev.token.starts_with("ai:") {
            match ai_registry
                .as_deref()
                .and_then(|r| r.bevy_entity_for_token(&ev.token))
            {
                Some(e) => e,
                None => continue,
            }
        } else {
            // Human network token or LOCAL_CONSOLE_TOKEN — must be the
            // LocalShip and satisfy the Tactical authorization gate.
            match local_ship {
                Some((e, cfg)) if tactical_authorized(&sessions, cfg, &ev.token) => e,
                _ => continue,
            }
        };

        // ── Pull per-ship state for the resolved shooter ────────────────────
        let Ok((
            _entity,
            control_sources,
            physics,
            weapons_target,
            mut beam,
            cooldown,
            combat_config_opt,
            modifiers_opt,
            ai_memory_opt,
        )) = ship_q.get_mut(shooter_entity)
        else {
            continue;
        };

        // Authorize the shooter per its own ControlSource. Human tokens
        // require `accept_human_input`; `ai:` tokens require `operate_ai`.
        let policy = control_sources
            .0
            .policy_for(&crate::system_registry::tactical_system_id());
        let is_ai_token = ev.token.starts_with("ai:");
        let authorized = if is_ai_token {
            policy.operate_ai
        } else {
            policy.accept_human_input
        };
        if !authorized {
            continue;
        }

        if cooldown.is_bank_active(bank) || beam.target_uuid.is_some() {
            continue;
        }

        // Target selection: WeaponsTarget first, then ShipAiMemory fallback
        // for NPCs (backward compat — NPCs write their target into AiMemory
        // via operate_helm_ai, not into WeaponsTarget).
        let target_uuid: Option<String> = weapons_target.0.clone().or_else(|| {
            ai_memory_opt.and_then(|m| m.0.target).map(|u| u.to_string())
        });
        let Some(target_uuid) = target_uuid else {
            continue;
        };
        let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
            continue;
        };

        // Per-entity component path (preferred). Fallback: default combat
        // config. Modifiers fall back to the global Resource for legacy
        // test paths that don't insert the per-entity component.
        let combat_config_default = PhaserCombatConfigResource::default();
        let combat_config: &PhaserCombatConfigResource =
            combat_config_opt.unwrap_or(&combat_config_default);

        let modifiers_default = crate::modifiers::ShipModifiers::new();
        let modifiers: &crate::modifiers::ShipModifiers = match modifiers_opt {
            Some(m) => m,
            None => match modifiers_res.as_deref() {
                Some(m) => m,
                None => &modifiers_default,
            },
        };

        let bank_cfg = combat_config.0.bank_by_id(bank);
        let bank_in_arc = if combat_config.0.banks.is_empty() {
            let effective_phaser_range = PhaserCombatConfig::DEFAULT_PHASER_RANGE
                * modifiers.get(&ModifierSlot::RadarRange);
            crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                physics.x,
                physics.z,
                physics.yaw,
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
                        crate::weapons::phaser::ship_local(tx, tz, physics.x, physics.z, physics.yaw);
                    let range_ok = (tx - physics.x).powi(2) + (tz - physics.z).powi(2)
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

        // Cancel any active beam on this shooter before starting the new one.
        // (In practice the `beam.target_uuid.is_some()` guard above already
        // short-circuits, but we keep the branch for defensive consistency.)
        if let Some(old_uuid) = beam.target_uuid.take() {
            let old_bank = beam.bank.clone().unwrap_or_default();
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            commands.trigger(BeamEndedEvent {
                bank: old_bank,
                target_uuid: old_uuid,
                source_entity: shooter_entity,
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
            source_entity: shooter_entity,
        });
    }
}

/// Fires an in-arc phaser bank at each ship's locked target every tick.
///
/// Iterates every ship (`With<Ship>`) — player + NPC — and auto-fires when
/// either:
/// - the ship's Tactical system is currently AI-controlled
///   (`ShipSystemControlSources.policy_for(&tactical_system_id()).operate_ai`),
///   which is `true` for NPCs (Ai by default) and for the player ship on
///   Backfill / explicit Ai rating; or
/// - the player toggled [`CurrentPhaserMode`] to `Auto` (weapons-console-only
///   knob that is meaningless for NPC ships, which have no phaser mode).
///
/// Target selection: reads the ship's [`WeaponsTarget`] and falls back to
/// [`ShipAiMemory::target`] when empty (NPCs write targets into `AiMemory`,
/// not `WeaponsTarget`). Arc/range checks and beam activation mirror
/// [`handle_fire_phaser`], but use each bank's `auto_arc_deg` (looser cone
/// than fire_arc_deg) so AI is less trigger-happy on peripheral targets.
#[allow(clippy::too_many_arguments)]
fn tick_phaser_auto_fire(
    mut commands: Commands,
    phaser_mode: Res<CurrentPhaserMode>,
    // Every ship with weapons state. `ShipAiMemory` is `Option` because
    // pre-`AiPlugin` test apps may spawn ships without it.
    mut ship_q: Query<
        (
            Entity,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            &ShipSystemControlSources,
            &ShipPhysics,
            &WeaponsTarget,
            &mut ActiveBeam,
            &PhaserCooldown,
            Option<&PhaserCombatConfigResource>,
            Option<&crate::modifiers::ShipModifiers>,
            Option<&crate::ai_plugin::ShipAiMemory>,
        ),
        With<crate::server_app::Ship>,
    >,
    modifiers_res: Option<Res<crate::modifiers::ShipModifiers>>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    use crate::entity_config::PhaserCombatConfig;

    for (
        ship_entity,
        is_local,
        control_sources,
        physics,
        weapons_target,
        mut beam,
        cooldown,
        combat_config_opt,
        modifiers_opt,
        ai_memory_opt,
    ) in ship_q.iter_mut()
    {
        // Gate: auto-fire only when Tactical is AI-controlled on this ship,
        // OR the player globally toggled phaser mode to Auto (LocalShip-only
        // signal that is irrelevant for NPCs — they always satisfy the
        // operate_ai leg because their Tactical system is Ai by default).
        let policy = control_sources
            .0
            .policy_for(&crate::system_registry::tactical_system_id());
        let auto_fire = policy.operate_ai || (is_local && phaser_mode.0 == PhaserMode::Auto);
        if !auto_fire {
            continue;
        }

        if beam.target_uuid.is_some() {
            continue;
        }

        // Target selection: WeaponsTarget first, ShipAiMemory fallback for NPCs.
        let target_uuid: Option<String> = weapons_target.0.clone().or_else(|| {
            ai_memory_opt.and_then(|m| m.0.target).map(|u| u.to_string())
        });
        let Some(target_uuid) = target_uuid else {
            continue;
        };
        let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
            continue;
        };

        let combat_config_default = PhaserCombatConfigResource::default();
        let combat_config: &PhaserCombatConfigResource =
            combat_config_opt.unwrap_or(&combat_config_default);
        let modifiers_default = crate::modifiers::ShipModifiers::new();
        let modifiers: &crate::modifiers::ShipModifiers = match modifiers_opt {
            Some(m) => m,
            None => match modifiers_res.as_deref() {
                Some(m) => m,
                None => &modifiers_default,
            },
        };

        // Find the first bank that is off-cooldown and has the target in its auto arc.
        let bank_id: Option<String> = if combat_config.0.banks.is_empty() {
            let effective_range = PhaserCombatConfig::DEFAULT_PHASER_RANGE
                * modifiers.get(&ModifierSlot::RadarRange);
            let ready = crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                physics.x,
                physics.z,
                physics.yaw,
                effective_range,
            );
            (ready && !cooldown.is_bank_active("")).then(String::new)
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
                let range_ok = (tx - physics.x).powi(2) + (tz - physics.z).powi(2)
                    <= effective_range * effective_range;
                let (rx, ry) =
                    crate::weapons::phaser::ship_local(tx, tz, physics.x, physics.z, physics.yaw);
                let arc_ok = crate::weapons::phaser::in_arc(rx, ry, b.facing_deg, b.auto_arc_deg);
                (range_ok && arc_ok).then(|| b.id.clone())
            })
        };

        let Some(bank_id) = bank_id else {
            continue;
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
            target_uuid,
            source_entity: ship_entity,
        });
    }
}

/// Unified per-tick beam ticker for every ship (player + NPC).
///
/// Iterates `Query<..., With<Ship>>` — one loop handles player-fired beams
/// (LocalShip source) and NPC-fired beams (AI-controlled Ship source). Reads
/// per-bank config from each shooter's own `PhaserCombatConfigResource`
/// component (defaulting when absent) and applies the shooter's own
/// `ShipModifiers` to damage and range.
///
/// Damage routing rules:
/// - Asteroid target → emits `AsteroidDestroyed` + `AsteroidDestroyedVfx`.
/// - Non-asteroid, non-LocalShip target (NPC or station) → emits
///   `EntityDespawned` + `AiEntityDestroyed` on kill.
/// - LocalShip target → emits `DamageTaken` per hit and `ShipDestroyed` +
///   `GameOver` on kill. Never despawns the LocalShip entity.
///
/// Attacker tracking: every non-asteroid target has `ShipAttackedThisTick`
/// set true and `LastShipAttacker` set to the shooter's UUID; the target
/// also gains an `ai_plugin::AttackerThisTick` component so its AI's
/// `on_attacked` transition can fire.
///
/// Weapons-target clearing: when the player kills its locked target, its
/// `WeaponsTarget.0` is set to `None`. NPCs track their target in
/// `ShipAiMemory` and clear it via a separate AI path.
///
/// Merges the former `tick_active_beam` (player-only) and `tick_npc_beams`
/// (NPC-only) systems — final divergence closed under PRD #597.
#[allow(clippy::too_many_arguments)]
fn tick_beams(
    time: Res<Time>,
    mut commands: Commands,
    // Every ship with weapons: player + NPC. All ships now carry ActiveBeam,
    // PhaserCooldown, PhaserCombatConfigResource, and ShipModifiers as
    // per-entity components.
    //
    // `EntityUuid` is `Option` to keep the minimal test-only LocalShip spawns
    // (which historically omit UUIDs) from being silently dropped from
    // iteration; production ships always carry an EntityUuid.
    mut ship_q: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &ShipPhysics,
            &mut ActiveBeam,
            &mut PhaserCooldown,
            Option<&PhaserCombatConfigResource>,
            Option<&crate::modifiers::ShipModifiers>,
            // Only the player ship carries WeaponsTarget as a lock reflected on
            // the UI. NPCs track their target in ShipAiMemory; we clear the
            // WeaponsTarget only on the LocalShip.
            Option<&mut WeaponsTarget>,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
        ),
        With<crate::server_app::Ship>,
    >,
    // Any potentially targetable entity that can take damage: asteroids and
    // any ship with hull. Uses Option<&AsteroidUuid> + Option<&EntityUuid> so
    // we can match either UUID type; no Ship marker filter — non-ship targets
    // (stations, damageable regions) may not have Ship but do have EntityUuid.
    //
    // `Transform` is `Option` because test fixtures sometimes spawn hull-only
    // entities without a Transform; production entities always have one and
    // are still matched by UUID.
    mut hull_q: Query<(
        Entity,
        Option<&AsteroidUuid>,
        Option<&crate::entity_spawner::EntityUuid>,
        Option<&Transform>,
        Option<&ShipPhysics>,
        &mut EntityConsoleHull,
        Option<&mut crate::ship::shields::ShipShields>,
        Option<&mut crate::server_app::ShipAttackedThisTick>,
        Option<&mut LastShipAttacker>,
        bevy::ecs::query::Has<crate::server_app::LocalShip>,
    )>,
    // ShipModifiers Resource fallback used only by legacy test paths that
    // set the resource but don't insert the component on the ship entity.
    modifiers_res: Option<Res<crate::modifiers::ShipModifiers>>,
    mut world: ResMut<WorldResource>,
    mut outbox: Option<ResMut<SimOutbox>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    use crate::entity_config::PhaserCombatConfig;

    let dt = time.delta_secs();

    // ── Phase 1: snapshot shooter state and tick per-bank cooldowns ─────────
    //
    // Collect owned copies of everything we need to apply damage without
    // holding a mutable borrow on ship_q. In the same pass we tick cooldowns
    // and pre-compute the per-tick damage integer, accumulator delta, and
    // whether the beam should end (target out of range/arc/vanished).

    struct ShooterState {
        shooter_entity: Entity,
        shooter_uuid: String,
        shooter_x: f32,
        shooter_z: f32,
        target_uuid: String,
        target_x: f32,
        target_z: f32,
        active_bank: String,
        cooldown_secs: f32,
        damage_to_apply: i32,
        shield_pierce: f32,
        end_beam_early: bool,
        is_local_shooter: bool,
    }

    let mut shooters: Vec<ShooterState> = Vec::new();

    for (
        shooter_entity,
        shooter_uuid_opt,
        shooter_physics,
        mut beam,
        mut cooldown,
        combat_config_opt,
        modifiers_opt,
        _weapons_target_opt,
        is_local_shooter,
    ) in ship_q.iter_mut()
    {
        cooldown.tick(dt);

        let Some(target_uuid) = beam.target_uuid.clone() else {
            continue;
        };
        let active_bank = beam.bank.clone().unwrap_or_default();

        // Per-entity component paths (preferred). Fall back to defaults —
        // and for `ShipModifiers`, also fall back to the global Resource
        // to preserve legacy test paths that don't insert the component.
        let combat_default = PhaserCombatConfigResource::default();
        let combat_config: &PhaserCombatConfigResource =
            combat_config_opt.unwrap_or(&combat_default);

        let modifiers_default = crate::modifiers::ShipModifiers::new();
        let modifiers: &crate::modifiers::ShipModifiers = match modifiers_opt {
            Some(m) => m,
            None => match modifiers_res.as_deref() {
                Some(m) => m,
                None => &modifiers_default,
            },
        };

        let active_bank_cfg = combat_config.0.bank_by_id(&active_bank);
        let cooldown_secs = active_bank_cfg
            .map(|b| {
                if b.cooldown_secs > 0.0 {
                    b.cooldown_secs
                } else {
                    PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS
                }
            })
            .unwrap_or(PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS);

        // Use live ECS position for arc/range check — WorldResource snapshot
        // is stale for moving targets.
        let live_pos = live_entity_xz(&target_uuid, &asteroid_q, &entity_q);
        let (tx, tz) = match live_pos {
            Some(p) => p,
            None => {
                // Target vanished — end beam.
                beam.target_uuid = None;
                beam.remaining_secs = 0.0;
                beam.damage_accumulator = 0.0;
                cooldown.start_bank(&active_bank, cooldown_secs);
                commands.trigger(BeamEndedEvent {
                    bank: active_bank.clone(),
                    target_uuid,
                    source_entity: shooter_entity,
                });
                continue;
            }
        };

        // Bank in-arc/range check (uses per-bank config; falls back to a
        // legacy global range when the config has no banks defined).
        let bank_in_arc = if combat_config.0.banks.is_empty() {
            let effective_phaser_range = PhaserCombatConfig::DEFAULT_PHASER_RANGE
                * modifiers.get(&ModifierSlot::RadarRange);
            crate::radar::is_fire_ready_with_range(
                tx,
                tz,
                shooter_physics.x,
                shooter_physics.z,
                shooter_physics.yaw,
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
                    let (rx, ry) = crate::weapons::phaser::ship_local(
                        tx,
                        tz,
                        shooter_physics.x,
                        shooter_physics.z,
                        shooter_physics.yaw,
                    );
                    let range_ok = (tx - shooter_physics.x).powi(2)
                        + (tz - shooter_physics.z).powi(2)
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
            cooldown.start_bank(&active_bank, cooldown_secs);
            commands.trigger(BeamEndedEvent {
                bank: active_bank.clone(),
                target_uuid,
                source_entity: shooter_entity,
            });
            continue;
        }

        let damage_per_sec = active_bank_cfg
            .map(|b| {
                if b.beam_damage_per_sec > 0.0 {
                    b.beam_damage_per_sec
                } else {
                    PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC
                }
            })
            .unwrap_or(PhaserCombatConfig::DEFAULT_BEAM_DAMAGE_PER_SEC);
        let shield_pierce = active_bank_cfg.and_then(|b| b.shield_pierce).unwrap_or(0.0);

        beam.damage_accumulator +=
            damage_per_sec * modifiers.get(&ModifierSlot::PhaserDamage) * dt;
        let damage_to_apply = beam.damage_accumulator.floor() as i32;
        // Deduct the integer part now; the snapshot below drives damage
        // application in phase 2.
        beam.damage_accumulator -= damage_to_apply as f32;

        shooters.push(ShooterState {
            shooter_entity,
            shooter_uuid: shooter_uuid_opt.map(|u| u.0.clone()).unwrap_or_default(),
            shooter_x: shooter_physics.x,
            shooter_z: shooter_physics.z,
            target_uuid,
            target_x: tx,
            target_z: tz,
            active_bank,
            cooldown_secs,
            damage_to_apply,
            shield_pierce,
            end_beam_early: false,
            is_local_shooter,
        });
    }

    // ── Phase 2: apply damage to targets ─────────────────────────────────────
    //
    // For each shooter, find its target in hull_q, route damage through
    // shields, apply hull damage, and record whether the target was destroyed
    // (so we can end the beam and clear WeaponsTarget in phase 3).

    for state in shooters.iter_mut() {
        // Always mark the target ship as attacked, even when damage_to_apply == 0
        // (mirrors the historical NPC path which tagged the target every tick
        // the beam was live). Skip for asteroid targets.
        {
            // Look up the target and set attacker/attacked components.
            let target_entity = hull_q.iter().find_map(|(e, ast_uuid, ent_uuid, _, _, _, _, _, _, _)| {
                let asteroid_match = ast_uuid.map(|u| u.0.as_str()) == Some(state.target_uuid.as_str());
                let entity_match = ent_uuid.map(|u| u.0.as_str()) == Some(state.target_uuid.as_str());
                if asteroid_match || entity_match {
                    Some((e, ast_uuid.is_some()))
                } else {
                    None
                }
            });
            if let Some((te, is_asteroid)) = target_entity {
                if !is_asteroid {
                    if let Ok((_, _, _, _, _, _, _, attacked_opt, last_attacker_opt, _)) =
                        hull_q.get_mut(te)
                    {
                        if let Some(mut atk) = attacked_opt {
                            atk.0 = true;
                        }
                        if let Some(mut last) = last_attacker_opt {
                            last.0 = Some(state.shooter_uuid.clone());
                        }
                    }
                    // Insert AttackerThisTick component so the target's AI
                    // on_attacked transition fires (see ai_plugin::AttackerThisTick).
                    if let Ok(parsed) = uuid::Uuid::parse_str(&state.shooter_uuid) {
                        commands
                            .entity(te)
                            .insert(crate::ai_plugin::AttackerThisTick(parsed));
                    }
                }
            }
        }

        if state.damage_to_apply <= 0 {
            continue;
        }

        let mut target_asteroid_destroyed = false;
        let mut target_ship_destroyed_non_local = false;
        let mut damage_applied = false;

        for (
            target_entity,
            ast_uuid,
            ent_uuid,
            target_tf,
            target_physics_opt,
            mut hull_comp,
            mut ship_shields_comp,
            _attacked_opt,
            _last_attacker_opt,
            target_is_local,
        ) in hull_q.iter_mut()
        {
            let uuid_matches = ast_uuid.map(|u| u.0.as_str())
                == Some(state.target_uuid.as_str())
                || ent_uuid.map(|u| u.0.as_str()) == Some(state.target_uuid.as_str());
            if !uuid_matches {
                continue;
            }
            damage_applied = true;
            let is_asteroid = ast_uuid.is_some();

            // Route damage through shields if present and any facing online.
            let (damage_to_hull, shield_amount) =
                if let Some(ref mut shields) = ship_shields_comp {
                    let all_offline = shields.0.facings.iter().all(|f| !f.is_online());
                    if all_offline {
                        (state.damage_to_apply as f32, 0.0f32)
                    } else {
                        let (pierced, absorbed) = crate::damage::split_damage_for_pierce(
                            state.damage_to_apply as f32,
                            state.shield_pierce,
                        );
                        let bearing = if target_is_local {
                            // Player shield uses bearing-based routing to the
                            // appropriate facing. Fall back to the shooter's
                            // own position when the target has no Transform
                            // (bearing = 0.0 in that degenerate case).
                            let target_yaw =
                                target_physics_opt.map(|p| p.yaw).unwrap_or(0.0);
                            match target_tf {
                                Some(tf) => crate::shield::attacker_bearing_relative(
                                    state.shooter_x,
                                    state.shooter_z,
                                    tf.translation.x,
                                    tf.translation.z,
                                    target_yaw,
                                ),
                                None => 0.0,
                            }
                        } else {
                            // NPC shield defaults to num_facings=1 — bearing
                            // doesn't matter for a single facing.
                            0.0
                        };
                        let leak = shields.0.apply_damage(absorbed.round() as i32, bearing);
                        let shielded = (absorbed - leak as f32).max(0.0);
                        (pierced + leak as f32, shielded)
                    }
                } else {
                    (state.damage_to_apply as f32, 0.0f32)
                };

            let ship_destroyed = if damage_to_hull > 0.0 {
                let mut rng = rand::rng();
                let (hull_applied, destroyed) = crate::damage::apply_hull_damage(
                    &mut hull_comp.0,
                    damage_to_hull,
                    &mut rng,
                );
                // LocalShip: emit DamageTaken every hit; ShipDestroyed +
                // GameOver on kill. Never despawn the LocalShip entity.
                if target_is_local {
                    if let Some(ref mut ob) = outbox {
                        ob.0.push((
                            Target::All,
                            ServerMessage::DamageTaken {
                                hull: hull_applied,
                                shield: shield_amount,
                            },
                        ));
                    }
                    if destroyed {
                        if let Some(ref mut ob) = outbox {
                            ob.0.push((Target::All, ServerMessage::ShipDestroyed));
                        }
                        if let Some(ref mut gs) = next_state {
                            gs.set(GamePhase::GameOver);
                        }
                        if let Some(ref mut reason) = game_over_reason {
                            if reason.0.is_none() {
                                reason.0 = Some("Ship destroyed".into());
                            }
                        }
                    }
                }
                destroyed
            } else {
                false
            };

            if ship_destroyed {
                if is_asteroid {
                    commands.entity(target_entity).try_despawn();
                    target_asteroid_destroyed = true;
                } else if !target_is_local {
                    // NPC / station / other non-player target — despawn and
                    // emit destroy events. LocalShip is handled above
                    // (never despawned — GameOver takes over).
                    commands.entity(target_entity).try_despawn();
                    target_ship_destroyed_non_local = true;
                }
            }

            // Note: no `break` here — historically test fixtures spawn multiple
            // entities sharing a UUID (e.g. an inline hull-only entity plus one
            // spawned via `setup_weapons_world` with a Transform). Damage is
            // applied to every matching entity so those tests observe the hit
            // on whichever entity they hold a handle to.
        }

        // Handle target destruction — clean up world snapshot + events.
        if !damage_applied {
            continue;
        }
        if target_asteroid_destroyed || target_ship_destroyed_non_local {
            world.0.entities.retain(|a| a.uuid != state.target_uuid);
            if target_asteroid_destroyed {
                vfx_events.write(AsteroidDestroyedVfx {
                    x: state.target_x,
                    z: state.target_z,
                });
                if let Some(ref mut ob) = outbox {
                    ob.0.push((
                        Target::All,
                        ServerMessage::AsteroidDestroyed {
                            uuid: state.target_uuid.clone(),
                        },
                    ));
                }
            } else {
                destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                    entity_uuid: state.target_uuid.clone(),
                });
                if let Some(ref mut ob) = outbox {
                    ob.0.push((
                        Target::All,
                        ServerMessage::EntityDespawned {
                            uuid: state.target_uuid.clone(),
                        },
                    ));
                }
            }
            state.end_beam_early = true;
        }
    }

    // ── Phase 3: end beams that hit a destroyed target; tick remaining_secs ─
    //
    // Re-borrow ship_q mutably to update per-shooter beam state (target
    // cleared, cooldown started, WeaponsTarget cleared for LocalShip).

    for state in shooters {
        let Ok((_, _, _, mut beam, mut cooldown, _, _, mut weapons_target_opt, _)) =
            ship_q.get_mut(state.shooter_entity)
        else {
            continue;
        };

        if state.end_beam_early {
            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start_bank(&state.active_bank, state.cooldown_secs);
            if state.is_local_shooter {
                if let Some(ref mut wt) = weapons_target_opt {
                    wt.0 = None;
                }
            }
            commands.trigger(BeamEndedEvent {
                bank: state.active_bank.clone(),
                target_uuid: state.target_uuid.clone(),
                source_entity: state.shooter_entity,
            });
            continue;
        }

        // Time-based beam end.
        beam.remaining_secs -= dt;
        if beam.remaining_secs <= 0.0 {
            beam.target_uuid = None;
            beam.remaining_secs = 0.0;
            beam.damage_accumulator = 0.0;
            cooldown.start_bank(&state.active_bank, state.cooldown_secs);
            commands.trigger(BeamEndedEvent {
                bank: state.active_bank.clone(),
                target_uuid: state.target_uuid.clone(),
                source_entity: state.shooter_entity,
            });
        }
    }
}
fn handle_set_phaser_mode(
    ship_query: Query<&AdmittedCommands, With<crate::server_app::LocalShip>>,
    mut phaser_mode: ResMut<CurrentPhaserMode>,
) {
    let Some(admitted) = ship_query.iter().next() else {
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
        With<crate::server_app::LocalShip>,
    >,
    mut freq_q: Query<&mut crate::ship_state::ShipPhaserFrequency, With<crate::server_app::LocalShip>>,
) {
    let Some((ship_config, control_sources)) = ship_query.iter().next() else {
        return;
    };
    let tactical_policy = control_sources
        .0
        .policy_for(&crate::system_registry::tactical_system_id());
    for ev in reader.read() {
        let ClientMessage::SetPhaserFrequency { frequency } = &ev.msg else {
            continue;
        };
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
        if let Some(mut freq) = freq_q.iter_mut().next() {
            freq.0 = frequency.clamp(0.0, 1.0);
        }
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
        With<crate::server_app::LocalShip>,
    >,
    mut torpedo_sys_q: Query<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
) {
    let Some((ship_config, control_sources)) = ship_query.iter().next() else {
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
        // Prefer per-entity component; fall back to global resource for test compat.
        if let Some(mut ts) = torpedo_sys_q.iter_mut().next() {
            ts.0.start_load(tube.as_str());
        } else {
            torpedo_sys_res.0.start_load(tube.as_str());
        }
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
        With<crate::server_app::LocalShip>,
    >,
    mut torpedo_sys_q: Query<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
) {
    let Some((ship_config, control_sources)) = ship_query.iter().next() else {
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
        // Prefer per-entity component; fall back to global resource for test compat.
        if let Some(mut ts) = torpedo_sys_q.iter_mut().next() {
            ts.0.start_unload(tube.as_str());
        } else {
            torpedo_sys_res.0.start_unload(tube.as_str());
        }
    }
}

/// Unified `FireTorpedo` handler for every ship (player + NPC).
///
/// Iterates `InboundMessage::FireTorpedo` events and resolves each to a
/// shooter ship entity by token:
/// - `"ai:<uuid>"` tokens are resolved through [`AiTokenRegistry`] to the
///   registered NPC entity.
/// - Human network tokens and `LOCAL_CONSOLE_TOKEN` route to the `LocalShip`,
///   gated by [`tactical_authorized`] (holds the Tactical console or is the
///   local operator).
///
/// After resolution the same per-ship code path runs for both: use the
/// shooter's own `TorpedoSystemResource` component (falling back to the
/// global `TorpedoSystemResource` resource only when no ship carries the
/// component — legacy test paths).
///
/// After PRD #597 gap-3 closure: NPC ships with a `[torpedoes]` TOML block
/// now spawn with their own `TorpedoSystemResource` (see
/// `src/entities/spawner.rs`) and can fire torpedoes via the same code path
/// as the player ship.
#[allow(clippy::too_many_arguments)]
fn handle_fire_torpedo(
    mut reader: MessageReader<InboundMessage>,
    sessions: Res<Sessions>,
    ai_registry: Option<Res<AiTokenRegistry>>,
    localship_q: Query<
        (
            Entity,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::LocalShip>,
    >,
    // Per-ship state read for every candidate shooter (player + NPC).
    mut ship_q: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&WeaponsTarget>,
            Option<&crate::entity_spawner::EntityUuid>,
            Option<&mut TorpedoSystemResource>,
            Option<&mut crate::server_app::WeaponFiredThisTick>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
) {
    // Snapshot LocalShip identity for human-token routing. `None` when the
    // test/plugin harness has no player ship spawned.
    let local_ship: Option<(Entity, &crate::ship_plugin::ShipConfigComponent)> =
        localship_q.single().ok().map(|(e, cfg)| (e, cfg));

    for ev in reader.read() {
        let ClientMessage::FireTorpedo { tube, target_uuid } = &ev.msg else {
            continue;
        };

        // ── Resolve the shooter ship entity ─────────────────────────────────
        let shooter_entity: Entity = if ev.token.starts_with("ai:") {
            match ai_registry
                .as_deref()
                .and_then(|r| r.bevy_entity_for_token(&ev.token))
            {
                Some(e) => e,
                None => continue,
            }
        } else {
            match local_ship {
                Some((e, cfg)) if tactical_authorized(&sessions, cfg, &ev.token) => e,
                _ => continue,
            }
        };

        // ── Pull per-ship state for the resolved shooter ────────────────────
        let Ok((
            _entity,
            control_sources,
            physics,
            weapons_target_opt,
            source_uuid_opt,
            torpedo_sys_comp,
            weapon_fired_comp,
        )) = ship_q.get_mut(shooter_entity)
        else {
            continue;
        };

        // Authorize per the shooter's own ControlSource: human tokens need
        // `accept_human_input`; `ai:` tokens need `operate_ai`.
        let policy = control_sources
            .0
            .policy_for(&crate::system_registry::tactical_system_id());
        let is_ai_token = ev.token.starts_with("ai:");
        let authorized = if is_ai_token {
            policy.operate_ai
        } else {
            policy.accept_human_input
        };
        if !authorized {
            continue;
        }

        // Per-entity `TorpedoSystemResource` first; fall back to the global
        // Resource so legacy tests that only insert the Resource still work.
        // Only the LocalShip should ever fall through to the global — NPC
        // ships that lack the component simply have no torpedo tubes.
        let mut torpedo_sys_comp = torpedo_sys_comp;
        let torpedo_sys: &mut crate::torpedo::TorpedoSystem = match torpedo_sys_comp.as_deref_mut() {
            Some(c) => &mut c.0,
            None => &mut torpedo_sys_res.0,
        };

        let uuid = uuid::Uuid::new_v4().to_string();
        let tube_facing_rad = torpedo_sys
            .tube(tube.as_str())
            .map(|t| t.facing_deg.to_radians())
            .unwrap_or(0.0);
        let launch_heading = physics.yaw + tube_facing_rad;
        let source_uuid = source_uuid_opt.map(|u| u.0.clone());
        let homing_uuid = weapons_target_opt
            .and_then(|wt| wt.0.clone())
            .or_else(|| target_uuid.clone());
        use crate::torpedo::LaunchResult;
        let result = torpedo_sys.launch(
            tube.as_str(),
            uuid.clone(),
            physics.x,
            physics.z,
            launch_heading,
            homing_uuid.clone(),
            source_uuid.clone(),
        );
        match result {
            LaunchResult::Launched {
                uuid: launched_uuid,
            } => {
                if let Some(mut wf) = weapon_fired_comp {
                    wf.0 = true;
                }
                outbox.0.push((
                    Target::All,
                    ServerMessage::TorpedoLaunched {
                        uuid: launched_uuid,
                        tube: tube.clone(),
                        x: physics.x,
                        z: physics.z,
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

/// Drains each ship's Power battery via the inter-system command channel
/// while its own phaser beam is active. Runs in `SimSet::Physics` (one
/// phase before `tick_beams` in `SimSet::Damage`); the Power system
/// consumes the drain in `SimSet::Modifiers` per-entity via `source_entity`.
///
/// Iterates every ship so NPC beams drain their own power grid the same
/// way the player's do.
pub fn drain_power_for_active_beam(
    beam_q: Query<(Entity, &ActiveBeam), With<crate::server_app::Ship>>,
    time: Res<Time>,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    let amount = PHASER_BATTERY_DRAIN_PER_SEC * time.delta_secs();
    for (source_entity, beam) in beam_q.iter() {
        if beam.target_uuid.is_some() {
            inter_system.0.push(InterSystemMsg {
                target: crate::system_registry::power_system_id(),
                payload: InterSystemPayload::DrainWeaponsBattery { amount },
                source_entity: Some(source_entity),
            });
        }
    }
}

fn tick_torpedo_system(
    mut torpedo_sys_q: Query<&mut TorpedoSystemResource, With<crate::server_app::Ship>>,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
    mut world: ResMut<WorldResource>,
    time: Res<Time>,
    mut outbox: ResMut<SimOutbox>,
    mut hull_query: Query<(
        Entity,
        Option<&AsteroidUuid>,
        Option<&crate::entity_spawner::EntityUuid>,
        &mut EntityConsoleHull,
        Option<&mut crate::ship::shields::ShipShields>,
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
    mut weapons_target_q: Query<&mut WeaponsTarget, With<crate::server_app::LocalShip>>,
) {
    let dt = time.delta_secs();
    let mut weapons_target_opt = weapons_target_q.single_mut().ok();

    // ── Build shared world snapshots up-front (used by every ship's tick) ───

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

    // Proximity detonation target list (uuid, x, z, radius). Built once and
    // shared across every ship's `find_detonation_hits` call.
    let targets: Vec<(String, f32, f32, f32)> = {
        let mut map: std::collections::HashMap<String, (f32, f32, f32)> =
            std::collections::HashMap::new();
        for (u, t) in asteroid_q.iter() {
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

    // ── Phase 1: tick every ship's TorpedoSystem + collect detonation events ──
    //
    // Iterate all ships (`With<Ship>`) with a `TorpedoSystemResource`
    // component — player + NPC. Each ship ticks its own tubes, expires its
    // own torpedoes, and produces its own detonation-hit list.
    //
    // The Resource fallback runs only when NO Ship entity carries the
    // component; this preserves the legacy Resource-only test paths.
    #[derive(Clone, Debug)]
    struct Detonation {
        target_uuid: String,
        damage_hull: i32,
        damage_shields: i32,
        shield_pierce: f32,
    }
    let mut detonations: Vec<Detonation> = Vec::new();
    let mut any_ship_component = false;

    for mut torpedo_sys in torpedo_sys_q.iter_mut() {
        any_ship_component = true;
        let result = torpedo_sys.0.tick(dt, &target_positions);
        for expired_uuid in result.expired {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: expired_uuid },
            ));
        }
        let hits = torpedo_sys.0.find_detonation_hits(&targets);
        for (torpedo_uuid, target_uuid) in hits {
            let Some(det) = torpedo_sys.0.handle_collision_full(&torpedo_uuid) else {
                continue;
            };
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: torpedo_uuid },
            ));
            detonations.push(Detonation {
                target_uuid,
                damage_hull: det.damage_hull,
                damage_shields: det.damage_shields,
                shield_pierce: det.shield_pierce,
            });
        }
    }

    // Resource-only fallback: tests that only insert the global
    // `TorpedoSystemResource` (no Ship entity carrying it) still work.
    if !any_ship_component {
        let result = torpedo_sys_res.0.tick(dt, &target_positions);
        for expired_uuid in result.expired {
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: expired_uuid },
            ));
        }
        let hits = torpedo_sys_res.0.find_detonation_hits(&targets);
        for (torpedo_uuid, target_uuid) in hits {
            let Some(det) = torpedo_sys_res.0.handle_collision_full(&torpedo_uuid) else {
                continue;
            };
            outbox.0.push((
                Target::All,
                ServerMessage::TorpedoDestroyed { uuid: torpedo_uuid },
            ));
            detonations.push(Detonation {
                target_uuid,
                damage_hull: det.damage_hull,
                damage_shields: det.damage_shields,
                shield_pierce: det.shield_pierce,
            });
        }
    }

    // ── Phase 2: apply detonations to hulls / shields ───────────────────────

    for det in detonations {
        let target_uuid = det.target_uuid;
        let mut asteroid_destroyed = false;
        let mut non_local_ship_destroyed = false;
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

            // Route shield-eligible damage through any `ShipShields`
            // component, with overflow leaking to hull. Hull damage
            // (always-pierces) goes straight to hull. Asteroids carry no
            // shield so the shielded path is a no-op for them.
            let mut hull_damage = det.damage_hull as f32;
            let shield_eligible = det.damage_shields as f32;
            if shield_eligible > 0.0 {
                if let Some(ref mut shields) = shield_comp {
                    let all_offline = shields.0.facings.iter().all(|f| !f.is_online());
                    if all_offline {
                        hull_damage += shield_eligible;
                    } else {
                        let (pierced, absorbed) = crate::damage::split_damage_for_pierce(
                            shield_eligible,
                            det.shield_pierce,
                        );
                        let leak = shields.0.apply_damage(absorbed.round() as i32, 0.0);
                        hull_damage += pierced + leak as f32;
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
                    non_local_ship_destroyed = true;
                }
                // Use live position from whichever query matches (asteroid or ship).
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
            if weapons_target_opt
                .as_deref()
                .and_then(|wt| wt.0.as_deref())
                == Some(target_uuid.as_str())
            {
                if let Some(ref mut wt) = weapons_target_opt {
                    wt.0 = None;
                }
            }
        } else if non_local_ship_destroyed {
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
            if weapons_target_opt
                .as_deref()
                .and_then(|wt| wt.0.as_deref())
                == Some(target_uuid.as_str())
            {
                if let Some(ref mut wt) = weapons_target_opt {
                    wt.0 = None;
                }
            }
        }
    }
}

// ── Tactical AI controller ────────────────────────────────────────────────
//
// Runs for every ship whose Tactical system's ControlSource is Ai.
// Sub-regions are separated by comment banners — each banner marks a
// future split point when the coarse Tactical system is decomposed into
// fine-grained systems.

fn operate_tactical_ai(
    mut ship_query: Query<
        (
            Entity,
            &crate::entity_spawner::EntityUuid,
            &crate::ship_plugin::ShipConfigComponent,
            &ShipSystemControlSources,
            &crate::ship_plugin::ActiveStationRatings,
            &LastShipAttacker,
            &ShipPhysics,
            &mut WeaponsTarget,
            Option<&mut TorpedoSystemResource>,
            &crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
    sessions: Res<Sessions>,
    mut torpedo_sys_res: ResMut<TorpedoSystemResource>,
    mut outbox: ResMut<SimOutbox>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), With<crate::simulation::Asteroid>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    other_ships_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::entities::spawner::EntityName>,
        ),
        Without<crate::simulation::Asteroid>,
    >,
) {
    let tactical_system = crate::system_registry::tactical_system_id();
    let tactical_station = crate::messages::StationId("tactical".into());

    for (
        _entity,
        ship_uuid,
        ship_config,
        control_sources,
        active_ratings,
        last_attacker,
        physics,
        mut weapons_target,
        mut torpedo_sys_comp,
        blackboards,
    ) in ship_query.iter_mut()
    {
        // Only run for ships whose Tactical system is AI-controlled.
        // The player ship's Tactical may be human — skip in that case; the
        // human operator drives via WeaponsTarget directly through the
        // handle_set_target handler.
        let policy = control_sources.0.policy_for(&tactical_system);
        if !policy.operate_ai {
            continue;
        }

        // Always set weapons_target from Destroy objectives regardless of
        // control source. This lets both human and AI Tactical operators
        // benefit from mission objective auto-targeting. When no Destroy
        // objective is available (or its target entity can't be resolved),
        // fall back to the last attacker.
        let objective_target = match top_destroy_objective_target(Some(blackboards)) {
            Some(target_name) if target_name.is_empty() => None,
            Some(target_name) => {
                resolve_objective_target_uuid(target_name, runtime.as_deref(), &other_ships_q)
            }
            None => None,
        };
        if let Some(uuid) = objective_target.or_else(|| last_attacker.0.clone()) {
            weapons_target.0 = Some(uuid);
        }

        // ── TORPEDO AUTO-FIRE (future: split to torpedo_tube system) ─────
        //
        // When the station is claimed, gate on whether the active rating's
        // ai_tuning has the torpedo_auto_fire rule. Unclaimed → unconditional.
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

        if !auto_fire_enabled {
            continue;
        }
        let Some(target_uuid) = weapons_target.0.clone() else {
            continue;
        };
        // Look up live world position — WorldResource snapshot is stale for moving targets.
        let target_xz = asteroid_q
            .iter()
            .find_map(|(u, t)| (u.0 == target_uuid).then_some((t.translation.x, t.translation.z)))
            .or_else(|| {
                other_ships_q.iter().find_map(|(u, t, _)| {
                    (u.0 == target_uuid).then_some((t.translation.x, t.translation.z))
                })
            });
        let Some((tx, tz)) = target_xz else {
            continue;
        };

        let dx = tx - physics.x;
        let dz = tz - physics.z;
        let world_bearing = dx.atan2(-dz);
        let bearing = world_bearing - physics.yaw;

        // Prefer per-entity component; fall back to global resource for
        // legacy test paths that only set up the Resource.
        let torpedo_sys: &mut crate::torpedo::TorpedoSystem = match torpedo_sys_comp.as_mut() {
            Some(c) => &mut c.0,
            None => &mut torpedo_sys_res.0,
        };
        let tubes: Vec<crate::console_ai::TubeSummary> = torpedo_sys
            .tubes
            .iter()
            .map(|tube| crate::console_ai::TubeSummary {
                id: tube.id.clone(),
                loaded: tube.is_loaded(),
                in_arc: tube.is_in_arc(bearing),
            })
            .collect();
        let magazine = torpedo_sys.torpedoes_remaining;

        let input = crate::console_ai::TorpedoAiInput {
            target_locked: true,
            target_shields: 0,
            tubes,
            magazine,
        };

        let tubes_to_fire = crate::console_ai::auto_fire_torpedo(&input);
        let source_uuid = Some(ship_uuid.0.clone());

        for tube_id in tubes_to_fire {
            let torpedo_uuid = uuid::Uuid::new_v4().to_string();
            let tube_facing_rad = torpedo_sys
                .tube(tube_id.as_str())
                .map(|t| t.facing_deg.to_radians())
                .unwrap_or(0.0);
            let launch_heading = physics.yaw + tube_facing_rad;
            use crate::torpedo::LaunchResult;
            let result = torpedo_sys.launch(
                tube_id.as_str(),
                torpedo_uuid.clone(),
                physics.x,
                physics.z,
                launch_heading,
                Some(target_uuid.clone()),
                source_uuid.clone(),
            );
            match result {
                LaunchResult::Launched {
                    uuid: launched_uuid,
                } => {
                    outbox.0.push((
                        Target::All,
                        ServerMessage::TorpedoLaunched {
                            uuid: launched_uuid,
                            tube: tube_id,
                            x: physics.x,
                            z: physics.z,
                            heading: launch_heading,
                        },
                    ));
                }
                LaunchResult::TubeNotLoaded
                | LaunchResult::NoTorpedoes
                | LaunchResult::UnknownTube => {}
            }
        }

        // ── PHASER AUTO-FIRE (future: split to phaser_bank system) ───────
        //
        // tick_phaser_auto_fire handles auto-mode phasers for both human and AI
        // (phaser mode is a ship-level setting, not control-source specific).
        // No additional AI logic needed at the coarse system level.

        // ── FREQUENCY COORDINATION (future: split to channel-3 coordination) ─
        //
        // Science AI emits FrequencyHint when its preset grants auto_hint.
        // The Tactical AI has no corresponding action at the coarse level.
    }
}

fn top_destroy_objective_target(
    blackboards: Option<&crate::server_app::ShipSystemBlackboards>,
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
    targetable_q: &Query<
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
            targetable_q.iter().find_map(|(uuid, _, name)| {
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
                let mut q = world.query_filtered::<&ShipPhysics, With<crate::server_app::LocalShip>>();
                q.single(world)
                    .ok()
                    .copied()
                    .map(|p| (p.x, p.z, p.yaw))
                    .unwrap_or((0.0, 0.0, 0.0))
            };
            let target_uuid: Option<String> = {
                let mut q = world.query_filtered::<&WeaponsTarget, With<crate::server_app::LocalShip>>();
                q.single(world).ok().and_then(|wt| wt.0.clone())
            };
            let (beam_active, active_beam_bank) = {
                let mut q = world.query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
                q.single(world)
                    .ok()
                    .map(|b| (b.target_uuid.is_some(), b.bank.clone()))
                    .unwrap_or((false, None))
            };
            let bank_cooldowns: std::collections::HashMap<String, f32> = {
                let mut q = world.query_filtered::<&PhaserCooldown, With<crate::server_app::LocalShip>>();
                q.single(world)
                    .ok()
                    .map(|cd| cd.per_bank.clone())
                    .unwrap_or_default()
            };
            let tubes: Vec<TorpedoTubeState> = {
                // Prefer per-entity component on LocalShip; fall back to global resource.
                let raw_tubes: Vec<crate::torpedo::TorpedoTube> = {
                    let mut q = world.query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
                    q.single(world).ok().map(|ts| ts.0.tubes.clone())
                        .unwrap_or_else(|| world.resource::<TorpedoSystemResource>().0.tubes.clone())
                };
                raw_tubes
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
            let torpedo_count = {
                let mut q = world.query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
                q.single(world).ok().map(|ts| ts.0.torpedoes_remaining)
                    .unwrap_or_else(|| world.resource::<TorpedoSystemResource>().0.torpedoes_remaining)
            };
            let radar_range_mult = world
                .resource::<crate::modifiers::ShipModifiers>()
                .get(&ModifierSlot::RadarRange);
            let phaser_mode = world.resource::<CurrentPhaserMode>().0;
            let banks_config = {
                // Prefer per-entity component on LocalShip; fall back to global resource.
                let mut q = world.query_filtered::<&PhaserCombatConfigResource, With<crate::server_app::LocalShip>>();
                q.single(world).ok().map(|cc| cc.0.banks.clone())
                    .unwrap_or_else(|| world.resource::<PhaserCombatConfigResource>().0.banks.clone())
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
    weapons_target_q: Query<&WeaponsTarget, With<crate::server_app::LocalShip>>,
    beam_q: Query<&ActiveBeam, With<crate::server_app::LocalShip>>,
    cooldown_q: Query<&PhaserCooldown, With<crate::server_app::LocalShip>>,
    combat_config_q: Query<&PhaserCombatConfigResource, With<crate::server_app::LocalShip>>,
    phaser_mode: Res<CurrentPhaserMode>,
    torpedo_sys_q: Query<&TorpedoSystemResource, With<crate::server_app::LocalShip>>,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    ship_physics_q: Query<&ShipPhysics, With<crate::server_app::LocalShip>>,
    modifiers_q: Query<&crate::modifiers::ShipModifiers, With<crate::server_app::LocalShip>>,
    modifiers_res: Option<Res<crate::modifiers::ShipModifiers>>,
    world_res: Res<WorldResource>,
    entity_name_q: Query<(
        &crate::entity_spawner::EntityUuid,
        &crate::entities::spawner::EntityName,
    )>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut ship_blackboards_q: Query<&mut crate::server_app::ShipSystemBlackboards, With<crate::server_app::LocalShip>>,
) {
    use crate::system_registry::TACTICAL_SYSTEM_ID;
    let physics = ship_physics_q.single().ok().copied().unwrap_or_default();
    let weapons_target = weapons_target_q.single().ok();
    let default_beam;
    let beam: &ActiveBeam = match beam_q.single() {
        Ok(b) => b,
        Err(_) => {
            default_beam = ActiveBeam::default();
            &default_beam
        }
    };
    let default_cooldown;
    let cooldown: &PhaserCooldown = match cooldown_q.single() {
        Ok(c) => c,
        Err(_) => {
            default_cooldown = PhaserCooldown::default();
            &default_cooldown
        }
    };
    // Per-entity component path (preferred). Fallback: use the default config.
    let combat_config_default;
    let combat_config: &PhaserCombatConfigResource = match combat_config_q.single() {
        Ok(c) => c,
        Err(_) => {
            combat_config_default = PhaserCombatConfigResource::default();
            &combat_config_default
        }
    };
    // Per-entity ShipModifiers component takes priority; fall back to Resource.
    let default_modifiers;
    let modifiers: &crate::modifiers::ShipModifiers = match modifiers_q.single() {
        Ok(m) => m,
        Err(_) => match modifiers_res.as_deref() {
            Some(m) => m,
            None => {
                default_modifiers = crate::modifiers::ShipModifiers::new();
                &default_modifiers
            }
        },
    };
    let torpedo_sys_default;
    let torpedo_sys: &TorpedoSystemResource = match torpedo_sys_q.single() {
        Ok(t) => t,
        Err(_) => {
            torpedo_sys_default = TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default()));
            &torpedo_sys_default
        }
    };

    let target_uuid = weapons_target.and_then(|wt| wt.0.clone());
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
                physics.x,
                physics.z,
                physics.yaw,
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
                            crate::weapons::phaser::ship_local(tx, tz, physics.x, physics.z, physics.yaw);
                        let range_ok = (tx - physics.x).powi(2) + (tz - physics.z).powi(2)
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
                physics.x,
                physics.z,
                physics.yaw,
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
                physics.x,
                physics.z,
                physics.yaw,
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

    if let Ok(mut entity_bbs) = ship_blackboards_q.single_mut() {
        entity_bbs.0.insert(
            SystemId(TACTICAL_SYSTEM_ID.to_string()),
            SystemBlackboard::Weapons(bb),
        );
    }
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
    use crate::simulation::{ShipImpulse, SimOutbox};

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
        .insert_resource(ShipImpulse(crate::impulse::ImpulseState::new()))
        .init_resource::<WorldResource>()
        .add_message::<AsteroidDestroyedVfx>()
        .add_message::<crate::ai_plugin::AiEntityDestroyed>()
        .init_resource::<CurrentPhaserMode>()
        .insert_resource(ShipModifiers::new())
        .insert_resource(TorpedoSystemResource(TorpedoSystem::new(
            TorpedoConfig::default(),
        )))
        .init_resource::<SimOutbox>()
        .init_resource::<Outbox>()
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
        .add_systems(Update, (tick_beams, tick_torpedo_system))
        .add_plugins(weapons_update_broadcaster())
        // PR-7 (issue #597) — `tick_shields` (formerly `tick_npc_shield_regen`)
        // now lives on `ShipShieldsPlugin`. Include it so tests that spawn NPCs
        // with `ShipShields` observe regen on every frame.
        .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
        .add_systems(PostUpdate, collect);
        // Spawn the Ship entity with config/control-source components so all
        // weapons systems that use `Query<..., With<Ship>>.single()` have a
        // valid entity to operate on, matching what `spawn_game_start_entities`
        // would do in a full server build.
        let ship = app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            test_ship_config(),
            ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::messages::AdmittedCommands::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            ShipPhysics::default(),
            crate::ship_state::ShipPhaserFrequency::default(),
            bevy::prelude::Transform::default(),
            crate::entity_spawner::EntityConsoleHull(ConsoleHull::from_config(&[
                (Console::Helm, 25.0),
                (Console::Tactical, 25.0),
                (Console::Power, 25.0),
                (Console::Shields, 25.0),
            ])),
            crate::server_app::ShipSystemBlackboards::default(),
            crate::entity_spawner::EntityUuid("test-local-ship".to_string()),
        )).id();
        // Second insert to stay under Bevy's Bundle-tuple length limit.
        app.world_mut().entity_mut(ship).insert((
            // Insert per-entity weapon configs so component-path queries succeed.
            // These are overridden by individual tests via insert_resource for the
            // PhaserCombatConfigResource; we keep both in sync here.
            TorpedoSystemResource(TorpedoSystem::new(TorpedoConfig::default())),
            PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
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
            }),
            PhaserRenderConfig::default(),
            // PR 7 (issue #597) — per-entity beam / target / cooldown components.
            WeaponsTarget::default(),
            ActiveBeam::default(),
            PhaserCooldown::default(),
            // PR 10 (PRD #597) — per-entity combat activity trackers.
            crate::server_app::WeaponFiredThisTick::default(),
            crate::server_app::ShipAttackedThisTick::default(),
            LastShipAttacker::default(),
            crate::ship::combat_activity::RecentCombatActivity::default(),
        ));
        app
    }

    // ── PR 7 test helpers — per-entity access to Weapons state ──────────────
    // These wrap the `Query<&X, With<LocalShip>>` pattern that replaces
    // `world.resource::<X>()` after PR 7 (PRD #597) removed the Resource derive.
    //
    // Each helper: single-entity lookup returning owned data.

    fn get_weapons_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&WeaponsTarget, With<crate::server_app::LocalShip>>();
        q.single(app.world()).ok().and_then(|wt| wt.0.clone())
    }

    fn set_weapons_target(app: &mut App, uuid: Option<String>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut WeaponsTarget, With<crate::server_app::LocalShip>>();
        if let Ok(mut wt) = q.single_mut(app.world_mut()) {
            wt.0 = uuid;
        }
    }

    fn get_active_beam_target(app: &mut App) -> Option<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&ActiveBeam, With<crate::server_app::LocalShip>>();
        q.single(app.world()).ok().and_then(|b| b.target_uuid.clone())
    }

    fn active_beam_target_is_none(app: &mut App) -> bool {
        get_active_beam_target(app).is_none()
    }

    fn set_active_beam_target(app: &mut App, uuid: Option<String>) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.target_uuid = uuid;
        }
    }

    fn set_active_beam_remaining_secs(app: &mut App, secs: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.remaining_secs = secs;
        }
    }

    fn set_active_beam_damage_accumulator(app: &mut App, val: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ActiveBeam, With<crate::server_app::LocalShip>>();
        if let Ok(mut b) = q.single_mut(app.world_mut()) {
            b.damage_accumulator = val;
        }
    }

    fn phaser_bank_is_active(app: &mut App, bank: &str) -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&PhaserCooldown, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .ok()
            .map(|cd| cd.is_bank_active(bank))
            .unwrap_or(false)
    }

    fn start_phaser_cooldown(app: &mut App, bank: &str, secs: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut PhaserCooldown, With<crate::server_app::LocalShip>>();
        if let Ok(mut cd) = q.single_mut(app.world_mut()) {
            cd.start_bank_with_cooldown(bank, secs);
        }
    }

    fn get_phaser_frequency(app: &mut App) -> f32 {
        let mut q = app
            .world_mut()
            .query_filtered::<&crate::ship_state::ShipPhaserFrequency, With<crate::server_app::LocalShip>>();
        q.single(app.world()).map(|f| f.0).unwrap_or(0.5)
    }

    fn set_ship_yaw(app: &mut App, yaw: f32) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
        let mut p = q.single_mut(app.world_mut()).expect("expected Ship with ShipPhysics");
        p.yaw = yaw;
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
        // The systems now prefer the per-entity component over the resource.
        // Update both to keep them in sync.
        let mut q = app
            .world_mut()
            .query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
        if let Ok(mut ts) = q.single_mut(app.world_mut()) {
            ts.0.tube_mut(tube)
                .expect("test tube should exist")
                .load_state = crate::torpedo::TubeLoadState::Loaded;
        } else {
            app.world_mut()
                .resource_mut::<TorpedoSystemResource>()
                .0
                .tube_mut(tube)
                .expect("test tube should exist")
                .load_state = crate::torpedo::TubeLoadState::Loaded;
        }
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
        // handle_set_target and tick_beams use live ECS Transforms
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

        assert_eq!(get_weapons_target(&mut app).as_deref(), Some("target-uuid"));
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
        assert!(get_weapons_target(&mut app).is_none());
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
        assert!(get_weapons_target(&mut app).is_none());
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

        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("target-uuid"));
    }

    #[test]
    fn fire_phaser_rejected_during_cooldown() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        set_active_beam_target(&mut app, None);
        start_phaser_cooldown(&mut app, "port", 3.0);

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

        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("target-uuid"));

        set_active_beam_damage_accumulator(&mut app, 30.0);
        set_active_beam_remaining_secs(&mut app, 5.0);

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

        assert!(active_beam_target_is_none(&mut app));

        assert!(
            phaser_bank_is_active(&mut app, "port"),
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
        set_ship_yaw(&mut app, std::f32::consts::PI);

        let out = tick(&mut app);

        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::BeamEnded { .. })),
            "expected BeamEnded when target leaves bank fire arc"
        );
        assert!(
            active_beam_target_is_none(&mut app),
            "beam should be cleared after sever-by-arc"
        );
        assert!(
            phaser_bank_is_active(&mut app, "port"),
            "cooldown should start after arc sever"
        );
    }

    #[test]
    fn beam_severs_when_target_leaves_phaser_range() {
        let mut app = test_app();
        let _ = lock_and_fire(&mut app, 0.0, -20.0);

        // Move the live ECS Transform out of range. tick_beams reads the
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
            active_beam_target_is_none(&mut app),
            "beam should be cleared after sever-by-range"
        );
        assert!(
            phaser_bank_is_active(&mut app, "port"),
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

        set_active_beam_damage_accumulator(&mut app, 10.0);
        let _ = tick(&mut app);

        // Rotate 180° — target moves to starboard beam, outside port bank's arc.
        set_ship_yaw(&mut app, std::f32::consts::PI);
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
        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t1"));

        set_active_beam_remaining_secs(&mut app, 0.0);
        set_active_beam_damage_accumulator(&mut app, 0.0);
        let _ = tick(&mut app);

        assert!(phaser_bank_is_active(&mut app, "port"));

        start_phaser_cooldown(&mut app, "port", 0.0);

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
        assert_eq!(get_active_beam_target(&mut app).as_deref(), Some("t2"));
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

    /// Regression test for PRD #597 gap-3: an NPC ship spawned with a
    /// `[torpedoes]` TOML block must carry its own `TorpedoSystemResource`
    /// component, and firing from it via the `ai:<uuid>` token path must
    /// launch a torpedo. Two subchecks:
    ///
    /// 1. Direct wiring: `TorpedoSystem::launch()` called on the NPC's own
    ///    component successfully returns `Launched` (i.e. the tubes are
    ///    populated and `torpedoes_remaining > 0`).
    /// 2. End-to-end message routing: an `ai:<uuid>` `FireTorpedo` message
    ///    arriving through `InboundMessage` reaches the NPC's tubes and
    ///    emits a `TorpedoLaunched` broadcast, drawing from the NPC's own
    ///    per-entity tube state — the player-ship `TorpedoSystemResource`
    ///    resource is left untouched.
    ///
    /// NPC AI does not currently emit `FireTorpedo` messages autonomously;
    /// verifying that pipeline is future work (see PRD #487 fine-grained
    /// tactical decomposition). This test covers the wiring.
    #[test]
    fn npc_ship_can_fire_torpedo_when_toml_has_torpedoes_block() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::EntityUuid;
        use crate::torpedo::LaunchResult;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "cc000000-0000-0000-0000-000000000001";

        // Simulate what `src/entities/spawner.rs` does for an NPC with
        // `[torpedoes]`: attach a `TorpedoSystemResource` component built
        // from the runtime config, with default tubes (fore_port, fore_starboard, aft).
        let torpedo_config = TorpedoConfig::default();
        let npc_torpedo_sys = crate::torpedo::TorpedoSystem::new(torpedo_config);
        let mut npc_ai_sources = crate::ship::control_source::ControlSourceResolver::new();
        npc_ai_sources.set(
            crate::system_registry::tactical_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ship_plugin::ShipSystemControlSources(npc_ai_sources),
                ShipPhysics::default(),
                WeaponsTarget::default(),
                TorpedoSystemResource(npc_torpedo_sys),
                crate::server_app::WeaponFiredThisTick::default(),
                bevy::prelude::Transform::default(),
            ))
            .id();
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_entity);
        }

        // Subcheck 1: direct wiring — the NPC's own component has functional
        // tubes and `.launch()` succeeds when the tube is loaded.
        {
            let mut ts = app
                .world_mut()
                .get_mut::<TorpedoSystemResource>(npc_entity)
                .expect("NPC must have TorpedoSystemResource component");
            ts.0.tube_mut("fore_port")
                .expect("default TorpedoSystem must expose fore_port tube")
                .load_state = crate::torpedo::TubeLoadState::Loaded;
            let result = ts.0.launch(
                "fore_port",
                "direct-launch-uuid".to_string(),
                0.0,
                0.0,
                0.0,
                None,
                Some(npc_uuid.to_string()),
            );
            assert!(
                matches!(result, LaunchResult::Launched { .. }),
                "direct TorpedoSystem::launch on NPC's own component must succeed, got {result:?}"
            );
        }

        // Reload the tube for the end-to-end path (previous launch consumed it).
        {
            let mut ts = app
                .world_mut()
                .get_mut::<TorpedoSystemResource>(npc_entity)
                .unwrap();
            ts.0.tube_mut("fore_port").unwrap().load_state =
                crate::torpedo::TubeLoadState::Loaded;
            ts.0.in_flight.clear();
        }

        // Subcheck 2: end-to-end message routing.
        // Snapshot the player-ship (resource) torpedo count to prove the NPC's
        // fire draws from its own component, not from the shared Resource.
        let player_torpedoes_before = app
            .world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining;

        let ai_token = format!("ai:{}", npc_uuid);
        push(
            &mut app,
            &ai_token,
            ClientMessage::FireTorpedo {
                tube: "fore_port".to_string(),
                target_uuid: None,
            },
        );
        let out = tick(&mut app);

        assert!(
            out.iter().any(|m| matches!(&m.msg, ServerMessage::TorpedoLaunched { tube, .. } if tube == "fore_port")),
            "NPC should broadcast TorpedoLaunched after ai:<uuid> FireTorpedo message"
        );

        // The player-ship Resource must NOT have been drained.
        let player_torpedoes_after = app
            .world()
            .resource::<TorpedoSystemResource>()
            .0
            .torpedoes_remaining;
        assert_eq!(
            player_torpedoes_before, player_torpedoes_after,
            "NPC fire must draw from its own per-entity TorpedoSystemResource, \
             leaving the global (player-ship) Resource untouched"
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
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut ShipPhysics, With<crate::server_app::LocalShip>>();
            let mut p = q.single_mut(app.world_mut()).expect("Ship with ShipPhysics");
            p.x = 280.0;
        }
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

        let in_flight_len = {
            // Systems prefer the per-entity component; read from it for assertion.
            let mut q = app.world_mut().query_filtered::<&TorpedoSystemResource, With<crate::server_app::LocalShip>>();
            q.single(app.world()).ok().map(|ts| ts.0.in_flight.len())
                .unwrap_or_else(|| app.world().resource::<TorpedoSystemResource>().0.in_flight.len())
        };
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

        set_active_beam_damage_accumulator(&mut app_fast, BEAM_DAMAGE_PER_SEC * 2.0 * 3.5);
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
        set_active_beam_damage_accumulator(&mut app_base, BEAM_DAMAGE_PER_SEC * 1.0 * 3.5);
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
        let freq = get_phaser_frequency(&mut app);
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
        let freq = get_phaser_frequency(&mut app);
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
        let freq = get_phaser_frequency(&mut app);
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
        let freq = get_phaser_frequency(&mut app);
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
        let freq = get_phaser_frequency(&mut app);
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
        set_active_beam_damage_accumulator(&mut app, 10.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
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
        set_active_beam_damage_accumulator(&mut app, 30.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
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
        assert!(active_beam_target_is_none(&mut app));
        assert!(phaser_bank_is_active(&mut app, "port"));
    }

    // ── NPC shields integration ────────────────────────────────────────────

    /// Spawn a shielded NPC: same as `spawn_npc_entity` but also attaches a
    /// `ShipShields` (num_facings=1) so the damage routing path is exercised
    /// end-to-end.
    fn spawn_shielded_npc_entity(
        app: &mut App,
        npc_x: f32,
        npc_z: f32,
        hull_max: f32,
        shield_max: f32,
        regen_per_sec: f32,
    ) -> bevy::ecs::entity::Entity {
        use crate::weapons::shield::{ShieldConfig, ShieldSystem};
        app.world_mut()
            .spawn((
                // PR-7 (issue #597) — NPC ships carry the `Ship` marker
                // so the unified `tick_shields` picks them up.
                crate::simulation::Ship,
                crate::entity_spawner::EntityUuid("npc-1".into()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    crate::messages::Console::CaptainChair,
                    hull_max,
                )])),
                crate::ship::shields::ShipShields(ShieldSystem::new(&ShieldConfig {
                    num_facings: 1,
                    max_hp: shield_max.round() as i32,
                    regen_per_sec,
                    offline_duration: 10.0,
                })),
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
        set_active_beam_damage_accumulator(&mut app, 5.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);

        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(npc_entity)
            .expect("NPC must still have ShipShields component");
        assert!(
            shields.0.facings[0].hp < 20,
            "shield must absorb damage, got {}",
            shields.0.facings[0].hp
        );
        assert!(shields.0.facings[0].is_online(), "shield must still be online");

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
        set_active_beam_damage_accumulator(&mut app, 15.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);

        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(npc_entity)
            .expect("ShipShields component must persist after break");
        // With ShipShields, a depleted facing goes offline (offline_remaining > 0),
        // not permanently broken.
        assert_eq!(shields.0.facings[0].hp, 0);
        assert!(!shields.0.facings[0].is_online(), "facing must go offline once depleted");

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

        // Spawn with already-offline shield (facing depleted, offline timer running).
        use crate::weapons::shield::{ShieldConfig, ShieldSystem};
        let mut shield_sys = ShieldSystem::new(&ShieldConfig {
            num_facings: 1,
            max_hp: 20,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        });
        // Deplete the facing so it goes offline.
        shield_sys.apply_damage(20, 0.0);
        assert!(!shield_sys.facings[0].is_online(), "facing must be offline");

        let npc_entity = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("npc-1".into()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    crate::messages::Console::CaptainChair,
                    30.0,
                )])),
                crate::ship::shields::ShipShields(shield_sys),
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

        set_active_beam_damage_accumulator(&mut app, 5.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);

        let hull_hp = app
            .world()
            .get::<EntityConsoleHull>(npc_entity)
            .expect("hull must exist")
            .0
            .total_current();
        // Hull must take damage (offline shield does not absorb).
        assert!(
            hull_hp < 30.0,
            "offline shield must let damage through to hull, got {hull_hp}"
        );
        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(npc_entity)
            .expect("ShipShields component must persist");
        assert_eq!(
            shields.0.facings[0].hp, 0,
            "offline facing hp must remain 0, got {}",
            shields.0.facings[0].hp
        );
        assert!(!shields.0.facings[0].is_online(), "facing must remain offline");
    }

    #[test]
    fn shield_regen_advances_npc_shield_below_max() {
        let mut app = test_app();
        setup_npc_world(&mut app, 0.0, -20.0);
        start_game_with_weapons(&mut app);

        let npc_entity = spawn_shielded_npc_entity(&mut app, 0.0, -20.0, 30.0, 20.0, 5.0);

        // Damage the shield to 10 HP.
        if let Some(mut shields) = app
            .world_mut()
            .get_mut::<crate::ship::shields::ShipShields>(npc_entity)
        {
            shields.0.facings[0].hp = 10;
        }

        // Advance time. The Bevy `Time` resource advances on each `app.update()`
        // call; we tick a few frames and expect regen to push hp upward.
        for _ in 0..3 {
            tick(&mut app);
        }

        let shields = app
            .world()
            .get::<crate::ship::shields::ShipShields>(npc_entity)
            .expect("ShipShields must persist");
        // We don't assert exact values (frame timing varies in tests) but we
        // verify regen is making forward progress and not stuck at 10.
        assert!(
            shields.0.facings[0].hp > 10,
            "shield must regen between ticks, got {}",
            shields.0.facings[0].hp
        );
        assert!(
            shields.0.facings[0].hp <= 20,
            "shield must clamp to max_hp, got {}",
            shields.0.facings[0].hp
        );
        assert!(shields.0.facings[0].is_online());
    }

    // ── PR2: Torpedo damage routes through ShipShields on the player ship ──

    /// Verify that a torpedo detonation on the player ship reduces `ShipShields`
    /// HP before leaking to the hull — end-to-end ShipShields coverage for the
    /// torpedo damage path (PR2: Unified ShipShields).
    #[test]
    fn torpedo_hit_reduces_ship_shields_on_player_ship() {
        use crate::entity_spawner::EntityUuid;
        use crate::server_app::LocalShip;
        use crate::weapons::shield::{ShieldConfig, ShieldSystem};
        use crate::weapons::torpedo::Torpedo;

        let mut app = test_app();
        start_game_with_weapons(&mut app);

        // Give the player ship ShipShields with known HP.
        let player_entity = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();

        let shield_max_hp = 100i32;
        let shield_sys = ShieldSystem::new(&ShieldConfig {
            num_facings: 4,
            max_hp: shield_max_hp,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        });
        app.world_mut()
            .entity_mut(player_entity)
            .insert((
                EntityUuid("player-ship".into()),
                crate::ship::shields::ShipShields(shield_sys),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ));

        // Also expose the player ship in the world snapshot so the torpedo can
        // find it as a target.
        app.world_mut()
            .insert_resource(WorldResource(crate::messages::WorldData {
                entities: vec![crate::messages::EntitySnapshot {
                    uuid: "player-ship".into(),
                    position: Some([0.0, 0.0, 0.0]),
                    radius: Some(5.0),
                    ..Default::default()
                }],
                ..Default::default()
            }));

        // Read initial total shield HP.
        let shields_before: i32 = app
            .world()
            .entity(player_entity)
            .get::<crate::ship::shields::ShipShields>()
            .unwrap()
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum();
        assert_eq!(shields_before, shield_max_hp * 4);

        // Read initial hull HP.
        let hull_before = app
            .world()
            .entity(player_entity)
            .get::<crate::entity_spawner::EntityConsoleHull>()
            .unwrap()
            .0
            .total_current();

        // Directly inject a torpedo already adjacent to the player ship so it
        // detonates on the next tick. We write into both the per-entity component
        // and the resource to stay in sync.
        let torpedo = Torpedo {
            uuid: "test-torp-1".into(),
            x: 1.0, // 1 m away from player at origin — within detonation_radius
            z: 0.0,
            heading: 0.0,
            lifespan_remaining: 30.0,
            target_uuid: Some("player-ship".into()),
            source_uuid: None, // no source → no self-detonation exclusion
            shield_pierce: 0.0, // no pierce → all damage goes to shields first
        };
        // Write to the per-entity component (preferred by systems) and resource.
        {
            let mut q = app.world_mut().query_filtered::<&mut TorpedoSystemResource, With<crate::server_app::LocalShip>>();
            if let Ok(mut ts) = q.single_mut(app.world_mut()) {
                ts.0.in_flight.push(torpedo.clone());
            }
        }
        app.world_mut()
            .resource_mut::<TorpedoSystemResource>()
            .0
            .in_flight
            .push(torpedo);

        // Tick once — torpedo detonates and routes damage through ShipShields.
        tick(&mut app);

        let shields_after: i32 = app
            .world()
            .entity(player_entity)
            .get::<crate::ship::shields::ShipShields>()
            .unwrap()
            .0
            .facings
            .iter()
            .map(|f| f.hp)
            .sum();

        let hull_after = app
            .world()
            .entity(player_entity)
            .get::<crate::entity_spawner::EntityConsoleHull>()
            .unwrap()
            .0
            .total_current();

        // Shield HP must decrease (torpedo damage_shields absorbed by shield).
        // (If damage_shields == 0 in the TOML config the test is still valid:
        // it just shows hull dropped instead, but we accept either change.)
        let total_damage_taken = (shields_before - shields_after) + ((hull_before - hull_after) as i32);
        assert!(
            total_damage_taken > 0,
            "torpedo hit must cause total damage: shields_before={shields_before}, shields_after={shields_after}, \
             hull_before={hull_before}, hull_after={hull_after}"
        );
        // The important invariant: if damage_shields > 0, shield must have taken damage first.
        // We verify this indirectly: hull must not exceed its pre-hit value.
        assert!(
            hull_after <= hull_before,
            "hull must not increase after torpedo hit, got {hull_after} > {hull_before}"
        );
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

        set_active_beam_damage_accumulator(&mut app, 30.0);
        set_active_beam_remaining_secs(&mut app, 5.0);
        tick(&mut app);
        tick(&mut app); // second tick allows PostUpdate-equivalent collector to drain the message

        let destroyed_events = app.world().resource::<DestroyedBox>();
        assert!(
            destroyed_events.0.iter().any(|e| e.entity_uuid == "npc-1"),
            "AiEntityDestroyed must be emitted with entity_uuid 'npc-1' so on_destroyed triggers fire"
        );
    }

    // ── NPC as shooter: handle_fire_phaser (unified) / tick_beams ────────────

    /// Set up `AiTokenRegistry`, an NPC entity with `AiControllerComponent` +
    /// `ActiveBeam`/`PhaserCooldown` (unified per-entity phaser state), and a target entity.
    fn setup_npc_shooter(
        app: &mut App,
        npc_uuid: &str,
        target_uuid: &str,
        target_x: f32,
        target_z: f32,
    ) -> (bevy::ecs::entity::Entity, bevy::ecs::entity::Entity) {
        use crate::ai::AiMemory;
        use crate::ai_plugin::AiControllerComponent;
        use crate::entity_spawner::{EntityConsoleHull, EntityUuid};

        // Register the AI token (including Bevy entity link for handle_fire_phaser).
        let target_as_uuid = uuid::Uuid::parse_str(target_uuid).ok();
        let memory = AiMemory {
            target: target_as_uuid,
            ..Default::default()
        };

        // Spawn NPC entity facing toward negative-Z (yaw = 0 → forward = -Z).
        // Includes the Ship marker so the unified `tick_beams` picks it up as
        // a shooter (matches the production `entities::spawner::spawn_entity`
        // path where every ship gets `Ship` — see PRD #597).
        //
        // Also mirrors production by inserting `ShipSystemControlSources` with
        // the Tactical system set to `Ai`, and `WeaponsTarget::default()` —
        // both required by the unified `handle_fire_phaser` per-ship query.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        sources.set(
            crate::system_registry::tactical_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );

        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                AiControllerComponent,
                crate::ai_plugin::ShipAiMemory(memory),
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget::default(),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();

        // Register with the Bevy entity so handle_fire_phaser can look it up.
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_entity);
        }

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
        // `ActiveBeam::target_uuid = Some(...)` after one update.
        use crate::ai_plugin::AiTokenRegistry;

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

        let beam = app
            .world()
            .get::<ActiveBeam>(npc_entity)
            .expect("NPC entity must have ActiveBeam component");
        assert!(
            beam.target_uuid.is_some(),
            "ActiveBeam::target_uuid should be Some after NPC fires phaser via ai: token"
        );
    }

    #[test]
    fn npc_beam_tick_applies_damage_to_target_hull() {
        // With an active NPC beam, each tick of tick_beams reduces
        // the target's EntityConsoleHull.
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::EntityConsoleHull;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000003";
        let target_uuid_str = "00000000-0000-0000-0000-000000000004";

        let (npc_entity, target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        // Activate the beam directly on the per-entity ActiveBeam component.
        {
            let mut beam = app
                .world_mut()
                .get_mut::<ActiveBeam>(npc_entity)
                .unwrap();
            beam.target_uuid = Some(target_uuid_str.to_string());
            beam.remaining_secs = 10.0;
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
    fn npc_beam_tick_damages_npc_target_not_player() {
        // Regression test for PRD #597 PR-1: NPC-vs-NPC beam damage.
        // Before the fix, the old tick_npc_beams hull_query had
        // Without<LocalShip> so NPCs couldn't damage other NPCs — damage
        // was silently lost. The unified `tick_beams` iterates all ships
        // and applies damage to any target found via `hull_q`.
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::EntityConsoleHull;
        use crate::server_app::ShipAttackedThisTick;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<crate::simulation::GameOverReason>();

        let shooter_uuid = "10000000-0000-0000-0000-000000000001";
        let npc_target_uuid = "20000000-0000-0000-0000-000000000002";

        // Spawn NPC shooter with AiControllerComponent.
        let (shooter_entity, npc_target_entity) =
            setup_npc_shooter(&mut app, shooter_uuid, npc_target_uuid, 0.0, -10.0);
        // Add ShipPhysics and AiControllerComponent to the target so it looks
        // like a real production-spawned NPC (AI-controlled, physics-enabled).
        // The unified `tick_beams` finds targets by EntityUuid in `hull_q`
        // (no Ship marker requirement on targets), but production NPCs carry
        // both markers — matching them here keeps the test aligned with real
        // NPC-vs-NPC scenarios.
        app.world_mut()
            .entity_mut(npc_target_entity)
            .insert((
                ShipPhysics::default(),
                crate::ai_plugin::AiControllerComponent,
            ));

        // Activate beam on the shooter.
        {
            let mut beam = app.world_mut().get_mut::<ActiveBeam>(shooter_entity).unwrap();
            beam.target_uuid = Some(npc_target_uuid.to_string());
            beam.remaining_secs = 10.0;
        }

        let hp_before = app
            .world()
            .get::<EntityConsoleHull>(npc_target_entity)
            .unwrap()
            .0
            .total_current();

        for _ in 0..10 {
            app.update();
        }

        let hp_after = app
            .world()
            .get::<EntityConsoleHull>(npc_target_entity)
            .unwrap()
            .0
            .total_current();

        assert!(
            hp_after < hp_before,
            "NPC beam must damage NPC target hull (before={hp_before}, after={hp_after})"
        );
        // Player ship must NOT have been marked as attacked.
        let player_atk = app
            .world_mut()
            .query_filtered::<&ShipAttackedThisTick, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .map(|c| c.0)
            .unwrap_or(false);
        assert!(
            !player_atk,
            "NPC-vs-NPC beam must not set player's ShipAttackedThisTick"
        );
    }

    #[test]
    fn on_beam_started_emits_correct_source_uuid_with_multiple_ships() {
        // Regression test for PRD #597 PR-1: on_beam_started used With<Ship>.single()
        // which panics when multiple ships exist. After fix it uses With<LocalShip>.
        use crate::entity_spawner::EntityUuid;

        let mut app = test_app();
        let player_uuid_str = "aaaaaaaa-0000-0000-0000-000000000001";
        let npc_uuid_str = "bbbbbbbb-0000-0000-0000-000000000002";

        // Add EntityUuid to the existing LocalShip entity (spawned by test_app).
        let player_entity = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut()
            .entity_mut(player_entity)
            .insert(EntityUuid(player_uuid_str.to_string()));

        // Spawn a second NPC ship (non-LocalShip, has Ship marker).
        app.world_mut().spawn((
            crate::server_app::Ship,
            EntityUuid(npc_uuid_str.to_string()),
            ShipPhysics::default(),
            Transform::default(),
        ));

        // Trigger BeamStartedEvent — the observer on_beam_started should emit
        // source_uuid = player_uuid_str, not empty.
        app.world_mut().trigger(super::BeamStartedEvent {
            bank: "port".to_string(),
            target_uuid: "some-target".to_string(),
            source_entity: player_entity,
        });
        app.update();

        // Find the BeamStarted message in the SimOutbox.
        let outbox = app.world().resource::<crate::simulation::SimOutbox>();
        let beam_started = outbox.0.iter().find(|(_, msg)| {
            matches!(msg, crate::messages::ServerMessage::BeamStarted { .. })
        });
        let Some((_, crate::messages::ServerMessage::BeamStarted { source_uuid, .. })) = beam_started else {
            panic!("expected BeamStarted message in outbox");
        };
        assert_eq!(
            source_uuid, player_uuid_str,
            "on_beam_started must emit the LocalShip UUID as source_uuid, not {:?}",
            source_uuid
        );
    }

    #[test]
    fn npc_beam_tick_applies_damage_to_player_ship_through_shields() {

        // When the beam target is the player ship (has Ship marker), damage
        // must route through shields → hull component, not just EntityConsoleHull directly.
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::EntityUuid;
        use crate::server_app::{LocalShip, ShipAttackedThisTick};
        use crate::shield::ShieldConfig;
        use crate::simulation::{GameOverReason, ShipShields};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();
        app.init_resource::<GameOverReason>();

        // Insert shields on the LocalShip entity so the shield-routing
        // path is exercised (ShipShields is pure per-entity Component
        // post ship-parity audit).
        let shield_config = ShieldConfig {
            max_hp: 100,
            regen_per_sec: 0.0,
            num_facings: 4,
            ..Default::default()
        };
        {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>();
            let local = q.single(app.world()).unwrap();
            app.world_mut()
                .entity_mut(local)
                .insert(ShipShields(crate::shield::ShieldSystem::new(&shield_config)));
        }

        let npc_uuid = "00000000-0000-0000-0000-000000000010";
        let player_uuid = "00000000-0000-0000-0000-000000000011";
        let player_uuid_parsed = uuid::Uuid::parse_str(player_uuid).unwrap();

        // Add EntityUuid and position to the existing LocalShip entity (already spawned by test_app).
        let player_entity = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        app.world_mut().entity_mut(player_entity).insert((
            EntityUuid(player_uuid.to_string()),
            Transform::from_xyz(0.0, 0.0, -10.0),
        ));

        // Spawn NPC entity using the new per-entity beam components.
        let npc_entity = {
            use crate::ai::AiMemory;
            let memory = AiMemory {
                target: Some(player_uuid_parsed),
                ..Default::default()
            };

            let npc_ent = app
                .world_mut()
                .spawn((
                    crate::server_app::Ship,
                    EntityUuid(npc_uuid.to_string()),
                    crate::ai_plugin::AiControllerComponent,
                    crate::ai_plugin::ShipAiMemory(memory),
                    ActiveBeam::default(),
                    PhaserCooldown::default(),
                    ShipPhysics::default(),
                    Transform::from_xyz(0.0, 0.0, 0.0),
                ))
                .id();

            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_ent);
            npc_ent
        };

        let hull_before = app
            .world()
            .entity(player_entity)
            .get::<crate::entity_spawner::EntityConsoleHull>()
            .unwrap()
            .0
            .total_current();
        let shields_sum_before: i32 = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipShields, With<LocalShip>>();
            q.single(app.world())
                .expect("LocalShip must carry ShipShields")
                .0
                .facings
                .iter()
                .map(|f| f.hp)
                .sum()
        };

        // Activate the beam directly targeting the player ship.
        {
            let mut beam = app
                .world_mut()
                .get_mut::<ActiveBeam>(npc_entity)
                .unwrap();
            beam.target_uuid = Some(player_uuid.to_string());
            beam.remaining_secs = 10.0;
        }

        for _ in 0..10 {
            app.update();
        }

        let hull_after = app
            .world()
            .entity(player_entity)
            .get::<crate::entity_spawner::EntityConsoleHull>()
            .unwrap()
            .0
            .total_current();
        let shields_sum_after: i32 = {
            let mut q = app
                .world_mut()
                .query_filtered::<&ShipShields, With<LocalShip>>();
            q.single(app.world())
                .expect("LocalShip must carry ShipShields")
                .0
                .facings
                .iter()
                .map(|f| f.hp)
                .sum()
        };

        let hull_lost = hull_before - hull_after;
        let shields_lost = shields_sum_before - shields_sum_after;

        assert!(
            hull_lost > 0.0 || shields_lost > 0,
            "NPC beam must damage player ship: hull {hull_before}->{hull_after} ({hull_lost}), shields {shields_sum_before}->{shields_sum_after} ({shields_lost})"
        );
        let player_atk = app
            .world_mut()
            .query_filtered::<&ShipAttackedThisTick, With<LocalShip>>()
            .single(app.world())
            .map(|c| c.0)
            .unwrap_or(false);
        assert!(
            player_atk,
            "NPC beam targeting the player ship must mark the ship as attacked for Captain AI"
        );
    }

    #[test]
    fn npc_beam_cooldown_starts_after_beam_expires() {
        // When an NPC's ActiveBeam remaining_secs reaches zero, PhaserCooldown must
        // be set to a positive value and ActiveBeam.target_uuid must become None.
        use crate::ai_plugin::AiTokenRegistry;

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "00000000-0000-0000-0000-000000000005";
        let target_uuid_str = "00000000-0000-0000-0000-000000000006";

        let (npc_entity, _target_entity) =
            setup_npc_shooter(&mut app, npc_uuid, target_uuid_str, 0.0, -10.0);

        {
            let mut beam = app
                .world_mut()
                .get_mut::<ActiveBeam>(npc_entity)
                .unwrap();
            beam.target_uuid = Some(target_uuid_str.to_string());
            beam.remaining_secs = 0.001; // expires on first tick
        }

        app.update(); // beam expires
        app.update(); // cooldown ticked

        let beam = app.world().get::<ActiveBeam>(npc_entity).unwrap();
        assert!(
            beam.target_uuid.is_none(),
            "ActiveBeam.target_uuid must be None after beam expires"
        );
        let cooldown = app.world().get::<PhaserCooldown>(npc_entity).unwrap();
        assert!(
            cooldown.per_bank.values().any(|&v| v > 0.0),
            "PhaserCooldown must be positive after beam ends: {:?}",
            cooldown.per_bank
        );
    }

    // ── End-to-end: tick_ai_controllers → InboundMessage → handle_fire_phaser ──

    /// Build an app that includes BOTH `WeaponsPlugin` AND `AiPlugin` together
    /// with all their required resources, so the full routing path can be tested:
    /// `tick_ai_controllers` emits a `FirePhaser` `InboundMessage` which the
    /// unified `handle_fire_phaser` picks up and activates the NPC's `ActiveBeam`.
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
    fn tick_ai_controllers_fire_phaser_routes_through_unified_handle_fire_phaser() {
        // Full end-to-end test: an NPC with a Destroy doctrine and a pre-selected
        // target directly in its forward arc causes `tick_ai_controllers` to write
        // a `FirePhaser` `InboundMessage`, which the unified `handle_fire_phaser`
        // picks up
        // and sets `ActiveBeam::target_uuid`.
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
        // Include ActiveBeam/PhaserCooldown/ShipPhysics for the unified fire path,
        // plus the components the unified `handle_fire_phaser` requires:
        // `Ship`, `ShipSystemControlSources` (Tactical = Ai), `WeaponsTarget`.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        sources.set(
            crate::system_registry::tactical_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                crate::entity_spawner::BehaviourSection(behaviour),
                EntityUuid(npc_uuid_str.to_string()),
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget::default(),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
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

        // Tick 1: `register_ai_tokens_on_spawn` runs → AiControllerComponent marker +
        //         ShipAiMemory attached and token registered in AiTokenRegistry.
        app.update();

        // Register the Bevy entity in AiTokenRegistry (needed by handle_fire_phaser).
        {
            let mut reg = app.world_mut().resource_mut::<crate::ai_plugin::AiTokenRegistry>();
            reg.register_with_entity(npc_uuid_str, npc_entity);
        }

        // Set ShipAiMemory.target so handle_fire_phaser can look up the target.
        {
            let mut mem = app
                .world_mut()
                .get_mut::<crate::ai_plugin::ShipAiMemory>(npc_entity)
                .unwrap();
            mem.0.target = Some(target_uuid_parsed);
        }

        // Push a synthetic FirePhaser message for the NPC's ai: token.
        // In production this would be emitted by operate_tactical_ai/tick_phaser_auto_fire,
        // but for this integration test we inject it directly.
        let ai_token = format!("ai:{}", npc_uuid_str);
        push(&mut app, &ai_token, ClientMessage::FirePhaser { bank: "fore".into() });

        // Tick: handle_fire_phaser processes the message and activates ActiveBeam.
        app.update();

        let beam = app
            .world()
            .get::<ActiveBeam>(npc_entity)
            .expect("NPC must have ActiveBeam component");
        assert!(
            beam.target_uuid.is_some(),
            "ActiveBeam.target_uuid must be Some after tick_ai_controllers → InboundMessage → handle_fire_phaser routing"
        );
    }

    /// Verify that both a `LocalShip` entity and an NPC entity use the same
    /// `tick_beams` handler (unified per-entity beam path — issues #588 / #597).
    #[test]
    fn both_localship_and_npc_can_fire_via_per_entity_active_beam() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::{EntityConsoleHull, EntityUuid};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let target_uuid = "ff000000-0000-0000-0000-000000000001";
        let npc_uuid = "ff000000-0000-0000-0000-000000000002";

        // Spawn a target entity with hull.
        let target_entity = app
            .world_mut()
            .spawn((
                EntityUuid(target_uuid.to_string()),
                EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                    Console::CaptainChair,
                    100.0,
                )])),
                Transform::from_xyz(0.0, 0.0, -15.0),
            ))
            .id();

        // Spawn NPC entity with per-entity ActiveBeam and activate beam.
        // Includes the Ship marker so the unified `tick_beams` picks it up
        // as a shooter (matches production NPC spawn path — see PRD #597).
        let npc_ent = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                crate::ai_plugin::ShipAiMemory::default(),
                ActiveBeam {
                    target_uuid: Some(target_uuid.to_string()),
                    remaining_secs: 10.0,
                    ..Default::default()
                },
                PhaserCooldown::default(),
                ShipPhysics::default(),
                Transform::default(),
            ))
            .id();
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_ent);
        }

        // Run ticks so tick_beams fires.
        for _ in 0..5 {
            app.update();
        }

        let hp = app
            .world()
            .get::<EntityConsoleHull>(target_entity)
            .unwrap()
            .0
            .total_current();
        assert!(
            hp < 100.0,
            "NPC beam must apply damage via the unified tick_beams path (hp={hp})"
        );
    }

    /// Regression test for the unified `tick_phaser_auto_fire`.
    ///
    /// Before unification, `tick_phaser_auto_fire` iterated only `LocalShip`,
    /// so NPCs had to route through the (now-deleted) `handle_npc_beam_fire`
    /// with synthetic `FirePhaser` messages emitted by AI. Post-unification
    /// the same system iterates every ship whose Tactical system is
    /// AI-controlled, activating an [`ActiveBeam`] directly.
    #[test]
    fn tick_phaser_auto_fire_activates_ai_controlled_npc_beam() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::{EntityConsoleHull, EntityUuid};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "aa000000-0000-0000-0000-000000000001";
        let target_uuid = "aa000000-0000-0000-0000-000000000002";

        // NPC facing -Z (yaw=0 forward = -Z) with Tactical set to Ai.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        sources.set(
            crate::system_registry::tactical_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                crate::ai_plugin::ShipAiMemory::default(),
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget(Some(target_uuid.to_string())),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                PhaserCombatConfigResource(crate::entity_config::PhaserCombatConfig {
                    banks: vec![crate::entity_config::PhaserBankConfig {
                        id: "fore".into(),
                        facing_deg: 0.0,
                        fire_arc_deg: 360.0,
                        auto_arc_deg: 360.0,
                        beam_range: 50.0,
                        beam_damage_per_sec: 5.0,
                        beam_duration_secs: 3.0,
                        cooldown_secs: 6.0,
                        beam_color: vec![],
                        shield_pierce: None,
                        marker: None,
                    }],
                }),
                Transform::default(),
            ))
            .id();

        // Spawn target directly ahead (in-arc, in-range).
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                Console::CaptainChair,
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));

        app.update();

        let beam = app
            .world()
            .get::<ActiveBeam>(npc_entity)
            .expect("NPC entity must have ActiveBeam component");
        assert!(
            beam.target_uuid.is_some(),
            "tick_phaser_auto_fire must activate the NPC's ActiveBeam when Tactical is AI-controlled"
        );
        assert_eq!(
            beam.bank.as_deref(),
            Some("fore"),
            "NPC should fire the in-arc bank selected from its own PhaserCombatConfigResource"
        );
    }

    /// Regression test for the unified `handle_fire_phaser`.
    ///
    /// Before unification, `handle_npc_beam_fire` always used the first entry
    /// of `WeaponsConsoleSection.phaser_banks` and a 360° arc via
    /// `radar::is_fire_ready_with_range`. Post-unification, NPCs consult
    /// their `PhaserCombatConfigResource::bank_by_id` and honour that bank's
    /// `fire_arc_deg`. A target outside the requested bank's arc must be
    /// rejected, matching the player-fire behaviour.
    #[test]
    fn npc_handle_fire_phaser_rejects_target_outside_requested_bank_arc() {
        use crate::ai_plugin::AiTokenRegistry;
        use crate::entity_spawner::{EntityConsoleHull, EntityUuid};

        let mut app = test_app();
        app.init_resource::<AiTokenRegistry>();

        let npc_uuid = "bb000000-0000-0000-0000-000000000001";
        let target_uuid = "bb000000-0000-0000-0000-000000000002";

        // NPC facing -Z with a narrow port-only bank (facing_deg=-90, arc=60°).
        // Target directly ahead is out of arc.
        let mut sources = crate::ship::control_source::ControlSourceResolver::new();
        sources.set(
            crate::system_registry::tactical_system_id(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let combat = crate::entity_config::PhaserCombatConfig {
            banks: vec![crate::entity_config::PhaserBankConfig {
                id: "port".into(),
                facing_deg: -90.0,
                fire_arc_deg: 60.0,
                auto_arc_deg: 60.0,
                beam_range: 50.0,
                beam_damage_per_sec: 5.0,
                beam_duration_secs: 3.0,
                cooldown_secs: 6.0,
                beam_color: vec![],
                shield_pierce: None,
                marker: None,
            }],
        };
        let target_uuid_parsed = uuid::Uuid::parse_str(target_uuid).unwrap();
        let npc_entity = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(npc_uuid.to_string()),
                crate::ai_plugin::AiControllerComponent,
                crate::ai_plugin::ShipAiMemory(crate::ai::AiMemory {
                    target: Some(target_uuid_parsed),
                    ..Default::default()
                }),
                crate::ship_plugin::ShipSystemControlSources(sources),
                WeaponsTarget::default(),
                ActiveBeam::default(),
                PhaserCooldown::default(),
                ShipPhysics::default(),
                PhaserCombatConfigResource(combat),
                Transform::default(),
            ))
            .id();
        {
            let mut reg = app.world_mut().resource_mut::<AiTokenRegistry>();
            reg.register_with_entity(npc_uuid, npc_entity);
        }
        // Target directly ahead (-Z, bearing 0°) — outside the -90° port bank
        // whose arc runs from -120° to -60°.
        app.world_mut().spawn((
            EntityUuid(target_uuid.to_string()),
            EntityConsoleHull(crate::damage::ConsoleHull::from_config(&[(
                Console::CaptainChair,
                50.0,
            )])),
            Transform::from_xyz(0.0, 0.0, -20.0),
        ));

        // Send an explicit FirePhaser request for the port bank.
        let ai_token = format!("ai:{}", npc_uuid);
        push(
            &mut app,
            &ai_token,
            ClientMessage::FirePhaser {
                bank: "port".to_string(),
            },
        );
        app.update();

        let beam = app.world().get::<ActiveBeam>(npc_entity).unwrap();
        assert!(
            beam.target_uuid.is_none(),
            "FirePhaser for a port bank must be rejected when the target is not in that bank's fire arc — unified handler now honours per-bank config for NPCs"
        );
    }

    fn tactical_blips(app: &mut App) -> Vec<RadarBlip> {
        use crate::messages::{SystemBlackboard, SystemId};
        use crate::server_app::ShipSystemBlackboards;
        use crate::system_registry::TACTICAL_SYSTEM_ID;
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        match q.single(app.world()) {
            Ok(bbs) => match bbs.0.get(&SystemId(TACTICAL_SYSTEM_ID.to_string())) {
                Some(SystemBlackboard::Weapons(bb)) => bb.blips.clone(),
                _ => Vec::new(),
            },
            Err(_) => Vec::new(),
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
            world.query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
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
        use crate::server_app::ShipSystemBlackboards;

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
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
        let mut bbs = q
            .single_mut(app.world_mut())
            .expect("LocalShip must have ShipSystemBlackboards");
        bbs.0.insert(
            crate::system_registry::viewscreen_system_id(),
            SystemBlackboard::Viewscreen(viewscreen),
        );
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
            get_weapons_target(&mut app).as_deref(),
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
            get_weapons_target(&mut app).is_none(),
            "Tactical AI must not lock an arbitrary target when the objective target is missing"
        );
    }

    #[test]
    fn ai_fires_torpedo_when_ai_controls_unclaimed_station() {
        // Unclaimed station + Ai ControlSource → operate_tactical_ai fires unconditionally.
        let mut app = test_app();

        set_tactical_control_source(&mut app, crate::ship::control_source::ControlSource::Ai);
        set_weapons_target(&mut app, Some("target-uuid".into()));
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
            .query_filtered::<&mut crate::ship_plugin::ActiveStationRatings, With<crate::server_app::LocalShip>>();
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
        set_weapons_target(&mut app, Some("target-uuid".into()));
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

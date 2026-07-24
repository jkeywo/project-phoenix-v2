//! Phaser beam weapons extracted from `server.rs` (issue #727): beam types,
//! fire/mode/frequency/target handlers, the three-phase beam tick, the phaser
//! auto-fire decider, and the active-beam power drain. Pure file split — no
//! functional changes; `server.rs` re-exports everything so existing paths
//! keep resolving.

use bevy::prelude::*;
use bevy_rapier3d::prelude::ReadRapierContext;

use crate::entity_spawner::{EntitySystemHull, FactionComponent};
use crate::lobby::{Target, WorldResource};
use crate::messages::{
    AdmittedCommands, GamePhase, InterSystemMsg, InterSystemPayload, InterSystemQueue,
    ModifierSlot, PhaserBank, PhaserMode, ServerMessage, SystemBlackboard, SystemControlPayload,
};
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipPhysics;
use crate::simulation::{AsteroidUuid, GameOverReason, SimOutbox};

use super::shared::{
    any_bank_accepts_human_input, any_bank_operates_ai, live_entity_xz, system_is_registered,
    BeamContext, ShooterState,
};
use super::{AsteroidDestroyedVfx, ShipDestroyedVfx, DEFAULT_SHIP_EXPLOSION_RADIUS};

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

/// The currently locked target UUID on the Weapons console. `None` means no
/// lock is active.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own weapons target.
#[derive(Component, Default, Clone, Debug)]
pub struct TacticalRadarSelection(pub Option<String>);

/// Per-ship resolved Tactical target selector (issue #777).
///
/// The Tactical mirror of [`crate::ship::sensors::SensorsTargetSelector`]:
/// holds the ship's data-driven [`crate::ai::selector::TargetSelector`] —
/// authored `[weapons_console.selector]` or the canonical
/// [`crate::entities::config::default_tactical_target_selector_config`] default
/// — plus the authored ship `power_rating`, which `ai_target_selection` exposes
/// to the selector's expressions as `self_fact(power_rating)`.
///
/// Attached at spawn on every ship (NPC and player) alongside
/// `SensorsTargetSelector`; ships without one (bare-`App` fixtures) fall back to
/// the default selector inside `ai_target_selection`. The selector RANKS
/// candidates; it never writes authoritative state — the host applies the
/// chosen UUID to `TacticalRadarSelection` directly, keeping Tactical the sole
/// writer (AC4).
#[derive(Component, Clone, Debug)]
pub struct TacticalTargetSelector {
    /// The resolved ranking policy.
    pub selector: crate::ai::selector::TargetSelector,
    /// Authored ship power rating, seeded from `EntityConfig.power_rating`.
    pub power_rating: Option<f32>,
}

impl Default for TacticalTargetSelector {
    fn default() -> Self {
        Self {
            selector: crate::entities::config::default_tactical_target_selector_config()
                .to_selector()
                .unwrap_or_default(),
            power_rating: None,
        }
    }
}

/// UUID of the last entity that attacked this ship. Written by the unified
/// `tick_beams` in the Damage phase on the targeted ship's entity;
/// consumed by that ship's `ai_target_selection` as a fallback target.
/// `None` when no recent attacker is known.
///
/// Per-ship `Component` — every ship (player + NPC) tracks its own attacker.
///
/// # Change detection is load-bearing (issue #702)
///
/// This is the single "who last attacked me" surface, and its change detection
/// is the rising-edge latch that fires `AiEntityAttacked` — which in turn drives
/// `on_entity_attacked` scenario triggers. Every writer must therefore
/// **compare before writing** (`set_if_neq`), or sustained fire from one shooter
/// re-fires the trigger every tick the beam is live. `PartialEq` exists for
/// exactly that reason; do not replace it with a blind assignment.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub struct LastShipAttacker(pub Option<String>);

/// Active phaser beam state. `target_uuid` is `Some` while a beam is firing.
/// `remaining_secs` counts down to 0. `damage_accumulator` tracks fractional
/// damage between ticks so 5 HP/s is applied accurately at any frame rate.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own beam state.
///
/// After issue #846, phaser fire commands arrive as admitted `ControlSystem`
/// payloads rather than through `PhaserIntents` — the `#[require]` was deleted
/// alongside the intents component.
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

pub(crate) fn on_beam_started(
    trigger: On<BeamStartedEvent>,
    mut outbox: ResMut<SimOutbox>,
    ship_q: Query<&crate::entity_spawner::EntityUuid>,
    mut weapon_fired_q: Query<&mut crate::server_app::WeaponFiredThisTick>,
    // `Option<Res<_>>`, never bare — observers run in bare-`App` weapons
    // fixtures with no `LogFilterConfig` inserted (see logging macro docs).
    log: Option<Res<crate::logging::LogFilterConfig>>,
    // `Option<ResMut<Messages<_>>>` so bare-`App` fixtures that never
    // registered the message still pass Bevy's parameter validation.
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
) {
    let ev = trigger.event();
    if let Ok(mut wf) = weapon_fired_q.get_mut(ev.source_entity) {
        wf.0 = true;
    }
    // "Opened fire" — an `info` edge scoped to the shooter, event-driven (once
    // per beam), not per-tick.
    crate::pinfo!(
        log,
        crate::logging::LogCat::Weapons,
        entity = ev.source_entity,
        "opened fire (bank {}) at {}",
        ev.bank,
        ev.target_uuid
    );
    let source_uuid = ship_q
        .get(ev.source_entity)
        .map(|u| u.0.clone())
        .unwrap_or_default();
    // Balance tracer: a beam opening is one shot leaving the ship (distinct
    // from the `DamageApplied` each tick it burns). Unconditional — all ships,
    // all builds. Blank uuid → `None`, matching the DamageApplied convention.
    if let Some(ref mut msgs) = balance_events {
        msgs.write(crate::balance::BalanceEvent::WeaponFired {
            shooter: Some(source_uuid.clone()).filter(|u| !u.is_empty()),
            weapon: ev.bank.clone(),
            kind: crate::balance::FIRED_KIND_BEAM.to_string(),
        });
    }
    outbox.0.push((
        Target::All,
        ServerMessage::BeamStarted {
            bank: ev.bank.clone(),
            source_uuid,
            target_uuid: ev.target_uuid.clone(),
        },
    ));
}

pub(crate) fn on_beam_ended(
    trigger: On<BeamEndedEvent>,
    mut outbox: ResMut<SimOutbox>,
    ship_q: Query<&crate::entity_spawner::EntityUuid>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    let ev = trigger.event();
    // "Ceased fire" is the trailing half of the opened-fire edge. Kept at
    // `debug` rather than `info`: a phaser opens and closes once per cooldown
    // cycle, so emitting both ends at `info` would double the headline volume
    // for what a balancer reads as one engagement.
    crate::pdebug!(
        log,
        crate::logging::LogCat::Weapons,
        entity = ev.source_entity,
        "ceased fire (bank {}) at {}",
        ev.bank,
        ev.target_uuid
    );
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

pub(crate) fn handle_set_target(
    ship_query: Query<
        (
            &AdmittedCommands,
            &ShipPhysics,
            &ShipSystemControlSources,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut weapons_target_q: Query<&mut TacticalRadarSelection, With<crate::server_app::LocalShip>>,
    modifiers_q: Query<&crate::modifiers::ShipModifiers, With<crate::server_app::LocalShip>>,
    mut outbox: ResMut<SimOutbox>,
    ship_client_config: Res<crate::lobby::server::ShipClientConfigResource>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    let Some((admitted, physics, control_sources, ship_config)) = ship_query.iter().next() else {
        return;
    };
    let Some(mut weapons_target) = weapons_target_q.iter_mut().next() else {
        return;
    };
    // Ship-level authorisation (issue #512, option c): SetTarget is a ship-wide
    // concern, not per-bank. Accept the message if ANY phaser bank on the ship
    // is currently human-operable — that mirrors the "fire when at least one
    // bank is alive" semantic. If no banks are declared in the ship config
    // (legacy / test ship), fall back to the coarse tactical policy.
    if !any_bank_accepts_human_input(control_sources, &ship_config.0) {
        return;
    }
    let default_modifiers;
    let modifiers: &crate::modifiers::ShipModifiers = match modifiers_q.single() {
        Ok(m) => m,
        Err(_) => {
            default_modifiers = crate::modifiers::ShipModifiers::new();
            &default_modifiers
        }
    };
    for cmd in admitted.for_target(crate::system_registry::TACTICAL_RADAR_SYSTEM_ID) {
        let SystemControlPayload::SetTarget { uuid } = &cmd.payload else {
            continue;
        };

        let radar_range_mult = modifiers.get(&ModifierSlot::RadarRange);
        let base_range = ship_client_config.0.tactical_radar_range;
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

/// Admitted-command consumer for `FirePhaser` (issue #846).
///
/// Reads each ship's own `AdmittedCommands` for `FirePhaser` payloads,
/// resolves the bank from the target `SystemId`, verifies arc/range and
/// cooldowns, then activates the ship's `ActiveBeam` + triggers
/// `BeamStartedEvent`.
///
/// Runs in `SimSet::Physics` — after the AI decider (`ai_phaser_auto_fire`,
/// `SimSet::Input`) has emitted its admitted command, but still within the
/// same tick so the beam starts without a tick of queue lag.
///
/// No `InboundMessage` / token resolution: admission stripped the source
/// identity. No human-vs-AI branch below this point.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_fire_phaser(
    mut commands: Commands,
    mut ship_q: Query<
        (
            Entity,
            &ShipSystemControlSources,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            &mut ActiveBeam,
            &PhaserCooldown,
            &AdmittedCommands,
            Option<&PhaserCombatConfigResource>,
            Option<&crate::modifiers::ShipModifiers>,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    use crate::entity_config::PhaserCombatConfig;

    for (
        ship_entity,
        control_sources,
        physics,
        blackboards,
        mut beam,
        cooldown,
        admitted,
        combat_config_opt,
        modifiers_opt,
    ) in ship_q.iter_mut()
    {
        for cmd in admitted.0.iter() {
            let SystemControlPayload::FirePhaser = &cmd.payload else {
                continue;
            };

            // Resolve the bank id from the target SystemId by running the
            // canonical forward mapping over each authored bank and comparing.
            let combat_config_default = PhaserCombatConfigResource::default();
            let combat_config: &PhaserCombatConfigResource =
                combat_config_opt.unwrap_or(&combat_config_default);
            let Some(bank_id) = (if combat_config.0.banks.is_empty() {
                // No banks in config: try the raw target string as bank id.
                let raw = cmd.target.0.strip_prefix("phaser-").map(|s| s.to_string());
                raw
            } else {
                combat_config.0.banks.iter().find_map(|b| {
                    crate::system_registry::phaser_bank_system_id(&b.id)
                        .filter(|id| id == &cmd.target)
                        .map(|_| b.id.clone())
                })
            }) else {
                continue;
            };

            // Verify the bank is registered and operable.
            let bank_system_id = crate::system_registry::phaser_bank_system_id(&bank_id)
                .filter(|id| system_is_registered(control_sources, id));
            let policy = match &bank_system_id {
                Some(id) => control_sources.0.policy_for(id),
                None => crate::ship::control_source::control_tick_policy(
                    crate::ship::control_source::ControlSource::default(),
                ),
            };
            // Admission already gated the token. This is a system-state gate:
            // the bank must be operable (not Offline).
            if !policy.accept_human_input && !policy.operate_ai {
                continue;
            }

            if cooldown.is_bank_active(&bank_id) || beam.target_uuid.is_some() {
                continue;
            }

            // Target: the combat lock from the ship's viewscreen blackboard.
            let Some(target_uuid) = (match blackboards
                .0
                .get(&crate::system_registry::viewscreen_system_id())
            {
                Some(SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
                _ => None,
            }) else {
                continue;
            };
            let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
                continue;
            };

            let modifiers_default = crate::modifiers::ShipModifiers::new();
            let modifiers: &crate::modifiers::ShipModifiers =
                modifiers_opt.unwrap_or(&modifiers_default);

            let bank_cfg = combat_config.0.bank_by_id(&bank_id);
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
                        let (rx, ry) = crate::weapons::phaser::ship_local(
                            tx,
                            tz,
                            physics.x,
                            physics.z,
                            physics.yaw,
                        );
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

            // Cancel any active beam before starting the new one.
            if let Some(old_uuid) = beam.target_uuid.take() {
                let old_bank = beam.bank.clone().unwrap_or_default();
                beam.remaining_secs = 0.0;
                beam.damage_accumulator = 0.0;
                commands.trigger(BeamEndedEvent {
                    bank: old_bank,
                    target_uuid: old_uuid,
                    source_entity: ship_entity,
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
            beam.bank = Some(bank_id.clone());

            commands.trigger(BeamStartedEvent {
                bank: bank_id,
                target_uuid: target_uuid.clone(),
                source_entity: ship_entity,
            });
        }
    }
}

/// Decide which in-arc phaser bank each ship should fire at its locked target
/// this tick, and emit into the ship's own `AdmittedCommands` (issue #846).
///
/// Retired `PhaserIntents` in favour of the standard `ControlSystem` →
/// `AdmittedCommands` seam: the decision logic is unchanged, but instead of
/// writing a private intent component it now calls `emit_ai_command` to push
/// a `FirePhaser` payload into the ship's admitted set. The paired applier
/// (`handle_fire_phaser`) reads that set in `SimSet::Physics`.
///
/// Iterates every ship (`With<Ship>`) — player + NPC — and auto-fires when
/// either:
/// - at least one phaser bank on the ship has `operate_ai == true` on its own
///   fine-system policy ([`any_bank_operates_ai`]), which holds for NPCs (Ai by
///   default) and for the player ship on Backfill / explicit Ai rating; or
/// - the player toggled [`CurrentPhaserMode`] to `Auto` (weapons-console-only
///   knob that is meaningless for NPC ships, which have no phaser mode).
///
/// # Gating
/// Deliberately **not** filtered on `AiHighFidelity`, unlike
/// `ai_torpedo_auto_fire`. Adding that filter while extracting this system
/// would silently stop every low-LOD NPC firing phasers — a gameplay change,
/// not a refactor. It would also be wrong on its own terms: the
/// `CurrentPhaserMode::Auto` leg is a *human* console toggle on the player's
/// ship, and phaser fire is the primary damage source low-LOD NPCs contribute
/// to a fight. `phaser_auto_fire_runs_for_low_lod_npc_without_ai_high_fidelity`
/// pins this.
///
/// Target selection: reads the ship's `combat_lock` from the viewscreen
/// blackboard — same surface `handle_fire_phaser` reads (issue #829, spec §3).
#[allow(clippy::too_many_arguments)]
pub(crate) fn ai_phaser_auto_fire(
    phaser_mode: Res<CurrentPhaserMode>,
    sessions: Res<crate::lobby::Sessions>,
    mut ship_q: Query<
        (
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            Option<&crate::entity_spawner::EntityUuid>,
            &ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            &ActiveBeam,
            &PhaserCooldown,
            &mut AdmittedCommands,
            Option<&PhaserCombatConfigResource>,
            Option<&crate::modifiers::ShipModifiers>,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    use crate::entity_config::PhaserCombatConfig;

    for (
        is_local,
        entity_uuid,
        control_sources,
        ship_config_opt,
        physics,
        blackboards,
        beam,
        cooldown,
        mut admitted,
        combat_config_opt,
        modifiers_opt,
    ) in ship_q.iter_mut()
    {
        // Gate: auto-fire only when at least one phaser bank on this ship is
        // AI-driven (per its own fine-system policy — issue #512), OR the
        // player globally toggled phaser mode to Auto (LocalShip-only signal
        // that is irrelevant for NPCs — they always satisfy the operate_ai
        // leg because their fine phaser bank systems are Ai by default).
        //
        // Per-bank gate: each bank's own policy is checked inside the
        // find_map below when choosing which bank to fire, so that AI can
        // auto-fire from one bank even when another is offline. This
        // ship-level `any_bank_operates_ai` predicate is only an early skip.
        let bank_ai_available = match ship_config_opt {
            Some(cfg) => any_bank_operates_ai(control_sources, &cfg.0),
            // No ship config (test-only spawns): derive the gate from the
            // same per-bank fine ids the fire loop below uses. No coarse
            // `tactical` fallback (issue #801).
            None => combat_config_opt.is_some_and(|cc| {
                cc.0.banks.iter().any(|b| {
                    crate::system_registry::phaser_bank_system_id(&b.id)
                        .is_some_and(|id| control_sources.0.policy_for(&id).operate_ai)
                })
            }),
        };
        let auto_fire = bank_ai_available || (is_local && phaser_mode.0 == PhaserMode::Auto);
        if !auto_fire {
            continue;
        }

        if beam.target_uuid.is_some() {
            continue;
        }

        // Target selection: the **Combat Lock** from this ship's frozen
        // viewscreen blackboard (issue #829, spec §1/§3). One-tick lag accepted
        // for firing. The tactical radar owns the live selection; auto-fire
        // reads the aggregated viewscreen fact.
        let Some(target_uuid) = (match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
            _ => None,
        }) else {
            continue;
        };
        let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
            continue;
        };

        let combat_config_default = PhaserCombatConfigResource::default();
        let combat_config: &PhaserCombatConfigResource =
            combat_config_opt.unwrap_or(&combat_config_default);
        let modifiers_default = crate::modifiers::ShipModifiers::new();
        let modifiers: &crate::modifiers::ShipModifiers =
            modifiers_opt.unwrap_or(&modifiers_default);

        // Find the first bank that is off-cooldown and has the target in its auto arc.
        // Per-bank policy gate (issue #512): skip banks whose fine system is
        // offline (damaged/destroyed) — auto-fire uses the same operate_ai
        // predicate as manual fire.
        let bank_id: Option<String> = if combat_config.0.banks.is_empty() {
            let effective_range =
                PhaserCombatConfig::DEFAULT_PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
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
                // Per-bank fine-system gate — skip offline banks.
                if let Some(bank_id) = crate::system_registry::phaser_bank_system_id(&b.id) {
                    if system_is_registered(control_sources, &bank_id)
                        && !control_sources.0.policy_for(&bank_id).operate_ai
                    {
                        return None;
                    }
                }
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

        // Emit as an admitted command through the shared AI seam.
        let Some(target) = crate::system_registry::phaser_bank_system_id(&bank_id) else {
            continue;
        };
        crate::command_admission::ai_emit::emit_ai_command(
            entity_uuid,
            target,
            crate::messages::SystemControlPayload::FirePhaser,
            control_sources,
            &sessions,
            ship_config_opt,
            &mut admitted,
        );
    }
}

/// Phase 1 of the beam tick (issue #723): snapshot shooter state and tick
/// per-bank cooldowns.
///
/// Unified per-tick beam handling for every ship (player + NPC). Iterates
/// `Query<..., With<Ship>>` — one loop handles player-fired beams
/// (LocalShip source) and NPC-fired beams (AI-controlled Ship source). Reads
/// per-bank config from each shooter's own `PhaserCombatConfigResource`
/// component (defaulting when absent) and applies the shooter's own
/// `ShipModifiers` to damage and range.
///
/// Collects an owned [`ShooterState`] snapshot per live beam — everything the
/// later phases ([`tick_beams_apply_damage`], [`tick_beams_tick_lifetimes`])
/// need to apply damage without holding a mutable borrow on the ship query —
/// into the one-tick [`BeamContext`] resource, cleared at the start of every
/// run so nothing goes stale across frames. In the same pass it ticks
/// cooldowns, pre-computes the per-tick damage integer and accumulator delta,
/// resolves the live target position, checks arc/range, and runs the LOS
/// raycast (Rapier) with friendly-fire classification to pick the effective
/// target. Shooters whose beam ends here (target vanished / out of arc) have
/// their cooldown started and `BeamEndedEvent` fired immediately and never
/// enter `BeamContext`.
///
/// The three phases together merge the former `tick_active_beam`
/// (player-only) and `tick_npc_beams` (NPC-only) systems — final divergence
/// closed under PRD #597.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tick_beams_prepare(
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
            // Every production ship carries TacticalRadarSelection (`Option` only for
            // minimal test spawns). We clear it here on the LocalShip alone,
            // because that lock is also the player's UI selection and nothing
            // else would drop it. NPC locks are left to `ai_target_selection`,
            // whose staleness guard clears a dead target on the next tick.
            Option<&mut TacticalRadarSelection>,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            // Shooter's faction for LOS friendly-fire check.
            Option<&FactionComponent>,
            // Shooter's phaser frequency for frequency-matching damage (issue #679).
            Option<&crate::ship_state::ShipPhaserFrequency>,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    // LOS raycast parameters (optional so tests without RapierPhysicsPlugin still pass).
    rapier_context: Option<ReadRapierContext>,
    faction_registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    // Read-only lookup: entity → UUID + faction, used to classify the LOS blocker.
    blocker_info_q: Query<(
        Entity,
        Option<&crate::entity_spawner::EntityUuid>,
        Option<&AsteroidUuid>,
        Option<&FactionComponent>,
    )>,
    mut beam_context: ResMut<BeamContext>,
) {
    use crate::entity_config::PhaserCombatConfig;

    let dt = time.delta_secs();

    // One-tick resource: clear before repopulating so nothing from the
    // previous frame survives.
    beam_context.clear();

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
        shooter_faction_opt,
        shooter_phaser_freq_opt,
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
        let modifiers: &crate::modifiers::ShipModifiers =
            modifiers_opt.unwrap_or(&modifiers_default);

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
            let effective_phaser_range =
                PhaserCombatConfig::DEFAULT_PHASER_RANGE * modifiers.get(&ModifierSlot::RadarRange);
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

        beam.damage_accumulator += damage_per_sec * modifiers.get(&ModifierSlot::PhaserDamage) * dt;
        let damage_to_apply = beam.damage_accumulator.floor() as i32;
        // Deduct the integer part now; the snapshot below drives damage
        // application in phase 2.
        beam.damage_accumulator -= damage_to_apply as f32;

        // ── LOS raycast: check if another entity blocks the beam this tick ──
        //
        // When Rapier physics is not loaded (tests without RapierPhysicsPlugin),
        // `rapier_context` is None — skip LOS and apply damage to the original
        // target as before.
        let (effective_target_uuid, effective_target_x, effective_target_z, zero_damage) =
            if let Some(ref ctx_param) = rapier_context {
                if let Ok(ctx) = ctx_param.single() {
                    let ray_origin = Vec3::new(shooter_physics.x, 0.0, shooter_physics.z);
                    let to_target = Vec3::new(tx - shooter_physics.x, 0.0, tz - shooter_physics.z);
                    let dist_to_target = to_target.length();
                    if dist_to_target > f32::EPSILON {
                        let ray_dir = to_target / dist_to_target;
                        let filter = bevy_rapier3d::prelude::QueryFilter::new()
                            .exclude_rigid_body(shooter_entity);
                        if let Some((hit_entity, toi)) =
                            ctx.cast_ray(ray_origin, ray_dir, dist_to_target, true, filter)
                        {
                            // Classify the blocking entity.
                            if let Ok((_, blocker_ent_uuid, blocker_ast_uuid, blocker_faction)) =
                                blocker_info_q.get(hit_entity)
                            {
                                let blocker_uuid_str: Option<&str> = blocker_ent_uuid
                                    .map(|u| u.0.as_str())
                                    .or_else(|| blocker_ast_uuid.map(|u| u.0.as_str()));

                                // Only reroute when blocker is a *different* entity from
                                // the original target (the ray hits the target itself at
                                // toi == dist_to_target).
                                let blocker_is_target = blocker_uuid_str
                                    .map(|u| u == target_uuid.as_str())
                                    .unwrap_or(false);

                                if !blocker_is_target && toi < dist_to_target {
                                    // Determine friendliness.
                                    let is_friendly = match (
                                        shooter_faction_opt.map(|f| f.0),
                                        blocker_faction.map(|f| f.0),
                                        &faction_registry,
                                    ) {
                                        (Some(sf), Some(bf), Some(reg)) => {
                                            !crate::faction::is_enemy(Some(sf), Some(bf), reg)
                                        }
                                        // No faction data → treat as non-friendly (takes damage).
                                        _ => false,
                                    };

                                    if is_friendly {
                                        // Friendly ship blocks; nobody takes damage this tick.
                                        (target_uuid.clone(), tx, tz, true)
                                    } else {
                                        // Enemy/neutral/asteroid blocks; blocker takes damage.
                                        let blocker_uuid =
                                            blocker_uuid_str.unwrap_or("").to_string();
                                        if blocker_uuid.is_empty() {
                                            // Blocker has no UUID — fall through to original target.
                                            (target_uuid.clone(), tx, tz, false)
                                        } else {
                                            // Use the ray hit position as the blocker position
                                            // for VFX (e.g. asteroid destruction effects).
                                            let hit_pos = ray_origin + ray_dir * toi;
                                            (blocker_uuid, hit_pos.x, hit_pos.z, false)
                                        }
                                    }
                                } else {
                                    // Hit was the target itself — no blocker.
                                    (target_uuid.clone(), tx, tz, false)
                                }
                            } else {
                                // Hit entity not in blocker_info_q — fall through to original target.
                                (target_uuid.clone(), tx, tz, false)
                            }
                        } else {
                            // No LOS blocker found.
                            (target_uuid.clone(), tx, tz, false)
                        }
                    } else {
                        // Shooter and target at same position — no LOS check.
                        (target_uuid.clone(), tx, tz, false)
                    }
                } else {
                    // Rapier context unavailable at this tick.
                    (target_uuid.clone(), tx, tz, false)
                }
            } else {
                // No Rapier plugin — skip LOS.
                (target_uuid.clone(), tx, tz, false)
            };

        beam_context.0.push(ShooterState {
            shooter_entity,
            shooter_uuid: shooter_uuid_opt.map(|u| u.0.clone()).unwrap_or_default(),
            shooter_x: shooter_physics.x,
            shooter_z: shooter_physics.z,
            target_uuid,
            active_bank,
            cooldown_secs,
            damage_to_apply,
            shield_pierce,
            end_beam_early: false,
            is_local_shooter,
            effective_target_uuid,
            effective_target_x,
            effective_target_z,
            zero_damage,
            shooter_phaser_freq: shooter_phaser_freq_opt.map(|f| f.0).unwrap_or(0.5),
        });
    }
}

/// Phase 2 of the beam tick (issue #723): apply damage to targets.
///
/// For each shooter snapshot in [`BeamContext`] (written by
/// [`tick_beams_prepare`] earlier this tick), find its target in `hull_q`,
/// route damage through shields, apply hull damage, and record whether the
/// target was destroyed — `end_beam_early`, read by
/// [`tick_beams_tick_lifetimes`] to end the beam and clear `TacticalRadarSelection`.
///
/// Damage routing rules:
/// - Asteroid target → emits `AsteroidDestroyed` + `AsteroidDestroyedVfx`.
/// - Non-asteroid, non-LocalShip target (NPC or station) → emits
///   `EntityDespawned` + `AiEntityDestroyed` on kill.
/// - LocalShip target → emits `DamageTaken` per hit and `ShipDestroyed` +
///   `GameOver` on kill. Never despawns the LocalShip entity.
///
/// Attacker tracking: every non-asteroid target has `ShipAttackedThisTick`
/// set true and `LastShipAttacker` set to the shooter's UUID — the latter
/// **compared before writing**, because its change detection is the rising edge
/// that fires `AiEntityAttacked` and the `on_entity_attacked` triggers behind it
/// (issue #702). See the note on [`LastShipAttacker`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn tick_beams_apply_damage(
    mut commands: Commands,
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
        &mut EntitySystemHull,
        Option<&mut crate::ship::shields::ShipShields>,
        Option<&mut crate::server_app::ShipAttackedThisTick>,
        Option<&mut LastShipAttacker>,
        bevy::ecs::query::Has<crate::server_app::LocalShip>,
        Option<&mut crate::entity_spawner::EntityShipArcHull>,
        Option<&crate::entity_spawner::ColliderSection>,
    )>,
    mut world: ResMut<WorldResource>,
    mut outbox: Option<ResMut<SimOutbox>>,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut ship_vfx_events: MessageWriter<ShipDestroyedVfx>,
    mut beam_context: ResMut<BeamContext>,
    // `Option<ResMut<Messages<_>>>` rather than `MessageWriter` so bare-`App`
    // fixtures that never registered the message still pass Bevy's parameter
    // validation. Balance telemetry must never be the reason a test app fails
    // to run.
    mut balance_events: Option<ResMut<Messages<crate::balance::BalanceEvent>>>,
    sim_rng: Option<Res<crate::sim_rng::SimRng>>,
    // Single-emission bookkeeping (issue #838): this kill site is the eager
    // emitter of `EntityDespawned`, so it must forget the uuid from the
    // `TrackedEntities` registry it despawns. Otherwise the reconcile sweep
    // (`reconcile_runtime_entities`), which emits `EntityDespawned` for every
    // reported uuid no longer in the ECS, re-emits a *second* one for the same
    // kill. `Option` because bare-`App` weapons fixtures never insert the
    // resource — there the sweep does not run either, so the eager emit stands
    // alone and the tests that assert on it stay green.
    mut tracked: Option<ResMut<crate::server_app::TrackedEntities>>,
    // `Option<Res<_>>`, never bare — bare-`App` weapons fixtures never insert
    // `LogFilterConfig` (see logging macro docs).
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    for state in beam_context.0.iter_mut() {
        // When a friendly ship blocks the beam this tick, skip all damage and
        // attacker tracking — nobody takes damage.
        if state.zero_damage {
            continue;
        }

        // Instagib: local ship deals 100x damage.
        if state.is_local_shooter && crate::bridge::is_instagib() {
            state.damage_to_apply = state.damage_to_apply.saturating_mul(100);
        }

        // Always mark the effective target ship as attacked, even when
        // damage_to_apply == 0 (mirrors the historical NPC path which tagged
        // the target every tick the beam was live). Skip for asteroid targets.
        {
            // Look up the effective target and set attacker/attacked components.
            let target_entity =
                hull_q
                    .iter()
                    .find_map(|(e, ast_uuid, ent_uuid, _, _, _, _, _, _, _, _, _)| {
                        let asteroid_match = ast_uuid.map(|u| u.0.as_str())
                            == Some(state.effective_target_uuid.as_str());
                        let entity_match = ent_uuid.map(|u| u.0.as_str())
                            == Some(state.effective_target_uuid.as_str());
                        if asteroid_match || entity_match {
                            Some((e, ast_uuid.is_some()))
                        } else {
                            None
                        }
                    });
            if let Some((te, is_asteroid)) = target_entity {
                if !is_asteroid {
                    if let Ok((_, _, _, _, _, _, _, attacked_opt, last_attacker_opt, _, _, _)) =
                        hull_q.get_mut(te)
                    {
                        if let Some(mut atk) = attacked_opt {
                            atk.0 = true;
                        }
                        // Compare before writing (issue #702). This branch runs
                        // every tick a beam is live, so a blind write would mark
                        // `LastShipAttacker` changed every one of them. Its
                        // change detection is the rising-edge latch that fires
                        // `AiEntityAttacked` (see
                        // `ai_plugin::emit_attacked_on_new_attacker`), and
                        // `on_entity_attacked` scenario triggers hang off that —
                        // so sustained fire from one shooter must touch the
                        // component exactly once, on the tick the shooter
                        // changes. `Mut::set_if_neq` is the same pattern
                        // `ai_target_selection` uses for `TacticalRadarSelection`.
                        if let Some(mut last) = last_attacker_opt {
                            last.set_if_neq(LastShipAttacker(Some(state.shooter_uuid.clone())));
                        }
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
        let mut destroyed_ship_radius = DEFAULT_SHIP_EXPLOSION_RADIUS;

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
            mut target_arc_hull,
            collider_opt,
        ) in hull_q.iter_mut()
        {
            let uuid_matches = ast_uuid.map(|u| u.0.as_str())
                == Some(state.effective_target_uuid.as_str())
                || ent_uuid.map(|u| u.0.as_str()) == Some(state.effective_target_uuid.as_str());
            if !uuid_matches {
                continue;
            }
            damage_applied = true;
            let is_asteroid = ast_uuid.is_some();

            // God mode: local ship takes no damage.
            if target_is_local && crate::bridge::is_god_mode() {
                if let Some(ref mut ob) = outbox {
                    ob.0.push((
                        Target::All,
                        ServerMessage::DamageTaken {
                            hull: 0.0,
                            shield: 0.0,
                        },
                    ));
                }
                break;
            }

            // Frequency-matching damage multiplier (issue #679).
            // When phaser frequency matches shield frequency, damage is at
            // 100%; the further apart they are, the less damage gets through.
            // Minimum 25% unless the target has no shields (then 100%).
            let freq_mult: f32 = if let Some(ref shields) = ship_shields_comp {
                let sf = shields.frequency();
                let pf = state.shooter_phaser_freq;
                (1.0 - (pf - sf).abs() * 0.5).clamp(0.25, 1.0)
            } else {
                1.0
            };
            let base_damage = (state.damage_to_apply as f32 * freq_mult).round() as i32;

            // Snapshot which facings are online *before* the shield apply, so
            // the online→offline edge can be reported as `ShieldArcCollapsed`
            // (issue #841). Cheap clone of ids; only ships carry shields.
            let arcs_online_before: Vec<(String, bool)> = ship_shields_comp
                .as_ref()
                .map(|s| {
                    s.0.facings
                        .iter()
                        .map(|f| (f.id.clone(), f.is_online()))
                        .collect()
                })
                .unwrap_or_default();

            // Route damage through shields if present and any facing online.
            let (damage_to_hull, shield_amount) = if let Some(ref mut shields) = ship_shields_comp {
                let all_offline = shields.0.facings.iter().all(|f| !f.is_online());
                if all_offline {
                    (base_damage as f32, 0.0f32)
                } else {
                    let (pierced, absorbed) = crate::damage::split_damage_for_pierce(
                        base_damage as f32,
                        state.shield_pierce,
                    );
                    let bearing = if target_is_local {
                        // Player shield uses bearing-based routing to the
                        // appropriate facing. Fall back to the shooter's
                        // own position when the target has no Transform
                        // (bearing = 0.0 in that degenerate case).
                        let target_yaw = target_physics_opt.map(|p| p.yaw).unwrap_or(0.0);
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
                (base_damage as f32, 0.0f32)
            };

            let mut hull_applied_total = 0.0f32;
            let ship_destroyed = if damage_to_hull > 0.0 {
                let (hull_applied, destroyed) = crate::sim_rng::with_stream(
                    sim_rng.as_deref(),
                    crate::sim_rng::SimStream::BeamDamage,
                    |rng| {
                        let result =
                            crate::damage::apply_hull_damage(&mut hull_comp.0, damage_to_hull, rng);
                        // Distribute the same absorbed amount across per-arc
                        // hull (issue #514). Skipped when the target has no
                        // `EntityShipArcHull` (NPCs, asteroids).
                        if let Some(ref mut arc_hull) = target_arc_hull {
                            arc_hull.0.apply_damage(result.0, rng);
                        }
                        result
                    },
                );
                hull_applied_total = hull_applied;
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
                                // The LocalShip died → defeat (#843), latched
                                // under the same first-write guard as the reason.
                                reason.1 = Some(crate::balance::Outcome::Defeat);
                                // EntityDestroyed for the player death, exactly
                                // once (guarded by the first-write of the reason).
                                // Killer credit = the beam's shooter (issue #841).
                                //
                                // Shared-latch coupling (issue #841): the death
                                // tracer piggybacks on `GameOverReason` as its
                                // "fire once" latch, but a scenario's
                                // `ActionCmd::SetGameOverReason`
                                // (world/server.rs) writes that same latch. If a
                                // scenario declared game-over in the same tick a
                                // local ship dies, the reason would already be
                                // `Some` and this EntityDestroyed (with its death
                                // timestamp) would be dropped. This is accepted,
                                // not a bug: weapon/region damage runs only in
                                // `GamePhase::InProgress`, so it needs a
                                // *coincident* scenario game-over plus a local
                                // death in one tick — vanishingly narrow, and the
                                // consequence is one missing telemetry row, never
                                // a gameplay effect. A dedicated latch is not
                                // worth the four extra call sites it would touch.
                                // The same coupling holds at the blaster,
                                // collision, and region death sites.
                                if let Some(ref mut msgs) = balance_events {
                                    msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                                        victim: state.effective_target_uuid.clone(),
                                        killer: Some(state.shooter_uuid.clone())
                                            .filter(|u| !u.is_empty()),
                                    });
                                }
                            }
                        }
                    }
                }
                destroyed
            } else {
                false
            };

            // Human-readable logging alongside the structured BalanceEvent
            // (does NOT replace it). Level discipline: a live beam applies
            // damage *every tick*, so the per-hit line is `trace` — that is
            // what trace is for. The one `info` edge here is destruction, a
            // true state edge a balancer reads as a headline. Both entity-
            // scoped to the victim so `--log-entity` narrows to one hull.
            let attacker_label: &str = if state.shooter_uuid.is_empty() {
                "unknown"
            } else {
                state.shooter_uuid.as_str()
            };
            crate::ptrace!(
                log,
                crate::logging::LogCat::Damage,
                entity = target_entity,
                "took {} (shield {:.0}/hull {:.0}) from {} via {}",
                base_damage,
                shield_amount,
                hull_applied_total,
                attacker_label,
                state.active_bank
            );
            if ship_destroyed && !is_asteroid {
                crate::pinfo!(
                    log,
                    crate::logging::LogCat::Damage,
                    entity = target_entity,
                    "destroyed by {}",
                    attacker_label
                );
            }

            // Balance tracer. Deliberately outside every `is_local` branch
            // above: the whole point is to see both halves of a fight.
            if let Some(ref mut msgs) = balance_events {
                msgs.write(crate::balance::BalanceEvent::DamageApplied {
                    // `ShooterState.shooter_uuid` is empty for a shooter with
                    // no `EntityUuid`. That is "unknown", not a ship called
                    // "" — every chokepoint models an unknown attacker as
                    // `None`, and a blank key would otherwise open a junk
                    // ledger row.
                    attacker: Some(state.shooter_uuid.clone()).filter(|u| !u.is_empty()),
                    victim: state.effective_target_uuid.clone(),
                    victim_kind: if is_asteroid {
                        crate::balance::VictimKind::Asteroid
                    } else {
                        crate::balance::VictimKind::Ship
                    },
                    weapon: state.active_bank.clone(),
                    amount: base_damage as f32,
                    shield_absorbed: shield_amount,
                    hull_damage: hull_applied_total,
                    system_hit: None,
                });
                // Emit `ShieldArcCollapsed` once per facing that just crossed
                // online→offline under this hit. Ships only — asteroids carry
                // no shields.
                if !is_asteroid {
                    if let Some(ref shields) = ship_shields_comp {
                        for (id, was_online) in &arcs_online_before {
                            if !was_online {
                                continue;
                            }
                            let now_offline = shields
                                .0
                                .facings
                                .iter()
                                .find(|f| &f.id == id)
                                .map(|f| !f.is_online())
                                .unwrap_or(false);
                            if now_offline {
                                msgs.write(crate::balance::BalanceEvent::ShieldArcCollapsed {
                                    ship: state.effective_target_uuid.clone(),
                                    arc_id: id.clone(),
                                });
                            }
                        }
                    }
                }
            }

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
                    destroyed_ship_radius = collider_opt
                        .map(|c| c.0.radius)
                        .unwrap_or(DEFAULT_SHIP_EXPLOSION_RADIUS);
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
            world
                .0
                .entities
                .retain(|a| a.uuid != state.effective_target_uuid);
            if target_asteroid_destroyed {
                vfx_events.write(AsteroidDestroyedVfx {
                    x: state.effective_target_x,
                    z: state.effective_target_z,
                });
                if let Some(ref mut ob) = outbox {
                    ob.0.push((
                        Target::All,
                        ServerMessage::AsteroidDestroyed {
                            uuid: state.effective_target_uuid.clone(),
                        },
                    ));
                }
            } else {
                destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                    entity_uuid: state.effective_target_uuid.clone(),
                });
                ship_vfx_events.write(ShipDestroyedVfx {
                    x: state.effective_target_x,
                    z: state.effective_target_z,
                    radius: destroyed_ship_radius,
                });
                if let Some(ref mut ob) = outbox {
                    ob.0.push((
                        Target::All,
                        ServerMessage::EntityDespawned {
                            uuid: state.effective_target_uuid.clone(),
                        },
                    ));
                }
                // Forget the uuid so the reconcile sweep does not re-emit
                // (issue #838, single-emission invariant).
                if let Some(t) = tracked.as_mut() {
                    t.forget(&state.effective_target_uuid);
                }
                // EntityDestroyed for the NPC kill, co-located with the
                // AiEntityDestroyed write so it fires exactly once. Killer
                // credit = the beam's shooter (issue #841).
                if let Some(ref mut msgs) = balance_events {
                    msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                        victim: state.effective_target_uuid.clone(),
                        killer: Some(state.shooter_uuid.clone()).filter(|u| !u.is_empty()),
                    });
                }
            }
            state.end_beam_early = true;
        }
    }
}

/// Phase 3 of the beam tick (issue #723): end beams that hit a destroyed
/// target and tick `remaining_secs` on the rest.
///
/// Reads the shooter snapshots from [`BeamContext`] (`end_beam_early` set by
/// [`tick_beams_apply_damage`]) and borrows the ship query mutably to update
/// per-shooter beam state (target cleared, cooldown started, `TacticalRadarSelection`
/// cleared for LocalShip).
///
/// Weapons-target clearing: when the player kills its locked target, its
/// `TacticalRadarSelection.0` is set to `None`. NPC locks are re-evaluated by
/// `ai_target_selection`, whose staleness guard clears a dead target.
pub(crate) fn tick_beams_tick_lifetimes(
    time: Res<Time>,
    mut commands: Commands,
    // Narrowed shooter query: phase 3 only mutates the shooter's beam,
    // cooldown, and (LocalShip only) weapons-target lock. Every production
    // ship carries TacticalRadarSelection (`Option` only for minimal test spawns).
    mut ship_q: Query<
        (
            &mut ActiveBeam,
            &mut PhaserCooldown,
            Option<&mut TacticalRadarSelection>,
        ),
        With<crate::server_app::Ship>,
    >,
    beam_context: Res<BeamContext>,
) {
    let dt = time.delta_secs();

    for state in beam_context.0.iter() {
        let Ok((mut beam, mut cooldown, mut weapons_target_opt)) =
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
pub(crate) fn handle_set_phaser_mode(
    ship_query: Query<
        (
            &AdmittedCommands,
            &ShipSystemControlSources,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut phaser_mode: ResMut<CurrentPhaserMode>,
) {
    let Some((admitted, control_sources, ship_config)) = ship_query.iter().next() else {
        return;
    };
    // Ship-level gate (issue #512, option c): any bank human-operable.
    if !any_bank_accepts_human_input(control_sources, &ship_config.0) {
        return;
    }
    for cmd in admitted.for_target(crate::system_registry::PHASER_CONTROL_SYSTEM_ID) {
        if let SystemControlPayload::SetPhaserMode { mode } = &cmd.payload {
            phaser_mode.0 = *mode;
        }
    }
}

pub(crate) fn handle_set_phaser_frequency(
    ship_query: Query<
        (
            &AdmittedCommands,
            &ShipSystemControlSources,
            &crate::ship_plugin::ShipConfigComponent,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut freq_q: Query<
        &mut crate::ship_state::ShipPhaserFrequency,
        With<crate::server_app::LocalShip>,
    >,
) {
    let Some((admitted, control_sources, ship_config)) = ship_query.iter().next() else {
        return;
    };
    // Ship-level gate (issue #512, option c): any bank human-operable.
    if !any_bank_accepts_human_input(control_sources, &ship_config.0) {
        return;
    }
    for cmd in admitted.for_target(crate::system_registry::PHASER_CONTROL_SYSTEM_ID) {
        if let SystemControlPayload::SetPhaserFrequency { frequency } = &cmd.payload {
            if let Some(mut freq) = freq_q.iter_mut().next() {
                freq.0 = frequency.clamp(0.0, 1.0);
            }
        }
    }
}

/// Drains each ship's Power battery via the inter-system command channel
/// while its own phaser beam is active. Runs in `SimSet::Physics` (one
/// phase before `tick_beams` in `SimSet::Damage`); the Power system
/// consumes the drain in `SimSet::Modifiers` per-entity via `source_entity`.
///
/// Iterates every ship so NPC beams drain their own power grid the same
/// way the player's do. The target is the **battery** fine system
/// (`POWER_BATTERY_SYSTEM_ID` — issue #513); a Disabled/Destroyed battery
/// refuses the drain via `handle_power_inter_system`'s offline gate.
pub fn drain_power_for_active_beam(
    beam_q: Query<(Entity, &ActiveBeam), With<crate::server_app::Ship>>,
    time: Res<Time>,
    mut inter_system: ResMut<InterSystemQueue>,
) {
    let amount = PHASER_BATTERY_DRAIN_PER_SEC * time.delta_secs();
    for (source_entity, beam) in beam_q.iter() {
        if beam.target_uuid.is_some() {
            inter_system.0.push(InterSystemMsg {
                target: crate::system_registry::power_battery_system_id(),
                payload: InterSystemPayload::DrainWeaponsBattery { amount },
                source_entity: Some(source_entity),
            });
        }
    }
}

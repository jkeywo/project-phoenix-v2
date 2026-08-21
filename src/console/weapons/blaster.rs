//! Blaster systems (issue #631), extracted from `server.rs` (issue #726).
//!
//! Note: `crate::blaster::BlasterSystem` (the pure-Rust bank model in
//! `src/weapons/blaster.rs`) is a different module from this file — this one
//! holds the Bevy systems and the ECS wrapper resource.

use crate::simmath;
use bevy::prelude::*;

use super::shared::{any_blaster_bank_operates_ai, live_entity_xz, system_is_registered};
use super::{AsteroidDestroyedVfx, ShipDestroyedVfx, DEFAULT_SHIP_EXPLOSION_RADIUS};
use crate::lobby::{Sessions, Target, WorldResource};
use crate::messages::{GamePhase, ServerMessage, SystemBlackboard, SystemControlPayload};

/// This ship's **Combat Lock** from its frozen viewscreen blackboard (issue
/// #829): the ship-wide target every weapons firing path reads, in place of the
/// retired `TacticalRadarSelection` component. One-tick lag at 30Hz accepted (spec §1).
fn blaster_combat_lock(
    blackboards: Option<&crate::server_app::ShipSystemBlackboards>,
) -> Option<String> {
    match blackboards?
        .0
        .get(&crate::system_registry::viewscreen_system_id())
    {
        Some(SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
        _ => None,
    }
}
use crate::model_rig::ModelMarkers;
use crate::ship_plugin::ShipSystemControlSources;
use crate::ship_state::ShipPhysics;
use crate::simulation::{AsteroidUuid, GameOverReason, SimOutbox};

/// Wraps the pure-Rust blaster system(s) so they can be used as a Bevy
/// component on each ship entity (issue #631).
///
/// Each element corresponds to one `[[weapons_console.blaster_banks]]` entry.
/// A ship with no blaster banks will have an empty `Vec`.
#[derive(Resource, Component, Clone, Default)]
pub struct BlasterSystemResource(pub Vec<crate::blaster::BlasterSystem>);

/// Per-ship map of each blaster bank's inline stateless open-fire policy
/// (issue #781), keyed by `BlasterBankConfig.id`. The blaster twin of
/// [`crate::weapons_plugin::PhaserBankAiPolicies`]: built at spawn from each
/// bank's authored `ai` block, falling back to the canonical
/// [`crate::entities::config::default_blaster_bank_ai_config`] (unconditional
/// fire) so a bank without an authored policy keeps auto-firing exactly as before
/// (AC1). Read by [`tick_blaster_auto_fire`].
#[derive(Component, Default, Clone, Debug)]
pub struct BlasterBankAiPolicies(
    pub std::collections::HashMap<crate::entity_config::BlasterBankId, crate::ai::policy::AiPolicy>,
);

/// Seed the per-tick policy fact snapshot for one blaster bank's open-fire
/// decision (issue #781), the blaster twin of
/// [`crate::weapons_plugin::seed_phaser_bank_facts`]. Closes the #779 empty-facts
/// edge for blaster banks: the host resolves the bank's live readiness before
/// calling this, so a `fact(...)` guard evaluates over real per-bank state while
/// `policy.rs` stays Bevy-free.
/// `posture` carries the SHIP-WIDE readings — the alert added by issue #872 and
/// the weapons hold added by issue #1041; see
/// [`crate::weapons_plugin::seed_phaser_bank_facts`] and
/// [`crate::weapons_plugin::WeaponsAlertPosture`] for the contract. Seeded
/// unconditionally so an authored guard reads a real `0.0`, never an absent
/// fact.
pub fn seed_blaster_bank_facts(
    target_valid: bool,
    on_cooldown: bool,
    cooldown_remaining: f32,
    in_range: bool,
    in_arc: bool,
    posture: crate::weapons_plugin::WeaponsAlertPosture,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("target_valid", if target_valid { 1.0 } else { 0.0 });
    facts.set("on_cooldown", if on_cooldown { 1.0 } else { 0.0 });
    facts.set("cooldown_remaining", cooldown_remaining as f64);
    facts.set("in_range", if in_range { 1.0 } else { 0.0 });
    facts.set("in_arc", if in_arc { 1.0 } else { 0.0 });
    facts.set(
        crate::entities::config::POWER_RED_ALERT_FACT,
        posture.alert_fact_value(),
    );
    facts
}

/// Resolve a blaster bank's policy to a bare "open fire this tick?" boolean
/// (issue #781). Returns `true` only when a guard fires on the `blaster_fire`
/// channel yielding `FireBlaster`; `None`/idle/mismatched verbs "hold".
fn blaster_bank_policy_fires(
    policy: &crate::ai::policy::AiPolicy,
    facts: &crate::world::flags::AiFacts,
    flags: &[&crate::world::flags::FlagStore],
) -> bool {
    policy.resolve_channel(crate::entities::config::BLASTER_FIRE_CHANNEL, facts, flags)
        == Some(&crate::ai::policy::AiPolicyVerb::FireBlaster)
}

/// Admitted-command consumer for blaster fire/charge control (issue #781).
///
/// Reads each ship's own `AdmittedCommands` for blaster payloads:
///
/// - `FireBlaster` — legacy alias, behaves as `ChargeBlasterStart` (instant-fire
///   when `charge_time_secs == 0`).
/// - `ChargeBlasterStart` — begins charge (or instant-fires when
///   `charge_time_secs == 0`, issue #636).
/// - `ChargeBlasterCancel` — cancels an in-progress charge with no penalty.
///
/// # Control-Source symmetry (issue #781, AC5/AC7 — the convergence fix)
///
/// Before #781 the human path read raw `InboundMessage`s here (with its own
/// `tactical_authorized` check) while the AI path called
/// `bank.request_charge_start()` DIRECTLY inside `tick_blaster_auto_fire` — two
/// origins, two code paths, and the human path applied an arc check the AI path
/// skipped. Now both origins converge exactly like phasers: a human's
/// `ControlSystem` is admitted by `admit_system_commands`, an AI decision is
/// emitted through `emit_ai_command`, and BOTH land in this ship's
/// `AdmittedCommands`, which this system consumes with no human-vs-AI branch. The
/// arc check below therefore applies identically to both origins.
///
/// Runs in `SimSet::Physics` — after admission's `clear_before_input` and after
/// the AI decider (`tick_blaster_auto_fire`, `SimSet::Input`) has emitted, but
/// ordered before `tick_blaster_system` so the armed volley launches the same
/// tick.
pub(crate) fn handle_fire_blaster(
    mut ship_q: Query<
        (
            &ShipSystemControlSources,
            &ShipPhysics,
            Option<&crate::server_app::ShipSystemBlackboards>,
            &mut BlasterSystemResource,
            &crate::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
) {
    for (control_sources, physics, blackboards_opt, mut blaster_res, admitted) in ship_q.iter_mut()
    {
        for cmd in admitted.0.iter() {
            // Accept FireBlaster (legacy), ChargeBlasterStart, and ChargeBlasterCancel.
            let is_charge_start = matches!(
                cmd.payload,
                SystemControlPayload::FireBlaster | SystemControlPayload::ChargeBlasterStart
            );
            let is_charge_cancel = matches!(cmd.payload, SystemControlPayload::ChargeBlasterCancel);
            if !is_charge_start && !is_charge_cancel {
                continue;
            }

            // Resolve the command's target back to one of THIS ship's banks by
            // running the canonical forward mapping over each authored bank id
            // and comparing — never by inverting the string (the mapping folds
            // `_` to `-`, so the inverse is lossy).
            let Some(bank_id) = blaster_res.0.iter().find_map(|b| {
                crate::system_registry::blaster_bank_system_id(&b.config.id)
                    .filter(|id| id == &cmd.target)
                    .map(|_| b.config.id.clone())
            }) else {
                continue;
            };

            // System-state gate: the bank must be operable (not Offline).
            // Admission already gated the token identity, so — like
            // `handle_fire_phaser` — this only checks operability, with no
            // human-vs-AI branch below this point.
            let bank_system_id = crate::system_registry::blaster_bank_system_id(&bank_id)
                .filter(|id| system_is_registered(control_sources, id));
            let policy = match &bank_system_id {
                Some(id) => control_sources.0.policy_for(id),
                None => crate::ship::control_source::control_tick_policy(
                    crate::ship::control_source::ControlSource::default(),
                ),
            };
            if !policy.accept_human_input && !policy.operate_ai {
                continue;
            }

            // Arc check for a fire/charge-start, applied to BOTH origins now that
            // the source identity is stripped. The target is the ship's frozen
            // combat lock — the same surface `tick_blaster_auto_fire` reads. A
            // cancel needs no target/arc.
            if is_charge_start {
                let Some(target_uuid) = blaster_combat_lock(blackboards_opt) else {
                    continue;
                };
                let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
                    continue;
                };
                let bank_arc_ok = blaster_res
                    .0
                    .iter()
                    .find(|b| b.config.id == bank_id)
                    .map(|bank| {
                        let (rx, ry) = crate::weapons::phaser::ship_local(
                            tx,
                            tz,
                            physics.x,
                            physics.z,
                            physics.yaw,
                        );
                        crate::weapons::phaser::in_arc(
                            rx,
                            ry,
                            bank.config.facing_deg,
                            bank.config.fire_arc_deg,
                        )
                    })
                    .unwrap_or(false);
                if !bank_arc_ok {
                    continue;
                }
            }

            // Dispatch to the matching bank. `request_charge_start` /
            // `request_fire` are self-guarding (no-op when the bank is not
            // fire-ready), so a redundant order is harmless.
            if let Some(bank) = blaster_res.0.iter_mut().find(|b| b.config.id == bank_id) {
                if is_charge_start {
                    bank.request_charge_start();
                } else {
                    bank.request_charge_cancel();
                }
            }
        }
    }
}

/// Decide which AI-controlled blaster banks should open fire this tick, and emit
/// an admitted `ChargeBlasterStart` for each through the shared AI seam
/// (issue #781).
///
/// Iterates every ship (`With<Ship>`) — player + NPC. For each blaster bank whose
/// fine-system policy has `operate_ai == true`, the host resolves the bank's live
/// readiness (target lock, fire-ready, range, arc), seeds a per-bank fact
/// snapshot, and resolves the bank's OWN inline stateless open-fire policy on the
/// `blaster_fire` channel. Only a bank whose policy fires emits — and it emits the
/// SAME typed `ChargeBlasterStart` a human does via
/// [`crate::command_admission::ai_emit::emit_ai_command`], converging with the
/// human path at [`handle_fire_blaster`] (AC5/AC7). No bank spawns a volley
/// directly anymore.
///
/// Target selection: the **Combat Lock** from this ship's frozen viewscreen
/// blackboard (issue #829, spec §1/§3). Range and arc checks use each bank's
/// config values (AGENTS.md #11 — thresholds stay TOML on the bank).
#[allow(clippy::too_many_arguments)]
pub(crate) fn tick_blaster_auto_fire(
    sessions: Res<Sessions>,
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    // Objective-contributed Command stances (issue #1110). `Option<Res<_>>` so a
    // bare-`App` weapons fixture with no world plugin reads no contributions.
    active_objective_stances: Option<Res<crate::console::command::server::ActiveObjectiveStances>>,
    mut ship_q: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &ShipPhysics,
            Option<&crate::server_app::ShipSystemBlackboards>,
            &BlasterSystemResource,
            Option<&BlasterBankAiPolicies>,
            &mut crate::messages::AdmittedCommands,
            // Issue #872: this ship's own red-alert state, seeded as a typed
            // fact for the bank's authored fire predicate. `Option<&_>` for
            // bare-`App` fixtures; absent reads `false`.
            Option<&crate::ship_state::ShipRedAlert>,
            // Issue #1041: the captain's weapons hold, folded with the alert
            // above into the one `red_alert` fact the authored gate reads.
            Option<&crate::ship_state::ShipWeaponsHold>,
            // Issue #1107: the ship's Command stance selections, so a directed
            // AI weapons Station's stance decides the fire posture in place of
            // the ship's own Red Alert. Absent reads as no direction.
            Option<&crate::console::command::server::ShipStationStances>,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    crate::ptrace!(
        log,
        crate::logging::LogCat::Weapons,
        "blaster auto-fire: {} ship(s) in query",
        ship_q.iter().len()
    );

    for (
        ship_entity,
        entity_uuid,
        control_sources,
        ship_config_opt,
        physics,
        blackboards_opt,
        blaster_res,
        bank_policies_opt,
        mut admitted,
        red_alert_opt,
        weapons_hold_opt,
        stances_opt,
    ) in ship_q.iter_mut()
    {
        // Read once per ship; seeded into every bank's snapshot. No Rust rule
        // consults it — the gate is the bank's authored predicate (#872), and
        // the weapons hold folded in beside it rides that same predicate
        // (#1041). The Command stance override (#1107) rides the same fact:
        // absent a direction it is `None` and the seeded value is bit-for-bit
        // the pre-#1107 reading.
        let stance_override = ship_config_opt.and_then(|cfg| {
            crate::console::command::server::weapons_station_stance_high_alert(
                stances_opt,
                active_objective_stances.as_deref(),
                &cfg.0,
                &control_sources.0,
                red_alert_opt.is_some_and(|r| r.0),
            )
        });
        let posture = crate::weapons_plugin::WeaponsAlertPosture::from_parts(
            red_alert_opt,
            weapons_hold_opt,
            stance_override,
        );
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(ship_entity);
        // Gate: only run when at least one blaster bank is AI-controlled.
        let ai_controlled = match ship_config_opt {
            Some(cfg) => any_blaster_bank_operates_ai(control_sources, &cfg.0),
            // No ship config (test-only spawns): derive the gate from the
            // same per-bank fine ids the fire loop uses. No coarse
            // `tactical` fallback (issue #801).
            None => blaster_res.0.iter().any(|bank| {
                crate::system_registry::blaster_bank_system_id(&bank.config.id)
                    .is_some_and(|id| control_sources.0.policy_for(&id).operate_ai)
            }),
        };
        if !ai_controlled {
            continue;
        }

        // Target selection: the **Combat Lock** from this ship's frozen
        // viewscreen blackboard (issue #829, spec §1/§3). One-tick lag accepted.
        let Some(target_uuid) = blaster_combat_lock(blackboards_opt) else {
            continue;
        };
        let Some((tx, tz)) = live_entity_xz(&target_uuid, &asteroid_q, &entity_q) else {
            continue;
        };

        // Collect the banks to fire first (immutable read of `blaster_res`),
        // then emit — `emit_ai_command` borrows `admitted` mutably.
        let mut banks_to_fire: Vec<String> = Vec::new();
        for bank in blaster_res.0.iter() {
            // Per-bank fine-system gate — skip banks whose fine system is not
            // AI-operated (offline/human), so one bank firing never depends on
            // another's control source.
            if let Some(bank_sid) = crate::system_registry::blaster_bank_system_id(&bank.config.id)
            {
                if system_is_registered(control_sources, &bank_sid)
                    && !control_sources.0.policy_for(&bank_sid).operate_ai
                {
                    continue;
                }
            }

            // Host readiness gates (AC2): availability/cooldown, range, arc.
            let fire_ready = bank.is_fire_ready();
            let dx = tx - physics.x;
            let dz = tz - physics.z;
            let in_range = dx * dx + dz * dz <= bank.config.range * bank.config.range;
            let (rx, ry) =
                crate::weapons::phaser::ship_local(tx, tz, physics.x, physics.z, physics.yaw);
            let in_arc = crate::weapons::phaser::in_arc(
                rx,
                ry,
                bank.config.facing_deg,
                bank.config.fire_arc_deg,
            );
            if !fire_ready || !in_range || !in_arc {
                continue;
            }

            // Per-bank policy gate (issue #781): the bank is host-ready — now
            // resolve its own authored open-fire policy over a seeded readiness
            // snapshot. An idle bank (or one whose guard holds) is skipped,
            // leaving other banks free to fire (per-bank independence, AC7).
            //
            // A bank with NO entry does not fire: since #885b stage 5d there is
            // no synthesised stand-in, and strict AI-declaration mode rejects a
            // bank that authors no inline `ai` block at load.
            let Some(policy) = bank_policies_opt.and_then(|p| p.0.get(&bank.config.id)) else {
                continue;
            };
            let facts = seed_blaster_bank_facts(true, false, 0.0, in_range, in_arc, posture);
            if blaster_bank_policy_fires(policy, &facts, &flag_chain) {
                banks_to_fire.push(bank.config.id.clone());
            }
        }

        // Emit the SAME typed input a human does for every firing bank
        // (AC5/AC7). `handle_fire_blaster` (Physics) consumes these and
        // dispatches the volley, converging with the human origin.
        for bank_id in banks_to_fire {
            let Some(target) = crate::system_registry::blaster_bank_system_id(&bank_id) else {
                continue;
            };
            crate::command_admission::ai_emit::emit_ai_command(
                entity_uuid,
                target,
                crate::messages::SystemControlPayload::ChargeBlasterStart,
                control_sources,
                &sessions,
                ship_config_opt,
                &mut admitted,
            );
        }
    }
}

/// Tick every ship's `BlasterSystemResource` — advance volley timers and
/// launch projectiles. Emits `ServerMessage::BlasterFired` for each
/// projectile launched. Runs in `SimSet::Physics`.
///
/// # Sanctioned out-of-band `ShipPhysics` writer (issue #699)
///
/// `integrate_ship_physics` is the sole *helm-path* writer of
/// `ShipPhysics.x/z/yaw/forward_speed/lateral_speed/roll`. The recoil impulse
/// (issue #638) accumulates into `forward_speed` directly and is an
/// intentional exception: it is a weapons-fire impulse added on top of
/// whatever the helm integrator produced, not a helm decision. It deliberately
/// does not opt into the debug `HelmPhysicsWriteGuard`. See the writer-policy
/// table on `ShipPhysics` (`src/ship/state.rs`).
///
/// # Why the queries are a [`ParamSet`] (the blaster-less-target fix)
///
/// The intercept solve needs the *target's* velocity, and a target is any ship
/// — not only one that happens to carry blaster banks. The firing query below
/// takes `&mut ShipPhysics` (recoil) and `&mut BlasterSystemResource`, so it
/// only ever yields ships with a blaster bank; building the velocity map from
/// it silently resolved every blaster-less hull to `(0.0, 0.0)` and aimed the
/// gun exactly where the target was standing. That is most of the shipped
/// content — `ship_harrow_patrol` and `alliance_cruiser` author no blaster bank —
/// so the lead vanished against precisely the hulls the artillery shoots at.
///
/// A plain second `Query<(&EntityUuid, &ShipPhysics), With<Ship>>` cannot
/// coexist with the firing query's `&mut ShipPhysics`: the two overlap and
/// Bevy panics on conflicting access. `ParamSet` is the mechanism for exactly
/// this — it makes the overlap explicit and lets only one half be borrowed at
/// a time, which suits a pass that reads every ship's velocity ONCE up front
/// and then never touches the read half again. The alternatives were worse: a
/// `Without<BlasterSystemResource>` split relies on the archetypes staying
/// exactly complementary and needs the map merged from two sources, and
/// dropping `&mut ShipPhysics` would take recoil with it.
pub(crate) fn tick_blaster_system(
    time: Res<Time>,
    mut ship_qs: ParamSet<(
        // p0 — the firing pass: every ship that carries blaster banks.
        Query<
            (
                Option<&crate::entity_spawner::EntityUuid>,
                &Transform,
                Option<&ModelMarkers>,
                &mut ShipPhysics,
                Option<&crate::server_app::ShipSystemBlackboards>,
                &mut BlasterSystemResource,
            ),
            With<crate::server_app::Ship>,
        >,
        // p1 — the velocity pass: EVERY ship, blaster-carrying or not. A ship
        // with no `EntityUuid` can never be a combat lock, so it is filtered
        // by the query rather than skipped in the body.
        Query<(&crate::entity_spawner::EntityUuid, &ShipPhysics), With<crate::server_app::Ship>>,
    )>,
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut outbox: ResMut<SimOutbox>,
    #[cfg(feature = "server")] mut shake_state: Option<
        ResMut<crate::server::viewscreen_border::ShakeState>,
    >,
    // `Option<ResMut<Messages<_>>>` so bare-`App` fixtures that never
    // registered the message still pass Bevy's parameter validation.
    mut balance_events: Option<ResMut<bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
    // Projectile ids are minted from the tick-scoped counter (issue #907): an
    // id that is a function of RNG draw order is stable within one seeded
    // instance but not across two. `Option<Res<_>>` for the same reason as
    // every other determinism resource — a bare `Res` fails parameter
    // validation in the bare-`App` weapons fixtures.
    id_mint: Option<Res<crate::world_id::WorldIdMint>>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();

    // Pre-compute world-space velocity for EVERY ship (see the ParamSet note
    // on this fn) so the per-ship loop below can look up the target's velocity
    // for intercept prediction. Read through `p1` and finished with before the
    // firing pass borrows `p0`.
    let ship_velocities: std::collections::HashMap<String, (f32, f32)> = ship_qs
        .p1()
        .iter()
        .map(|(uuid, physics)| {
            let vx = physics.forward_speed * simmath::sin(physics.yaw);
            let vz = -physics.forward_speed * simmath::cos(physics.yaw);
            (uuid.0.clone(), (vx, vz))
        })
        .collect();

    let mut ship_q = ship_qs.p0();
    for (source_uuid_opt, transform, markers_opt, mut physics, blackboards_opt, mut blaster_res) in
        ship_q.iter_mut()
    {
        let source_uuid = source_uuid_opt
            .map(|u| u.0.as_str())
            .unwrap_or("")
            .to_string();

        // Resolve target position and velocity from the frozen Combat Lock
        // (issue #829). Ships supply world-space velocity from ShipPhysics;
        // asteroids/objects are stationary (velocity = 0).
        let target_uuid = blaster_combat_lock(blackboards_opt);
        let (target_x, target_z, target_vx, target_vz) = if let Some(ref uuid) = target_uuid {
            let pos = live_entity_xz(uuid, &asteroid_q, &entity_q);
            let vel = ship_velocities.get(uuid);
            let (vx, vz) = vel.copied().unwrap_or((0.0, 0.0));
            pos.map(|(x, z)| (x, z, vx, vz))
                .unwrap_or((physics.x, physics.z - 100.0, 0.0, 0.0))
        } else {
            let fwd_x = simmath::sin(physics.yaw);
            let fwd_z = -simmath::cos(physics.yaw);
            (
                physics.x + fwd_x * 100.0,
                physics.z + fwd_z * 100.0,
                0.0,
                0.0,
            )
        };

        for bank in blaster_res.0.iter_mut() {
            let bank_id = bank.config.id.clone();
            let visual_scale = bank.config.visual_scale;
            let recoil_impulse = bank.config.recoil_impulse;
            let screenshake_magnitude = bank.config.screenshake_magnitude;

            // Resolve one world-space origin per authored barrel marker (issue
            // #765). A bank with no barrels authored has one implicit barrel =
            // the bank's single `marker`. Each name resolves via the rig, and
            // falls back to ship centre when the bank has no marker or the
            // sidecar doesn't declare it.
            let barrel_names: Vec<Option<String>> = if bank.config.barrels.is_empty() {
                vec![bank.config.marker.clone()]
            } else {
                bank.config.barrels.iter().cloned().map(Some).collect()
            };
            let barrel_origins: Vec<(f32, f32)> = barrel_names
                .iter()
                .map(|name| {
                    name.as_deref()
                        .and_then(|n| {
                            markers_opt.and_then(|m| m.resolve_world_position(transform, n))
                        })
                        .map(|pos| (pos.x, pos.z))
                        .unwrap_or((physics.x, physics.z))
                })
                .collect();

            let events = bank.tick(
                dt,
                &barrel_origins,
                physics.yaw,
                target_x,
                target_z,
                target_vx,
                target_vz,
                &source_uuid,
                // Was OS randomness, then a draw off the seeded RNG stream —
                // both of which gave two instances different ids for the same
                // shot, and those ids key the in-flight/hit bookkeeping. Now a
                // tick-scoped mint (issue #907).
                &mut || {
                    crate::world_id::mint_id_with(
                        id_mint.as_deref(),
                        crate::world_id::IdNamespace::Projectile,
                    )
                },
            );
            for ev in &events {
                // ── Recoil impulse (issue #638) ─────────────────────────────
                // Apply an instantaneous velocity impulse to the firing ship
                // in the direction opposite to the projectile's heading.
                // The physics model is 1D (forward_speed along ship axis), so
                // we project the impulse onto the ship's forward axis and
                // accumulate it into forward_speed. The opposite-to-fire
                // convention is: impulse_dir = heading + π.
                if recoil_impulse > 0.0 {
                    // Ship forward direction in world space: (sin(yaw), -cos(yaw)).
                    // Projectile direction: (sin(heading), -cos(heading)).
                    // Recoil direction = opposite to projectile = -projectile.
                    // Projection of recoil onto ship forward:
                    //   dot((−sin(h), cos(h)), (sin(yaw), −cos(yaw)))
                    //   = −sin(h)·sin(yaw) + cos(h)·(−cos(yaw))
                    //   = −(sin(h)·sin(yaw) + cos(h)·cos(yaw))
                    //   = −cos(h − yaw)
                    let heading = ev.heading;
                    let yaw = physics.yaw;
                    let projection = -simmath::cos(heading - yaw);
                    physics.forward_speed += projection * recoil_impulse;
                }

                // ── Screenshake (issue #638) ─────────────────────────────────
                // Push a synthetic entry into the rolling shake window.
                // The shake system sums hull_damage in the window; we scale
                // screenshake_magnitude so that 1.0 produces a noticeable
                // single-shot kick. The formula:
                //   magnitude = (total_hull / 30.0).min(1.0) * SHAKE_MAX_MAGNITUDE
                // So pushing hull_damage = screenshake_magnitude * 30.0 maps
                // 1.0 → full shake, 0.5 → half shake, etc.
                #[cfg(feature = "server")]
                if screenshake_magnitude > 0.0 {
                    if let Some(ref mut shake) = shake_state {
                        let hull_equiv = screenshake_magnitude * 30.0;
                        shake.entries.push((now, hull_equiv));
                    }
                }

                // Balance tracer: the blaster bolt left the ship. Unconditional
                // — all ships, all builds. Blank uuid → `None`.
                if let Some(ref mut msgs) = balance_events {
                    msgs.write(crate::balance::BalanceEvent::WeaponFired {
                        shooter: Some(source_uuid.clone()).filter(|u| !u.is_empty()),
                        weapon: bank_id.clone(),
                        kind: crate::balance::FIRED_KIND_BLASTER.to_string(),
                    });
                }
                outbox.0.push((
                    Target::All,
                    ServerMessage::BlasterFired {
                        bank: bank_id.clone(),
                        source_uuid: source_uuid.clone(),
                        projectile_id: ev.projectile_id.clone(),
                        x: ev.x,
                        z: ev.z,
                        heading: ev.heading,
                        visual_scale,
                    },
                ));
            }
        }
    }
}

/// Check blaster projectile hits against all entities and apply shields-first
/// damage. Emits `ServerMessage::BlasterHit`. Runs in `SimSet::Damage`.
///
/// Hit detection uses live ECS Transform positions — the same approach as
/// `build_torpedo_target_snapshot`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_blaster_hits(
    asteroid_q: Query<(&AsteroidUuid, &Transform), Without<crate::entity_spawner::EntityUuid>>,
    entity_q: Query<(&crate::entity_spawner::EntityUuid, &Transform), Without<AsteroidUuid>>,
    mut hit_target_q: Query<(
        Entity,
        Option<&AsteroidUuid>,
        Option<&crate::entity_spawner::EntityUuid>,
        &mut crate::entity_spawner::EntitySystemHull,
        Option<&mut crate::ship::shields::ShipShields>,
        Option<&mut crate::entity_spawner::EntityShipArcHull>,
        bevy::ecs::query::Has<crate::server_app::LocalShip>,
    )>,
    // `Entity` + `EntityUuid` ride along so the shooter walk below can be
    // sorted into a stable order (issue #1052) rather than taken in archetype
    // order; `EntityUuid` is `Option` because minimal test-only ship spawns
    // omit it, exactly as the beam and collision paths allow.
    mut blaster_res_q: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &mut BlasterSystemResource,
        ),
        With<crate::server_app::Ship>,
    >,
    mut outbox: ResMut<SimOutbox>,
    mut commands: Commands,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<GameOverReason>>,
    mut world: ResMut<WorldResource>,
    mut destroyed_events: MessageWriter<crate::ai_plugin::AiEntityDestroyed>,
    mut vfx_events: MessageWriter<AsteroidDestroyedVfx>,
    mut ship_vfx_events: MessageWriter<ShipDestroyedVfx>,
    collider_q: Query<&crate::entity_spawner::ColliderSection>,
    // `Option<ResMut<Messages<_>>>` so bare-`App` fixtures that never
    // registered the message still pass Bevy's parameter validation.
    mut balance_events: Option<ResMut<Messages<crate::balance::BalanceEvent>>>,
    // See `tick_beams_apply_damage` (issue #838): forget the killed uuid from
    // the registry so the reconcile sweep does not re-emit `EntityDespawned`.
    mut tracked: Option<ResMut<crate::server_app::TrackedEntities>>,
    // Seeded RNG + log filter + God Mode (issue #900), bundled: separately
    // they put this system one over Bevy's 16-parameter ceiling.
    ambient: crate::server_app::SimRngAndLog,
) {
    let sim_rng = &ambient.rng;
    let log = &ambient.log;
    let god_mode = ambient.god_mode_active();
    // Build target list from live ECS transforms.
    let mut targets: Vec<(String, f32, f32, f32)> = Vec::new();
    for (ast_uuid, transform) in asteroid_q.iter() {
        targets.push((
            ast_uuid.0.clone(),
            transform.translation.x,
            transform.translation.z,
            0.0,
        ));
    }
    for (ent_uuid, transform) in entity_q.iter() {
        targets.push((
            ent_uuid.0.clone(),
            transform.translation.x,
            transform.translation.z,
            0.0,
        ));
    }
    // Sorted by uuid before it is used (issue #1052), for the same reason the
    // torpedo path already sorts its own proximity list: `find_hits` takes the
    // FIRST target a projectile overlaps and stops, so this list's order is
    // what decides which of two overlapping bodies a bolt hits. Built from two
    // queries walked in archetype order, it was otherwise a function of how the
    // world happened to spawn.
    targets.sort_by(|a, b| a.0.cmp(&b.0));

    #[derive(Clone)]
    struct BlasterDetonation {
        bank_id: String,
        projectile_id: String,
        target_uuid: String,
        damage: i32,
        shield_pierce: f32,
        /// Who fired it — needed for balance attribution, since the firing
        /// ship is long out of scope by the time damage is applied. `None`
        /// when the shooter had no `EntityUuid`: the projectile carries `""`
        /// for that case, which is "unknown", not a ship named `""`.
        source_uuid: Option<String>,
    }

    // Stable shooter order (issue #1052), the same mechanism
    // `server_app::handle_collisions` has used since #896. The detonations this
    // walk collects are applied — and draw from `SimStream::BlasterDamage` — in
    // the order they were collected, so archetype order decided which bolt
    // consumed which draw and therefore which of the victim's systems absorbed
    // it.
    let mut shooter_order: Vec<((String, bevy::ecs::entity::EntityIndex), Entity)> = blaster_res_q
        .iter()
        .map(|(entity, uuid, _)| {
            (
                (
                    uuid.map(|u| u.0.clone()).unwrap_or_default(),
                    entity.index(),
                ),
                entity,
            )
        })
        .collect();
    shooter_order.sort();

    let mut detonations: Vec<BlasterDetonation> = Vec::new();
    for shooter in shooter_order.into_iter().map(|(_, entity)| entity) {
        let Ok((_, _, mut blaster_res)) = blaster_res_q.get_mut(shooter) else {
            continue;
        };
        for bank in blaster_res.0.iter_mut() {
            let hits = bank.find_hits(&targets);
            for (proj_id, target_uuid) in hits {
                if let Some(hit_data) = bank.consume_hit(&proj_id) {
                    detonations.push(BlasterDetonation {
                        bank_id: bank.config.id.clone(),
                        projectile_id: proj_id,
                        target_uuid,
                        damage: hit_data.damage,
                        shield_pierce: hit_data.shield_pierce,
                        source_uuid: Some(hit_data.source_uuid).filter(|u| !u.is_empty()),
                    });
                }
            }
        }
    }

    for det in detonations {
        outbox.0.push((
            Target::All,
            ServerMessage::BlasterHit {
                bank: det.bank_id.clone(),
                projectile_id: det.projectile_id,
                target_uuid: det.target_uuid.clone(),
            },
        ));

        // Apply shields-first damage to the matching entity.
        for (entity, ast_uuid, ent_uuid, mut hull_comp, mut shield_comp, mut arc_hull, is_local) in
            hit_target_q.iter_mut()
        {
            let uuid_matches = ast_uuid.map(|u| u.0.as_str()) == Some(det.target_uuid.as_str())
                || ent_uuid.map(|u| u.0.as_str()) == Some(det.target_uuid.as_str());
            if !uuid_matches {
                continue;
            }

            // God mode: local ship takes no damage.
            if is_local && god_mode {
                outbox.0.push((
                    Target::All,
                    ServerMessage::DamageTaken {
                        hull: 0.0,
                        shield: 0.0,
                    },
                ));
                break;
            }

            let mut hull_damage = det.damage as f32;

            // Snapshot online facings before the shield apply so the
            // online→offline edge can be reported (issue #841).
            let arcs_online_before: Vec<(String, bool)> = shield_comp
                .as_ref()
                .map(|s| {
                    s.0.facings
                        .iter()
                        .map(|f| (f.id.clone(), f.is_online()))
                        .collect()
                })
                .unwrap_or_default();

            let shield_amount = if let Some(ref mut shields) = shield_comp {
                let all_offline = shields.0.facings.iter().all(|f| !f.is_online());
                if !all_offline {
                    let (pierced, absorbed) = crate::damage::split_damage_for_pierce(
                        det.damage as f32,
                        det.shield_pierce,
                    );
                    let leak = shields.0.apply_damage(absorbed.round() as i32, 0.0);
                    let shielded = (absorbed - leak as f32).max(0.0);
                    hull_damage = pierced + leak as f32;
                    shielded
                } else {
                    0.0
                }
            } else {
                0.0
            };

            // Balance tracer, emitted for every hit on every ship — including
            // one the shields ate whole, which never reaches the branch below.
            let mut hull_applied_total = 0.0f32;
            // Hoisted out of the branch below so the destroyed-by log line can
            // be written once, next to the per-hit line, rather than twice
            // inside the local/non-local arms.
            let mut ship_destroyed = false;
            if hull_damage > 0.0 {
                let (hull_applied, destroyed) = crate::sim_rng::with_stream(
                    sim_rng.as_deref(),
                    crate::sim_rng::SimStream::BlasterDamage,
                    |rng| {
                        let result =
                            crate::damage::apply_hull_damage(&mut hull_comp.0, hull_damage, rng);
                        if let Some(ref mut ah) = arc_hull {
                            ah.0.apply_damage(result.0, rng);
                        }
                        result
                    },
                );
                hull_applied_total = hull_applied;
                ship_destroyed = destroyed;
                if is_local {
                    outbox.0.push((
                        Target::All,
                        ServerMessage::DamageTaken {
                            hull: hull_applied,
                            shield: shield_amount,
                        },
                    ));
                    if destroyed {
                        outbox.0.push((Target::All, ServerMessage::ShipDestroyed));
                        if let Some(ref mut ns) = next_state {
                            ns.set(GamePhase::GameOver);
                        }
                        if let Some(ref mut reason) = game_over_reason {
                            if reason.0.is_none() {
                                reason.0 = Some("server.game_over.ship_destroyed".into());
                                // The LocalShip died → defeat (#843), latched
                                // under the same first-write guard as the reason.
                                reason.1 = Some(crate::balance::Outcome::Defeat);
                                // EntityDestroyed for the player death, once
                                // (guarded by the first reason write). Killer =
                                // the blaster's shooter (issue #841). Shares the
                                // `GameOverReason` latch with a scenario's
                                // `SetGameOverReason`; see the beam death site
                                // for why that coupling is accepted.
                                if let Some(ref mut msgs) = balance_events {
                                    msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                                        victim: det.target_uuid.clone(),
                                        killer: det.source_uuid.clone(),
                                    });
                                }
                            }
                        }
                    }
                } else if destroyed {
                    commands.entity(entity).try_despawn();
                    // Historically silent for non-local targets — neither the
                    // asteroid ripple nor a client despawn broadcast fired for
                    // a blaster kill. Bring it in line with the phaser/torpedo
                    // paths so every weapon type destroys entities the same
                    // way.
                    let is_asteroid = ast_uuid.is_some();
                    let (hit_x, hit_z) = targets
                        .iter()
                        .find(|(u, ..)| u == &det.target_uuid)
                        .map(|(_, x, z, _)| (*x, *z))
                        .unwrap_or((0.0, 0.0));
                    world.0.entities.retain(|a| a.uuid != det.target_uuid);
                    if is_asteroid {
                        vfx_events.write(AsteroidDestroyedVfx { x: hit_x, z: hit_z });
                        outbox.0.push((
                            Target::All,
                            ServerMessage::AsteroidDestroyed {
                                uuid: det.target_uuid.clone(),
                            },
                        ));
                    } else {
                        destroyed_events.write(crate::ai_plugin::AiEntityDestroyed {
                            entity_uuid: det.target_uuid.clone(),
                        });
                        let radius = collider_q
                            .get(entity)
                            .map(|c| c.0.radius)
                            .unwrap_or(DEFAULT_SHIP_EXPLOSION_RADIUS);
                        ship_vfx_events.write(ShipDestroyedVfx {
                            x: hit_x,
                            z: hit_z,
                            radius,
                        });
                        outbox.0.push((
                            Target::All,
                            ServerMessage::EntityDespawned {
                                uuid: det.target_uuid.clone(),
                            },
                        ));
                        if let Some(t) = tracked.as_mut() {
                            t.forget(&det.target_uuid);
                        }
                        // EntityDestroyed for the blaster kill, co-located with
                        // the AiEntityDestroyed write (exactly once). Killer =
                        // the firing ship (issue #841).
                        if let Some(ref mut msgs) = balance_events {
                            msgs.write(crate::balance::BalanceEvent::EntityDestroyed {
                                victim: det.target_uuid.clone(),
                                killer: det.source_uuid.clone(),
                            });
                        }
                    }
                }
            }
            // Human-readable logging alongside the structured BalanceEvent
            // (does NOT replace it). Same level discipline as the beam site:
            // per-hit detail is `trace`, and the one `info` edge is
            // destruction — the state change a balancer reads as a headline.
            // Both entity-scoped to the victim so `--log-entity` narrows to
            // one hull.
            let attacker_label: &str = det.source_uuid.as_deref().unwrap_or("unknown");
            crate::ptrace!(
                log,
                crate::logging::LogCat::Damage,
                entity = entity,
                "took {} (shield {:.0}/hull {:.0}) from {} via {}",
                det.damage,
                shield_amount,
                hull_applied_total,
                attacker_label,
                det.bank_id
            );
            if ship_destroyed && ast_uuid.is_none() {
                crate::pinfo!(
                    log,
                    crate::logging::LogCat::Damage,
                    entity = entity,
                    "destroyed by {}",
                    attacker_label
                );
            }

            if let Some(ref mut msgs) = balance_events {
                msgs.write(crate::balance::BalanceEvent::DamageApplied {
                    attacker: det.source_uuid.clone(),
                    victim: det.target_uuid.clone(),
                    victim_kind: if ast_uuid.is_some() {
                        crate::balance::VictimKind::Asteroid
                    } else {
                        crate::balance::VictimKind::Ship
                    },
                    weapon: det.bank_id.clone(),
                    amount: det.damage as f32,
                    shield_absorbed: shield_amount,
                    hull_damage: hull_applied_total,
                    system_hit: None,
                });
                if ast_uuid.is_none() {
                    if let Some(ref shields) = shield_comp {
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
                                    ship: det.target_uuid.clone(),
                                    arc_id: id.clone(),
                                });
                            }
                        }
                    }
                }
            }
            break; // UUID is unique.
        }
    }
}

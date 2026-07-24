//! Server-side console AI orchestrator.
//!
//! Complexity preset machinery (ComplexityRules, ConsoleComplexityState,
//! build_complexity_rules, track_complexity_changes) removed in B4 (issue #534).
//! AI behaviour is now gated by StationRatingConfig.ai_tuning.
//!
//! Issue #692 wires the previously-orphaned pure decision functions from
//! `console_ai::core` into Bevy systems here:
//! - `ai_shield_focus` — replaces the old fused `operate_shields_ai`
//!   (formerly in `ship::shields`). Originally paired with an
//!   `integrate_shield_state` adapter draining a `ShieldArcIntents`
//!   component; issue #826 retired that pair — the decide system now emits
//!   admitted `SetShieldArcFocus` payloads via
//!   `command_admission::validate_and_admit` and the human path's
//!   `ship::shields::handle_shields_messages` applies them same-tick.
//! - `ai_frequency_hint` — wires `console_ai::tick_frequency_hint`, which had
//!   no caller anywhere prior to this issue.
//!
//! Issue #694 (preliminary) added `ai_torpedo_auto_fire` /
//! `integrate_torpedo_intents`, replacing the old fused torpedo sub-block
//! that used to live inside `console::weapons::operate_tactical_ai`
//! with the same decide/`TorpedoIntents`-write + mutate-only-adapter shape.
//! `operate_tactical_ai` kept running for Tactical target selection only.
//!
//! Issue #698 completes that work: `ai_torpedo_auto_fire` is no longer
//! preliminary (it reads real target-lock and target-shield state instead of
//! hardcoding them), and `integrate_torpedo_intents` is gone — its body moved
//! into `console::weapons::integrate_weapons_state`, the single
//! adapter that drains both `TorpedoIntents` and `PhaserIntents`.
//!
//! Issue #700 finished the decomposition: `operate_tactical_ai` is gone
//! entirely, and target selection now lives wholly in
//! `console::weapons::ai_target_selection`.

use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::console_ai::shields_emit::emit_shields_ai_command;

// AI rule keys — match the keys used in [[station.rating]].ai_tuning tables.
pub const AI_RULE_TORPEDO_AUTO_FIRE: &str = "torpedo_auto_fire";
pub const AI_RULE_FREQUENCY_MATCH: &str = "frequency_match";
/// Matches `[[station.rating]].ai_tuning.auto_hint` for the Sensors station.
/// Consulted by `ai_frequency_hint` via a per-ship claimed/unclaimed split
/// mirroring `AI_RULE_TORPEDO_AUTO_FIRE`'s use in `ai_torpedo_auto_fire`:
/// `ShipConfig::sensors_station()` resolves the owning station per-ship (no
/// global "Tactical" assumption), so a claimed NPC never sees a human
/// session's rating and an unclaimed ship (every NPC, and the player ship
/// before anyone takes Sensors) hints unconditionally.
pub const AI_RULE_AUTO_HINT: &str = "auto_hint";
pub const AI_RULE_MOVEMENT_RULE: &str = "movement_rule";
pub const AI_RULE_RED_ALERT_RULE: &str = "red_alert_rule";

/// Per-ship persistent state for `ai_frequency_hint`'s delayed-hint timer.
/// Bevy-facing wrapper around `console_ai::FrequencyHintState`.
///
/// Present only while the ship carries `AiHighFidelity` — bundled alongside
/// that marker at every spawn/promote site (mirrors `ShipPowerAiState`'s
/// scoping; see `ai::server::lod_ai_ships` and the `AiHighFidelity` spawn
/// sites in `server_app.rs` / `ship_plugin.rs` / `ai/server.rs`).
#[derive(Component, Default, Clone, Debug)]
pub struct ShipFrequencyHintState(pub crate::console_ai::FrequencyHintState);

/// Console AI orchestrator plugin.
pub struct ConsoleAiPlugin;

impl Plugin for ConsoleAiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                // Decide only (issue #826): emits admitted SetShieldArcFocus
                // payloads; the single applier is `ship::shields::
                // handle_shields_messages` (registered by ShipShieldsPlugin
                // in this same Physics set). The `.before` edge is the one
                // explicit ordering between them — admission clears
                // AdmittedCommands before Input each tick, so the applier
                // must consume same-tick after this emit.
                ai_shield_focus
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel)
                    .before(crate::ship::shields::handle_shields_messages),
                // Decide only (issue #831, mirroring shields #826): emits
                // admitted SetPowerGroupAllocation payloads; the single applier
                // is `ship::power::handle_power_messages` (registered by
                // ShipPowerPlugin in this same Physics set). The `.before` edge
                // is the one explicit ordering between them — admission clears
                // AdmittedCommands before Input each tick, so the applier must
                // consume same-tick after this emit.
                ai_power_allocation
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel)
                    .before(crate::ship::power::handle_power_messages),
                // Decide only. The apply half is
                // `console::weapons::integrate_weapons_state`
                // (issue #698), which drains `TorpedoIntents` and
                // `PhaserIntents` together and is registered by
                // `WeaponsPlugin` — see its docs for why the weapons
                // integrator does not live here.
                ai_torpedo_auto_fire
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel),
                // The loading half of the torpedo AI. `Input`, not `Physics`,
                // and explicitly before the volley-target handler: the command
                // it emits has to be consumed in the SAME tick, exactly as
                // `operate_captain_ai` is ordered before
                // `handle_set_red_alert`. That also puts `target_count` in
                // place before `tick_torpedo_lifecycle` (Physics) runs its
                // auto-load block, so a tube starts loading the tick the order
                // is given rather than the tick after.
                ai_torpedo_load
                    .in_set(crate::sim_sets::SimSet::Input)
                    .before(crate::weapons_plugin::handle_set_torpedo_volley_target),
                ai_frequency_hint.in_set(crate::sim_sets::SimSet::Input),
            ),
        );
    }
}

// ── Shields AI ───────────────────────────────────────────────────────────────

// The shield-focus AI's private `emit_shield_ai_command` (issue #826) is gone:
// it was one of seven byte-identical copies, all now routed through the shared
// `command_admission::ai_emit::emit_ai_command` seam (issue #738).

/// AI shield-focus decision system (issue #692).
///
/// Replaces the old fused `ship::shields::operate_shields_ai`: reads each
/// AI-controlled ship's shield facings + damage history and emits the
/// decision as an admitted `SetShieldArcFocus` payload into the ship's own
/// `AdmittedCommands` (issue #826 — previously a `ShieldArcIntents` write
/// drained by a paired `integrate_shield_state` adapter), for
/// `ship::shields::handle_shields_messages` to apply later this tick,
/// rather than mutating `ShipShields` directly.
///
/// # Emission shape
/// `Focus(idx)` → `SetShieldArcFocus { focused: true }` targeted at that
/// arc's `shield-arc-<id>` SystemId; `ClearFocus` →
/// `SetShieldArcFocus { focused: false }` targeted at the CURRENTLY focused
/// arc's SystemId (matching `handle_shields_messages`' clear-only-if-target-
/// matches-current semantics; no focus held means nothing to emit).
///
/// # Gating
/// - `ShipSystemControlSources.policy_for(shields_system_id()).operate_ai`
///   (unchanged from the old `operate_shields_ai` gate).
/// - `AiHighFidelity` (new constraint vs. the old system — query filter).
///   Low-LOD NPCs no longer run shield-focus AI; they retain whatever focus
///   they last had when demoted.
///
/// # Threat-bearing override
/// Preserved unchanged: if Sensors has delivered a bearing via
/// `PendingShieldsThreatBearing` (channel-3 coordination), it takes priority
/// over the damage/health decision — the closest facing to the bearing is
/// focused and the damage-based path is skipped for the tick.
///
/// # Damage tracking + decision
/// Same algorithm as the old `operate_shields_ai`: compares each arc's
/// current HP against `ShieldsDamageHistory`, records deltas, prunes outside
/// the configured window, then calls the pure `console_ai::tick_shield_focus_ai`.
///
/// # Data source choice
/// Facing/HP data is read from the live `ShipShields` component rather than
/// the published `ShieldsBlackboard`/`ShieldArcBlackboard`: those blackboards
/// are only written in `SimSet::Publish`, which runs *after* this system
/// (`SimSet::Physics`), so reading them here would see last tick's state.
///
/// `WorldSnapshot` is consulted as a narrow safety guard: if this ship's own
/// entry reports `hull_fraction <= 0.0` (destroyed this tick, before the hull
/// broadcast catches up), the decision is skipped. `AiWorldEntity.shields`
/// remains the unpopulated placeholder `build_world_snapshot` always writes
/// (`None`) — not used here, since the live component is fresher anyway.
pub(crate) fn ai_shield_focus(
    time: Res<Time>,
    world_snapshot: Res<crate::ai_plugin::WorldSnapshot>,
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship::shields::ShipShields,
            &mut crate::ship::shields::ShieldsDamageHistory,
            Option<&crate::ship::shields::ShieldsAiConfigResource>,
            &mut crate::ship::shields::PendingShieldsThreatBearing,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
        ),
        (
            With<crate::ai_plugin::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
) {
    let current_time = time.elapsed_secs();

    for (
        entity_uuid,
        control_sources,
        shields,
        mut damage_history,
        ai_config_comp,
        mut pending_threat,
        ship_config,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Emit `Focus(idx)`: admitted `focused: true` at that arc's SystemId.
        // Arcs with an empty authored id have no fine SystemId and cannot be
        // addressed (they never occur in authored content — `ShieldSystem`
        // defaults every arc id).
        let emit_focus = |idx: usize,
                          admitted: &mut crate::messages::AdmittedCommands,
                          shields: &crate::ship::shields::ShipShields| {
            let Some(sid) = shields
                .0
                .facings
                .get(idx)
                .and_then(|f| crate::system_registry::shield_arc_system_id(&f.id))
            else {
                return;
            };
            emit_shields_ai_command(
                entity_uuid,
                sid,
                crate::messages::SystemControlPayload::SetShieldArcFocus { focused: true },
                control_sources,
                &sessions,
                ship_config,
                admitted,
            );
        };

        let policy = control_sources
            .0
            .policy_for(&crate::system_registry::shields_system_id());
        if !policy.operate_ai {
            continue;
        }

        // Safety guard: skip a ship the world snapshot already reports as
        // destroyed this tick (the hull-integrity broadcast may lag by one
        // publish cycle behind the live component state).
        let already_destroyed = entity_uuid
            .and_then(|u| uuid::Uuid::parse_str(&u.0).ok())
            .and_then(|uuid| world_snapshot.entities.iter().find(|e| e.uuid == uuid))
            .and_then(|e| e.hull_fraction)
            .is_some_and(|frac| frac <= 0.0);
        if already_destroyed {
            continue;
        }

        // ── Threat-bearing override ─────────────────────────────────────────
        // If Sensors has sent a threat bearing, override the normal damage-based
        // focus to rotate/raise the closest facing toward the incoming threat.
        if let Some(bearing_rad) = pending_threat.0.take() {
            let bearing_deg = (bearing_rad.to_degrees() + 360.0) % 360.0;
            let closest_idx = (0..shields.0.facings.len()).min_by(|&a, &b| {
                let da = crate::ship::shields::angular_distance_deg(
                    shields.0.facings[a].center_deg,
                    bearing_deg,
                );
                let db = crate::ship::shields::angular_distance_deg(
                    shields.0.facings[b].center_deg,
                    bearing_deg,
                );
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            });
            if let Some(idx) = closest_idx {
                emit_focus(idx, &mut admitted, shields);
            }
            continue; // Threat bearing takes priority over damage analysis
        }

        // Per-ship tuning only (issue #738). This used to fall back to the
        // global `ShieldsAiConfigResource` Resource, which `server_app` writes
        // from the PLAYER ship's `[shields_console.ai]` TOML — so an NPC with
        // no `[shields_console.ai]` section silently inherited the player
        // ship's shield-AI tuning. The fallback is now the serde-side default
        // (the same values `ShieldsAiConfigResource::default()` supplies while
        // parsing a ship TOML that omits the section).
        let default_ai_cfg;
        let ai_cfg: &crate::ship::shields::ShieldsAiConfigResource = match ai_config_comp {
            Some(c) => c,
            None => {
                default_ai_cfg = crate::ship::shields::ShieldsAiConfigResource::default();
                &default_ai_cfg
            }
        };
        let facings = &shields.0.facings;

        // Single-arc ships have nothing to focus.
        if facings.len() < 2 {
            continue;
        }

        // Lazily resize damage history to match arc count.
        damage_history.ensure_len(facings.len());

        // ── Detect damage: compare current HP vs last-observed HP ───────────
        for (idx, facing) in facings.iter().enumerate() {
            let prev_hp = damage_history.last_observed_hp(idx, facing.hp);

            // Detect a decrease in HP (damage taken) while the arc was online.
            // If the arc went offline the HP dropped to 0 but offline_remaining
            // is set, which shows as a big jump in offline_remaining — we still
            // want to record that as damage to the arc.
            if facing.hp < prev_hp {
                // Focus-decay guard (issue #747): while some OTHER arc is
                // focused, a non-focused arc sitting above its reduced
                // effective max_hp bleeds toward that cap at
                // `focus_config.decay_rate` (see `ShieldSystem::tick`). That HP
                // drop is a focus side effect, not real incoming fire, so it
                // must not be recorded as concentrated damage. A drop that
                // leaves the arc at or above its (reduced) max_hp on a
                // non-focused arc is decay bleeding toward the cap; real damage
                // pushes an arc below its cap and is still recorded (from the
                // full prev→current delta).
                let decay_only = !facing.is_focused && facing.hp >= facing.max_hp;
                if !decay_only {
                    let delta = prev_hp - facing.hp;
                    damage_history.record_damage(idx, current_time, delta);
                }
            }
            damage_history.observe_hp(idx, facing.hp);
        }

        // Prune records outside the damage window.
        damage_history.prune_old(current_time, ai_cfg.damage_window_secs);

        // ── Build AI input ──────────────────────────────────────────────────
        let facings_snapshot: Vec<_> = facings.iter().map(|f| f.snapshot()).collect();
        let shields_is_low = true; // Rating gate deferred to per-ship AiTuning

        let input = crate::console_ai::ShieldFocusAiInput {
            facings: facings_snapshot,
            shields_is_low,
            damage_history: damage_history.arcs.clone(),
            damage_window_secs: ai_cfg.damage_window_secs,
            min_damage_window_secs: ai_cfg.min_damage_window_secs,
            damage_pct_threshold: ai_cfg.damage_pct_threshold,
            health_ratio_threshold: ai_cfg.health_ratio_threshold,
            current_time_secs: current_time,
        };

        let decision = crate::console_ai::tick_shield_focus_ai(&input);

        match decision {
            crate::console_ai::ShieldFocusAiOutput::Focus { facing_index } => {
                if facing_index < facings.len() {
                    emit_focus(facing_index, &mut admitted, shields);
                }
            }
            crate::console_ai::ShieldFocusAiOutput::ClearFocus => {
                // Target the CURRENTLY focused arc: `handle_shields_messages`
                // only clears when `focused: false` names the arc that holds
                // the focus. No focus held → nothing to clear (the old
                // `set_focused_facing(None)` was a no-op there too).
                let current_sid = shields.0.focused_facing.and_then(|i| {
                    shields
                        .0
                        .facings
                        .get(i)
                        .and_then(|f| crate::system_registry::shield_arc_system_id(&f.id))
                });
                if let Some(sid) = current_sid {
                    emit_shields_ai_command(
                        entity_uuid,
                        sid,
                        crate::messages::SystemControlPayload::SetShieldArcFocus { focused: false },
                        control_sources,
                        &sessions,
                        ship_config,
                        &mut admitted,
                    );
                }
            }
            crate::console_ai::ShieldFocusAiOutput::None => {}
        }
    }
}

// `integrate_shield_state` (issue #692's mutate-only adapter) was deleted by
// issue #826: truth-integration for shields lives in
// `ship::shields::handle_shields_messages`, the single admitted-command
// applier for human and AI commands alike.

// ── Power AI ─────────────────────────────────────────────────────────────────

// `emit_power_ai_command` (issue #831) likewise collapsed into the shared
// `command_admission::ai_emit::emit_ai_command` seam (issue #738).

/// AI power-allocation decision system (issue #693, admitted transport #831).
///
/// Wires the previously-orphaned `console_ai::tick_power_movement_rule` and
/// `console_ai::tick_power_red_alert_rule` pure functions: sustained thrust
/// nudges the helm power group +1 (and drops it -1 once battery/thrust
/// conditions lapse); sustained red alert does the same to the weapons power
/// group. Emits the decision as an admitted `SetPowerGroupAllocation` payload
/// targeted at `POWER_REACTOR_SYSTEM_ID` (issue #831 — previously a
/// `PowerReactorIntents` write drained by a paired `integrate_power_state`
/// adapter), for `ship::power::handle_power_messages` to apply later this tick,
/// rather than mutating `ShipPowerSystem` directly.
///
/// Replaces the old fused `ship::power::operate_power_ai` (absolute-set,
/// non-timer, non-`AiHighFidelity`-gated) entirely — see that module's
/// history note above `tick_power_brownout_advisory`.
///
/// # Gating
/// - `ShipSystemControlSources.policy_for(power_reactor_system_id()).operate_ai`
///   (unchanged from the old `operate_power_ai` gate).
/// - `AiHighFidelity` (new constraint vs. the old system — query filter).
///
/// The old `operate_power_ai` additionally checked
/// `sessions.holder_for_station(&StationId("power"))` to yield to a human
/// Power console holder on the player ship. `ai_shield_focus` (issue #692)
/// has no equivalent sessions-based check and relies solely on the
/// `operate_ai` policy gate — a human claiming a station flips that
/// station's `operate_ai` policy to false via the station-claim path, so the
/// policy gate alone already represents "no human is occupying this
/// station". The redundant sessions check is dropped here for consistency
/// with the #692 precedent.
///
/// # ±1 level semantics
/// `PowerEngageOutput::Engage` adds +1 to the target group (helm for the
/// movement rule, weapons for the red-alert rule); `Disengage` subtracts 1;
/// `NoChange` emits nothing. The target level is clamped to `[1, 4]` (the same
/// per-group range `PowerSystem::set_group_allocation` enforces), and a command
/// is emitted **only when that clamped level differs from the current
/// allocation** — a saturated Engage at level 4 (or Disengage at level 1) is a
/// no-op and is skipped so admission is not spammed every tick. The applier
/// (`handle_power_messages`) additionally re-clamps and enforces the total<=8
/// cap, so the emitted value is safe regardless.
///
/// # Data sources
/// - `red_alert` from `ShipRedAlert`, default `false` if absent.
/// - `thrust` from `LastHelmInput.thrust`, default `0.0` if absent.
/// - `battery_pct` from `power.0.battery_charge / cfg.capacity * 100.0` (the
///   pure functions expect a 0.0-100.0 percentage, not a 0.0-1.0 fraction —
///   confirmed by `console_ai::core`'s own test defaults, e.g.
///   `battery_recharge_pct: 100.0`).
/// - `power_is_low` is hardcoded `true`, mirroring `ai_shield_focus`'s
///   `shields_is_low` precedent — a retired Low/Full complexity model, not
///   wired up now.
/// - `dt` from `Time::delta_secs()`.
fn ai_power_allocation(
    time: Res<Time>,
    sessions: Res<crate::lobby::Sessions>,
    // `Option<Res<_>>`, never bare — this system runs in bare-`App` fixtures
    // that never insert `LogFilterConfig` (see the logging macro docs). The
    // global `PowerConfigResource`/`PowerAiConfigResource` reads that used to
    // sit alongside it are gone: issue #738 made tuning per-entity.
    log: Option<Res<crate::logging::LogFilterConfig>>,
    mut ships: Query<
        (
            // `Entity` for the log filter's entity scoping, `EntityUuid` for
            // the `ai:` token `emit_ai_command` builds — both, not either.
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship::power::ShipPowerSystem,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::ship_plugin::LastHelmInput>,
            Option<&crate::ship::power::PowerConfigResource>,
            Option<&crate::ship::power::PowerAiConfigResource>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::ship::power::ShipPowerAiState,
            &mut crate::messages::AdmittedCommands,
        ),
        (
            With<crate::ai_plugin::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
) {
    let dt = time.delta_secs();

    for (
        ship_entity,
        entity_uuid,
        control_sources,
        power,
        red_alert_comp,
        last_helm_comp,
        cfg_comp,
        ai_cfg_comp,
        ship_config,
        mut ai_state,
        mut admitted,
    ) in ships.iter_mut()
    {
        let policy = control_sources
            .0
            .policy_for(&crate::system_registry::power_reactor_system_id());
        if !policy.operate_ai {
            // Not (or no longer) AI-driven — reset both rules' timers so a
            // later hand-back to AI control doesn't fire an instantly-stale
            // engage/disengage decision.
            *ai_state = crate::ship::power::ShipPowerAiState::default();
            continue;
        }

        // Per-entity config components only (issue #738). These used to fall
        // back to the global `PowerConfigResource` / `PowerAiConfigResource`
        // Resources, which `server_app` writes from the PLAYER ship's `[power]`
        // TOML — so an NPC spawned without those components ran its reactor AI
        // against the player ship's capacity and thresholds. The fallback is
        // now the parse-time default each type already supplies for a ship TOML
        // with no `[power]` block (see `entity_spawner`, which inserts exactly
        // those defaults on every spawned ship).
        let cfg_default;
        let cfg: &crate::ship::power::PowerConfigResource = match cfg_comp {
            Some(c) => c,
            None => {
                cfg_default = crate::ship::power::PowerConfigResource::default();
                &cfg_default
            }
        };
        let ai_cfg_default;
        let ai_cfg: &crate::ship::power::PowerAiConfigResource = match ai_cfg_comp {
            Some(c) => c,
            None => {
                ai_cfg_default = crate::ship::power::PowerAiConfigResource::default();
                &ai_cfg_default
            }
        };

        let red_alert = red_alert_comp.map(|ra| ra.0).unwrap_or(false);
        let thrust = last_helm_comp.map(|l| l.thrust).unwrap_or(0.0);

        let battery_pct = if cfg.0.capacity > 0.0 {
            (power.0.battery_charge / cfg.0.capacity) * 100.0
        } else {
            0.0
        };

        // Emit an admitted `SetPowerGroupAllocation` toward the reactor for a
        // ±1 nudge on `group`, but only when the clamped target level actually
        // differs from the current allocation (skip saturated no-ops so a held
        // Engage/Disengage doesn't spam admission every tick — issue #831).
        //
        // `reason` carries the branch's per-rule reactor logging (PRD #835)
        // into main's emit shape: the log line lives here rather than in the
        // match arms so it reports the *clamped* level actually asked for and
        // stays silent on the skipped no-ops.
        let emit_delta =
            |group: &crate::messages::PowerGroupId,
             delta: i16,
             reason: &str,
             admitted: &mut crate::messages::AdmittedCommands| {
                let current = power.0.level_for(group);
                let target = (current as i16 + delta).clamp(1, 4) as u8;
                if target == current {
                    return;
                }
                // Engage/Disengage are the natural power edges — they only fire
                // after the pure rule's own delay/battery gates lapse, so they
                // are event-driven, not per-tick. `info`, entity-scoped.
                crate::pinfo!(
                    log,
                    crate::logging::LogCat::Power,
                    entity = ship_entity,
                    "{} power {} -> {} ({})",
                    group.0,
                    current,
                    target,
                    reason
                );
                emit_ai_command(
                    entity_uuid,
                    crate::system_registry::power_reactor_system_id(),
                    crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                        group: group.clone(),
                        level: target,
                    },
                    control_sources,
                    &sessions,
                    ship_config,
                    admitted,
                );
            };

        // ── Data-authored per-group rules (issue #762) ───────────────────
        //
        // Iterate the ship's authored rules instead of the two hardcoded
        // movement→helm / red-alert→weapons blocks. Each rule carries its own
        // battery floor (`min_battery_reserve`); the pure `tick_power_rule`
        // engine (reusing the same `EngageState` hysteresis) will not engage
        // below that floor (AC2), so allocation only rises when the battery can
        // sustain it — avoiding preventable brownouts (AC3). Humans keep
        // control because we already `continue`d above when `operate_ai` is
        // false; the emitted command funnels through the same
        // `handle_power_messages` applier as human input (no source branch).
        let rule_input = crate::console_ai::PowerRuleInput {
            thrust,
            red_alert,
            battery_pct,
            dt,
            // Already gated on `operate_ai` above; the reactor is AI-driven.
            enabled: true,
        };
        for (index, rule) in ai_cfg.rules.iter().enumerate() {
            // Stable per-rule slot key so two rules targeting the same group
            // (e.g. thrust and red-alert both nudging weapons) keep independent
            // engage timers.
            let key = format!("{index}:{}", rule.group);
            let state = ai_state.rules.entry(key).or_default();
            let output = crate::console_ai::tick_power_rule(state, rule, &rule_input);
            let group_id = crate::messages::PowerGroupId(rule.group.clone());
            let reason = match &rule.trigger {
                crate::console_ai::PowerRuleTrigger::Thrust { .. } => "thrust rule",
                crate::console_ai::PowerRuleTrigger::RedAlert => "red-alert rule",
            };
            match output {
                crate::console_ai::PowerEngageOutput::Engage => {
                    emit_delta(&group_id, rule.nudge, reason, &mut admitted);
                }
                crate::console_ai::PowerEngageOutput::Disengage => {
                    emit_delta(&group_id, -rule.nudge, reason, &mut admitted);
                }
                crate::console_ai::PowerEngageOutput::NoChange => {}
            }
        }
    }
}

// `integrate_power_state` (issue #693's mutate-only adapter) was deleted by
// issue #831: truth-integration for power lives in
// `ship::power::handle_power_messages`, the single admitted-command applier
// for human and AI commands alike (mirroring the shields #826 retirement of
// `integrate_shield_state`).

// ── Torpedo AI (issue #694, completed by #698) ──────────────────────────────

/// AI torpedo auto-fire decision system (issue #694; promoted to full in #698).
///
/// Wires the previously-orphaned `console_ai::auto_fire_torpedo`: for ships
/// whose Tactical target is already locked (`TacticalRadarSelection`, written by
/// `console::weapons::ai_target_selection`), decides which loaded,
/// in-arc torpedo tubes to fire and writes the
/// decision into `TorpedoIntents` for
/// `console::weapons::integrate_weapons_state` to apply, rather than
/// calling `TorpedoSystem::launch` directly.
///
/// Replaces the old fused torpedo sub-block that used to run inline inside
/// `operate_tactical_ai` (banner-marked "TORPEDO AUTO-FIRE (future: split to
/// torpedo_tube system)").
///
/// # Real inputs (issue #698)
/// #694 landed with `TorpedoAiInput { target_locked: true, target_shields: 0 }`
/// hardcoded, because target selection still lived inside `operate_tactical_ai`
/// and there was no settled place to read a lock from. #697 split selection out,
/// so both now come from real state:
///
/// - `target_locked` — the locked target actually resolves to a live entity in
///   the world this tick. A `TacticalRadarSelection` naming a destroyed entity is not a
///   lock. (Pre-#698 this position lookup happened anyway, purely to compute
///   bearing; the difference is that failing it is now expressed as
///   `target_locked = false` rather than an early `continue`.)
/// - `target_facing_shields` — the HP of the single shield arc a torpedo
///   arriving from this ship would strike, resolved by handing the attack
///   bearing to the target's own `ShieldSystem::facing_index_for_bearing` —
///   the same bearing→arc resolver the damage path uses, so the gate asks
///   about the arc the shot is on course to meet. (The gate predicts from the
///   launcher's bearing; `tick_torpedo_lifecycle` routes the hit from the
///   torpedo's own impact point, so a torpedo that homes far enough around a
///   moving target can still land on a neighbouring arc. One resolver, two
///   moments in the shot's life.) `auto_fire_torpedo` only fires when this is
///   `<= 0`: phasers strip the shields, torpedoes finish the hull.
///
///   It is deliberately *not* the sum over all arcs. Summing let three
///   healthy REAR arcs veto a shot into a collapsed FRONT arc while the
///   attacker was dead ahead — the hull is exposed exactly where the torpedo
///   would land. With per-arc regen and short offline windows a four-arc
///   Alliance hull practically never has every arc down at once, so the
///   summed gate meant AI crews on those hulls never fired a torpedo. A
///   single-omni-arc NPC is unaffected: its one arc is the facing arc for
///   every bearing. An offline arc reports 0 because it passes damage through
///   to the hull and so is not blocking the shot, and a target with no
///   `ShipShields` at all (asteroids, debris) reports 0 and stays
///   torpedo-eligible — which is what preserves the pre-#698 behaviour for
///   every non-ship target.
///
/// # Gating
/// - `ShipSystemControlSources.policy_for(torpedo_magazine_system_id()).operate_ai`
///   — new constraint vs. the old fused block, which had no torpedo-specific
///   gate of its own beyond the combined `any_tactical_system_operates_ai`
///   check that ran before the old `operate_tactical_ai` (that check still
///   gates `ai_target_selection`; this is an *additional*, torpedo-specific
///   gate). The torpedo magazine is the shared bottleneck
///   resource across tubes, so its policy is the natural per-system gate
///   (no single unified `torpedo_system_id()` exists).
/// - `AiHighFidelity` (query filter, new constraint vs. the old system).
/// - Claimed/unclaimed Tactical station distinction (preserved verbatim from
///   the old block): when a session holds the Tactical station, auto-fire is
///   additionally gated on the holder's active rating having the
///   `torpedo_auto_fire` `ai_tuning` rule; when unclaimed, auto-fire is
///   unconditional. This exact distinction is what
///   `console::weapons`'s `ai_stops_firing_when_rating_switches_to_std`
///   test asserts on.
pub(crate) fn ai_torpedo_auto_fire(
    sessions: Res<crate::lobby::Sessions>,
    mut ships: Query<
        (
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ActiveStationRatings,
            &crate::ship_state::ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::weapons_plugin::TorpedoSystemResource>,
            &mut crate::messages::AdmittedCommands,
        ),
        (
            With<crate::ai_plugin::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
    asteroid_q: Query<
        (&crate::simulation::AsteroidUuid, &Transform),
        With<crate::simulation::Asteroid>,
    >,
    other_ships_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::ship::shields::ShipShields>,
            Option<&crate::ship_state::ShipPhysics>,
        ),
        Without<crate::simulation::Asteroid>,
    >,
) {
    let policy_sid = crate::system_registry::torpedo_magazine_system_id();

    for (
        entity_uuid,
        ship_config,
        control_sources,
        active_ratings,
        physics,
        blackboards,
        torpedo_sys_comp,
        mut admitted,
    ) in ships.iter_mut()
    {
        let policy = control_sources.0.policy_for(&policy_sid);
        if !policy.operate_ai {
            continue;
        }

        // The station owning this ship's weapons — resolved per-ship rather
        // than assumed to be named "tactical". NPCs have no weapons owner, so
        // the fallback keeps them on the unclaimed (fire unconditionally) path.
        let tactical_station = ship_config.0.weapons_station().unwrap_or_else(|| {
            crate::messages::StationId(crate::system_registry::TACTICAL_STATION_ID.into())
        });

        // Claimed/unclaimed distinction, preserved verbatim from the old
        // fused block: claimed stations gate on the active rating's
        // ai_tuning; unclaimed stations fire unconditionally.
        let auto_fire_enabled = match sessions.0.holder_for_station(&tactical_station) {
            Some(_) => active_ratings.0.get(&tactical_station).is_some_and(|r| {
                ship_config
                    .0
                    .has_ai_rule(&tactical_station, r, AI_RULE_TORPEDO_AUTO_FIRE)
            }),
            None => true,
        };
        if !auto_fire_enabled {
            continue;
        }

        // Combat Lock from this ship's frozen viewscreen blackboard (issue
        // #829, spec §1/§3). One-tick lag accepted, including for firing.
        let Some(target_uuid) = (match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
            _ => None,
        }) else {
            continue;
        };

        // Look up live world position + shields — the WorldResource snapshot is
        // stale for moving targets. A target that resolves here is a real lock
        // (`target_locked`); one that does not is a UUID naming something that
        // no longer exists, so there is nothing to shoot at.
        let target_state = asteroid_q
            .iter()
            .find_map(|(u, t)| {
                (u.0 == target_uuid).then_some(((t.translation.x, t.translation.z), None, 0.0))
            })
            .or_else(|| {
                other_ships_q.iter().find_map(|(u, t, shields, tphys)| {
                    (u.0 == target_uuid).then_some((
                        (t.translation.x, t.translation.z),
                        shields,
                        tphys.map(|p| p.yaw).unwrap_or(0.0),
                    ))
                })
            });
        let Some(((tx, tz), target_shields_comp, target_yaw)) = target_state else {
            continue;
        };

        // HP of the ONE arc a torpedo from this ship would strike. Arcs are
        // authored relative to the target's own facing, so the bearing is taken
        // from the target's frame (hence `target_yaw`) and resolved by the
        // target's own `facing_index_for_bearing` — the same resolver
        // `apply_damage` uses (`tick_torpedo_lifecycle` feeds it the torpedo's
        // impact bearing), so the gate and the eventual hit agree about
        // which arc is in the way. A healthy rear arc must not veto a shot into
        // a collapsed front arc. An offline arc reads 0 (it passes damage
        // through to the hull, so it is not blocking), and a target with no
        // `ShipShields` (asteroids, debris) reads 0 — torpedo-eligible.
        let target_facing_shields: i32 = target_shields_comp
            .map(|s| {
                let incoming = crate::shield::attacker_bearing_relative(
                    physics.x, physics.z, tx, tz, target_yaw,
                );
                let facing = &s.0.facings[s.0.facing_index_for_bearing(incoming)];
                if facing.is_online() {
                    facing.hp
                } else {
                    0
                }
            })
            .unwrap_or(0);

        let dx = tx - physics.x;
        let dz = tz - physics.z;
        let world_bearing = dx.atan2(-dz);
        let bearing = world_bearing - physics.yaw;

        // Per-entity component only (issue #738). This used to fall back to
        // the global `TorpedoSystemResource` Resource, which mirrors the
        // LOCAL ship's magazine and tube state — so an NPC with no `[torpedoes]`
        // block decided its auto-fire from the player's tubes. A ship with no
        // torpedo system has nothing to fire.
        let Some(torpedo_sys_comp) = torpedo_sys_comp else {
            continue;
        };
        let torpedo_sys: &crate::torpedo::TorpedoSystem = &torpedo_sys_comp.0;
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
            // Reaching here means `TacticalRadarSelection` named an entity that
            // resolved to a live world position above — a real lock.
            target_locked: true,
            target_facing_shields,
            tubes,
            magazine,
        };

        let tubes_to_fire = crate::console_ai::auto_fire_torpedo(&input);
        for tube_id in tubes_to_fire {
            // Emit as an admitted command through the shared AI seam (issue
            // #846), instead of the retired `TorpedoIntents` buffer.
            let Some(target) = crate::system_registry::torpedo_tube_system_id(&tube_id) else {
                continue;
            };
            crate::command_admission::ai_emit::emit_ai_command(
                entity_uuid,
                target,
                crate::messages::SystemControlPayload::FireTorpedo {
                    target_uuid: Some(target_uuid.clone()),
                },
                control_sources,
                &sessions,
                Some(ship_config),
                &mut admitted,
            );
        }
    }
}

/// AI torpedo *loading* system — the missing half of the torpedo AI.
///
/// `ai_torpedo_auto_fire` decides whether to fire an already-loaded tube, but
/// nothing ever asked for one to be loaded: `TorpedoSystem::from_configs`
/// starts every tube at `target_count: 0` and the auto-load block in
/// `TorpedoSystem::tick` is gated on `target_count > 0`, so an AI-crewed ship
/// never launched a torpedo in its life. This system is the console operator
/// nobody in an AI-crewed run is.
///
/// # Why it goes through `AdmittedCommands`
///
/// It emits `SetTorpedoVolleyTarget` into the ship's own `AdmittedCommands`
/// rather than writing `target_count` directly, so the AI issues *the same
/// command a human's console sends* and `handle_set_torpedo_volley_target`
/// stays the single writer of the tube's volley target (AGENTS.md — "humans
/// and AI are symmetric"; admission is the only place that knows who sent a
/// command). The alternative — a non-zero `target_count` default in
/// `from_configs` — would silently pre-load every *human* player's tubes and
/// drain their magazine with no order given.
///
/// This is the `operate_captain_ai` → `handle_set_red_alert` pattern: push
/// into each ship's own `AdmittedCommands` in `SimSet::Input`, ordered before
/// the handler that drains it, and let the handler iterate every ship so NPC
/// commands are not dropped.
///
/// # Gating
/// - Per-tube: the tube's own fine `torpedo-tube-<id>` system must be
///   `operate_ai` (an unregistered tube id falls back to the default-source
///   policy, matching `handle_load_tube`'s treatment — issue #801).
/// - The shared magazine's fine system must also be `operate_ai`, mirroring
///   `ai_torpedo_auto_fire`'s magazine gate: the magazine is the bottleneck
///   resource every tube draws from, and a magazine no AI is operating should
///   not be emptied into the tubes.
///
/// How many rounds to keep loaded is TOML, never a constant: the per-tube
/// `[[torpedoes.tubes]] ai_target_count`, the ship-wide `[torpedoes]
/// ai_volley_target`, or the tube's `volley_max` — resolved at construction
/// into `TorpedoTube::ai_target_count`.
///
/// Deliberately *not* gated on `AiHighFidelity`: loading is a slow standing
/// order (seconds of load time), and a ship demoted to low LOD mid-load would
/// otherwise stop maintaining tubes it is about to need on promotion.
pub(crate) fn ai_torpedo_load(
    sessions: Res<crate::lobby::Sessions>,
    // `Option<Res<_>>`, never bare — bare-`App` fixtures never insert it.
    log: Option<Res<crate::logging::LogFilterConfig>>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &crate::weapons_plugin::TorpedoSystemResource,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let magazine_id = crate::system_registry::torpedo_magazine_system_id();

    for (ship_entity, entity_uuid, control_sources, ship_config, torpedo_sys, mut admitted) in
        ships.iter_mut()
    {
        if !control_sources.0.policy_for(&magazine_id).operate_ai {
            continue;
        }

        let tubes: Vec<crate::console_ai::TubeLoadSummary> = torpedo_sys
            .0
            .tubes
            .iter()
            .map(|tube| {
                let tube_system_id = crate::system_registry::torpedo_tube_system_id(&tube.id)
                    .filter(|id| {
                        crate::console::weapons::shared::system_is_registered(control_sources, id)
                    });
                let policy = match &tube_system_id {
                    Some(id) => control_sources.0.policy_for(id),
                    // Unregistered fine system → default-source policy
                    // (issue #801 — no coarse fallback).
                    None => crate::ship::control_source::control_tick_policy(
                        crate::ship::control_source::ControlSource::default(),
                    ),
                };
                crate::console_ai::TubeLoadSummary {
                    id: tube.id.clone(),
                    target_count: tube.target_count,
                    ai_target_count: tube.ai_target_count,
                    operates_ai: policy.operate_ai,
                }
            })
            .collect();

        for (tube_id, count) in crate::console_ai::torpedo_load_orders(&tubes) {
            let Some(target) = crate::system_registry::torpedo_tube_system_id(&tube_id) else {
                continue;
            };
            // Through the shared AI-emit seam (issue #738), never a raw
            // `admitted.0.push`: admission is the only place that decides what
            // an `ai:` token may do, and it re-checks `operate_ai` on this
            // exact tube SystemId.
            let admitted_ok = emit_ai_command(
                entity_uuid,
                target.clone(),
                crate::messages::SystemControlPayload::SetTorpedoVolleyTarget { count },
                control_sources,
                &sessions,
                ship_config,
                &mut admitted,
            );
            // A refusal here is silent by construction — no panic, no order, no
            // torpedo — and the symptom (an AI crew that never fires) is a long
            // way from the cause. The decide gate above already required this
            // tube to be `operate_ai`, so a refusal means the tube's fine
            // system is not registered in this ship's `ControlSourceResolver`
            // (an authored `[[torpedoes.tubes]]` with no matching `[[system]]`
            // block) and the two gates are reading different worlds. Warn.
            if !admitted_ok {
                crate::pwarn!(
                    log,
                    crate::logging::LogCat::Weapons,
                    entity = ship_entity,
                    "torpedo load order for {} refused at admission — no such \
                     fine system on this ship's control sources",
                    target.0
                );
            }
        }
    }
}

// `integrate_torpedo_intents` (issue #694) lived here until issue #698 folded
// it into `console::weapons::integrate_weapons_state`, which drains
// `TorpedoIntents` and `PhaserIntents` in one adapter. Nothing else may drain
// `TorpedoIntents`: two systems both draining it would race for the launch.

// ── Frequency-hint AI ─────────────────────────────────────────────────────────

/// AI frequency-hint decision system (issue #692).
///
/// Wires the previously-orphaned `console_ai::tick_frequency_hint`: for
/// ships whose Sensors is AI-operated, waits
/// `SensorsAiConfigResource::frequency_hint_delay_secs` after a target lock
/// before emitting a `FrequencyHint` coordination message to Tactical —
/// replicating a Low-complexity Sensors operator's reaction delay, rather
/// than the instantaneous readout `ship::sensors::tick_sensors_frequency_hint`
/// provides for human-held Sensors.
///
/// # Gating
/// - `ShipSystemControlSources.policy_for(sensors_system_id()).operate_ai`.
/// - `AiHighFidelity` (query filter).
/// - Claimed/unclaimed Sensors station distinction, mirroring
///   `ai_torpedo_auto_fire`'s `AI_RULE_TORPEDO_AUTO_FIRE` gate: when a session
///   holds this ship's Sensors station, the hint additionally requires the
///   holder's active rating to declare the `auto_hint` `ai_tuning` rule; when
///   unclaimed (the NPC case — no human ever mans a synthetic ship's Sensors),
///   the hint fires unconditionally. `ShipConfigComponent`/`ActiveStationRatings`
///   are read as `Option` so ships in tests or contexts that predate this gate
///   (no config, no ratings) fall back to "unclaimed" rather than being
///   silently excluded from the query.
///
/// `tick_sensors_frequency_hint` explicitly skips ships that satisfy both of
/// these conditions, so the two systems never double-emit for the same ship.
fn ai_frequency_hint(
    time: Res<Time>,

    sessions: Option<Res<crate::lobby::Sessions>>,
    mut ships: Query<
        (
            Entity,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &mut ShipFrequencyHintState,
            Option<&crate::ship::sensors::SensorsAiConfigResource>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::ship_plugin::ActiveStationRatings>,
        ),
        (
            With<crate::ai_plugin::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
    target_shields_q: Query<(
        &crate::entity_spawner::EntityUuid,
        &crate::ship::shields::ShipShields,
    )>,
    mut writer: MessageWriter<crate::ship_plugin::CoordinationEnqueue>,
) {
    let dt = time.delta_secs();
    let sensors_sid = crate::system_registry::sensors_system_id();

    for (
        entity,
        control_sources,
        blackboards,
        mut hint_state,
        ai_config_comp,
        ship_config_comp,
        active_ratings_comp,
    ) in ships.iter_mut()
    {
        let policy = control_sources.0.policy_for(&sensors_sid);
        if !policy.operate_ai {
            // Not (or no longer) AI-driven — reset so a later hand-back to
            // AI control doesn't fire an instantly-stale hint.
            *hint_state = ShipFrequencyHintState::default();
            continue;
        }

        // Claimed/unclaimed Sensors distinction (issue #692 follow-up: the
        // `auto_hint` rule key was parsed but never consulted). No
        // `ShipConfigComponent` or no claimed session both fall back to
        // "unclaimed" — hint unconditionally, matching the NPC default.
        let auto_hint_enabled = match (&ship_config_comp, &sessions) {
            (Some(ship_config), Some(sessions)) => {
                let sensors_station = ship_config.0.sensors_station().unwrap_or_else(|| {
                    crate::messages::StationId(crate::system_registry::SENSORS_SYSTEM_ID.into())
                });
                match sessions.0.holder_for_station(&sensors_station) {
                    Some(_) => active_ratings_comp
                        .and_then(|r| r.0.get(&sensors_station))
                        .is_some_and(|rating| {
                            ship_config
                                .0
                                .has_ai_rule(&sensors_station, rating, AI_RULE_AUTO_HINT)
                        }),
                    None => true,
                }
            }
            _ => true,
        };
        if !auto_hint_enabled {
            continue;
        }

        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3),
        // identical to how the human twin `tick_sensors_frequency_hint` and the
        // firing paths now read it — never the tactical radar's live selection.
        let locked_target = match blackboards
            .0
            .get(&crate::system_registry::viewscreen_system_id())
        {
            Some(crate::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
            _ => None,
        };

        // Look up the target entity's shield frequency; fall back to 0.5,
        // mirroring `tick_sensors_frequency_hint`'s own fallback.
        let correct_frequency = locked_target
            .as_ref()
            .and_then(|uuid| {
                target_shields_q
                    .iter()
                    .find(|(u, _)| u.0.as_str() == uuid.as_str())
                    .map(|(_, shields)| shields.frequency())
            })
            .unwrap_or(0.5);

        // Per-ship tuning only (issue #738). This used to fall back to the
        // global `SensorsAiConfigResource` Resource while iterating every ship,
        // so whatever last wrote that Resource applied fleet-wide. Nothing in
        // the shipped app writes it — unlike the shields Resource, which
        // `server_app` dual-writes from the PLAYER ship's TOML — so the leak
        // was latent rather than live: it is registered purely for structural
        // symmetry with shields/power, and never seeded. The fallback is now
        // the parse-time default the type already supplies for a ship TOML that
        // omits the section.
        let default_ai_cfg;
        let ai_cfg: &crate::ship::sensors::SensorsAiConfigResource = match ai_config_comp {
            Some(c) => c,
            None => {
                default_ai_cfg = crate::ship::sensors::SensorsAiConfigResource::default();
                &default_ai_cfg
            }
        };

        let input = crate::console_ai::FrequencyHintInput {
            locked_target,
            correct_frequency,
            dt,
            delay_secs: ai_cfg.frequency_hint_delay_secs,
        };

        let output = crate::console_ai::tick_frequency_hint(&mut hint_state.0, &input);

        if let crate::console_ai::FrequencyHintOutput::Hint { frequency } = output {
            let sender_origin = control_sources.0.source_for(&sensors_sid);
            writer.write(crate::ship_plugin::CoordinationEnqueue {
                source_entity: entity,
                sender_origin,
                target: crate::system_registry::tactical_station_key(),
                payload: crate::messages::CoordinationPayload::FrequencyHint { frequency },
                sender_label: "Sensors".to_string(),
            });
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_plugin::AiHighFidelity;
    use crate::messages::{AdmittedCommands, CoordinationPayload};
    use crate::ship::control_source::ControlSource;
    use crate::ship::shields::{
        PendingShieldsThreatBearing, ShieldsAiConfigResource, ShieldsDamageHistory, ShipShields,
    };
    use crate::ship_plugin::{CoordinationEnqueue, ShipSystemControlSources};
    use crate::weapons_plugin::TacticalRadarSelection;

    #[derive(Resource, Default)]
    struct CoordBox(Vec<CoordinationEnqueue>);

    fn collect_coord(mut reader: MessageReader<CoordinationEnqueue>, mut box_: ResMut<CoordBox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    /// Registers `ai_shield_focus` (decide + admitted emit) chained before
    /// the shields module's `handle_shields_messages` (the single applier for
    /// human and AI commands, issue #826) — the production pipeline minus
    /// `AdmissionPlugin`'s per-tick clear, which these single-shot scenarios
    /// don't need.
    fn shield_test_app() -> App {
        let config = crate::shield::ShieldConfig {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .init_resource::<crate::ai_plugin::WorldSnapshot>()
            .init_resource::<ShieldsAiConfigResource>()
            .init_resource::<CoordBox>()
            .insert_resource(crate::lobby::Sessions(
                crate::lobby::session::SessionManager::new(),
            ))
            .add_message::<CoordinationEnqueue>()
            .add_systems(
                Update,
                (
                    ai_shield_focus.before(crate::ship::shields::handle_shields_messages),
                    crate::ship::shields::handle_shields_messages,
                ),
            )
            .add_systems(PostUpdate, collect_coord);

        app.world_mut().spawn((
            crate::server_app::Ship,
            ShipShields(crate::shield::ShieldSystem::new(&config), 0.5),
            ShieldsDamageHistory::default(),
            PendingShieldsThreatBearing::default(),
            ai_shield_control_sources(),
            AdmittedCommands::default(),
            AiHighFidelity,
        ));

        app
    }

    /// Mimics `AdmissionPlugin`'s per-tick clear of every ship's
    /// `AdmittedCommands` for multi-tick shield scenarios. Production clears
    /// admitted commands each tick in Input before the AI (Physics) refills
    /// them; without it, focus/clear commands would pile up across ticks and a
    /// stale `focused: true` could out-vote a later `focused: false`.
    /// Scheduled `.before(ai_shield_focus)` so the AI still refills same-tick.
    fn clear_admitted_each_tick(mut q: Query<&mut AdmittedCommands>) {
        for mut a in q.iter_mut() {
            a.0.clear();
        }
    }

    /// Coarse shields system (the decide gate) + every synthesised
    /// `shield-arc-<id>` fine system (the admission gate) set to Ai —
    /// matching how the entity spawner rosters an NPC's systems (arcs are
    /// synthesised into `ShipConfig.systems`, so the all-Ai loop covers
    /// them in production).
    fn ai_shield_control_sources() -> ShipSystemControlSources {
        let mut control_sources = ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::shields_system_id(),
            ControlSource::Ai,
        );
        for arc_id in ["fore", "port", "aft", "starboard"] {
            control_sources.0.set(
                crate::system_registry::shield_arc_system_id(arc_id).expect("arc id"),
                ControlSource::Ai,
            );
        }
        control_sources
    }

    fn focused_facing(app: &App, e: Entity) -> Option<usize> {
        app.world()
            .entity(e)
            .get::<ShipShields>()
            .unwrap()
            .0
            .focused_facing
    }

    fn ship_entity(app: &mut App) -> Entity {
        app.world_mut()
            .query_filtered::<Entity, With<ShipShields>>()
            .single(app.world())
            .unwrap()
    }

    #[test]
    fn ai_shield_focus_emits_admitted_focus_toward_damaged_facing() {
        // Simulates an attacker's hit landing on facing 0 (a real attack
        // always lands on one specific facing — "toward the attacker" from
        // the acceptance criteria). `tick_shield_focus_ai`'s health-imbalance
        // branch focuses the critically-weak facing whenever no arc's damage
        // history clears the damage-concentration threshold, which is what
        // fires here since facing 0 (20/100 HP) is far below the others
        // (100/100 HP). This exercises the full ai_shield_focus ->
        // validate_and_admit -> handle_shields_messages pipeline end to end
        // (issue #826).
        let mut app = shield_test_app();
        let e = ship_entity(&mut app);

        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 20; // heavy damage to facing 0 only
        }
        app.update();

        assert_eq!(
            focused_facing(&app, e),
            Some(0),
            "shield focus should follow the facing that took the attacker's damage \
             (ai_shield_focus decided, handle_shields_messages applied the admitted command)"
        );
    }

    #[test]
    fn ai_emitted_focus_applies_to_npc_own_ship_shields_only() {
        // Two NPC ships, both AI-operated. Only ship A takes damage; the
        // admitted `SetShieldArcFocus` lands in A's own `AdmittedCommands`,
        // so only A's `ShipShields` gains a focus — B is untouched (the
        // per-entity admission routing from issue #824, applied to shields
        // by #826).
        let mut app = shield_test_app();
        let a = ship_entity(&mut app);

        let config = crate::shield::ShieldConfig {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let b = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                ShipShields(crate::shield::ShieldSystem::new(&config), 0.5),
                ShieldsDamageHistory::default(),
                PendingShieldsThreatBearing::default(),
                ai_shield_control_sources(),
                AdmittedCommands::default(),
                AiHighFidelity,
            ))
            .id();

        {
            let mut entity_mut = app.world_mut().entity_mut(a);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 20;
        }
        app.update();

        assert_eq!(
            focused_facing(&app, a),
            Some(0),
            "the damaged NPC's own shields must gain the AI focus"
        );
        assert_eq!(
            focused_facing(&app, b),
            None,
            "the undamaged NPC must not be contaminated by another ship's AI command"
        );
    }

    #[test]
    fn npc_shield_ai_reads_its_own_tuning_not_the_player_ships_global_resource() {
        // Issue #738 isolation: `ai_shield_focus` used to resolve its tuning as
        // `per_entity_component.unwrap_or(&*global_resource)`, and `server_app`
        // writes that global Resource from the PLAYER ship's
        // `[shields_console.ai]` TOML. An NPC without the component therefore
        // ran the player's thresholds.
        //
        // Here the global Resource carries a permissive 90% health-ratio rule
        // and the per-entity components carry the parse-time default (50%). One
        // arc sits at 60/100 with the rest full: 0.6 < 0.9 focuses, 0.6 < 0.5
        // does not. So the global tuning is observable — and must not be what
        // the NPC uses.
        let mut app = shield_test_app();
        let npc = ship_entity(&mut app);
        app.insert_resource(ShieldsAiConfigResource {
            health_ratio_threshold: 90.0,
            ..Default::default()
        });

        // A second ship that DOES carry the permissive tuning as its own
        // per-entity component, proving the 60/100 arc is focusable at all.
        let config = crate::shield::ShieldConfig {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let tuned = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                ShipShields(crate::shield::ShieldSystem::new(&config), 0.5),
                ShieldsDamageHistory::default(),
                PendingShieldsThreatBearing::default(),
                ai_shield_control_sources(),
                AdmittedCommands::default(),
                AiHighFidelity,
                ShieldsAiConfigResource {
                    health_ratio_threshold: 90.0,
                    ..Default::default()
                },
            ))
            .id();

        for e in [npc, tuned] {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 60;
        }
        app.update();

        assert_eq!(
            focused_facing(&app, tuned),
            Some(0),
            "a ship carrying the permissive tuning on its own entity must focus the weak arc"
        );
        assert_eq!(
            focused_facing(&app, npc),
            None,
            "an NPC without its own shields-AI tuning must fall back to the parse-time \
             default, never to the global Resource holding the player ship's tuning"
        );
    }

    #[test]
    fn human_held_shield_arc_rejects_ai_emission() {
        // The decide gate (coarse shields system) still says AI, but the
        // targeted arc's control source is Human — `validate_and_admit`
        // refuses the `ai:` token (`operate_ai` does not hold on the arc), so
        // no admitted command exists and the focus never flips. This is the
        // admission-refusal path the retired `integrate_shield_state` adapter
        // could not express (it applied intents unconditionally).
        let mut app = shield_test_app();
        let e = ship_entity(&mut app);
        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut cs = entity_mut.get_mut::<ShipSystemControlSources>().unwrap();
            for arc_id in ["fore", "port", "aft", "starboard"] {
                cs.0.set(
                    crate::system_registry::shield_arc_system_id(arc_id).expect("arc id"),
                    ControlSource::Human,
                );
            }
        }

        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 20;
        }
        app.update();

        assert_eq!(
            focused_facing(&app, e),
            None,
            "an ai: emission targeting a human-held shield arc must be refused at admission"
        );
        assert!(
            app.world()
                .entity(e)
                .get::<AdmittedCommands>()
                .unwrap()
                .0
                .is_empty(),
            "the refused command must never reach AdmittedCommands"
        );
    }

    #[test]
    fn ai_shield_focus_detects_damage_concentration_without_health_imbalance() {
        // Regression test for a bug where damage-concentration detection was
        // dead code: `prev_hp` was derived from the last DamageRecord's
        // `amount` (a delta, not an HP value) instead of a real per-arc HP
        // baseline, so `facing.hp < prev_hp` could never be true on an arc's
        // first-ever hit — and since a record could therefore never be
        // created, it could never be true on any later hit either.
        //
        // This scenario deliberately keeps health imbalance below its
        // trigger threshold (facing 1 ends at 60/100 — normalized 0.6, not
        // below 0.5 * 1.0) so ONLY the damage-concentration branch can
        // produce a Focus decision. With the bug, this test fails (no
        // focus); with the fix, one recorded hit is enough.
        let mut app = shield_test_app();
        let e = ship_entity(&mut app);

        // Tick 1: establish the damage-history baseline (first observation
        // of this HP value never counts as damage).
        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[1].hp = 90;
        }
        app.update();
        assert_eq!(
            focused_facing(&app, e),
            None,
            "the baseline-establishing tick must not itself register damage"
        );

        // Tick 2: a real hit lands on facing 1, dropping it further while
        // every other facing stays untouched — 100% of window damage on one
        // arc, but not enough absolute HP loss to trip health imbalance.
        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[1].hp = 60;
        }
        app.update();

        assert_eq!(
            focused_facing(&app, e),
            Some(1),
            "damage-concentration detection must focus the arc that just took \
             a real hit, even when health imbalance alone would not trigger"
        );
    }

    #[test]
    fn ai_shield_focus_accumulates_repeated_hits_on_one_arc_across_ticks() {
        // Issue #747: repeated hits on the same arc over separate ticks must
        // accumulate in that arc's damage history (not overwrite), so a stream
        // of small hits sums to a concentrated signal over the authored window.
        let mut app = shield_test_app();
        app.add_systems(Update, clear_admitted_each_tick.before(ai_shield_focus));
        let e = ship_entity(&mut app);

        // Tick 1: baseline observation (never counts as damage).
        app.update();
        assert_eq!(focused_facing(&app, e), None);

        // Tick 2: first small hit on facing 1 (100 -> 97). Health stays
        // balanced (0.97), so only concentration can drive a focus.
        {
            let mut em = app.world_mut().entity_mut(e);
            em.get_mut::<ShipShields>().unwrap().0.facings[1].hp = 97;
        }
        app.update();
        assert_eq!(
            focused_facing(&app, e),
            Some(1),
            "the first recorded hit should focus arc 1 by concentration"
        );

        // Tick 3: a second small hit on the same arc (97 -> 94). Both hits
        // must be retained in arc 1's window and keep the arc focused.
        {
            let mut em = app.world_mut().entity_mut(e);
            em.get_mut::<ShipShields>().unwrap().0.facings[1].hp = 94;
        }
        app.update();

        assert_eq!(
            focused_facing(&app, e),
            Some(1),
            "repeated hits on arc 1 must keep it focused"
        );
        let history = app.world().entity(e).get::<ShieldsDamageHistory>().unwrap();
        assert_eq!(
            history.arcs[1].len(),
            2,
            "both hits on arc 1 must accumulate as separate records in the window"
        );
        let arc1_total: i32 = history.arcs[1].iter().map(|r| r.amount).sum();
        assert_eq!(
            arc1_total, 6,
            "accumulated window damage on arc 1 must be 3 + 3"
        );
    }

    #[test]
    fn ai_shield_focus_reverts_when_concentrated_damage_expires() {
        // Issue #747: once the concentrated hit ages out of the authored
        // damage window (4s), the concentration signal disappears and, with
        // health balanced, the AI must clear the focus it took. `tick_shields`
        // is scheduled so non-focused arcs settle to their reduced cap (the
        // production steady state) rather than sitting above it forever.
        let mut app = shield_test_app();
        app.add_systems(Update, clear_admitted_each_tick.before(ai_shield_focus));
        app.add_systems(
            Update,
            crate::ship::shields::tick_shields.after(crate::ship::shields::handle_shields_messages),
        );
        let e = ship_entity(&mut app);

        // Tick 1: baseline.
        app.update();
        // Tick 2: one hit on facing 1 (100 -> 90) focuses it by concentration.
        {
            let mut em = app.world_mut().entity_mut(e);
            em.get_mut::<ShipShields>().unwrap().0.facings[1].hp = 90;
        }
        app.update();
        assert_eq!(
            focused_facing(&app, e),
            Some(1),
            "the concentrated hit should focus arc 1"
        );

        // Advance ~5s of ManualDuration(100ms) ticks with no further hits. The
        // record at ~t=0.2 ages past the 4s window and prunes; the focus must
        // revert to None once concentration is gone and health is balanced.
        for _ in 0..50 {
            app.update();
        }

        assert_eq!(
            focused_facing(&app, e),
            None,
            "focus must clear once the concentrated damage expires from the window"
        );
    }

    #[test]
    fn ai_shield_focus_ignores_focus_decay_as_incoming_damage() {
        // Issue #747: focusing one arc reduces the others' effective max_hp, so
        // `tick_shields` bleeds those non-focused arcs down toward the reduced
        // cap. That HP drop is a focus side effect, not incoming fire — the
        // damage detector must NOT record it, or a decaying arc would steal the
        // focus. Here only arc 1 is ever hit; the decaying arcs must stay
        // record-free and never take the focus.
        let mut app = shield_test_app();
        app.add_systems(Update, clear_admitted_each_tick.before(ai_shield_focus));
        app.add_systems(
            Update,
            crate::ship::shields::tick_shields.after(crate::ship::shields::handle_shields_messages),
        );
        let e = ship_entity(&mut app);

        app.update(); // baseline
        {
            let mut em = app.world_mut().entity_mut(e);
            em.get_mut::<ShipShields>().unwrap().0.facings[1].hp = 90;
        }
        app.update(); // focus arc 1
        assert_eq!(focused_facing(&app, e), Some(1));

        // Let the non-focused arcs decay from 100 toward their reduced cap.
        for _ in 0..20 {
            app.update();
        }

        assert_eq!(
            focused_facing(&app, e),
            Some(1),
            "decay on non-focused arcs must not steal the focus from the hit arc"
        );
        let history = app.world().entity(e).get::<ShieldsDamageHistory>().unwrap();
        for idx in [0usize, 2, 3] {
            assert!(
                history.arcs[idx].is_empty(),
                "non-focused arc {idx} decaying toward its cap must record no incoming damage"
            );
        }
    }

    #[test]
    fn ai_shield_focus_skips_ships_where_shields_are_not_ai_operated() {
        let mut app = shield_test_app();
        let e = ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .get_mut::<ShipSystemControlSources>()
            .unwrap()
            .0
            .set(
                crate::system_registry::shields_system_id(),
                ControlSource::Human,
            );

        app.update();
        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 20;
        }
        app.update();

        assert_eq!(
            focused_facing(&app, e),
            None,
            "human-operated shields must not be focused by the AI decision system"
        );
    }

    #[test]
    fn ai_shield_focus_threat_bearing_override_focuses_closest_facing_via_admission() {
        let mut app = shield_test_app();
        let e = ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .get_mut::<PendingShieldsThreatBearing>()
            .unwrap()
            .0 = Some(90_f32.to_radians());

        app.update();

        let focused = focused_facing(&app, e);
        assert!(
            focused.is_some(),
            "threat-bearing override must focus a facing via the admitted-command path"
        );

        // The override takes priority over damage analysis and must consume
        // the pending bearing.
        assert_eq!(
            app.world()
                .entity(e)
                .get::<PendingShieldsThreatBearing>()
                .unwrap()
                .0,
            None,
            "pending threat bearing must be consumed (taken) once applied"
        );
    }

    // ── ai_frequency_hint ─────────────────────────────────────────────────

    /// Test-only glue (issue #829): seed each ship's viewscreen combat_lock from
    /// its `TacticalRadarSelection` before `ai_frequency_hint` reads the frozen
    /// fact — standing in for the radar publisher + viewscreen aggregator the
    /// full app runs, exactly like the other frequency/firing test harnesses.
    fn seed_viewscreen_from_selection(
        mut q: Query<
            (
                Option<&crate::weapons_plugin::TacticalRadarSelection>,
                &mut crate::server_app::ShipSystemBlackboards,
            ),
            With<crate::server_app::Ship>,
        >,
    ) {
        for (tac, mut bbs) in q.iter_mut() {
            let combat_lock = tac.and_then(|t| t.0.clone());
            let mut vbb = match bbs.0.get(&crate::system_registry::viewscreen_system_id()) {
                Some(crate::messages::SystemBlackboard::Viewscreen(v)) => v.clone(),
                _ => crate::messages::ViewscreenBlackboard::default(),
            };
            vbb.combat_lock = combat_lock;
            bbs.0.insert(
                crate::system_registry::viewscreen_system_id(),
                crate::messages::SystemBlackboard::Viewscreen(vbb),
            );
        }
    }

    fn freq_hint_test_app() -> App {
        let mut app = App::new();
        // Manual `Time::advance_by` (mirroring `ai::server`'s LOD tests)
        // rather than `TimePlugin` + `TimeUpdateStrategy`: the latter reports
        // a zero delta on the frame it's added, which would otherwise force
        // every test here to burn an extra warm-up `app.update()`.
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::sensors::SensorsAiConfigResource>()
            .init_resource::<CoordBox>()
            .add_message::<CoordinationEnqueue>()
            .add_systems(
                Update,
                (seed_viewscreen_from_selection, ai_frequency_hint).chain(),
            )
            .add_systems(PostUpdate, collect_coord);

        let mut control_sources = ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Ai,
        );

        let target = app
            .world_mut()
            .spawn((
                crate::entity_spawner::EntityUuid("target-1".into()),
                ShipShields(crate::shield::ShieldSystem::default(), 0.75),
            ))
            .id();

        let source = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                control_sources,
                crate::server_app::ShipSystemBlackboards::default(),
                TacticalRadarSelection(Some("target-1".into())),
                ShipFrequencyHintState::default(),
                AiHighFidelity,
            ))
            .id();

        let _ = target;
        app.insert_resource(SourceShip(source));
        app
    }

    #[derive(Resource)]
    struct SourceShip(Entity);

    fn tick_with_dt(app: &mut App, dt_secs: f32) {
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(dt_secs));
        app.update();
    }

    #[test]
    fn ai_frequency_hint_propagates_after_delay_for_ai_operated_sensors() {
        let mut app = freq_hint_test_app();
        // 4s exceeds the 3s default delay in a single tick.
        tick_with_dt(&mut app, 4.0);

        let coord = &app.world().resource::<CoordBox>().0;
        let hint = coord
            .iter()
            .find(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
            .expect("expected a FrequencyHint CoordinationEnqueue after the delay elapses");

        match &hint.payload {
            CoordinationPayload::FrequencyHint { frequency } => {
                assert!(
                    (*frequency - 0.75).abs() < f32::EPSILON,
                    "hint should carry the locked target's shield frequency"
                );
            }
            other => panic!("expected FrequencyHint, got {other:?}"),
        }
        assert_eq!(
            hint.target,
            crate::system_registry::tactical_station_key(),
            "frequency hint should target Tactical"
        );
    }

    #[test]
    fn npc_frequency_hint_reads_its_own_tuning_not_the_global_resource() {
        // Issue #738 isolation, mirroring the shields case: `ai_frequency_hint`
        // used to resolve its delay as
        // `per_entity_component.unwrap_or(&*global_resource)` while iterating
        // every ship, so any write to that Resource would have applied
        // fleet-wide. Nothing writes it today (unlike the shields Resource,
        // which `server_app` dual-writes from the player ship) — this test
        // seeds it by hand so the leak stays closed if anything ever does.
        //
        // The global Resource here carries an eager 0.5s delay; one tick of
        // 1.0s therefore fires under the global tuning but not under the
        // parse-time default (3.0s).
        let mut app = freq_hint_test_app();
        let npc = app.world().resource::<SourceShip>().0;
        app.insert_resource(crate::ship::sensors::SensorsAiConfigResource {
            frequency_hint_delay_secs: 0.5,
        });

        let mut tuned_sources = ShipSystemControlSources::default();
        tuned_sources.0.set(
            crate::system_registry::sensors_system_id(),
            ControlSource::Ai,
        );
        let tuned = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                tuned_sources,
                crate::server_app::ShipSystemBlackboards::default(),
                TacticalRadarSelection(Some("target-1".into())),
                ShipFrequencyHintState::default(),
                AiHighFidelity,
                crate::ship::sensors::SensorsAiConfigResource {
                    frequency_hint_delay_secs: 0.5,
                },
            ))
            .id();

        tick_with_dt(&mut app, 1.0);

        let coord = &app.world().resource::<CoordBox>().0;
        let hinting_ships: Vec<Entity> = coord
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
            .map(|m| m.source_entity)
            .collect();
        assert!(
            hinting_ships.contains(&tuned),
            "a ship carrying the eager 0.5s delay on its own entity must hint after 1.0s"
        );
        assert!(
            !hinting_ships.contains(&npc),
            "an NPC without its own sensors-AI tuning must fall back to the parse-time \
             3.0s default, never to the global Resource holding the player ship's tuning"
        );
    }

    #[test]
    fn ai_frequency_hint_skips_ships_where_sensors_are_not_ai_operated() {
        let mut app = freq_hint_test_app();
        let source = app.world().resource::<SourceShip>().0;
        app.world_mut()
            .entity_mut(source)
            .get_mut::<ShipSystemControlSources>()
            .unwrap()
            .0
            .set(
                crate::system_registry::sensors_system_id(),
                ControlSource::Human,
            );

        app.update();

        let coord = &app.world().resource::<CoordBox>().0;
        assert!(
            !coord
                .iter()
                .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
            "human-operated Sensors must not be hinted by the AI system"
        );
    }

    // ── ai_frequency_hint: AI_RULE_AUTO_HINT claimed/unclaimed gate ─────────
    //
    // `ai_torpedo_auto_fire`'s claimed/unclaimed pattern: an unclaimed Sensors
    // station (no human session holder — every NPC, and any ship before a
    // human takes Sensors) hints unconditionally; once claimed, the hint
    // additionally requires the holder's active rating to declare
    // `auto_hint` in its `ai_tuning` table.

    fn sensors_ship_config() -> crate::ship::config::ShipConfig {
        let toml = r#"
[[station]]
id = "sensors"
name = "Sensors"
description = "Long-range sensors."
rank = "Ens."

[[station.rating]]
name = "Assisted"
automated_systems = []
[station.rating.ai_tuning]
auto_hint = {}

[[station.rating]]
name = "Std"
automated_systems = []

[[system]]
id = "sensors"
kind = "sensors"
station = "sensors"
"#;
        crate::ship::config::ShipConfig::from_toml(toml, &["sensors"]).unwrap()
    }

    /// Adds `ShipConfigComponent` + `ActiveStationRatings` to the ship spawned
    /// by `freq_hint_test_app`, and a `Sessions` resource with the Sensors
    /// station claimed by `holder_token`. Returns the source ship entity.
    fn claim_sensors_station(app: &mut App, holder_token: &str, rating: &str) -> Entity {
        let source = app.world().resource::<SourceShip>().0;
        let sensors_station = crate::messages::StationId("sensors".into());

        let mut sm = crate::lobby::session::SessionManager::new();
        sm.register(holder_token.into(), "Operator".into()).unwrap();
        sm.set_station(holder_token, Some(sensors_station.clone()));
        app.insert_resource(crate::lobby::Sessions(sm));

        let mut active_ratings = crate::ship_plugin::ActiveStationRatings::default();
        active_ratings.0.insert(sensors_station, rating.into());

        app.world_mut()
            .entity_mut(source)
            .insert(crate::ship_plugin::ShipConfigComponent(
                sensors_ship_config(),
            ))
            .insert(active_ratings);

        source
    }

    #[test]
    fn ai_frequency_hint_fires_when_claimed_station_rating_declares_auto_hint() {
        let mut app = freq_hint_test_app();
        claim_sensors_station(&mut app, "op1", "Assisted");

        tick_with_dt(&mut app, 4.0);

        let coord = &app.world().resource::<CoordBox>().0;
        assert!(
            coord
                .iter()
                .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
            "a claimed Sensors station whose active rating declares auto_hint \
             must still be hinted by the AI system"
        );
    }

    #[test]
    fn ai_frequency_hint_stays_silent_when_claimed_station_rating_lacks_auto_hint() {
        let mut app = freq_hint_test_app();
        claim_sensors_station(&mut app, "op1", "Std");

        tick_with_dt(&mut app, 4.0);

        let coord = &app.world().resource::<CoordBox>().0;
        assert!(
            !coord
                .iter()
                .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
            "a claimed Sensors station on a rating without auto_hint in its \
             ai_tuning table must not be hinted by the AI system"
        );
    }

    #[test]
    fn ai_frequency_hint_fires_unconditionally_when_sensors_station_is_unclaimed() {
        let mut app = freq_hint_test_app();
        let source = app.world().resource::<SourceShip>().0;

        // Ship config + ratings present, but no session holds the station —
        // e.g. an NPC, or the player ship before anyone takes Sensors.
        app.insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ));
        app.world_mut()
            .entity_mut(source)
            .insert(crate::ship_plugin::ShipConfigComponent(
                sensors_ship_config(),
            ))
            .insert(crate::ship_plugin::ActiveStationRatings::default());

        tick_with_dt(&mut app, 4.0);

        let coord = &app.world().resource::<CoordBox>().0;
        assert!(
            coord
                .iter()
                .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
            "an unclaimed Sensors station must be hinted unconditionally, \
             regardless of any rating's ai_tuning table"
        );
    }

    // ── ai_power_allocation (emit → admit → apply) ──────────────────────────

    /// Wires the real production pair: the AI decide system
    /// (`ai_power_allocation`, emit) `.before` the single applier
    /// (`ship::power::handle_power_messages`, issue #831), minus the
    /// `AdmissionPlugin` per-tick clear these single-shot scenarios don't need
    /// (mirrors `shield_test_app`).
    fn power_test_app() -> App {
        let mut app = App::new();
        // Manual `Time::advance_by` (mirroring `ai_frequency_hint`'s test
        // app above) rather than `TimePlugin` + `TimeUpdateStrategy`.
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
            .init_resource::<crate::ship::power::PowerAiConfigResource>()
            .insert_resource(crate::lobby::Sessions(
                crate::lobby::session::SessionManager::new(),
            ))
            .add_systems(
                Update,
                (
                    ai_power_allocation.before(crate::ship::power::handle_power_messages),
                    crate::ship::power::handle_power_messages,
                ),
            );

        let mut control_sources = ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );

        app.world_mut().spawn((
            crate::server_app::Ship,
            control_sources,
            crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::default(),
            ),
            crate::ship_state::ShipRedAlert::default(),
            crate::ship_plugin::LastHelmInput::default(),
            crate::ship::power::ShipPowerAiState::default(),
            crate::messages::AdmittedCommands::default(),
            AiHighFidelity,
        ));

        app
    }

    fn power_ship_entity(app: &mut App) -> Entity {
        app.world_mut()
            .query_filtered::<Entity, With<crate::ship::power::ShipPowerSystem>>()
            .single(app.world())
            .unwrap()
    }

    fn power_level(app: &App, e: Entity, group: &str) -> u8 {
        app.world()
            .entity(e)
            .get::<crate::ship::power::ShipPowerSystem>()
            .unwrap()
            .0
            .level_for(&crate::messages::PowerGroupId(group.into()))
    }

    fn power_tick_with_dt(app: &mut App, dt_secs: f32) {
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(dt_secs));
        app.update();
    }

    #[test]
    fn ai_power_allocation_reallocates_toward_weapons_on_red_alert() {
        // Explicit acceptance criterion: NPC under red alert reallocates
        // power toward weapons.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);

        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        let before = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);

        // 4s exceeds the 3s default red_alert_engage_delay_secs in a single
        // tick; default battery is full (100%), well above the 10% floor.
        power_tick_with_dt(&mut app, 4.0);

        let after = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        assert!(
            after > before,
            "weapons power should increase under sustained red alert (before={before}, after={after})"
        );
    }

    #[test]
    fn npc_power_ai_reads_its_own_tuning_not_the_player_ships_global_resource() {
        // Issue #738 isolation: `ai_power_allocation` used to resolve both its
        // reactor config and its AI tuning as
        // `per_entity_component.unwrap_or(&*global_resource)`, and `server_app`
        // writes those Resources from the PLAYER ship's `[power]` TOML. An NPC
        // spawned without the components therefore ran the player's thresholds.
        //
        // The global `PowerAiConfigResource` here carries an eager 0.5s
        // red-alert delay, so a single 1.0s tick engages under the global
        // tuning but not under the parse-time 3.0s default.
        let mut app = power_test_app();
        let npc = power_ship_entity(&mut app);
        // Eager 0.5s red-alert rule as a GLOBAL resource — `ai_power_allocation`
        // must NOT fall back to it for a ship that carries no per-entity tuning.
        let eager_rules = vec![crate::console_ai::PowerAiRule {
            group: crate::modifiers::power_system::WEAPONS_POWER_GROUP.to_string(),
            trigger: crate::console_ai::PowerRuleTrigger::RedAlert,
            min_battery_reserve: 10.0,
            battery_recharge_pct: 100.0,
            engage_delay_secs: 0.5,
            nudge: 1,
        }];
        app.insert_resource(crate::ship::power::PowerAiConfigResource {
            rules: eager_rules.clone(),
        });

        let mut tuned_sources = ShipSystemControlSources::default();
        tuned_sources.0.set(
            crate::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
        let tuned = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                tuned_sources,
                crate::ship::power::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::default(),
                ),
                crate::ship_state::ShipRedAlert(true),
                crate::ship_plugin::LastHelmInput::default(),
                crate::ship::power::ShipPowerAiState::default(),
                crate::messages::AdmittedCommands::default(),
                AiHighFidelity,
                crate::ship::power::PowerAiConfigResource { rules: eager_rules },
            ))
            .id();

        app.world_mut()
            .entity_mut(npc)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        let npc_before = power_level(
            &app,
            npc,
            crate::modifiers::power_system::WEAPONS_POWER_GROUP,
        );
        let tuned_before = power_level(
            &app,
            tuned,
            crate::modifiers::power_system::WEAPONS_POWER_GROUP,
        );

        power_tick_with_dt(&mut app, 1.0);

        assert!(
            power_level(
                &app,
                tuned,
                crate::modifiers::power_system::WEAPONS_POWER_GROUP
            ) > tuned_before,
            "a ship carrying the eager 0.5s red-alert delay on its own entity must engage \
             after 1.0s"
        );
        assert_eq!(
            power_level(
                &app,
                npc,
                crate::modifiers::power_system::WEAPONS_POWER_GROUP
            ),
            npc_before,
            "an NPC without its own power-AI tuning must fall back to the parse-time 3.0s \
             default, never to the global Resource holding the player ship's tuning"
        );
    }

    #[test]
    fn ai_power_allocation_reallocates_toward_helm_on_sustained_thrust() {
        // Movement-rule equivalent: high sustained thrust + healthy battery
        // increases helm power (±1 per engage event under the new
        // timer/hysteresis semantics, not an absolute target of 3).
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);

        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_plugin::LastHelmInput>()
            .unwrap()
            .thrust = 0.9;

        let before = power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP);

        // 4s exceeds the 3s default movement_engage_delay_secs in a single
        // tick; default battery is full (100%), well above the 50% floor.
        power_tick_with_dt(&mut app, 4.0);

        let after = power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP);
        assert!(
            after > before,
            "helm power should increase under sustained high thrust (before={before}, after={after})"
        );
    }

    #[test]
    fn ai_power_allocation_skips_ships_without_ai_high_fidelity() {
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut().entity_mut(e).remove::<AiHighFidelity>();
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        let before = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        power_tick_with_dt(&mut app, 4.0);
        let after = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        assert_eq!(
            before, after,
            "ships without AiHighFidelity must not be touched by ai_power_allocation"
        );
    }

    #[test]
    fn ai_power_allocation_skips_ships_where_power_is_not_ai_operated() {
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .get_mut::<ShipSystemControlSources>()
            .unwrap()
            .0
            .set(
                crate::system_registry::power_reactor_system_id(),
                ControlSource::Human,
            );
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        let before = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        power_tick_with_dt(&mut app, 4.0);
        let after = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        assert_eq!(
            before, after,
            "human-operated power reactor must not be touched by ai_power_allocation"
        );
    }

    #[test]
    fn ai_power_reallocation_dual_writes_resource_for_local_ship() {
        // Issue #693 review finding, rewired for issue #831: when the AI path
        // reallocates power for the LocalShip (e.g. a disconnected player's
        // station backfilled to AI per AGENTS.md rule 5), the admitted
        // `SetPowerGroupAllocation` must flow through
        // `ship::power::handle_power_messages` — the single applier — which
        // dual-writes the legacy global `ShipPowerSystem` Resource, not just
        // the per-entity Component. (Previously asserted against the retired
        // `integrate_power_state` adapter.)
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .insert(crate::server_app::LocalShip);
        app.world_mut()
            .insert_resource(crate::ship::power::ShipPowerSystem(
                crate::modifiers::power_system::PowerSystem::default(),
            ));
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        // 4s exceeds the 3s default red_alert_engage_delay_secs in a single
        // tick, so the weapons group should engage by +1.
        power_tick_with_dt(&mut app, 4.0);

        let component_level =
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        let resource_level = app
            .world()
            .resource::<crate::ship::power::ShipPowerSystem>()
            .0
            .level_for(&crate::messages::PowerGroupId(
                crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
            ));
        assert_eq!(
            component_level, resource_level,
            "global ShipPowerSystem Resource must be dual-written to match \
             the LocalShip's per-entity Component after AI reallocation"
        );
        assert!(
            resource_level > 1,
            "expected the AI-driven weapons reallocation to be reflected in \
             the dual-written Resource (resource_level={resource_level})"
        );
    }

    #[test]
    fn ai_power_allocation_emits_admitted_set_power_group_allocation() {
        // The decide system emits its reallocation as an admitted
        // `SetPowerGroupAllocation` targeting the reactor (issue #831), rather
        // than writing a private `PowerReactorIntents` component. A saturated
        // no-op must NOT be admitted every tick.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        // Run just the emit half so AdmittedCommands is observable before the
        // applier drains it: a fresh app whose only system is the decide one.
        let mut emit_app = App::new();
        emit_app
            .insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
            .init_resource::<crate::ship::power::PowerAiConfigResource>()
            .insert_resource(crate::lobby::Sessions(
                crate::lobby::session::SessionManager::new(),
            ))
            .add_systems(Update, ai_power_allocation);
        let mut cs = ShipSystemControlSources::default();
        cs.0.set(
            crate::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
        let ee = emit_app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                cs,
                crate::ship::power::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::default(),
                ),
                crate::ship_state::ShipRedAlert(true),
                crate::ship_plugin::LastHelmInput::default(),
                crate::ship::power::ShipPowerAiState::default(),
                crate::messages::AdmittedCommands::default(),
                AiHighFidelity,
            ))
            .id();
        {
            let mut time = emit_app.world_mut().resource_mut::<Time>();
            time.advance_by(std::time::Duration::from_secs_f32(4.0));
        }
        emit_app.update();

        let admitted = emit_app
            .world()
            .entity(ee)
            .get::<crate::messages::AdmittedCommands>()
            .unwrap();
        let has_weapons_alloc = admitted.0.iter().any(|c| {
            c.target == crate::system_registry::power_reactor_system_id()
                && matches!(
                    &c.payload,
                    crate::messages::SystemControlPayload::SetPowerGroupAllocation { group, .. }
                        if group.0 == crate::modifiers::power_system::WEAPONS_POWER_GROUP
                )
        });
        assert!(
            has_weapons_alloc,
            "sustained red alert must admit a SetPowerGroupAllocation for weapons"
        );

        // No-op guard: with weapons now saturated at 4, a further sustained
        // engage produces the same target level and must NOT be re-admitted.
        {
            let mut ent = emit_app.world_mut().entity_mut(ee);
            let mut ps = ent
                .get_mut::<crate::ship::power::ShipPowerSystem>()
                .unwrap();
            ps.0.set_group_allocation(
                &crate::messages::PowerGroupId(
                    crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                ),
                4,
            )
            .unwrap();
            let mut ac = ent.get_mut::<crate::messages::AdmittedCommands>().unwrap();
            ac.0.clear();
        }
        {
            let mut time = emit_app.world_mut().resource_mut::<Time>();
            time.advance_by(std::time::Duration::from_secs_f32(4.0));
        }
        emit_app.update();
        let admitted = emit_app
            .world()
            .entity(ee)
            .get::<crate::messages::AdmittedCommands>()
            .unwrap();
        assert!(
            !admitted.0.iter().any(|c| matches!(
                &c.payload,
                crate::messages::SystemControlPayload::SetPowerGroupAllocation { group, .. }
                    if group.0 == crate::modifiers::power_system::WEAPONS_POWER_GROUP
            )),
            "a saturated weapons allocation (already at 4) must not re-admit a no-op"
        );
    }

    #[test]
    fn two_ships_with_different_authored_rules_allocate_independently() {
        // AC1 + AC4 per-ship isolation: two AI ships carry DIFFERENT authored
        // rules — one boosts `helm` on thrust, the other boosts `sensors` on
        // thrust. Under identical sustained thrust each ship nudges only its
        // own authored group, and neither touches the other's, proving the
        // rules are per-ship data (not a shared hardcoded category set).
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
            .init_resource::<crate::ship::power::PowerAiConfigResource>()
            .insert_resource(crate::lobby::Sessions(
                crate::lobby::session::SessionManager::new(),
            ))
            .add_systems(
                Update,
                (
                    ai_power_allocation.before(crate::ship::power::handle_power_messages),
                    crate::ship::power::handle_power_messages,
                ),
            );

        let spawn_ruled = |app: &mut App, group: &str| -> Entity {
            let mut cs = ShipSystemControlSources::default();
            cs.0.set(
                crate::system_registry::power_reactor_system_id(),
                ControlSource::Ai,
            );
            let helm = crate::ship_plugin::LastHelmInput {
                thrust: 0.9,
                ..Default::default()
            };
            app.world_mut()
                .spawn((
                    crate::server_app::Ship,
                    cs,
                    crate::ship::power::ShipPowerSystem(
                        crate::modifiers::power_system::PowerSystem::default(),
                    ),
                    crate::ship_state::ShipRedAlert::default(),
                    helm,
                    crate::ship::power::ShipPowerAiState::default(),
                    crate::messages::AdmittedCommands::default(),
                    AiHighFidelity,
                    crate::ship::power::PowerAiConfigResource {
                        rules: vec![crate::console_ai::PowerAiRule {
                            group: group.to_string(),
                            trigger: crate::console_ai::PowerRuleTrigger::Thrust { threshold: 0.7 },
                            min_battery_reserve: 50.0,
                            battery_recharge_pct: 100.0,
                            engage_delay_secs: 3.0,
                            nudge: 1,
                        }],
                    },
                ))
                .id()
        };

        let helm_ship = spawn_ruled(&mut app, crate::modifiers::power_system::HELM_POWER_GROUP);
        let sensors_ship = spawn_ruled(
            &mut app,
            crate::modifiers::power_system::SENSORS_POWER_GROUP,
        );

        power_tick_with_dt(&mut app, 4.0);

        // Ship A boosted helm, left sensors alone.
        assert_eq!(
            power_level(
                &app,
                helm_ship,
                crate::modifiers::power_system::HELM_POWER_GROUP
            ),
            3,
            "helm-ruled ship must raise its own helm group"
        );
        assert_eq!(
            power_level(
                &app,
                helm_ship,
                crate::modifiers::power_system::SENSORS_POWER_GROUP
            ),
            2,
            "helm-ruled ship must NOT touch sensors"
        );
        // Ship B boosted sensors, left helm alone.
        assert_eq!(
            power_level(
                &app,
                sensors_ship,
                crate::modifiers::power_system::SENSORS_POWER_GROUP
            ),
            3,
            "sensors-ruled ship must raise its own sensors group"
        );
        assert_eq!(
            power_level(
                &app,
                sensors_ship,
                crate::modifiers::power_system::HELM_POWER_GROUP
            ),
            2,
            "sensors-ruled ship must NOT touch helm"
        );
    }

    #[test]
    fn conflicting_rules_on_same_group_converge_without_error() {
        // AC4 conflicting rules at the system level: one ship authors TWO rules
        // that both target `weapons` (thrust and red alert). Both fire this
        // tick; the emitted admitted commands funnel through the single applier
        // and the group settles one step up (no double-apply, no error).
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .insert(crate::ship::power::PowerAiConfigResource {
                rules: vec![
                    crate::console_ai::PowerAiRule {
                        group: crate::modifiers::power_system::WEAPONS_POWER_GROUP.to_string(),
                        trigger: crate::console_ai::PowerRuleTrigger::Thrust { threshold: 0.7 },
                        min_battery_reserve: 50.0,
                        battery_recharge_pct: 100.0,
                        engage_delay_secs: 3.0,
                        nudge: 1,
                    },
                    crate::console_ai::PowerAiRule {
                        group: crate::modifiers::power_system::WEAPONS_POWER_GROUP.to_string(),
                        trigger: crate::console_ai::PowerRuleTrigger::RedAlert,
                        min_battery_reserve: 10.0,
                        battery_recharge_pct: 100.0,
                        engage_delay_secs: 3.0,
                        nudge: 1,
                    },
                ],
            });
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_plugin::LastHelmInput>()
            .unwrap()
            .thrust = 0.9;
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        power_tick_with_dt(&mut app, 4.0);

        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            3,
            "two rules on weapons both engage and converge to a single +1 step"
        );
    }

    #[test]
    fn human_held_power_reactor_rejects_ai_emission() {
        // A human holds the Power reactor. Unlike shields (coarse system vs.
        // per-arc fine systems), power's decide gate and admission target the
        // SAME system — the reactor — so `operate_ai` being false stops the
        // AI at the decide gate, and `validate_and_admit` would independently
        // refuse the same `ai:` token were it reached. Either way no admitted
        // command exists and the allocation never changes — the refusal the
        // retired `integrate_power_state` adapter could not express (it applied
        // intents unconditionally).
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .get_mut::<ShipSystemControlSources>()
            .unwrap()
            .0
            .set(
                crate::system_registry::power_reactor_system_id(),
                ControlSource::Human,
            );
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        let before = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        power_tick_with_dt(&mut app, 4.0);
        let after = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        assert_eq!(
            before, after,
            "an ai: emission targeting a human-held power reactor must be refused at admission"
        );
        assert!(
            app.world()
                .entity(e)
                .get::<crate::messages::AdmittedCommands>()
                .unwrap()
                .0
                .is_empty(),
            "the refused command must never reach AdmittedCommands"
        );
    }
    // ── AI torpedo loading (admitted-command path) ──────────────────

    /// One tube, `volley_max = 2`, no per-tube AI override — so its resolved
    /// `ai_target_count` is `volley_max`.
    fn torpedo_load_app(tube_source: ControlSource) -> (App, Entity) {
        let mut app = App::new();
        // `Sessions` because the emit goes through the admission seam
        // (`emit_ai_command`), which asks it about station tenure.
        app.insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .add_systems(
            Update,
            (
                ai_torpedo_load,
                crate::weapons_plugin::handle_set_torpedo_volley_target,
            )
                .chain(),
        );

        let mut control_sources = ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::torpedo_magazine_system_id(),
            ControlSource::Ai,
        );
        control_sources.0.set(
            crate::system_registry::torpedo_tube_system_id("fore_port").unwrap(),
            tube_source,
        );

        let torpedoes = crate::torpedo::TorpedoSystem::from_configs(
            &[crate::entity_config::TorpedoTubeConfig {
                id: "fore_port".into(),
                facing_deg: 0.0,
                fire_arc_deg: 90.0,
                load_time: None,
                marker: None,
                volley_max: 2,
                ai_target_count: None,
            }],
            crate::torpedo::TorpedoConfig::default(),
        );

        let e = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                control_sources,
                crate::weapons_plugin::TorpedoSystemResource(torpedoes),
                AdmittedCommands::default(),
            ))
            .id();
        (app, e)
    }

    fn tube_target_count(app: &App, e: Entity) -> u32 {
        app.world()
            .entity(e)
            .get::<crate::weapons_plugin::TorpedoSystemResource>()
            .unwrap()
            .0
            .tube("fore_port")
            .unwrap()
            .target_count
    }

    /// The gap this system closes: an AI-crewed ship now asks for its tubes to
    /// be loaded, and it does so through the same `SetTorpedoVolleyTarget`
    /// command a human console sends — so the order lands on an NPC's own
    /// torpedo system, which the LocalShip-only handler could never do.
    #[test]
    fn ai_torpedo_load_sets_volley_target_through_admitted_commands() {
        let (mut app, e) = torpedo_load_app(ControlSource::Ai);
        app.update();

        assert_eq!(
            tube_target_count(&app, e),
            2,
            "an AI-operated tube should be ordered to its configured \
             ai_target_count (volley_max = 2 here)"
        );
        let admitted = app.world().entity(e).get::<AdmittedCommands>().unwrap();
        assert_eq!(
            admitted.0.len(),
            1,
            "exactly one SetTorpedoVolleyTarget should have been issued"
        );
        assert!(
            matches!(
                admitted.0[0].payload,
                crate::messages::SystemControlPayload::SetTorpedoVolleyTarget { count: 2 }
            ),
            "the AI must issue the ordinary console command, not poke state"
        );
    }

    /// The tube is already where the AI wants it, so no second order goes out.
    #[test]
    fn ai_torpedo_load_does_not_reissue_an_identical_order() {
        let (mut app, e) = torpedo_load_app(ControlSource::Ai);
        app.update();
        app.update();

        let admitted = app.world().entity(e).get::<AdmittedCommands>().unwrap();
        assert_eq!(
            admitted.0.len(),
            1,
            "the AI must not re-issue a volley order the tube already satisfies"
        );
    }

    /// A human-crewed tube is the operator's to load. The AI must not touch it
    /// — this is the behaviour a non-zero `target_count` default in
    /// `TorpedoSystem::from_configs` would have broken.
    #[test]
    fn ai_torpedo_load_leaves_human_controlled_tubes_alone() {
        let (mut app, e) = torpedo_load_app(ControlSource::Human);
        app.update();

        assert_eq!(
            tube_target_count(&app, e),
            0,
            "a Human-controlled tube must stay exactly as its operator left it"
        );
        assert!(app
            .world()
            .entity(e)
            .get::<AdmittedCommands>()
            .unwrap()
            .0
            .is_empty());
    }

    /// Offline (rating- or damage-driven) means nobody loads it, AI included.
    #[test]
    fn ai_torpedo_load_skips_offline_tubes() {
        let (mut app, e) = torpedo_load_app(ControlSource::Offline);
        app.update();

        assert_eq!(tube_target_count(&app, e), 0);
    }
}

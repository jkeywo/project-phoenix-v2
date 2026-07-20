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
/// that marker at every spawn/promote site (mirrors `PowerReactorIntents`'s
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
                ai_power_allocation
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel),
                integrate_power_state
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(ai_power_allocation),
                // Decide only. The apply half is
                // `console::weapons::integrate_weapons_state`
                // (issue #698), which drains `TorpedoIntents` and
                // `PhaserIntents` together and is registered by
                // `WeaponsPlugin` — see its docs for why the weapons
                // integrator does not live here.
                ai_torpedo_auto_fire
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel),
                ai_frequency_hint.in_set(crate::sim_sets::SimSet::Input),
            ),
        );
    }
}

// ── Shields AI ───────────────────────────────────────────────────────────────

/// Validate-and-enqueue one shield-focus AI decision into this ship's own
/// `AdmittedCommands` (issue #826, mirroring `ship::helm_ai::
/// emit_helm_ai_command` from #824): the AI's `ai:` token flows through the
/// same `validate_and_admit` seam network commands do, checked against this
/// entity's own `ControlSourceResolver` (`operate_ai` must hold on the
/// targeted arc). The write happens in the same tick —
/// `ship::shields::handle_shields_messages` applies it later this frame — so
/// there is no one-tick queue lag on the AI shields path.
fn emit_shield_ai_command(
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    target: crate::messages::SystemId,
    payload: crate::messages::SystemControlPayload,
    sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
    admitted: &mut crate::messages::AdmittedCommands,
) -> bool {
    let token = entity_uuid
        .map(|u| format!("ai:{}", u.0))
        .unwrap_or_else(|| "ai:backfill".to_string());
    let default_config;
    let config = match ship_config {
        Some(c) => &c.0,
        None => {
            default_config = crate::ship::config::ShipConfig {
                stations: vec![],
                systems: vec![],
                power_groups: std::collections::HashMap::new(),
                coordination_lag_secs: 0.0,
            };
            &default_config
        }
    };
    crate::command_admission::validate_and_admit(
        &token, target, payload, sources, sessions, config, admitted,
    )
}

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
    global_ai_config: Res<crate::ship::shields::ShieldsAiConfigResource>,
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
            emit_shield_ai_command(
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

        let ai_cfg: &crate::ship::shields::ShieldsAiConfigResource =
            ai_config_comp.unwrap_or(&*global_ai_config);
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
                let delta = prev_hp - facing.hp;
                damage_history.record_damage(idx, current_time, delta);
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
                    emit_shield_ai_command(
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

/// AI power-allocation decision system (issue #693).
///
/// Wires the previously-orphaned `console_ai::tick_power_movement_rule` and
/// `console_ai::tick_power_red_alert_rule` pure functions: sustained thrust
/// nudges the helm power group +1 (and drops it -1 once battery/thrust
/// conditions lapse); sustained red alert does the same to the weapons power
/// group. Writes the decision into `PowerReactorIntents` for
/// `integrate_power_state` to apply, rather than mutating `ShipPowerSystem`
/// directly.
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
/// `NoChange` emits no intent. `PowerSystem::set_group_allocation` clamps to
/// `[1, 4]` and enforces the total<=8 cap in `integrate_power_state`, so
/// passing a saturated/clamped-ish value through here is safe regardless.
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
    ai_config_res: Option<Res<crate::ship::power::PowerAiConfigResource>>,
    config_res: Option<Res<crate::ship::power::PowerConfigResource>>,
    mut ships: Query<
        (
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship::power::ShipPowerSystem,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::ship_plugin::LastHelmInput>,
            Option<&crate::ship::power::PowerConfigResource>,
            Option<&crate::ship::power::PowerAiConfigResource>,
            &mut crate::ship::power::ShipPowerAiState,
            &mut crate::ship::power::PowerReactorIntents,
        ),
        (
            With<crate::ai_plugin::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
) {
    let dt = time.delta_secs();

    // Three-step "shadow default, deref-or-fallback" idiom — copied verbatim
    // from the deleted `operate_power_ai` for both config reads.
    let ai_cfg_default;
    let ai_cfg_fallback: &crate::ship::power::PowerAiConfigResource = match ai_config_res.as_deref()
    {
        Some(c) => c,
        None => {
            ai_cfg_default = crate::ship::power::PowerAiConfigResource::default();
            &ai_cfg_default
        }
    };
    let cfg_default;
    let cfg_fallback: &crate::ship::power::PowerConfigResource = match config_res.as_deref() {
        Some(c) => c,
        None => {
            cfg_default = crate::ship::power::PowerConfigResource::default();
            &cfg_default
        }
    };

    for (
        control_sources,
        power,
        red_alert_comp,
        last_helm_comp,
        cfg_comp,
        ai_cfg_comp,
        mut ai_state,
        mut intents,
    ) in ships.iter_mut()
    {
        // Clear last tick's intents unconditionally — a stale intent must
        // never survive into a tick where the decision didn't run.
        intents.0.clear();

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

        let cfg: &crate::ship::power::PowerConfigResource = cfg_comp.unwrap_or(cfg_fallback);
        let ai_cfg: &crate::ship::power::PowerAiConfigResource =
            ai_cfg_comp.unwrap_or(ai_cfg_fallback);

        let red_alert = red_alert_comp.map(|ra| ra.0).unwrap_or(false);
        let thrust = last_helm_comp.map(|l| l.thrust).unwrap_or(0.0);
        let power_is_low = true; // Rating gate deferred to per-ship AiTuning

        let battery_pct = if cfg.0.capacity > 0.0 {
            (power.0.battery_charge / cfg.0.capacity) * 100.0
        } else {
            0.0
        };

        let helm_id =
            crate::messages::PowerGroupId(crate::modifiers::power_system::HELM_POWER_GROUP.into());
        let weapons_id = crate::messages::PowerGroupId(
            crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
        );

        // ── Movement rule → helm group ───────────────────────────────────
        let movement_input = crate::console_ai::PowerMovementInput {
            thrust,
            thrust_threshold: ai_cfg.movement_thrust_threshold,
            engage_delay_secs: ai_cfg.movement_engage_delay_secs,
            battery_engage_min_pct: ai_cfg.movement_battery_engage_min_pct,
            battery_recharge_pct: ai_cfg.movement_battery_recharge_pct,
            battery_pct,
            dt,
            power_is_low,
        };
        let movement_output =
            crate::console_ai::tick_power_movement_rule(&mut ai_state.movement, &movement_input);
        match movement_output {
            crate::console_ai::PowerEngageOutput::Engage => {
                let current = power.0.level_for(&helm_id);
                intents.0.push(crate::ship::power::PowerReactorCommand {
                    group: helm_id.clone(),
                    level: current.saturating_add(1),
                });
            }
            crate::console_ai::PowerEngageOutput::Disengage => {
                let current = power.0.level_for(&helm_id);
                intents.0.push(crate::ship::power::PowerReactorCommand {
                    group: helm_id.clone(),
                    level: current.saturating_sub(1),
                });
            }
            crate::console_ai::PowerEngageOutput::NoChange => {}
        }

        // ── Red-alert rule → weapons group ───────────────────────────────
        let red_alert_input = crate::console_ai::PowerRedAlertInput {
            red_alert,
            engage_delay_secs: ai_cfg.red_alert_engage_delay_secs,
            battery_engage_min_pct: ai_cfg.red_alert_battery_engage_min_pct,
            battery_recharge_pct: ai_cfg.red_alert_battery_recharge_pct,
            battery_pct,
            dt,
            power_is_low,
        };
        let red_alert_output =
            crate::console_ai::tick_power_red_alert_rule(&mut ai_state.red_alert, &red_alert_input);
        match red_alert_output {
            crate::console_ai::PowerEngageOutput::Engage => {
                let current = power.0.level_for(&weapons_id);
                intents.0.push(crate::ship::power::PowerReactorCommand {
                    group: weapons_id.clone(),
                    level: current.saturating_add(1),
                });
            }
            crate::console_ai::PowerEngageOutput::Disengage => {
                let current = power.0.level_for(&weapons_id);
                intents.0.push(crate::ship::power::PowerReactorCommand {
                    group: weapons_id.clone(),
                    level: current.saturating_sub(1),
                });
            }
            crate::console_ai::PowerEngageOutput::NoChange => {}
        }
    }
}

/// Adapter: applies `PowerReactorIntents` written by `ai_power_allocation`
/// to `ShipPowerSystem::set_group_allocation` — the same mutation primitive
/// `ship::power::handle_power_messages` (the human path) uses. Runs
/// immediately after `ai_power_allocation` in the same tick so the
/// reallocation is visible to `tick_power_system` /
/// `publish_power_blackboard` this frame.
///
/// **Dual-write.** Mirrors `handle_power_messages`'s `Has<LocalShip>` +
/// Resource-sync pattern: when the mutated entity is the `LocalShip`, also
/// snapshot the updated per-entity Component into the global `ShipPowerSystem`
/// Resource (legacy Resource path for tests). This matters because a
/// disconnected player's Power station can flip to Backfill AI (AGENTS.md
/// rule 5), so `ai_power_allocation` / `integrate_power_state` can
/// legitimately be the system driving the player's own ship's power grid.
fn integrate_power_state(
    mut ships: Query<(
        &mut crate::ship::power::ShipPowerSystem,
        &mut crate::ship::power::PowerReactorIntents,
        Has<crate::server_app::LocalShip>,
    )>,
    power_res: Option<ResMut<crate::ship::power::ShipPowerSystem>>,
) {
    let mut power_res = power_res;
    for (mut power, mut intents, is_local) in ships.iter_mut() {
        if intents.0.is_empty() {
            continue;
        }
        for cmd in intents.0.drain(..) {
            let _ = power.0.set_group_allocation(&cmd.group, cmd.level);
        }
        // Dual-write: keep the Resource in sync with the LocalShip's
        // Component (legacy Resource path for tests).
        if is_local {
            if let Some(pr) = power_res.as_deref_mut() {
                pr.0 = power.0.clone();
            }
        }
    }
}

// ── Torpedo AI (issue #694, completed by #698) ──────────────────────────────

/// AI torpedo auto-fire decision system (issue #694; promoted to full in #698).
///
/// Wires the previously-orphaned `console_ai::auto_fire_torpedo`: for ships
/// whose Tactical target is already locked (`WeaponsTarget`, written by
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
///   the world this tick. A `WeaponsTarget` naming a destroyed entity is not a
///   lock. (Pre-#698 this position lookup happened anyway, purely to compute
///   bearing; the difference is that failing it is now expressed as
///   `target_locked = false` rather than an early `continue`.)
/// - `target_shields` — the sum of the target's **online** shield facings' HP,
///   read from its `ShipShields`. `auto_fire_torpedo` only fires when this is
///   `<= 0`, which is the documented doctrine: phasers strip the shields,
///   torpedoes finish the hull. Offline facings are excluded because they let
///   damage through, so they are not blocking the shot. A target with no
///   `ShipShields` at all (asteroids, debris) contributes 0 and is therefore
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
            &crate::ship_plugin::ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ActiveStationRatings,
            &crate::ship_state::ShipPhysics,
            &crate::weapons_plugin::WeaponsTarget,
            Option<&crate::weapons_plugin::TorpedoSystemResource>,
            &mut crate::weapons_plugin::TorpedoIntents,
        ),
        (
            With<crate::ai_plugin::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
    torpedo_sys_res: Res<crate::weapons_plugin::TorpedoSystemResource>,
    asteroid_q: Query<
        (&crate::simulation::AsteroidUuid, &Transform),
        With<crate::simulation::Asteroid>,
    >,
    other_ships_q: Query<
        (
            &crate::entity_spawner::EntityUuid,
            &Transform,
            Option<&crate::ship::shields::ShipShields>,
        ),
        Without<crate::simulation::Asteroid>,
    >,
) {
    let policy_sid = crate::system_registry::torpedo_magazine_system_id();

    for (
        ship_config,
        control_sources,
        active_ratings,
        physics,
        weapons_target,
        torpedo_sys_comp,
        mut intents,
    ) in ships.iter_mut()
    {
        // Clear last tick's intents unconditionally — a stale intent must
        // never survive into a tick where the decision didn't run.
        intents.0.clear();

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

        let Some(target_uuid) = weapons_target.0.clone() else {
            continue;
        };

        // Look up live world position + shields — the WorldResource snapshot is
        // stale for moving targets. A target that resolves here is a real lock
        // (`target_locked`); one that does not is a UUID naming something that
        // no longer exists, so there is nothing to shoot at.
        let target_state = asteroid_q
            .iter()
            .find_map(|(u, t)| {
                (u.0 == target_uuid).then_some(((t.translation.x, t.translation.z), None))
            })
            .or_else(|| {
                other_ships_q.iter().find_map(|(u, t, shields)| {
                    (u.0 == target_uuid).then_some(((t.translation.x, t.translation.z), shields))
                })
            });
        let Some(((tx, tz), target_shields_comp)) = target_state else {
            continue;
        };

        // Combined shield HP across the target's ONLINE facings. Offline
        // facings pass damage through to the hull, so they are not blocking the
        // torpedo and must not count towards "shields still up". Targets with
        // no `ShipShields` (asteroids, debris) read 0 — torpedo-eligible.
        let target_shields: i32 = target_shields_comp
            .map(|s| {
                s.0.facings
                    .iter()
                    .filter(|f| f.is_online())
                    .map(|f| f.hp)
                    .sum()
            })
            .unwrap_or(0);

        let dx = tx - physics.x;
        let dz = tz - physics.z;
        let world_bearing = dx.atan2(-dz);
        let bearing = world_bearing - physics.yaw;

        // Prefer per-entity component; fall back to global resource for
        // legacy test paths that only set up the Resource.
        let torpedo_sys: &crate::torpedo::TorpedoSystem = match torpedo_sys_comp {
            Some(c) => &c.0,
            None => &torpedo_sys_res.0,
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
            // Reaching here means `WeaponsTarget` named an entity that
            // resolved to a live world position above — a real lock.
            target_locked: true,
            target_shields,
            tubes,
            magazine,
        };

        let tubes_to_fire = crate::console_ai::auto_fire_torpedo(&input);
        for tube_id in tubes_to_fire {
            intents.0.push(crate::weapons_plugin::TorpedoCmd {
                tube_id,
                target_uuid: target_uuid.clone(),
            });
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
    global_ai_config: Res<crate::ship::sensors::SensorsAiConfigResource>,
    sessions: Option<Res<crate::lobby::Sessions>>,
    mut ships: Query<
        (
            Entity,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::weapons_plugin::WeaponsTarget,
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
        weapons_target,
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

        let locked_target = weapons_target.0.clone();

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

        let ai_cfg: &crate::ship::sensors::SensorsAiConfigResource =
            ai_config_comp.unwrap_or(&*global_ai_config);

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
    use crate::weapons_plugin::WeaponsTarget;

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
            .add_systems(Update, ai_frequency_hint)
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
                WeaponsTarget(Some("target-1".into())),
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

    // ── ai_power_allocation / integrate_power_state ─────────────────────────

    fn power_test_app() -> App {
        let mut app = App::new();
        // Manual `Time::advance_by` (mirroring `ai_frequency_hint`'s test
        // app above) rather than `TimePlugin` + `TimeUpdateStrategy`.
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
            .init_resource::<crate::ship::power::PowerAiConfigResource>()
            .add_systems(
                Update,
                (
                    ai_power_allocation.before(integrate_power_state),
                    integrate_power_state,
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
            crate::ship::power::PowerReactorIntents::default(),
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
    fn integrate_power_state_dual_writes_resource_for_local_ship() {
        // Issue #693 review finding: when the AI path reallocates power for
        // the LocalShip (e.g. a disconnected player's station backfilled to
        // AI per AGENTS.md rule 5), `integrate_power_state` must dual-write
        // the legacy global `ShipPowerSystem` Resource, mirroring
        // `ship::power::handle_power_messages`'s dual-write for the human
        // path — not just the per-entity Component.
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
}

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
//! - `tick_frequency_hint_high_fidelity` (named `ai_frequency_hint` until issue
//!   #873 took the operator branch out of it) — wires
//!   `console_ai::tick_frequency_hint`, which had no caller anywhere prior to
//!   this issue.
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

use crate::simmath;
use bevy::prelude::*;

use crate::command_admission::ai_emit::emit_ai_command;
use crate::console_ai::shields_emit::emit_shields_ai_command;

// AI rule keys — match the keys used in [[station.rating]].ai_tuning tables.
pub const AI_RULE_TORPEDO_AUTO_FIRE: &str = "torpedo_auto_fire";
pub const AI_RULE_FREQUENCY_MATCH: &str = "frequency_match";
// `AI_RULE_AUTO_HINT` ("auto_hint") was deleted by issue #873. It gated the
// Sensors frequency hint on whether a *human session* held the Sensors station
// and, if so, on that holder's active rating — so a coordination fact derived
// entirely from authoritative ship state stopped being emitted the moment a
// human sat down. That is the human/AI branch AGENTS.md rule 6 forbids, and no
// shipped hull authored the key anyway. Do not reintroduce it: a station rating
// tunes what a console offers its own operator, never whether the ship's state
// reaches the rest of the bridge.
pub const AI_RULE_MOVEMENT_RULE: &str = "movement_rule";
pub const AI_RULE_RED_ALERT_RULE: &str = "red_alert_rule";

/// Per-ship persistent state for `tick_frequency_hint_high_fidelity`'s
/// delayed-hint timer.
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
        // The ONE shared AI decision cadence (issues #889, #895). Every decider
        // below used to be UNGATED, and while `SimSet` was configured in Bevy's
        // `Update` that meant one decision per rendered frame — at display
        // refresh rate, over a `WorldSnapshot` rebuilt on an unrelated clock.
        // They now share the helm axes' latch, derived from the logical tick.
        crate::ai::cadence::register_ai_cadence(app);
        app.add_systems(
            FixedUpdate,
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
                    .before(crate::ship::shields::handle_shields_messages)
                    .run_if(crate::ai::cadence::ai_tick_ready),
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
                    .before(crate::ship::power::handle_power_messages)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                // Decide only. The apply half is
                // `console::weapons::handle_fire_torpedo` (issue #846 —
                // previously `integrate_weapons_state`, which drained the
                // retired `TorpedoIntents`), registered by `WeaponsPlugin` in
                // this same `Physics` set.
                //
                // The `.before` edge is NOT decoration, and its absence was a
                // live production bug: both systems sat in `SimSet::Physics`
                // with no edge between them, and the resolved order put the
                // CONSUMER first. The admitted `FireTorpedo` therefore sat in
                // `AdmittedCommands` untouched until `clear_before_input`
                // wiped it at the top of the next tick, so an AI-crewed ship
                // never launched a torpedo in a real run — every unit test
                // passed only because the weapons harness adds this edge
                // itself. Same class as #881 (an AI-emitted command whose
                // applier was never ordered against the emitter); the
                // shields/power siblings above carry the identical edge for
                // the identical reason.
                ai_torpedo_auto_fire
                    .in_set(crate::sim_sets::SimSet::Physics)
                    .after(crate::sim_sets::AiTickLabel)
                    .before(crate::console::weapons::handle_fire_torpedo)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                // The loading half of the torpedo AI. `Input`, not `Physics`,
                // and explicitly before the volley-target handler: the command
                // it emits has to be consumed in the SAME tick, exactly as
                // `operate_captain_ai` is ordered before
                // `handle_set_red_alert`. That also puts `target_count` in
                // place before `tick_torpedo_lifecycle` (Physics) runs its
                // auto-load block, so a tube starts loading the tick the order
                // is given rather than the tick after.
                //
                // Gated on the shared base cadence like its launch half. The
                // two run in different sets (`Input` load, `Physics` launch)
                // but on the SAME latch, so a tube that is loaded and ready
                // still launches in the tick the launch decider next fires —
                // quantising both halves does not insert an extra tick between
                // them.
                ai_torpedo_load
                    .in_set(crate::sim_sets::SimSet::Input)
                    .before(crate::weapons_plugin::handle_set_torpedo_volley_target)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                // Not an AI-operator decider despite living in this plugin
                // (issue #873): it emits the ship's Sensors frequency advisory
                // for every high-fidelity hull regardless of who holds Sensors.
                // It stays under `ai_tick_ready` because its reaction-delay
                // model advances by the authored tick period, not `Time::delta`.
                tick_frequency_hint_high_fidelity
                    .in_set(crate::sim_sets::SimSet::Input)
                    .run_if(crate::ai::cadence::ai_tick_ready),
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
    // Read-only scenario flag/counter chain (issue #891 stage 2). `Option` so
    // bare-`App` fixtures still pass parameter validation; absent, the chain is
    // empty and flag-guards read false.
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship::shields::ShipShields,
            &mut crate::ship::shields::ShieldsDamageHistory,
            Option<&crate::ship::shields::ShieldsFocusAiPolicy>,
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
        ship_entity,
        entity_uuid,
        control_sources,
        shields,
        mut damage_history,
        focus_policy_comp,
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

        // Per-ship inline stateless Shields focus policy (issue #783). The
        // authored windows/thresholds live in the policy `param` map, and the
        // policy gates WHETHER the retained arc-ranking kernel acts this tick.
        //
        // A ship without the component takes no damage-analysis action at all.
        // Since #885b stage 5d there is no synthesised stand-in: strict
        // AI-declaration mode rejects an AI-capable hull that omits
        // `[shields_console.ai_policy]` at load, so an absent component means the
        // declaration is missing and a missing declaration gets no automation
        // (PRD #774 US7).
        let Some(focus_policy_comp) = focus_policy_comp else {
            continue;
        };
        let policy: &crate::ai::policy::AiPolicy = &focus_policy_comp.0;

        // Authored windows/thresholds, read from the policy `param` map INSTEAD of
        // the typed `ai_cfg.*` (issue #783). Absent params fall back to the
        // retained typed defaults so a hand-built policy that omits one still
        // behaves — but the canonical default seeds all four.
        let cfg_default = crate::ship::shields::ShieldsAiConfigResource::default();
        let damage_window_secs = policy
            .params
            .get(crate::entities::config::SHIELD_FOCUS_DAMAGE_WINDOW_PARAM)
            .map(|v| v as f32)
            .unwrap_or(cfg_default.damage_window_secs);
        let min_damage_window_secs = policy
            .params
            .get(crate::entities::config::SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM)
            .map(|v| v as f32)
            .unwrap_or(cfg_default.min_damage_window_secs);
        let damage_pct_threshold = policy
            .params
            .get(crate::entities::config::SHIELD_FOCUS_DAMAGE_PCT_PARAM)
            .map(|v| v as f32)
            .unwrap_or(cfg_default.damage_pct_threshold);
        let health_ratio_threshold = policy
            .params
            .get(crate::entities::config::SHIELD_FOCUS_HEALTH_RATIO_PARAM)
            .map(|v| v as f32)
            .unwrap_or(cfg_default.health_ratio_threshold);

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
        damage_history.prune_old(current_time, damage_window_secs);

        // ── Build AI input ──────────────────────────────────────────────────
        let facings_snapshot: Vec<_> = facings.iter().map(|f| f.snapshot()).collect();
        let shields_is_low = true; // Rating gate deferred to per-ship AiTuning

        // ── Resolve the authored policy gate (issue #783) ───────────────────
        // Seed BOUNDED per-arc recent-damage facts from the already-pruned
        // window, then resolve the `shield_focus` channel. `focus_shield_arc`
        // (act) → run the retained ranking kernel; `None` (idle/hold) → emit
        // nothing this tick.
        let facts = crate::console_ai::seed_shields_focus_facts(
            &facings_snapshot,
            &damage_history.arcs,
            damage_window_secs,
            min_damage_window_secs,
            current_time,
        );
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(ship_entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );
        let acts = policy.resolve_channel(
            crate::entities::config::SHIELD_FOCUS_CHANNEL,
            &facts,
            &flag_chain,
        ) == Some(&crate::ai::policy::AiPolicyVerb::FocusShieldArc);
        if !acts {
            continue;
        }

        let input = crate::console_ai::ShieldFocusAiInput {
            facings: facings_snapshot,
            shields_is_low,
            damage_history: damage_history.arcs.clone(),
            damage_window_secs,
            min_damage_window_secs,
            damage_pct_threshold,
            health_ratio_threshold,
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

/// AI power-allocation decision system (issue #784: inline stateless policy
/// spine; admitted transport #831).
///
/// Reworked from the retired stateful engine (`PowerAiConfigResource` +
/// `ShipPowerAiState` `EngageState` hysteresis, #762) onto the same inline
/// stateless `AiPolicy` spine as #779–#783. For each of the ship's AUTHORED
/// power groups it resolves that group's own output channel — the channel IS the
/// power group id, dynamic per-ship data, NOT a fixed catalogue (AC1) — over an
/// immutable per-tick fact snapshot, and emits the winning
/// `SetPowerGroupAllocation(level)` verb's absolute target as an admitted
/// `SetPowerGroupAllocation` payload targeting `POWER_REACTOR_SYSTEM_ID`, for
/// `ship::power::handle_power_messages` to apply later this tick — the same
/// admitted seam a human Power operator uses (AGENTS.md rule #6). It never writes
/// `ShipPowerSystem` directly.
///
/// # Gating (AC5 human Control Source)
/// - `ShipSystemControlSources.policy_for(power_reactor_system_id()).operate_ai`
///   — a human holding the Power station flips this to false, so the AI stands
///   down at the decide gate. No per-tick state to reset (statelessness removed
///   the last `EngageState` timer, AGENTS.md rule #7), so regaining AI control
///   the next tick yields a clean decision from the fresh snapshot (lifecycle
///   reset is trivially correct).
/// - `AiHighFidelity` (query filter).
///
/// # Facts + scenario flags (AC3)
/// `seed_power_facts` builds the immutable SHIP/THREAT/OBJECTIVE/SYSTEM fact
/// snapshot host-side (the #779 empty-facts lesson: a guard never fires unless
/// the host seeds the fact). The read-only scenario `flags` chain — anchored at
/// the layer that spawned the ship and terminating at
/// `WorldContentRuntime.flags` (issue #891 stage 2) — is passed to
/// `resolve_channel` so authored guards can gate on world flags/counters: the
/// read surface #784 introduced and #891 spread to every host.
///
/// # Brownout avoidance (AC5), no global emergency exception
/// Each rule's reserve is an authored `param` referenced by its `when` guard
/// (`fact(battery_pct) >= param(min_reserve)`). Below the reserve the elevate
/// guard does not fire, so allocation never rises when the battery can't sustain
/// it — the per-rule reserve guards REPLACE the retired global emergency
/// exception. The applier's drain/exhaustion-lock/recovery is unchanged.
///
/// # Absolute-level emit, spent against the reactor's budget (issue #959)
/// The verb carries an absolute target level, and the levels for ALL of the
/// ship's groups are decided together rather than one channel at a time.
/// [`crate::modifiers::power_system::plan_allocation`] takes each group's bid —
/// the winning rule's level, that rule's authored `priority`, and the group's
/// authored `max_level` — reserves the budget the un-bid groups are already
/// holding, and hands out what is left in authored-priority order. The host emits **only the
/// groups whose commanded level actually changes**, decreases first, so a
/// settled ship emits nothing.
///
/// That is the fix for the silent cap-refusal loop. The applier's re-clamp to
/// `[1, 4]` and to the ship-wide
/// reactor's authored `max_commanded_total` is still there as the
/// backstop it always was, but it no longer has anything to catch: a plan that
/// cannot be refused cannot be re-emitted for ever either.
fn ai_power_allocation(
    time: Res<Time>,
    sessions: Res<crate::lobby::Sessions>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
    // Read-only scenario flag/counter store (AC3). `Option<Res<_>>` so bare-`App`
    // fixtures that never insert `WorldContentRuntime` still pass parameter
    // validation; absent, the flag chain is empty and flag-guards read false.
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    // Loaded sub-world layers (issue #891 stage 2): the chain is anchored at
    // the layer that spawned each ship, `parent:`-walkable to the base store.
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // Global objective pool for the OBJECTIVE fact. `Option<Res<_>>` for the same
    // bare-`App` reason. Scored once per tick, outside the per-ship loop.
    objectives: Option<Res<crate::world::server::ObjectiveManagerRes>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    // The shared AI base cadence's raw tick + interval (issue #889's
    // evaluate_every_ticks, wired at runtime). `Option<Res<_>>` for the usual
    // bare-`App` reason: `power_test_app` below never calls
    // `register_ai_cadence`, so these read the same (0, 1) fallback
    // `evaluate_every_ticks_ready` already treats as "always due" — identical
    // to this system's pre-existing (ungated w.r.t. per-host cadence)
    // behaviour in every such fixture.
    tick: Option<Res<crate::sim_tick::SimTick>>,
    base_interval: Option<Res<crate::ai::cadence::AiBaseInterval>>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship::power::ShipPowerSystem,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::ship_plugin::LastHelmInput>,
            Option<&crate::ship::power::PowerConfigResource>,
            Option<&crate::ship::power::PowerAiPolicy>,
            Option<&crate::ship::power::PowerAiCadence>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::ship::combat_activity::RecentCombatActivity>,
            &mut crate::messages::AdmittedCommands,
        ),
        (
            With<crate::ai_plugin::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
) {
    let now = time.elapsed_secs();
    let tick = tick.map(|t| t.0).unwrap_or(0);
    let base_interval = base_interval.map(|b| b.0).unwrap_or(1);

    // OBJECTIVE fact, scored once per tick: does the active pool carry a Destroy
    // directive? A broad "the ship has something to kill" signal an authored rule
    // may use to bias weapons power.
    let has_destroy_objective = objectives
        .as_ref()
        .map(|om| {
            om.0.scored_pool(&crate::objectives::WorldConditions::default())
                .iter()
                .any(|s| matches!(s.directive, crate::messages::AiDirective::Destroy { .. }))
        })
        .unwrap_or(false);

    for (
        ship_entity,
        entity_uuid,
        control_sources,
        power,
        red_alert_comp,
        last_helm_comp,
        cfg_comp,
        policy_comp,
        cadence_comp,
        ship_config,
        combat_activity,
        mut admitted,
    ) in ships.iter_mut()
    {
        let control_policy = control_sources
            .0
            .policy_for(&crate::system_registry::power_reactor_system_id());
        if !control_policy.operate_ai {
            // Not (or no longer) AI-driven — human Control Source. Stateless, so
            // nothing to reset; the next tick under AI control decides cleanly.
            continue;
        }

        // Per-entity power config only (issue #738 isolation): the parse-time
        // default each ship is spawned with, never a player-ship global.
        let cfg_default;
        let cfg: &crate::ship::power::PowerConfigResource = match cfg_comp {
            Some(c) => c,
            None => {
                cfg_default = crate::ship::power::PowerConfigResource::default();
                &cfg_default
            }
        };

        // No attached `[power.ai_policy]` ⇒ no allocation decisions, and every
        // group holds the level the reactor seeded. Since #885b stage 5d there
        // is no synthesised stand-in.
        let Some(policy_comp) = policy_comp else {
            continue;
        };
        let policy: &crate::ai::policy::AiPolicy = &policy_comp.0;

        // Per-host multiplier on the shared base cadence (issue #889's
        // evaluate_every_ticks, wired at runtime): a ship whose
        // `[power.ai_policy]` authors `evaluate_every_ticks = n` decides on
        // every Nth arm of `ai_tick_ready`, not every arm. `1` (every shipped
        // hull today) reduces this to a no-op — see `AiBaseInterval`'s docs.
        let evaluate_every_ticks = cadence_comp.map(|c| c.0).unwrap_or(1);
        if !crate::ai::cadence::evaluate_every_ticks_ready(
            tick,
            base_interval,
            evaluate_every_ticks,
        ) {
            continue;
        }

        // The read-only scenario flag chain (AC3), anchored at the layer that
        // spawned THIS ship (issue #891 stage 2) — correctly layered, so
        // `parent:` prefixes climb toward the base store exactly as a trigger
        // authored in that layer would.
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(ship_entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );

        let red_alert = red_alert_comp.map(|ra| ra.0).unwrap_or(false);
        let thrust = last_helm_comp.map(|l| l.thrust).unwrap_or(0.0);
        let battery_pct = if cfg.0.capacity > 0.0 {
            (power.0.battery_charge / cfg.0.capacity) * 100.0
        } else {
            0.0
        };
        let secs_since_combat = combat_activity.and_then(|a| {
            let last = most_recent_combat(a);
            last.map(|s| now - s)
        });

        // Immutable per-tick fact snapshot. `offline_system_count` is seeded 0
        // for now (a fuller per-system availability seed from blackboards is
        // future work); the fact still exists so an authored guard referencing it
        // validates and reads a real value.
        let facts = crate::ship::power::seed_power_facts(
            &power.0,
            battery_pct,
            thrust,
            red_alert,
            secs_since_combat,
            None,
            has_destroy_objective,
            0,
        );

        // Collect the ship's AUTHORED power groups' bids (dynamic channels,
        // AC1). For each group resolve its channel; the highest-priority
        // matching rule wins (AC4) and its verb carries the absolute level it
        // asks the group to hold, alongside that rule's own authored priority.
        // A group with no matching rule resolves `None` and bids for nothing —
        // it holds whatever it was last commanded to, and `plan_allocation`
        // reserves the budget for it.
        //
        // Nothing is emitted from this loop. Issue #959: a per-group emit made
        // in ignorance of what the other groups had asked for is exactly how the
        // silent cap refusal happened — the applier's budget check refuses the
        // surplus without an error, and the next decision arm asks again.
        let group_ids: Vec<crate::messages::PowerGroupId> =
            power.0.iter().map(|(id, _)| id.clone()).collect();
        let mut bids: Vec<crate::modifiers::power_system::AllocationBid> =
            Vec::with_capacity(group_ids.len());
        for group_id in &group_ids {
            // Only the allocation verb ever resolves on a power-group channel
            // (the policy is validated to carry no other), so an `if let` is
            // exhaustive in practice; any other verb holds (no bid).
            if let Some((
                rule_priority,
                crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(level),
            )) = policy.resolve_channel_ranked(&group_id.0, &facts, &flag_chain)
            {
                bids.push(crate::modifiers::power_system::AllocationBid {
                    group: group_id.clone(),
                    want: *level,
                    // This group's own authored ceiling. A hull that declares no
                    // `[power_groups.*]` block at all has none to read, so the
                    // parse default stands in — the same value its groups were
                    // seeded and are clamped by.
                    max_level: ship_config
                        .and_then(|sc| sc.0.power_groups.get(group_id))
                        .map(|g| g.max_level)
                        .unwrap_or_else(crate::ship::config::default_max_power_level),
                    rule_priority,
                });
            }
        }

        // Spend the reactor's budget across those bids (issue #959). What comes
        // back is a set of levels that FITS — nothing in it can be refused —
        // with every decrease ahead of every increase so no intermediate state
        // trips the cap either, and with no-ops already dropped so a settled
        // ship emits nothing at all.
        //
        // No-op suppression compares against the COMMANDED level, not the
        // effective one (issue #952), and `plan_allocation` inherits that. A
        // group the battery floor is holding down reads lower than it was set
        // to, and comparing against that reading would make the AI think its own
        // standing order had already been carried out: it would stop correcting
        // the order downward for as long as the brownout lasted, and the group
        // would snap back to a level nobody had asked for since the fight
        // started the moment the reserve recovered. The floor is the REACTOR's
        // business; what the crew has asked for is this policy's, and the two
        // are allowed to disagree.
        for (group_id, level) in crate::modifiers::power_system::plan_allocation(&power.0, &bids) {
            crate::pinfo!(
                log,
                crate::logging::LogCat::Power,
                entity = ship_entity,
                "{} power {} -> {} (policy)",
                group_id.0,
                power.0.commanded_level_for(&group_id),
                level
            );
            emit_ai_command(
                entity_uuid,
                crate::system_registry::power_reactor_system_id(),
                crate::messages::SystemControlPayload::SetPowerGroupAllocation {
                    group: group_id,
                    level,
                },
                control_sources,
                &sessions,
                ship_config,
                &mut admitted,
            );
        }
    }
}

/// Most-recent combat timestamp for the THREAT `secs_since_combat` fact,
/// mirroring `operate_captain_ai`'s reduction over the same activity fields.
fn most_recent_combat(a: &crate::ship::combat_activity::RecentCombatActivity) -> Option<f32> {
    [
        a.last_damage_taken,
        a.last_hostile_fire_taken,
        a.last_weapon_fired,
    ]
    .into_iter()
    .flatten()
    .fold(None, |acc: Option<f32>, s| {
        Some(acc.map_or(s, |cur| cur.max(s)))
    })
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
///   moments in the shot's life.) Since issue #956 nothing in Rust compares it
///   to anything: it is seeded into the tube's launch snapshot and the tube's
///   own authored `fact(target_facing_shields) <= 0` guard is what holds fire.
///   Phasers strip the shields, torpedoes finish the hull — said in TOML.
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
    // Read-only scenario flag/counter chain (issue #891 stage 2). `Option` so
    // bare-`App` fixtures still pass parameter validation.
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ActiveStationRatings,
            &crate::ship_state::ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::weapons_plugin::TorpedoSystemResource>,
            Option<&crate::weapons_plugin::TorpedoTubeAiPolicies>,
            &mut crate::messages::AdmittedCommands,
            // Issue #872: this ship's own red-alert state, seeded as a typed
            // fact for the tube's authored LAUNCH predicate. `Option<&_>` for
            // fixtures that spawn a ship without it; absent reads `false`.
            Option<&crate::ship_state::ShipRedAlert>,
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
        ship_entity,
        entity_uuid,
        ship_config,
        control_sources,
        active_ratings,
        physics,
        blackboards,
        torpedo_sys_comp,
        tube_policies,
        mut admitted,
        red_alert_opt,
    ) in ships.iter_mut()
    {
        // Read once per ship; seeded into every tube's launch snapshot. No Rust
        // rule consults it — the gate is the tube's authored predicate (#872).
        let red_alert = red_alert_opt.is_some_and(|r| r.0);
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(ship_entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );
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
                s.0.hp_facing_attacker(physics.x, physics.z, tx, tz, target_yaw)
            })
            .unwrap_or(0);

        let dx = tx - physics.x;
        let dz = tz - physics.z;
        let world_bearing = simmath::atan2(dx, -dz);
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
        // Reported, not a gate: `torpedoes_remaining` is the rounds left to
        // RELOAD with — the ones in the tubes were drawn from it when their load
        // started — so it must never veto a launch. See `auto_fire_torpedo`.
        let magazine = torpedo_sys.torpedoes_remaining;
        // Ship-wide "every tube is at its volley capacity" (issue #791). A
        // doctrine that spends a shield gap on one full salvo needs this rather
        // than the per-tube `loaded` (which is `loaded_count > 0`), and it is a
        // reading of the ship's own tubes rather than an authored number — the
        // capacity itself is `[[torpedoes.tubes]] volley_max`. A hull with no
        // tubes at all cannot reach here (`tubes` would be empty and
        // `auto_fire_torpedo` would return nothing), so the vacuous-true case of
        // `all` is unreachable rather than merely unlikely.
        let tubes_full = torpedo_sys
            .tubes
            .iter()
            .all(|tube| tube.loaded_count >= tube.volley_max);

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
            // Per-tube LAUNCH policy gate (issue #782): `auto_fire_torpedo`
            // already resolved this tube's host readiness (loaded, in arc, target
            // locked — NOT the magazine, whose rounds are the ones left to reload
            // with and were already spent for whatever is in the tubes, and NOT
            // the striking arc's shields, which issue #956 moved into the guard
            // below); now resolve
            // the tube's own authored launch policy over a seeded snapshot. Only a
            // tube whose policy fires `LaunchTorpedo` launches — an idle tube (or
            // one whose guard holds) is skipped, leaving other tubes free to fire
            // (per-tube independence). The candidates all passed the host gates,
            // so those facts are `true`; `target_facing_shields` carries its live
            // per-arc HP reading, and is now the ONLY gate on the shield state.
            //
            // A tube with NO entry does not launch: since #885b stage 5d there
            // is no synthesised stand-in, and strict AI-declaration mode rejects
            // a tube that authors no inline `ai` block at load.
            let Some(launch_policy) = tube_policies.and_then(|p| p.0.get(&tube_id)) else {
                continue;
            };
            let facts = crate::weapons_plugin::seed_torpedo_tube_launch_facts(
                true,
                true,
                true,
                true,
                target_facing_shields,
                tubes_full,
                red_alert,
            );
            if !crate::weapons_plugin::torpedo_tube_launch_policy_fires(
                launch_policy,
                &facts,
                &flag_chain,
            ) {
                continue;
            }
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
    // Read-only scenario flag/counter chain (issue #891 stage 2).
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entity_spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &crate::weapons_plugin::TorpedoSystemResource,
            Option<&crate::weapons_plugin::TorpedoTubeAiPolicies>,
            &mut crate::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let magazine_id = crate::system_registry::torpedo_magazine_system_id();

    for (
        ship_entity,
        entity_uuid,
        control_sources,
        ship_config,
        torpedo_sys,
        tube_policies,
        mut admitted,
    ) in ships.iter_mut()
    {
        if !control_sources.0.policy_for(&magazine_id).operate_ai {
            continue;
        }

        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(ship_entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );

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
            // Per-tube LOAD policy gate (issue #782): `torpedo_load_orders`
            // decided this tube is AI-operated and off its configured volley
            // target; now resolve the tube's own authored load policy over a
            // seeded fact snapshot. Only a tube whose policy fires `LoadTorpedo`
            // emits the volley order — an idle tube (or one whose guard holds)
            // is skipped, leaving other tubes free to load (per-tube
            // independence).
            //
            // A tube with NO entry does not load: since #885b stage 5d there is
            // no synthesised stand-in, and strict AI-declaration mode rejects a
            // tube that authors no inline `ai` block at load.
            let Some(policy) = tube_policies.and_then(|p| p.0.get(&tube_id)) else {
                continue;
            };
            let tube_ref = torpedo_sys.0.tube(&tube_id);
            let facts = crate::weapons_plugin::seed_torpedo_tube_load_facts(
                tube_ref.map(|t| t.loaded_count).unwrap_or(0),
                tube_ref.map(|t| t.target_count).unwrap_or(0),
                count,
                torpedo_sys.0.torpedoes_remaining,
                true,
            );
            if !crate::weapons_plugin::torpedo_tube_load_policy_fires(policy, &facts, &flag_chain) {
                continue;
            }
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

/// High-fidelity Sensors frequency-hint emitter (issues #692, #873).
///
/// Wires `console_ai::tick_frequency_hint`: waits
/// `SensorsAiConfigResource::frequency_hint_delay_secs` after a target lock
/// before emitting a `FrequencyHint` coordination message to Tactical,
/// replicating a Sensors operator's reaction delay rather than the instantaneous
/// readout `ship::sensors::tick_sensors_frequency_hint` produces.
///
/// # Gating — `AiHighFidelity` and nothing else
///
/// The only gate is the `AiHighFidelity` query filter, i.e. the hull's
/// simulation level of detail. `tick_sensors_frequency_hint` skips exactly the
/// ships this one serves, so the two never double-emit and every ship emits
/// through one of them.
///
/// Issue #873 removed two gates that were both, underneath, the same
/// human-vs-AI branch:
///
/// * `policy_for(sensors).operate_ai` — a human on Sensors silenced this
///   emitter and diverted the fact to the immediate path instead, so the hint's
///   timing (and, across a lag boundary, its content) depended on who was
///   holding the console.
/// * The `auto_hint` `ai_tuning` rule, consulted only when a *session held* the
///   Sensors station. No shipped hull authors `auto_hint`, so in practice a
///   human sitting down at Sensors on a high-fidelity ship silenced the ship's
///   frequency advisory outright — a coordination fact whose existence turned
///   on the presence of a human. A station rating may tune what a console shows
///   its own operator; it may not decide whether the ship's authoritative state
///   reaches the rest of the bridge.
///
/// `sender_origin` is resolved at the write below and used for nothing but the
/// delivery-time routing tag (AGENTS.md rule 6).
/// `pub(crate)` so the end-to-end AC5 fixture in `ship::coordination_systems`
/// can register the emitter the PLAYER ship actually runs. The player hull is
/// permanently high-fidelity (`server_app::spawn_game_start_entities` gives
/// `LocalShip` the marker at spawn and `ai::server::lod_ai_ships` never
/// evaluates `LocalShip`), so this — not `tick_sensors_frequency_hint` — is the
/// emitter behind a human Sensors officer's advisory on the player ship.
pub(crate) fn tick_frequency_hint_high_fidelity(
    // The AUTHORED tick period, not `Time::delta` (issue #889). This system is
    // gated by `run_if(ai_tick_ready)`, so it observes one shared AI tick per
    // run — feeding it the frame delta would accumulate only the frames it
    // happened to run on and stretch the authored hint delay by the
    // frame-rate-to-tick-rate ratio (2x at 60 Hz, ~4.8x at 144 Hz), reversing
    // the frame-independence the gate exists to provide. Same shape as
    // `ai_policy_state_tick`'s `AiPolicyTickClock`.
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<
        (
            Entity,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &mut ShipFrequencyHintState,
            Option<&crate::ship::sensors::SensorsAiConfigResource>,
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
    let hz = world_config
        .as_deref()
        .map(|wc| wc.global.ai_tick_hz)
        .unwrap_or_else(|| crate::entity_config::GlobalConfig::default().ai_tick_hz);
    let dt = if hz > 0.0 { 1.0 / hz } else { 0.0 };
    let sensors_sid = crate::system_registry::sensors_system_id();

    for (entity, control_sources, blackboards, mut hint_state, ai_config_comp) in ships.iter_mut() {
        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3),
        // identical to how the low-fidelity twin `tick_sensors_frequency_hint`
        // and the firing paths read it — never the tactical radar's live
        // selection.
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
                sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
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
        PendingShieldsThreatBearing, ShieldsAiConfigResource, ShieldsDamageHistory,
        ShieldsFocusAiPolicy, ShipShields,
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
            default_focus_policy(),
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

    /// The canonical default Shields focus policy (issue #783) — reproduces
    /// today's decisions, kernel and all. Bare-`App` fixtures may omit the
    /// component (the host falls back to this same policy), but attaching it
    /// explicitly documents the wiring and lets a test swap in an authored one.
    fn default_focus_policy() -> ShieldsFocusAiPolicy {
        ShieldsFocusAiPolicy(
            crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus")
                .to_policy()
                .unwrap(),
        )
    }

    /// A Shields focus policy whose health-imbalance fallback threshold is `pct`
    /// (0–100), every other authored number left at the default. Proves
    /// per-entity policy `param`s drive the decision (issue #783).
    fn focus_policy_with_health_ratio(pct: f32) -> ShieldsFocusAiPolicy {
        let mut cfg = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus");
        cfg.param.insert(
            crate::entities::config::SHIELD_FOCUS_HEALTH_RATIO_PARAM.to_string(),
            pct,
        );
        ShieldsFocusAiPolicy(cfg.to_policy().unwrap())
    }

    /// A Shields focus policy that declares an explicit idle — the host must
    /// take no AI focus action regardless of damage (issue #783 gate).
    fn idle_focus_policy() -> ShieldsFocusAiPolicy {
        ShieldsFocusAiPolicy(crate::ai::policy::AiPolicy {
            idle: true,
            ..Default::default()
        })
    }

    /// A Shields focus policy with a SINGLE rule guarded on the seeded
    /// `recent_damage_total` fact and NO unconditional fallback — so the retained
    /// kernel runs only when the bounded recent-damage fact clears the gate.
    /// Proves the `fact(...)` guard actually fires (facts are seeded, closing the
    /// #779 empty-facts sharp edge).
    fn damage_only_focus_policy() -> ShieldsFocusAiPolicy {
        let mut cfg = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus");
        cfg.param.insert("min_recent_damage".to_string(), 0.0);
        cfg.rule = vec![crate::entities::config::FineSystemAiRuleToml {
            priority: 10,
            channel: crate::entities::config::SHIELD_FOCUS_CHANNEL.to_string(),
            when: "fact(recent_damage_total) > param(min_recent_damage)".to_string(),
            verb: crate::entities::config::SHIELD_FOCUS_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }];
        ShieldsFocusAiPolicy(cfg.to_policy().unwrap())
    }

    /// A Shields focus policy whose ONLY rule is guarded on a world flag — the
    /// #891 stage 2 read surface — with no unconditional fallback.
    fn flag_only_focus_policy() -> ShieldsFocusAiPolicy {
        let mut cfg = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus");
        cfg.rule = vec![crate::entities::config::FineSystemAiRuleToml {
            priority: 10,
            channel: crate::entities::config::SHIELD_FOCUS_CHANNEL.to_string(),
            when: "flag(brace_for_impact)".to_string(),
            verb: crate::entities::config::SHIELD_FOCUS_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }];
        ShieldsFocusAiPolicy(cfg.to_policy().unwrap())
    }

    /// Issue #891 stage 2, per-host both-directions proof for the Shields
    /// focus host: with heavy damage on facing 0 (the kernel's pick), a
    /// `flag()`-gated policy holds while the scenario flag is clear and
    /// focuses once it is set.
    #[test]
    fn ai_shield_focus_flag_guard_reads_the_world_in_both_directions() {
        let mut app = shield_test_app();
        app.init_resource::<crate::world::server::WorldContentRuntime>();
        let e = ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .insert(flag_only_focus_policy());
        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 20; // heavy damage to facing 0 only
        }

        // Flag CLEAR -> the gate reads false, the kernel never runs, no focus.
        app.update();
        assert_eq!(
            focused_facing(&app, e),
            None,
            "with the world flag clear the focus gate must read false and hold"
        );

        // Flag SET -> the SAME gate fires and the kernel focuses the weak facing.
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .flags
            .set_flag("brace_for_impact");
        app.update();
        assert_eq!(
            focused_facing(&app, e),
            Some(0),
            "with the world flag set the same gate must fire and focus facing 0"
        );
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
                default_focus_policy(),
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
    fn shield_ai_reads_its_own_policy_params_per_entity() {
        // Issue #783 isolation: the authored windows/thresholds now live in each
        // ship's own `ShieldsFocusAiPolicy` `param` map, read per-entity in the
        // host. A ship carrying a permissive 90% health-ratio param must focus a
        // 60/100 arc (0.6 < 0.9 · 1.0), while a ship on the default 50% policy
        // must not (0.6 < 0.5 · 1.0 is false) — proving one ship's authored
        // tuning never bleeds onto another (the #738 isolation guarantee, now
        // carried by the per-entity policy rather than a global Resource).
        let mut app = shield_test_app();
        // The base fixture ship carries the DEFAULT policy (50%).
        let defaulted = ship_entity(&mut app);

        let config = crate::shield::ShieldConfig {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        // A second ship carrying the permissive tuning as its own policy param.
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
                focus_policy_with_health_ratio(90.0),
            ))
            .id();

        for e in [defaulted, tuned] {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 60;
        }
        app.update();

        assert_eq!(
            focused_facing(&app, tuned),
            Some(0),
            "a ship carrying the permissive health-ratio param must focus the weak arc"
        );
        assert_eq!(
            focused_facing(&app, defaulted),
            None,
            "a ship on the default policy must not focus a 60/100 arc — one ship's \
             authored params must never bleed onto another"
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

    #[test]
    fn authored_focus_policy_drives_focus_via_its_params() {
        // An authored (non-default) policy carrying a permissive 90% health-ratio
        // param focuses a 60/100 arc the default 50% policy would leave alone —
        // observable proof the authored windows/thresholds route through the
        // policy `param` map into the retained kernel (issue #783 AC2/AC4).
        let mut app = shield_test_app();
        let e = ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .insert(focus_policy_with_health_ratio(90.0));

        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 60; // 0.6 < 0.9·1.0 → focus under 90%, not 50%
        }
        app.update();

        assert_eq!(
            focused_facing(&app, e),
            Some(0),
            "an authored permissive policy must focus the weak arc its params allow"
        );
    }

    #[test]
    fn idle_focus_policy_takes_no_ai_focus_even_under_damage() {
        // The gate: an idle policy resolves the `shield_focus` channel to None,
        // so the host emits nothing even when an arc is heavily damaged and the
        // kernel would otherwise focus it (issue #783 AC4 idle opt-out).
        let mut app = shield_test_app();
        let e = ship_entity(&mut app);
        app.world_mut().entity_mut(e).insert(idle_focus_policy());

        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[0].hp = 20; // heavy damage the default would focus
        }
        app.update();

        assert_eq!(
            focused_facing(&app, e),
            None,
            "an idle Shields focus policy must suppress all AI focus changes"
        );
    }

    #[test]
    fn fact_guarded_focus_rule_fires_only_when_recent_damage_is_seeded() {
        // #779 empty-facts guard: a policy whose ONLY rule is guarded on the
        // seeded `recent_damage_total` fact (no unconditional fallback) must NOT
        // act on a quiet ship, but MUST act once a real hit seeds the fact —
        // proving `seed_shields_focus_facts` populates the window so a `fact(...)`
        // guard can fire at all.
        let mut app = shield_test_app();
        app.add_systems(Update, clear_admitted_each_tick.before(ai_shield_focus));
        let e = ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .insert(damage_only_focus_policy());

        // Tick 1: baseline observation, no damage recorded yet — the fact-guarded
        // rule finds `recent_damage_total = 0` and does not fire.
        app.update();
        assert_eq!(
            focused_facing(&app, e),
            None,
            "with no recent damage the fact-guarded rule must not fire"
        );

        // Tick 2: a real hit on facing 1 seeds `recent_damage_total > 0`; the
        // guard fires, the kernel runs, and the hit arc is focused.
        {
            let mut entity_mut = app.world_mut().entity_mut(e);
            let mut shields = entity_mut.get_mut::<ShipShields>().unwrap();
            shields.0.facings[1].hp = 60;
        }
        app.update();
        assert_eq!(
            focused_facing(&app, e),
            Some(1),
            "a seeded recent-damage fact must let the guarded rule fire and focus the hit arc"
        );
    }

    // ── tick_frequency_hint_high_fidelity ─────────────────────────────────

    /// Test-only glue (issue #829): seed each ship's viewscreen combat_lock from
    /// its `TacticalRadarSelection` before the hint emitter reads the frozen
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
                (
                    seed_viewscreen_from_selection,
                    tick_frequency_hint_high_fidelity,
                )
                    .chain(),
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

    /// Advance the hint by `secs` of AI-TICK time.
    ///
    /// Issue #889: the hint emitter runs under `run_if(ai_tick_ready)` and
    /// advances its delay by one authored tick period per run, not by
    /// `Time::delta` — otherwise the gate would stretch the authored delay by
    /// the frame-rate-to-tick-rate ratio. So the fixture now drives AI TICKS
    /// rather than one oversized wall-clock jump: `secs` of hint time is
    /// `secs * ai_tick_hz` updates. Wall-clock is advanced alongside purely so
    /// any other `Time` reader in the harness sees a consistent world.
    fn tick_with_dt(app: &mut App, secs: f32) {
        let hz = crate::entity_config::GlobalConfig::default().ai_tick_hz;
        let period = 1.0 / hz;
        let ticks = (secs * hz).ceil().max(1.0) as usize;
        for _ in 0..ticks {
            let mut time = app.world_mut().resource_mut::<Time>();
            time.advance_by(std::time::Duration::from_secs_f32(period));
            app.update();
        }
    }

    #[test]
    fn frequency_hint_propagates_after_the_authored_reaction_delay() {
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
        // Issue #738 isolation, mirroring the shields case: the hint emitter
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

    /// Issue #873, replacing `ai_frequency_hint_skips_ships_where_sensors_are
    /// _not_ai_operated`.
    ///
    /// That test pinned the branch this issue exists to delete: the emitter
    /// stood down whenever a human held Sensors, so the ship's frequency
    /// advisory came from the AI operator path rather than from authoritative
    /// state. Its premise is gone, so it is re-pointed at the rule that
    /// replaced it — the same fact, from the same state, for a human-held
    /// console — with a strictly stronger assertion: not merely that something
    /// is emitted, but that it carries the human origin as a routing TAG.
    ///
    /// `sender_origin == Human` is the whole point. It proves the emitter read
    /// the control source (so the tag is live, not a hardcoded `Ai` the way
    /// `tick_power_brownout_advisory` used to stamp one) while proving the
    /// value did not gate the emission.
    #[test]
    fn frequency_hint_fires_from_a_human_held_sensors_station_and_tags_it_human() {
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

        tick_with_dt(&mut app, 4.0);

        let coord = &app.world().resource::<CoordBox>().0;
        let hint = coord
            .iter()
            .find(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. }))
            .expect(
                "a human-held Sensors console must still feed the ship's coordination bus \
                 — the fact comes from authoritative state, not from who is sitting there",
            );
        assert_eq!(
            hint.sender_origin,
            ControlSource::Human,
            "sender_origin must report the live control source, and be used only as a \
             delivery-routing tag"
        );
        match &hint.payload {
            CoordinationPayload::FrequencyHint { frequency } => assert!(
                (*frequency - 0.75).abs() < f32::EPSILON,
                "the human-sent hint must carry the same authoritative shield frequency \
                 the AI-sent one does"
            ),
            other => panic!("expected FrequencyHint, got {other:?}"),
        }
    }

    // ── The retired `auto_hint` rating gate (issue #873) ────────────────────
    //
    // These two tests used to pin a claimed/unclaimed split copied from
    // `ai_torpedo_auto_fire`: once a human session held Sensors, the hint
    // additionally required that holder's active rating to declare `auto_hint`
    // in its `ai_tuning` table, and stayed silent otherwise.
    //
    // That is a coordination fact whose emission turned on the presence of a
    // human, which AGENTS.md rule 6 forbids and issue #873 removes. Both are
    // kept — the fixture is exactly the interesting one — and re-pointed at the
    // surviving rule: the rating table is now irrelevant to emission in BOTH
    // directions, which takes two tests to state and could not be stated by
    // deleting either.

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
    fn frequency_hint_fires_when_a_claimed_station_rating_declares_auto_hint() {
        let mut app = freq_hint_test_app();
        claim_sensors_station(&mut app, "op1", "Assisted");

        tick_with_dt(&mut app, 4.0);

        let coord = &app.world().resource::<CoordBox>().0;
        assert!(
            coord
                .iter()
                .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
            "a claimed Sensors station whose active rating declares auto_hint \
             must be hinted"
        );
    }

    /// The half that changed. `"Std"` is a rating with no `ai_tuning` table at
    /// all, held by a live session — the configuration that used to silence the
    /// ship's frequency advisory completely.
    #[test]
    fn frequency_hint_fires_when_a_claimed_station_rating_lacks_auto_hint() {
        let mut app = freq_hint_test_app();
        claim_sensors_station(&mut app, "op1", "Std");

        tick_with_dt(&mut app, 4.0);

        let coord = &app.world().resource::<CoordBox>().0;
        assert!(
            coord
                .iter()
                .any(|m| matches!(&m.payload, CoordinationPayload::FrequencyHint { .. })),
            "a station rating's ai_tuning table must not decide whether a coordination \
             fact derived from authoritative state is emitted at all (issue #873): a human \
             on a rating without auto_hint still feeds the ship's backfilled Tactical"
        );
    }

    #[test]
    fn frequency_hint_fires_unconditionally_when_sensors_station_is_unclaimed() {
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

    // ── ai_power_allocation (inline stateless policy spine, issue #784) ──────

    use crate::entities::config::{
        FineSystemAiConfigToml, FineSystemAiRuleToml, POWER_SET_ALLOCATION_VERB,
    };
    use crate::ship::power::PowerAiPolicy;

    /// Build a `PowerAiPolicy` through the real `to_policy` decode path so the
    /// tests exercise the value-carrying `set_power_group_allocation` verb + the
    /// `level` payload just as authored TOML would.
    fn power_policy(params: &[(&str, f32)], rules: Vec<FineSystemAiRuleToml>) -> PowerAiPolicy {
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
            idle: false,
            param: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            rule: rules,
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        PowerAiPolicy(cfg.to_policy().expect("power policy decodes"))
    }

    fn alloc_rule(priority: i32, channel: &str, when: &str, level: u8) -> FineSystemAiRuleToml {
        FineSystemAiRuleToml {
            priority,
            channel: channel.to_string(),
            when: when.to_string(),
            verb: POWER_SET_ALLOCATION_VERB.to_string(),
            value: false,
            level,
            response_index: 0,
        }
    }

    fn default_power_policy() -> PowerAiPolicy {
        PowerAiPolicy(
            crate::entities::authored_ai_pins::shipped_policy_toml("power")
                .to_policy()
                .unwrap(),
        )
    }

    /// Wires the real production pair: the AI decide system
    /// (`ai_power_allocation`, emit) `.before` the single applier
    /// (`ship::power::handle_power_messages`, issue #831). Attaches the canonical
    /// default `PowerAiPolicy` (baseline: helm←thrust / weapons←red alert with
    /// reserve guards) unless the caller overrides it.
    fn power_test_app() -> App {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
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
            default_power_policy(),
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

    fn set_battery(app: &mut App, e: Entity, charge: f32) {
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship::power::ShipPowerSystem>()
            .unwrap()
            .0
            .battery_charge = charge;
    }

    fn power_tick_with_dt(app: &mut App, dt_secs: f32) {
        let mut time = app.world_mut().resource_mut::<Time>();
        time.advance_by(std::time::Duration::from_secs_f32(dt_secs));
        app.update();
    }

    /// Variant of [`power_test_app`] built from a SHIPPED hull file: its own
    /// `[power]` reactor (capacity, rates, emergency threshold), its own
    /// `[power_groups.*]` seeding, and its own `[power.ai_policy]`, with
    /// `ship::power::tick_power_system` chained after the applier so the reactor
    /// integrates the battery and exhaustion lock every tick.
    ///
    /// Nothing is hand-written: everything the ladder depends on comes off the
    /// file the fleet actually flies.
    fn shipped_hull_power_app(path: &str) -> (App, Entity) {
        let config = crate::entity_includes::load_entity_config(path)
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let reactor = config.power.as_ref().expect("hull authors [power]");
        let power_groups = config
            .ship_config
            .as_ref()
            .map(|s| s.power_groups.clone())
            .unwrap_or_default();
        let power_config =
            crate::ship::power::PowerConfigResource(crate::modifiers::power_system::PowerConfig {
                capacity: reactor.capacity,
                rates: reactor.rates,
                sustainable_total: reactor.sustainable_total,
                max_commanded_total: reactor.max_commanded_total,
                emergency_threshold: reactor.emergency_threshold,
            });
        let seed = crate::ship::power::authored_power_group_seed(&power_groups);
        let policy = PowerAiPolicy(
            reactor
                .ai_policy
                .as_ref()
                .expect("hull authors [power.ai_policy]")
                .to_policy()
                .expect("shipped policy decodes"),
        );

        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
            .insert_resource(crate::lobby::Sessions(
                crate::lobby::session::SessionManager::new(),
            ))
            .add_systems(
                Update,
                (
                    ai_power_allocation,
                    crate::ship::power::handle_power_messages,
                    crate::ship::power::tick_power_system,
                )
                    .chain(),
            );

        let mut control_sources = ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
        let e = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                control_sources,
                crate::ship::power::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::from_authored_groups(
                        &power_config.0,
                        &seed,
                    ),
                ),
                power_config,
                crate::ship_state::ShipRedAlert(true),
                crate::ship_plugin::LastHelmInput {
                    thrust: 0.9,
                    ..Default::default()
                },
                policy,
                AdmittedCommands::default(),
                AiHighFidelity,
            ))
            .id();
        (app, e)
    }

    #[test]
    fn baseline_default_reallocates_toward_weapons_on_red_alert() {
        // Baseline preservation: the synthesised default policy reproduces the
        // retired red-alert→weapons behaviour. Under red alert with a full
        // battery (well above the 10% weapons reserve) weapons rises to its
        // elevated level 3.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        power_tick_with_dt(&mut app, 0.1);

        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            3,
            "sustained red alert must elevate weapons power (default baseline)"
        );
    }

    #[test]
    fn baseline_default_reallocates_toward_helm_on_sustained_thrust() {
        // Baseline preservation: sustained high thrust + healthy battery, AT RED
        // ALERT, raises helm power to its elevated level 3 (reproducing the
        // retired movement→helm behaviour, now absolute + stateless). The
        // red-alert guard was added to fix ships browning out on ordinary
        // (non-combat) transit — `plan_helm_travel` commands near-max thrust for
        // any far-off waypoint, so without the guard this elevation held for the
        // whole cruise, not just combat.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
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

        power_tick_with_dt(&mut app, 0.1);

        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
            3,
            "sustained high thrust at red alert must elevate helm power (default baseline)"
        );
    }

    #[test]
    fn sustained_thrust_without_red_alert_does_not_elevate_helm() {
        // Regression guard for the brownout-outside-combat fix: ordinary cruise
        // thrust (no red alert) must hold helm at the baseline 2, not the
        // combat-burst 3 — the whole point of the `red_alert` guard on the
        // elevate rule.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_plugin::LastHelmInput>()
            .unwrap()
            .thrust = 0.9;

        power_tick_with_dt(&mut app, 0.1);

        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
            2,
            "sustained thrust away from red alert must NOT elevate helm \
             (that used to brown out ships on ordinary transit)"
        );
    }

    #[test]
    fn reserve_gate_blocks_elevation_below_the_authored_floor() {
        // AC2 + AC5: with the battery drained below the helm 50% reserve, the
        // elevate guard cannot fire even under full thrust at red alert, so the
        // baseline fallback holds helm at level 2 — allocation never rises when
        // the battery can't sustain it (no avoidable brownout). Above the
        // reserve it elevates.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
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

        // 40% battery is below the 50% helm reserve → held at baseline 2.
        set_battery(&mut app, e, 40.0);
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
            2,
            "below-reserve thrust must NOT elevate helm (brownout avoidance)"
        );

        // Recharge above the reserve → the same thrust now elevates.
        set_battery(&mut app, e, 80.0);
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
            3,
            "above-reserve thrust must elevate helm"
        );
    }

    #[test]
    fn reserve_gate_lowers_allocation_when_battery_dips_under_load() {
        // AC5: a group already elevated is brought back down by the lowering
        // baseline rule once the battery falls below the reserve while still
        // under thrust — the per-rule reserve guard is the brownout-avoidance
        // mechanism, with no global emergency exception.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
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

        set_battery(&mut app, e, 80.0);
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
            3
        );

        set_battery(&mut app, e, 40.0);
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
            2,
            "an elevated group must drop back to baseline once battery dips below reserve"
        );
    }

    // ── Budget-aware allocation (issue #959) ─────────────────────────────────

    /// A four-group Alliance-shaped reactor (`ops` outside the canonical trio)
    /// crewed by `policy`, with the production decide→apply pair and a per-tick
    /// `AdmittedCommands` clear so each arm's emits can be counted on their own.
    ///
    /// Commanded at the 8-point cap on arrival — helm 3 / weapons 2 / shields 2
    /// / ops 1 — because the interesting failure is what a policy does when
    /// there is NOTHING left to spend.
    fn over_budget_power_app(policy: PowerAiPolicy) -> (App, Entity) {
        use crate::modifiers::power_system::{
            HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
        };
        let seed = [
            (crate::messages::PowerGroupId(HELM_POWER_GROUP.into()), 3u8),
            (crate::messages::PowerGroupId(WEAPONS_POWER_GROUP.into()), 2),
            (crate::messages::PowerGroupId(SHIELDS_POWER_GROUP.into()), 2),
            (crate::messages::PowerGroupId("ops".into()), 1),
        ];

        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
            .insert_resource(crate::lobby::Sessions(
                crate::lobby::session::SessionManager::new(),
            ))
            .add_systems(
                Update,
                (
                    clear_admitted_each_tick,
                    ai_power_allocation,
                    crate::ship::power::handle_power_messages,
                )
                    .chain(),
            );

        let mut control_sources = ShipSystemControlSources::default();
        control_sources.0.set(
            crate::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
        let e = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                control_sources,
                crate::ship::power::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::from_authored_groups(
                        &crate::modifiers::power_system::PowerConfig::default(),
                        &seed,
                    ),
                ),
                crate::ship_state::ShipRedAlert(true),
                crate::ship_plugin::LastHelmInput::default(),
                policy,
                AdmittedCommands::default(),
                AiHighFidelity,
            ))
            .id();
        (app, e)
    }

    fn commanded(app: &App, e: Entity, group: &str) -> u8 {
        app.world()
            .entity(e)
            .get::<crate::ship::power::ShipPowerSystem>()
            .unwrap()
            .0
            .commanded_level_for(&crate::messages::PowerGroupId(group.into()))
    }

    fn commanded_total(app: &App, e: Entity) -> u8 {
        app.world()
            .entity(e)
            .get::<crate::ship::power::ShipPowerSystem>()
            .unwrap()
            .0
            .commanded_total()
    }

    /// Every `SetPowerGroupAllocation` this ship emitted on the arm that just
    /// ran, in emission order.
    fn emitted_allocations(app: &App, e: Entity) -> Vec<(String, u8)> {
        app.world()
            .entity(e)
            .get::<AdmittedCommands>()
            .unwrap()
            .for_target(crate::system_registry::POWER_REACTOR_SYSTEM_ID)
            .filter_map(|c| match &c.payload {
                crate::messages::SystemControlPayload::SetPowerGroupAllocation { group, level } => {
                    Some((group.0.clone(), *level))
                }
                _ => None,
            })
            .collect()
    }

    /// **The silent cap-refusal-and-reemit loop is gone (issue #959).**
    ///
    /// Three rules ask for level 4 on a reactor that has 8 points and four
    /// groups, so the policy is asking for roughly half again what the ship
    /// owns. Before this issue each channel was emitted in isolation:
    /// `PowerSystem::increase` refused the surplus SILENTLY, the refused groups
    /// never reached the level they had been commanded to, and the decider —
    /// which only skips an emit when the commanded level already MATCHES —
    /// re-issued the identical admitted command on every arm for the rest of
    /// the encounter.
    ///
    /// Now the arm plans against the budget: the ship lands on a legal
    /// allocation in ONE arm, every emitted command is actually carried out,
    /// and the arms that follow emit nothing at all.
    #[test]
    fn an_over_budget_power_policy_settles_in_one_arm_and_stops_re_emitting() {
        use crate::modifiers::power_system::{
            HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
        };
        let policy = power_policy(
            &[],
            vec![
                alloc_rule(20, WEAPONS_POWER_GROUP, "true", 4),
                alloc_rule(10, HELM_POWER_GROUP, "true", 4),
                alloc_rule(5, SHIELDS_POWER_GROUP, "true", 4),
            ],
        );
        let (mut app, e) = over_budget_power_app(policy);

        power_tick_with_dt(&mut app, 0.1);

        // Highest authored priority is paid in full; the rest take what is left
        // and land on their minimum, with `ops` (nothing bid for it) reserved.
        assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 4);
        assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 2);
        assert_eq!(commanded(&app, e, SHIELDS_POWER_GROUP), 1);
        assert_eq!(commanded(&app, e, "ops"), 1);

        let total = app
            .world()
            .entity(e)
            .get::<crate::ship::power::ShipPowerSystem>()
            .unwrap()
            .0
            .commanded_total();
        assert_eq!(total, 8, "the budget is spent, and not overspent");

        // The emitted ORDER is load-bearing, not incidental: the applier tests
        // the budget one command at a time, so weapons 2 → 4 is only affordable
        // after helm and shields have given their points back. Emitting the
        // increase first would have it refused — silently — with the ship
        // already at the cap.
        assert_eq!(
            emitted_allocations(&app, e),
            vec![
                (HELM_POWER_GROUP.to_string(), 2),
                (SHIELDS_POWER_GROUP.to_string(), 1),
                (WEAPONS_POWER_GROUP.to_string(), 4),
            ],
            "decreases must be emitted before the increases they pay for"
        );

        // Every command the arm emitted was actually carried out — the silent
        // refusal has nothing left to swallow.
        for (group, level) in emitted_allocations(&app, e) {
            assert_eq!(
                commanded(&app, e, &group),
                level,
                "{group} was commanded to {level} and the reactor refused it"
            );
        }

        // …and the decision has settled: nothing is re-emitted, for ever.
        for arm in 0..6 {
            power_tick_with_dt(&mut app, 0.1);
            assert!(
                emitted_allocations(&app, e).is_empty(),
                "arm {arm} re-emitted after the allocation had settled: {:?}",
                emitted_allocations(&app, e)
            );
            assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 4);
            assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 2);
        }
    }

    /// **Which group wins a budget collision is the HULL's decision.** The same
    /// three over-budget bids with the authored priorities swapped hand the
    /// spare points to helm instead of weapons. Both of `plan_allocation`'s
    /// ordering keys are authored, so there is no branch to override the
    /// authored config with — the Rust seed order breaks only a tie the hull
    /// left identical on both.
    #[test]
    fn the_authored_rule_priority_decides_who_gets_the_last_reactor_point() {
        use crate::modifiers::power_system::{
            HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
        };
        let policy = power_policy(
            &[],
            vec![
                alloc_rule(5, WEAPONS_POWER_GROUP, "true", 4),
                alloc_rule(20, HELM_POWER_GROUP, "true", 4),
                alloc_rule(10, SHIELDS_POWER_GROUP, "true", 4),
            ],
        );
        let (mut app, e) = over_budget_power_app(policy);
        power_tick_with_dt(&mut app, 0.1);

        assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 4);
        assert_eq!(commanded(&app, e, SHIELDS_POWER_GROUP), 2);
        assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 1);
    }

    /// The shipped fleet's combat allocation settles with its three intended
    /// groups: helm 3 / weapons 3 / shields 2.
    ///
    /// All four groups are pinned individually, not just jointly by the total —
    /// Shields receive no combat bid, so their authored resting level is
    /// reserved rather than cut to buy another group an extra point.
    ///
    /// The settle is the second half of the name, and it is assertable here
    /// without a per-tick `AdmittedCommands` clear: `handle_power_messages` does
    /// not drain the queue, so a second arm that decides to emit nothing leaves
    /// the emitted list byte-identical.
    ///
    /// The emitted ORDER is pinned too: helm and weapons both win at
    /// `priority = 10`, so with the battery floors reverted there is no
    /// secondary authored key, and the stable sort falls back to the reactor's
    /// own seed order (`POWER_GROUP_ORDER`: helm before weapons). Both are paid
    /// in full either way.
    #[test]
    fn the_shipped_combat_stations_allocation_is_unchanged_and_settles() {
        use crate::modifiers::power_system::{
            HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
        };
        let (mut app, e) = shipped_hull_power_app("assets/entities/alliance_destroyer.toml");
        power_tick_with_dt(&mut app, 0.1);

        assert_eq!(commanded(&app, e, HELM_POWER_GROUP), 3);
        assert_eq!(commanded(&app, e, WEAPONS_POWER_GROUP), 3);
        assert_eq!(
            commanded(&app, e, SHIELDS_POWER_GROUP),
            2,
            "reserved, not cut"
        );
        assert_eq!(commanded_total(&app, e), 8);

        let first_arm = emitted_allocations(&app, e);
        assert_eq!(
            first_arm,
            vec![
                (HELM_POWER_GROUP.to_string(), 3),
                (WEAPONS_POWER_GROUP.to_string(), 3),
            ],
            "equal-priority bidders fall back to the reactor's seed order"
        );

        // …and it has settled: the second arm adds nothing to the queue.
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            emitted_allocations(&app, e),
            first_arm,
            "the shipped allocation re-emitted after it had settled"
        );
    }

    /// A four-group hull config whose `weapons` group is capped at
    /// `max_level = 2`, parsed through the real `ShipConfig` path so
    /// `[power_groups.<id>] max_level` is read exactly as an authored hull file
    /// supplies it — including the `#[serde(default)]` fallback the other three
    /// groups take by omitting the key.
    fn weapons_capped_ship_config() -> crate::ship::config::ShipConfig {
        let toml = r#"
[[system]]
id = "power-reactor"
kind = "power_reactor"
ai_only = true

[power_groups.ops]
label = "Operations"
default_level = 1

[power_groups.helm]
label = "Propulsion"
default_level = 3

[power_groups.weapons]
label = "Weapons"
default_level = 2
max_level = 2

[power_groups.shields]
label = "Shields"
default_level = 2
"#;
        crate::ship::config::ShipConfig::from_toml(
            toml,
            &[crate::system_registry::POWER_REACTOR_KIND],
        )
        .unwrap()
    }

    /// **The hull's own `[power_groups.<id>] max_level` is what caps a grant.**
    ///
    /// The wiring test for `AllocationBid::max_level`. Every other power fixture
    /// in this file spawns a ship with no `ShipConfigComponent` at all, so the
    /// bid has always taken `ship::config::default_max_power_level`'s 4 through
    /// the `unwrap_or_else` fallback — a read that ignored the authored ceiling
    /// entirely would have passed the lot of them.
    ///
    /// `weapons` holds the top authored priority and asks for 4 on a hull that
    /// caps it at 2, so the two points it may not have fall through to `helm`
    /// queued behind it. Read the ceiling wrong and the same 8 points land the
    /// exact inverse — weapons 4, helm 2 — which is what makes the authored
    /// number observable here rather than merely present.
    #[test]
    fn the_authored_group_max_level_caps_a_grant_and_the_rest_falls_through() {
        use crate::modifiers::power_system::{
            HELM_POWER_GROUP, SHIELDS_POWER_GROUP, WEAPONS_POWER_GROUP,
        };
        let policy = power_policy(
            &[],
            vec![
                alloc_rule(20, WEAPONS_POWER_GROUP, "true", 4),
                alloc_rule(10, HELM_POWER_GROUP, "true", 4),
                alloc_rule(5, SHIELDS_POWER_GROUP, "true", 4),
            ],
        );
        let (mut app, e) = over_budget_power_app(policy);
        app.world_mut()
            .entity_mut(e)
            .insert(crate::ship_plugin::ShipConfigComponent(
                weapons_capped_ship_config(),
            ));

        power_tick_with_dt(&mut app, 0.1);

        assert_eq!(
            commanded(&app, e, WEAPONS_POWER_GROUP),
            2,
            "the top-priority bid asked for 4 and its own hull caps it at 2"
        );
        assert_eq!(
            commanded(&app, e, HELM_POWER_GROUP),
            4,
            "the points weapons was not allowed to take fall through to the next bid"
        );
        assert_eq!(commanded(&app, e, SHIELDS_POWER_GROUP), 1);
        assert_eq!(commanded(&app, e, "ops"), 1);
        assert_eq!(commanded_total(&app, e), 8);
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
        power_tick_with_dt(&mut app, 0.1);
        let after = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        assert_eq!(
            before, after,
            "ships without AiHighFidelity must not be touched by ai_power_allocation"
        );
    }

    #[test]
    fn human_control_source_holds_and_regains_cleanly() {
        // AC5 human Control Source + lifecycle reset: while a human holds the
        // Power reactor the AI stands down (allocation unchanged). Because the
        // decision is stateless there is nothing to reset — the very next tick
        // after AI control is regained yields a clean decision from the fresh
        // snapshot.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;
        app.world_mut()
            .entity_mut(e)
            .get_mut::<ShipSystemControlSources>()
            .unwrap()
            .0
            .set(
                crate::system_registry::power_reactor_system_id(),
                ControlSource::Human,
            );

        let before = power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            before,
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            "human-operated power reactor must not be touched by ai_power_allocation"
        );

        // Hand back to AI: a clean decision this tick, no stale carry-over.
        app.world_mut()
            .entity_mut(e)
            .get_mut::<ShipSystemControlSources>()
            .unwrap()
            .0
            .set(
                crate::system_registry::power_reactor_system_id(),
                ControlSource::Ai,
            );
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            3,
            "regaining AI control yields a clean elevate decision (stateless reset)"
        );
    }

    #[test]
    fn emits_admitted_set_power_group_allocation_and_skips_no_ops() {
        // AC: the decide system emits its reallocation as an admitted
        // `SetPowerGroupAllocation` targeting the reactor, and a saturated no-op
        // (target == current) is NOT re-admitted every tick.
        let mut emit_app = App::new();
        emit_app
            .insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
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
                default_power_policy(),
                crate::messages::AdmittedCommands::default(),
                AiHighFidelity,
            ))
            .id();
        {
            let mut time = emit_app.world_mut().resource_mut::<Time>();
            time.advance_by(std::time::Duration::from_secs_f32(0.1));
        }
        emit_app.update();

        let admitted = emit_app
            .world()
            .entity(ee)
            .get::<crate::messages::AdmittedCommands>()
            .unwrap();
        let weapons_alloc = admitted.0.iter().find_map(|c| match &c.payload {
            crate::messages::SystemControlPayload::SetPowerGroupAllocation { group, level }
                if c.target == crate::system_registry::power_reactor_system_id()
                    && group.0 == crate::modifiers::power_system::WEAPONS_POWER_GROUP =>
            {
                Some(*level)
            }
            _ => None,
        });
        assert_eq!(
            weapons_alloc,
            Some(3),
            "red alert must admit an absolute SetPowerGroupAllocation(3) for weapons"
        );

        // Saturate weapons at the emitted level and clear admissions: a further
        // tick produces the same target and must NOT re-admit a no-op.
        {
            let mut ent = emit_app.world_mut().entity_mut(ee);
            ent.get_mut::<crate::ship::power::ShipPowerSystem>()
                .unwrap()
                .0
                .set_group_allocation(
                    &crate::messages::PowerGroupId(
                        crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
                    ),
                    3,
                )
                .unwrap();
            ent.get_mut::<crate::messages::AdmittedCommands>()
                .unwrap()
                .0
                .clear();
        }
        {
            let mut time = emit_app.world_mut().resource_mut::<Time>();
            time.advance_by(std::time::Duration::from_secs_f32(0.1));
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
            "a group already at the target level must not re-admit a no-op"
        );
    }

    #[test]
    fn ai_power_reallocation_dual_writes_resource_for_local_ship() {
        // When the AI path reallocates power for the LocalShip, the admitted
        // command flows through `handle_power_messages` — the single applier —
        // which dual-writes the legacy global `ShipPowerSystem` Resource.
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

        power_tick_with_dt(&mut app, 0.1);

        let component_level =
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP);
        let resource_level = app
            .world()
            .resource::<crate::ship::power::ShipPowerSystem>()
            .0
            .level_for(&crate::messages::PowerGroupId(
                crate::modifiers::power_system::WEAPONS_POWER_GROUP.into(),
            ));
        assert_eq!(component_level, resource_level);
        assert_eq!(resource_level, 3);
    }

    #[test]
    fn two_ships_with_different_authored_group_layouts_allocate_independently() {
        // AC1 + AC4 + AC6 per-ship isolation: two AI ships carry DIFFERENT
        // authored group layouts. Ship A (helm/weapons/sensors/ops) elevates its
        // `ops` group on thrust; ship B (canonical three) elevates `sensors` on
        // thrust. Under identical thrust each nudges only its own authored group,
        // proving the channels are per-ship data, not a shared catalogue.
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
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

        let spawn = |app: &mut App, groups: &[(&str, u8)], policy: PowerAiPolicy| -> Entity {
            let mut cs = ShipSystemControlSources::default();
            cs.0.set(
                crate::system_registry::power_reactor_system_id(),
                ControlSource::Ai,
            );
            let seed: Vec<(crate::messages::PowerGroupId, u8)> = groups
                .iter()
                .map(|(g, l)| (crate::messages::PowerGroupId(g.to_string()), *l))
                .collect();
            app.world_mut()
                .spawn((
                    crate::server_app::Ship,
                    cs,
                    crate::ship::power::ShipPowerSystem(
                        crate::modifiers::power_system::PowerSystem::from_authored_groups(
                            &crate::modifiers::power_system::PowerConfig::default(),
                            &seed,
                        ),
                    ),
                    crate::ship_state::ShipRedAlert::default(),
                    crate::ship_plugin::LastHelmInput {
                        thrust: 0.9,
                        ..Default::default()
                    },
                    policy,
                    crate::messages::AdmittedCommands::default(),
                    AiHighFidelity,
                ))
                .id()
        };

        let ops_ship = spawn(
            &mut app,
            &[("helm", 2), ("weapons", 2), ("sensors", 1), ("ops", 1)],
            power_policy(
                &[("reserve", 10.0)],
                vec![alloc_rule(
                    10,
                    "ops",
                    "fact(thrust) >= 0.7 and fact(battery_pct) >= param(reserve)",
                    3,
                )],
            ),
        );
        let sensors_ship = spawn(
            &mut app,
            &[("helm", 2), ("weapons", 2), ("sensors", 2)],
            power_policy(
                &[("reserve", 10.0)],
                vec![alloc_rule(
                    10,
                    "sensors",
                    "fact(thrust) >= 0.7 and fact(battery_pct) >= param(reserve)",
                    4,
                )],
            ),
        );

        power_tick_with_dt(&mut app, 0.1);

        assert_eq!(power_level(&app, ops_ship, "ops"), 3, "ops ship raises ops");
        assert_eq!(
            power_level(&app, ops_ship, "sensors"),
            1,
            "ops ship leaves sensors alone"
        );
        assert_eq!(
            power_level(&app, sensors_ship, "sensors"),
            4,
            "sensors ship raises sensors to its authored level"
        );
        assert_eq!(
            power_level(&app, sensors_ship, "helm"),
            2,
            "sensors ship leaves helm alone"
        );
    }

    #[test]
    fn highest_priority_matching_rule_wins_on_one_group() {
        // AC4 conflicting rules on ONE group: two rules target `weapons`, both
        // firing this tick. The higher-priority rule's absolute level wins.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut().entity_mut(e).insert(power_policy(
            &[("reserve", 10.0)],
            vec![
                // Low priority: hold weapons at 2.
                alloc_rule(0, "weapons", "true", 2),
                // High priority: on red alert, elevate to 4 — this must win.
                alloc_rule(
                    10,
                    "weapons",
                    "fact(red_alert) > 0 and fact(battery_pct) >= param(reserve)",
                    4,
                ),
            ],
        ));
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;

        power_tick_with_dt(&mut app, 0.1);

        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            4,
            "the highest-priority matching weapons rule wins the channel"
        );
    }

    #[test]
    fn authored_guard_fires_from_seeded_facts() {
        // The #779 empty-facts lesson, applied to power: an authored `fact(...)`
        // guard actually fires because the host SEEDS the fact. Here a guard on
        // the seeded `total_allocation` ship fact elevates shields only once the
        // total crosses the authored threshold.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        // Default seed totals 2+2+2 = 6. A guard `total_allocation >= 6` fires;
        // `>= 7` would not.
        app.world_mut().entity_mut(e).insert(power_policy(
            &[],
            vec![alloc_rule(10, "shields", "fact(total_allocation) >= 6", 3)],
        ));

        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::SHIELDS_POWER_GROUP),
            3,
            "a guard reading the seeded total_allocation fact fires"
        );
    }

    #[test]
    fn scenario_flag_guard_gates_allocation() {
        // AC3: a rule may read read-only SCENARIO flags. Weapons stays at
        // baseline until a world flag is set; once the `WorldContentRuntime`
        // flag chain carries it, the same tick elevates weapons.
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
            .init_resource::<crate::world::server::WorldContentRuntime>()
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
        let mut cs = ShipSystemControlSources::default();
        cs.0.set(
            crate::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
        let e = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                cs,
                crate::ship::power::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::default(),
                ),
                crate::ship_state::ShipRedAlert::default(),
                crate::ship_plugin::LastHelmInput::default(),
                power_policy(
                    &[],
                    vec![alloc_rule(10, "weapons", "flag(battle_stations)", 4)],
                ),
                crate::messages::AdmittedCommands::default(),
                AiHighFidelity,
            ))
            .id();

        // Flag unset → weapons holds at its seeded baseline 2.
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            2,
            "with the scenario flag unset the guard does not fire"
        );

        // Set the scenario flag → the same guard now fires and elevates weapons.
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .flags
            .set_flag("battle_stations");
        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            4,
            "once the scenario flag is set the guard fires (AC3 read-only flags)"
        );
    }

    /// Issue #891 stage 2, the LAYERING half: the chain a host passes is
    /// anchored at the layer that spawned the ship — not flattened onto the
    /// base store. A flag set only in the spawning LAYER's store fires the
    /// ship's guard, and a `parent:`-prefixed guard reads the base store from
    /// there, exactly as a trigger authored in that layer would.
    #[test]
    fn scenario_flag_chain_is_anchored_at_the_ships_spawning_layer() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .init_resource::<crate::ship::power::PowerConfigResource>()
            .init_resource::<crate::world::server::WorldContentRuntime>()
            .init_resource::<crate::world::server::WorldLayerMap>()
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
        let mut cs = ShipSystemControlSources::default();
        cs.0.set(
            crate::system_registry::power_reactor_system_id(),
            ControlSource::Ai,
        );
        let e = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                cs,
                crate::ship::power::ShipPowerSystem(
                    crate::modifiers::power_system::PowerSystem::default(),
                ),
                crate::ship_state::ShipRedAlert::default(),
                crate::ship_plugin::LastHelmInput::default(),
                power_policy(
                    &[],
                    vec![
                        alloc_rule(10, "weapons", "flag(layer_flag)", 4),
                        // A DROP, so the ship-wide total cap cannot mask the
                        // read: two simultaneous elevations would fight the
                        // total-allocation clamp.
                        alloc_rule(10, "shields", "flag(parent:base_flag)", 1),
                        // The mirror-image case, driven through the real host
                        // rather than asserted against a hand-built chain
                        // (issue #891 review finding 4): an UNPREFIXED guard
                        // on `base_flag` must NOT fire for this layer-spawned
                        // ship. `resolve_chain` indexes by depth, so an
                        // unprefixed name reads chain[0] — the spawning
                        // layer's own store — and `base_flag` lives only in
                        // the base store two hops further out.
                        alloc_rule(10, "helm", "flag(base_flag)", 3),
                    ],
                ),
                crate::messages::AdmittedCommands::default(),
                AiHighFidelity,
            ))
            .id();

        // The ship was spawned by a loaded sub-world layer; the layer's OWN
        // store carries `layer_flag`, the BASE store carries `base_flag`.
        // Stamping `EntityOriginLayer` mirrors what the two real spawn sites
        // do (issue #891 review finding 1) — `entity_flag_chain` now reads
        // the origin off this component, not off `spawned_entities`.
        {
            let mut layer = crate::world::server::WorldRuntime::default();
            layer.flags.set_flag("layer_flag");
            layer.spawned_entities.push(e);
            app.world_mut()
                .resource_mut::<crate::world::server::WorldLayerMap>()
                .0
                .insert("assets/worlds/sub.toml".to_string(), layer);
            app.world_mut()
                .resource_mut::<crate::world::server::WorldContentRuntime>()
                .flags
                .set_flag("base_flag");
            app.world_mut()
                .entity_mut(e)
                .insert(crate::world::server::EntityOriginLayer(
                    "assets/worlds/sub.toml".to_string(),
                ));
        }

        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            4,
            "a flag set in the SPAWNING LAYER's store fires the layer-spawned \
             ship's guard — the chain is anchored at the layer, not the base"
        );
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::SHIELDS_POWER_GROUP),
            1,
            "a `parent:`-prefixed guard climbs from the layer to the BASE store \
             — the chain is layered, not flattened"
        );
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
            2,
            "an UNPREFIXED guard on a layer-spawned ship reads the layer store, \
             not the base store — base_flag never reaches it, so the rule does \
             not fire and helm holds its default level"
        );
    }

    #[test]
    fn idle_policy_holds_every_group() {
        // A ship whose policy is an explicit idle takes no power action.
        let mut app = power_test_app();
        let e = power_ship_entity(&mut app);
        app.world_mut()
            .entity_mut(e)
            .insert(PowerAiPolicy(crate::ai::policy::AiPolicy {
                idle: true,
                ..Default::default()
            }));
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_state::ShipRedAlert>()
            .unwrap()
            .0 = true;
        app.world_mut()
            .entity_mut(e)
            .get_mut::<crate::ship_plugin::LastHelmInput>()
            .unwrap()
            .thrust = 0.9;

        power_tick_with_dt(&mut app, 0.1);
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::WEAPONS_POWER_GROUP),
            2
        );
        assert_eq!(
            power_level(&app, e, crate::modifiers::power_system::HELM_POWER_GROUP),
            2
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
                barrels: Vec::new(),
                pattern: Vec::new(),
                volley_max: 2,
                ai_target_count: None,
                ai: None,
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
                // The SHIPPED authored per-tube policy. Since #885b stage 5d a
                // tube with no entry in `TorpedoTubeAiPolicies` is never ordered
                // to load — there is no synthesised stand-in.
                crate::weapons_plugin::TorpedoTubeAiPolicies(
                    [(
                        "fore_port".to_string(),
                        crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube")
                            .to_policy()
                            .expect("the shipped torpedo-tube policy decodes"),
                    )]
                    .into_iter()
                    .collect(),
                ),
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

    // ── Per-tube LOAD policy gate (issue #782) ──────────────────────────────

    /// Attach a single-tube `TorpedoTubeAiPolicies` map to `e` for the
    /// `fore_port` tube, built from an authored `when` guard on the
    /// `torpedo_load` channel.
    fn attach_load_policy(app: &mut App, e: Entity, when: &str) {
        let ai = crate::entity_config::FineSystemAiConfigToml {
            evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
            idle: false,
            param: Default::default(),
            rule: vec![crate::entity_config::FineSystemAiRuleToml {
                priority: 0,
                channel: crate::entity_config::TORPEDO_LOAD_CHANNEL.into(),
                when: when.into(),
                verb: crate::entity_config::TORPEDO_LOAD_VERB.into(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        let mut map = std::collections::HashMap::new();
        map.insert("fore_port".to_string(), ai.to_policy().unwrap());
        app.world_mut()
            .entity_mut(e)
            .insert(crate::weapons_plugin::TorpedoTubeAiPolicies(map));
    }

    /// An idle tube policy holds the load: no `SetTorpedoVolleyTarget` is issued
    /// even though the tube is AI-operated and off its configured volley target.
    #[test]
    fn ai_torpedo_load_idle_tube_policy_holds() {
        let (mut app, e) = torpedo_load_app(ControlSource::Ai);
        let mut map = std::collections::HashMap::new();
        map.insert(
            "fore_port".to_string(),
            crate::ai::policy::AiPolicy {
                idle: true,
                ..Default::default()
            },
        );
        app.world_mut()
            .entity_mut(e)
            .insert(crate::weapons_plugin::TorpedoTubeAiPolicies(map));
        app.update();

        assert_eq!(
            tube_target_count(&app, e),
            0,
            "an idle tube policy must hold the AI load order"
        );
        assert!(
            app.world()
                .entity(e)
                .get::<AdmittedCommands>()
                .unwrap()
                .0
                .is_empty(),
            "no volley command should be admitted when the tube policy is idle"
        );
    }

    /// The #779 empty-facts lesson: the host seeds real per-tube facts, so a
    /// `fact(...)` guard actually evaluates. A guard that can never hold over the
    /// live magazine count (`fact(magazine) > 100`, magazine is 10) holds the
    /// load; the complementary guard (`fact(magazine) > 0`) fires it — proving the
    /// facts are seeded, not empty.
    #[test]
    fn ai_torpedo_load_fact_guard_fires_over_seeded_facts() {
        // Unsatisfiable guard → hold.
        let (mut app, e) = torpedo_load_app(ControlSource::Ai);
        attach_load_policy(&mut app, e, "fact(magazine) > 100");
        app.update();
        assert_eq!(
            tube_target_count(&app, e),
            0,
            "a load guard that never holds over the seeded magazine fact must hold"
        );

        // Satisfiable guard → fire. If facts were empty (#779), `fact(magazine)`
        // would read 0 and this guard would also hold — so a fire here proves the
        // magazine fact was seeded.
        let (mut app, e) = torpedo_load_app(ControlSource::Ai);
        attach_load_policy(&mut app, e, "fact(magazine) > 0");
        app.update();
        assert_eq!(
            tube_target_count(&app, e),
            2,
            "a load guard satisfied by the seeded magazine fact must fire the order"
        );
    }

    /// Issue #891 stage 2, per-host both-directions proof for the Torpedo tube
    /// LOAD host: a `flag()` guard fires when the scenario sets the flag and
    /// reads false when it does not — through the full decide → admit → apply
    /// pipeline, not the policy evaluator alone.
    #[test]
    fn ai_torpedo_load_flag_guard_reads_the_world_in_both_directions() {
        // Flag CLEAR → the guard reads false and the load holds.
        let (mut app, e) = torpedo_load_app(ControlSource::Ai);
        app.init_resource::<crate::world::server::WorldContentRuntime>();
        attach_load_policy(&mut app, e, "flag(resupply_authorised)");
        app.update();
        assert_eq!(
            tube_target_count(&app, e),
            0,
            "with the world flag clear the load guard must read false and hold"
        );

        // Flag SET → the SAME guard fires and the volley order lands.
        app.world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .flags
            .set_flag("resupply_authorised");
        app.update();
        assert_eq!(
            tube_target_count(&app, e),
            2,
            "with the world flag set the same guard must fire the load order"
        );
    }
}

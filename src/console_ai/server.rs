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

// AI rule keys — match the keys used in [[station.rating]].ai_tuning tables.
pub const AI_RULE_TORPEDO_AUTO_FIRE: &str = "torpedo_auto_fire";
// `AI_RULE_AUTO_HINT` ("auto_hint") was deleted by issue #873. It gated the
// Sensors frequency hint on whether a *human session* held the Sensors station
// and, if so, on that holder's active rating — so a coordination fact derived
// entirely from authoritative ship state stopped being emitted the moment a
// human sat down. That is the human/AI branch AGENTS.md rule 6 forbids, and no
// shipped hull authored the key anyway. Do not reintroduce it: a station rating
// tunes what a console offers its own operator, never whether the ship's state
// reaches the rest of the bridge.

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
        // The shared AI host spine's read-only world context (issue #1205). No
        // host in this plugin consumes `AiHostEnv` yet — this is the additive
        // wiring point so the bare-`Res` env is registered wherever the AI host
        // plugin runs, mirrored by `ship::test_support` for fixtures.
        crate::ai::host::register_ai_host_env(app);
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
                    .before(crate::console::weapons::handle_set_torpedo_volley_target)
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
    world_snapshot: Res<crate::ai::server::WorldSnapshot>,
    // The read-only AI-host world context — flag chain, sessions (consulted by
    // the admission emitter), and origin stamps — behind one bare-`Res` system
    // param (issue #1207). A fixture that runs this host must register it
    // (`register_ai_host_env`) or fail loudly at schedule build, so a bare `App`
    // cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship::shields::ShipShields,
            &mut crate::ship::shields::ShieldsDamageHistory,
            Option<&crate::ship::shields::ShieldsFocusAiPolicy>,
            &mut crate::ship::shields::PendingShieldsThreatBearing,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        (
            With<crate::ai::server::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
) {
    let current_time = time.elapsed_secs();

    // The AI side of the typed input path (issue #1211, which deleted the
    // per-operator `console_ai::shields_emit` shim): the emitter binds the
    // session table once, and each per-ship `emit` supplies the ship-specific
    // context. It routes the SetShieldArcFocus commands through the same shared
    // `command_admission::ai_emit` seam a human command crosses.
    let emitter = ai_env.emitter();

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
                          admitted: &mut crate::core::messages::AdmittedCommands,
                          shields: &crate::ship::shields::ShipShields| {
            let Some(sid) = shields
                .0
                .facings
                .get(idx)
                .and_then(|f| crate::ship::system_registry::shield_arc_system_id(&f.id))
            else {
                return;
            };
            emitter.emit(
                entity_uuid,
                sid,
                crate::core::messages::SystemControlPayload::SetShieldArcFocus { focused: true },
                control_sources,
                ship_config,
                admitted,
            );
        };

        let policy = control_sources
            .0
            .policy_for(&crate::ship::system_registry::shields_system_id());
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
        let flag_chain = ai_env.flag_chain(ship_entity);
        // Resolve the `shield_focus` channel through the shared AI host spine
        // (issue #1208). The Control-Source gate and the declaration check stay
        // explicit ABOVE this call because they guard the sanctioned
        // threat-bearing override and the pre-resolve params/damage analysis the
        // spine cannot see — `decide` re-confirms both (a no-op here, since we
        // only reach it AI-operated and declared) and owns the resolution.
        let tick = crate::ai::host::HostTick {
            system: crate::ship::system_registry::shields_system_id(),
            channel: crate::entities::config::SHIELD_FOCUS_CHANNEL,
            facts: &facts,
            flags: &flag_chain,
            state: None,
        };
        let acts = matches!(
            crate::ai::host::decide(&control_sources.0, Some(policy), &tick),
            crate::ai::host::HostOutcome::Act(crate::ai::policy::AiPolicyVerb::FocusShieldArc)
        );
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
                        .and_then(|f| crate::ship::system_registry::shield_arc_system_id(&f.id))
                });
                if let Some(sid) = current_sid {
                    emitter.emit(
                        entity_uuid,
                        sid,
                        crate::core::messages::SystemControlPayload::SetShieldArcFocus {
                            focused: false,
                        },
                        control_sources,
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

/// The read-only resource context `ai_power_allocation` reads besides the
/// [`crate::ai::host::AiHostEnv`], bundled as one `SystemParam` (issue #1185):
/// the session table the emit seam consults, the log filter, the global
/// objective pool scored for the OBJECTIVE fact, and the shared AI base cadence
/// (raw tick + interval).
///
/// A signature grouping only — every field keeps its type and `Option` fallback
/// (`log`/`objectives`/`tick`/`base_interval` stay `Option` for the bare-`App`
/// fixtures that never insert them), so the access set is byte-for-byte
/// unchanged; the system destructures it back to its original locals at entry.
#[derive(bevy::ecs::system::SystemParam)]
struct PowerAiContext<'w> {
    sessions: Res<'w, crate::lobby::Sessions>,
    log: Option<Res<'w, crate::logging::LogFilterConfig>>,
    objectives: Option<Res<'w, crate::world::server::ObjectiveManagerRes>>,
    tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    base_interval: Option<Res<'w, crate::ai::cadence::AiBaseInterval>>,
}

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
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    // The session table, log filter, objective pool, and shared AI base cadence
    // (tick + interval) bundled as one `SystemParam` (issue #1185). See
    // [`PowerAiContext`].
    context: PowerAiContext,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship::power::ShipPowerSystem,
            Option<&crate::ship::state::ShipRedAlert>,
            // The ACTUAL actuator throttle, written for every ship (NPC and
            // player alike) by `process_helm_inputs`. Deliberately NOT
            // `LastHelmInput`, which is only a LocalShip HUD mirror and stays
            // at its spawn default (0) on every NPC — so seeding the power
            // `thrust` fact from it pinned every NPC's helm channel below its
            // elevate guard, letting only the human-piloted player ever reach
            // the top power rung.
            Option<&crate::ship::helm::ThrustInput>,
            Option<&crate::ship::power::PowerConfigResource>,
            Option<&crate::ship::power::PowerAiPolicy>,
            Option<&crate::ship::power::PowerAiCadence>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::ship::combat_activity::RecentCombatActivity>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        (
            With<crate::ai::server::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
) {
    // Restore the pre-#1185 locals so the body below is byte-for-byte unchanged.
    let PowerAiContext {
        sessions,
        log,
        objectives,
        tick,
        base_interval,
    } = context;

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
                .any(|s| {
                    matches!(
                        s.directive,
                        crate::core::messages::AiDirective::Destroy { .. }
                    )
                })
        })
        .unwrap_or(false);

    for (
        ship_entity,
        entity_uuid,
        control_sources,
        power,
        red_alert_comp,
        thrust_comp,
        cfg_comp,
        policy_comp,
        cadence_comp,
        ship_config,
        combat_activity,
        mut admitted,
    ) in ships.iter_mut()
    {
        // Control-Source gate through the shared AI host spine (issue #1208): not
        // (or no longer) AI-driven — a human Control Source — stands the reactor
        // down. Power resolves a RANKED channel the spine does not model, so only
        // its gate — the one step it shares with the policy hosts — routes here.
        // Stateless, so nothing to reset; the next tick under AI control decides
        // cleanly.
        if !crate::ai::host::ai_operates(
            &control_sources.0,
            crate::ship::system_registry::power_reactor_system_id(),
        ) {
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
        let flag_chain = ai_env.flag_chain(ship_entity);

        let red_alert = red_alert_comp.map(|ra| ra.0).unwrap_or(false);
        let thrust = thrust_comp.map(|t| t.0).unwrap_or(0.0);
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
        let group_ids: Vec<crate::core::messages::PowerGroupId> =
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
                crate::ship::system_registry::power_reactor_system_id(),
                crate::core::messages::SystemControlPayload::SetPowerGroupAllocation {
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
///   own authored guard is what holds fire. Phasers strip the shields, torpedoes
///   finish the hull — said in TOML, and therefore sayable differently by
///   different hulls. Every armed tube currently authors
///   `fact(target_facing_shields) <= 0`, and it is worth more than it used to be:
///   since issue #929 a round delivers `damage_hull` only into a DOWN arc and the
///   far smaller `damage_shields` into a live one, so the gate decides the
///   payload rather than merely the timing. Do not read a fleet-wide rule out of
///   this seeding all the same — #929's first pass had `alliance_cruiser`'s three
///   tubes compare against a `param(max_striking_shield_hp)` set past any arc
///   reading, switching the conjunct off, and its second pass put the fleet text
///   back and raised that hull's guns instead.
///   `torpedo_launch_shield_gate_truth_table` is the enumeration; this seeding
///   takes no position.
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
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    // Objective-contributed Command stances (issue #1110). `Option<Res<_>>` so a
    // bare-`App` weapons fixture with no world plugin reads no contributions.
    active_objective_stances: Option<Res<crate::console::command::server::ActiveObjectiveStances>>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipConfigComponent,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ActiveStationRatings,
            &crate::ship::state::ShipPhysics,
            &crate::server_app::ShipSystemBlackboards,
            Option<&crate::console::weapons::TorpedoSystemResource>,
            Option<&crate::console::weapons::TorpedoTubeAiPolicies>,
            &mut crate::core::messages::AdmittedCommands,
            // Issue #872: this ship's own red-alert state, seeded as a typed
            // fact for the tube's authored LAUNCH predicate. `Option<&_>` for
            // fixtures that spawn a ship without it; absent reads `false`.
            Option<&crate::ship::state::ShipRedAlert>,
            // Issue #1041: the captain's weapons hold, folded with the alert
            // above into the one `red_alert` fact the tube's authored LAUNCH
            // predicate already reads.
            Option<&crate::ship::state::ShipWeaponsHold>,
            // Issue #1107: the ship's Command stance selections, so a directed
            // AI weapons Station's stance decides the launch posture in place of
            // the ship's own Red Alert. Absent reads as no direction.
            Option<&crate::console::command::server::ShipStationStances>,
        ),
        (
            With<crate::ai::server::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
    asteroid_q: Query<
        (&crate::server_app::AsteroidUuid, &Transform),
        With<crate::server_app::Asteroid>,
    >,
    other_ships_q: Query<
        (
            &crate::entities::spawner::EntityUuid,
            &Transform,
            Option<&crate::ship::shields::ShipShields>,
            Option<&crate::ship::state::ShipPhysics>,
        ),
        Without<crate::server_app::Asteroid>,
    >,
) {
    let policy_sid = crate::ship::system_registry::torpedo_magazine_system_id();

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
        weapons_hold_opt,
        stances_opt,
    ) in ships.iter_mut()
    {
        // Read once per ship; seeded into every tube's launch snapshot. No Rust
        // rule consults it — the gate is the tube's authored predicate (#872),
        // and the weapons hold folded in beside it rides that same predicate
        // (#1041). The Command stance override (#1107) rides the same fact;
        // absent a direction it is `None` and the seeded value is unchanged.
        let stance_override = crate::console::command::server::weapons_station_stance_high_alert(
            stances_opt,
            active_objective_stances.as_deref(),
            &ship_config.0,
            &control_sources.0,
            red_alert_opt.is_some_and(|r| r.0),
        );
        let posture = crate::console::weapons::WeaponsAlertPosture::from_parts(
            red_alert_opt,
            weapons_hold_opt,
            stance_override,
        );
        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(ship_entity);
        // Control-Source gate through the shared AI host spine (issue #1208): the
        // torpedo MAGAZINE's own operate_ai is the natural per-ship gate — the
        // shared bottleneck resource across tubes, and there is no unified
        // torpedo_system_id. The per-tube LAUNCH resolution stays in
        // `torpedo_tube_launch_policy_fires`.
        if !crate::ai::host::ai_operates(&control_sources.0, policy_sid.clone()) {
            continue;
        }

        // The station owning this ship's weapons — resolved per-ship rather
        // than assumed to be named "tactical". NPCs have no weapons owner, so
        // the fallback keeps them on the unclaimed (fire unconditionally) path.
        let tactical_station = ship_config.0.weapons_station().unwrap_or_else(|| {
            crate::core::messages::StationId(
                crate::ship::system_registry::TACTICAL_STATION_ID.into(),
            )
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
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
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
        let torpedo_sys: &crate::weapons::torpedo::TorpedoSystem = &torpedo_sys_comp.0;
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
            let facts = crate::console::weapons::seed_torpedo_tube_launch_facts(
                true,
                true,
                true,
                true,
                target_facing_shields,
                tubes_full,
                posture,
            );
            if !crate::console::weapons::torpedo_tube_launch_policy_fires(
                launch_policy,
                &facts,
                &flag_chain,
            ) {
                continue;
            }
            // Emit as an admitted command through the shared AI seam (issue
            // #846), instead of the retired `TorpedoIntents` buffer.
            let Some(target) = crate::ship::system_registry::torpedo_tube_system_id(&tube_id)
            else {
                continue;
            };
            crate::command_admission::ai_emit::emit_ai_command(
                entity_uuid,
                target,
                crate::core::messages::SystemControlPayload::FireTorpedo {
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
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &crate::console::weapons::TorpedoSystemResource,
            Option<&crate::console::weapons::TorpedoTubeAiPolicies>,
            &mut crate::core::messages::AdmittedCommands,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let magazine_id = crate::ship::system_registry::torpedo_magazine_system_id();

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
        // Control-Source gate through the shared AI host spine (issue #1208): the
        // torpedo MAGAZINE's own operate_ai. The per-tube LOAD resolution stays in
        // `torpedo_tube_load_policy_fires`.
        if !crate::ai::host::ai_operates(&control_sources.0, magazine_id.clone()) {
            continue;
        }

        // The scenario flag chain, anchored at the layer that spawned this
        // ship (issue #891 stage 2).
        let flag_chain = ai_env.flag_chain(ship_entity);

        let tubes: Vec<crate::console_ai::TubeLoadSummary> = torpedo_sys
            .0
            .tubes
            .iter()
            .map(|tube| {
                let tube_system_id = crate::ship::system_registry::torpedo_tube_system_id(&tube.id)
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
            let facts = crate::console::weapons::seed_torpedo_tube_load_facts(
                tube_ref.map(|t| t.loaded_count).unwrap_or(0),
                tube_ref.map(|t| t.target_count).unwrap_or(0),
                count,
                torpedo_sys.0.torpedoes_remaining,
                true,
            );
            if !crate::console::weapons::torpedo_tube_load_policy_fires(policy, &facts, &flag_chain)
            {
                continue;
            }
            let Some(target) = crate::ship::system_registry::torpedo_tube_system_id(&tube_id)
            else {
                continue;
            };
            // Through the shared AI-emit seam (issue #738), never a raw
            // `admitted.0.push`: admission is the only place that decides what
            // an `ai:` token may do, and it re-checks `operate_ai` on this
            // exact tube SystemId.
            let admitted_ok = emit_ai_command(
                entity_uuid,
                target.clone(),
                crate::core::messages::SystemControlPayload::SetTorpedoVolleyTarget { count },
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
            &crate::ship_plugin::ShipConfigComponent,
            &mut ShipFrequencyHintState,
            Option<&crate::ship::sensors::SensorsAiConfigResource>,
        ),
        (
            With<crate::ai::server::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
    target_shields_q: Query<(
        &crate::entities::spawner::EntityUuid,
        &crate::ship::shields::ShipShields,
    )>,
    mut writer: MessageWriter<crate::ship_plugin::CoordinationEnqueue>,
) {
    let hz = world_config
        .as_deref()
        .map(|wc| wc.global.ai_tick_hz)
        .unwrap_or_else(|| crate::entities::config::GlobalConfig::default().ai_tick_hz);
    let dt = if hz > 0.0 { 1.0 / hz } else { 0.0 };
    let sensors_sid = crate::ship::system_registry::sensors_system_id();

    for (entity, control_sources, blackboards, ship_config, mut hint_state, ai_config_comp) in
        ships.iter_mut()
    {
        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3),
        // identical to how the low-fidelity twin `tick_sensors_frequency_hint`
        // and the firing paths read it — never the tactical radar's live
        // selection.
        let locked_target = match blackboards
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(bb)) => bb.combat_lock.clone(),
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
            let Some(address) = crate::ship::coordination::address_for_system(
                &ship_config.0,
                &crate::ship::system_registry::tactical_radar_system_id(),
            ) else {
                continue;
            };
            writer.write(crate::ship_plugin::CoordinationEnqueue {
                source_entity: entity,
                sender_origin,
                address,
                payload: crate::core::messages::CoordinationPayload::FrequencyHint { frequency },
                presentation: crate::core::messages::CoordinationPresentation::new(
                    "coordination.frequency_hint.title",
                    "coordination.frequency_hint.body",
                )
                .with_body_param("frequency", frequency),
                sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
                sender_system: sensors_sid.clone(),
            });
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;

use crate::simmath;
use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::core::messages::{
    AdmittedCommands, CoordinationPayload, ShieldArcBlackboard, ShieldFacingStatus,
    ShieldsBlackboard, SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::ship_plugin::CoordinationEnqueue;

/// Pending Sensors->Shields threat bearing, delivered via the channel-3
/// coordination bus (issue #683). Set by `process_coordination_lag` when a
/// `CoordinationPayload::ThreatBearing` is consumed by an AI-controlled
/// Shields; read by `console_ai::server::ai_shield_focus` to bias facing
/// toward the threat.
#[derive(Component, Clone, Debug, Default)]
pub struct PendingShieldsThreatBearing(pub Option<f32>);

// `ShieldArcCmd` / `ShieldArcIntents` (issue #692's decide/apply transport)
// were retired by issue #826: `console_ai::server::ai_shield_focus` now emits
// admitted `SetShieldArcFocus` payloads through
// `command_admission::validate_and_admit`, and `handle_shields_messages`
// below is the single applier for human and AI commands alike.

// ── Components ─────────────────────────────────────────────────────────────────

/// The ship's shield system.
///
/// Per-ship shield system — a `ShieldSystem` wrapped in a Component.
///
/// Pure per-ship Component post ship-parity audit; the legacy `Resource`
/// derive has been dropped since no production code reads a global
/// `Res<ShipShields>`.
#[derive(Component)]
pub struct ShipShields(pub crate::weapons::shield::ShieldSystem, pub f32);

impl ShipShields {
    pub fn frequency(&self) -> f32 {
        self.1
    }
}

// ── Resources ──────────────────────────────────────────────────────────────────

/// A single damage event recorded for the shields AI damage-tracking window.
#[derive(Clone, Debug)]
pub struct DamageRecord {
    /// Absolute time (seconds) at which damage was recorded.
    pub timestamp: f32,
    /// Amount of HP lost in this damage event.
    pub amount: i32,
}

/// Per-arc damage history for the shields AI controller.
///
/// Indexed by facing index (`Vec<Vec<DamageRecord>>`). Each record is
/// stamped with an absolute time and pruned when older than
/// `ShieldsAiConfigResource::damage_window_secs`.
///
/// Per-ship `Component` so each ship (player + NPC) maintains independent
/// tracking. Initialised lazily to match the ship's actual arc count.
#[derive(Component, Default, Clone)]
pub struct ShieldsDamageHistory {
    pub arcs: Vec<Vec<DamageRecord>>,
    /// Last-observed HP per arc, updated every tick regardless of whether
    /// damage occurred. Used as the damage-detection baseline instead of
    /// the last recorded `DamageRecord.amount` (which is a delta, not an
    /// HP value, and cannot be reused for that purpose).
    pub last_hp: Vec<i32>,
}

impl ShieldsDamageHistory {
    /// `pub(crate)`: called from `console_ai::server::ai_shield_focus`
    /// (issue #692 split the old fused `operate_shields_ai` out of this
    /// module).
    pub(crate) fn ensure_len(&mut self, n: usize) {
        if self.arcs.len() < n {
            self.arcs.resize(n, Vec::new());
        }
        if self.last_hp.len() < n {
            // 0 is a safe placeholder: HP is never negative, so a
            // freshly-grown slot can never register a spurious "damage"
            // detection on the tick it's created — the real HP is written
            // into it unconditionally at the end of that same tick's
            // detection loop, before any future comparison happens.
            self.last_hp.resize(n, 0);
        }
    }

    /// Baseline HP for damage detection: the value observed last tick, or
    /// the current HP if this arc has never been observed.
    pub(crate) fn last_observed_hp(&self, facing_idx: usize, current_hp: i32) -> i32 {
        self.last_hp.get(facing_idx).copied().unwrap_or(current_hp)
    }

    /// Records this tick's HP as the new baseline for the next tick's
    /// damage-detection comparison. Must be called once per arc, per tick,
    /// after `last_observed_hp` has been consulted.
    pub(crate) fn observe_hp(&mut self, facing_idx: usize, current_hp: i32) {
        if facing_idx < self.last_hp.len() {
            self.last_hp[facing_idx] = current_hp;
        }
    }

    pub(crate) fn record_damage(&mut self, facing_idx: usize, timestamp: f32, amount: i32) {
        if facing_idx < self.arcs.len() {
            self.arcs[facing_idx].push(DamageRecord { timestamp, amount });
        }
    }

    pub(crate) fn prune_old(&mut self, current_time: f32, window_secs: f32) {
        let cutoff = current_time - window_secs;
        for arc in &mut self.arcs {
            arc.retain(|r| r.timestamp > cutoff);
        }
    }
}

/// TOML-loaded configuration for the shields AI controller.
///
/// Loaded from `[shields_console.ai]` in the ship entity TOML. Defaults are
/// used when the section is absent.
///
/// Dual `Resource + Component`. Since issue #738 **every production read goes
/// through the per-entity Component** — `ai_shield_focus` and
/// `emit_shields_coordination` both query it, and the spawner always attaches
/// one. The `Resource` form survives only as `server_app`'s dual-write of the
/// PLAYER ship's tuning; nothing reads it. Do not reintroduce a `Res<_>` read:
/// it applies one ship's tuning to every ship.
#[derive(Resource, Component, Clone, Debug)]
pub struct ShieldsAiConfigResource {
    /// HP fraction (0.0–1.0) at or above which a restored facing fires the
    /// `ShieldFacingRestored` coordination message to Helm.
    pub restored_notify_pct: f32,
    /// Maximum time window (seconds) for tracking incoming damage per arc.
    pub damage_window_secs: f32,
    /// Minimum time window (seconds) before the AI acts on damage concentration.
    pub min_damage_window_secs: f32,
    /// Percentage of total damage in the window that must hit the same arc
    /// before the AI focuses it (0.0–100.0).
    pub damage_pct_threshold: f32,
    /// Percentage threshold: if the lowest-arc normalized health is below this
    /// fraction of the next-lowest arc, focus the weakest arc (0.0–100.0).
    pub health_ratio_threshold: f32,
}

impl Default for ShieldsAiConfigResource {
    fn default() -> Self {
        Self {
            restored_notify_pct: 0.5,
            damage_window_secs: 4.0,
            min_damage_window_secs: 1.0,
            damage_pct_threshold: 50.0,
            health_ratio_threshold: 50.0,
        }
    }
}

/// Per-ship inline stateless AI policy for the Shields focus fine system
/// (issue #783), the shields twin of the helm axes' authored policies (the
/// [`crate::ship::helm_ai::FineSystemAiPolicies`] map)
/// / [`crate::console::captain::server::CaptainAiPolicy`].
///
/// Built at spawn from `[shields_console.ai_policy]` if authored, else the
/// canonical [`crate::entities::config::default_shields_focus_ai_config`] (the
/// default reproduces today's decisions — see that fn). Read by
/// [`crate::console_ai::server::ai_shield_focus`]: the host seeds bounded per-arc
/// recent-damage facts ([`crate::console_ai::seed_shields_focus_facts`]) and
/// resolves this policy on the `shield_focus` channel. On `focus_shield_arc`
/// (act) the retained arc-ranking kernel picks and emits the arc; on `None`
/// (idle/hold) the host emits nothing. The authored windows/thresholds flow into
/// the kernel from this policy's `param` map, so the policy owns the authored
/// numbers while the kernel owns the 4-way argmax.
#[derive(Component, Default, Clone, Debug)]
pub struct ShieldsFocusAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-facing notification state for the shields coordination emitter.
///
/// Indexed by facing index (usize). Both flags reset when a facing comes back
/// online so the down/restore cycle repeats on the next offline event.
///
/// Per-ship Component so NPC ships' shields can emit their own advisories
/// through their own `CoordinationQueue` without stepping on the player's
/// shield-notification state.
#[derive(Component, Default, Clone)]
pub struct ShieldsCoordinationState {
    pub down_notified: Vec<bool>,
    pub restore_notified: Vec<bool>,
}

impl ShieldsCoordinationState {
    fn ensure_len(&mut self, n: usize) {
        if self.down_notified.len() < n {
            self.down_notified.resize(n, false);
            self.restore_notified.resize(n, false);
        }
    }
}

// ── Plugin ─────────────────────────────────────────────────────────────────────

pub struct ShipShieldsPlugin;

impl Plugin for ShipShieldsPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumer (issue #833): `handle_shields_messages`
        // reads every generated `shield-arc-*` instance (one id per authored
        // facing), so the claim names both its kind and generated-id prefix.
        app.register_admitted_consumer(ConsumerMatcher::prefix(
            crate::ship::system_registry::SHIELD_ARC_KIND,
            "shield-arc-",
        ));
        app.add_message::<CoordinationEnqueue>()
            .init_resource::<ShieldsAiConfigResource>()
            .add_systems(
                FixedUpdate,
                (
                    // In `SimSet::Physics`, not Input (issue #826):
                    // `admit_system_commands` clears every ship's
                    // `AdmittedCommands` before Input each tick, and the AI
                    // decide system (`console_ai::server::ai_shield_focus`,
                    // Physics) refills it same-tick via `validate_and_admit`
                    // — so the applier must consume in Physics *after* the AI
                    // emit or AI commands would be silently lost.
                    // `ConsoleAiPlugin` declares the explicit
                    // `ai_shield_focus.before(handle_shields_messages)` edge;
                    // set ordering keeps this before `tick_shields`
                    // (Modifiers) and `publish_shields_blackboard` (Publish).
                    handle_shields_messages.in_set(crate::sim_sets::SimSet::Physics),
                    emit_shields_coordination.in_set(crate::sim_sets::SimSet::Input),
                    // `translate_power_modifiers` is ALSO in `Modifiers`, so
                    // set membership alone leaves their order unspecified and
                    // `tick_shields` would read a one-tick-stale
                    // `ModifierSlot::ShieldRegen` (issue #952). The explicit
                    // edge makes a same-tick reallocation land on this tick's
                    // regen. Dropped harmlessly in harnesses that register
                    // `ShipShieldsPlugin` without the simulation's modifier
                    // translators.
                    tick_shields
                        .in_set(crate::sim_sets::SimSet::Modifiers)
                        .after(crate::modifiers::coordination::translate_power_modifiers),
                    publish_shields_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                ),
            )
            .add_plugins(shields_state_broadcaster());
    }
}

/// Tick shield regen and offline timers each frame for every ship
/// (player + NPCs). PR-7 (issue #597) unifies this with the old
/// `tick_npc_shield_regen` — one system iterating all ships with `Ship` marker.
///
/// Each ship regenerates at its own [`ModifierSlot::ShieldRegen`] multiplier
/// (issue #952), which the `shields` power group drives through
/// `modifiers::coordination::apply_power_modifiers_from_read_state`. A ship
/// without a `ShipModifiers` component regenerates at ×1.0 — its arcs'
/// authored rates, unchanged.
///
/// Runs in `SimSet::Modifiers`, i.e. AFTER `translate_power_modifiers`, so a
/// reallocation made this tick is already in the slot when it is read here.
pub fn tick_shields(
    time: Res<Time>,
    mut shields_q: Query<
        (&mut ShipShields, Option<&crate::modifiers::ShipModifiers>),
        With<crate::server_app::Ship>,
    >,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut shield, mods) in shields_q.iter_mut() {
        let regen_scale = mods
            .map(|m| m.get(&crate::core::messages::ModifierSlot::ShieldRegen))
            .unwrap_or(1.0);
        shield.0.tick_with_regen_scale(dt, regen_scale);
    }
}

// ── Broadcaster ────────────────────────────────────────────────────────────────

pub fn shields_state_broadcaster() -> SimBroadcaster {
    SimBroadcaster::new().register(
        Audience::HoldingSystem(SystemId("shields-system".into())),
        Cadence::Hz(10.0),
        |world: &mut World| {
            let Ok(shields) = world
                .query_filtered::<&ShipShields, With<crate::server_app::LocalShip>>()
                .single(world)
            else {
                return vec![];
            };
            let facings = shield_facing_statuses(&shields.0.snapshot());
            let frequency = shields.frequency();
            vec![crate::core::messages::ServerMessage::ShieldStatus { facings, frequency }]
        },
    )
}

// ── Systems ────────────────────────────────────────────────────────────────────

/// Handle `SetShieldArcFocus` messages from every ship's Shields console.
///
/// Iterates every ship (player + NPC) so both the player's Shields console
/// commands and the AI's admitted `ai_shield_focus` emissions (issue #826)
/// flip each ship's own shield focus — one applier, no origin branching.
///
/// Per-arc dispatch (#514): each `[[shield_arc]]` synthesises a
/// `SystemId("shield-arc-<id>")`; the JS panel sends
/// `SetShieldArcFocus { focused: bool }` targeted at that arc's SystemId.
/// The handler iterates the facings on each ship and picks up admitted
/// commands per arc target. Setting `focused = true` on one arc clears
/// focus on any other arc (the shield system carries a single focus slot).
pub fn handle_shields_messages(
    mut ship_query: Query<(&AdmittedCommands, &mut ShipShields), With<crate::server_app::Ship>>,
) {
    for (admitted, mut shields) in ship_query.iter_mut() {
        // Snapshot arc ids first so we don't hold an immutable borrow across
        // the mutable `set_focused_facing` call.
        let arc_targets: Vec<(String, crate::core::messages::SystemId)> = shields
            .0
            .facings
            .iter()
            .filter_map(|f| {
                if f.id.is_empty() {
                    None
                } else {
                    crate::ship::system_registry::shield_arc_system_id(&f.id)
                        .map(|sid| (f.id.clone(), sid))
                }
            })
            .collect();

        // Track the desired new focus: `Some(Some(idx))` = focus this idx,
        // `Some(None)` = clear focus, `None` = no change.
        let mut new_focus: Option<Option<usize>> = None;

        for (arc_id, sid) in &arc_targets {
            for cmd in admitted.for_target(&sid.0) {
                let SystemControlPayload::SetShieldArcFocus { focused } = &cmd.payload else {
                    continue;
                };
                if *focused {
                    // Locate the arc index and mark it as the new focus.
                    let idx = shields.0.facings.iter().position(|f| f.id == *arc_id);
                    if let Some(i) = idx {
                        new_focus = Some(Some(i));
                    }
                } else {
                    // Clear focus only if the request is targeting the
                    // currently focused arc — matches the "toggle off"
                    // behaviour of the previous SetShieldFocus{ facing: None }
                    // payload.
                    let current_focus_arc_id = shields
                        .0
                        .focused_facing
                        .and_then(|i| shields.0.facings.get(i).map(|f| f.id.clone()));
                    if current_focus_arc_id.as_deref() == Some(arc_id.as_str()) {
                        new_focus = Some(None);
                    }
                }
            }
        }

        if let Some(focus) = new_focus {
            shields.0.set_focused_facing(focus);
        }
    }
}

/// Emit `ShieldFacingDown` and `ShieldFacingRestored` coordination messages
/// per-ship via the centralized `CoordinationEnqueue` channel (channel 3).
///
/// Iterates every ship (player + NPC). Each `CoordinationEnqueue` stamps
/// its source ship so `handle_coordination_enqueue` routes it into the
/// correct ship's `CoordinationQueue` component.
pub fn emit_shields_coordination(
    mut ship_q: Query<
        (
            Entity,
            &ShipShields,
            &crate::ship::state::ShipRedAlert,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::ship_plugin::ShipConfigComponent,
            &mut ShieldsCoordinationState,
            Option<&ShieldsAiConfigResource>,
        ),
        With<crate::server_app::Ship>,
    >,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    // Per-ship tuning only (issue #738). This used to read the global
    // `ShieldsAiConfigResource` Resource — which `server_app` writes from the
    // PLAYER ship's `[shields_console.ai]` TOML — while iterating EVERY ship,
    // so every NPC's restore-notification threshold followed the player's. The
    // spawner now always attaches the per-entity Component; a ship without one
    // falls back to the parse-time default the type already supplies for a TOML
    // that omits the section.
    let default_ai_cfg = ShieldsAiConfigResource::default();
    for (
        entity,
        shields,
        red_alert,
        control_sources,
        ship_config,
        mut coord_state,
        ai_config_comp,
    ) in ship_q.iter_mut()
    {
        let ai_config: &ShieldsAiConfigResource = ai_config_comp.unwrap_or(&default_ai_cfg);
        let snapshots = shields.0.snapshot();
        coord_state.ensure_len(snapshots.len());

        let red_alert = red_alert.0;
        // Post-#514: the coarse `shields` SystemId is no longer a registered
        // fine system. Coordination messages carry the sender's origin
        // (Human vs AI); pick the *first* arc's control source as the
        // representative sender origin — it matches the ship-wide
        // coordination surface (a single shields console operator drives
        // all arcs). Fall back to the default `ControlSource` if no arc is
        // configured (very unusual).
        let first_arc_sid = snapshots
            .iter()
            .find_map(|s| crate::ship::system_registry::shield_arc_system_id(&s.id));
        let sender_origin = first_arc_sid
            .as_ref()
            .map(|sid| control_sources.0.source_for(sid))
            .unwrap_or_default();
        let Some(helm_address) = crate::ship::coordination::address_for_system(
            &ship_config.0,
            &crate::ship::system_registry::helm_steering_system_id(),
        ) else {
            continue;
        };

        for (i, snap) in snapshots.iter().enumerate() {
            if !snap.online {
                if !coord_state.down_notified[i] {
                    coord_state.down_notified[i] = true;
                    coord_state.restore_notified[i] = false;

                    let payload = CoordinationPayload::ShieldFacingDown {
                        label: snap.label.clone(),
                        offline_remaining: snap.offline_remaining,
                    };
                    let presentation = crate::core::messages::CoordinationPresentation::titled(
                        "coordination.shield_offline.title",
                    )
                    .with_title_param("label", snap.label.clone());
                    writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin,
                        address: helm_address.clone(),
                        payload,
                        presentation,
                        sender_label: crate::ship::coordination::CHATTER_SENDER_SHIELDS.to_string(),
                        sender_system: first_arc_sid
                            .clone()
                            .unwrap_or_else(crate::ship::system_registry::shields_system_id),
                    });
                }
            } else {
                // Facing is online. Check for restore notification before clearing state.
                if coord_state.down_notified[i]
                    && !coord_state.restore_notified[i]
                    && red_alert
                    && snap.max_hp > 0
                    && (snap.hp as f32 / snap.max_hp as f32) >= ai_config.restored_notify_pct
                {
                    coord_state.restore_notified[i] = true;

                    let payload = CoordinationPayload::ShieldFacingRestored {
                        label: snap.label.clone(),
                    };
                    let presentation = crate::core::messages::CoordinationPresentation::titled(
                        "coordination.shield_restored.title",
                    )
                    .with_title_param("label", snap.label.clone());
                    writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin,
                        address: helm_address.clone(),
                        payload,
                        presentation,
                        sender_label: crate::ship::coordination::CHATTER_SENDER_SHIELDS.to_string(),
                        sender_system: first_arc_sid
                            .clone()
                            .unwrap_or_else(crate::ship::system_registry::shields_system_id),
                    });
                }

                // Reset cycle state when facing returns to full online status so
                // the next offline event starts fresh.
                if coord_state.restore_notified[i] || !coord_state.down_notified[i] {
                    // Already clean — nothing to reset.
                } else if snap.max_hp > 0
                    && (snap.hp as f32 / snap.max_hp as f32) >= ai_config.restored_notify_pct
                    && !red_alert
                {
                    // Facing recovered but not on red alert; clear so next cycle works.
                    coord_state.down_notified[i] = false;
                    coord_state.restore_notified[i] = false;
                }
            }
        }
    }
}

// ── Wire conversion ──────────────────────────────────────────────────────────

/// Convert a `ShieldSystem::snapshot()` result into the wire
/// `Vec<ShieldFacingStatus>` shape.
///
/// The one conversion every broadcaster of a ship's shield facings uses:
/// this ship's own `ShieldsBlackboard.facings` (below, via
/// `publish_shields_blackboard`), a reconnecting client's resync
/// `ShieldStatus` (`core::broadcast::cache_registry::resync_for_token`), the
/// `SimState` world snapshot other ships see this ship's facings through
/// when it is their Sensors target (issue #927,
/// `server_app::build_sim_state_entity_states`), and the periodic 10 Hz
/// `ShieldStatus` broadcast to the holder of the authored Shields System
/// (`shields_state_broadcaster`).
/// One producer, four callers, so a facing field added here reaches all
/// four without a second hand-written mapping to drift out of sync.
pub fn shield_facing_statuses(
    snapshots: &[crate::weapons::shield::ShieldFacingSnapshot],
) -> Vec<ShieldFacingStatus> {
    snapshots
        .iter()
        .map(|s| ShieldFacingStatus {
            label: s.label.clone(),
            hp: s.hp,
            max_hp: s.max_hp,
            online: s.online,
            offline_remaining: s.offline_remaining,
            is_focused: s.is_focused,
            center_deg: s.center_deg,
            width_deg: s.width_deg,
            arc_id: s.id.clone(),
            priority: s.priority,
        })
        .collect()
}

/// Project a `shield_facing_statuses()` result for the `SimState`
/// broadcaster's delta gate: identical to the input except `offline_remaining`
/// is bucketed to whole seconds (ceiling), so the projection is stable
/// between second-boundary crossings.
///
/// `ShieldFacingStatus` derives `PartialEq` over every field, including
/// `offline_remaining` — which `tick_shields` decrements continuously
/// through a ~30s recovery. Comparing raw `shield_facing_statuses()` output
/// tick-over-tick therefore reports "changed" on effectively every 10 Hz
/// tick while any facing is offline, even though nothing a player can
/// perceive moved. The client has no sub-second countdown display (only an
/// ONLINE/OFFLINE state and, on the battleship's dedicated Sensors readout,
/// no numeric remaining-time render at all), so whole seconds is the
/// coarsest bucket that still reports every honestly-observable change —
/// finer buckets would just be resending noise, and no bucket at all (i.e.
/// dropping `offline_remaining` from comparison entirely) would hide a
/// facing coming back online a tick early inside a `[1.0, 2.0)` window.
/// Callers gate a delta-cache comparison on this projection's equality; the
/// wire payload itself is never built from this — it stays the raw,
/// unbucketed `shield_facing_statuses()` value so the receiver still gets
/// full precision whenever a send actually happens.
pub fn shields_delta_projection(
    facings: &Option<Vec<ShieldFacingStatus>>,
) -> Option<Vec<ShieldFacingStatus>> {
    facings.as_ref().map(|fs| {
        fs.iter()
            .cloned()
            .map(|mut f| {
                f.offline_remaining = f.offline_remaining.max(0.0).ceil();
                f
            })
            .collect()
    })
}

// ── Blackboard publish ─────────────────────────────────────────────────────────

/// Publish every ship's own `Shields` aggregate + per-arc `ShieldArc`
/// blackboards into that ship's `ShipSystemBlackboards` (issue #826 — was
/// LocalShip-only; per-Ship following the #824 helm precedent).
///
/// No field here is player-only: hull integrity, control sources, and physics
/// are all read from the same entity being published, so there is no
/// `Has<LocalShip>` split — every ship gets the identical derivation.
/// `combat_lock_bearing` reads this ship's OWN frozen viewscreen
/// `combat_lock` (issue #829, spec §3), not a live targeting component.
/// `threat_bearing` reads this ship's OWN `ship::sensors::SensorsThreatState`
/// (issue #926) — the same authoritative fact
/// `console_ai::server::ai_shield_focus` reads (delayed, via the channel-3
/// `PendingShieldsThreatBearing` inbox) to override the damage-based focus
/// decision. Reading the live component here, rather than the AI's one-shot
/// coordination inbox, is deliberate: `PendingShieldsThreatBearing` is
/// consumed (`Option::take`) the instant the AI reads it and is only ever
/// populated for an AI-controlled Shields, so it cannot serve as a standing
/// value for a human console and never clears back to `None` on its own.
/// `SensorsThreatState` is the one producer both paths ultimately derive
/// from, and it clears to `None` itself (`ship::sensors::tick_sensors_threat_warning`)
/// the moment Sensors reports no hostile in range.
fn publish_shields_blackboard(
    mut ships_q: Query<
        (
            &ShipShields,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
            Option<&crate::ship::state::ShipPhysics>,
            Option<&crate::ship::sensors::SensorsThreatState>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
    asteroid_q: Query<
        (&crate::server_app::AsteroidUuid, &Transform),
        Without<crate::entities::spawner::EntityUuid>,
    >,
    entity_q: Query<
        (&crate::entities::spawner::EntityUuid, &Transform),
        Without<crate::server_app::AsteroidUuid>,
    >,
) {
    for (shields, hull, control_sources, physics, sensors_threat, mut bbs) in ships_q.iter_mut() {
        let physics = physics.copied().unwrap_or_default();
        // Frozen Combat Lock from this ship's viewscreen blackboard (written in
        // the previous tick's PublishAggregate — this system runs in Publish).
        let combat_lock: Option<String> = match bbs
            .0
            .get(&crate::ship::system_registry::viewscreen_system_id())
        {
            Some(crate::core::messages::SystemBlackboard::Viewscreen(vbb)) => {
                vbb.combat_lock.clone()
            }
            _ => None,
        };

        // Snapshot facings once so we can reuse them for both the aggregate
        // and per-arc blackboards.
        let snapshots = shields.0.snapshot();
        let facings: Vec<ShieldFacingStatus> = shield_facing_statuses(&snapshots);

        let (total_hp, total_current) = hull
            .map(|h| (h.0.total_max(), h.0.total_current()))
            .unwrap_or((100.0, 100.0));
        let hull_integrity_pct = if total_hp > 0.0 {
            ((total_current / total_hp) * 100.0).clamp(0.0, 100.0)
        } else {
            100.0
        };

        let focused_facing = facings
            .iter()
            .find(|f| f.is_focused)
            .map(|f| f.label.clone());

        let any_offline = facings.iter().any(|f| !f.online);
        let grid_status = if any_offline {
            "EMITTER OFFLINE"
        } else {
            "GRID NOMINAL"
        }
        .to_string();

        let combat_lock_bearing = combat_lock.as_ref().and_then(|uuid| {
            let live = asteroid_q
                .iter()
                .find(|(u, _)| u.0 == *uuid)
                .map(|(_, t)| (t.translation.x, t.translation.z))
                .or_else(|| {
                    entity_q
                        .iter()
                        .find(|(u, _)| u.0 == *uuid)
                        .map(|(_, t)| (t.translation.x, t.translation.z))
                })?;
            let dx = live.0 - physics.x;
            let dz = live.1 - physics.z;
            let bearing_rad = (simmath::atan2(dz, dx) - physics.yaw + std::f32::consts::PI)
                % (2.0 * std::f32::consts::PI);
            Some(bearing_rad.to_degrees())
        });

        // Same conversion `console_ai::server::ai_shield_focus` applies to
        // the delayed copy of this same fact, so the console marker and the
        // AI's decision agree numerically, not just in source.
        let threat_bearing = sensors_threat
            .and_then(|s| s.last_bearing_rad)
            .map(|rad| (rad.to_degrees() + 360.0) % 360.0);

        let bb = ShieldsBlackboard {
            facings: facings.clone(),
            hull_integrity_pct,
            focused_facing,
            combat_lock_bearing,
            threat_bearing,
            grid_status,
            frequency: shields.frequency(),
        };

        // Per-arc fine blackboards (issue #514). One entry per arc under
        // `SystemId("shield-arc-<id>")`. `is_online` combines hull-based
        // offline (from `offline_systems`) with shield-timer offline
        // (`snap.online`) — matches the pattern used by
        // `PowerReactorBlackboard.is_online` derivation.
        let per_arc: Vec<(SystemId, ShieldArcBlackboard)> = snapshots
            .iter()
            .filter_map(|snap| {
                if snap.id.is_empty() {
                    return None;
                }
                let sid = crate::ship::system_registry::shield_arc_system_id(&snap.id)?;
                let hull_offline = control_sources
                    .map(|cs| cs.0.is_offline(&sid))
                    .unwrap_or(false);
                let is_online = snap.online && !hull_offline;
                Some((
                    sid,
                    ShieldArcBlackboard {
                        label: snap.label.clone(),
                        hp: snap.hp,
                        max_hp: snap.max_hp,
                        is_online,
                        is_focused: snap.is_focused,
                        offline_remaining: snap.offline_remaining,
                        center_deg: snap.center_deg,
                        width_deg: snap.width_deg,
                    },
                ))
            })
            .collect();

        bbs.0.insert(
            SystemId(crate::ship::system_registry::SHIELDS_SYSTEM_ID.to_string()),
            SystemBlackboard::Shields(bb),
        );
        for (sid, arc_bb) in per_arc {
            bbs.0.insert(sid, SystemBlackboard::ShieldArc(arc_bb));
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

// ── AI controller ────────────────────────────────────────────────────────────────
//
// The fused decide+mutate `operate_shields_ai` system (damage tracking,
// health monitoring, focus decision, threat-bearing override) was split in
// issue #692 into a decide system + apply adapter. Issue #826 retired the
// adapter and its `ShieldArcIntents` transport: `console_ai::server::
// ai_shield_focus` (decision, unchanged) now emits admitted
// `SetShieldArcFocus` payloads through `command_admission::validate_and_admit`
// with the ship's own `ai:<uuid>` token, and `handle_shields_messages` above
// applies them — the single truth-integration point for human and AI alike.
// `ShieldsDamageHistory`, `ShieldsAiConfigResource`, and
// `PendingShieldsThreatBearing` remain here since they're shield-domain state
// read/written by the decide system.

/// Angular distance (degrees) between two angles on a circle, always in [0, 180].
pub(crate) fn angular_distance_deg(a: f32, b: f32) -> f32 {
    let diff = (a - b).abs() % 360.0;
    diff.min(360.0 - diff)
}

#[cfg(test)]
#[path = "shields_tests.rs"]
mod tests;

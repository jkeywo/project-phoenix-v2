use crate::core::messages::{
    ActionFeedbackOutcome, AdmittedCommands, CoordinationPayload, ShieldArcBlackboard,
    ShieldFacingStatus, ShieldsBlackboard, SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::ship_plugin::{
    CoordinationDelivery, CoordinationEnqueue, DeliveredCoordination, ShipConfigComponent,
    ShipSystemControlSources,
};
use crate::simmath;
use bevy::prelude::*;

/// Pending Sensors->Shields threat bearing, delivered via the channel-3
/// coordination bus (issue #683). Set by [`receive_shields_coordination`] after
/// the generic lag router delivers a `CoordinationPayload::ThreatBearing` to
/// AI-controlled Shields; read by `console_ai::server::shields_tree::ai_shields` to bias
/// facing toward the threat.
#[derive(Component, Clone, Debug, Default)]
pub struct PendingShieldsThreatBearing(pub Option<f32>);

/// Resolve the AI-operated System that owns this ship's Shields focus decision.
///
/// The capability exists only when the hull authors a `kind = "shields"`
/// system (the shipped id happens to be `shields-system`) and that exact system
/// currently operates AI. Ownerless `shield_arc` systems do not imply the
/// higher-level focus capability. The legacy aggregate blackboard id `shields`
/// is deliberately not a control-source key.
pub fn ai_operated_shields_focus_system<'a>(
    control_sources: &ShipSystemControlSources,
    ship_config: &'a crate::ship::config::ShipConfig,
) -> Option<&'a SystemId> {
    let system = ship_config
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::SHIELDS_KIND)?;

    control_sources
        .0
        .policy_for(&system.id)
        .operate_ai
        .then_some(&system.id)
}

pub fn shields_focus_operates_ai(
    control_sources: &ShipSystemControlSources,
    ship_config: &crate::ship::config::ShipConfig,
) -> bool {
    ai_operated_shields_focus_system(control_sources, ship_config).is_some()
}

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

/// A single damage event recorded for the shields AI damage-tracking window.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DamageRecord {
    /// Absolute logical simulation tick on which damage was recorded.
    pub recorded_tick: u64,
    /// Amount of HP lost in this damage event.
    pub amount: i32,
}

/// Whether a damage record belongs to the current tick-native history window.
///
/// The end boundary is deliberately strict: a record remains recent while
/// `age < window_ticks` and expires exactly when its age reaches the authored
/// window. A future-stamped record is never recent; treating subtraction as
/// saturating would otherwise make malformed continuation state look new.
pub fn damage_record_is_recent(recorded_tick: u64, current_tick: u64, window_ticks: u64) -> bool {
    current_tick
        .checked_sub(recorded_tick)
        .is_some_and(|age| age < window_ticks)
}

/// Per-arc damage history for the shields AI controller.
///
/// Indexed by facing index (`Vec<Vec<DamageRecord>>`). Each record is
/// stamped with the authoritative [`crate::sim_tick::SimTick`] and pruned once
/// its tick age reaches the configured history window.
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
    /// Used by the independent physical damage-history producer.
    pub fn ensure_len(&mut self, n: usize) {
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
    pub fn last_observed_hp(&self, facing_idx: usize, current_hp: i32) -> i32 {
        self.last_hp.get(facing_idx).copied().unwrap_or(current_hp)
    }

    /// Records this tick's HP as the new baseline for the next tick's
    /// damage-detection comparison. Must be called once per arc, per tick,
    /// after `last_observed_hp` has been consulted.
    pub fn observe_hp(&mut self, facing_idx: usize, current_hp: i32) {
        if facing_idx < self.last_hp.len() {
            self.last_hp[facing_idx] = current_hp;
        }
    }

    pub fn record_damage(&mut self, facing_idx: usize, recorded_tick: u64, amount: i32) {
        if facing_idx < self.arcs.len() {
            self.arcs[facing_idx].push(DamageRecord {
                recorded_tick,
                amount,
            });
        }
    }

    pub fn prune_old(&mut self, current_tick: u64, window_ticks: u64) {
        for arc in &mut self.arcs {
            arc.retain(|record| {
                damage_record_is_recent(record.recorded_tick, current_tick, window_ticks)
            });
        }
    }
}

/// TOML-loaded configuration for the shields AI controller.
///
/// Loaded from `[shields_console.ai]` in the ship entity TOML. Defaults are
/// used when the section is absent.
///
/// Dual `Resource + Component`. Since issue #738 **every production read goes
/// through the per-entity Component** — the individual arc-tree host and
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
    /// Read the authoritative advisory debounce for continuation projection.
    pub fn continuation(&self) -> (&[bool], &[bool]) {
        (&self.down_notified, &self.restore_notified)
    }

    /// Replace bootstrap debounce state with a restored continuation.
    pub fn replace_continuation(&mut self, down_notified: Vec<bool>, restore_notified: Vec<bool>) {
        self.down_notified = down_notified;
        self.restore_notified = restore_notified;
    }

    fn ensure_len(&mut self, n: usize) {
        if self.down_notified.len() < n {
            self.down_notified.resize(n, false);
            self.restore_notified.resize(n, false);
        }
    }
}

/// Shields-owned receiver for delayed Coordination deliveries (issue #1258).
///
/// The generic lag router owns the delay and the live Station routing decision;
/// it emits [`DeliveredCoordination`] only for a Station that resolves to AI.
/// This receiver owns the Shields-domain consequence: a typed
/// [`CoordinationPayload::ThreatBearing`] is latched for the next Shields AI
/// decision. The resolved AI outcome, explicit address, and live authored
/// Shields capability policy are checked again so a malformed handoff, or an
/// ownership change at the receiving seam, cannot write Shields' private
/// pending state.
pub fn receive_shields_coordination(
    mut delivered: MessageReader<DeliveredCoordination>,
    mut ships: Query<
        (
            &mut PendingShieldsThreatBearing,
            &ShipSystemControlSources,
            &ShipConfigComponent,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    for message in delivered.read() {
        if message.delivery != CoordinationDelivery::Ai {
            continue;
        }
        let Ok((mut pending, control_sources, ship_config)) = ships.get_mut(message.source_entity)
        else {
            continue;
        };
        let Some(shields_address) = crate::ship::coordination::address_for_system_kind(
            &ship_config.0,
            crate::ship::system_registry::SHIELD_ARC_KIND,
        ) else {
            continue;
        };
        if message.address != shields_address
            || !shields_focus_operates_ai(control_sources, &ship_config.0)
        {
            continue;
        }
        if let CoordinationPayload::ThreatBearing { bearing_rad, .. } = &message.payload {
            pending.0 = Some(*bearing_rad);
        }
    }
}

/// Fund physical shield work for every equipped entity, including static actors.
/// Runs after reactor intake and shield commands, before this tick's impacts.
/// Upkeep precedes recovery and recharge; ordinary modifiers scale the desired
/// recharge rate, while the canonical local reserve pays the actual work.
pub fn tick_shields(
    time: Res<Time>,
    mut shields_q: Query<(
        &mut ShipShields,
        Option<&crate::modifiers::ShipModifiers>,
        Option<&mut crate::ship::power::ShipPowerSystem>,
        Option<&ShipSystemControlSources>,
        Option<&mut ShieldsDamageHistory>,
        Option<&crate::ship::physical_damage::ShipDamage>,
    )>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut shield, mods, mut power, sources, mut history, physical) in shields_q.iter_mut() {
        let operation = |id: &SystemId| {
            physical.map_or(1.0, |damage| {
                damage.factor(id, phoenix_model::damage::DamageBinding::Operation)
            })
        };
        let before_work: Vec<_> = shield.0.facings.iter().map(|facing| facing.hp).collect();
        let regen_scale = mods
            .map(|m| m.get(&crate::core::messages::ModifierSlot::ShieldRegen))
            .unwrap_or(1.0);
        let mut order: Vec<_> = (0..shield.0.facings.len()).collect();
        order.sort_by(|a, b| shield.0.facings[*a].id.cmp(&shield.0.facings[*b].id));
        // Every arc pays upkeep before any arc gets discretionary recharge.
        for &index in &order {
            let facing = &mut shield.0.facings[index];
            let Some(id) = crate::ship::system_registry::shield_arc_system_id(&facing.id) else {
                facing.protection_fraction = 0.0;
                continue;
            };
            let available =
                sources.is_none_or(|sources| !sources.0.is_offline(&id)) && operation(&id) > 0.0;
            facing.protection_fraction = if available && facing.raised && facing.is_online() {
                power.as_deref_mut().map_or(0.0, |power| {
                    power
                        .0
                        .spend_continuous(&id, facing.energy_config.upkeep_per_sec * dt)
                        .map_or(0.0, |grant| grant.fraction * operation(&id))
                })
            } else {
                0.0
            };
        }

        // Recovery credits canonical batteries before the recipient's ordinary
        // recharge. A donor being discharged does not also recharge this tick.
        for &index in &order {
            let facing = &mut shield.0.facings[index];
            let Some(target) = facing.recovery_target.as_ref() else {
                continue;
            };
            let config = &facing.energy_config;
            let Some(id) = crate::ship::system_registry::shield_arc_system_id(&facing.id) else {
                continue;
            };
            let efficiency = operation(&id);
            if efficiency <= 0.0
                || !config.recovery_targets.contains(target)
                || sources.is_some_and(|sources| sources.0.is_offline(&id))
                || !facing.is_online()
                || config.recovery_energy_per_hp <= 0.0
                || config.discharge_hp_per_sec <= 0.0
            {
                continue;
            }
            let wanted = facing
                .stored_strength()
                .min(config.discharge_hp_per_sec as f64 * dt as f64);
            let conversion = config.recovery_energy_per_hp as f64 * efficiency as f64;
            let exact_offer = (wanted * conversion).min(f32::MAX as f64);
            let mut offered = exact_offer as f32;
            if offered as f64 > exact_offer {
                offered = f32::from_bits(offered.to_bits() - 1);
            }
            let Some(power) = power.as_deref_mut() else {
                continue;
            };
            if power
                .0
                .network()
                .consumers
                .get(&id)
                .is_none_or(|state| state.thermal_stopped)
            {
                continue;
            }
            // Stage both owners: malformed numerical state cannot credit the
            // recipient and then fail to debit its donor.
            let mut staged = power.0.clone();
            let intake = physical.map_or(1.0, |damage| {
                damage.factor(target, phoenix_model::damage::DamageBinding::Intake)
            });
            let Ok(accepted) =
                staged.accept_recovered_energy_with_intake_factor(target, dt, offered, intake)
            else {
                continue;
            };
            let removal = accepted as f64 / conversion;
            if removal > wanted || facing.discharge_strength(removal).is_err() {
                continue;
            }
            power.0 = staged;
        }
        let grants: Vec<_> = shield
            .0
            .facings
            .iter()
            .enumerate()
            .map(|(index, facing)| {
                let decaying = shield.0.focused_facing != Some(index) && facing.hp > facing.max_hp;
                if !facing.is_online()
                    || !facing.raised
                    || facing.recovery_target.is_some()
                    || decaying
                    || facing.hp >= facing.max_hp
                    || facing.regen_per_sec <= 0.0
                {
                    return 0.0;
                }
                let wanted_hp = ((facing.max_hp - facing.hp) as f32 - facing.hp_frac()).max(0.0);
                let Some(id) = crate::ship::system_registry::shield_arc_system_id(&facing.id)
                else {
                    return 0.0;
                };
                if sources.is_some_and(|sources| sources.0.is_offline(&id)) || operation(&id) <= 0.0
                {
                    return 0.0;
                }
                let wanted_hp = wanted_hp.min(facing.regen_per_sec * dt * regen_scale.max(0.0));
                let cost = wanted_hp * facing.energy_config.recharge_energy_per_hp;
                power.as_deref_mut().map_or(0.0, |p| {
                    p.0.spend_continuous(&id, cost).map_or(0.0, |grant| {
                        wanted_hp * grant.fraction * operation(&id) / (facing.regen_per_sec * dt)
                    })
                })
            })
            .collect();
        shield.0.tick_with_regen_scales(dt, &grants);
        for (index, facing) in shield.0.facings.iter_mut().enumerate() {
            if let Some(baseline) = history
                .as_deref_mut()
                .and_then(|history| history.last_hp.get_mut(index))
            {
                *baseline = (*baseline as i64 + facing.hp as i64 - before_work[index] as i64)
                    .clamp(0, i32::MAX as i64) as i32;
            }
            let stopped = crate::ship::system_registry::shield_arc_system_id(&facing.id)
                .and_then(|id| {
                    power
                        .as_deref()
                        .and_then(|power| power.0.network().consumers.get(&id))
                })
                .is_none_or(|state| state.thermal_stopped);
            if stopped {
                facing.protection_fraction = 0.0;
            }
        }
    }
}

/// Handle `SetShieldArcFocus` messages from every ship's Shields console.
///
/// Iterates every ship (player + NPC) so both the player's Shields console
/// commands and the AI's admitted the individual arc-tree host emissions (issue #826)
/// flip each ship's own shield focus — one applier, no origin branching.
///
/// Per-arc dispatch (#514): each `[[shield_arc]]` synthesises a
/// `SystemId("shield-arc-<id>")`; the JS panel sends
/// `SetShieldArcFocus { focused: bool }` targeted at that arc's SystemId.
/// The handler iterates the facings on each ship and picks up admitted
/// commands per arc target. Setting `focused = true` on one arc clears
/// focus on any other arc (the shield system carries a single focus slot).
pub fn handle_shields_messages(
    mut ship_query: Query<(&AdmittedCommands, &mut ShipShields)>,
    mut outbound: Option<
        ResMut<bevy::ecs::message::Messages<crate::lobby::server::OutboundMessage>>,
    >,
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
                let index = shields
                    .0
                    .facings
                    .iter()
                    .position(|f| f.id == *arc_id)
                    .expect("target collected from this shield");
                match &cmd.payload {
                    SystemControlPayload::SetShieldRaised { raised } => {
                        shields.0.facings[index].raised = *raised;
                        crate::command_admission::finish_action_feedback(
                            cmd,
                            &mut outbound,
                            ActionFeedbackOutcome::Applied,
                        );
                        continue;
                    }
                    SystemControlPayload::SetShieldRecovery { target } => {
                        let config = &shields.0.facings[index].energy_config;
                        let valid = target.as_ref().is_none_or(|target| {
                            config.recovery_energy_per_hp > 0.0
                                && config.discharge_hp_per_sec > 0.0
                                && config.recovery_targets.contains(target)
                        });
                        if valid {
                            shields.0.facings[index].recovery_target = target.clone();
                        }
                        crate::command_admission::finish_action_feedback(
                            cmd,
                            &mut outbound,
                            if valid {
                                ActionFeedbackOutcome::Applied
                            } else {
                                ActionFeedbackOutcome::Refused
                            },
                        );
                        continue;
                    }
                    _ => {}
                }
                let SystemControlPayload::SetShieldArcFocus { focused } = &cmd.payload else {
                    continue;
                };
                if *focused && !shields.0.facings[index].focus_available {
                    crate::command_admission::finish_action_feedback(
                        cmd,
                        &mut outbound,
                        ActionFeedbackOutcome::Refused,
                    );
                    continue;
                }
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
                    let effective_focus = new_focus.unwrap_or(shields.0.focused_facing);
                    if effective_focus == Some(index) {
                        new_focus = Some(None);
                    }
                }
                crate::command_admission::finish_action_feedback(
                    cmd,
                    &mut outbound,
                    ActionFeedbackOutcome::Applied,
                );
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

/// Convert a `ShieldSystem::snapshot()` result into the wire
/// `Vec<ShieldFacingStatus>` shape.
///
/// The one conversion every broadcaster of a ship's shield facings uses:
/// this ship's own `ShieldsBlackboard.facings` (below, via
/// `publish_shields_blackboard`), a reconnecting client's resync
/// `ShieldStatus` (`reconnect_shields_projection`), the
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
/// `console_ai::server::shields_tree::ai_shields` reads (delayed, via the channel-3
/// `PendingShieldsThreatBearing` inbox) to override the damage-based focus
/// decision. Reading the live component here, rather than the AI's one-shot
/// coordination inbox, is deliberate: `PendingShieldsThreatBearing` is
/// consumed (`Option::take`) the instant the AI reads it and is only ever
/// populated for an AI-controlled Shields, so it cannot serve as a standing
/// value for a human console and never clears back to `None` on its own.
/// `SensorsThreatState` is the one producer both paths ultimately derive
/// from, and it clears to `None` itself (`ship::sensors::tick_sensors_threat_warning`)
/// the moment Sensors reports no hostile in range.
pub fn publish_shields_blackboard(
    mut ships_q: Query<(
        &ShipShields,
        Option<&crate::ship::power::ShipPowerSystem>,
        Option<&ShieldsDamageHistory>,
        Option<&crate::entities::spawner::EntitySystemHull>,
        Option<&crate::ship_plugin::ShipSystemControlSources>,
        Option<&crate::ship::state::ShipPhysics>,
        Option<&crate::ship::sensors::SensorsThreatState>,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
    asteroid_q: Query<
        (&crate::server_app::AsteroidUuid, &Transform),
        Without<crate::entities::spawner::EntityUuid>,
    >,
    entity_q: Query<
        (&crate::entities::spawner::EntityUuid, &Transform),
        Without<crate::server_app::AsteroidUuid>,
    >,
) {
    for (shields, power, history, hull, control_sources, physics, sensors_threat, mut bbs) in
        ships_q.iter_mut()
    {
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

        // Same conversion `console_ai::server::shields_tree::ai_shields` applies to
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
                let facing = shields
                    .0
                    .facings
                    .iter()
                    .find(|facing| facing.id == snap.id)?;
                let reserve = power.and_then(|power| {
                    let profile = power
                        .0
                        .config()
                        .consumers
                        .iter()
                        .find(|profile| profile.system == sid)?;
                    let state = power.0.network().consumers.get(&sid)?;
                    Some(phoenix_model::energy::ConsumerEnergyReading {
                        system: sid.clone(),
                        group: power
                            .0
                            .ship_config()
                            .systems
                            .iter()
                            .find(|system| system.id == sid)
                            .and_then(|system| system.power_group.clone()),
                        energy: state.energy,
                        capacity: profile.reserve.capacity,
                        heat: state.heat,
                        trip_heat: profile.reserve.trip_heat,
                        recover_heat: profile.reserve.recover_heat,
                        thermal_stopped: state.thermal_stopped,
                        intake_per_sec: power.0.last_intake_per_sec(&sid).unwrap_or(0.0),
                        max_intake_per_sec: profile.reserve.max_intake_per_sec,
                        flow_override_per_sec: power.0.network().fine_flows.get(&sid).copied(),
                    })
                });
                Some((
                    sid,
                    ShieldArcBlackboard {
                        arc_id: snap.id.clone(),
                        label: snap.label.clone(),
                        hp: snap.hp,
                        max_hp: snap.max_hp,
                        is_online,
                        is_focused: snap.is_focused,
                        offline_remaining: snap.offline_remaining,
                        center_deg: snap.center_deg,
                        width_deg: snap.width_deg,
                        recent_damage: shields
                            .0
                            .facings
                            .iter()
                            .position(|facing| facing.id == snap.id)
                            .and_then(|index| history.and_then(|history| history.arcs.get(index)))
                            .map_or(0.0, |records| {
                                records
                                    .iter()
                                    .map(|record| record.amount as f64)
                                    .sum::<f64>()
                                    .min(f32::MAX as f64) as f32
                            }),
                        operation: Some(crate::core::messages::ShieldOperationBlackboard {
                            focus_available: facing.focus_available,
                            raised: facing.raised,
                            can_recharge: facing.raised
                                && facing.is_online()
                                && !hull_offline
                                && reserve
                                    .as_ref()
                                    .is_some_and(|reserve| !reserve.thermal_stopped),
                            protection_fraction: facing.protection_fraction,
                            reserve,
                            upkeep_per_sec: facing.energy_config.upkeep_per_sec,
                            recharge_energy_per_hp: facing.energy_config.recharge_energy_per_hp,
                            recovery_energy_per_hp: facing.energy_config.recovery_energy_per_hp,
                            discharge_hp_per_sec: facing.energy_config.discharge_hp_per_sec,
                            recovery_target: facing.recovery_target.clone(),
                            recovery_targets: facing.energy_config.recovery_targets.clone(),
                        }),
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

/// Angular distance (degrees) between two angles on a circle, always in [0, 180].
pub fn angular_distance_deg(a: f32, b: f32) -> f32 {
    let diff = (a - b).abs() % 360.0;
    diff.min(360.0 - diff)
}

#[cfg(test)]
#[path = "shields_tests.rs"]
mod tests;

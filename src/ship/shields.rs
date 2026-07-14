use bevy::prelude::*;

use crate::core::broadcast::{Audience, Cadence, SimBroadcaster};
use crate::messages::{
    AdmittedCommands, CoordinationPayload, ShieldArcBlackboard, ShieldFacingStatus,
    ShieldsBlackboard, SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::ship_plugin::CoordinationEnqueue;

// ── Components ─────────────────────────────────────────────────────────────────

/// The ship's shield system.
///
/// Per-ship shield system — a `ShieldSystem` wrapped in a Component.
///
/// Pure per-ship Component post ship-parity audit; the legacy `Resource`
/// derive has been dropped since no production code reads a global
/// `Res<ShipShields>`.
#[derive(Component)]
pub struct ShipShields(pub crate::shield::ShieldSystem, pub f32);

impl ShipShields {
    pub fn frequency(&self) -> f32 {
        self.1
    }

    pub fn set_frequency(&mut self, freq: f32) {
        self.1 = freq.clamp(0.0, 1.0);
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
}

impl ShieldsDamageHistory {
    fn ensure_len(&mut self, n: usize) {
        if self.arcs.len() < n {
            self.arcs.resize(n, Vec::new());
        }
    }

    fn record_damage(&mut self, facing_idx: usize, timestamp: f32, amount: i32) {
        if facing_idx < self.arcs.len() {
            self.arcs[facing_idx].push(DamageRecord { timestamp, amount });
        }
    }

    fn prune_old(&mut self, current_time: f32, window_secs: f32) {
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
/// Dual `Resource + Component` post ship-parity audit: production reads
/// use the Resource form (single ship-wide AI tuning), but the Component
/// derive is available if NPC ships ever need per-ship AI tuning.
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
        app.add_message::<CoordinationEnqueue>()
            .init_resource::<ShieldsAiConfigResource>()
            .add_systems(
                Update,
                (
                    handle_shields_messages.in_set(crate::sim_sets::SimSet::Input),
                    emit_shields_coordination.in_set(crate::sim_sets::SimSet::Input),
                    operate_shields_ai.in_set(crate::sim_sets::SimSet::Physics),
                    tick_shields.in_set(crate::sim_sets::SimSet::Modifiers),
                    publish_shields_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                ),
            )
            .add_plugins(shields_state_broadcaster());
    }
}

/// Tick shield regen and offline timers each frame for every ship
/// (player + NPCs). PR-7 (issue #597) unifies this with the old
/// `tick_npc_shield_regen` — one system iterating all ships with `Ship` marker.
pub fn tick_shields(
    time: Res<Time>,
    mut shields_q: Query<&mut ShipShields, With<crate::server_app::Ship>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for mut shield in shields_q.iter_mut() {
        shield.0.tick(dt);
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
            let facings: Vec<ShieldFacingStatus> = shields
                .0
                .snapshot()
                .into_iter()
                .map(|s| ShieldFacingStatus {
                    label: s.label,
                    hp: s.hp,
                    max_hp: s.max_hp,
                    online: s.online,
                    offline_remaining: s.offline_remaining,
                    is_focused: s.is_focused,
                    center_deg: s.center_deg,
                    width_deg: s.width_deg,
                    arc_id: s.id,
                    priority: s.priority,
                })
                .collect();
            let frequency = shields.frequency();
            vec![crate::messages::ServerMessage::ShieldStatus { facings, frequency }]
        },
    )
}

// ── Systems ────────────────────────────────────────────────────────────────────

/// Handle `SetShieldArcFocus` messages from every ship's Shields console.
///
/// Iterates every ship (player + NPC) so both the player's Shields console
/// commands and the future NPC `operate_shields_ai` writes into
/// `AdmittedCommands` flip each ship's own shield focus.
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
        let arc_targets: Vec<(String, crate::messages::SystemId)> = shields
            .0
            .facings
            .iter()
            .filter_map(|f| {
                if f.id.is_empty() {
                    None
                } else {
                    crate::system_registry::shield_arc_system_id(&f.id)
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
            &crate::ship_state::ShipRedAlert,
            &crate::ship_plugin::ShipSystemControlSources,
            &mut ShieldsCoordinationState,
        ),
        With<crate::server_app::Ship>,
    >,
    ai_config: Res<ShieldsAiConfigResource>,
    mut writer: MessageWriter<CoordinationEnqueue>,
) {
    for (entity, shields, red_alert, control_sources, mut coord_state) in ship_q.iter_mut() {
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
            .find_map(|s| crate::system_registry::shield_arc_system_id(&s.id));
        let sender_origin = first_arc_sid
            .as_ref()
            .map(|sid| control_sources.0.source_for(sid))
            .unwrap_or_default();

        for (i, snap) in snapshots.iter().enumerate() {
            if !snap.online {
                if !coord_state.down_notified[i] {
                    coord_state.down_notified[i] = true;
                    coord_state.restore_notified[i] = false;

                    let payload = CoordinationPayload::ShieldFacingDown {
                        label: snap.label.clone(),
                        offline_remaining: snap.offline_remaining,
                    };
                    writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin,
                        target: crate::system_registry::helm_system_id(),
                        payload,
                        sender_label: "Shields".to_string(),
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
                    writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin,
                        target: crate::system_registry::helm_system_id(),
                        payload,
                        sender_label: "Shields".to_string(),
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

// ── Blackboard publish ─────────────────────────────────────────────────────────

fn publish_shields_blackboard(
    shields_q: Query<&ShipShields, With<crate::server_app::LocalShip>>,
    hull_q: Query<&crate::entity_spawner::EntitySystemHull, With<crate::server_app::LocalShip>>,
    control_sources_q: Query<
        &crate::ship_plugin::ShipSystemControlSources,
        With<crate::server_app::LocalShip>,
    >,
    physics_q: Query<&crate::ship_state::ShipPhysics, With<crate::simulation::LocalShip>>,
    weapons_target_q: Query<
        &crate::weapons_plugin::WeaponsTarget,
        With<crate::server_app::LocalShip>,
    >,
    asteroid_q: Query<
        (&crate::simulation::AsteroidUuid, &Transform),
        Without<crate::entity_spawner::EntityUuid>,
    >,
    entity_q: Query<
        (&crate::entity_spawner::EntityUuid, &Transform),
        Without<crate::simulation::AsteroidUuid>,
    >,
    mut ship_bbs_q: Query<
        &mut crate::server_app::ShipSystemBlackboards,
        With<crate::server_app::LocalShip>,
    >,
) {
    let Some(shields) = shields_q.iter().next() else {
        return;
    };
    let physics = physics_q.single().ok().copied().unwrap_or_default();
    let control_sources = control_sources_q.single().ok();

    // Snapshot facings once so we can reuse them for both the aggregate
    // and per-arc blackboards.
    let snapshots = shields.0.snapshot();
    let facings: Vec<ShieldFacingStatus> = snapshots
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
        .collect();

    let (total_hp, total_current) = hull_q
        .single()
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

    let target_bearing = weapons_target_q.single().ok().and_then(|wt| {
        let uuid = wt.0.as_ref()?;
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
        let bearing_rad =
            (dz.atan2(dx) - physics.yaw + std::f32::consts::PI) % (2.0 * std::f32::consts::PI);
        Some(bearing_rad.to_degrees())
    });

    let bb = ShieldsBlackboard {
        facings: facings.clone(),
        hull_integrity_pct,
        focused_facing,
        target_bearing,
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
            let sid = crate::system_registry::shield_arc_system_id(&snap.id)?;
            let hull_offline = control_sources
                .map(|cs| cs.0.offline_systems.contains(&sid))
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

    if let Some(mut bbs) = ship_bbs_q.iter_mut().next() {
        bbs.0.insert(
            SystemId(crate::system_registry::SHIELDS_SYSTEM_ID.to_string()),
            SystemBlackboard::Shields(bb),
        );
        for (sid, arc_bb) in per_arc {
            bbs.0.insert(sid, SystemBlackboard::ShieldArc(arc_bb));
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

// ── AI controller ────────────────────────────────────────────────────────────────

/// Per-kind AI plugin for shields.
///
/// Gated on policy.operate_ai for the Shields system. Runs damage tracking,
/// health monitoring, and focus decisions for all AI-controlled shield systems
/// on player and NPC ships.
///
/// # Damage Tracking
/// Damage is detected by comparing each arc's current HP against the last
/// recorded HP stored in `ShieldsDamageHistory`. When HP decreases (and the
/// arc is not offline), the delta is recorded as a `DamageRecord` with the
/// current timestamp.
///
/// # Focus Decision (`tick_shield_focus_ai`)
/// 1. **Damage concentration** — in the configurable time window, if any arc
///    receives ≥ `damage_pct_threshold` % of total damage, focus it.
/// 2. **Health imbalance** — if no arc met the damage threshold, compare
///    normalized HP fractions; focus the weakest arc if it is below
///    `health_ratio_threshold` % of the second-weakest arc.
/// 3. **Otherwise clear focus.**
///
/// Ships with fewer than 2 arcs exit early (nothing to focus).
fn operate_shields_ai(
    time: Res<Time>,
    global_ai_config: Res<ShieldsAiConfigResource>,
    mut ships: Query<
        (
            &crate::ship_plugin::ShipSystemControlSources,
            &mut ShipShields,
            &mut ShieldsDamageHistory,
            Option<&ShieldsAiConfigResource>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let current_time = time.elapsed_secs();

    for (control_sources, mut shields, mut damage_history, ai_config_comp) in ships.iter_mut() {
        let policy = control_sources
            .0
            .policy_for(&crate::system_registry::shields_system_id());
        if !policy.operate_ai {
            continue;
        }

        let ai_cfg: &ShieldsAiConfigResource = ai_config_comp.unwrap_or(&*global_ai_config);
        let facings = &shields.0.facings;

        // Single-arc ships have nothing to focus.
        if facings.len() < 2 {
            continue;
        }

        // Lazily resize damage history to match arc count.
        damage_history.ensure_len(facings.len());

        // ── Detect damage: compare current HP vs last recorded ──────────────
        for (idx, facing) in facings.iter().enumerate() {
            // Use the last record's HP as previous, or current HP if no records.
            let prev_hp = damage_history
                .arcs
                .get(idx)
                .and_then(|records| records.last())
                .map(|r| r.amount)
                .unwrap_or(facing.hp);

            // Detect a decrease in HP (damage taken) while the arc was online.
            // If the arc went offline the HP dropped to 0 but offline_remaining
            // is set, which shows as a big jump in offline_remaining — we still
            // want to record that as damage to the arc.
            if facing.hp < prev_hp {
                let delta = prev_hp - facing.hp;
                damage_history.record_damage(idx, current_time, delta);
            }
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

        // Apply the focus decision.
        let new_focus = match decision {
            crate::console_ai::ShieldFocusAiOutput::Focus { facing_index } => {
                if facing_index < facings.len() {
                    Some(Some(facing_index))
                } else {
                    None
                }
            }
            crate::console_ai::ShieldFocusAiOutput::ClearFocus => Some(None),
            crate::console_ai::ShieldFocusAiOutput::None => None,
        };

        if let Some(focus) = new_focus {
            shields.0.set_focused_facing(focus);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::{ClientMessage, *};
    use crate::server_app::{LocalShip, ShipSystemBlackboards};
    use crate::ship::control_source::ControlSource;
    use crate::ship_plugin::CoordinationEnqueue;
    use crate::simulation::{
        LastBroadcastEntityPositions, LastBroadcastHull, LastBroadcastShields, ShipImpulse,
        ShipShields, SimOutbox,
    };
    use crate::system_registry::SHIELDS_SYSTEM_ID;

    #[derive(Resource)]
    struct ShipEntity(Entity);

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    #[derive(Resource, Default)]
    struct CoordEnqueueBox(Vec<CoordinationEnqueue>);

    fn collect_coord(
        mut reader: MessageReader<CoordinationEnqueue>,
        mut box_: ResMut<CoordEnqueueBox>,
    ) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    fn test_app() -> App {
        let config = crate::shield::ShieldConfig {
            num_facings: 2,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::server_app::LocalShip,
                ShipShields(crate::shield::ShieldSystem::new(&config), 0.5),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                {
                    let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                    // Post-#514: coordination emitter looks up the first
                    // arc's SystemId. `ShieldSystem::new` populates arc ids
                    // "fore"/"aft" for a 2-facing default.
                    cs.0.set(
                        crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                        ControlSource::Ai,
                    );
                    cs
                },
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::messages::AdmittedCommands::default(),
                crate::ship_state::ShipRedAlert::default(),
                ShieldsCoordinationState::default(),
                ShipImpulse(crate::impulse::ImpulseState::new()),
            ))
            .id();
        app.insert_resource(ShipEntity(ship));
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .init_resource::<CoordEnqueueBox>()
            .add_plugins(ShipShieldsPlugin)
            .add_systems(PostUpdate, collect)
            .add_systems(PostUpdate, collect_coord);
        app
    }

    fn push_msg(app: &mut App, token: &str, msg: ClientMessage) {
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
            out.push(OutboundMessage {
                target,
                msg,
                delivery: crate::messages::DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        out
    }

    fn drain_coord(app: &mut App) -> Vec<CoordinationEnqueue> {
        let msgs = app.world().resource::<CoordEnqueueBox>().0.clone();
        app.world_mut().resource_mut::<CoordEnqueueBox>().0.clear();
        msgs
    }

    // Superseded by `start_game_with_shields_and_helm` below for tests that
    // also need a Helm session; retained as a documented no-op since no test
    // in this module currently calls the captain-only variant directly.
    #[allow(dead_code)]
    fn start_game_with_shields(app: &mut App) {
        push_msg(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "shields",
            ClientMessage::Identify {
                token: "shields".into(),
                name: "Scotty".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "shields",
            ClientMessage::SelectStation {
                station: "Shields".into(),
            },
        );
        tick(app);
        push_msg(app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(app, "shields", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    // ── Blackboard publish tests ─────────────────────────────────────────────

    fn shields_bb(app: &mut App) -> ShieldsBlackboard {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        // Safety: test always spawns exactly one LocalShip entity.
        let bbs = q
            .single(app.world())
            .expect("no LocalShip with ShipSystemBlackboards");
        let key = SystemId(SHIELDS_SYSTEM_ID.to_string());
        let SystemBlackboard::Shields(bb) = bbs.0.get(&key).unwrap() else {
            panic!("expected Shields blackboard");
        };
        bb.clone()
    }

    #[test]
    fn publish_shields_blackboard_contains_hull_integrity() {
        let mut app = test_app();
        app.update();
        assert!((shields_bb(&mut app).hull_integrity_pct - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn publish_shields_blackboard_four_facings() {
        let config = crate::shield::ShieldConfig {
            num_facings: 4,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        let _ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::server_app::LocalShip,
                ShipShields(crate::shield::ShieldSystem::new(&config), 0.5),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                {
                    let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                    // Post-#514: emit_shields_coordination reads the first
                    // arc's SystemId as sender_origin. Set "fore" for the
                    // 4-facing default (Fore, Port, Aft, Starboard).
                    cs.0.set(
                        crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                        ControlSource::Ai,
                    );
                    cs
                },
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::messages::AdmittedCommands::default(),
                ShipImpulse(crate::impulse::ImpulseState::new()),
            ))
            .id();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .add_plugins(ShipShieldsPlugin);
        app.update();
        assert_eq!(shields_bb(&mut app).facings.len(), 4);
    }

    fn ship_e(app: &mut App) -> Entity {
        app.world().resource::<ShipEntity>().0
    }

    #[test]
    fn publish_shields_blackboard_shows_focused_facing() {
        let mut app = test_app();
        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .set_focused_facing(Some(0));
        app.update();
        assert!(shields_bb(&mut app).focused_facing.is_some());
    }

    #[test]
    fn publish_shields_blackboard_clears_focused_facing() {
        let mut app = test_app();
        let se = ship_e(&mut app);
        {
            let mut e = app.world_mut().entity_mut(se);
            let mut shields = e.get_mut::<ShipShields>().unwrap();
            shields.0.set_focused_facing(Some(0));
            shields.0.set_focused_facing(None);
        }
        app.update();
        assert_eq!(shields_bb(&mut app).focused_facing, None);
    }

    #[test]
    fn publish_shields_blackboard_grid_offline_when_facing_down() {
        let mut app = test_app();
        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);
        app.update();
        assert_eq!(shields_bb(&mut app).grid_status, "EMITTER OFFLINE");
    }

    #[test]
    fn publish_shields_blackboard_stable_on_double_update() {
        let mut app = test_app();
        app.update();
        app.update();
        assert!((shields_bb(&mut app).hull_integrity_pct - 100.0).abs() < f32::EPSILON);
    }

    // ── Coordination tests ──────────────────────────────────────────────────

    fn test_app_with_helm() -> App {
        let config = crate::shield::ShieldConfig {
            num_facings: 2,
            max_hp: 100,
            regen_per_sec: 0.0,
            offline_duration: 10.0,
        };
        let mut app = App::new();
        let ship = app
            .world_mut()
            .spawn((
                crate::simulation::Ship,
                crate::server_app::LocalShip,
                ShipShields(crate::shield::ShieldSystem::new(&config), 0.5),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::ship_plugin::ShipConfigComponent::default(),
                {
                    let mut cs = crate::ship_plugin::ShipSystemControlSources::default();
                    // Post-#514: emit_shields_coordination looks up the first
                    // arc's SystemId as the sender_origin. `ShieldSystem::new`
                    // populates arc ids from `default_arc_id`; for 2-facing
                    // that's "fore" and "aft". Set the first arc to Ai so the
                    // test asserts continue to hold.
                    cs.0.set(
                        crate::system_registry::shield_arc_system_id("fore").expect("fore"),
                        ControlSource::Ai,
                    );
                    cs
                },
                crate::ship_plugin::ActiveStationRatings::default(),
                crate::ship_plugin::CoordinationQueue::default(),
                crate::messages::AdmittedCommands::default(),
                crate::ship_state::ShipRedAlert::default(),
                ShieldsCoordinationState::default(),
                ShipImpulse(crate::impulse::ImpulseState::new()),
            ))
            .id();
        app.insert_resource(ShipEntity(ship));
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(100),
            ))
            .init_resource::<crate::lobby::WorldResource>()
            .init_resource::<SimOutbox>()
            .init_resource::<LastBroadcastEntityPositions>()
            .init_resource::<crate::simulation::LastBroadcastEntityHealth>()
            .init_resource::<LastBroadcastHull>()
            .init_resource::<LastBroadcastShields>()
            .init_resource::<Outbox>()
            .init_resource::<CoordEnqueueBox>()
            .add_plugins(ShipShieldsPlugin)
            .add_systems(PostUpdate, collect)
            .add_systems(PostUpdate, collect_coord);
        app
    }

    fn start_game_with_shields_and_helm(app: &mut App) {
        push_msg(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "helm",
            ClientMessage::Identify {
                token: "helm".into(),
                name: "Sulu".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "helm",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(app);
        push_msg(app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(app, "helm", ClientMessage::SetReady { ready: true });
        tick(app);
    }

    #[test]
    fn shield_facing_down_coordination_sent_to_helm_when_facing_goes_offline() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        // Drain facing 0 offline.
        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);

        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);

        let down_msgs: Vec<_> = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
            .collect();

        assert!(
            !down_msgs.is_empty(),
            "expected a ShieldFacingDown CoordinationEnqueue to be sent"
        );
        assert!(
            down_msgs
                .iter()
                .all(|m| m.target == crate::system_registry::helm_system_id()),
            "ShieldFacingDown should target the helm system"
        );
    }

    #[test]
    fn shield_facing_down_fires_only_once_per_offline_cycle() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);

        tick(&mut app); // first tick — fires
        drain_coord(&mut app); // discard first tick's messages

        tick(&mut app); // second tick — should not re-fire
        let coord_msgs = drain_coord(&mut app);

        let count = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
            .count();

        assert_eq!(
            count, 0,
            "ShieldFacingDown should not fire again on the same offline cycle"
        );
    }

    #[test]
    fn shield_facing_restored_fires_on_red_alert_when_hp_recovers() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        // Put facing offline.
        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);
        tick(&mut app);
        drain_coord(&mut app); // discard down notification

        // Manually restore the facing and set HP to above threshold.
        {
            let mut e = app.world_mut().entity_mut(se);
            let mut shields = e.get_mut::<ShipShields>().unwrap();
            let facing = &mut shields.0.facings[0];
            facing.offline_remaining = 0.0;
            facing.hp = 60; // 60/100 = 0.6 >= 0.5 threshold
        }

        // Activate red alert via per-entity ShipRedAlert component.
        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_state::ShipRedAlert, bevy::prelude::With<crate::simulation::LocalShip>>();
            if let Ok(mut ra) = q.single_mut(app.world_mut()) {
                ra.toggle();
            }
        }

        // Mark down_notified on the per-ship ShieldsCoordinationState so
        // the restore branch can fire.
        {
            let se = ship_e(&mut app);
            let mut e = app.world_mut().entity_mut(se);
            let mut coord = e.get_mut::<ShieldsCoordinationState>().unwrap();
            if coord.down_notified.is_empty() {
                coord.down_notified.push(true);
                coord.restore_notified.push(false);
            } else {
                coord.down_notified[0] = true;
            }
        }

        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);

        let restored_msgs: Vec<_> = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingRestored { .. }))
            .collect();

        assert!(
            !restored_msgs.is_empty(),
            "expected a ShieldFacingRestored CoordinationEnqueue on red alert after recovery"
        );
    }

    #[test]
    fn shield_facing_restored_does_not_fire_without_red_alert() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);
        tick(&mut app);
        drain_coord(&mut app); // discard down notification

        {
            let mut e = app.world_mut().entity_mut(se);
            let mut shields = e.get_mut::<ShipShields>().unwrap();
            let facing = &mut shields.0.facings[0];
            facing.offline_remaining = 0.0;
            facing.hp = 60;
        }

        if let Some(mut coord) = app
            .world_mut()
            .entity_mut(se)
            .get_mut::<ShieldsCoordinationState>()
        {
            if coord.down_notified.is_empty() {
                coord.down_notified.push(true);
                coord.restore_notified.push(false);
            } else {
                coord.down_notified[0] = true;
            }
        }

        // No red alert active.
        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);

        let count = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingRestored { .. }))
            .count();

        assert_eq!(
            count, 0,
            "ShieldFacingRestored should not fire when not on red alert"
        );
    }

    /// Verify that the `CoordinationEnqueue` event carries `sender_origin = Ai`
    /// by default (no explicit `ShipSystemControlSources` set), confirming the
    /// channel-3 routing matrix will treat it as AI-originated and route
    /// correctly (AI → Human = Popup; AI → AI = Consume) at delivery time.
    #[test]
    fn shield_facing_down_coordination_carries_ai_sender_origin_for_routing() {
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0);

        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);

        let down_msgs: Vec<_> = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
            .collect();

        assert!(!down_msgs.is_empty(), "expected ShieldFacingDown enqueue");
        assert!(
            down_msgs
                .iter()
                .all(|m| m.sender_origin == ControlSource::Ai),
            "default sender_origin should be Ai (shields console has no holder)"
        );
        assert!(
            down_msgs
                .iter()
                .all(|m| m.target == crate::system_registry::helm_system_id()),
            "ShieldFacingDown should target the helm system"
        );
    }

    // ── Issue #514 tests ─────────────────────────────────────────────────────

    #[test]
    fn shield_facing_down_still_fires_for_variable_arc() {
        // Regression: after the SystemId shape flipped from `shields` to
        // per-arc `shield-arc-<id>`, coordination messages must still fire
        // when an arc goes offline. The test app uses a 2-facing default so
        // arc ids are "fore" and "aft".
        let mut app = test_app_with_helm();
        start_game_with_shields_and_helm(&mut app);

        let se = ship_e(&mut app);
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .apply_damage(9999, 0.0); // deplete facing 0 (fore)

        tick(&mut app);
        let coord_msgs = drain_coord(&mut app);
        let down_msgs: Vec<_> = coord_msgs
            .iter()
            .filter(|m| matches!(&m.payload, CoordinationPayload::ShieldFacingDown { .. }))
            .collect();
        assert!(
            !down_msgs.is_empty(),
            "expected a ShieldFacingDown after arc depletion (variable-arc regression)"
        );
    }

    #[test]
    fn handle_set_shield_arc_focus_flips_focus() {
        // Basic wire-shape assertion: a `SetShieldArcFocus { focused: true }`
        // targeted at `shield-arc-fore` moves focus to that facing.
        let mut app = test_app();
        // Manually admit the command (bypasses the full authorisation stack).
        let se = ship_e(&mut app);
        let arc_sid = crate::system_registry::shield_arc_system_id("fore").expect("fore");
        app.world_mut()
            .entity_mut(se)
            .get_mut::<crate::messages::AdmittedCommands>()
            .unwrap()
            .0
            .push(crate::messages::AdmittedCommand {
                target: arc_sid.clone(),
                payload: SystemControlPayload::SetShieldArcFocus { focused: true },
                response_token: None,
            });
        tick(&mut app);
        let shields = app.world().entity(se).get::<ShipShields>().unwrap();
        assert_eq!(shields.0.focused_facing, Some(0), "fore arc focused");
    }

    #[test]
    fn handle_set_shield_arc_focus_clears_focus_when_target_matches_current() {
        let mut app = test_app();
        let se = ship_e(&mut app);
        // Manually set focus first.
        app.world_mut()
            .entity_mut(se)
            .get_mut::<ShipShields>()
            .unwrap()
            .0
            .set_focused_facing(Some(0));
        // Send `focused: false` targeted at fore → clears.
        let arc_sid = crate::system_registry::shield_arc_system_id("fore").expect("fore");
        app.world_mut()
            .entity_mut(se)
            .get_mut::<crate::messages::AdmittedCommands>()
            .unwrap()
            .0
            .push(crate::messages::AdmittedCommand {
                target: arc_sid,
                payload: SystemControlPayload::SetShieldArcFocus { focused: false },
                response_token: None,
            });
        tick(&mut app);
        let shields = app.world().entity(se).get::<ShipShields>().unwrap();
        assert_eq!(shields.0.focused_facing, None);
    }

    #[test]
    fn publish_writes_shield_arc_blackboard_per_arc() {
        // The publish system emits one `SystemBlackboard::ShieldArc` entry
        // per arc under `SystemId("shield-arc-<id>")`, alongside the
        // aggregate `Shields` blackboard.
        let mut app = test_app();
        tick(&mut app);
        let se = ship_e(&mut app);
        let bbs = app
            .world()
            .entity(se)
            .get::<crate::server_app::ShipSystemBlackboards>()
            .expect("ShipSystemBlackboards");
        // 2-facing default: fore + aft.
        for arc_id in &["fore", "aft"] {
            let sid = crate::system_registry::shield_arc_system_id(arc_id).expect("arc id");
            let bb = bbs.0.get(&sid).unwrap_or_else(|| {
                panic!(
                    "expected ShieldArc blackboard under {sid:?}, got {:?}",
                    bbs.0.keys().collect::<Vec<_>>()
                )
            });
            match bb {
                SystemBlackboard::ShieldArc(arc_bb) => {
                    assert_eq!(arc_bb.hp, 100, "arc {arc_id} starts full");
                    assert!(arc_bb.is_online, "arc {arc_id} starts online");
                }
                other => panic!("expected ShieldArc variant, got {other:?}"),
            }
        }
        // Aggregate `shields` blackboard also present.
        assert!(
            bbs.0.contains_key(&SystemId(
                crate::system_registry::SHIELDS_SYSTEM_ID.to_string()
            )),
            "aggregate shields blackboard must still be published"
        );
    }

    #[test]
    fn publish_shield_arc_blackboard_is_online_reflects_offline_systems() {
        // When a fine shield-arc-<id> SystemId is in offline_systems (via
        // hull-damage sync), the arc's `is_online` in the per-arc
        // blackboard must be false.
        let mut app = test_app();
        let se = ship_e(&mut app);
        // Directly mark fore arc as offline via ControlSources.
        let arc_sid = crate::system_registry::shield_arc_system_id("fore").expect("fore");
        app.world_mut()
            .entity_mut(se)
            .get_mut::<crate::ship_plugin::ShipSystemControlSources>()
            .unwrap()
            .0
            .offline_systems
            .insert(arc_sid.clone());
        tick(&mut app);
        let bbs = app
            .world()
            .entity(se)
            .get::<crate::server_app::ShipSystemBlackboards>()
            .expect("ShipSystemBlackboards");
        let bb = bbs.0.get(&arc_sid).expect("fore arc blackboard");
        match bb {
            SystemBlackboard::ShieldArc(arc_bb) => {
                assert!(
                    !arc_bb.is_online,
                    "fore arc must report is_online=false when in offline_systems"
                );
            }
            other => panic!("expected ShieldArc variant, got {other:?}"),
        }
    }
}
